//! Name resolution, type checking and the strictness rules.

use crate::ast::*;
use crate::diag::{Diag, Span};
use std::collections::HashMap;

pub struct FnSig {
    pub params: Vec<Ty>,
    pub ret: Ty,
}

struct Var {
    ty: Ty,
    mutable: bool,
    read: bool,
    written: bool,
    span: Span,
    /// Where a `mut ` fix would go, or the `mut` to remove.
    decl: Span,
}

struct Ctx<'a> {
    sigs: &'a HashMap<String, FnSig>,
    scopes: Vec<HashMap<String, Var>>,
    diags: &'a mut Vec<Diag>,
    ret: Ty,
    loops: u32,
}

const RESERVED_TYPES: &[&str] = &["i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "str"];
const BUILTINS: &[&str] = &["print"];

fn resolve_type(t: &TypeRef, diags: &mut Vec<Diag>) -> Ty {
    match t.name.as_str() {
        "int" => Ty::Int,
        "bool" => Ty::Bool,
        n if RESERVED_TYPES.contains(&n) => {
            diags.push(Diag::new("E0900", t.span, format!("type `{n}` is not available in claw 0.1")));
            Ty::Err
        }
        n => {
            diags.push(Diag::new("E0112", t.span, format!("unknown type `{n}`")));
            Ty::Err
        }
    }
}

pub fn check(p: &Program, diags: &mut Vec<Diag>) -> HashMap<String, FnSig> {
    let mut sigs: HashMap<String, FnSig> = HashMap::new();
    for f in &p.funcs {
        if BUILTINS.contains(&f.name.as_str()) {
            diags.push(Diag::new("E0102", f.name_span, format!("`{}` is a built-in name", f.name)));
            continue;
        }
        if sigs.contains_key(&f.name) {
            diags.push(Diag::new("E0110", f.name_span, format!("function `{}` is defined twice", f.name)));
            continue;
        }
        let params = f.params.iter().map(|x| resolve_type(&x.ty, diags)).collect();
        let ret = f.ret.as_ref().map(|t| resolve_type(t, diags)).unwrap_or(Ty::Void);
        sigs.insert(f.name.clone(), FnSig { params, ret });
    }

    match p.funcs.iter().find(|f| f.name == "main") {
        None => diags.push(Diag::new("E0109", Span { start: 0, end: 0, line: 1, col: 1 }, "no `fn main()`; every program needs one")),
        Some(f) => {
            if !f.params.is_empty() || f.ret.is_some() {
                diags.push(Diag::new("E0109", f.name_span, "`main` takes no parameters and returns nothing"));
            }
        }
    }

    for f in &p.funcs {
        let ret = f.ret.as_ref().map(|t| {
            let mut sink = Vec::new();
            resolve_type(t, &mut sink)
        });
        let mut cx = Ctx { sigs: &sigs, scopes: vec![HashMap::new()], diags: &mut *diags, ret: ret.unwrap_or(Ty::Void), loops: 0 };
        for x in &f.params {
            let mut sink = Vec::new();
            let ty = resolve_type(&x.ty, &mut sink);
            if x.name != "_" {
                cx.declare(&x.name, ty, false, x.span, x.span);
            }
        }
        let ends = cx.block(&f.body);
        cx.pop_scope();
        if !ends && cx.ret != Ty::Void && cx.ret != Ty::Err {
            let msg = format!("`{}` can reach its end without `ret`", f.name);
            cx.diags.push(Diag::new("E0107", f.body.end, msg));
        }
    }
    sigs
}

impl<'a> Ctx<'a> {
    fn lookup(&mut self, name: &str) -> Option<&mut Var> {
        self.scopes.iter_mut().rev().find_map(|s| s.get_mut(name))
    }

    fn declare(&mut self, name: &str, ty: Ty, mutable: bool, span: Span, decl: Span) {
        if self.lookup(name).is_some() || BUILTINS.contains(&name) || self.sigs.contains_key(name) {
            self.diags.push(Diag::new("E0102", span, format!("`{name}` is already defined; names cannot be reused or hidden")));
            if self.lookup(name).is_none() {
                // It clashes with a function: keep the variable so its uses don't cascade.
                let v = Var { ty, mutable, read: true, written: true, span, decl };
                self.scopes.last_mut().unwrap().insert(name.to_string(), v);
            }
            return;
        }
        let v = Var { ty, mutable, read: false, written: false, span, decl };
        self.scopes.last_mut().unwrap().insert(name.to_string(), v);
    }

    fn pop_scope(&mut self) {
        let scope = self.scopes.pop().unwrap();
        for (name, v) in scope {
            if !v.read {
                let d = Diag::new("E0105", v.span, format!("`{name}` is never read"))
                    .fix(v.span, "_", format!("rename `{name}` to `_`"));
                self.diags.push(d);
            } else if v.mutable && !v.written {
                let d = Diag::new("E0116", v.span, format!("`{name}` is `mut` but never changes"))
                    .fix(v.decl, "", "remove `mut`");
                self.diags.push(d);
            }
        }
    }

    /// Returns true if control can never run past the end of the block.
    fn block(&mut self, b: &Block) -> bool {
        self.scopes.push(HashMap::new());
        let mut ends = false;
        let mut reported = false;
        for s in &b.stmts {
            if ends && !reported {
                self.diags.push(Diag::new("E0113", s.span, "this code can never run"));
                reported = true;
            }
            if self.stmt(s) {
                ends = true;
            }
        }
        self.pop_scope();
        ends
    }

    fn expect(&mut self, e: &Expr, want: Ty) {
        let got = self.expr(e);
        self.require(e.span, got, want);
    }

    fn require(&mut self, span: Span, got: Ty, want: Ty) {
        if got != want && got != Ty::Err && want != Ty::Err {
            self.diags.push(Diag::new("E0104", span, format!("expected {}, found {}", want.name(), got.name())));
        }
    }

    fn value_type(&mut self, span: Span, ty: Ty) -> Ty {
        match ty {
            Ty::Void => {
                self.diags.push(Diag::new("E0104", span, "this call returns nothing, so it has no value"));
                Ty::Err
            }
            Ty::StrLit => {
                self.diags.push(Diag::new("E0900", span, "strings outside `print` are not available in claw 0.1"));
                Ty::Err
            }
            t => t,
        }
    }

    fn stmt(&mut self, s: &Stmt) -> bool {
        match &s.kind {
            StmtKind::Let { mutable, name, name_span, ty, init } => {
                let got = self.expr(init);
                let got = self.value_type(init.span, got);
                let final_ty = match ty {
                    Some(t) => {
                        let want = resolve_type(t, self.diags);
                        self.require(init.span, got, want);
                        want
                    }
                    None => got,
                };
                let decl = if *mutable {
                    Span { start: s.span.start, end: name_span.start, line: s.span.line, col: s.span.col }
                } else {
                    Span { start: s.span.start, end: s.span.start, line: s.span.line, col: s.span.col }
                };
                self.declare(name, final_ty, *mutable, *name_span, decl);
                false
            }
            StmtKind::Assign { name, name_span, op, value } => {
                let got = self.expr(value);
                let var = self.lookup(name).map(|v| (v.ty, v.mutable, v.decl));
                let (ty, mutable, decl) = match var {
                    Some(v) => v,
                    None => {
                        self.diags.push(Diag::new("E0101", *name_span, format!("unknown name `{name}`; declare it with `{name} := ...`")));
                        return false;
                    }
                };
                if !mutable {
                    let d = Diag::new("E0103", *name_span, format!("`{name}` cannot change"))
                        .fix(decl, "mut ", format!("declare `mut {name}`"));
                    self.diags.push(d);
                }
                self.lookup(name).unwrap().written = true;
                if op.is_some() && ty != Ty::Int && ty != Ty::Err {
                    self.diags.push(Diag::new("E0104", *name_span, format!("`{}=` needs int, found {}", op.unwrap().text(), ty.name())));
                }
                self.require(value.span, got, ty);
                false
            }
            StmtKind::Discard(e) => {
                let got = self.expr(e);
                self.value_type(e.span, got);
                false
            }
            StmtKind::Expr(e) => {
                let got = self.expr(e);
                match &e.kind {
                    ExprKind::Call { .. } => {
                        if got != Ty::Void && got != Ty::Err {
                            let d = Diag::new("E0111", e.span, "result is not used")
                                .fix(e.span, "_ = ", "assign it to `_` if it really does not matter");
                            self.diags.push(d);
                        }
                    }
                    _ => self.diags.push(Diag::new("E0114", e.span, "this expression does nothing")),
                }
                false
            }
            StmtKind::If { cond, then, els } => {
                self.expect(cond, Ty::Bool);
                let a = self.block(then);
                match els {
                    None => false,
                    Some(Else::Block(b)) => {
                        let c = self.block(b);
                        a && c
                    }
                    Some(Else::If(i)) => {
                        let c = self.stmt(i);
                        a && c
                    }
                }
            }
            StmtKind::ForRange { var, var_span, start, end, body } => {
                self.expect(start, Ty::Int);
                self.expect(end, Ty::Int);
                self.scopes.push(HashMap::new());
                if var != "_" {
                    self.declare(var, Ty::Int, false, *var_span, *var_span);
                }
                self.loops += 1;
                self.block(body);
                self.loops -= 1;
                self.pop_scope();
                false
            }
            StmtKind::ForCond { cond, body } => {
                self.expect(cond, Ty::Bool);
                self.loops += 1;
                self.block(body);
                self.loops -= 1;
                false
            }
            StmtKind::ForEver { body } => {
                self.loops += 1;
                self.block(body);
                self.loops -= 1;
                !has_break(body)
            }
            StmtKind::Ret(v) => {
                match v {
                    Some(e) => {
                        let got = self.expr(e);
                        if self.ret == Ty::Void {
                            self.diags.push(Diag::new("E0104", e.span, "this function returns nothing; write `ret` alone"));
                        } else {
                            let got = self.value_type(e.span, got);
                            self.require(e.span, got, self.ret);
                        }
                    }
                    None => {
                        if self.ret != Ty::Void && self.ret != Ty::Err {
                            self.diags.push(Diag::new("E0104", s.span, format!("this function must return {}", self.ret.name())));
                        }
                    }
                }
                true
            }
            StmtKind::Break | StmtKind::Continue => {
                if self.loops == 0 {
                    self.diags.push(Diag::new("E0108", s.span, "`break` and `continue` only work inside `for`"));
                }
                true
            }
        }
    }

    fn expr(&mut self, e: &Expr) -> Ty {
        let t = self.expr_inner(e);
        e.ty.set(t);
        t
    }

    fn expr_inner(&mut self, e: &Expr) -> Ty {
        match &e.kind {
            ExprKind::Int(v) => {
                if *v > i64::MAX as i128 || *v < i64::MIN as i128 {
                    self.diags.push(Diag::new("E0115", e.span, "number does not fit in 64-bit int"));
                    return Ty::Err;
                }
                Ty::Int
            }
            ExprKind::Bool(_) => Ty::Bool,
            ExprKind::Str(_) => Ty::StrLit,
            ExprKind::Var(name) => {
                if let Some(v) = self.lookup(name) {
                    v.read = true;
                    v.ty
                } else {
                    let msg = if name == "_" {
                        "`_` can only be assigned to, not read".to_string()
                    } else {
                        format!("unknown name `{name}`")
                    };
                    self.diags.push(Diag::new("E0101", e.span, msg));
                    Ty::Err
                }
            }
            ExprKind::Unary { op, e: inner } => {
                let want = if *op == UnOp::Not { Ty::Bool } else { Ty::Int };
                self.expect(inner, want);
                want
            }
            ExprKind::Binary { op, l, r } => self.binary(*op, l, r),
            ExprKind::Call { name, name_span, args } => self.call(name, *name_span, args),
        }
    }

    fn binary(&mut self, op: BinOp, l: &Expr, r: &Expr) -> Ty {
        match op {
            BinOp::And | BinOp::Or => {
                self.expect(l, Ty::Bool);
                self.expect(r, Ty::Bool);
                Ty::Bool
            }
            BinOp::Eq | BinOp::Ne => {
                let a = self.expr(l);
                let b = self.expr(r);
                let a = self.value_type(l.span, a);
                let b = self.value_type(r.span, b);
                self.require(r.span, b, a);
                Ty::Bool
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                self.expect(l, Ty::Int);
                self.expect(r, Ty::Int);
                Ty::Bool
            }
            _ => {
                self.expect(l, Ty::Int);
                self.expect(r, Ty::Int);
                Ty::Int
            }
        }
    }

    fn call(&mut self, name: &str, span: Span, args: &[Expr]) -> Ty {
        if name == "print" {
            if args.len() != 1 {
                self.diags.push(Diag::new("E0106", span, format!("`print` takes 1 value, found {}", args.len())));
            }
            for a in args {
                let got = self.expr(a);
                if let ExprKind::Str(s) = &a.kind {
                    if s.contains('{') {
                        self.diags.push(Diag::new("E0900", a.span, "`{x}` inside strings is not available in claw 0.1"));
                    }
                } else if got == Ty::Void {
                    self.value_type(a.span, got);
                }
            }
            return Ty::Void;
        }
        let sigs = self.sigs;
        let sig = match sigs.get(name) {
            Some(s) => s,
            None => {
                for a in args {
                    self.expr(a);
                }
                self.diags.push(Diag::new("E0101", span, format!("unknown function `{name}`")));
                return Ty::Err;
            }
        };
        if sig.params.len() != args.len() {
            self.diags.push(Diag::new("E0106", span, format!("`{name}` takes {} values, found {}", sig.params.len(), args.len())));
        }
        for (i, a) in args.iter().enumerate() {
            let got = self.expr(a);
            let got = self.value_type(a.span, got);
            if let Some(want) = sig.params.get(i) {
                self.require(a.span, got, *want);
            }
        }
        sig.ret
    }
}

/// Does a `break` inside this block leave *this* loop?
fn has_break(b: &Block) -> bool {
    b.stmts.iter().any(stmt_breaks)
}

fn stmt_breaks(s: &Stmt) -> bool {
    match &s.kind {
        StmtKind::Break => true,
        StmtKind::If { then, els, .. } => {
            has_break(then)
                || match els {
                    Some(Else::Block(b)) => has_break(b),
                    Some(Else::If(i)) => stmt_breaks(i),
                    None => false,
                }
        }
        _ => false,
    }
}
