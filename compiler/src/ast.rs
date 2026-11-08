//! Syntax tree. Sema writes each expression's type into `Expr::ty`.

use crate::diag::Span;
use crate::sema::ConstVal;
use crate::types::{Ty, ERR};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub enum Size {
    Lit(i128),
    Name(String),
}

#[derive(Debug, Clone)]
pub enum TypeKind {
    Name { module: Option<String>, name: String, targs: Vec<TypeExpr> },
    Pointer(Box<TypeExpr>),
    Ref(Box<TypeExpr>),
    Slice(Box<TypeExpr>),
    Array(Size, Box<TypeExpr>),
    Option(Box<TypeExpr>),
    /// `T ! E`, or `! E` when the success value is nothing.
    Result(Option<Box<TypeExpr>>, Box<TypeExpr>),
}

#[derive(Debug, Clone)]
pub struct TypeExpr {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Debug, Default, Clone)]
pub struct Program {
    pub funcs: Vec<Func>,
    pub types: Vec<TypeDecl>,
    pub consts: Vec<ConstDecl>,
    pub externs: Vec<ExternDecl>,
    pub uses: Vec<UseDecl>,
}

/// `use a.b`: the file a/b.claw, known here as `b`.
#[derive(Debug, Clone)]
pub struct UseDecl {
    pub path: Vec<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ExternDecl {
    /// The C function's name, when it differs from `name`.
    pub symbol: Option<String>,
    pub name: String,
    pub span: Span,
    pub public: bool,
    pub params: Vec<Param>,
    pub variadic: bool,
    pub ret: Option<TypeExpr>,
}

#[derive(Debug, Clone)]
pub struct FieldDecl {
    pub name: String,
    pub span: Span,
    pub ty: TypeExpr,
}

#[derive(Debug, Clone)]
pub struct VariantDecl {
    pub name: String,
    pub span: Span,
    pub fields: Vec<FieldDecl>,
}

#[derive(Debug, Clone)]
pub enum TypeBody {
    Struct(Vec<FieldDecl>),
    Variants(Vec<VariantDecl>),
}

#[derive(Debug, Clone)]
pub struct TypeDecl {
    pub name: String,
    pub span: Span,
    pub public: bool,
    pub tparams: Vec<(String, Span)>,
    pub body: TypeBody,
}

#[derive(Debug, Clone)]
pub struct ConstDecl {
    pub name: String,
    pub span: Span,
    pub public: bool,
    pub ty: Option<TypeExpr>,
    pub value: Expr,
}

#[derive(Debug, Clone)]
pub struct Func {
    pub name: String,
    pub name_span: Span,
    pub public: bool,
    pub claw: bool,
    pub tparams: Vec<(String, Span)>,
    /// Scalar locals whose address is taken, so they must live in memory.
    pub addressed: RefCell<HashSet<String>>,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    /// Parsed since 0.2, checked from 0.5.
    pub contracts: Vec<Expr>,
    pub effects: Vec<String>,
    pub body: Block,
}

#[derive(Debug, Clone)]
pub struct Param {
    /// `own x T`: the function takes the value over.
    pub own: bool,
    pub name: String,
    pub span: Span,
    pub ty: TypeExpr,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub end: Span,
}

#[derive(Debug, Clone)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Else {
    Block(Block),
    If(Box<Stmt>),
}

#[derive(Debug, Clone)]
pub enum Pat {
    Wild,
    Variant { name: String, binds: Vec<(String, Span)> },
    Int(i128),
    Bool(bool),
}

#[derive(Debug, Clone)]
pub struct Arm {
    pub pat: Pat,
    pub span: Span,
    pub body: Block,
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    Let { mutable: bool, name: String, name_span: Span, ty: Option<TypeExpr>, init: Expr },
    Assign { target: Expr, op: Option<BinOp>, value: Expr },
    Discard(Expr),
    Expr(Expr),
    If { cond: Expr, then: Block, els: Option<Else> },
    ForRange { var: String, var_span: Span, start: Expr, end: Expr, body: Block },
    ForIn { elem: String, elem_span: Span, index: Option<(String, Span)>, subject: Expr, body: Block },
    ForCond { cond: Expr, body: Block },
    ForEver { body: Block },
    Match { subject: Expr, arms: Vec<Arm> },
    Ret(Option<Expr>),
    Fail(Expr),
    Defer(Block),
    Break,
    Continue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

impl BinOp {
    pub fn text(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "and",
            BinOp::Or => "or",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    BitNot,
    Not,
}

/// What a name turned out to mean, decided by sema and read by lowering.
#[derive(Debug, Clone, Default)]
pub enum Res {
    #[default]
    None,
    Func(String),
    Const(ConstVal, Ty),
    Variant(usize, usize),
}

/// An implicit wrap sema decided on: `T` used where `?T` or `T ! E` is expected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coerce {
    No,
    Some(Ty),
    Ok(Ty),
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    pub ty: Cell<Ty>,
    pub coerce: Cell<Coerce>,
    pub res: RefCell<Res>,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Expr {
        Expr { kind, span, ty: Cell::new(ERR), coerce: Cell::new(Coerce::No), res: RefCell::new(Res::None) }
    }
}

#[derive(Debug, Clone)]
pub enum Part {
    Lit(String),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub enum ElseKind {
    Value(Box<Expr>),
    Block(Option<(String, Span)>, Block),
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Int(i128),
    Float(f64),
    Char(u8),
    Bool(bool),
    Str(String),
    Interp(Vec<Part>),
    None,
    /// `ret` inside a `sure` contract.
    RetVal,
    Var(String),
    Unary { op: UnOp, e: Box<Expr> },
    Binary { op: BinOp, l: Box<Expr>, r: Box<Expr> },
    Call { module: Option<String>, name: String, name_span: Span, targs: Vec<TypeExpr>, args: Vec<Expr> },
    StructLit { module: Option<String>, name: String, name_span: Span, targs: Vec<TypeExpr>, args: Vec<Expr> },
    AddrOf(Box<Expr>),
    Deref(Box<Expr>),
    ArrayLit(Vec<Expr>),
    ArrayFill(Box<Expr>),
    Field { e: Box<Expr>, name: String, name_span: Span },
    Index { e: Box<Expr>, i: Box<Expr> },
    /// `a[lo..hi]`, either end optional: a read-only view.
    Slice { e: Box<Expr>, lo: Option<Box<Expr>>, hi: Option<Box<Expr>> },
    Try(Box<Expr>),
    Else { e: Box<Expr>, handler: ElseKind },
}
