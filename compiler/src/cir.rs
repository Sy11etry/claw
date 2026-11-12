//! CIR, the Claw intermediate representation: typed instructions over virtual
//! registers, grouped into basic blocks. Until 0.6 every vreg lives on the stack.
//! A vreg holds a scalar (integers kept sign- or zero-extended to 64 bits, floats
//! as raw bits) or the address of an aggregate.

use crate::types::{Ty, Vk};

pub type VReg = u32;
pub type BlockId = usize;
pub type FuncId = usize;
pub type ObjId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loc {
    pub line: u32,
    pub col: u32,
    pub file: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Num {
    pub bits: u8,
    pub signed: bool,
    /// False once the optimizer has proved the operation cannot trap.
    pub checked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mem {
    I8,
    I16,
    I32,
    U8,
    U16,
    U32,
    X64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arith {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    And,
    Or,
    Xor,
    Shl,
    Shr,
}

/// Exact arithmetic inside contracts, on 128-bit (lo, hi) pairs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WideOp {
    Add,
    Sub,
    Mul,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FArith {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cc {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Copy, Debug)]
pub struct CArg {
    pub v: VReg,
    /// Set for floats, which travel in d-registers.
    pub float: Option<u8>,
    pub size: u32,
}

#[derive(Clone, Copy, Debug)]
pub enum CRet {
    None,
    Int(Mem),
    Float(u8),
}

#[derive(Clone, Debug)]
pub enum Inst {
    Const { d: VReg, v: i64 },
    Mov { d: VReg, s: VReg },
    /// Only while the optimizer works; gone before code is generated.
    Phi { d: VReg, args: Vec<(BlockId, VReg)> },
    /// Also only inside the optimizer: `d` is `s`, and here `s cc other` is known to hold.
    Pi { d: VReg, s: VReg, cc: Cc, other: VReg, unsigned: bool },
    Bin { op: Arith, d: VReg, a: VReg, b: VReg, num: Num, loc: Loc },
    Neg { d: VReg, s: VReg, num: Num, loc: Loc },
    BitNot { d: VReg, s: VReg, num: Num },
    BoolNot { d: VReg, s: VReg },
    Cmp { cc: Cc, d: VReg, a: VReg, b: VReg, unsigned: bool },
    FBin { op: FArith, d: VReg, a: VReg, b: VReg, bits: u8 },
    FNeg { d: VReg, s: VReg, bits: u8 },
    FCmp { cc: Cc, d: VReg, a: VReg, b: VReg, bits: u8 },
    IntCast { d: VReg, s: VReg, from: Num, to: Num, loc: Loc },
    IntToFloat { d: VReg, s: VReg, signed: bool, bits: u8 },
    FloatToInt { d: VReg, s: VReg, bits: u8, to: Num, loc: Loc },
    FloatCast { d: VReg, s: VReg, to_bits: u8 },
    /// `vsig`: bit i (i < 8) says argument i is a 128-bit vector, bit 8 says the result is one.
    Call { d: Option<VReg>, f: FuncId, args: Vec<VReg>, vsig: u16 },
    /// A call through a function value.
    CallInd { d: Option<VReg>, callee: VReg, args: Vec<VReg> },
    /// A function as a value.
    FuncAddr { d: VReg, f: FuncId },
    ObjAddr { d: VReg, obj: ObjId },
    AddImm { d: VReg, s: VReg, imm: u64 },
    Load { d: VReg, p: VReg, off: u64, mem: Mem },
    Store { p: VReg, off: u64, s: VReg, mem: Mem },
    MemCopy { dst: VReg, src: VReg, size: u64 },
    MemCopyDyn { dst: VReg, src: VReg, n: VReg },
    IndexAddr { d: VReg, base: VReg, idx: VReg, size: u64 },
    Bounds { idx: VReg, len: u64, loc: Loc },
    BoundsDyn { idx: VReg, len: VReg, loc: Loc },
    /// `malloc` of `n` items, trapping on a bad size or on failure.
    Alloc { d: VReg, n: VReg, elem: u64, loc: Loc },
    Free { p: VReg },
    /// The address of NUL-terminated bytes, for handing to C.
    StrAddr { d: VReg, text: String },
    CallC { d: Option<VReg>, sym: String, args: Vec<CArg>, variadic: Option<usize>, ret: CRet },
    BufStr { text: String },
    BufInt { s: VReg, signed: bool },
    BufPtr { p: VReg, n: VReg },
    /// From here on the output buffer collects a string instead of printing.
    CaptureStart,
    CaptureTake { ptr: VReg, len: VReg },
    BufBool { s: VReg },
    BufFloat { s: VReg, bits: u8 },
    Flush { fd: u32 },
    Exit { code: u32 },
    WideBin { op: WideOp, dlo: VReg, dhi: VReg, alo: VReg, ahi: VReg, blo: VReg, bhi: VReg, loc: Loc },
    WideCmp { cc: Cc, d: VReg, alo: VReg, ahi: VReg, blo: VReg, bhi: VReg },
    /// The address of the runtime's words: 0 is the contract guard, 1 the current site.
    RtState { d: VReg },
    /// Remembers where the next allocation or release comes from.
    SetSite { text: String },
    /// Read-only bytes worked out at compile time.
    DataAddr { d: VReg, bytes: Vec<u8> },
    /// Only in code for the interpreter: the value of a `const` that needs a function call.
    ConstRef { d: VReg, id: usize },
    /// Only in code for the interpreter: execution reached this source line.
    Line { loc: Loc },
    /// Only in code for the interpreter: variable `name` now holds `v`.
    Note { name: String, v: VReg, show: Show },
    /// SIMD, from `claw fn` only: a 128-bit value in one vector register. `Add Sub Mul`, and `Div` on floats.
    VBin { op: FArith, d: VReg, a: VReg, b: VReg, ty: Vk },
    /// `d = a * b + c`, fused.
    VFma { d: VReg, a: VReg, b: VReg, c: VReg, ty: Vk },
    VNeg { d: VReg, s: VReg, ty: Vk },
    /// Every lane gets the scalar `s`.
    VSplat { d: VReg, s: VReg, ty: Vk },
    VLoad { d: VReg, p: VReg, off: u64 },
    VStore { p: VReg, off: u64, s: VReg },
    /// One lane as a scalar of the lane type (an `i32x4` lane is sign-extended to 64 bits).
    VLane { d: VReg, s: VReg, lane: u8, ty: Vk },
}

/// How a variable's value reads in a report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    Int,
    Uint,
    Bool,
    F32,
    F64,
}

#[derive(Clone, Debug)]
pub enum Term {
    Jmp(BlockId),
    Br { c: VReg, t: BlockId, f: BlockId },
    Ret(Option<VReg>),
    /// Control cannot get here; sema guarantees it.
    Unreachable,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub insts: Vec<Inst>,
    pub term: Term,
}

#[derive(Debug, Clone)]
pub struct Func {
    pub name: String,
    pub nparams: usize,
    /// The registers holding the parameters, in order.
    pub params: Vec<VReg>,
    /// Which parameters (bit i, i < 8) and whether the result (bit 8) are 128-bit vectors.
    pub vsig: u16,
    /// Written as `claw fn`: raw pointers may alias anything.
    pub claw: bool,
    /// Registers were allocated rather than every value living on the stack.
    pub optimized: bool,
    /// Nothing can reach it, so no code is generated.
    pub dead: bool,
    pub nvregs: u32,
    /// Stack objects: (size, alignment).
    pub objects: Vec<(u64, u64)>,
    pub blocks: Vec<Block>,
}

#[derive(Debug)]
pub struct Module {
    pub funcs: Vec<Func>,
    pub main: FuncId,
    /// Present in check mode: the memory checker's entry points.
    pub check: Option<CheckRt>,
    /// (name, function) of each `test` block, in `claw test`.
    pub tests: Vec<(String, FuncId)>,
    /// Functions the fuzzer may call, when built for the interpreter.
    pub autos: Vec<Auto>,
    /// The function computing each `const` that needs a call, in id order.
    pub const_funcs: Vec<FuncId>,
}

/// A function of the root file, and what the fuzzer needs to call it.
#[derive(Debug, Clone)]
pub struct Auto {
    /// As `effects` knows it.
    pub full: String,
    pub name: String,
    pub func: FuncId,
    /// Says whether arguments satisfy the `need` clauses.
    pub filter: Option<FuncId>,
    pub params: Vec<(String, Ty, bool)>,
    pub ret: Ty,
}

#[derive(Debug, Clone, Copy)]
pub struct CheckRt {
    pub alloc: FuncId,
    pub free: FuncId,
    pub finish: FuncId,
}

macro_rules! walk_inst {
    ($i:expr, $f:ident, $($m:tt)?) => {
        match $i {
            Inst::Const { d, .. } | Inst::ObjAddr { d, .. } | Inst::StrAddr { d, .. } | Inst::RtState { d } | Inst::DataAddr { d, .. } | Inst::ConstRef { d, .. } | Inst::FuncAddr { d, .. } => $f(d, true),
            Inst::Mov { d, s } | Inst::BitNot { d, s, .. } | Inst::BoolNot { d, s } | Inst::Neg { d, s, .. } | Inst::FNeg { d, s, .. }
            | Inst::VNeg { d, s, .. } | Inst::VSplat { d, s, .. } | Inst::VLane { d, s, .. }
            | Inst::IntCast { d, s, .. } | Inst::IntToFloat { d, s, .. } | Inst::FloatToInt { d, s, .. } | Inst::FloatCast { d, s, .. }
            | Inst::AddImm { d, s, .. } => {
                $f(s, false);
                $f(d, true);
            }
            Inst::Pi { d, s, other, .. } => {
                $f(s, false);
                $f(other, false);
                $f(d, true);
            }
            Inst::Phi { d, args } => {
                for (_, v) in args {
                    $f(v, false);
                }
                $f(d, true);
            }
            Inst::Bin { d, a, b, .. } | Inst::Cmp { d, a, b, .. } | Inst::FBin { d, a, b, .. } | Inst::FCmp { d, a, b, .. } | Inst::VBin { d, a, b, .. } => {
                $f(a, false);
                $f(b, false);
                $f(d, true);
            }
            Inst::VFma { d, a, b, c, .. } => {
                $f(a, false);
                $f(b, false);
                $f(c, false);
                $f(d, true);
            }
            Inst::VLoad { d, p, .. } => {
                $f(p, false);
                $f(d, true);
            }
            Inst::VStore { p, s, .. } => {
                $f(p, false);
                $f(s, false);
            }
            Inst::Call { d, args, .. } => {
                for a in args {
                    $f(a, false);
                }
                if let Some(d) = d {
                    $f(d, true);
                }
            }
            Inst::CallInd { d, callee, args } => {
                $f(callee, false);
                for a in args {
                    $f(a, false);
                }
                if let Some(d) = d {
                    $f(d, true);
                }
            }
            Inst::CallC { d, args, .. } => {
                for a in args {
                    $f(&$($m)? a.v, false);
                }
                if let Some(d) = d {
                    $f(d, true);
                }
            }
            Inst::Load { d, p, .. } => {
                $f(p, false);
                $f(d, true);
            }
            Inst::Store { p, s, .. } => {
                $f(p, false);
                $f(s, false);
            }
            Inst::MemCopy { dst, src, .. } => {
                $f(dst, false);
                $f(src, false);
            }
            Inst::MemCopyDyn { dst, src, n } => {
                $f(dst, false);
                $f(src, false);
                $f(n, false);
            }
            Inst::IndexAddr { d, base, idx, .. } => {
                $f(base, false);
                $f(idx, false);
                $f(d, true);
            }
            Inst::Bounds { idx, .. } => $f(idx, false),
            Inst::BoundsDyn { idx, len, .. } => {
                $f(idx, false);
                $f(len, false);
            }
            Inst::Alloc { d, n, .. } => {
                $f(n, false);
                $f(d, true);
            }
            Inst::Free { p } => $f(p, false),
            Inst::BufInt { s, .. } | Inst::BufBool { s } | Inst::BufFloat { s, .. } => $f(s, false),
            Inst::BufPtr { p, n } => {
                $f(p, false);
                $f(n, false);
            }
            Inst::CaptureTake { ptr, len } => {
                $f(ptr, true);
                $f(len, true);
            }
            Inst::WideBin { dlo, dhi, alo, ahi, blo, bhi, .. } => {
                $f(alo, false);
                $f(ahi, false);
                $f(blo, false);
                $f(bhi, false);
                $f(dlo, true);
                $f(dhi, true);
            }
            Inst::WideCmp { d, alo, ahi, blo, bhi, .. } => {
                $f(alo, false);
                $f(ahi, false);
                $f(blo, false);
                $f(bhi, false);
                $f(d, true);
            }
            Inst::Note { v, .. } => $f(v, false),
            Inst::BufStr { .. } | Inst::CaptureStart | Inst::Flush { .. } | Inst::Exit { .. } | Inst::SetSite { .. } | Inst::Line { .. } => {}
        }
    };
}

impl Inst {
    /// Calls `f` with every register, and whether this instruction writes it.
    pub fn each(&self, f: &mut dyn FnMut(VReg, bool)) {
        let mut g = |v: &VReg, def: bool| f(*v, def);
        walk_inst!(self, g,);
    }

    pub fn each_mut(&mut self, f: &mut dyn FnMut(&mut VReg, bool)) {
        let mut g = |v: &mut VReg, def: bool| f(v, def);
        walk_inst!(self, g, mut);
    }

    /// The one register this writes, for instructions that write exactly one.
    pub fn def(&self) -> Option<VReg> {
        let mut out = None;
        let mut n = 0;
        self.each(&mut |v, def| {
            if def {
                out = Some(v);
                n += 1;
            }
        });
        if n == 1 {
            out
        } else {
            None
        }
    }

    /// Can a call's register rules touch this? Then x0-x7 are not safe across or into it.
    pub fn is_call(&self) -> bool {
        match self {
            Inst::Call { .. } | Inst::CallInd { .. } | Inst::CallC { .. } | Inst::MemCopyDyn { .. } | Inst::Alloc { .. } | Inst::Free { .. } | Inst::BufStr { .. } | Inst::BufInt { .. } | Inst::BufPtr { .. } | Inst::CaptureTake { .. } | Inst::BufBool { .. } | Inst::BufFloat { .. } | Inst::Flush { .. } | Inst::Exit { .. } => true,
            Inst::MemCopy { size, .. } => *size > 64,
            _ => false,
        }
    }

    /// If the result is unused, may this instruction simply disappear?
    pub fn removable(&self) -> bool {
        match self {
            Inst::Const { .. } | Inst::Mov { .. } | Inst::Phi { .. } | Inst::Pi { .. } | Inst::BitNot { .. } | Inst::BoolNot { .. } | Inst::Cmp { .. } | Inst::FBin { .. } | Inst::FNeg { .. } | Inst::FCmp { .. } | Inst::IntToFloat { .. } | Inst::FloatCast { .. } | Inst::ObjAddr { .. } | Inst::AddImm { .. } | Inst::Load { .. } | Inst::IndexAddr { .. } | Inst::StrAddr { .. } | Inst::RtState { .. } | Inst::WideCmp { .. } | Inst::DataAddr { .. } | Inst::FuncAddr { .. } | Inst::VBin { .. } | Inst::VFma { .. } | Inst::VNeg { .. } | Inst::VSplat { .. } | Inst::VLoad { .. } | Inst::VLane { .. } => true,
            Inst::Bin { op, num, .. } => match op {
                Arith::And | Arith::Or | Arith::Xor => true,
                Arith::Add | Arith::Sub | Arith::Mul => !num.signed || !num.checked,
                _ => !num.checked,
            },
            Inst::Neg { num, .. } => !num.checked,
            Inst::IntCast { from, to, .. } => {
                !to.signed || (from.bits <= to.bits && (from.signed || from.bits < to.bits)) || !to.checked
            }
            _ => false,
        }
    }
}

pub fn dump(m: &Module) -> String {
    let mut out = String::new();
    for f in m.funcs.iter().filter(|f| !f.dead) {
        out.push_str(&format!("fn {} params={} vregs={} objects={:?}\n", f.name, f.nparams, f.nvregs, f.objects));
        for (i, b) in f.blocks.iter().enumerate() {
            out.push_str(&format!("  b{i}:\n"));
            for inst in &b.insts {
                out.push_str(&format!("    {inst:?}\n"));
            }
            out.push_str(&format!("    {:?}\n", b.term));
        }
    }
    out
}
