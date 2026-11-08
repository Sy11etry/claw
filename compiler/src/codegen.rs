//! CIR to ARM64. Still deliberately naive: every virtual register is a stack
//! slot; each instruction loads its operands, computes, stores back.
//!
//! Internal calling convention: every value (floats as raw bits, aggregates as
//! addresses) goes in x0-x7, then on the stack. The result comes back in x0;
//! an aggregate result is written through a hidden first argument.

use crate::arm64::*;
use crate::cir::*;

const IMPORTS: &[&str] = &["_write", "_exit", "_snprintf", "_memcpy"];
const IMP_WRITE: usize = 0;
const IMP_EXIT: usize = 1;
const IMP_SNPRINTF: usize = 2;
const IMP_MEMCPY: usize = 3;

/// Output buffer: 8 bytes of length, then the bytes.
const BUF_CAP: u64 = 1 << 16;
/// Stay well inside the default 8 MB main-thread stack.
const MAX_FRAME: u64 = 7 << 20;

#[derive(Clone, Copy)]
enum Target {
    User(FuncId),
    Trap,
    Flush,
    BufStr,
    BufInt,
    BufUint,
    BufBool,
    BufFloat,
    Import(usize),
}

pub struct Object {
    pub code: Vec<u32>,
    pub rodata: Vec<u8>,
    /// (index of `adrp`, followed by `add`; offset into rodata)
    pub data_refs: Vec<(usize, usize)>,
    /// index of `adrp`, followed by `add`
    pub bss_refs: Vec<usize>,
    /// (index of `adrp`, followed by `ldr`; GOT slot)
    pub got_refs: Vec<(usize, usize)>,
    pub imports: Vec<String>,
    /// Symbol name and code position in instructions.
    pub symbols: Vec<(String, usize)>,
    pub entry: usize,
    pub bss_size: u64,
}

struct Gen {
    code: Vec<u32>,
    rodata: Vec<u8>,
    data_refs: Vec<(usize, usize)>,
    bss_refs: Vec<usize>,
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
    Bounds,
    Convert,
}

impl Trap {
    fn text(self) -> &'static str {
        match self {
            Trap::Overflow => "integer overflow",
            Trap::DivZero => "division by zero",
            Trap::Shift => "shift out of range",
            Trap::Bounds => "index out of bounds",
            Trap::Convert => "value does not fit the target type",
        }
    }
}

enum Pending {
    B(usize, BlockId),
    Cond(usize, BlockId),
}

struct Frame {
    outgoing: u32,
    objects: Vec<u64>,
}

impl Frame {
    fn slot(&self, v: VReg) -> u32 {
        self.outgoing + v * 8
    }
}

type Traps = Vec<(usize, Trap, Loc)>;

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

    /// Far slots are reached through x16; returns the base register and offset.
    fn float_slot(&mut self, f: &Frame, v: VReg) -> (Reg, u32) {
        let off = f.slot(v);
        if off <= 16380 {
            (SP, off)
        } else {
            self.mov_imm(X16, off as i64);
            self.e(add_ext(X16, SP, X16));
            (X16, 0)
        }
    }

    fn fload(&mut self, f: &Frame, r: Reg, v: VReg, bits: u8) {
        let (base, off) = self.float_slot(f, v);
        self.e(if bits == 64 { ldr_d(r, base, off) } else { ldr_s(r, base, off) });
    }

    fn fstore(&mut self, f: &Frame, r: Reg, v: VReg, bits: u8) {
        let (base, off) = self.float_slot(f, v);
        self.e(if bits == 64 { str_d(r, base, off) } else { str_s(r, base, off) });
    }

    fn data_addr(&mut self, r: Reg, off: usize) {
        self.data_refs.push((self.here(), off));
        self.e(adrp(r, 0));
        self.e(add_imm(r, r, 0));
    }

    fn bss_addr(&mut self, r: Reg) {
        self.bss_refs.push(self.here());
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

    /// `__claw_flush(x0 = fd)` writes out the buffer, retrying short writes.
    fn rt_flush(&mut self) -> usize {
        let pos = self.here();
        self.e(stp_pre(FP, LR, SP, -48));
        self.e(mov_sp(FP, SP));
        self.e(stp(X19, X20, SP, 16));
        self.e(str(X21, SP, 32));
        self.e(mov(X21, X0));
        self.bss_addr(X9);
        self.e(ldr(X20, X9, 0));
        self.e(str(XZR, X9, 0));
        self.e(add_imm(X19, X9, 8));
        self.e(cbz(X20, 10));
        self.e(mov(X0, X21));
        self.e(mov(X1, X19));
        self.e(mov(X2, X20));
        self.call(BL, Target::Import(IMP_WRITE));
        self.e(cmp_imm(X0, 0));
        self.e(b_cond(Cond::Le, 4));
        self.e(add(X19, X19, X0));
        self.e(sub(X20, X20, X0));
        self.e(b_at(B, -9));
        self.e(ldp(X19, X20, SP, 16));
        self.e(ldr(X21, SP, 32));
        self.e(ldp_post(FP, LR, SP, 48));
        self.e(ret());
        pos
    }

    /// `__claw_buf_str(x0 = ptr, x1 = len)` appends to the output buffer.
    fn rt_buf_str(&mut self) -> usize {
        let pos = self.here();
        self.e(stp_pre(FP, LR, SP, -32));
        self.e(mov_sp(FP, SP));
        self.e(stp(X19, X20, SP, 16));
        self.e(mov(X19, X0));
        self.e(add(X20, X0, X1));
        // loop:
        self.e(cmp(X19, X20));
        self.e(b_cond(Cond::Eq, 17));
        self.bss_addr(X9);
        self.e(ldr(X10, X9, 0));
        self.e(movz(X11, (BUF_CAP >> 16) as u32, 1));
        self.e(cmp(X10, X11));
        self.e(b_cond(Cond::Ne, 4));
        self.e(movz(X0, 1, 0));
        self.call(BL, Target::Flush);
        self.e(b_at(B, -10));
        self.e(add_imm(X12, X9, 8));
        self.e(ldrb(X13, X19, 0));
        self.e(add_imm(X19, X19, 1));
        self.e(strb_reg(X13, X12, X10));
        self.e(add_imm(X10, X10, 1));
        self.e(str(X10, X9, 0));
        self.e(b_at(B, -17));
        // done:
        self.e(ldp(X19, X20, SP, 16));
        self.e(ldp_post(FP, LR, SP, 32));
        self.e(ret());
        pos
    }

    /// `__claw_buf_int(x0)`; digits are built backwards on the stack.
    fn rt_buf_num(&mut self, signed: bool) -> usize {
        let pos = self.here();
        self.e(stp_pre(FP, LR, SP, -48));
        self.e(mov_sp(FP, SP));
        self.e(add_imm(X9, SP, 48));
        self.e(movz(X10, 10, 0));
        self.e(mov(X11, X0));
        if signed {
            self.e(cmp_imm(X0, 0));
            self.e(b_cond(Cond::Ge, 2));
            // The magnitude of i64::MIN wraps to 2^63, correct when read unsigned.
            self.e(neg(X11, X0));
        }
        self.e(udiv(X12, X11, X10));
        self.e(msub(X13, X12, X10, X11));
        self.e(add_imm(X13, X13, 48));
        self.e(sub_imm(X9, X9, 1));
        self.e(strb(X13, X9, 0));
        self.e(mov(X11, X12));
        self.e(cbnz(X11, -6));
        if signed {
            self.e(cmp_imm(X0, 0));
            self.e(b_cond(Cond::Ge, 4));
            self.e(movz(X13, 45, 0));
            self.e(sub_imm(X9, X9, 1));
            self.e(strb(X13, X9, 0));
        }
        self.e(mov(X0, X9));
        self.e(add_imm(X1, SP, 48));
        self.e(sub(X1, X1, X9));
        self.call(BL, Target::BufStr);
        self.e(ldp_post(FP, LR, SP, 48));
        self.e(ret());
        pos
    }

    /// `__claw_buf_bool(x0 = 0 or 1)`
    fn rt_buf_bool(&mut self) -> usize {
        let pos = self.here();
        let f = self.add_str(b"false");
        let t = self.add_str(b"true");
        self.e(cbnz(X0, 5));
        self.data_addr(X0, f);
        self.e(movz(X1, 5, 0));
        self.e(b_at(B, 4));
        self.data_addr(X0, t);
        self.e(movz(X1, 4, 0));
        self.call(B, Target::BufStr);
        pos
    }

    /// `__claw_buf_float(d0 = value, x2 = format)`, via libSystem's snprintf.
    /// On Apple ARM64 the variadic double is passed on the stack.
    fn rt_buf_float(&mut self) -> usize {
        let pos = self.here();
        self.e(stp_pre(FP, LR, SP, -16));
        self.e(mov_sp(FP, SP));
        self.e(sub_imm(SP, SP, 80));
        self.e(str_d(D0, SP, 0));
        self.e(add_imm(X0, SP, 16));
        self.e(movz(X1, 64, 0));
        self.call(BL, Target::Import(IMP_SNPRINTF));
        self.e(mov(X1, X0));
        self.e(add_imm(X0, SP, 16));
        self.call(BL, Target::BufStr);
        self.e(mov_sp(SP, FP));
        self.e(ldp_post(FP, LR, SP, 16));
        self.e(ret());
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

    fn func(&mut self, f: &Func, fmts: (usize, usize)) {
        let mut outgoing = 0u32;
        for b in &f.blocks {
            for i in &b.insts {
                if let Inst::Call { args, .. } = i {
                    outgoing = outgoing.max(args.len().saturating_sub(8) as u32 * 8);
                }
            }
        }
        let mut off = (outgoing + f.nvregs * 8) as u64;
        let mut objects = Vec::with_capacity(f.objects.len());
        for (size, align) in &f.objects {
            off = (off + align - 1) & !(align - 1);
            objects.push(off);
            off += size;
        }
        let size = (off + 15) & !15;
        if size >= MAX_FRAME {
            self.too_large = Some(f.name.clone());
            return;
        }
        let size = size as u32;
        let fr = Frame { outgoing, objects };

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
        let mut traps: Traps = Vec::new();

        for (idx, b) in f.blocks.iter().enumerate() {
            block_pos[idx] = self.here();
            for i in &b.insts {
                self.inst(&fr, i, &mut traps, fmts);
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

    fn trap_if(&mut self, cond: Cond, kind: Trap, loc: Loc, traps: &mut Traps) {
        traps.push((self.here(), kind, loc));
        self.e(b_cond(cond, 0));
    }

    /// Computes `p + off` into x9.
    fn addr_x9(&mut self, fr: &Frame, p: VReg, off: u64) {
        self.load(fr, X9, p);
        if off == 0 {
            return;
        }
        if off < 4096 {
            self.e(add_imm(X9, X9, off as u32));
        } else {
            self.mov_imm(X16, off as i64);
            self.e(add(X9, X9, X16));
        }
    }

    fn inst(&mut self, fr: &Frame, i: &Inst, traps: &mut Traps, fmts: (usize, usize)) {
        match i {
            Inst::Const { d, v } => {
                self.mov_imm(X9, *v);
                self.store(fr, X9, *d);
            }
            Inst::Mov { d, s } => {
                self.load(fr, X9, *s);
                self.store(fr, X9, *d);
            }
            Inst::Bin { op, d, a, b, num, loc } => {
                self.load(fr, X9, *a);
                self.load(fr, X10, *b);
                self.bin(*op, *num, *loc, traps);
                self.store(fr, X11, *d);
            }
            Inst::Neg { d, s, num, loc } => {
                self.load(fr, X9, *s);
                if num.bits == 64 {
                    self.e(negs(X11, X9));
                    self.trap_if(Cond::Vs, Trap::Overflow, *loc, traps);
                } else {
                    self.e(neg(X11, X9));
                    self.fit_check(*num, *loc, traps);
                }
                self.store(fr, X11, *d);
            }
            Inst::BitNot { d, s, num } => {
                self.load(fr, X9, *s);
                self.e(mvn(X11, X9));
                self.mask_unsigned(*num);
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
            Inst::Cmp { cc, d, a, b, unsigned } => {
                self.load(fr, X9, *a);
                self.load(fr, X10, *b);
                self.e(cmp(X9, X10));
                let c = match (cc, unsigned) {
                    (Cc::Eq, _) => Cond::Eq,
                    (Cc::Ne, _) => Cond::Ne,
                    (Cc::Lt, false) => Cond::Lt,
                    (Cc::Le, false) => Cond::Le,
                    (Cc::Gt, false) => Cond::Gt,
                    (Cc::Ge, false) => Cond::Ge,
                    (Cc::Lt, true) => Cond::Lo,
                    (Cc::Le, true) => Cond::Ls,
                    (Cc::Gt, true) => Cond::Hi,
                    (Cc::Ge, true) => Cond::Hs,
                };
                self.e(cset(X11, c as u32));
                self.store(fr, X11, *d);
            }
            Inst::FBin { op, d, a, b, bits } => {
                self.fload(fr, D0, *a, *bits);
                self.fload(fr, D1, *b, *bits);
                self.e(match op {
                    FArith::Add => fadd(D0, D0, D1, *bits),
                    FArith::Sub => fsub(D0, D0, D1, *bits),
                    FArith::Mul => fmul(D0, D0, D1, *bits),
                    FArith::Div => fdiv(D0, D0, D1, *bits),
                });
                self.fstore(fr, D0, *d, *bits);
            }
            Inst::FNeg { d, s, bits } => {
                self.fload(fr, D0, *s, *bits);
                self.e(fneg(D0, D0, *bits));
                self.fstore(fr, D0, *d, *bits);
            }
            Inst::FCmp { cc, d, a, b, bits } => {
                self.fload(fr, D0, *a, *bits);
                self.fload(fr, D1, *b, *bits);
                self.e(fcmp(D0, D1, *bits));
                // These conditions are all false for NaN, except `!=`.
                let c = match cc {
                    Cc::Eq => Cond::Eq,
                    Cc::Ne => Cond::Ne,
                    Cc::Lt => Cond::Mi,
                    Cc::Le => Cond::Ls,
                    Cc::Gt => Cond::Gt,
                    Cc::Ge => Cond::Ge,
                };
                self.e(cset(X11, c as u32));
                self.store(fr, X11, *d);
            }
            Inst::IntCast { d, s, from, to, loc } => {
                self.load(fr, X9, *s);
                self.int_cast(*from, *to, *loc, traps);
                self.store(fr, X11, *d);
            }
            Inst::IntToFloat { d, s, signed, bits } => {
                self.load(fr, X9, *s);
                self.e(if *signed { scvtf(D0, X9, *bits) } else { ucvtf(D0, X9, *bits) });
                self.fstore(fr, D0, *d, *bits);
            }
            Inst::FloatToInt { d, s, bits, to, loc } => {
                self.fload(fr, D0, *s, *bits);
                if *bits == 32 {
                    self.e(fcvt_s_to_d(D0, D0));
                }
                self.float_range_check(*to, *loc, traps);
                self.e(if to.signed { fcvtzs(X11, D0) } else { fcvtzu(X11, D0) });
                self.store(fr, X11, *d);
            }
            Inst::FloatCast { d, s, to_bits } => {
                let from = if *to_bits == 64 { 32 } else { 64 };
                self.fload(fr, D0, *s, from);
                self.e(if *to_bits == 64 { fcvt_s_to_d(D0, D0) } else { fcvt_d_to_s(D0, D0) });
                self.fstore(fr, D0, *d, *to_bits);
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
            Inst::ObjAddr { d, obj } => {
                let off = fr.objects[*obj];
                if off < 4096 {
                    self.e(add_imm(X9, SP, off as u32));
                } else {
                    self.mov_imm(X16, off as i64);
                    self.e(add_ext(X9, SP, X16));
                }
                self.store(fr, X9, *d);
            }
            Inst::AddImm { d, s, imm } => {
                self.addr_x9(fr, *s, *imm);
                self.store(fr, X9, *d);
            }
            Inst::Load { d, p, off, mem } => {
                self.addr_x9(fr, *p, *off);
                self.e(match mem {
                    Mem::I8 => ldrsb(X10, X9),
                    Mem::I16 => ldrsh(X10, X9),
                    Mem::I32 => ldrsw(X10, X9),
                    Mem::U8 => ldrb(X10, X9, 0),
                    Mem::U16 => ldrh(X10, X9),
                    Mem::U32 => ldr_w(X10, X9),
                    Mem::X64 => ldr(X10, X9, 0),
                });
                self.store(fr, X10, *d);
            }
            Inst::Store { p, off, s, mem } => {
                self.addr_x9(fr, *p, *off);
                self.load(fr, X10, *s);
                self.e(match mem {
                    Mem::I8 | Mem::U8 => strb(X10, X9, 0),
                    Mem::I16 | Mem::U16 => strh(X10, X9),
                    Mem::I32 | Mem::U32 => str_w(X10, X9),
                    Mem::X64 => str(X10, X9, 0),
                });
            }
            Inst::MemCopy { dst, src, size } => self.mem_copy(fr, *dst, *src, *size),
            Inst::IndexAddr { d, base, idx, size } => {
                self.load(fr, X9, *base);
                self.load(fr, X10, *idx);
                self.mov_imm(X11, *size as i64);
                self.e(mul(X10, X10, X11));
                self.e(add(X9, X9, X10));
                self.store(fr, X9, *d);
            }
            Inst::Bounds { idx, len, loc } => {
                self.load(fr, X9, *idx);
                self.mov_imm(X10, *len as i64);
                self.e(cmp(X9, X10));
                // Unsigned compare: a negative index looks huge.
                self.trap_if(Cond::Hs, Trap::Bounds, *loc, traps);
            }
            Inst::BufStr { text } => {
                let off = self.add_str(text.as_bytes());
                self.data_addr(X0, off);
                self.mov_imm(X1, text.len() as i64);
                self.call(BL, Target::BufStr);
            }
            Inst::BufInt { s, signed } => {
                self.load(fr, X0, *s);
                self.call(BL, if *signed { Target::BufInt } else { Target::BufUint });
            }
            Inst::BufBool { s } => {
                self.load(fr, X0, *s);
                self.call(BL, Target::BufBool);
            }
            Inst::BufFloat { s, bits } => {
                self.fload(fr, D0, *s, *bits);
                if *bits == 32 {
                    self.e(fcvt_s_to_d(D0, D0));
                }
                self.data_addr(X2, if *bits == 64 { fmts.0 } else { fmts.1 });
                self.call(BL, Target::BufFloat);
            }
            Inst::Flush { fd } => {
                self.e(movz(X0, *fd, 0));
                self.call(BL, Target::Flush);
            }
            Inst::Exit { code } => {
                self.e(movz(X0, *code, 0));
                self.call(BL, Target::Import(IMP_EXIT));
            }
        }
    }

    fn mem_copy(&mut self, fr: &Frame, dst: VReg, src: VReg, size: u64) {
        if size > 64 {
            self.load(fr, X0, dst);
            self.load(fr, X1, src);
            self.mov_imm(X2, size as i64);
            self.call(BL, Target::Import(IMP_MEMCPY));
            return;
        }
        self.load(fr, X9, dst);
        self.load(fr, X10, src);
        let mut off = 0u32;
        while (size - off as u64) >= 8 {
            self.e(ldr(X11, X10, off));
            self.e(str(X11, X9, off));
            off += 8;
        }
        while (off as u64) < size {
            self.e(ldrb(X11, X10, off));
            self.e(strb(X11, X9, off));
            off += 1;
        }
    }

    /// Traps if x11 is outside the signed range of `num`, which is narrower than 64 bits.
    fn fit_check(&mut self, num: Num, loc: Loc, traps: &mut Traps) {
        self.e(sbfx(X12, X11, num.bits as u32));
        self.e(cmp(X12, X11));
        self.trap_if(Cond::Ne, Trap::Overflow, loc, traps);
    }

    fn mask_unsigned(&mut self, num: Num) {
        if !num.signed && num.bits < 64 {
            self.e(ubfx(X11, X11, num.bits as u32));
        }
    }

    /// After an operation on 64-bit registers: narrow signed types trap, unsigned wrap.
    fn normalize(&mut self, num: Num, loc: Loc, traps: &mut Traps) {
        if num.signed {
            if num.bits < 64 {
                self.fit_check(num, loc, traps);
            }
        } else {
            self.mask_unsigned(num);
        }
    }

    fn int_cast(&mut self, from: Num, to: Num, loc: Loc, traps: &mut Traps) {
        if to.signed {
            if !from.signed && from.bits == 64 {
                // A u64 with the top bit set is negative when read as signed.
                self.e(cmp_imm(X9, 0));
                self.trap_if(Cond::Lt, Trap::Convert, loc, traps);
            }
            if to.bits < 64 {
                self.e(sbfx(X12, X9, to.bits as u32));
                self.e(cmp(X12, X9));
                self.trap_if(Cond::Ne, Trap::Convert, loc, traps);
            }
            self.e(mov(X11, X9));
        } else if to.bits < 64 {
            self.e(ubfx(X11, X9, to.bits as u32));
        } else {
            self.e(mov(X11, X9));
        }
    }

    /// Traps unless d0 converts to `to`: not NaN and within range after truncation.
    fn float_range_check(&mut self, to: Num, loc: Loc, traps: &mut Traps) {
        self.e(fcmp(D0, D0, 64));
        self.trap_if(Cond::Vs, Trap::Convert, loc, traps);
        let bits = to.bits as i32;
        let (hi, lo, lo_inclusive) = if to.signed {
            let hi = 2f64.powi(bits - 1);
            if bits == 64 {
                (hi, -hi, true)
            } else {
                (hi, -hi - 1.0, false)
            }
        } else {
            (2f64.powi(bits), -1.0, false)
        };
        self.mov_imm(X16, hi.to_bits() as i64);
        self.e(fmov_d_x(D1, X16));
        self.e(fcmp(D0, D1, 64));
        self.trap_if(Cond::Ge, Trap::Convert, loc, traps);
        self.mov_imm(X16, lo.to_bits() as i64);
        self.e(fmov_d_x(D1, X16));
        self.e(fcmp(D0, D1, 64));
        self.trap_if(if lo_inclusive { Cond::Lt } else { Cond::Le }, Trap::Convert, loc, traps);
    }

    /// Operands in x9 and x10, result in x11.
    fn bin(&mut self, op: Arith, num: Num, loc: Loc, traps: &mut Traps) {
        let wide_signed = num.signed && num.bits == 64;
        match op {
            Arith::Add => {
                if wide_signed {
                    self.e(adds(X11, X9, X10));
                    self.trap_if(Cond::Vs, Trap::Overflow, loc, traps);
                } else {
                    self.e(add(X11, X9, X10));
                    self.normalize(num, loc, traps);
                }
            }
            Arith::Sub => {
                if wide_signed {
                    self.e(subs(X11, X9, X10));
                    self.trap_if(Cond::Vs, Trap::Overflow, loc, traps);
                } else {
                    self.e(sub(X11, X9, X10));
                    self.normalize(num, loc, traps);
                }
            }
            Arith::Mul => {
                self.e(mul(X11, X9, X10));
                if wide_signed {
                    self.e(smulh(X12, X9, X10));
                    self.e(cmp_asr(X12, X11, 63));
                    self.trap_if(Cond::Ne, Trap::Overflow, loc, traps);
                } else {
                    self.normalize(num, loc, traps);
                }
            }
            Arith::Div => {
                self.e(cmp_imm(X10, 0));
                self.trap_if(Cond::Eq, Trap::DivZero, loc, traps);
                if num.signed {
                    if num.bits == 64 {
                        // MIN / -1 is the only quotient that does not fit in 64 bits.
                        // Negating the dividend overflows exactly when it is MIN.
                        self.e(cmn_imm(X10, 1));
                        self.e(b_cond(Cond::Ne, 3));
                        self.e(negs(X12, X9));
                        self.trap_if(Cond::Vs, Trap::Overflow, loc, traps);
                    }
                    self.e(sdiv(X11, X9, X10));
                    self.normalize(num, loc, traps);
                } else {
                    self.e(udiv(X11, X9, X10));
                }
            }
            Arith::Rem => {
                self.e(cmp_imm(X10, 0));
                self.trap_if(Cond::Eq, Trap::DivZero, loc, traps);
                self.e(if num.signed { sdiv(X12, X9, X10) } else { udiv(X12, X9, X10) });
                self.e(msub(X11, X12, X10, X9));
            }
            Arith::And => self.e(and(X11, X9, X10)),
            Arith::Or => self.e(orr(X11, X9, X10)),
            Arith::Xor => self.e(eor(X11, X9, X10)),
            Arith::Shl => {
                self.e(cmp_imm(X10, num.bits as u32));
                self.trap_if(Cond::Hs, Trap::Shift, loc, traps);
                self.e(lsl(X11, X9, X10));
                if wide_signed {
                    self.e(asr(X12, X11, X10));
                    self.e(cmp(X12, X9));
                    self.trap_if(Cond::Ne, Trap::Overflow, loc, traps);
                } else {
                    self.normalize(num, loc, traps);
                }
            }
            Arith::Shr => {
                self.e(cmp_imm(X10, num.bits as u32));
                self.trap_if(Cond::Hs, Trap::Shift, loc, traps);
                self.e(if num.signed { asr(X11, X9, X10) } else { lsr(X11, X9, X10) });
            }
        }
    }
}

pub fn generate(m: &Module, file: &str) -> Result<Object, String> {
    let mut g = Gen {
        code: Vec::new(),
        rodata: Vec::new(),
        data_refs: Vec::new(),
        bss_refs: Vec::new(),
        got_refs: Vec::new(),
        calls: Vec::new(),
        file: file.to_string(),
        too_large: None,
    };
    let mut symbols: Vec<(String, usize)> = Vec::new();

    let fmt64 = g.add_str(b"%.15g\0");
    let fmt32 = g.add_str(b"%.7g\0");

    let stubs: Vec<usize> = (0..IMPORTS.len()).map(|s| g.import_stub(s)).collect();
    let trap = g.rt_trap();
    let flush = g.rt_flush();
    let buf_str = g.rt_buf_str();
    let buf_int = g.rt_buf_num(true);
    let buf_uint = g.rt_buf_num(false);
    let buf_bool = g.rt_buf_bool();
    let buf_float = g.rt_buf_float();
    for (n, p) in [
        ("trap", trap),
        ("flush", flush),
        ("buf_str", buf_str),
        ("buf_int", buf_int),
        ("buf_uint", buf_uint),
        ("buf_bool", buf_bool),
        ("buf_float", buf_float),
    ] {
        symbols.push((format!("___claw_{n}"), p));
    }

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
        g.func(f, (fmt64, fmt32));
    }

    for (at, t) in std::mem::take(&mut g.calls) {
        let dest = match t {
            Target::User(f) => func_pos[f],
            Target::Trap => trap,
            Target::Flush => flush,
            Target::BufStr => buf_str,
            Target::BufInt => buf_int,
            Target::BufUint => buf_uint,
            Target::BufBool => buf_bool,
            Target::BufFloat => buf_float,
            Target::Import(i) => stubs[i],
        };
        g.code[at] = b_at(g.code[at], (dest as i64 - at as i64) as i32);
    }

    if let Some(name) = g.too_large {
        return Err(format!("E0901 function `{name}` is too large for this version: at most 1 MB of code and 7 MB of local data"));
    }

    Ok(Object {
        code: g.code,
        rodata: g.rodata,
        data_refs: g.data_refs,
        bss_refs: g.bss_refs,
        got_refs: g.got_refs,
        imports: IMPORTS.iter().map(|s| s.to_string()).collect(),
        symbols,
        entry,
        bss_size: BUF_CAP + 8,
    })
}
