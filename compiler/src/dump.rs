//! `claw __ast file`: the syntax tree as text, to compare with the parser written in Claw.
//! `claw __ast --load file`: the same for every unit `modules::load` finds.
//! `claw __sema [--check] [--tests] file`: what sema and the effect check worked out, or their errors.
//! `claw __cir [--check] [--tests] [--fast] file`: the program right after lowering.

use crate::ast::*;
use crate::diag::{self, Diag, Span};
use crate::sema::{Checked, ConstVal, FnSig};
use crate::types::{Kind, Ty, Types};
use crate::{cir, ctfe, effects, lexer, lower, modules, parser, sema};
use std::fmt::Write;

fn atom(o: &mut String, s: &str) {
    o.push(' ');
    o.push_str(s);
}

fn open(o: &mut String, name: &str) {
    o.push_str(" (");
    o.push_str(name);
}

fn close(o: &mut String) {
    o.push(')');
}

fn int(o: &mut String, v: u64) {
    let _ = write!(o, " {v}");
}

fn flag(o: &mut String, v: bool) {
    atom(o, if v { "t" } else { "f" });
}

/// Past 64 bits the compiler written in Claw only knows that a number is big.
fn number(o: &mut String, v: i128) {
    if v.unsigned_abs() > u64::MAX as u128 {
        atom(o, if v < 0 { "-big" } else { "big" });
    } else {
        let _ = write!(o, " {v}");
    }
}

fn span(o: &mut String, s: Span) {
    let _ = write!(o, " @{},{},{},{}", s.start, s.end, s.line, s.col);
}

fn quote(o: &mut String, s: &str) {
    o.push_str(" \"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            '\r' => o.push_str("\\r"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let _ = write!(o, "\\x{:02x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
}

fn opt_str(o: &mut String, s: &Option<String>) {
    match s {
        Some(s) => quote(o, s),
        None => atom(o, "-"),
    }
}

fn bind(o: &mut String, b: &(String, Span)) {
    quote(o, &b.0);
    span(o, b.1);
}

fn opt_bind(o: &mut String, b: &Option<(String, Span)>) {
    match b {
        Some(b) => bind(o, b),
        None => atom(o, "-"),
    }
}

fn strs(o: &mut String, name: &str, items: &[String]) {
    open(o, name);
    for s in items {
        quote(o, s);
    }
    close(o);
}

fn binds(o: &mut String, name: &str, items: &[(String, Span)]) {
    open(o, name);
    for b in items {
        bind(o, b);
    }
    close(o);
}

fn ty(o: &mut String, t: &TypeExpr) {
    match &t.kind {
        TypeKind::Name { module, name, targs } => {
            open(o, "name");
            opt_str(o, module);
            quote(o, name);
            types(o, targs);
        }
        TypeKind::Pointer(x) => {
            open(o, "ptr");
            ty(o, x);
        }
        TypeKind::Ref(x) => {
            open(o, "ref");
            ty(o, x);
        }
        TypeKind::Slice(x) => {
            open(o, "slice");
            ty(o, x);
        }
        TypeKind::Array(size, x) => {
            open(o, "array");
            match size {
                Size::Lit(v) => number(o, *v),
                Size::Name(n) => quote(o, n),
            }
            ty(o, x);
        }
        TypeKind::Option(x) => {
            open(o, "opt");
            ty(o, x);
        }
        TypeKind::Result(ok, e) => {
            open(o, "result");
            match ok {
                Some(x) => ty(o, x),
                None => atom(o, "-"),
            }
            ty(o, e);
        }
        TypeKind::Fn { params, ret, effects } => {
            open(o, "fnty");
            open(o, "params");
            for (own, t) in params {
                flag(o, *own);
                ty(o, t);
            }
            close(o);
            match ret {
                Some(x) => ty(o, x),
                None => atom(o, "-"),
            }
            strs(o, "effects", effects);
        }
    }
    span(o, t.span);
    close(o);
}

fn types(o: &mut String, items: &[TypeExpr]) {
    open(o, "targs");
    for t in items {
        ty(o, t);
    }
    close(o);
}

fn opt_ty(o: &mut String, t: &Option<TypeExpr>) {
    match t {
        Some(t) => ty(o, t),
        None => atom(o, "-"),
    }
}

fn exprs(o: &mut String, items: &[Expr]) {
    open(o, "args");
    for e in items {
        expr(o, e);
    }
    close(o);
}

fn opt_expr(o: &mut String, e: &Option<Box<Expr>>) {
    match e {
        Some(e) => expr(o, e),
        None => atom(o, "-"),
    }
}

fn expr(o: &mut String, e: &Expr) {
    match &e.kind {
        ExprKind::Int(v) => {
            open(o, "int");
            number(o, *v);
        }
        ExprKind::Float(v) => {
            open(o, "float");
            int(o, v.to_bits());
        }
        ExprKind::Char(c) => {
            open(o, "char");
            int(o, *c as u64);
        }
        ExprKind::Bool(v) => {
            open(o, "bool");
            flag(o, *v);
        }
        ExprKind::Str(s) => {
            open(o, "str");
            quote(o, s);
        }
        ExprKind::Interp(parts) => {
            open(o, "interp");
            for p in parts {
                match p {
                    Part::Lit(s) => quote(o, s),
                    Part::Expr(x) => expr(o, x),
                }
            }
        }
        ExprKind::None => open(o, "none"),
        ExprKind::RetVal => open(o, "retval"),
        ExprKind::Old(x) => {
            open(o, "old");
            expr(o, x);
        }
        ExprKind::Var(n) => {
            open(o, "var");
            quote(o, n);
        }
        ExprKind::Unary { op, e } => {
            open(o, "unary");
            atom(o, match op {
                UnOp::Neg => "neg",
                UnOp::BitNot => "bitnot",
                UnOp::Not => "not",
            });
            expr(o, e);
        }
        ExprKind::Binary { op, l, r } => {
            open(o, "binary");
            atom(o, op.text());
            expr(o, l);
            expr(o, r);
        }
        ExprKind::Call { module, name, name_span, targs, args, via } => {
            open(o, "call");
            opt_str(o, module);
            quote(o, name);
            span(o, *name_span);
            types(o, targs);
            exprs(o, args);
            opt_expr(o, via);
        }
        ExprKind::CallValue { f, args } => {
            open(o, "callvalue");
            expr(o, f);
            exprs(o, args);
        }
        ExprKind::StructLit { module, name, name_span, targs, args } => {
            open(o, "structlit");
            opt_str(o, module);
            quote(o, name);
            span(o, *name_span);
            types(o, targs);
            exprs(o, args);
        }
        ExprKind::AddrOf(x) => {
            open(o, "addrof");
            expr(o, x);
        }
        ExprKind::Deref(x) => {
            open(o, "deref");
            expr(o, x);
        }
        ExprKind::ArrayLit(items) => {
            open(o, "arraylit");
            exprs(o, items);
        }
        ExprKind::ArrayFill(x) => {
            open(o, "arrayfill");
            expr(o, x);
        }
        ExprKind::Field { e, name, name_span } => {
            open(o, "field");
            expr(o, e);
            quote(o, name);
            span(o, *name_span);
        }
        ExprKind::Index { e, i } => {
            open(o, "index");
            expr(o, e);
            expr(o, i);
        }
        ExprKind::Slice { e, lo, hi } => {
            open(o, "slice");
            expr(o, e);
            opt_expr(o, lo);
            opt_expr(o, hi);
        }
        ExprKind::Try(x) => {
            open(o, "try");
            expr(o, x);
        }
        ExprKind::Else { e, handler } => {
            open(o, "else");
            expr(o, e);
            match handler {
                ElseKind::Value(v) => {
                    open(o, "value");
                    expr(o, v);
                    close(o);
                }
                ElseKind::Block(b, body) => {
                    open(o, "handler");
                    opt_bind(o, b);
                    block(o, body);
                    close(o);
                }
            }
        }
    }
    span(o, e.span);
    close(o);
}

fn block(o: &mut String, b: &Block) {
    open(o, "block");
    for s in &b.stmts {
        stmt(o, s);
    }
    span(o, b.end);
    close(o);
}

fn opt_op(o: &mut String, op: &Option<BinOp>) {
    match op {
        Some(op) => atom(o, op.text()),
        None => atom(o, "-"),
    }
}

fn pat(o: &mut String, p: &Pat) {
    match p {
        Pat::Wild => atom(o, "_"),
        Pat::Variant { name, binds: b } => {
            open(o, "variant");
            quote(o, name);
            binds(o, "binds", b);
            close(o);
        }
        Pat::Int(v) => {
            open(o, "pint");
            number(o, *v);
            close(o);
        }
        Pat::Bool(v) => {
            open(o, "pbool");
            flag(o, *v);
            close(o);
        }
    }
}

fn stmt(o: &mut String, s: &Stmt) {
    o.push('\n');
    match &s.kind {
        StmtKind::Let { mutable, name, name_span, ty: t, init } => {
            open(o, "let");
            flag(o, *mutable);
            quote(o, name);
            span(o, *name_span);
            opt_ty(o, t);
            expr(o, init);
        }
        StmtKind::Assign { target, op, value } => {
            open(o, "assign");
            expr(o, target);
            opt_op(o, op);
            expr(o, value);
        }
        StmtKind::Discard(e) => {
            open(o, "discard");
            expr(o, e);
        }
        StmtKind::Expr(e) => {
            open(o, "expr");
            expr(o, e);
        }
        StmtKind::If { cond, then, els } => {
            open(o, "if");
            expr(o, cond);
            block(o, then);
            match els {
                Some(Else::Block(b)) => block(o, b),
                Some(Else::If(x)) => stmt(o, x),
                None => atom(o, "-"),
            }
        }
        StmtKind::ForRange { var, var_span, start, end, body } => {
            open(o, "forrange");
            quote(o, var);
            span(o, *var_span);
            expr(o, start);
            expr(o, end);
            block(o, body);
        }
        StmtKind::ForIn { elem, elem_span, index, subject, body } => {
            open(o, "forin");
            quote(o, elem);
            span(o, *elem_span);
            opt_bind(o, index);
            expr(o, subject);
            block(o, body);
        }
        StmtKind::ForCond { cond, body } => {
            open(o, "forcond");
            expr(o, cond);
            block(o, body);
        }
        StmtKind::ForEver { body } => {
            open(o, "forever");
            block(o, body);
        }
        StmtKind::Match { subject, arms } => {
            open(o, "match");
            expr(o, subject);
            for a in arms {
                open(o, "arm");
                pat(o, &a.pat);
                span(o, a.span);
                block(o, &a.body);
                close(o);
            }
        }
        StmtKind::Ret(e) => {
            open(o, "ret");
            match e {
                Some(e) => expr(o, e),
                None => atom(o, "-"),
            }
        }
        StmtKind::Fail(e) => {
            open(o, "fail");
            expr(o, e);
        }
        StmtKind::Defer(b) => {
            open(o, "defer");
            block(o, b);
        }
        StmtKind::Sure(e, text) => {
            open(o, "sure");
            expr(o, e);
            quote(o, text);
        }
        StmtKind::Break => open(o, "break"),
        StmtKind::Continue => open(o, "continue"),
    }
    span(o, s.span);
    close(o);
}

fn fields(o: &mut String, items: &[FieldDecl]) {
    open(o, "fields");
    for f in items {
        quote(o, &f.name);
        span(o, f.span);
        ty(o, &f.ty);
    }
    close(o);
}

fn params(o: &mut String, items: &[Param]) {
    open(o, "params");
    for p in items {
        flag(o, p.own);
        quote(o, &p.name);
        span(o, p.span);
        ty(o, &p.ty);
    }
    close(o);
}

fn func(o: &mut String, f: &Func) {
    open(o, "fn");
    quote(o, &f.name);
    span(o, f.name_span);
    flag(o, f.public);
    flag(o, f.claw);
    binds(o, "tparams", &f.tparams);
    params(o, &f.params);
    opt_ty(o, &f.ret);
    open(o, "contracts");
    for c in &f.contracts {
        flag(o, c.sure);
        expr(o, &c.e);
        quote(o, &c.text);
    }
    close(o);
    strs(o, "effects", &f.effects);
    span(o, f.sig_end);
    opt_str(o, &f.test);
    block(o, &f.body);
    close(o);
    o.push('\n');
}

fn program(o: &mut String, p: &Program) {
    for u in &p.uses {
        open(o, "use");
        strs(o, "path", &u.path);
        span(o, u.span);
        close(o);
        o.push('\n');
    }
    for e in &p.externs {
        open(o, "extern");
        opt_str(o, &e.symbol);
        quote(o, &e.name);
        span(o, e.span);
        flag(o, e.public);
        params(o, &e.params);
        flag(o, e.variadic);
        opt_ty(o, &e.ret);
        close(o);
        o.push('\n');
    }
    for t in &p.types {
        open(o, "type");
        quote(o, &t.name);
        span(o, t.span);
        flag(o, t.public);
        binds(o, "tparams", &t.tparams);
        match &t.body {
            TypeBody::Struct(f) => fields(o, f),
            TypeBody::Variants(vs) => {
                open(o, "variants");
                for v in vs {
                    open(o, "variant");
                    quote(o, &v.name);
                    span(o, v.span);
                    fields(o, &v.fields);
                    close(o);
                }
                close(o);
            }
        }
        close(o);
        o.push('\n');
    }
    for c in &p.consts {
        open(o, "const");
        quote(o, &c.name);
        span(o, c.span);
        flag(o, c.public);
        opt_ty(o, &c.ty);
        expr(o, &c.value);
        flag(o, c.computed);
        quote(o, &c.text);
        close(o);
        o.push('\n');
    }
    for f in &p.funcs {
        func(o, f);
    }
}

fn diagnostics(o: &mut String, diags: &[Diag]) {
    for d in diags {
        let _ = writeln!(o, "{} {}:{} {}", d.code, d.span.line, d.span.col, d.msg);
    }
}

fn one(path: &str) {
    let Ok(src) = std::fs::read_to_string(path) else {
        println!("unreadable");
        return;
    };
    let mut diags = Vec::new();
    let toks = lexer::lex(&src, 0, &mut diags);
    let p = parser::Parser::new(toks, &mut diags).with_source(&src).parse_program();
    let mut o = String::new();
    program(&mut o, &p);
    diagnostics(&mut o, &diags);
    println!("{o}");
}

fn all(path: &str) {
    let mut o = String::new();
    match modules::load(path, true) {
        Ok(l) => {
            for (i, u) in l.units.iter().enumerate() {
                let _ = writeln!(o, "unit {i} {:?} {}", u.prefix, u.is_std);
                let mut names: Vec<&String> = u.imports.keys().collect();
                names.sort();
                for n in names {
                    let _ = write!(o, "import {n} {}", u.imports[n]);
                    span(&mut o, u.import_spans[n]);
                    o.push('\n');
                }
                program(&mut o, &u.program);
            }
            let _ = writeln!(o, "core {}", l.core);
            for (f, b) in l.files.iter().zip(&l.base_names) {
                let _ = writeln!(o, "file {f} {b}");
            }
        }
        Err((diags, files)) => {
            diagnostics(&mut o, &diags);
            for f in &files {
                let _ = writeln!(o, "file {f}");
            }
        }
    }
    println!("{o}");
}

pub fn run(args: &[String]) {
    if args.first().is_some_and(|a| a == "--load") {
        all(&args[1]);
    } else {
        one(&args[0]);
    }
}

fn ty_text(types: &Types, t: Ty) -> String {
    format!("{} {}", t.0, types.name(t))
}

fn num_text(v: i128) -> String {
    if v.unsigned_abs() > u64::MAX as u128 {
        if v < 0 { "-big".to_string() } else { "big".to_string() }
    } else {
        v.to_string()
    }
}

fn kind_text(k: Kind) -> String {
    match k {
        Kind::Int { bits, signed } => format!("int {bits} {signed}"),
        Kind::Float { bits } => format!("float {bits}"),
        Kind::Bool => "bool".to_string(),
        Kind::Void => "void".to_string(),
        Kind::StrLit => "strlit".to_string(),
        Kind::Err => "err".to_string(),
        Kind::Struct(id) => format!("struct {id}"),
        Kind::Enum(id) => format!("enum {id}"),
        Kind::Array(e, n) => format!("array {} {n}", e.0),
        Kind::Pointer(p) => format!("ptr {}", p.0),
        Kind::Slice(e) => format!("slice {}", e.0),
        Kind::Str => "str".to_string(),
        Kind::Ref(p) => format!("ref {}", p.0),
        Kind::Option(p) => format!("opt {}", p.0),
        Kind::Result(a, b) => format!("result {} {}", a.0, b.0),
        Kind::Fn(id) => format!("fn {id}"),
        Kind::Vec(v) => format!("vec {}", v.name()),
    }
}

fn expr_tag(k: &ExprKind) -> &'static str {
    match k {
        ExprKind::Int(_) => "int",
        ExprKind::Float(_) => "float",
        ExprKind::Char(_) => "char",
        ExprKind::Bool(_) => "bool",
        ExprKind::Str(_) => "str",
        ExprKind::Interp(_) => "interp",
        ExprKind::None => "none",
        ExprKind::RetVal => "retval",
        ExprKind::Old(_) => "old",
        ExprKind::Var(_) => "var",
        ExprKind::Unary { .. } => "unary",
        ExprKind::Binary { .. } => "binary",
        ExprKind::Call { .. } => "call",
        ExprKind::CallValue { .. } => "callvalue",
        ExprKind::StructLit { .. } => "structlit",
        ExprKind::AddrOf(_) => "addrof",
        ExprKind::Deref(_) => "deref",
        ExprKind::ArrayLit(_) => "arraylit",
        ExprKind::ArrayFill(_) => "arrayfill",
        ExprKind::Field { .. } => "field",
        ExprKind::Index { .. } => "index",
        ExprKind::Slice { .. } => "slice",
        ExprKind::Try(_) => "try",
        ExprKind::Else { .. } => "else",
    }
}

fn sema_expr(o: &mut String, c: &Checked, e: &Expr) {
    let coerce = match e.coerce.get() {
        Coerce::No => "no".to_string(),
        Coerce::Some(t) => format!("some {}", ty_text(&c.types, t)),
        Coerce::Ok(t) => format!("ok {}", ty_text(&c.types, t)),
        Coerce::View => "view".to_string(),
    };
    let res = match &*e.res.borrow() {
        Res::None => "-".to_string(),
        Res::Func(f) => format!("func {f}"),
        Res::Value(t) => format!("value {}", ty_text(&c.types, *t)),
        Res::Const(v, t) => {
            let val = match v {
                ConstVal::Int(n) => format!("int {}", num_text(*n)),
                ConstVal::Float(x) => format!("float {}", x.to_bits()),
                ConstVal::Bool(b) => format!("bool {b}"),
                ConstVal::Computed(i) => format!("computed {i}"),
            };
            format!("const {val} {}", ty_text(&c.types, *t))
        }
        Res::Variant(a, b) => format!("variant {a} {b}"),
    };
    let _ = writeln!(o, "e {}:{} {} | {} | {coerce} | {res}", e.span.line, e.span.col, expr_tag(&e.kind), ty_text(&c.types, e.ty.get()));
    match &e.kind {
        ExprKind::Interp(parts) => {
            for p in parts {
                if let Part::Expr(x) = p {
                    sema_expr(o, c, x);
                }
            }
        }
        ExprKind::Old(x) | ExprKind::AddrOf(x) | ExprKind::Deref(x) | ExprKind::ArrayFill(x) | ExprKind::Try(x) | ExprKind::Field { e: x, .. } | ExprKind::Unary { e: x, .. } => sema_expr(o, c, x),
        ExprKind::Binary { l, r, .. } => {
            sema_expr(o, c, l);
            sema_expr(o, c, r);
        }
        ExprKind::Call { args, via, .. } => {
            for a in args {
                sema_expr(o, c, a);
            }
            if let Some(v) = via {
                sema_expr(o, c, v);
            }
        }
        ExprKind::CallValue { f, args } => {
            sema_expr(o, c, f);
            for a in args {
                sema_expr(o, c, a);
            }
        }
        ExprKind::StructLit { args, .. } | ExprKind::ArrayLit(args) => {
            for a in args {
                sema_expr(o, c, a);
            }
        }
        ExprKind::Index { e: x, i } => {
            sema_expr(o, c, x);
            sema_expr(o, c, i);
        }
        ExprKind::Slice { e: x, lo, hi } => {
            sema_expr(o, c, x);
            if let Some(v) = lo {
                sema_expr(o, c, v);
            }
            if let Some(v) = hi {
                sema_expr(o, c, v);
            }
        }
        ExprKind::Else { e: x, handler } => {
            sema_expr(o, c, x);
            match handler {
                ElseKind::Value(v) => sema_expr(o, c, v),
                ElseKind::Block(_, b) => sema_block(o, c, b),
            }
        }
        _ => {}
    }
}

fn sema_block(o: &mut String, c: &Checked, b: &Block) {
    for s in &b.stmts {
        sema_stmt(o, c, s);
    }
}

fn sema_stmt(o: &mut String, c: &Checked, s: &Stmt) {
    match &s.kind {
        StmtKind::Let { init, .. } => sema_expr(o, c, init),
        StmtKind::Assign { target, value, .. } => {
            sema_expr(o, c, target);
            sema_expr(o, c, value);
        }
        StmtKind::Discard(e) | StmtKind::Expr(e) | StmtKind::Fail(e) | StmtKind::Sure(e, _) => sema_expr(o, c, e),
        StmtKind::If { cond, then, els } => {
            sema_expr(o, c, cond);
            sema_block(o, c, then);
            match els {
                Some(Else::Block(b)) => sema_block(o, c, b),
                Some(Else::If(i)) => sema_stmt(o, c, i),
                None => {}
            }
        }
        StmtKind::ForRange { start, end, body, .. } => {
            sema_expr(o, c, start);
            sema_expr(o, c, end);
            sema_block(o, c, body);
        }
        StmtKind::ForIn { subject, body, .. } => {
            sema_expr(o, c, subject);
            sema_block(o, c, body);
        }
        StmtKind::ForCond { cond, body } => {
            sema_expr(o, c, cond);
            sema_block(o, c, body);
        }
        StmtKind::ForEver { body } | StmtKind::Defer(body) => sema_block(o, c, body),
        StmtKind::Match { subject, arms } => {
            sema_expr(o, c, subject);
            for a in arms {
                sema_block(o, c, &a.body);
            }
        }
        StmtKind::Ret(e) => {
            if let Some(e) = e {
                sema_expr(o, c, e);
            }
        }
        StmtKind::Break | StmtKind::Continue => {}
    }
}

fn sema_func(o: &mut String, c: &Checked, full: &str, f: &Func) {
    let _ = writeln!(o, "func {full}");
    let mut addressed: Vec<String> = f.addressed.borrow().iter().cloned().collect();
    addressed.sort();
    for a in &addressed {
        let _ = writeln!(o, "addressed {a}");
    }
    for x in &f.contracts {
        let _ = writeln!(o, "contract {}", x.sure);
        sema_expr(o, c, &x.e);
    }
    sema_block(o, c, &f.body);
}

fn fields_text(o: &mut String, fields: &[crate::types::Field]) {
    for f in fields {
        let _ = writeln!(o, "  field {} {} {}", f.name, f.ty.0, f.offset);
    }
}

fn sig_text(o: &mut String, name: &str, s: &FnSig) {
    let _ = write!(o, "sig {name} params");
    for p in &s.params {
        let _ = write!(o, " {}", p.0);
    }
    let _ = write!(o, " own");
    for x in &s.own {
        let _ = write!(o, " {x}");
    }
    let _ = writeln!(o, " ret {} ext {} variadic {} need {}", s.ret.0, s.ext.as_deref().unwrap_or("-"), s.variadic, s.has_need);
}

fn checked(o: &mut String, units: &[modules::Unit], c: &Checked, report: &effects::Report) {
    let t = &c.types;
    let count = t.count();
    let _ = writeln!(o, "types {count}");
    for i in 0..count {
        let ty = Ty(i as u32);
        let _ = writeln!(o, "t {} | {} | {}", i, t.name(ty), kind_text(t.kind(ty)));
    }
    for (i, s) in t.structs.iter().enumerate() {
        let _ = writeln!(o, "struct {i} {} size {} align {} module {} state {}", s.name, s.size, s.align, s.module, s.state);
        fields_text(o, &s.fields);
    }
    for (i, e) in t.enums.iter().enumerate() {
        let _ = writeln!(o, "enum {i} {} size {} module {} state {}", e.name, e.size, e.module, e.state);
        for v in &e.variants {
            let _ = writeln!(o, " variant {}", v.name);
            fields_text(o, &v.fields);
        }
    }
    for (i, f) in t.fns.iter().enumerate() {
        let _ = write!(o, "fntype {i} params");
        for p in &f.params {
            let _ = write!(o, " {}", p.0);
        }
        let _ = write!(o, " own");
        for x in &f.own {
            let _ = write!(o, " {x}");
        }
        let _ = writeln!(o, " ret {} effects {}", f.ret.0, f.effects);
    }
    let mut drops: Vec<(&Ty, &String)> = t.user_drop.iter().collect();
    drops.sort_by_key(|x| x.0 .0);
    for (ty, name) in drops {
        let _ = writeln!(o, "drop {} {name}", ty.0);
    }
    let mut names: Vec<&String> = c.sigs.keys().collect();
    names.sort();
    for n in names {
        sig_text(o, n, &c.sigs[n]);
    }
    for x in &c.computed {
        let _ = writeln!(o, "computed {} {} {}", x.name, x.full, x.ty.0);
    }
    for u in units {
        for f in u.program.funcs.iter().filter(|f| f.tparams.is_empty()) {
            sema_func(o, c, &format!("{}{}", u.prefix, f.name), f);
        }
    }
    for f in &c.instances {
        sema_func(o, c, &f.name, f);
    }
    let _ = writeln!(o, "slow {}", report.slow);
    let mut known: Vec<(&String, &u8)> = report.known.iter().collect();
    known.sort();
    for (n, e) in known {
        let _ = writeln!(o, "known {n} {e}");
    }
}

fn all_diagnostics(o: &mut String, mut diags: Vec<Diag>, files: &[String]) {
    diag::sort(&mut diags);
    for d in &diags {
        let _ = write!(o, "{}:{}:{} {} {}", files[d.span.file as usize], d.span.line, d.span.col, d.code, d.msg);
        if let Some(f) = &d.fix {
            let _ = write!(o, "; fix: {}", f.label);
        }
        o.push('\n');
    }
}

pub fn sema_dump(args: &[String]) {
    let memory_checker = args.iter().any(|a| a == "--check");
    let tests_only = args.iter().any(|a| a == "--tests");
    let path = args.iter().find(|a| !a.starts_with("--")).expect("__sema needs a file");
    let mut o = String::new();
    match modules::load(path, memory_checker) {
        Err((diags, files)) => all_diagnostics(&mut o, diags, &files),
        Ok(l) => {
            let mut diags = Vec::new();
            let c = sema::check(&l.units, l.core, tests_only, &mut diags);
            let report = effects::check(&l.units, &c, &mut diags);
            if diags.is_empty() {
                checked(&mut o, &l.units, &c, &report);
            } else {
                all_diagnostics(&mut o, diags, &l.files);
            }
        }
    }
    println!("{o}");
}

/// `claw __cir [--check] [--tests] [--fast] [--regs] file`: the program as lowered, before any
/// optimization, in the same words as `selfhost/dump_cir`. The flags pick the build:
/// none is `claw build --no-opt`, `--check` is `claw run`, `--check --tests` is `claw test --blocks`.
pub fn cir_dump(args: &[String]) {
    let check = args.iter().any(|a| a == "--check");
    let test = args.iter().any(|a| a == "--tests");
    let fast = args.iter().any(|a| a == "--fast");
    let regs = args.iter().any(|a| a == "--regs");
    let path = args.iter().find(|a| !a.starts_with("--")).expect("__cir needs a file");
    let mut o = String::new();
    match modules::load(path, check) {
        Err((diags, files)) => all_diagnostics(&mut o, diags, &files),
        Ok(l) => {
            let mut diags = Vec::new();
            let c = sema::check(&l.units, l.core, test, &mut diags);
            effects::check(&l.units, &c, &mut diags);
            let consts = if diags.is_empty() { ctfe::evaluate(&l.units, &c, &l.base_names, &mut diags) } else { Vec::new() };
            if diags.is_empty() {
                let opts = lower::Opts { check, test, fast, interp: false, debug: false, consts: &consts };
                let m = lower::lower(&l.units, &c, &l.base_names, opts);
                o = if regs { reg_dump(&m) } else { cir::dump(&m) };
            } else {
                all_diagnostics(&mut o, diags, &l.files);
            }
        }
    }
    print!("{o}");
}

/// What `Inst::each`, `def` and `is_call` say about every instruction.
fn reg_dump(m: &cir::Module) -> String {
    let mut o = String::new();
    for f in &m.funcs {
        let _ = writeln!(o, "fn {}", f.name);
        for b in &f.blocks {
            for i in &b.insts {
                let mut regs = Vec::new();
                i.each(&mut |v, def| regs.push((v, def)));
                let _ = writeln!(o, "{regs:?} {:?} {}", i.def(), i.is_call());
            }
        }
    }
    o
}
