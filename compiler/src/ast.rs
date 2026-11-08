//! Syntax tree. Sema writes each expression's type into `Expr::ty`.

use crate::diag::Span;
use crate::types::{Ty, ERR};
use std::cell::Cell;

#[derive(Debug)]
pub enum Size {
    Lit(i128),
    Name(String),
}

#[derive(Debug)]
pub enum TypeKind {
    Name(String),
    Array(Size, Box<TypeExpr>),
    Option(Box<TypeExpr>),
    /// `T ! E`, or `! E` when the success value is nothing.
    Result(Option<Box<TypeExpr>>, Box<TypeExpr>),
}

#[derive(Debug)]
pub struct TypeExpr {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Debug, Default)]
pub struct Program {
    pub funcs: Vec<Func>,
    pub types: Vec<TypeDecl>,
    pub consts: Vec<ConstDecl>,
}

#[derive(Debug)]
pub struct FieldDecl {
    pub name: String,
    pub span: Span,
    pub ty: TypeExpr,
}

#[derive(Debug)]
pub struct VariantDecl {
    pub name: String,
    pub span: Span,
    pub fields: Vec<FieldDecl>,
}

#[derive(Debug)]
pub enum TypeBody {
    Struct(Vec<FieldDecl>),
    Variants(Vec<VariantDecl>),
}

#[derive(Debug)]
pub struct TypeDecl {
    pub name: String,
    pub span: Span,
    pub body: TypeBody,
}

#[derive(Debug)]
pub struct ConstDecl {
    pub name: String,
    pub span: Span,
    pub ty: Option<TypeExpr>,
    pub value: Expr,
}

#[derive(Debug)]
pub struct Func {
    pub name: String,
    pub name_span: Span,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    /// Parsed since 0.2, checked from 0.5.
    pub contracts: Vec<Expr>,
    pub effects: Vec<String>,
    pub body: Block,
}

#[derive(Debug)]
pub struct Param {
    pub name: String,
    pub span: Span,
    pub ty: TypeExpr,
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
pub enum Pat {
    Wild,
    Variant { name: String, binds: Vec<(String, Span)> },
    Int(i128),
    Bool(bool),
}

#[derive(Debug)]
pub struct Arm {
    pub pat: Pat,
    pub span: Span,
    pub body: Block,
}

#[derive(Debug)]
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

/// An implicit wrap sema decided on: `T` used where `?T` or `T ! E` is expected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coerce {
    No,
    Some(Ty),
    Ok(Ty),
}

#[derive(Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    pub ty: Cell<Ty>,
    pub coerce: Cell<Coerce>,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Expr {
        Expr { kind, span, ty: Cell::new(ERR), coerce: Cell::new(Coerce::No) }
    }
}

#[derive(Debug)]
pub enum Part {
    Lit(String),
    Expr(Expr),
}

#[derive(Debug)]
pub enum ElseKind {
    Value(Box<Expr>),
    Block(Option<(String, Span)>, Block),
}

#[derive(Debug)]
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
    Call { name: String, name_span: Span, args: Vec<Expr> },
    StructLit { name: String, name_span: Span, args: Vec<Expr> },
    ArrayLit(Vec<Expr>),
    ArrayFill(Box<Expr>),
    Field { e: Box<Expr>, name: String, name_span: Span },
    Index { e: Box<Expr>, i: Box<Expr> },
    Try(Box<Expr>),
    Else { e: Box<Expr>, handler: ElseKind },
}
