//! Syntax tree. Sema writes each expression's type into `Expr::ty`.

use crate::diag::Span;
use std::cell::Cell;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    Int,
    Bool,
    Void,
    /// A string literal. In 0.1 it can only appear directly inside `print`.
    StrLit,
    /// Already reported; suppresses follow-up errors.
    Err,
}

impl Ty {
    pub fn name(self) -> &'static str {
        match self {
            Ty::Int => "int",
            Ty::Bool => "bool",
            Ty::Void => "nothing",
            Ty::StrLit => "string literal",
            Ty::Err => "<error>",
        }
    }
}

#[derive(Debug)]
pub struct TypeRef {
    pub name: String,
    pub span: Span,
}

#[derive(Debug)]
pub struct Program {
    pub funcs: Vec<Func>,
}

#[derive(Debug)]
pub struct Func {
    pub name: String,
    pub name_span: Span,
    pub params: Vec<Param>,
    pub ret: Option<TypeRef>,
    pub body: Block,
}

#[derive(Debug)]
pub struct Param {
    pub name: String,
    pub span: Span,
    pub ty: TypeRef,
}

#[derive(Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub end: Span,
}

#[derive(Debug)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug)]
pub enum Else {
    Block(Block),
    If(Box<Stmt>),
}

#[derive(Debug)]
pub enum StmtKind {
    Let { mutable: bool, name: String, name_span: Span, ty: Option<TypeRef>, init: Expr },
    Assign { name: String, name_span: Span, op: Option<BinOp>, value: Expr },
    Discard(Expr),
    Expr(Expr),
    If { cond: Expr, then: Block, els: Option<Else> },
    ForRange { var: String, var_span: Span, start: Expr, end: Expr, body: Block },
    ForCond { cond: Expr, body: Block },
    ForEver { body: Block },
    Ret(Option<Expr>),
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

#[derive(Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    pub ty: Cell<Ty>,
}

#[derive(Debug)]
pub enum ExprKind {
    Int(i128),
    Bool(bool),
    Str(String),
    Var(String),
    Unary { op: UnOp, e: Box<Expr> },
    Binary { op: BinOp, l: Box<Expr>, r: Box<Expr> },
    Call { name: String, name_span: Span, args: Vec<Expr> },
}
