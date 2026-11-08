//! CIR, the Claw intermediate representation: typed instructions over virtual
//! registers, grouped into basic blocks. In 0.1 every vreg lives on the stack.

pub type VReg = u32;
pub type BlockId = usize;
pub type FuncId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loc {
    pub line: u32,
    pub col: u32,
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
pub enum Cc {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Debug)]
pub enum Inst {
    Const { d: VReg, v: i64 },
    Mov { d: VReg, s: VReg },
    Bin { op: Arith, d: VReg, a: VReg, b: VReg, loc: Loc },
    Neg { d: VReg, s: VReg, loc: Loc },
    BitNot { d: VReg, s: VReg },
    BoolNot { d: VReg, s: VReg },
    /// `d += 1` for a loop counter already known to be below its limit.
    Inc { d: VReg },
    Cmp { cc: Cc, d: VReg, a: VReg, b: VReg },
    Call { d: Option<VReg>, f: FuncId, args: Vec<VReg> },
    PrintInt { s: VReg },
    PrintBool { s: VReg },
    PrintStr { text: String },
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
        out.push_str(&format!("fn {} params={} vregs={}\n", f.name, f.nparams, f.nvregs));
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
