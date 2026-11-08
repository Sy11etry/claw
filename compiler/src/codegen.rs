//! CIR to ARM64. In 0.1 this is deliberately naive: every virtual register is
//! a stack slot; each instruction loads its operands, computes, stores back.

use crate::arm64::*;
use crate::cir::*;

const IMPORTS: &[&str] = &["_write", "_exit"];
const IMP_WRITE: usize = 0;
const IMP_EXIT: usize = 1;

#[derive(Clone, Copy)]
enum Target {
    User(FuncId),
    Write,
    Trap,
    PrintInt,
    PrintBool,
    Import(usize),
}

pub struct Object {
    pub code: Vec<u32>,
    pub rodata: Vec<u8>,
    /// (index of `adrp`, followed by `add`; offset into rodata)
    pub data_refs: Vec<(usize, usize)>,
    /// (index of `adrp`, followed by `ldr`; GOT slot)
    pub got_refs: Vec<(usize, usize)>,
    pub imports: Vec<String>,
    /// Symbol name and code position in instructions.
    pub symbols: Vec<(String, usize)>,
    pub entry: usize,
}

struct Gen {
    code: Vec<u32>,
    rodata: Vec<u8>,
    data_refs: Vec<(usize, usize)>,
    got_refs: Vec<(usize, usize)>,
    /// Instructions waiting for a `b`/`bl` displacement.
    calls: Vec<(usize, Target)>,
    file: String,
    too_large: Option<String>,
}

#[derive(Clone, Copy)]
enum Trap {
    Overflow,
    DivZero,
    Shift,
}

impl Trap {
    fn text(self) -> &'static str {
        match self {
            Trap::Overflow => "integer overflow",
            Trap::DivZero => "division by zero",
            Trap::Shift => "shift out of range",
        }
    }
}

enum Pending {
    B(usize, BlockId),
    Cond(usize, BlockId),
}

struct Frame {
    outgoing: u32,
}

impl Frame {
    fn slot(&self, v: VReg) -> u32 {
        self.outgoing + v * 8
    }
}

impl Gen {
    fn e(&mut self, w: u32) {
        self.code.push(w);
    }

    fn here(&self) -> usize {
        self.code.len()
    }

    fn call(&mut self, base: u32, t: Target) {
        self.calls.push((self.code.len(), t));
        self.e(base);
    }

    fn mov_imm(&mut self, rd: Reg, v: i64) {
        let u = v as u64;
        if u <= 0xFFFF {
            self.e(movz(rd, u as u32, 0));
            return;
        }
        if v < 0 && !v as u64 <= 0xFFFF {
            self.e(movn(rd, !v as u32, 0));
            return;
        }
        let mut first = true;
        for hw in 0..4 {
            let part = ((u >> (hw * 16)) & 0xFFFF) as u32;
            if part == 0 {
                continue;
            }
            if first {
                self.e(movz(rd, part, hw));
                first = false;
            } else {
                self.e(movk(rd, part, hw));
            }
        }
    }

    fn load(&mut self, f: &Frame, r: Reg, v: VReg) {
        let off = f.slot(v);
        if off <= 32760 {
            self.e(ldr(r, SP, off));
        } else {
            self.mov_imm(X16, off as i64);
            self.e(ldr_reg(r, SP, X16));
        }
    }

    fn store(&mut self, f: &Frame, r: Reg, v: VReg) {
        let off = f.slot(v);
        if off <= 32760 {
            self.e(str(r, SP, off));
        } else {
            self.mov_imm(X16, off as i64);
            self.e(str_reg(r, SP, X16));
        }
    }

    fn data_addr(&mut self, r: Reg, off: usize) {
        self.data_refs.push((self.here(), off));
        self.e(adrp(r, 0));
        self.e(add_imm(r, r, 0));
    }

    fn add_str(&mut self, bytes: &[u8]) -> usize {
        let off = self.rodata.len();
        self.rodata.extend_from_slice(bytes);
        off
    }

    fn import_stub(&mut self, slot: usize) -> usize {
        let pos = self.here();
        self.got_refs.push((pos, slot));
        self.e(adrp(X16, 0));
        self.e(ldr(X16, X16, 0));
        self.e(br(X16));
        pos
    }

    /// `__claw_write(x0 = ptr, x1 = len)` writes to stdout.
    fn rt_write(&mut self) -> usize {
        let pos = self.here();
        self.e(mov(X2, X1));
        self.e(mov(X1, X0));
        self.e(movz(X0, 1, 0));
        self.call(B, Target::Import(IMP_WRITE));
        pos
    }

    /// `__claw_trap(x0 = ptr, x1 = len)` reports to stderr and exits with 101.
    fn rt_trap(&mut self) -> usize {
        let pos = self.here();
        self.e(stp_pre(FP, LR, SP, -16));
        self.e(mov_sp(FP, SP));
        self.e(mov(X2, X1));
        self.e(mov(X1, X0));
        self.e(movz(X0, 2, 0));
        self.call(BL, Target::Import(IMP_WRITE));
        self.e(movz(X0, 101, 0));
        self.call(BL, Target::Import(IMP_EXIT));
        self.e(brk(1));
        pos
    }

    /// `__claw_print_int(x0 = value)`; digits are built backwards on the stack.
    fn rt_print_int(&mut self) -> usize {
        let pos = self.here();
        self.e(stp_pre(FP, LR, SP, -48));
        self.e(mov_sp(FP, SP));
        self.e(add_imm(X9, SP, 47));
        self.e(movz(X10, 10, 0));
        self.e(strb(X10, X9, 0));
        self.e(mov(X11, X0));
        self.e(cmp_imm(X0, 0));
        self.e(b_cond(Cond::Ge, 2));
        // The magnitude of i64::MIN wraps to 2^63, correct when read unsigned.
        self.e(neg(X11, X0));
        self.e(udiv(X12, X11, X10));
        self.e(msub(X13, X12, X10, X11));
        self.e(add_imm(X13, X13, 48));
        self.e(sub_imm(X9, X9, 1));
        self.e(strb(X13, X9, 0));
        self.e(mov(X11, X12));
        self.e(cbnz(X11, -6));
        self.e(cmp_imm(X0, 0));
        self.e(b_cond(Cond::Ge, 4));
        self.e(movz(X13, 45, 0));
        self.e(sub_imm(X9, X9, 1));
        self.e(strb(X13, X9, 0));
        self.e(mov(X1, X9));
        self.e(add_imm(X2, SP, 48));
        self.e(sub(X2, X2, X9));
        self.e(movz(X0, 1, 0));
        self.call(BL, Target::Import(IMP_WRITE));
        self.e(ldp_post(FP, LR, SP, 48));
        self.e(ret());
        pos
    }

    /// `__claw_print_bool(x0 = 0 or 1)`
    fn rt_print_bool(&mut self) -> usize {
        let pos = self.here();
        let f = self.add_str(b"false\n");
        let t = self.add_str(b"true\n");
        self.e(cbnz(X0, 5));
        self.data_addr(X0, f);
        self.e(movz(X1, 6, 0));
        self.e(b_at(B, 4));
        self.data_addr(X0, t);
        self.e(movz(X1, 5, 0));
        self.call(B, Target::Write);
        pos
    }

    fn func(&mut self, f: &Func) {
        let mut outgoing = 0u32;
        for b in &f.blocks {
            for i in &b.insts {
                if let Inst::Call { args, .. } = i {
                    outgoing = outgoing.max(args.len().saturating_sub(8) as u32 * 8);
                }
            }
        }
        let size = (outgoing + f.nvregs * 8 + 15) & !15;
        if size >= 1 << 24 {
            self.too_large = Some(f.name.clone());
            return;
        }
        let fr = Frame { outgoing };

        self.e(stp_pre(FP, LR, SP, -16));
        self.e(mov_sp(FP, SP));
        if size >= 4096 {
            self.e(sub_imm_lsl12(SP, SP, size >> 12));
        }
        if size & 0xFFF != 0 {
            self.e(sub_imm(SP, SP, size & 0xFFF));
        }
        for i in 0..f.nparams as u32 {
            if i < 8 {
                self.store(&fr, i, i);
            } else {
                self.e(ldr(X9, FP, 16 + (i - 8) * 8));
                self.store(&fr, X9, i);
            }
        }

        let mut block_pos = vec![0usize; f.blocks.len()];
        let mut pending: Vec<Pending> = Vec::new();
        let mut traps: Vec<(usize, Trap, Loc)> = Vec::new();

        for (idx, b) in f.blocks.iter().enumerate() {
            block_pos[idx] = self.here();
            for i in &b.insts {
                self.inst(&fr, i, &mut traps);
            }
            let next = idx + 1;
            match &b.term {
                Term::Jmp(t) => {
                    if *t != next {
                        pending.push(Pending::B(self.here(), *t));
                        self.e(B);
                    }
                }
                Term::Br { c, t, f: fb } => {
                    self.load(&fr, X9, *c);
                    if *t == next {
                        pending.push(Pending::Cond(self.here(), *fb));
                        self.e(cbz(X9, 0));
                    } else {
                        pending.push(Pending::Cond(self.here(), *t));
                        self.e(cbnz(X9, 0));
                        if *fb != next {
                            pending.push(Pending::B(self.here(), *fb));
                            self.e(B);
                        }
                    }
                }
                Term::Ret(v) => {
                    if let Some(v) = v {
                        self.load(&fr, X0, *v);
                    }
                    self.e(mov_sp(SP, FP));
                    self.e(ldp_post(FP, LR, SP, 16));
                    self.e(ret());
                }
                Term::Unreachable => self.e(brk(2)),
            }
        }

        for p in pending {
            let (at, target, is_cond) = match p {
                Pending::B(at, t) => (at, t, false),
                Pending::Cond(at, t) => (at, t, true),
            };
            let disp = block_pos[target] as i64 - at as i64;
            if is_cond {
                self.patch_cond(at, disp, &f.name);
            } else {
                self.code[at] = b_at(self.code[at], disp as i32);
            }
        }

        // Cold paths go after the function so the hot path falls straight through.
        for (at, kind, loc) in traps {
            let disp = self.here() as i64 - at as i64;
            self.patch_cond(at, disp, &f.name);
            let msg = format!("trap: {} at {}:{}:{}\n", kind.text(), self.file, loc.line, loc.col);
            let off = self.add_str(msg.as_bytes());
            self.data_addr(X0, off);
            self.e(movz(X1, msg.len() as u32, 0));
            self.call(BL, Target::Trap);
        }
    }

    fn patch_cond(&mut self, at: usize, disp: i64, name: &str) {
        if !(-(1 << 18)..(1 << 18)).contains(&disp) {
            self.too_large = Some(name.to_string());
            return;
        }
        self.code[at] |= (disp as u32 & 0x7FFFF) << 5;
    }

    fn trap_if(&mut self, cond: Cond, kind: Trap, loc: Loc, traps: &mut Vec<(usize, Trap, Loc)>) {
        traps.push((self.here(), kind, loc));
        self.e(b_cond(cond, 0));
    }

    fn inst(&mut self, fr: &Frame, i: &Inst, traps: &mut Vec<(usize, Trap, Loc)>) {
        match i {
            Inst::Const { d, v } => {
                self.mov_imm(X9, *v);
                self.store(fr, X9, *d);
            }
            Inst::Mov { d, s } => {
                self.load(fr, X9, *s);
                self.store(fr, X9, *d);
            }
            Inst::Bin { op, d, a, b, loc } => {
                self.load(fr, X9, *a);
                self.load(fr, X10, *b);
                self.bin(*op, *loc, traps);
                self.store(fr, X11, *d);
            }
            Inst::Neg { d, s, loc } => {
                self.load(fr, X9, *s);
                self.e(negs(X11, X9));
                self.trap_if(Cond::Vs, Trap::Overflow, *loc, traps);
                self.store(fr, X11, *d);
            }
            Inst::BitNot { d, s } => {
                self.load(fr, X9, *s);
                self.e(mvn(X11, X9));
                self.store(fr, X11, *d);
            }
            Inst::BoolNot { d, s } => {
                self.load(fr, X9, *s);
                self.e(eor_1(X11, X9));
                self.store(fr, X11, *d);
            }
            Inst::Inc { d } => {
                self.load(fr, X9, *d);
                self.e(add_imm(X9, X9, 1));
                self.store(fr, X9, *d);
            }
            Inst::Cmp { cc, d, a, b } => {
                self.load(fr, X9, *a);
                self.load(fr, X10, *b);
                self.e(cmp(X9, X10));
                let c = match cc {
                    Cc::Eq => Cond::Eq,
                    Cc::Ne => Cond::Ne,
                    Cc::Lt => Cond::Lt,
                    Cc::Le => Cond::Le,
                    Cc::Gt => Cond::Gt,
                    Cc::Ge => Cond::Ge,
                };
                self.e(cset(X11, c as u32));
                self.store(fr, X11, *d);
            }
            Inst::Call { d, f, args } => {
                for (n, a) in args.iter().enumerate().skip(8) {
                    self.load(fr, X9, *a);
                    self.e(str(X9, SP, (n as u32 - 8) * 8));
                }
                for (n, a) in args.iter().enumerate().take(8) {
                    self.load(fr, n as Reg, *a);
                }
                self.call(BL, Target::User(*f));
                if let Some(d) = d {
                    self.store(fr, X0, *d);
                }
            }
            Inst::PrintInt { s } => {
                self.load(fr, X0, *s);
                self.call(BL, Target::PrintInt);
            }
            Inst::PrintBool { s } => {
                self.load(fr, X0, *s);
                self.call(BL, Target::PrintBool);
            }
            Inst::PrintStr { text } => {
                let mut bytes = text.as_bytes().to_vec();
                bytes.push(b'\n');
                let off = self.add_str(&bytes);
                self.data_addr(X0, off);
                self.mov_imm(X1, bytes.len() as i64);
                self.call(BL, Target::Write);
            }
        }
    }

    /// Operands in x9 and x10, result in x11.
    fn bin(&mut self, op: Arith, loc: Loc, traps: &mut Vec<(usize, Trap, Loc)>) {
        match op {
            Arith::Add => {
                self.e(adds(X11, X9, X10));
                self.trap_if(Cond::Vs, Trap::Overflow, loc, traps);
            }
            Arith::Sub => {
                self.e(subs(X11, X9, X10));
                self.trap_if(Cond::Vs, Trap::Overflow, loc, traps);
            }
            Arith::Mul => {
                self.e(mul(X11, X9, X10));
                self.e(smulh(X12, X9, X10));
                self.e(cmp_asr(X12, X11, 63));
                self.trap_if(Cond::Ne, Trap::Overflow, loc, traps);
            }
            Arith::Div => {
                self.e(cmp_imm(X10, 0));
                self.trap_if(Cond::Eq, Trap::DivZero, loc, traps);
                // MIN / -1 is the only quotient that does not fit in 64 bits.
                // Negating the dividend overflows exactly when it is MIN.
                self.e(cmn_imm(X10, 1));
                self.e(b_cond(Cond::Ne, 3));
                self.e(negs(X12, X9));
                self.trap_if(Cond::Vs, Trap::Overflow, loc, traps);
                self.e(sdiv(X11, X9, X10));
            }
            Arith::Rem => {
                self.e(cmp_imm(X10, 0));
                self.trap_if(Cond::Eq, Trap::DivZero, loc, traps);
                self.e(sdiv(X12, X9, X10));
                self.e(msub(X11, X12, X10, X9));
            }
            Arith::And => self.e(and(X11, X9, X10)),
            Arith::Or => self.e(orr(X11, X9, X10)),
            Arith::Xor => self.e(eor(X11, X9, X10)),
            Arith::Shl => {
                self.e(cmp_imm(X10, 64));
                self.trap_if(Cond::Hs, Trap::Shift, loc, traps);
                self.e(lsl(X11, X9, X10));
                self.e(asr(X12, X11, X10));
                self.e(cmp(X12, X9));
                self.trap_if(Cond::Ne, Trap::Overflow, loc, traps);
            }
            Arith::Shr => {
                self.e(cmp_imm(X10, 64));
                self.trap_if(Cond::Hs, Trap::Shift, loc, traps);
                self.e(asr(X11, X9, X10));
            }
        }
    }

}

pub fn generate(m: &Module, file: &str) -> Result<Object, String> {
    let mut g = Gen {
        code: Vec::new(),
        rodata: Vec::new(),
        data_refs: Vec::new(),
        got_refs: Vec::new(),
        calls: Vec::new(),
        file: file.to_string(),
        too_large: None,
    };
    let mut symbols: Vec<(String, usize)> = Vec::new();

    let stubs: Vec<usize> = (0..IMPORTS.len()).map(|s| g.import_stub(s)).collect();
    let write = g.rt_write();
    symbols.push(("___claw_write".to_string(), write));
    let trap = g.rt_trap();
    symbols.push(("___claw_trap".to_string(), trap));
    let print_int = g.rt_print_int();
    symbols.push(("___claw_print_int".to_string(), print_int));
    let print_bool = g.rt_print_bool();
    symbols.push(("___claw_print_bool".to_string(), print_bool));

    let entry = g.here();
    symbols.push(("___claw_entry".to_string(), entry));
    g.e(stp_pre(FP, LR, SP, -16));
    g.e(mov_sp(FP, SP));
    g.call(BL, Target::User(m.main));
    g.e(movz(X0, 0, 0));
    g.e(ldp_post(FP, LR, SP, 16));
    g.e(ret());

    let mut func_pos = Vec::with_capacity(m.funcs.len());
    for f in &m.funcs {
        func_pos.push(g.here());
        symbols.push((format!("_{}", f.name), g.here()));
        g.func(f);
    }

    for (at, t) in std::mem::take(&mut g.calls) {
        let dest = match t {
            Target::User(f) => func_pos[f],
            Target::Write => write,
            Target::Trap => trap,
            Target::PrintInt => print_int,
            Target::PrintBool => print_bool,
            Target::Import(i) => stubs[i],
        };
        g.code[at] = b_at(g.code[at], (dest as i64 - at as i64) as i32);
    }

    if let Some(name) = g.too_large {
        return Err(format!("E0901 function `{name}` is too large for claw 0.1 (1 MB of code); split it"));
    }

    Ok(Object {
        code: g.code,
        rodata: g.rodata,
        data_refs: g.data_refs,
        got_refs: g.got_refs,
        imports: IMPORTS.iter().map(|s| s.to_string()).collect(),
        symbols,
        entry,
    })
}
