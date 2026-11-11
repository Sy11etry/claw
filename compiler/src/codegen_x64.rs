//! CIR to x86-64 (macOS, System V AMD64 ABI for everything that touches C).
//!
//! Like the first arm64 back end this is deliberately naive: every virtual register is a
//! stack slot at `[rbp - 8 * (v + 1)]`, each instruction loads its operands into rax/rcx/rdx,
//! computes, and stores the result back. Optimized CIR gets the same treatment.
//!
//! Internal calling convention: every argument (floats as raw bits, aggregates as addresses)
//! is stored in the caller's outgoing area at `[rsp + 8 * i]`; the callee finds it at
//! `[rbp + 16 + 8 * i]`. The result comes back in rax. Calls to C follow System V instead.
//! The stack pointer is 16-byte aligned everywhere except between a `call` and the callee's
//! `push rbp`.

use crate::cir::*;
use crate::x64::*;

const IMPORTS: &[&str] = &["_write", "_exit", "_snprintf", "_memcpy", "_malloc", "_free"];
const IMP_WRITE: usize = 0;
const IMP_EXIT: usize = 1;
const IMP_SNPRINTF: usize = 2;
const IMP_MEMCPY: usize = 3;
const IMP_MALLOC: usize = 4;
const IMP_FREE: usize = 5;

// The runtime's words live in bss and mirror codegen.rs: same offsets, same sizes.
/// Output buffer: 8 bytes of length, then the bytes.
const BUF_CAP: u64 = 1 << 16;
/// Stay well inside the default 8 MB main-thread stack.
const MAX_FRAME: u64 = 7 << 20;
/// Set while the buffer is collecting a string rather than output.
const CAPTURE_FLAG: u64 = BUF_CAP + 8;
/// Counters for `--alloc-stats`, the leak check used by the test suite.
const ALLOC_COUNT: u64 = BUF_CAP + 16;
const FREE_COUNT: u64 = BUF_CAP + 24;
/// Words shared with the contract guard and the memory checker.
const STATE: u64 = BUF_CAP + 32;
const STATE_SIZE: u64 = 128;
const SITE_WORD: u64 = 8;

const INT_ARGS: [Reg; 6] = [RDI, RSI, RDX, RCX, R8, R9];

#[derive(Clone, Copy)]
enum Target {
    User(FuncId),
    Trap,
    RtAlloc,
    RtFree,
    RtTake,
    Flush,
    BufStr,
    BufInt,
    BufUint,
    BufBool,
    BufFloat,
    Import(usize),
}

pub struct ObjectX64 {
    pub code: Vec<u8>,
    pub rodata: Vec<u8>,
    /// (position of a rip-relative disp32 in `code`, offset into rodata)
    pub data_refs: Vec<(usize, usize)>,
    /// (position of a disp32, offset into bss)
    pub bss_refs: Vec<(usize, u64)>,
    /// (position of a disp32, GOT slot)
    pub got_refs: Vec<(usize, usize)>,
    pub imports: Vec<String>,
    /// Symbol name and code position in bytes.
    pub symbols: Vec<(String, usize)>,
    pub entry: usize,
    pub bss_size: u64,
}

struct Gen {
    c: Vec<u8>,
    rodata: Vec<u8>,
    data_refs: Vec<(usize, usize)>,
    bss_refs: Vec<(usize, u64)>,
    got_refs: Vec<(usize, usize)>,
    /// rel32 fields waiting for a call or jump target.
    calls: Vec<(usize, Target)>,
    /// `FuncAddr`: position of the `lea`'s disp32, and the function.
    func_refs: Vec<(usize, FuncId)>,
    files: Vec<String>,
    imports: Vec<String>,
    too_large: Option<String>,
}

/// Where one C argument travels.
enum Slot {
    Int(Reg),
    Flt(u8),
    Stack(u32),
}

/// System V: six integer and eight vector registers, then 8-byte stack slots in order.
fn assign_c(args: &[CArg]) -> (Vec<Slot>, u32) {
    let (mut ni, mut nf, mut stack) = (0usize, 0u8, 0u32);
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        if a.float.is_some() && nf < 8 {
            out.push(Slot::Flt(nf));
            nf += 1;
        } else if a.float.is_none() && ni < 6 {
            out.push(Slot::Int(INT_ARGS[ni]));
            ni += 1;
        } else {
            out.push(Slot::Stack(stack));
            stack += 8;
        }
    }
    (out, stack)
}

fn int_cond(cc: Cc, unsigned: bool) -> Cond {
    match (cc, unsigned) {
        (Cc::Eq, _) => Cond::E,
        (Cc::Ne, _) => Cond::Ne,
        (Cc::Lt, false) => Cond::L,
        (Cc::Le, false) => Cond::Le,
        (Cc::Gt, false) => Cond::G,
        (Cc::Ge, false) => Cond::Ge,
        (Cc::Lt, true) => Cond::B,
        (Cc::Le, true) => Cond::Be,
        (Cc::Gt, true) => Cond::A,
        (Cc::Ge, true) => Cond::Ae,
    }
}

#[derive(Clone, Copy)]
enum Trap {
    Overflow,
    DivZero,
    Shift,
    Bounds,
    Convert,
    AllocSize,
    OutOfMemory,
    ContractMath,
}

impl Trap {
    fn text(self) -> &'static str {
        match self {
            Trap::Overflow => "integer overflow",
            Trap::DivZero => "division by zero",
            Trap::Shift => "shift out of range",
            Trap::Bounds => "index out of bounds",
            Trap::Convert => "value does not fit the target type",
            Trap::AllocSize => "invalid allocation size",
            Trap::OutOfMemory => "out of memory",
            Trap::ContractMath => "a number in a contract is too large even for exact arithmetic",
        }
    }
}

struct Frame {
    /// Distance below rbp of each stack object's lowest byte.
    objects: Vec<i32>,
}

/// The stack slot of a virtual register.
fn slot(v: VReg) -> Op {
    at(RBP, -(v as i32 + 1) * 8)
}

type Traps = Vec<(usize, Trap, Loc)>;

impl Gen {
    fn here(&self) -> usize {
        self.c.len()
    }

    // ---- labels and references ----

    fn call_to(&mut self, t: Target) {
        call(&mut self.c);
        self.calls.push((self.c.len() - 4, t));
    }

    fn jmp_to(&mut self, t: Target) {
        jmp(&mut self.c);
        self.calls.push((self.c.len() - 4, t));
    }

    /// A forward conditional jump; `bind` settles where it lands.
    fn jcc_fwd(&mut self, cc: Cond) -> usize {
        jcc(&mut self.c, cc);
        self.c.len() - 4
    }

    fn jmp_fwd(&mut self) -> usize {
        jmp(&mut self.c);
        self.c.len() - 4
    }

    fn bind(&mut self, at: usize) {
        let here = self.here();
        patch_rel(&mut self.c, at, here);
    }

    /// A backward jump to a known position.
    fn jcc_back(&mut self, cc: Cond, target: usize) {
        jcc(&mut self.c, cc);
        let at = self.c.len() - 4;
        patch_rel(&mut self.c, at, target);
    }

    fn jmp_back(&mut self, target: usize) {
        jmp(&mut self.c);
        let at = self.c.len() - 4;
        patch_rel(&mut self.c, at, target);
    }

    fn lea_data(&mut self, r: Reg, off: usize) {
        lea(&mut self.c, r, Op::Rip);
        self.data_refs.push((self.c.len() - 4, off));
    }

    fn lea_bss(&mut self, r: Reg, off: u64) {
        lea(&mut self.c, r, Op::Rip);
        self.bss_refs.push((self.c.len() - 4, off));
    }

    fn add_str(&mut self, bytes: &[u8]) -> usize {
        let off = self.rodata.len();
        self.rodata.extend_from_slice(bytes);
        off
    }

    fn import_stub(&mut self, slot: usize) -> usize {
        let pos = self.here();
        jmp_rip(&mut self.c);
        self.got_refs.push((self.c.len() - 4, slot));
        pos
    }

    // ---- values ----

    fn ld(&mut self, r: Reg, v: VReg) {
        mov_load(&mut self.c, r, slot(v));
    }

    fn st(&mut self, v: VReg, r: Reg) {
        mov(&mut self.c, slot(v), r);
    }

    fn fld(&mut self, x: Xmm, v: VReg, bits: u8) {
        movs_load(&mut self.c, bits, x, slot(v));
    }

    fn fst(&mut self, v: VReg, x: Xmm, bits: u8) {
        movs_store(&mut self.c, bits, slot(v), x);
    }

    fn imm(&mut self, r: Reg, v: i64) {
        if v == 0 {
            mov_ri32(&mut self.c, r, 0);
        } else if (0..=u32::MAX as i64).contains(&v) {
            mov_ri32(&mut self.c, r, v as u32);
        } else if i32::try_from(v).is_ok() {
            mov_ri_sext(&mut self.c, r, v as i32);
        } else {
            mov_ri64(&mut self.c, r, v);
        }
    }

    fn trap_if(&mut self, cc: Cond, kind: Trap, loc: Loc, traps: &mut Traps) {
        let at = self.jcc_fwd(cc);
        traps.push((at, kind, loc));
    }

    // ---- runtime pieces ----

    /// push rbp; mov rbp, rsp; push the saved registers; keep rsp 16-byte aligned.
    fn prologue(&mut self, saved: &[Reg]) {
        push(&mut self.c, RBP);
        mov(&mut self.c, reg(RBP), RSP);
        for r in saved {
            push(&mut self.c, *r);
        }
        if saved.len() % 2 == 1 {
            alu_i(&mut self.c, Alu::Sub, reg(RSP), 8);
        }
    }

    fn epilogue(&mut self, saved: &[Reg]) {
        if !saved.is_empty() {
            lea(&mut self.c, RSP, at(RBP, -8 * saved.len() as i32));
            for r in saved.iter().rev() {
                pop(&mut self.c, *r);
            }
        }
        pop(&mut self.c, RBP);
        ret(&mut self.c);
    }

    /// `__claw_flush(edi = fd)` writes out the buffer, retrying short writes.
    fn rt_flush(&mut self) -> usize {
        let pos = self.here();
        let saved = [RBX, R12, R13];
        self.prologue(&saved);
        mov(&mut self.c, reg(RBX), RDI);
        // Filling up while building a string: there is nowhere to flush to.
        let msg = "trap: interpolated string is too long (64 KB)\n";
        let off = self.add_str(msg.as_bytes());
        self.lea_bss(RAX, CAPTURE_FLAG);
        mov_load(&mut self.c, RAX, at(RAX, 0));
        test(&mut self.c, RAX, RAX);
        let ok = self.jcc_fwd(Cond::E);
        self.lea_data(RDI, off);
        mov_ri32(&mut self.c, RSI, msg.len() as u32);
        self.call_to(Target::Trap);
        self.bind(ok);
        self.lea_bss(R12, 0);
        mov_load(&mut self.c, R13, at(R12, 0));
        mov_mi(&mut self.c, at(R12, 0), 0);
        alu_i(&mut self.c, Alu::Add, reg(R12), 8);
        let top = self.here();
        test(&mut self.c, R13, R13);
        let done = self.jcc_fwd(Cond::E);
        mov(&mut self.c, reg(RDI), RBX);
        mov(&mut self.c, reg(RSI), R12);
        mov(&mut self.c, reg(RDX), R13);
        self.call_to(Target::Import(IMP_WRITE));
        test(&mut self.c, RAX, RAX);
        let fail = self.jcc_fwd(Cond::Le);
        alu(&mut self.c, Alu::Add, reg(R12), RAX);
        alu(&mut self.c, Alu::Sub, reg(R13), RAX);
        self.jmp_back(top);
        self.bind(done);
        self.bind(fail);
        self.epilogue(&saved);
        pos
    }

    /// `__claw_buf_str(rdi = ptr, rsi = len)` appends to the output buffer.
    fn rt_buf_str(&mut self) -> usize {
        let pos = self.here();
        let saved = [RBX, R12, R13];
        self.prologue(&saved);
        mov(&mut self.c, reg(RBX), RDI);
        mov(&mut self.c, reg(R12), RSI);
        let top = self.here();
        test(&mut self.c, R12, R12);
        let done = self.jcc_fwd(Cond::E);
        self.lea_bss(R13, 0);
        mov_load(&mut self.c, RAX, at(R13, 0));
        mov_ri32(&mut self.c, RCX, BUF_CAP as u32);
        alu(&mut self.c, Alu::Sub, reg(RCX), RAX);
        let room = self.jcc_fwd(Cond::Ne);
        mov_ri32(&mut self.c, RDI, 1);
        self.call_to(Target::Flush);
        self.jmp_back(top);
        self.bind(room);
        // rcx = min(room, remaining)
        alu(&mut self.c, Alu::Cmp, reg(RCX), R12);
        cmov(&mut self.c, Cond::A, RCX, R12);
        mov(&mut self.c, reg(RDX), RCX);
        lea(&mut self.c, RDI, at_idx(R13, RAX, 1, 8));
        mov(&mut self.c, reg(RSI), RBX);
        alu(&mut self.c, Alu::Add, reg(RAX), RCX);
        mov(&mut self.c, at(R13, 0), RAX);
        rep_movsb(&mut self.c);
        mov(&mut self.c, reg(RBX), RSI);
        alu(&mut self.c, Alu::Sub, reg(R12), RDX);
        self.jmp_back(top);
        self.bind(done);
        self.epilogue(&saved);
        pos
    }

    /// Adds 1 to a counter; touches only r11.
    fn bump_counter(&mut self, off: u64) {
        self.lea_bss(R11, off);
        alu_i(&mut self.c, Alu::Add, at(R11, 0), 1);
    }

    fn report_alloc_stats(&mut self) {
        for (text, off) in [("allocs=", ALLOC_COUNT), (" frees=", FREE_COUNT)] {
            let s = self.add_str(text.as_bytes());
            self.lea_data(RDI, s);
            mov_ri32(&mut self.c, RSI, text.len() as u32);
            self.call_to(Target::BufStr);
            self.lea_bss(RAX, off);
            mov_load(&mut self.c, RDI, at(RAX, 0));
            self.call_to(Target::BufInt);
        }
        let s = self.add_str(b"\n");
        self.lea_data(RDI, s);
        mov_ri32(&mut self.c, RSI, 1);
        self.call_to(Target::BufStr);
        mov_ri32(&mut self.c, RDI, 2);
        self.call_to(Target::Flush);
    }

    /// `__claw_take()` hands back the buffer as heap bytes: rax = pointer, rdx = length.
    fn rt_take(&mut self) -> usize {
        let pos = self.here();
        let saved = [RBX, R12];
        self.prologue(&saved);
        self.lea_bss(RBX, 0);
        self.lea_bss(RAX, CAPTURE_FLAG);
        mov_mi(&mut self.c, at(RAX, 0), 0);
        mov_load(&mut self.c, R12, at(RBX, 0));
        mov_mi(&mut self.c, at(RBX, 0), 0);
        test(&mut self.c, R12, R12);
        let empty = self.jcc_fwd(Cond::E);
        mov(&mut self.c, reg(RDI), R12);
        self.call_to(Target::RtAlloc);
        test(&mut self.c, RAX, RAX);
        let got = self.jcc_fwd(Cond::Ne);
        ud2(&mut self.c);
        self.bind(got);
        mov(&mut self.c, reg(RDI), RAX);
        lea(&mut self.c, RSI, at(RBX, 8));
        mov(&mut self.c, reg(RDX), R12);
        self.call_to(Target::Import(IMP_MEMCPY));
        let join = self.jmp_fwd();
        // Empty: no allocation at all.
        self.bind(empty);
        mov_ri32(&mut self.c, RAX, 0);
        self.bind(join);
        mov(&mut self.c, reg(RDX), R12);
        self.epilogue(&saved);
        pos
    }

    /// `__claw_buf_int(rdi)`; digits are built backwards on the stack.
    fn rt_buf_num(&mut self, signed: bool) -> usize {
        let pos = self.here();
        self.prologue(&[]);
        alu_i(&mut self.c, Alu::Sub, reg(RSP), 48);
        mov(&mut self.c, reg(RAX), RDI);
        lea(&mut self.c, R9, at(RSP, 32));
        mov_ri32(&mut self.c, RCX, 10);
        if signed {
            test(&mut self.c, RDI, RDI);
            let skip = self.jcc_fwd(Cond::Ns);
            // The magnitude of i64::MIN wraps to 2^63, correct when read unsigned.
            neg(&mut self.c, RAX);
            self.bind(skip);
        }
        let top = self.here();
        mov_ri32(&mut self.c, RDX, 0);
        div(&mut self.c, reg(RCX));
        alu_i(&mut self.c, Alu::Add, reg(RDX), 48);
        alu_i(&mut self.c, Alu::Sub, reg(R9), 1);
        mov8(&mut self.c, at(R9, 0), RDX);
        test(&mut self.c, RAX, RAX);
        self.jcc_back(Cond::Ne, top);
        if signed {
            test(&mut self.c, RDI, RDI);
            let skip = self.jcc_fwd(Cond::Ns);
            mov_ri32(&mut self.c, RDX, 45);
            alu_i(&mut self.c, Alu::Sub, reg(R9), 1);
            mov8(&mut self.c, at(R9, 0), RDX);
            self.bind(skip);
        }
        mov(&mut self.c, reg(RDI), R9);
        lea(&mut self.c, RSI, at(RSP, 32));
        alu(&mut self.c, Alu::Sub, reg(RSI), R9);
        self.call_to(Target::BufStr);
        leave(&mut self.c);
        ret(&mut self.c);
        pos
    }

    /// `__claw_buf_bool(edi = 0 or 1)`
    fn rt_buf_bool(&mut self) -> usize {
        let pos = self.here();
        let f = self.add_str(b"false");
        let t = self.add_str(b"true");
        test(&mut self.c, RDI, RDI);
        let is_false = self.jcc_fwd(Cond::E);
        self.lea_data(RDI, t);
        mov_ri32(&mut self.c, RSI, 4);
        self.jmp_to(Target::BufStr);
        self.bind(is_false);
        self.lea_data(RDI, f);
        mov_ri32(&mut self.c, RSI, 5);
        self.jmp_to(Target::BufStr);
        pos
    }

    /// `__claw_buf_float(xmm0 = value, rdx = format)`, via libSystem's snprintf.
    fn rt_buf_float(&mut self) -> usize {
        let pos = self.here();
        self.prologue(&[]);
        alu_i(&mut self.c, Alu::Sub, reg(RSP), 80);
        lea(&mut self.c, RDI, at(RSP, 16));
        mov_ri32(&mut self.c, RSI, 64);
        mov_ri32(&mut self.c, RAX, 1); // one vector register carries an argument
        self.call_to(Target::Import(IMP_SNPRINTF));
        mov32(&mut self.c, reg(RSI), RAX);
        lea(&mut self.c, RDI, at(RSP, 16));
        self.call_to(Target::BufStr);
        leave(&mut self.c);
        ret(&mut self.c);
        pos
    }

    /// `__claw_trap(rdi = ptr, rsi = len)` reports to stderr and exits with 101.
    fn rt_trap(&mut self) -> usize {
        let pos = self.here();
        self.prologue(&[]);
        mov(&mut self.c, reg(RDX), RSI);
        mov(&mut self.c, reg(RSI), RDI);
        mov_ri32(&mut self.c, RDI, 2);
        self.call_to(Target::Import(IMP_WRITE));
        mov_ri32(&mut self.c, RDI, 101);
        self.call_to(Target::Import(IMP_EXIT));
        ud2(&mut self.c);
        pos
    }

    /// Entry for `alloc`/`free`: count, then hand over to malloc/free or to the memory checker.
    /// The checker is a Claw function, so its one argument goes where Claw functions expect it.
    fn rt_wrap(&mut self, counter: Option<u64>, import: usize, user: Option<FuncId>) -> usize {
        let pos = self.here();
        if let Some(off) = counter {
            self.bump_counter(off);
        }
        match user {
            None => self.jmp_to(Target::Import(import)),
            Some(f) => {
                self.prologue(&[]);
                alu_i(&mut self.c, Alu::Sub, reg(RSP), 16);
                mov(&mut self.c, at(RSP, 0), RDI);
                self.call_to(Target::User(f));
                leave(&mut self.c);
                ret(&mut self.c);
            }
        }
        pos
    }

    // ---- functions ----

    fn func(&mut self, f: &Func, fmts: (usize, usize)) {
        let mut outgoing = 0u32;
        for b in &f.blocks {
            for i in &b.insts {
                match i {
                    Inst::Call { args, .. } | Inst::CallInd { args, .. } => outgoing = outgoing.max(args.len() as u32 * 8),
                    Inst::CallC { args, .. } => outgoing = outgoing.max(assign_c(args).1),
                    _ => {}
                }
            }
        }
        let mut off = f.nvregs as u64 * 8;
        let mut objects = Vec::with_capacity(f.objects.len());
        for (size, align) in &f.objects {
            let align = (*align).max(1);
            off = (off + size + align - 1) & !(align - 1);
            objects.push(off as i32);
        }
        let size = (off + outgoing as u64 + 15) & !15;
        if size >= MAX_FRAME {
            self.too_large = Some(f.name.clone());
            return;
        }
        let fr = Frame { objects };

        push(&mut self.c, RBP);
        mov(&mut self.c, reg(RBP), RSP);
        if size > 0 {
            alu_i(&mut self.c, Alu::Sub, reg(RSP), size as i32);
        }
        for (i, p) in f.params.iter().enumerate() {
            mov_load(&mut self.c, RAX, at(RBP, 16 + 8 * i as i32));
            self.st(*p, RAX);
        }

        let mut block_pos = vec![0usize; f.blocks.len()];
        let mut pending: Vec<(usize, BlockId)> = Vec::new();
        let mut traps: Traps = Vec::new();

        // How often each register is read, to spot a comparison only the branch after it wants.
        let mut uses = vec![0u32; f.nvregs as usize];
        if f.optimized {
            for b in &f.blocks {
                for i in &b.insts {
                    i.each(&mut |v, def| {
                        if !def {
                            uses[v as usize] += 1;
                        }
                    });
                }
                match &b.term {
                    Term::Br { c, .. } => uses[*c as usize] += 1,
                    Term::Ret(Some(v)) => uses[*v as usize] += 1,
                    _ => {}
                }
            }
        }

        for (idx, b) in f.blocks.iter().enumerate() {
            block_pos[idx] = self.here();
            let fused = matches!((&b.term, b.insts.last()), (Term::Br { c, .. }, Some(Inst::Cmp { d, .. })) if f.optimized && c == d && uses[*c as usize] == 1);
            let count = b.insts.len() - fused as usize;
            for i in &b.insts[..count] {
                self.inst(&fr, i, &mut traps, fmts);
            }
            let next = idx + 1;
            if fused {
                let Term::Br { t, f: fb, .. } = b.term else { unreachable!() };
                let Some(Inst::Cmp { cc, a, b: rhs, unsigned, .. }) = b.insts.last() else { unreachable!() };
                self.ld(RAX, *a);
                alu_load(&mut self.c, Alu::Cmp, RAX, slot(*rhs));
                let cond = int_cond(*cc, *unsigned);
                if t == next {
                    let at = self.jcc_fwd(cond.not());
                    pending.push((at, fb));
                } else {
                    let at = self.jcc_fwd(cond);
                    pending.push((at, t));
                    if fb != next {
                        let at = self.jmp_fwd();
                        pending.push((at, fb));
                    }
                }
                continue;
            }
            match &b.term {
                Term::Jmp(t) => {
                    if *t != next {
                        let at = self.jmp_fwd();
                        pending.push((at, *t));
                    }
                }
                Term::Br { c, t, f: fb } => {
                    alu_i(&mut self.c, Alu::Cmp, slot(*c), 0);
                    if *t == next {
                        let at = self.jcc_fwd(Cond::E);
                        pending.push((at, *fb));
                    } else {
                        let at = self.jcc_fwd(Cond::Ne);
                        pending.push((at, *t));
                        if *fb != next {
                            let at = self.jmp_fwd();
                            pending.push((at, *fb));
                        }
                    }
                }
                Term::Ret(v) => {
                    if let Some(v) = v {
                        self.ld(RAX, *v);
                    }
                    leave(&mut self.c);
                    ret(&mut self.c);
                }
                Term::Unreachable => ud2(&mut self.c),
            }
        }

        for (at, target) in pending {
            patch_rel(&mut self.c, at, block_pos[target]);
        }

        // Cold paths go after the function so the hot path falls straight through.
        for (at, kind, loc) in traps {
            self.bind(at);
            let msg = format!("trap: {} at {}:{}:{}\n", kind.text(), self.files[loc.file as usize], loc.line, loc.col);
            let off = self.add_str(msg.as_bytes());
            self.lea_data(RDI, off);
            mov_ri32(&mut self.c, RSI, msg.len() as u32);
            self.call_to(Target::Trap);
        }
    }

    /// Traps if rax is outside the signed range of `num`, which is narrower than 64 bits.
    fn fit_check(&mut self, num: Num, loc: Loc, traps: &mut Traps) {
        self.sign_extend(R11, RAX, num.bits);
        alu(&mut self.c, Alu::Cmp, reg(R11), RAX);
        self.trap_if(Cond::Ne, Trap::Overflow, loc, traps);
    }

    /// `dst = sign-extended low `bits` of `src``.
    fn sign_extend(&mut self, dst: Reg, src: Reg, bits: u8) {
        match bits {
            8 => movsx8(&mut self.c, dst, reg(src)),
            16 => movsx16(&mut self.c, dst, reg(src)),
            32 => movsx32(&mut self.c, dst, reg(src)),
            _ => {
                if dst != src {
                    mov(&mut self.c, reg(dst), src);
                }
                shift_i(&mut self.c, Shift::Shl, dst, 64 - bits);
                shift_i(&mut self.c, Shift::Sar, dst, 64 - bits);
            }
        }
    }

    /// `dst = zero-extended low `bits` of `src``.
    fn zero_extend(&mut self, dst: Reg, src: Reg, bits: u8) {
        match bits {
            8 => movzx8(&mut self.c, dst, reg(src)),
            16 => movzx16(&mut self.c, dst, reg(src)),
            32 => mov32(&mut self.c, reg(dst), src),
            _ => {
                if dst != src {
                    mov(&mut self.c, reg(dst), src);
                }
                shift_i(&mut self.c, Shift::Shl, dst, 64 - bits);
                shift_i(&mut self.c, Shift::Shr, dst, 64 - bits);
            }
        }
    }

    fn mask_unsigned(&mut self, num: Num) {
        if !num.signed && num.bits < 64 {
            self.zero_extend(RAX, RAX, num.bits);
        }
    }

    /// After an operation on 64-bit registers: narrow signed types trap, unsigned wrap.
    fn normalize(&mut self, num: Num, loc: Loc, traps: &mut Traps) {
        if num.signed {
            if num.bits < 64 && num.checked {
                self.fit_check(num, loc, traps);
            }
        } else {
            self.mask_unsigned(num);
        }
    }

    fn int_cast(&mut self, from: Num, to: Num, loc: Loc, traps: &mut Traps) {
        // The value is in rax, the result goes back to rax.
        if to.signed && !to.checked {
            return;
        }
        if to.signed {
            if !from.signed && from.bits == 64 {
                // A u64 with the top bit set is negative when read as signed.
                test(&mut self.c, RAX, RAX);
                self.trap_if(Cond::S, Trap::Convert, loc, traps);
            }
            if to.bits < 64 {
                self.sign_extend(R11, RAX, to.bits);
                alu(&mut self.c, Alu::Cmp, reg(R11), RAX);
                self.trap_if(Cond::Ne, Trap::Convert, loc, traps);
            }
        } else if to.bits < 64 {
            self.zero_extend(RAX, RAX, to.bits);
        }
    }

    /// Traps unless xmm0 converts to `to`: not NaN and within range after truncation.
    fn float_range_check(&mut self, to: Num, loc: Loc, traps: &mut Traps) {
        ucomis(&mut self.c, 64, 0, reg(0));
        self.trap_if(Cond::P, Trap::Convert, loc, traps);
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
        mov_ri64(&mut self.c, RCX, hi.to_bits() as i64);
        movq_xr(&mut self.c, 1, RCX);
        ucomis(&mut self.c, 64, 0, reg(1));
        self.trap_if(Cond::Ae, Trap::Convert, loc, traps);
        mov_ri64(&mut self.c, RCX, lo.to_bits() as i64);
        movq_xr(&mut self.c, 1, RCX);
        ucomis(&mut self.c, 64, 0, reg(1));
        self.trap_if(if lo_inclusive { Cond::B } else { Cond::Be }, Trap::Convert, loc, traps);
    }

    /// Is `off` something an addressing mode can carry?
    fn small(off: u64) -> Option<i32> {
        i32::try_from(off).ok()
    }

    /// An operand for `[rax + off]`; far offsets are added to rax first.
    fn mem_at_rax(&mut self, off: u64) -> Op {
        match Self::small(off) {
            Some(o) => at(RAX, o),
            None => {
                mov_ri64(&mut self.c, RCX, off as i64);
                alu(&mut self.c, Alu::Add, reg(RAX), RCX);
                at(RAX, 0)
            }
        }
    }

    fn inst(&mut self, fr: &Frame, i: &Inst, traps: &mut Traps, fmts: (usize, usize)) {
        match i {
            Inst::Const { d, v } => {
                if let Ok(small) = i32::try_from(*v) {
                    mov_mi(&mut self.c, slot(*d), small);
                } else {
                    mov_ri64(&mut self.c, RAX, *v);
                    self.st(*d, RAX);
                }
            }
            Inst::Mov { d, s } => {
                self.ld(RAX, *s);
                self.st(*d, RAX);
            }
            Inst::Bin { op, d, a, b, num, loc } => {
                self.ld(RAX, *a);
                self.ld(RCX, *b);
                self.bin(*op, *num, *loc, traps);
                self.st(*d, RAX);
            }
            Inst::Neg { d, s, num, loc } => {
                self.ld(RAX, *s);
                neg(&mut self.c, RAX);
                if num.checked {
                    if num.bits == 64 {
                        self.trap_if(Cond::O, Trap::Overflow, *loc, traps);
                    } else {
                        self.fit_check(*num, *loc, traps);
                    }
                }
                self.st(*d, RAX);
            }
            Inst::BitNot { d, s, num } => {
                self.ld(RAX, *s);
                not(&mut self.c, RAX);
                self.mask_unsigned(*num);
                self.st(*d, RAX);
            }
            Inst::BoolNot { d, s } => {
                self.ld(RAX, *s);
                alu_i(&mut self.c, Alu::Xor, reg(RAX), 1);
                self.st(*d, RAX);
            }
            Inst::Phi { .. } | Inst::Pi { .. } => unreachable!("gone before code generation"),
            Inst::ConstRef { .. } | Inst::Line { .. } | Inst::Note { .. } => unreachable!("only the interpreter sees these"),
            Inst::DataAddr { d, bytes } => {
                let off = self.add_str(bytes);
                self.lea_data(RAX, off);
                self.st(*d, RAX);
            }
            Inst::Cmp { cc, d, a, b, unsigned } => {
                self.ld(RAX, *a);
                alu_load(&mut self.c, Alu::Cmp, RAX, slot(*b));
                mov_ri32(&mut self.c, RAX, 0);
                setcc(&mut self.c, int_cond(*cc, *unsigned), RAX);
                self.st(*d, RAX);
            }
            Inst::FBin { op, d, a, b, bits } => {
                self.fld(0, *a, *bits);
                let o = match op {
                    FArith::Add => Sse::Add,
                    FArith::Sub => Sse::Sub,
                    FArith::Mul => Sse::Mul,
                    FArith::Div => Sse::Div,
                };
                sse(&mut self.c, o, *bits, 0, slot(*b));
                self.fst(*d, 0, *bits);
            }
            Inst::FNeg { d, s, bits } => {
                self.ld(RAX, *s);
                if *bits == 64 {
                    mov_ri64(&mut self.c, RCX, i64::MIN);
                } else {
                    mov_ri32(&mut self.c, RCX, 0x8000_0000);
                }
                alu(&mut self.c, Alu::Xor, reg(RAX), RCX);
                self.st(*d, RAX);
            }
            Inst::FCmp { cc, d, a, b, bits } => {
                // ucomis sets ZF, PF and CF all to 1 for NaN, so the conditions below that
                // test CF or ZF alone come out false for NaN, except `!=`.
                let (x, y) = if matches!(cc, Cc::Lt | Cc::Le) { (*b, *a) } else { (*a, *b) };
                mov_ri32(&mut self.c, RAX, 0);
                mov_ri32(&mut self.c, RCX, 0);
                self.fld(0, x, *bits);
                ucomis(&mut self.c, *bits, 0, slot(y));
                match cc {
                    Cc::Gt | Cc::Lt => setcc(&mut self.c, Cond::A, RAX),
                    Cc::Ge | Cc::Le => setcc(&mut self.c, Cond::Ae, RAX),
                    Cc::Eq => {
                        setcc(&mut self.c, Cond::E, RAX);
                        setcc(&mut self.c, Cond::Np, RCX);
                        alu(&mut self.c, Alu::And, reg(RAX), RCX);
                    }
                    Cc::Ne => {
                        setcc(&mut self.c, Cond::Ne, RAX);
                        setcc(&mut self.c, Cond::P, RCX);
                        alu(&mut self.c, Alu::Or, reg(RAX), RCX);
                    }
                }
                self.st(*d, RAX);
            }
            Inst::IntCast { d, s, from, to, loc } => {
                self.ld(RAX, *s);
                self.int_cast(*from, *to, *loc, traps);
                self.st(*d, RAX);
            }
            Inst::IntToFloat { d, s, signed, bits } => {
                self.ld(RAX, *s);
                if *signed {
                    cvtsi2s(&mut self.c, *bits, 0, RAX);
                } else {
                    // A u64 with the top bit set: halve it (keeping the low bit so rounding
                    // stays correct), convert, double.
                    test(&mut self.c, RAX, RAX);
                    let big = self.jcc_fwd(Cond::S);
                    cvtsi2s(&mut self.c, *bits, 0, RAX);
                    let done = self.jmp_fwd();
                    self.bind(big);
                    mov(&mut self.c, reg(RCX), RAX);
                    shift_i(&mut self.c, Shift::Shr, RCX, 1);
                    alu_i(&mut self.c, Alu::And, reg(RAX), 1);
                    alu(&mut self.c, Alu::Or, reg(RCX), RAX);
                    cvtsi2s(&mut self.c, *bits, 0, RCX);
                    sse(&mut self.c, Sse::Add, *bits, 0, reg(0));
                    self.bind(done);
                }
                self.fst(*d, 0, *bits);
            }
            Inst::FloatToInt { d, s, bits, to, loc } => {
                self.fld(0, *s, *bits);
                if *bits == 32 {
                    cvtss2sd(&mut self.c, 0, 0);
                }
                if to.checked {
                    self.float_range_check(*to, *loc, traps);
                }
                if to.signed || to.bits < 64 {
                    cvtts2si(&mut self.c, 64, RAX, 0);
                } else {
                    // u64: values from 2^63 up need the top bit put back by hand.
                    mov_ri64(&mut self.c, RCX, 2f64.powi(63).to_bits() as i64);
                    movq_xr(&mut self.c, 1, RCX);
                    ucomis(&mut self.c, 64, 0, reg(1));
                    let big = self.jcc_fwd(Cond::Ae);
                    cvtts2si(&mut self.c, 64, RAX, 0);
                    let done = self.jmp_fwd();
                    self.bind(big);
                    sse(&mut self.c, Sse::Sub, 64, 0, reg(1));
                    cvtts2si(&mut self.c, 64, RAX, 0);
                    mov_ri64(&mut self.c, RCX, i64::MIN);
                    alu(&mut self.c, Alu::Xor, reg(RAX), RCX);
                    self.bind(done);
                }
                self.st(*d, RAX);
            }
            Inst::FloatCast { d, s, to_bits } => {
                if *to_bits == 64 {
                    self.fld(0, *s, 32);
                    cvtss2sd(&mut self.c, 0, 0);
                } else {
                    self.fld(0, *s, 64);
                    cvtsd2ss(&mut self.c, 0, 0);
                }
                self.fst(*d, 0, *to_bits);
            }
            Inst::Call { d, f, args } => {
                for (n, a) in args.iter().enumerate() {
                    self.ld(RAX, *a);
                    mov(&mut self.c, at(RSP, 8 * n as i32), RAX);
                }
                self.call_to(Target::User(*f));
                if let Some(d) = d {
                    self.st(*d, RAX);
                }
            }
            Inst::CallInd { d, callee, args } => {
                for (n, a) in args.iter().enumerate() {
                    self.ld(RAX, *a);
                    mov(&mut self.c, at(RSP, 8 * n as i32), RAX);
                }
                self.ld(R11, *callee);
                call_r(&mut self.c, R11);
                if let Some(d) = d {
                    self.st(*d, RAX);
                }
            }
            Inst::FuncAddr { d, f } => {
                lea(&mut self.c, RAX, Op::Rip);
                self.func_refs.push((self.c.len() - 4, *f));
                self.st(*d, RAX);
            }
            Inst::ObjAddr { d, obj } => {
                lea(&mut self.c, RAX, at(RBP, -fr.objects[*obj]));
                self.st(*d, RAX);
            }
            Inst::AddImm { d, s, imm } => {
                self.ld(RAX, *s);
                match i32::try_from(*imm as i64) {
                    Ok(k) if *imm < (1 << 31) => alu_i(&mut self.c, Alu::Add, reg(RAX), k),
                    _ => {
                        mov_ri64(&mut self.c, RCX, *imm as i64);
                        alu(&mut self.c, Alu::Add, reg(RAX), RCX);
                    }
                }
                self.st(*d, RAX);
            }
            Inst::Load { d, p, off, mem } => {
                self.ld(RAX, *p);
                let m = self.mem_at_rax(*off);
                match mem {
                    Mem::I8 => movsx8(&mut self.c, RAX, m),
                    Mem::I16 => movsx16(&mut self.c, RAX, m),
                    Mem::I32 => movsx32(&mut self.c, RAX, m),
                    Mem::U8 => movzx8(&mut self.c, RAX, m),
                    Mem::U16 => movzx16(&mut self.c, RAX, m),
                    Mem::U32 => mov32_load(&mut self.c, RAX, m),
                    Mem::X64 => mov_load(&mut self.c, RAX, m),
                }
                self.st(*d, RAX);
            }
            Inst::Store { p, off, s, mem } => {
                self.ld(RAX, *p);
                self.ld(RDX, *s);
                let m = self.mem_at_rax(*off);
                match mem {
                    Mem::I8 | Mem::U8 => mov8(&mut self.c, m, RDX),
                    Mem::I16 | Mem::U16 => mov16(&mut self.c, m, RDX),
                    Mem::I32 | Mem::U32 => mov32(&mut self.c, m, RDX),
                    Mem::X64 => mov(&mut self.c, m, RDX),
                }
            }
            Inst::MemCopy { dst, src, size } => self.mem_copy(*dst, *src, *size),
            Inst::MemCopyDyn { dst, src, n } => {
                self.ld(RDI, *dst);
                self.ld(RSI, *src);
                self.ld(RDX, *n);
                self.call_to(Target::Import(IMP_MEMCPY));
            }
            Inst::BoundsDyn { idx, len, loc } => {
                self.ld(RAX, *idx);
                alu_load(&mut self.c, Alu::Cmp, RAX, slot(*len));
                // Unsigned compare: a negative index looks huge.
                self.trap_if(Cond::Ae, Trap::Bounds, *loc, traps);
            }
            Inst::BufPtr { p, n } => {
                self.ld(RDI, *p);
                self.ld(RSI, *n);
                self.call_to(Target::BufStr);
            }
            Inst::CaptureStart => {
                self.lea_bss(RAX, CAPTURE_FLAG);
                mov_mi(&mut self.c, at(RAX, 0), 1);
            }
            Inst::CaptureTake { ptr, len } => {
                self.call_to(Target::RtTake);
                self.st(*ptr, RAX);
                self.st(*len, RDX);
            }
            Inst::IndexAddr { d, base, idx, size } => {
                self.ld(RAX, *base);
                self.ld(RCX, *idx);
                if matches!(size, 1 | 2 | 4 | 8) {
                    lea(&mut self.c, RAX, at_idx(RAX, RCX, *size as u8, 0));
                } else {
                    self.imm(RDX, *size as i64);
                    imul(&mut self.c, RCX, reg(RDX));
                    alu(&mut self.c, Alu::Add, reg(RAX), RCX);
                }
                self.st(*d, RAX);
            }
            Inst::Bounds { idx, len, loc } => {
                self.ld(RAX, *idx);
                match i32::try_from(*len) {
                    Ok(k) => alu_i(&mut self.c, Alu::Cmp, reg(RAX), k),
                    Err(_) => {
                        mov_ri64(&mut self.c, RCX, *len as i64);
                        alu(&mut self.c, Alu::Cmp, reg(RAX), RCX);
                    }
                }
                // Unsigned compare: a negative index looks huge.
                self.trap_if(Cond::Ae, Trap::Bounds, *loc, traps);
            }
            Inst::Alloc { d, n, elem, loc } => {
                self.ld(RAX, *n);
                test(&mut self.c, RAX, RAX);
                self.trap_if(Cond::S, Trap::AllocSize, *loc, traps);
                self.imm(RCX, *elem as i64);
                mul1(&mut self.c, reg(RCX));
                test(&mut self.c, RDX, RDX);
                self.trap_if(Cond::Ne, Trap::AllocSize, *loc, traps);
                mov(&mut self.c, reg(RDI), RAX);
                self.call_to(Target::RtAlloc);
                test(&mut self.c, RAX, RAX);
                self.trap_if(Cond::E, Trap::OutOfMemory, *loc, traps);
                self.st(*d, RAX);
            }
            Inst::Free { p } => {
                self.ld(RDI, *p);
                self.call_to(Target::RtFree);
            }
            Inst::StrAddr { d, text } => {
                let mut bytes = text.as_bytes().to_vec();
                bytes.push(0);
                let off = self.add_str(&bytes);
                self.lea_data(RAX, off);
                self.st(*d, RAX);
            }
            Inst::CallC { d, sym, args, variadic, ret } => self.call_c(*d, sym, args, *variadic, *ret),
            Inst::BufStr { text } => {
                let off = self.add_str(text.as_bytes());
                self.lea_data(RDI, off);
                self.imm(RSI, text.len() as i64);
                self.call_to(Target::BufStr);
            }
            Inst::BufInt { s, signed } => {
                self.ld(RDI, *s);
                self.call_to(if *signed { Target::BufInt } else { Target::BufUint });
            }
            Inst::BufBool { s } => {
                self.ld(RDI, *s);
                self.call_to(Target::BufBool);
            }
            Inst::BufFloat { s, bits } => {
                self.fld(0, *s, *bits);
                if *bits == 32 {
                    cvtss2sd(&mut self.c, 0, 0);
                }
                self.lea_data(RDX, if *bits == 64 { fmts.0 } else { fmts.1 });
                self.call_to(Target::BufFloat);
            }
            Inst::WideBin { op, dlo, dhi, alo, ahi, blo, bhi, loc } => {
                self.ld(R8, *alo);
                self.ld(R9, *ahi);
                self.ld(R10, *blo);
                self.ld(R11, *bhi);
                match op {
                    WideOp::Add | WideOp::Sub => {
                        let (lo, hi) = if matches!(op, WideOp::Add) { (Alu::Add, Alu::Adc) } else { (Alu::Sub, Alu::Sbb) };
                        mov(&mut self.c, reg(RAX), R8);
                        mov(&mut self.c, reg(RDX), R9);
                        alu(&mut self.c, lo, reg(RAX), R10);
                        alu(&mut self.c, hi, reg(RDX), R11);
                        self.trap_if(Cond::O, Trap::ContractMath, *loc, traps);
                    }
                    WideOp::Mul => {
                        // Exact only while both sides fit in 64 signed bits.
                        mov(&mut self.c, reg(RAX), R8);
                        shift_i(&mut self.c, Shift::Sar, RAX, 63);
                        alu(&mut self.c, Alu::Cmp, reg(RAX), R9);
                        self.trap_if(Cond::Ne, Trap::ContractMath, *loc, traps);
                        mov(&mut self.c, reg(RAX), R10);
                        shift_i(&mut self.c, Shift::Sar, RAX, 63);
                        alu(&mut self.c, Alu::Cmp, reg(RAX), R11);
                        self.trap_if(Cond::Ne, Trap::ContractMath, *loc, traps);
                        mov(&mut self.c, reg(RAX), R8);
                        imul1(&mut self.c, reg(R10));
                    }
                }
                self.st(*dlo, RAX);
                self.st(*dhi, RDX);
            }
            Inst::WideCmp { cc, d, alo, ahi, blo, bhi } => {
                // `a > b` is `b < a`: only L and GE are valid after cmp + sbb.
                let (x, y) = if matches!(cc, Cc::Gt | Cc::Le) { ((*blo, *bhi), (*alo, *ahi)) } else { ((*alo, *ahi), (*blo, *bhi)) };
                self.ld(R8, x.0);
                self.ld(R9, x.1);
                self.ld(R10, y.0);
                self.ld(R11, y.1);
                let cond = match cc {
                    Cc::Eq | Cc::Ne => {
                        alu(&mut self.c, Alu::Xor, reg(R8), R10);
                        alu(&mut self.c, Alu::Xor, reg(R9), R11);
                        alu(&mut self.c, Alu::Or, reg(R8), R9);
                        if *cc == Cc::Eq { Cond::E } else { Cond::Ne }
                    }
                    _ => {
                        alu(&mut self.c, Alu::Cmp, reg(R8), R10);
                        alu(&mut self.c, Alu::Sbb, reg(R9), R11);
                        if matches!(cc, Cc::Lt | Cc::Gt) { Cond::L } else { Cond::Ge }
                    }
                };
                mov_ri32(&mut self.c, RAX, 0);
                setcc(&mut self.c, cond, RAX);
                self.st(*d, RAX);
            }
            Inst::RtState { d } => {
                self.lea_bss(RAX, STATE);
                self.st(*d, RAX);
            }
            Inst::SetSite { text } => {
                let mut bytes = text.as_bytes().to_vec();
                bytes.push(0);
                let off = self.add_str(&bytes);
                self.lea_data(RAX, off);
                self.lea_bss(RCX, STATE + SITE_WORD);
                mov(&mut self.c, at(RCX, 0), RAX);
            }
            Inst::Flush { fd } => {
                mov_ri32(&mut self.c, RDI, *fd);
                self.call_to(Target::Flush);
            }
            Inst::Exit { code } => {
                mov_ri32(&mut self.c, RDI, *code);
                self.call_to(Target::Import(IMP_EXIT));
            }
        }
    }

    fn call_c(&mut self, d: Option<VReg>, sym: &str, args: &[CArg], variadic: Option<usize>, ret: CRet) {
        let (slots, _) = assign_c(args);
        for (a, s) in args.iter().zip(&slots) {
            if let Slot::Stack(off) = s {
                self.ld(RAX, a.v);
                mov(&mut self.c, at(RSP, *off as i32), RAX);
            }
        }
        let mut nflt = 0;
        for (a, s) in args.iter().zip(&slots) {
            match s {
                Slot::Int(r) => self.ld(*r, a.v),
                Slot::Flt(x) => {
                    self.fld(*x, a.v, a.float.unwrap_or(64));
                    nflt += 1;
                }
                Slot::Stack(_) => {}
            }
        }
        if variadic.is_some() {
            // al says how many vector registers carry arguments.
            mov_ri32(&mut self.c, RAX, nflt);
        }
        let want = format!("_{sym}");
        let index = self.imports.iter().position(|n| *n == want).expect("imports were collected up front");
        self.call_to(Target::Import(index));
        // C leaves the upper bits of a narrow result undefined.
        match ret {
            CRet::None => {}
            CRet::Float(bits) => {
                if let Some(d) = d {
                    if bits == 64 {
                        movq_rx(&mut self.c, RAX, 0);
                    } else {
                        movd_rx(&mut self.c, RAX, 0);
                    }
                    self.st(d, RAX);
                }
            }
            CRet::Int(mem) => {
                match mem {
                    Mem::I8 => movsx8(&mut self.c, RAX, reg(RAX)),
                    Mem::I16 => movsx16(&mut self.c, RAX, reg(RAX)),
                    Mem::I32 => movsx32(&mut self.c, RAX, reg(RAX)),
                    Mem::U8 => movzx8(&mut self.c, RAX, reg(RAX)),
                    Mem::U16 => movzx16(&mut self.c, RAX, reg(RAX)),
                    Mem::U32 => mov32(&mut self.c, reg(RAX), RAX),
                    Mem::X64 => {}
                }
                if let Some(d) = d {
                    self.st(d, RAX);
                }
            }
        }
    }

    fn mem_copy(&mut self, dst: VReg, src: VReg, size: u64) {
        if size > 64 {
            self.ld(RDI, dst);
            self.ld(RSI, src);
            self.imm(RDX, size as i64);
            self.call_to(Target::Import(IMP_MEMCPY));
            return;
        }
        self.ld(RAX, dst);
        self.ld(RCX, src);
        let mut off = 0i32;
        while (size as i64 - off as i64) >= 8 {
            mov_load(&mut self.c, RDX, at(RCX, off));
            mov(&mut self.c, at(RAX, off), RDX);
            off += 8;
        }
        while (off as u64) < size {
            movzx8(&mut self.c, RDX, at(RCX, off));
            mov8(&mut self.c, at(RAX, off), RDX);
            off += 1;
        }
    }

    /// Operands are in rax and rcx; the result is left in rax. rdx and r11 are free to use.
    fn bin(&mut self, op: Arith, num: Num, loc: Loc, traps: &mut Traps) {
        let wide_signed = num.signed && num.bits == 64 && num.checked;
        match op {
            Arith::Add | Arith::Sub => {
                let o = if op == Arith::Add { Alu::Add } else { Alu::Sub };
                alu(&mut self.c, o, reg(RAX), RCX);
                if wide_signed {
                    self.trap_if(Cond::O, Trap::Overflow, loc, traps);
                } else {
                    self.normalize(num, loc, traps);
                }
            }
            Arith::Mul => {
                imul(&mut self.c, RAX, reg(RCX));
                if wide_signed {
                    self.trap_if(Cond::O, Trap::Overflow, loc, traps);
                } else {
                    self.normalize(num, loc, traps);
                }
            }
            Arith::Div => {
                if num.checked {
                    test(&mut self.c, RCX, RCX);
                    self.trap_if(Cond::E, Trap::DivZero, loc, traps);
                }
                if num.signed {
                    if num.bits == 64 && num.checked {
                        // MIN / -1 is the only quotient that does not fit in 64 bits.
                        // Negating the dividend overflows exactly when it is MIN.
                        alu_i(&mut self.c, Alu::Cmp, reg(RCX), -1);
                        let skip = self.jcc_fwd(Cond::Ne);
                        mov(&mut self.c, reg(RDX), RAX);
                        neg(&mut self.c, RDX);
                        self.trap_if(Cond::O, Trap::Overflow, loc, traps);
                        self.bind(skip);
                    }
                    cqo(&mut self.c);
                    idiv(&mut self.c, reg(RCX));
                    self.normalize(num, loc, traps);
                } else {
                    mov_ri32(&mut self.c, RDX, 0);
                    div(&mut self.c, reg(RCX));
                }
            }
            Arith::Rem => {
                if num.checked {
                    test(&mut self.c, RCX, RCX);
                    self.trap_if(Cond::E, Trap::DivZero, loc, traps);
                }
                if num.signed {
                    if num.bits == 64 {
                        // x % -1 is 0, but idiv faults on MIN / -1.
                        alu_i(&mut self.c, Alu::Cmp, reg(RCX), -1);
                        let normal = self.jcc_fwd(Cond::Ne);
                        mov_ri32(&mut self.c, RDX, 0);
                        let done = self.jmp_fwd();
                        self.bind(normal);
                        cqo(&mut self.c);
                        idiv(&mut self.c, reg(RCX));
                        self.bind(done);
                    } else {
                        cqo(&mut self.c);
                        idiv(&mut self.c, reg(RCX));
                    }
                } else {
                    mov_ri32(&mut self.c, RDX, 0);
                    div(&mut self.c, reg(RCX));
                }
                mov(&mut self.c, reg(RAX), RDX);
            }
            Arith::And => alu(&mut self.c, Alu::And, reg(RAX), RCX),
            Arith::Or => alu(&mut self.c, Alu::Or, reg(RAX), RCX),
            Arith::Xor => alu(&mut self.c, Alu::Xor, reg(RAX), RCX),
            Arith::Shl => {
                if num.checked {
                    alu_i(&mut self.c, Alu::Cmp, reg(RCX), num.bits as i32);
                    self.trap_if(Cond::Ae, Trap::Shift, loc, traps);
                }
                if wide_signed {
                    mov(&mut self.c, reg(R11), RAX);
                    shift_cl(&mut self.c, Shift::Shl, RAX);
                    mov(&mut self.c, reg(RDX), RAX);
                    shift_cl(&mut self.c, Shift::Sar, RDX);
                    alu(&mut self.c, Alu::Cmp, reg(RDX), R11);
                    self.trap_if(Cond::Ne, Trap::Overflow, loc, traps);
                } else {
                    shift_cl(&mut self.c, Shift::Shl, RAX);
                    self.normalize(num, loc, traps);
                }
            }
            Arith::Shr => {
                if num.checked {
                    alu_i(&mut self.c, Alu::Cmp, reg(RCX), num.bits as i32);
                    self.trap_if(Cond::Ae, Trap::Shift, loc, traps);
                }
                shift_cl(&mut self.c, if num.signed { Shift::Sar } else { Shift::Shr }, RAX);
            }
        }
    }
}

pub fn generate(m: &Module, files: &[String], alloc_stats: bool) -> Result<ObjectX64, String> {
    let mut imports: Vec<String> = IMPORTS.iter().map(|s| s.to_string()).collect();
    for f in m.funcs.iter().filter(|f| !f.dead) {
        for b in &f.blocks {
            for i in &b.insts {
                if let Inst::CallC { sym, .. } = i {
                    let name = format!("_{sym}");
                    if !imports.contains(&name) {
                        imports.push(name);
                    }
                }
            }
        }
    }
    let mut g = Gen {
        c: Vec::new(),
        rodata: Vec::new(),
        data_refs: Vec::new(),
        bss_refs: Vec::new(),
        got_refs: Vec::new(),
        calls: Vec::new(),
        func_refs: Vec::new(),
        files: files.to_vec(),
        imports: imports.clone(),
        too_large: None,
    };
    let mut symbols: Vec<(String, usize)> = Vec::new();

    let fmt64 = g.add_str(b"%.15g\0");
    let fmt32 = g.add_str(b"%.7g\0");

    let stubs: Vec<usize> = (0..imports.len()).map(|s| g.import_stub(s)).collect();
    // Every allocation passes through these two, so a later version can record them.
    let rt_alloc = g.rt_wrap(alloc_stats.then_some(ALLOC_COUNT), IMP_MALLOC, m.check.map(|c| c.alloc));
    let rt_free = g.rt_wrap(alloc_stats.then_some(FREE_COUNT), IMP_FREE, m.check.map(|c| c.free));
    let trap = g.rt_trap();
    let flush = g.rt_flush();
    let take = g.rt_take();
    let buf_str = g.rt_buf_str();
    let buf_int = g.rt_buf_num(true);
    let buf_uint = g.rt_buf_num(false);
    let buf_bool = g.rt_buf_bool();
    let buf_float = g.rt_buf_float();
    for (n, p) in [
        ("alloc", rt_alloc),
        ("free", rt_free),
        ("trap", trap),
        ("flush", flush),
        ("take", take),
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
    g.prologue(&[]);
    g.call_to(Target::User(m.main));
    if let Some(c) = m.check {
        g.call_to(Target::User(c.finish));
    }
    if alloc_stats {
        g.report_alloc_stats();
    }
    mov_ri32(&mut g.c, RAX, 0);
    g.epilogue(&[]);

    let mut func_pos = Vec::with_capacity(m.funcs.len());
    for f in &m.funcs {
        func_pos.push(g.here());
        if f.dead {
            continue;
        }
        symbols.push((format!("_{}", f.name), g.here()));
        g.func(f, (fmt64, fmt32));
    }

    for (pos, t) in std::mem::take(&mut g.calls) {
        let dest = match t {
            Target::User(f) => func_pos[f],
            Target::Trap => trap,
            Target::RtAlloc => rt_alloc,
            Target::RtFree => rt_free,
            Target::RtTake => take,
            Target::Flush => flush,
            Target::BufStr => buf_str,
            Target::BufInt => buf_int,
            Target::BufUint => buf_uint,
            Target::BufBool => buf_bool,
            Target::BufFloat => buf_float,
            Target::Import(i) => stubs[i],
        };
        patch_rel(&mut g.c, pos, dest);
    }
    for (pos, f) in std::mem::take(&mut g.func_refs) {
        patch_rel(&mut g.c, pos, func_pos[f]);
    }

    if let Some(name) = g.too_large {
        return Err(format!("E0901 function `{name}` is too large for this version: at most 1 MB of code and 7 MB of local data"));
    }

    Ok(ObjectX64 {
        code: g.c,
        rodata: g.rodata,
        data_refs: g.data_refs,
        bss_refs: g.bss_refs,
        got_refs: g.got_refs,
        imports,
        symbols,
        entry,
        bss_size: STATE + STATE_SIZE,
    })
}
