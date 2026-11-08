//! CIR, the Claw intermediate representation: typed instructions over virtual
//! registers, grouped into basic blocks. Until 0.6 every vreg lives on the stack.
//! A vreg holds a scalar (integers kept sign- or zero-extended to 64 bits, floats
//! as raw bits) or the address of an aggregate.

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    Bin { op: Arith, d: VReg, a: VReg, b: VReg, num: Num, loc: Loc },
    Neg { d: VReg, s: VReg, num: Num, loc: Loc },
    BitNot { d: VReg, s: VReg, num: Num },
    BoolNot { d: VReg, s: VReg },
    /// `d += 1` for a loop counter already known to be below its limit.
    Inc { d: VReg },
    Cmp { cc: Cc, d: VReg, a: VReg, b: VReg, unsigned: bool },
    FBin { op: FArith, d: VReg, a: VReg, b: VReg, bits: u8 },
    FNeg { d: VReg, s: VReg, bits: u8 },
    FCmp { cc: Cc, d: VReg, a: VReg, b: VReg, bits: u8 },
    IntCast { d: VReg, s: VReg, from: Num, to: Num, loc: Loc },
    IntToFloat { d: VReg, s: VReg, signed: bool, bits: u8 },
    FloatToInt { d: VReg, s: VReg, bits: u8, to: Num, loc: Loc },
    FloatCast { d: VReg, s: VReg, to_bits: u8 },
    Call { d: Option<VReg>, f: FuncId, args: Vec<VReg> },
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
}

#[derive(Clone, Debug)]
pub enum Term {
    Jmp(BlockId),
    Br { c: VReg, t: BlockId, f: BlockId },
    Ret(Option<VReg>),
    /// Control cannot get here; sema guarantees it.
    Unreachable,
}

#[derive(Debug)]
pub struct Block {
    pub insts: Vec<Inst>,
    pub term: Term,
}

#[derive(Debug)]
pub struct Func {
    pub name: String,
    pub nparams: usize,
    pub nvregs: u32,
    /// Stack objects: (size, alignment).
    pub objects: Vec<(u64, u64)>,
    pub blocks: Vec<Block>,
}

#[derive(Debug)]
pub struct Module {
    pub funcs: Vec<Func>,
    pub main: FuncId,
}

pub fn dump(m: &Module) -> String {
    let mut out = String::new();
    for f in &m.funcs {
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
