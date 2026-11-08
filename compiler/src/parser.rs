//! Hand-written recursive descent with precedence climbing for expressions.

use crate::ast::*;
use crate::diag::{Diag, Span};
use crate::lexer::{Tok, Token};
use std::cell::Cell;

type PResult<T> = Result<T, ()>;

pub struct Parser<'a> {
    toks: Vec<Token>,
    pos: usize,
    diags: &'a mut Vec<Diag>,
}

fn mk(kind: ExprKind, span: Span) -> Expr {
    Expr { kind, span, ty: Cell::new(Ty::Err) }
}

fn join(a: Span, b: Span) -> Span {
    Span { start: a.start, end: b.end, line: a.line, col: a.col }
}

fn not_yet(what: &str) -> String {
    format!("{what} is not available in claw 0.1")
}

impl<'a> Parser<'a> {
    pub fn new(toks: Vec<Token>, diags: &'a mut Vec<Diag>) -> Parser<'a> {
        Parser { toks, pos: 0, diags }
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn peek_at(&self, off: usize) -> &Tok {
        let i = (self.pos + off).min(self.toks.len() - 1);
        &self.toks[i].tok
    }

    fn span(&self) -> Span {
        self.toks[self.pos].span
    }

    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn at_op(&self, op: &str) -> bool {
        matches!(self.peek(), Tok::Op(o) if *o == op)
    }

    fn at_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Tok::Kw(k) if *k == kw)
    }

    fn eat_op(&mut self, op: &str) -> bool {
        if self.at_op(op) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn describe(&self) -> String {
        match self.peek() {
            Tok::Int(v) => format!("number `{v}`"),
            Tok::Str(_) => "a string".to_string(),
            Tok::Ident(s) => format!("name `{s}`"),
            Tok::Kw(k) => format!("keyword `{k}`"),
            Tok::Op(o) => format!("`{o}`"),
            Tok::Newline => "end of line".to_string(),
            Tok::Eof => "end of file".to_string(),
        }
    }

    fn error<T>(&mut self, code: &'static str, span: Span, msg: String) -> PResult<T> {
        self.diags.push(Diag::new(code, span, msg));
        Err(())
    }

    fn expected<T>(&mut self, what: &str) -> PResult<T> {
        let msg = format!("expected {what}, found {}", self.describe());
        let span = self.span();
        self.error("E0201", span, msg)
    }

    fn expect_op(&mut self, op: &str) -> PResult<Token> {
        if self.at_op(op) {
            Ok(self.bump())
        } else {
            self.expected(&format!("`{op}`"))
        }
    }

    fn expect_ident(&mut self, what: &str) -> PResult<(String, Span)> {
        if let Tok::Ident(s) = self.peek() {
            let s = s.clone();
            let span = self.bump().span;
            Ok((s, span))
        } else if let Tok::Kw(k) = self.peek() {
            let msg = format!("`{k}` is a keyword and cannot be used as {what}");
            let span = self.span();
            self.error("E0202", span, msg)
        } else {
            self.expected(what)
        }
    }

    fn skip_newlines(&mut self) {
        while *self.peek() == Tok::Newline {
            self.bump();
        }
    }

    pub fn parse_program(&mut self) -> Program {
        let mut funcs = Vec::new();
        loop {
            self.skip_newlines();
            if *self.peek() == Tok::Eof {
                break;
            }
            if self.at_kw("fn") {
                match self.parse_func() {
                    Ok(f) => funcs.push(f),
                    Err(()) => self.sync_top(),
                }
            } else {
                let span = self.span();
                if let Tok::Kw(k) = self.peek() {
                    let k = *k;
                    if matches!(k, "type" | "use" | "pub" | "const" | "test" | "claw" | "extern") {
                        self.diags.push(Diag::new("E0900", span, not_yet(&format!("`{k}`"))));
                        self.bump();
                        self.sync_top();
                        continue;
                    }
                }
                let msg = format!("expected `fn`, found {}", self.describe());
                self.diags.push(Diag::new("E0201", span, msg));
                self.bump();
                self.sync_top();
            }
        }
        funcs_program(funcs)
    }

    /// Recover at the next `fn` that starts a line.
    fn sync_top(&mut self) {
        loop {
            match self.peek() {
                Tok::Eof => return,
                Tok::Kw("fn") if self.pos == 0 || self.toks[self.pos - 1].tok == Tok::Newline => return,
                _ => {
                    self.bump();
                }
            }
        }
    }

    fn parse_type(&mut self) -> PResult<TypeRef> {
        if let Tok::Op(o) = self.peek() {
            if matches!(*o, "[" | "?" | "*" | "&") {
                let (o, span) = (*o, self.span());
                return self.error("E0900", span, not_yet(&format!("type syntax `{o}`")));
            }
        }
        let (name, span) = self.expect_ident("a type")?;
        Ok(TypeRef { name, span })
    }

    fn parse_func(&mut self) -> PResult<Func> {
        self.bump();
        let (name, name_span) = self.expect_ident("a function name")?;
        if self.at_op("[") {
            let span = self.span();
            return self.error("E0900", span, not_yet("generics"));
        }
        self.expect_op("(")?;
        let mut params = Vec::new();
        while !self.at_op(")") {
            if self.at_kw("own") {
                let span = self.span();
                return self.error("E0900", span, not_yet("`own`"));
            }
            let (pname, span) = self.expect_ident("a parameter name")?;
            let ty = self.parse_type()?;
            params.push(Param { name: pname, span, ty });
            if !self.eat_op(",") {
                break;
            }
        }
        self.expect_op(")")?;
        let ret = if matches!(self.peek(), Tok::Ident(_)) || self.at_op("[") || self.at_op("?") || self.at_op("*") {
            Some(self.parse_type()?)
        } else {
            None
        };
        if self.at_op("!") || self.at_op("+") {
            let span = self.span();
            let what = if self.at_op("!") { "error returns `!`" } else { "effects `+mem`, `+io`" };
            return self.error("E0900", span, not_yet(what));
        }
        self.skip_newlines();
        if self.at_kw("need") || self.at_kw("sure") {
            let span = self.span();
            return self.error("E0900", span, not_yet("contracts `need`, `sure`"));
        }
        let body = self.parse_block()?;
        Ok(Func { name, name_span, params, ret, body })
    }

    fn parse_block(&mut self) -> PResult<Block> {
        self.expect_op("{")?;
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if self.at_op("}") {
                break;
            }
            if *self.peek() == Tok::Eof {
                let span = self.span();
                return self.error("E0204", span, "block is not closed; add `}`".to_string());
            }
            let stmt = self.parse_stmt()?;
            stmts.push(stmt);
            if !self.at_op("}") && *self.peek() != Tok::Newline {
                let msg = format!("one statement per line; found {} after the statement", self.describe());
                let span = self.span();
                return self.error("E0205", span, msg);
            }
        }
        let end = self.bump().span;
        Ok(Block { stmts, end })
    }

    fn parse_stmt(&mut self) -> PResult<Stmt> {
        let start = self.span();
        match self.peek().clone() {
            Tok::Kw("mut") => {
                self.bump();
                self.parse_let(start, true)
            }
            Tok::Kw("if") => self.parse_if(),
            Tok::Kw("for") => self.parse_for(),
            Tok::Kw("ret") => {
                self.bump();
                let value = if *self.peek() == Tok::Newline || self.at_op("}") {
                    None
                } else {
                    Some(self.parse_expr(0)?)
                };
                Ok(Stmt { kind: StmtKind::Ret(value), span: start })
            }
            Tok::Kw("break") => {
                self.bump();
                Ok(Stmt { kind: StmtKind::Break, span: start })
            }
            Tok::Kw("continue") => {
                self.bump();
                Ok(Stmt { kind: StmtKind::Continue, span: start })
            }
            Tok::Kw("else") => self.error(
                "E0203",
                start,
                "`else` must be on the same line as the closing `}`".to_string(),
            ),
            Tok::Kw(k) if matches!(k, "match" | "fail" | "defer" | "need" | "sure") => {
                self.error("E0900", start, not_yet(&format!("`{k}`")))
            }
            Tok::Ident(name) => match self.peek_at(1).clone() {
                Tok::Op(":=") => self.parse_let(start, false),
                Tok::Ident(_) => self.parse_let(start, false),
                Tok::Op("=") if name == "_" => {
                    self.bump();
                    self.bump();
                    let e = self.parse_expr(0)?;
                    Ok(Stmt { kind: StmtKind::Discard(e), span: start })
                }
                Tok::Op(op) if assign_op(op).is_some() => {
                    let (name, name_span) = self.expect_ident("a name")?;
                    self.bump();
                    let value = self.parse_expr(0)?;
                    let op = assign_op(op).unwrap();
                    Ok(Stmt { kind: StmtKind::Assign { name, name_span, op, value }, span: start })
                }
                _ => self.parse_expr_stmt(start),
            },
            _ => self.parse_expr_stmt(start),
        }
    }

    fn parse_expr_stmt(&mut self, start: Span) -> PResult<Stmt> {
        let e = self.parse_expr(0)?;
        Ok(Stmt { kind: StmtKind::Expr(e), span: start })
    }

    fn parse_let(&mut self, start: Span, mutable: bool) -> PResult<Stmt> {
        let (name, name_span) = self.expect_ident("a variable name")?;
        let ty = if self.at_op(":=") {
            self.bump();
            None
        } else {
            let ty = self.parse_type()?;
            self.expect_op("=")?;
            Some(ty)
        };
        let init = self.parse_expr(0)?;
        Ok(Stmt { kind: StmtKind::Let { mutable, name, name_span, ty, init }, span: start })
    }

    fn parse_if(&mut self) -> PResult<Stmt> {
        let start = self.bump().span;
        let cond = self.parse_expr(0)?;
        let then = self.parse_block()?;
        let els = if self.at_kw("else") {
            self.bump();
            if self.at_kw("if") {
                Some(Else::If(Box::new(self.parse_if()?)))
            } else {
                Some(Else::Block(self.parse_block()?))
            }
        } else {
            None
        };
        Ok(Stmt { kind: StmtKind::If { cond, then, els }, span: start })
    }

    fn parse_for(&mut self) -> PResult<Stmt> {
        let start = self.bump().span;
        if self.at_op("{") {
            let body = self.parse_block()?;
            return Ok(Stmt { kind: StmtKind::ForEver { body }, span: start });
        }
        if matches!(self.peek(), Tok::Ident(_)) && matches!(self.peek_at(1), Tok::Kw("in")) {
            let (var, var_span) = self.expect_ident("a loop variable")?;
            self.bump();
            let from = self.parse_expr(0)?;
            if !self.at_op("..") {
                let span = from.span;
                return self.error("E0900", span, not_yet("`for` over a collection"));
            }
            self.bump();
            let to = self.parse_expr(0)?;
            let body = self.parse_block()?;
            return Ok(Stmt {
                kind: StmtKind::ForRange { var, var_span, start: from, end: to, body },
                span: start,
            });
        }
        let cond = self.parse_expr(0)?;
        let body = self.parse_block()?;
        Ok(Stmt { kind: StmtKind::ForCond { cond, body }, span: start })
    }

    fn peek_binop(&self) -> Option<(BinOp, u8)> {
        Some(match self.peek() {
            Tok::Kw("or") => (BinOp::Or, 1),
            Tok::Kw("and") => (BinOp::And, 2),
            Tok::Op(o) => match *o {
                "==" => (BinOp::Eq, 3),
                "!=" => (BinOp::Ne, 3),
                "<" => (BinOp::Lt, 3),
                "<=" => (BinOp::Le, 3),
                ">" => (BinOp::Gt, 3),
                ">=" => (BinOp::Ge, 3),
                "+" => (BinOp::Add, 4),
                "-" => (BinOp::Sub, 4),
                "|" => (BinOp::BitOr, 4),
                "^" => (BinOp::BitXor, 4),
                "*" => (BinOp::Mul, 5),
                "/" => (BinOp::Div, 5),
                "%" => (BinOp::Rem, 5),
                "<<" => (BinOp::Shl, 5),
                ">>" => (BinOp::Shr, 5),
                "&" => (BinOp::BitAnd, 5),
                _ => return None,
            },
            _ => return None,
        })
    }

    fn parse_expr(&mut self, min_prec: u8) -> PResult<Expr> {
        let mut left = self.parse_unary()?;
        while let Some((op, prec)) = self.peek_binop() {
            if prec < min_prec {
                break;
            }
            self.bump();
            let right = self.parse_expr(prec + 1)?;
            let span = join(left.span, right.span);
            left = mk(ExprKind::Binary { op, l: Box::new(left), r: Box::new(right) }, span);
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> PResult<Expr> {
        let start = self.span();
        if self.at_kw("not") {
            self.bump();
            // Like Python: `not a == b` means `not (a == b)`.
            let e = self.parse_expr(3)?;
            let span = join(start, e.span);
            return Ok(mk(ExprKind::Unary { op: UnOp::Not, e: Box::new(e) }, span));
        }
        if self.at_op("-") {
            self.bump();
            if let Tok::Int(v) = self.peek().clone() {
                let tok_span = self.bump().span;
                let span = join(start, tok_span);
                return self.parse_postfix(mk(ExprKind::Int(-v), span));
            }
            let e = self.parse_unary()?;
            let span = join(start, e.span);
            return Ok(mk(ExprKind::Unary { op: UnOp::Neg, e: Box::new(e) }, span));
        }
        if self.at_op("~") {
            self.bump();
            let e = self.parse_unary()?;
            let span = join(start, e.span);
            return Ok(mk(ExprKind::Unary { op: UnOp::BitNot, e: Box::new(e) }, span));
        }
        if self.at_op("&") || self.at_op("*") {
            let msg = not_yet("references and pointers");
            return self.error("E0900", start, msg);
        }
        let e = self.parse_primary()?;
        self.parse_postfix(e)
    }

    fn parse_postfix(&mut self, e: Expr) -> PResult<Expr> {
        let span = self.span();
        if self.at_op(".") {
            return self.error("E0900", span, not_yet("field access"));
        }
        if self.at_op("[") {
            return self.error("E0900", span, not_yet("indexing"));
        }
        if self.at_op("?") {
            return self.error("E0900", span, not_yet("`?`"));
        }
        if self.at_kw("else") {
            return self.error("E0900", span, not_yet("`else` on a value"));
        }
        Ok(e)
    }

    fn parse_primary(&mut self) -> PResult<Expr> {
        let span = self.span();
        match self.peek().clone() {
            Tok::Int(v) => {
                self.bump();
                Ok(mk(ExprKind::Int(v), span))
            }
            Tok::Str(s) => {
                self.bump();
                Ok(mk(ExprKind::Str(s), span))
            }
            Tok::Kw("true") => {
                self.bump();
                Ok(mk(ExprKind::Bool(true), span))
            }
            Tok::Kw("false") => {
                self.bump();
                Ok(mk(ExprKind::Bool(false), span))
            }
            Tok::Kw("none") => self.error("E0900", span, not_yet("`none`")),
            Tok::Ident(name) => {
                self.bump();
                if self.at_op("(") {
                    self.bump();
                    let mut args = Vec::new();
                    while !self.at_op(")") {
                        args.push(self.parse_expr(0)?);
                        if !self.eat_op(",") {
                            break;
                        }
                    }
                    let close = self.expect_op(")")?;
                    Ok(mk(ExprKind::Call { name, name_span: span, args }, join(span, close.span)))
                } else {
                    Ok(mk(ExprKind::Var(name), span))
                }
            }
            Tok::Op("(") => {
                self.bump();
                let e = self.parse_expr(0)?;
                self.expect_op(")")?;
                Ok(e)
            }
            _ => self.expected("an expression"),
        }
    }
}

fn funcs_program(funcs: Vec<Func>) -> Program {
    Program { funcs }
}

/// `Some(None)` is plain `=`, `Some(Some(op))` is a compound assignment.
fn assign_op(op: &str) -> Option<Option<BinOp>> {
    Some(match op {
        "=" => None,
        "+=" => Some(BinOp::Add),
        "-=" => Some(BinOp::Sub),
        "*=" => Some(BinOp::Mul),
        "/=" => Some(BinOp::Div),
        "%=" => Some(BinOp::Rem),
        "&=" => Some(BinOp::BitAnd),
        "|=" => Some(BinOp::BitOr),
        "^=" => Some(BinOp::BitXor),
        "<<=" => Some(BinOp::Shl),
        ">>=" => Some(BinOp::Shr),
        _ => return None,
    })
}
