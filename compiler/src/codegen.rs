//! CIR to ARM64. Still deliberately naive: every virtual register is a stack
//! slot; each instruction loads its operands, computes, stores back.
//!
//! Internal calling convention: every value (floats as raw bits, aggregates as
//! addresses) goes in x0-x7, then on the stack. The result comes back in x0;
//! an aggregate result is written through a hidden first argument.

use crate::arm64::*;
use crate::cir::*;
use crate::regalloc::{self, Alloc, Place};

const IMPORTS: &[&str] = &["_write", "_exit", "_snprintf", "_memcpy", "_malloc", "_free"];
const IMP_WRITE: usize = 0;
const IMP_EXIT: usize = 1;
const IMP_SNPRINTF: usize = 2;
const IMP_MEMCPY: usize = 3;
const IMP_MALLOC: usize = 4;
const IMP_FREE: usize = 5;

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
    files: Vec<String>,
    imports: Vec<String>,
    too_large: Option<String>,
}

/// Where one C argument travels.
enum Slot {
    X(u32),
    D(u32),
    Stack(u32),
}

/// Apple's ARM64 rules: 8 integer and 8 float registers, then the stack with each
/// value at its natural size. Everything variadic goes on the stack in 8-byte slots.
fn assign_c(args: &[CArg], variadic: Option<usize>) -> (Vec<Slot>, u32) {
    let (mut x, mut d, mut stack) = (0u32, 0u32, 0u32);
    let mut out = Vec::with_capacity(args.len());
    for (i, a) in args.iter().enumerate() {
        if variadic.is_some_and(|v| i >= v) {
            stack = (stack + 7) & !7;
            out.push(Slot::Stack(stack));
            stack += 8;
        } else if a.float.is_some() && d < 8 {
            out.push(Slot::D(d));
            d += 1;
        } else if a.float.is_none() && x < 8 {
            out.push(Slot::X(x));
            x += 1;
        } else {
            let size = a.size.max(1);
            stack = (stack + size - 1) & !(size - 1);
            out.push(Slot::Stack(stack));
            stack += size;
        }
    }
    (out, stack)
}

fn int_cond(cc: Cc, unsigned: bool) -> Cond {
    match (cc, unsigned) {
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
    }
}

/// These conditions are all false for NaN, except `!=`.
fn float_cond(cc: Cc) -> Cond {
    match cc {
        Cc::Eq => Cond::Eq,
        Cc::Ne => Cond::Ne,
        Cc::Lt => Cond::Mi,
        Cc::Le => Cond::Ls,
        Cc::Gt => Cond::Gt,
        Cc::Ge => Cond::Ge,
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

enum Pending {
    B(usize, BlockId),
    Cond(usize, BlockId),
}

struct Frame {
    objects: Vec<u64>,
    alloc: Alloc,
    /// Where spilled values start in the frame.
    spill_base: u32,
    /// Where callee-saved registers are kept.
    save_base: u32,
}

impl Frame {
    fn place(&self, v: VReg) -> Place {
        self.alloc.places[v as usize]
    }

    fn slot(&self, n: u32) -> u32 {
        self.spill_base + n * 8
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

    /// A register holding `v`: its own if it has one, otherwise `scratch`.
    fn use_x(&mut self, f: &Frame, v: VReg, scratch: Reg) -> Reg {
        if let Place::Reg(x) = f.place(v) {
            return x;
        }
        self.load(f, scratch, v);
        scratch
    }

    /// Where to compute `v`; `done_x` then puts it in place.
    fn def_x(&self, f: &Frame, v: VReg, scratch: Reg) -> Reg {
        match f.place(v) {
            Place::Reg(x) => x,
            _ => scratch,
        }
    }

    fn done_x(&mut self, f: &Frame, r: Reg, v: VReg) {
        self.store(f, r, v);
    }

    fn use_d(&mut self, f: &Frame, v: VReg, scratch: Reg, bits: u8) -> Reg {
        if let Place::Fpr(d) = f.place(v) {
            return d;
        }
        self.fload(f, scratch, v, bits);
        scratch
    }

    fn def_d(&self, f: &Frame, v: VReg, scratch: Reg) -> Reg {
        match f.place(v) {
            Place::Fpr(d) => d,
            _ => scratch,
        }
    }

    /// `v` if it is a small non-negative constant an instruction can carry itself.
    fn small_const(f: &Frame, v: VReg) -> Option<u32> {
        match f.place(v) {
            Place::Const(c) if (0..4096).contains(&c) => Some(c as u32),
            _ => None,
        }
    }

    fn compare(&mut self, fr: &Frame, a: VReg, b: VReg) {
        let x = self.use_x(fr, a, X9);
        match Self::small_const(fr, b) {
            Some(k) => self.e(cmp_imm(x, k)),
            None => {
                let y = self.use_x(fr, b, X10);
                self.e(cmp(x, y));
            }
        }
    }

    fn fcompare(&mut self, fr: &Frame, a: VReg, b: VReg, bits: u8) {
        let x = self.use_d(fr, a, D0, bits);
        let y = self.use_d(fr, b, D1, bits);
        self.e(fcmp(x, y, bits));
    }

    fn obj_addr(&mut self, f: &Frame, r: Reg, obj: usize) {
        let off = f.objects[obj];
        if off < 4096 {
            self.e(add_imm(r, SP, off as u32));
        } else {
            self.mov_imm(X16, off as i64);
            self.e(add_ext(r, SP, X16));
        }
    }

    fn load(&mut self, f: &Frame, r: Reg, v: VReg) {
        match f.place(v) {
            Place::Reg(x) => {
                if x != r {
                    self.e(mov(r, x));
                }
            }
            Place::Fpr(d) => self.e(fmov_x_d(r, d)),
            Place::Const(c) => self.mov_imm(r, c),
            Place::Obj(o) => self.obj_addr(f, r, o),
            Place::Slot(n) => {
                let off = f.slot(n);
                if off <= 32760 {
                    self.e(ldr(r, SP, off));
                } else {
                    self.mov_imm(X16, off as i64);
                    self.e(ldr_reg(r, SP, X16));
                }
            }
            Place::None => self.e(movz(r, 0, 0)),
        }
    }

    fn store(&mut self, f: &Frame, r: Reg, v: VReg) {
        match f.place(v) {
            Place::Reg(x) => {
                if x != r {
                    self.e(mov(x, r));
                }
            }
            Place::Fpr(d) => self.e(fmov_d_x(d, r)),
            Place::Slot(n) => {
                let off = f.slot(n);
                if off <= 32760 {
                    self.e(str(r, SP, off));
                } else {
                    self.mov_imm(X16, off as i64);
                    self.e(str_reg(r, SP, X16));
                }
            }
            Place::Const(_) | Place::Obj(_) | Place::None => {}
        }
    }

    /// Far slots are reached through x16; returns the base register and offset.
    fn float_slot(&mut self, off: u32) -> (Reg, u32) {
        if off <= 16380 {
            (SP, off)
        } else {
            self.mov_imm(X16, off as i64);
            self.e(add_ext(X16, SP, X16));
            (X16, 0)
        }
    }

    fn fload(&mut self, f: &Frame, r: Reg, v: VReg, bits: u8) {
        match f.place(v) {
            Place::Fpr(d) => {
                if d != r {
                    self.e(fmov_d_d(r, d));
                }
            }
            Place::Reg(x) => self.e(fmov_d_x(r, x)),
            Place::Const(c) => {
                self.mov_imm(X16, c);
                self.e(fmov_d_x(r, X16));
            }
            Place::Slot(n) => {
                let (base, off) = self.float_slot(f.slot(n));
                self.e(if bits == 64 { ldr_d(r, base, off) } else { ldr_s(r, base, off) });
            }
            Place::Obj(_) | Place::None => self.e(fmov_d_x(r, XZR)),
        }
    }

    fn fstore(&mut self, f: &Frame, r: Reg, v: VReg, bits: u8) {
        match f.place(v) {
            Place::Fpr(d) => {
                if d != r {
                    self.e(fmov_d_d(d, r));
                }
            }
            Place::Reg(x) => self.e(fmov_x_d(x, r)),
            Place::Slot(n) => {
                let (base, off) = self.float_slot(f.slot(n));
                self.e(if bits == 64 { str_d(r, base, off) } else { str_s(r, base, off) });
            }
            Place::Const(_) | Place::Obj(_) | Place::None => {}
        }
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
        // Filling up while building a string: there is nowhere to flush to.
        let msg = "trap: interpolated string is too long (64 KB)\n";
        let off = self.add_str(msg.as_bytes());
        self.bss_addr(X9);
        self.mov_imm(X16, CAPTURE_FLAG as i64);
        self.e(add(X9, X9, X16));
        self.e(ldr(X10, X9, 0));
        self.e(cbz(X10, 5));
        self.data_addr(X0, off);
        self.e(movz(X1, msg.len() as u32, 0));
        self.call(B, Target::Trap);
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

    /// Adds 1 to a counter; touches only x9, x10 and x16.
    fn bump_counter(&mut self, off: u64) {
        self.bss_addr(X9);
        self.mov_imm(X16, off as i64);
        self.e(add(X9, X9, X16));
        self.e(ldr(X10, X9, 0));
        self.e(add_imm(X10, X10, 1));
        self.e(str(X10, X9, 0));
    }

    fn report_alloc_stats(&mut self) {
        for (text, off) in [("allocs=", ALLOC_COUNT), (" frees=", FREE_COUNT)] {
            let s = self.add_str(text.as_bytes());
            self.data_addr(X0, s);
            self.e(movz(X1, text.len() as u32, 0));
            self.call(BL, Target::BufStr);
            self.bss_addr(X9);
            self.mov_imm(X16, off as i64);
            self.e(add(X9, X9, X16));
            self.e(ldr(X0, X9, 0));
            self.call(BL, Target::BufInt);
        }
        let s = self.add_str(b"\n");
        self.data_addr(X0, s);
        self.e(movz(X1, 1, 0));
        self.call(BL, Target::BufStr);
        self.e(movz(X0, 2, 0));
        self.call(BL, Target::Flush);
    }

    /// `__claw_take()` hands back the buffer as heap bytes: x0 = pointer, x1 = length.
    fn rt_take(&mut self) -> usize {
        let pos = self.here();
        self.e(stp_pre(FP, LR, SP, -32));
        self.e(mov_sp(FP, SP));
        self.e(stp(X19, X20, SP, 16));
        self.bss_addr(X19);
        self.mov_imm(X16, CAPTURE_FLAG as i64);
        self.e(add(X9, X19, X16));
        self.e(str(XZR, X9, 0));
        self.e(ldr(X20, X19, 0));
        self.e(str(XZR, X19, 0));
        self.e(cbz(X20, 9));
        self.e(mov(X0, X20));
        self.call(BL, Target::RtAlloc);
        self.e(cbnz(X0, 2));
        self.e(brk(3));
        self.e(add_imm(X1, X19, 8));
        self.e(mov(X2, X20));
        self.call(BL, Target::Import(IMP_MEMCPY));
        self.e(b_at(B, 2));
        // Empty: no allocation at all.
        self.e(movz(X0, 0, 0));
        self.e(mov(X1, X20));
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
                if let Inst::CallC { args, variadic, .. } = i {
                    outgoing = outgoing.max((assign_c(args, *variadic).1 + 7) & !7);
                }
            }
        }
        let alloc = if f.optimized { regalloc::allocate(f) } else { regalloc::naive(f) };
        let save_base = outgoing;
        let spill_base = save_base + (alloc.saved_x.len() + alloc.saved_d.len()) as u32 * 8;
        let mut off = (spill_base + alloc.slots * 8) as u64;
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
        let fr = Frame { objects, alloc, spill_base, save_base };

        self.e(stp_pre(FP, LR, SP, -16));
        self.e(mov_sp(FP, SP));
        if size >= 4096 {
            self.e(sub_imm_lsl12(SP, SP, size >> 12));
        }
        if size & 0xFFF != 0 {
            self.e(sub_imm(SP, SP, size & 0xFFF));
        }
        self.save_regs(&fr, false);
        for (i, p) in f.params.iter().enumerate() {
            let i = i as u32;
            if fr.place(*p) == Place::None {
                continue;
            }
            if i < 8 {
                self.store(&fr, i, *p);
            } else {
                self.e(ldr(X9, FP, 16 + (i - 8) * 8));
                self.store(&fr, X9, *p);
            }
        }

        let mut block_pos = vec![0usize; f.blocks.len()];
        let mut pending: Vec<Pending> = Vec::new();
        let mut traps: Traps = Vec::new();

        // How often each register is read, to spot values only one instruction wants.
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
                let mut t = b.term.clone();
                crate::ssa::term_each_use(&mut t, &mut |v| uses[*v as usize] += 1);
            }
        }

        for (idx, b) in f.blocks.iter().enumerate() {
            block_pos[idx] = self.here();
            // A comparison used only by the branch right after it needs no 0-or-1 value.
            let fused = match (&b.term, b.insts.last()) {
                (Term::Br { c, .. }, Some(Inst::Cmp { d, .. } | Inst::FCmp { d, .. })) if f.optimized && c == d && uses[*c as usize] == 1 => true,
                _ => false,
            };
            let count = b.insts.len() - fused as usize;
            let mut k = 0;
            while k < count {
                // `a[i]` loaded or stored at once: the address never needs a register.
                if f.optimized && k + 1 < count && self.indexed_access(&fr, &b.insts[k], &b.insts[k + 1], &uses) {
                    k += 2;
                    continue;
                }
                self.inst(&fr, &b.insts[k], &mut traps, fmts);
                k += 1;
            }
            let next = idx + 1;
            if fused {
                let Term::Br { t, f: fb, .. } = b.term else { unreachable!() };
                let cond = match b.insts.last() {
                    Some(Inst::Cmp { cc, a, b, unsigned, .. }) => {
                        self.compare(&fr, *a, *b);
                        int_cond(*cc, *unsigned)
                    }
                    Some(Inst::FCmp { cc, a, b, bits, .. }) => {
                        self.fcompare(&fr, *a, *b, *bits);
                        float_cond(*cc)
                    }
                    _ => unreachable!(),
                };
                if t == next {
                    // Flipping the low bit gives the exact opposite, NaN included.
                    pending.push(Pending::Cond(self.here(), fb));
                    self.e(b_cond(cond, 0) ^ 1);
                } else {
                    pending.push(Pending::Cond(self.here(), t));
                    self.e(b_cond(cond, 0));
                    if fb != next {
                        pending.push(Pending::B(self.here(), fb));
                        self.e(B);
                    }
                }
                continue;
            }
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
                    self.save_regs(&fr, true);
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
            let msg = format!("trap: {} at {}:{}:{}\n", kind.text(), self.files[loc.file as usize], loc.line, loc.col);
            let off = self.add_str(msg.as_bytes());
            self.data_addr(X0, off);
            self.e(movz(X1, msg.len() as u32, 0));
            self.call(BL, Target::Trap);
        }
    }

    /// Emits `IndexAddr` + `Load`/`Store` as one instruction when the sizes allow it.
    fn indexed_access(&mut self, fr: &Frame, first: &Inst, second: &Inst, uses: &[u32]) -> bool {
        let Inst::IndexAddr { d, base, idx, size } = first else { return false };
        if uses[*d as usize] != 1 {
            return false;
        }
        match second {
            Inst::Load { d: value, p, off: 0, mem } if p == d => {
                match (mem, size, fr.place(*value)) {
                    (Mem::X64, 8, Place::Fpr(x)) => {
                        let b = self.use_x(fr, *base, X9);
                        let i = self.use_x(fr, *idx, X10);
                        self.e(ldr_d_idx(x, b, i));
                    }
                    (Mem::X64, 8, _) => {
                        let b = self.use_x(fr, *base, X9);
                        let i = self.use_x(fr, *idx, X10);
                        let r = self.def_x(fr, *value, X11);
                        self.e(ldr_idx(r, b, i));
                        self.done_x(fr, r, *value);
                    }
                    (Mem::U8, 1, _) => {
                        let b = self.use_x(fr, *base, X9);
                        let i = self.use_x(fr, *idx, X10);
                        let r = self.def_x(fr, *value, X11);
                        self.e(ldrb_idx(r, b, i));
                        self.done_x(fr, r, *value);
                    }
                    _ => return false,
                }
                true
            }
            Inst::Store { p, off: 0, s, mem } if p == d => {
                let b_needed = matches!((mem, size), (Mem::X64, 8) | (Mem::U8 | Mem::I8, 1));
                if !b_needed {
                    return false;
                }
                let b = self.use_x(fr, *base, X9);
                let i = self.use_x(fr, *idx, X10);
                match (mem, fr.place(*s)) {
                    (Mem::X64, Place::Fpr(x)) => self.e(str_d_idx(x, b, i)),
                    (Mem::X64, _) => {
                        let r = self.use_x(fr, *s, X11);
                        self.e(str_idx(r, b, i));
                    }
                    _ => {
                        let r = self.use_x(fr, *s, X11);
                        self.e(strb_idx(r, b, i));
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// Writes the callee-saved registers this function uses, or reads them back.
    fn save_regs(&mut self, fr: &Frame, restore: bool) {
        let mut off = fr.save_base;
        for r in fr.alloc.saved_x.clone() {
            self.e(if restore { ldr(r, SP, off) } else { str(r, SP, off) });
            off += 8;
        }
        for r in fr.alloc.saved_d.clone() {
            self.e(if restore { ldr_d(r, SP, off) } else { str_d(r, SP, off) });
            off += 8;
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
                if !matches!(fr.place(*d), Place::Const(_) | Place::None) {
                    self.mov_imm(X9, *v);
                    self.store(fr, X9, *d);
                }
            }
            Inst::Mov { d, s } => {
                if let Place::Fpr(x) = fr.place(*d) {
                    self.fload(fr, x, *s, 64);
                } else {
                    let r = self.def_x(fr, *d, X9);
                    self.load(fr, r, *s);
                    self.done_x(fr, r, *d);
                }
            }
            Inst::Bin { op, d, a, b, num, loc } => {
                let ra = self.use_x(fr, *a, X9);
                let rd = self.def_x(fr, *d, X11);
                let wide = num.bits == 64 && (num.checked || !num.signed);
                let no_mask = num.bits == 64 || (num.signed && !num.checked);
                match (op, Self::small_const(fr, *b)) {
                    (Arith::Add, Some(k)) if no_mask && !(num.signed && num.checked) => self.e(add_imm(rd, ra, k)),
                    (Arith::Sub, Some(k)) if no_mask && !(num.signed && num.checked) => self.e(sub_imm(rd, ra, k)),
                    (Arith::Add, Some(k)) if wide && num.signed => {
                        self.e(adds_imm(rd, ra, k));
                        self.trap_if(Cond::Vs, Trap::Overflow, *loc, traps);
                    }
                    (Arith::Sub, Some(k)) if wide && num.signed => {
                        self.e(subs_imm(rd, ra, k));
                        self.trap_if(Cond::Vs, Trap::Overflow, *loc, traps);
                    }
                    _ => {
                        let rb = self.use_x(fr, *b, X10);
                        self.bin(*op, *num, *loc, traps, rd, ra, rb);
                    }
                }
                self.done_x(fr, rd, *d);
            }
            Inst::Neg { d, s, num, loc } => {
                self.load(fr, X9, *s);
                if !num.checked {
                    self.e(neg(X11, X9));
                } else if num.bits == 64 {
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
            Inst::Phi { .. } | Inst::Pi { .. } => unreachable!("gone before code generation"),
            Inst::ConstRef { .. } | Inst::Line { .. } | Inst::Note { .. } => unreachable!("only the interpreter sees these"),
            Inst::DataAddr { d, bytes } => {
                let off = self.add_str(bytes);
                self.data_addr(X9, off);
                self.store(fr, X9, *d);
            }
            Inst::Cmp { cc, d, a, b, unsigned } => {
                self.compare(fr, *a, *b);
                let rd = self.def_x(fr, *d, X11);
                self.e(cset(rd, int_cond(*cc, *unsigned) as u32));
                self.done_x(fr, rd, *d);
            }
            Inst::FBin { op, d, a, b, bits } => {
                let x = self.use_d(fr, *a, D0, *bits);
                let y = self.use_d(fr, *b, D1, *bits);
                let r = self.def_d(fr, *d, D0);
                self.e(match op {
                    FArith::Add => fadd(r, x, y, *bits),
                    FArith::Sub => fsub(r, x, y, *bits),
                    FArith::Mul => fmul(r, x, y, *bits),
                    FArith::Div => fdiv(r, x, y, *bits),
                });
                self.fstore(fr, r, *d, *bits);
            }
            Inst::FNeg { d, s, bits } => {
                let x = self.use_d(fr, *s, D0, *bits);
                let r = self.def_d(fr, *d, D0);
                self.e(fneg(r, x, *bits));
                self.fstore(fr, r, *d, *bits);
            }
            Inst::FCmp { cc, d, a, b, bits } => {
                self.fcompare(fr, *a, *b, *bits);
                let rd = self.def_x(fr, *d, X11);
                self.e(cset(rd, float_cond(*cc) as u32));
                self.done_x(fr, rd, *d);
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
                if to.checked {
                    self.float_range_check(*to, *loc, traps);
                }
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
                if !matches!(fr.place(*d), Place::Obj(_) | Place::None) {
                    self.obj_addr(fr, X9, *obj);
                    self.store(fr, X9, *d);
                }
            }
            Inst::AddImm { d, s, imm } => {
                let rd = self.def_x(fr, *d, X9);
                if *imm < 4096 {
                    let x = self.use_x(fr, *s, X9);
                    self.e(add_imm(rd, x, *imm as u32));
                } else {
                    self.addr_x9(fr, *s, *imm);
                    if rd != X9 {
                        self.e(mov(rd, X9));
                    }
                }
                self.done_x(fr, rd, *d);
            }
            Inst::Load { d, p, off, mem } => {
                if *mem == Mem::X64 && *off <= 32760 && off % 8 == 0 {
                    let base = self.use_x(fr, *p, X9);
                    if let Place::Fpr(x) = fr.place(*d) {
                        if *off <= 16380 {
                            self.e(ldr_d(x, base, *off as u32));
                            return;
                        }
                    }
                    let r = self.def_x(fr, *d, X10);
                    self.e(ldr(r, base, *off as u32));
                    self.done_x(fr, r, *d);
                    return;
                }
                self.addr_x9(fr, *p, *off);
                let r = self.def_x(fr, *d, X10);
                self.e(match mem {
                    Mem::I8 => ldrsb(r, X9),
                    Mem::I16 => ldrsh(r, X9),
                    Mem::I32 => ldrsw(r, X9),
                    Mem::U8 => ldrb(r, X9, 0),
                    Mem::U16 => ldrh(r, X9),
                    Mem::U32 => ldr_w(r, X9),
                    Mem::X64 => ldr(r, X9, 0),
                });
                self.done_x(fr, r, *d);
            }
            Inst::Store { p, off, s, mem } => {
                if *mem == Mem::X64 && *off <= 32760 && off % 8 == 0 {
                    let base = self.use_x(fr, *p, X9);
                    if let Place::Fpr(x) = fr.place(*s) {
                        if *off <= 16380 {
                            self.e(str_d(x, base, *off as u32));
                            return;
                        }
                    }
                    let r = self.use_x(fr, *s, X10);
                    self.e(str(r, base, *off as u32));
                    return;
                }
                self.addr_x9(fr, *p, *off);
                let r = self.use_x(fr, *s, X10);
                self.e(match mem {
                    Mem::I8 | Mem::U8 => strb(r, X9, 0),
                    Mem::I16 | Mem::U16 => strh(r, X9),
                    Mem::I32 | Mem::U32 => str_w(r, X9),
                    Mem::X64 => str(r, X9, 0),
                });
            }
            Inst::MemCopy { dst, src, size } => self.mem_copy(fr, *dst, *src, *size),
            Inst::MemCopyDyn { dst, src, n } => {
                self.load(fr, X0, *dst);
                self.load(fr, X1, *src);
                self.load(fr, X2, *n);
                self.call(BL, Target::Import(IMP_MEMCPY));
            }
            Inst::BoundsDyn { idx, len, loc } => {
                self.compare(fr, *idx, *len);
                self.trap_if(Cond::Hs, Trap::Bounds, *loc, traps);
            }
            Inst::BufPtr { p, n } => {
                self.load(fr, X0, *p);
                self.load(fr, X1, *n);
                self.call(BL, Target::BufStr);
            }
            Inst::CaptureStart => {
                self.bss_addr(X9);
                self.mov_imm(X16, CAPTURE_FLAG as i64);
                self.e(add(X9, X9, X16));
                self.e(movz(X10, 1, 0));
                self.e(str(X10, X9, 0));
            }
            Inst::CaptureTake { ptr, len } => {
                self.call(BL, Target::RtTake);
                self.store(fr, X0, *ptr);
                self.store(fr, X1, *len);
            }
            Inst::IndexAddr { d, base, idx, size } => {
                let b = self.use_x(fr, *base, X9);
                let i = self.use_x(fr, *idx, X10);
                let rd = self.def_x(fr, *d, X11);
                if size.is_power_of_two() && *size <= 16 {
                    self.e(add_lsl(rd, b, i, size.trailing_zeros()));
                } else {
                    self.mov_imm(X12, *size as i64);
                    self.e(mul(X12, i, X12));
                    self.e(add(rd, b, X12));
                }
                self.done_x(fr, rd, *d);
            }
            Inst::Bounds { idx, len, loc } => {
                let x = self.use_x(fr, *idx, X9);
                if *len < 4096 {
                    self.e(cmp_imm(x, *len as u32));
                } else {
                    self.mov_imm(X10, *len as i64);
                    self.e(cmp(x, X10));
                }
                // Unsigned compare: a negative index looks huge.
                self.trap_if(Cond::Hs, Trap::Bounds, *loc, traps);
            }
            Inst::Alloc { d, n, elem, loc } => {
                self.load(fr, X9, *n);
                self.e(cmp_imm(X9, 0));
                self.trap_if(Cond::Lt, Trap::AllocSize, *loc, traps);
                self.mov_imm(X10, *elem as i64);
                self.e(umulh(X12, X9, X10));
                self.e(cmp_imm(X12, 0));
                self.trap_if(Cond::Ne, Trap::AllocSize, *loc, traps);
                self.e(mul(X0, X9, X10));
                self.call(BL, Target::RtAlloc);
                self.e(cmp_imm(X0, 0));
                self.trap_if(Cond::Eq, Trap::OutOfMemory, *loc, traps);
                self.store(fr, X0, *d);
            }
            Inst::Free { p } => {
                self.load(fr, X0, *p);
                self.call(BL, Target::RtFree);
            }
            Inst::StrAddr { d, text } => {
                let mut bytes = text.as_bytes().to_vec();
                bytes.push(0);
                let off = self.add_str(&bytes);
                self.data_addr(X9, off);
                self.store(fr, X9, *d);
            }
            Inst::CallC { d, sym, args, variadic, ret } => self.call_c(fr, *d, sym, args, *variadic, *ret),
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
            Inst::WideBin { op, dlo, dhi, alo, ahi, blo, bhi, loc } => {
                self.load(fr, X9, *alo);
                self.load(fr, X10, *ahi);
                self.load(fr, X11, *blo);
                self.load(fr, X12, *bhi);
                match op {
                    WideOp::Add => {
                        self.e(adds(X13, X9, X11));
                        self.e(adcs(X14, X10, X12));
                        self.trap_if(Cond::Vs, Trap::ContractMath, *loc, traps);
                    }
                    WideOp::Sub => {
                        self.e(subs(X13, X9, X11));
                        self.e(sbcs(X14, X10, X12));
                        self.trap_if(Cond::Vs, Trap::ContractMath, *loc, traps);
                    }
                    WideOp::Mul => {
                        // Exact only while both sides fit in 64 signed bits.
                        self.e(cmp_asr(X10, X9, 63));
                        self.trap_if(Cond::Ne, Trap::ContractMath, *loc, traps);
                        self.e(cmp_asr(X12, X11, 63));
                        self.trap_if(Cond::Ne, Trap::ContractMath, *loc, traps);
                        self.e(mul(X13, X9, X11));
                        self.e(smulh(X14, X9, X11));
                    }
                }
                self.store(fr, X13, *dlo);
                self.store(fr, X14, *dhi);
            }
            Inst::WideCmp { cc, d, alo, ahi, blo, bhi } => {
                // `a > b` is `b < a`: only LT and GE are valid after subs + sbcs.
                let (x, y) = if matches!(cc, Cc::Gt | Cc::Le) { ((*blo, *bhi), (*alo, *ahi)) } else { ((*alo, *ahi), (*blo, *bhi)) };
                self.load(fr, X9, x.0);
                self.load(fr, X10, x.1);
                self.load(fr, X11, y.0);
                self.load(fr, X12, y.1);
                let cond = match cc {
                    Cc::Eq | Cc::Ne => {
                        self.e(eor(X13, X9, X11));
                        self.e(eor(X14, X10, X12));
                        self.e(orr(X13, X13, X14));
                        self.e(cmp_imm(X13, 0));
                        if *cc == Cc::Eq { Cond::Eq } else { Cond::Ne }
                    }
                    _ => {
                        self.e(subs(X13, X9, X11));
                        self.e(sbcs(X14, X10, X12));
                        if matches!(cc, Cc::Lt | Cc::Gt) { Cond::Lt } else { Cond::Ge }
                    }
                };
                self.e(cset(X13, cond as u32));
                self.store(fr, X13, *d);
            }
            Inst::RtState { d } => {
                self.bss_addr(X9);
                self.mov_imm(X16, STATE as i64);
                self.e(add(X9, X9, X16));
                self.store(fr, X9, *d);
            }
            Inst::SetSite { text } => {
                let mut bytes = text.as_bytes().to_vec();
                bytes.push(0);
                let off = self.add_str(&bytes);
                self.data_addr(X9, off);
                self.bss_addr(X10);
                self.mov_imm(X16, (STATE + SITE_WORD) as i64);
                self.e(add(X10, X10, X16));
                self.e(str(X9, X10, 0));
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

    fn call_c(&mut self, fr: &Frame, d: Option<VReg>, sym: &str, args: &[CArg], variadic: Option<usize>, ret: CRet) {
        let (slots, _) = assign_c(args, variadic);
        for (i, (a, slot)) in args.iter().zip(&slots).enumerate() {
            if let Slot::Stack(off) = slot {
                self.load(fr, X9, a.v);
                self.e(add_imm(X10, SP, *off));
                let size = if variadic.is_some_and(|v| i >= v) { 8 } else { a.size };
                self.e(match size {
                    1 => strb(X9, X10, 0),
                    2 => strh(X9, X10),
                    4 => str_w(X9, X10),
                    _ => str(X9, X10, 0),
                });
            }
        }
        for (a, slot) in args.iter().zip(&slots) {
            match slot {
                Slot::X(r) => self.load(fr, *r, a.v),
                Slot::D(r) => self.fload(fr, *r, a.v, a.float.unwrap_or(64)),
                Slot::Stack(_) => {}
            }
        }
        let want = format!("_{sym}");
        let index = self.imports.iter().position(|n| *n == want).expect("imports were collected up front");
        self.call(BL, Target::Import(index));
        // C leaves the upper bits of a narrow result undefined.
        match ret {
            CRet::None => {}
            CRet::Float(bits) => {
                if let Some(d) = d {
                    self.fstore(fr, D0, d, bits);
                }
            }
            CRet::Int(mem) => {
                match mem {
                    Mem::I8 => self.e(sbfx(X0, X0, 8)),
                    Mem::I16 => self.e(sbfx(X0, X0, 16)),
                    Mem::I32 => self.e(sbfx(X0, X0, 32)),
                    Mem::U8 => self.e(ubfx(X0, X0, 8)),
                    Mem::U16 => self.e(ubfx(X0, X0, 16)),
                    Mem::U32 => self.e(ubfx(X0, X0, 32)),
                    Mem::X64 => {}
                }
                if let Some(d) = d {
                    self.store(fr, X0, d);
                }
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

    /// `normalize`, for a result that is not in x11.
    fn normalize_in(&mut self, num: Num, loc: Loc, traps: &mut Traps, r: Reg) {
        if r != X11 {
            self.e(mov(X11, r));
        }
        self.normalize(num, loc, traps);
        if r != X11 {
            self.e(mov(r, X11));
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
        if to.signed && !to.checked {
            self.e(mov(X11, X9));
        } else if to.signed {
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

    /// The registers may coincide; x11 and x12 are free to use.
    fn bin(&mut self, op: Arith, num: Num, loc: Loc, traps: &mut Traps, rd: Reg, ra: Reg, rb: Reg) {
        let wide_signed = num.signed && num.bits == 64 && num.checked;
        match op {
            Arith::Add => {
                if wide_signed {
                    self.e(adds(rd, ra, rb));
                    self.trap_if(Cond::Vs, Trap::Overflow, loc, traps);
                } else {
                    self.e(add(rd, ra, rb));
                    self.normalize_in(num, loc, traps, rd);
                }
            }
            Arith::Sub => {
                if wide_signed {
                    self.e(subs(rd, ra, rb));
                    self.trap_if(Cond::Vs, Trap::Overflow, loc, traps);
                } else {
                    self.e(sub(rd, ra, rb));
                    self.normalize_in(num, loc, traps, rd);
                }
            }
            Arith::Mul => {
                if wide_signed {
                    self.e(smulh(X12, ra, rb));
                    self.e(mul(rd, ra, rb));
                    self.e(cmp_asr(X12, rd, 63));
                    self.trap_if(Cond::Ne, Trap::Overflow, loc, traps);
                } else {
                    self.e(mul(rd, ra, rb));
                    self.normalize_in(num, loc, traps, rd);
                }
            }
            Arith::Div => {
                if num.checked {
                    self.e(cmp_imm(rb, 0));
                    self.trap_if(Cond::Eq, Trap::DivZero, loc, traps);
                }
                if num.signed {
                    if num.bits == 64 && num.checked {
                        // MIN / -1 is the only quotient that does not fit in 64 bits.
                        // Negating the dividend overflows exactly when it is MIN.
                        self.e(cmn_imm(rb, 1));
                        self.e(b_cond(Cond::Ne, 3));
                        self.e(negs(X12, ra));
                        self.trap_if(Cond::Vs, Trap::Overflow, loc, traps);
                    }
                    self.e(sdiv(rd, ra, rb));
                    self.normalize_in(num, loc, traps, rd);
                } else {
                    self.e(udiv(rd, ra, rb));
                }
            }
            Arith::Rem => {
                if num.checked {
                    self.e(cmp_imm(rb, 0));
                    self.trap_if(Cond::Eq, Trap::DivZero, loc, traps);
                }
                self.e(if num.signed { sdiv(X12, ra, rb) } else { udiv(X12, ra, rb) });
                self.e(msub(rd, X12, rb, ra));
            }
            Arith::And => self.e(and(rd, ra, rb)),
            Arith::Or => self.e(orr(rd, ra, rb)),
            Arith::Xor => self.e(eor(rd, ra, rb)),
            Arith::Shl => {
                if num.checked {
                    self.e(cmp_imm(rb, num.bits as u32));
                    self.trap_if(Cond::Hs, Trap::Shift, loc, traps);
                }
                if wide_signed {
                    self.e(lsl(X11, ra, rb));
                    self.e(asr(X12, X11, rb));
                    self.e(cmp(X12, ra));
                    self.trap_if(Cond::Ne, Trap::Overflow, loc, traps);
                    self.e(mov(rd, X11));
                } else {
                    self.e(lsl(rd, ra, rb));
                    self.normalize_in(num, loc, traps, rd);
                }
            }
            Arith::Shr => {
                if num.checked {
                    self.e(cmp_imm(rb, num.bits as u32));
                    self.trap_if(Cond::Hs, Trap::Shift, loc, traps);
                }
                self.e(if num.signed { asr(rd, ra, rb) } else { lsr(rd, ra, rb) });
            }
        }
    }
}

pub fn generate(m: &Module, files: &[String], alloc_stats: bool) -> Result<Object, String> {
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
        code: Vec::new(),
        rodata: Vec::new(),
        data_refs: Vec::new(),
        bss_refs: Vec::new(),
        got_refs: Vec::new(),
        calls: Vec::new(),
        files: files.to_vec(),
        imports: imports.clone(),
        too_large: None,
    };
    let mut symbols: Vec<(String, usize)> = Vec::new();

    let fmt64 = g.add_str(b"%.15g\0");
    let fmt32 = g.add_str(b"%.7g\0");

    let stubs: Vec<usize> = (0..imports.len()).map(|s| g.import_stub(s)).collect();
    // Every allocation passes through these two, so a later version can record them.
    let rt_alloc = g.here();
    if alloc_stats {
        g.bump_counter(ALLOC_COUNT);
    }
    g.call(B, m.check.map_or(Target::Import(IMP_MALLOC), |c| Target::User(c.alloc)));
    let rt_free = g.here();
    if alloc_stats {
        g.bump_counter(FREE_COUNT);
    }
    g.call(B, m.check.map_or(Target::Import(IMP_FREE), |c| Target::User(c.free)));
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
    g.e(stp_pre(FP, LR, SP, -16));
    g.e(mov_sp(FP, SP));
    g.call(BL, Target::User(m.main));
    if let Some(c) = m.check {
        g.call(BL, Target::User(c.finish));
    }
    if alloc_stats {
        g.report_alloc_stats();
    }
    g.e(movz(X0, 0, 0));
    g.e(ldp_post(FP, LR, SP, 16));
    g.e(ret());

    let mut func_pos = Vec::with_capacity(m.funcs.len());
    for f in &m.funcs {
        func_pos.push(g.here());
        if f.dead {
            continue;
        }
        symbols.push((format!("_{}", f.name), g.here()));
        g.func(f, (fmt64, fmt32));
    }

    for (at, t) in std::mem::take(&mut g.calls) {
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
        imports,
        symbols,
        entry,
        bss_size: STATE + STATE_SIZE,
    })
}
