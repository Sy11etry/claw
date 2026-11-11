//! What an AI or a person reads instead of the source: `claw card`, `sig`, `ctx`, `audit`.

use crate::ast::*;
use crate::effects::{self, Report};
use crate::modules::Unit;
use crate::sema::{walk, Checked, FnSig};
use crate::types::*;
use std::collections::BTreeSet;

pub const CARD: &str = include_str!("../../docs/CARD.md");

pub fn type_text(t: &TypeExpr) -> String {
    match &t.kind {
        TypeKind::Name { module, name, targs } => {
            let prefix = module.as_ref().map(|m| format!("{m}.")).unwrap_or_default();
            if targs.is_empty() {
                format!("{prefix}{name}")
            } else {
                format!("{prefix}{name}[{}]", targs.iter().map(type_text).collect::<Vec<_>>().join(", "))
            }
        }
        TypeKind::Pointer(x) => format!("*{}", type_text(x)),
        TypeKind::Ref(x) => format!("&{}", type_text(x)),
        TypeKind::Slice(x) => format!("[]{}", type_text(x)),
        TypeKind::Array(Size::Lit(n), x) => format!("[{n}]{}", type_text(x)),
        TypeKind::Array(Size::Name(n), x) => format!("[{n}]{}", type_text(x)),
        TypeKind::Option(x) => format!("?{}", type_text(x)),
        TypeKind::Result(Some(ok), err) => format!("{} ! {}", type_text(ok), type_text(err)),
        TypeKind::Result(None, err) => format!("! {}", type_text(err)),
        TypeKind::Fn { params, ret, effects } => {
            let list: Vec<String> = params.iter().map(|(own, p)| format!("{}{}", if *own { "own " } else { "" }, type_text(p))).collect();
            let mut out = format!("fn({})", list.join(", "));
            if let Some(r) = ret {
                out += &format!(" {}", type_text(r));
            }
            for e in effects {
                out += &format!(" +{e}");
            }
            out
        }
    }
}

fn tparams(list: &[(String, crate::diag::Span)]) -> String {
    if list.is_empty() {
        String::new()
    } else {
        format!("[{}]", list.iter().map(|p| p.0.clone()).collect::<Vec<_>>().join(", "))
    }
}

/// The header of `f` as it could be written, with `effects` in place of whatever was declared.
fn header(f: &Func, effects: &str) -> String {
    let params: Vec<String> = f.params.iter().map(|p| format!("{}{} {}", if p.own { "own " } else { "" }, p.name, type_text(&p.ty))).collect();
    let mut out = format!("{}{}fn {}{}({})", if f.public { "pub " } else { "" }, if f.claw { "claw " } else { "" }, f.name, tparams(&f.tparams), params.join(", "));
    if let Some(r) = &f.ret {
        out += &format!(" {}", type_text(r));
    }
    if !effects.is_empty() {
        out += &format!(" {effects}");
    }
    out.push('\n');
    for c in &f.contracts {
        out += &format!("    {} {}\n", if c.sure { "sure" } else { "need" }, c.text);
    }
    out
}

fn fields_text(fields: &[FieldDecl]) -> String {
    fields.iter().map(|f| format!("{} {}", f.name, type_text(&f.ty))).collect::<Vec<_>>().join(", ")
}

fn type_decl(d: &TypeDecl) -> String {
    let head = format!("{}type {}{}", if d.public { "pub " } else { "" }, d.name, tparams(&d.tparams));
    match &d.body {
        TypeBody::Struct(fields) => format!("{head} {{ {} }}\n", fields_text(fields)),
        TypeBody::Variants(vars) => {
            let list: Vec<String> = vars.iter().map(|v| if v.fields.is_empty() { v.name.clone() } else { format!("{}{{{}}}", v.name, fields_text(&v.fields)) }).collect();
            format!("{head} = {}\n", list.join(" | "))
        }
    }
}

fn const_decl(c: &ConstDecl) -> String {
    let ty = c.ty.as_ref().map(|t| format!(" {}", type_text(t))).unwrap_or_default();
    format!("{}const {}{ty} = {}\n", if c.public { "pub " } else { "" }, c.name, c.text)
}

fn extern_decl(x: &ExternDecl) -> String {
    let mut params: Vec<String> = x.params.iter().map(|p| format!("{} {}", p.name, type_text(&p.ty))).collect();
    if x.variadic {
        params.push("...".to_string());
    }
    let symbol = x.symbol.as_ref().map(|s| format!("\"{s}\" ")).unwrap_or_default();
    let ret = x.ret.as_ref().map(|t| format!(" {}", type_text(t))).unwrap_or_default();
    format!("{}extern {symbol}fn {}({}){ret}\n", if x.public { "pub " } else { "" }, x.name, params.join(", "))
}

/// Functions a reader can see: not the hidden ones behind `const`, not `test` blocks.
fn visible(f: &Func) -> bool {
    f.test.is_none() && !f.name.starts_with("const ")
}

fn sig_of_program(p: &Program, public_only: bool, effects: &dyn Fn(&Func) -> String) -> String {
    let mut out = String::new();
    for d in p.types.iter().filter(|d| d.public || !public_only) {
        out += &type_decl(d);
    }
    for c in p.consts.iter().filter(|c| c.public || !public_only) {
        out += &const_decl(c);
    }
    for x in p.externs.iter().filter(|x| x.public || !public_only) {
        out += &extern_decl(x);
    }
    for f in p.funcs.iter().filter(|f| visible(f) && (f.public || !public_only)) {
        out += &header(f, &effects(f));
    }
    out
}

fn declared(f: &Func) -> String {
    f.effects.iter().map(|e| format!("+{e}")).collect::<Vec<_>>().join(" ")
}

/// What `f` really does: worked out if the signature does not say.
fn effects_of(f: &Func, prefix: &str, report: &Report) -> String {
    if !f.tparams.is_empty() {
        return declared(f);
    }
    spell_effects(report.effects(&format!("{prefix}{}", f.name)))
}

pub fn sig(units: &[Unit], report: &Report, public_only: bool) -> String {
    let root = units.last().expect("the root file is always loaded");
    sig_of_program(&root.program, public_only, &|f| effects_of(f, &root.prefix, report))
}

/// What a name stands for, as one declaration, and where it is declared: for hover and go-to-definition.
/// The root file is searched first, then the modules it uses.
pub fn describe(units: &[Unit], report: &Report, name: &str) -> Option<(String, crate::diag::Span)> {
    for u in units.iter().rev() {
        if let Some(f) = u.program.funcs.iter().find(|f| visible(f) && f.name == name) {
            return Some((header(f, &effects_of(f, &u.prefix, report)).trim_end().to_string(), f.name_span));
        }
        if let Some(d) = u.program.types.iter().find(|d| d.name == name) {
            return Some((type_decl(d).trim_end().to_string(), d.span));
        }
        if let Some(c) = u.program.consts.iter().find(|c| c.name == name) {
            return Some((const_decl(c).trim_end().to_string(), c.span));
        }
        if let Some(x) = u.program.externs.iter().find(|x| x.name == name) {
            return Some((extern_decl(x).trim_end().to_string(), x.span));
        }
    }
    None
}

pub fn sig_std() -> String {
    let mut out = String::new();
    for (name, p) in crate::modules::std_programs() {
        let text = sig_of_program(&p, true, &declared);
        if !text.is_empty() {
            out += &format!("// {name}\n{text}");
        }
    }
    out
}

/// Every expression of a block, however deeply nested.
fn walk_block<'e>(b: &'e Block, f: &mut dyn FnMut(&'e Expr)) {
    for s in &b.stmts {
        walk_stmt(s, f);
    }
}

fn walk_expr<'e>(e: &'e Expr, f: &mut dyn FnMut(&'e Expr)) {
    let mut blocks: Vec<&'e Block> = Vec::new();
    walk(e, &mut |x| {
        f(x);
        if let ExprKind::Else { handler: ElseKind::Block(_, b), .. } = &x.kind {
            blocks.push(b);
        }
    });
    for b in blocks {
        walk_block(b, f);
    }
}

fn walk_stmt<'e>(s: &'e Stmt, f: &mut dyn FnMut(&'e Expr)) {
    match &s.kind {
        StmtKind::Let { init: e, .. } | StmtKind::Discard(e) | StmtKind::Expr(e) | StmtKind::Fail(e) | StmtKind::Sure(e, _) => walk_expr(e, f),
        StmtKind::Assign { target, value, .. } => {
            walk_expr(target, f);
            walk_expr(value, f);
        }
        StmtKind::If { cond, then, els } => {
            walk_expr(cond, f);
            walk_block(then, f);
            match els {
                Some(Else::Block(b)) => walk_block(b, f),
                Some(Else::If(i)) => walk_stmt(i, f),
                None => {}
            }
        }
        StmtKind::ForRange { start, end, body, .. } => {
            walk_expr(start, f);
            walk_expr(end, f);
            walk_block(body, f);
        }
        StmtKind::ForIn { subject, body, .. } => {
            walk_expr(subject, f);
            walk_block(body, f);
        }
        StmtKind::ForCond { cond, body } => {
            walk_expr(cond, f);
            walk_block(body, f);
        }
        StmtKind::ForEver { body } | StmtKind::Defer(body) => walk_block(body, f),
        StmtKind::Match { subject, arms } => {
            walk_expr(subject, f);
            for a in arms {
                walk_block(&a.body, f);
            }
        }
        StmtKind::Ret(Some(e)) => walk_expr(e, f),
        StmtKind::Ret(None) | StmtKind::Break | StmtKind::Continue => {}
    }
}

fn base_name(s: &str) -> &str {
    s.split('[').next().unwrap_or(s)
}

/// The types and annotations a function spells out in its body.
fn annotated<'e>(b: &'e Block, out: &mut Vec<&'e TypeExpr>) {
    for s in &b.stmts {
        match &s.kind {
            StmtKind::Let { ty: Some(t), .. } => out.push(t),
            StmtKind::If { then, els, .. } => {
                annotated(then, out);
                match els {
                    Some(Else::Block(e)) => annotated(e, out),
                    Some(Else::If(i)) => {
                        if let StmtKind::If { then, .. } = &i.kind {
                            annotated(then, out);
                        }
                    }
                    None => {}
                }
            }
            StmtKind::ForRange { body, .. } | StmtKind::ForIn { body, .. } | StmtKind::ForCond { body, .. } | StmtKind::ForEver { body } | StmtKind::Defer(body) => annotated(body, out),
            StmtKind::Match { arms, .. } => {
                for a in arms {
                    annotated(&a.body, out);
                }
            }
            _ => {}
        }
    }
}

struct Finder<'a> {
    units: &'a [Unit],
}

impl<'a> Finder<'a> {
    /// Who `x` calls, as a unit and a name, when `x` is a call of a plain function.
    fn callee(&self, unit: usize, x: &Expr) -> Option<(usize, String)> {
        let ExprKind::Call { module, name, .. } = &x.kind else { return None };
        if let Res::Func(full) = &*x.res.borrow() {
            let base = base_name(full);
            for (i, u) in self.units.iter().enumerate() {
                if let Some(rest) = base.strip_prefix(u.prefix.as_str()) {
                    if u.program.funcs.iter().any(|f| f.name == rest) || u.program.externs.iter().any(|f| f.name == rest) {
                        return Some((i, rest.to_string()));
                    }
                }
            }
            return None;
        }
        // Inside a template nothing has been resolved: go by the name as written.
        let u = &self.units[unit];
        let target = match module {
            Some(alias) => *u.imports.get(alias)?,
            None => unit,
        };
        let known = |p: &Program| p.funcs.iter().any(|f| f.name == *name) || p.externs.iter().any(|f| f.name == *name);
        known(&self.units[target].program).then(|| (target, name.clone()))
    }
}

fn note_ty(t: Ty, types: &Types, out: &mut BTreeSet<(usize, String)>) {
    match types.kind(t) {
        Kind::Struct(id) => {
            out.insert((types.structs[id].module, base_name(&types.structs[id].name).to_string()));
        }
        Kind::Enum(id) => {
            out.insert((types.enums[id].module, base_name(&types.enums[id].name).to_string()));
        }
        Kind::Array(e, _) | Kind::Pointer(e) | Kind::Slice(e) | Kind::Ref(e) | Kind::Option(e) => note_ty(e, types, out),
        Kind::Result(a, b) => {
            note_ty(a, types, out);
            note_ty(b, types, out);
        }
        Kind::Fn(id) => {
            for p in types.fns[id].params.clone() {
                note_ty(p, types, out);
            }
            note_ty(types.fns[id].ret, types, out);
        }
        _ => {}
    }
}

fn note_expr(t: &TypeExpr, unit: usize, units: &[Unit], out: &mut BTreeSet<(usize, String)>) {
    match &t.kind {
        TypeKind::Name { module, name, targs } => {
            let target = match module {
                Some(m) => units[unit].imports.get(m).copied(),
                None => Some(unit),
            };
            if let Some(i) = target {
                out.insert((i, name.clone()));
            }
            for a in targs {
                note_expr(a, unit, units, out);
            }
        }
        TypeKind::Pointer(x) | TypeKind::Ref(x) | TypeKind::Slice(x) | TypeKind::Array(_, x) | TypeKind::Option(x) => note_expr(x, unit, units, out),
        TypeKind::Result(ok, err) => {
            if let Some(k) = ok {
                note_expr(k, unit, units, out);
            }
            note_expr(err, unit, units, out);
        }
        TypeKind::Fn { params, ret, .. } => {
            for p in params {
                note_expr(&p.1, unit, units, out);
            }
            if let Some(r) = ret {
                note_expr(r, unit, units, out);
            }
        }
    }
}

/// The function asked about, and what one needs to see to change it safely.
pub fn ctx(units: &[Unit], c: &Checked, report: &Report, files: &[String], base: &[String], name: &str) -> Result<String, String> {
    let finder = Finder { units };
    let wanted = units.iter().enumerate().filter(|(_, u)| !u.is_std).find_map(|(i, u)| {
        let rest = name.strip_prefix(u.prefix.as_str())?;
        u.program.funcs.iter().find(|f| f.name == rest && visible(f)).map(|f| (i, f))
    });
    let Some((unit, f)) = wanted else { return Err(format!("no function `{name}` in this program")) };

    let mut callees: BTreeSet<(usize, String)> = BTreeSet::new();
    let mut types: BTreeSet<(usize, String)> = BTreeSet::new();
    let mut consts: BTreeSet<(usize, String)> = BTreeSet::new();
    let mut note = |x: &Expr| {
        if let Some(found) = finder.callee(unit, x) {
            callees.insert(found);
        }
        note_ty(x.ty.get(), &c.types, &mut types);
        match &x.kind {
            ExprKind::Var(n) if units[unit].program.consts.iter().any(|k| k.name == *n) => {
                consts.insert((unit, n.clone()));
            }
            ExprKind::StructLit { module: None, name, .. } => {
                types.insert((unit, name.clone()));
            }
            _ => {}
        }
    };
    walk_block(&f.body, &mut note);
    for k in &f.contracts {
        walk_expr(&k.e, &mut note);
    }
    let mut spelled: Vec<&TypeExpr> = f.params.iter().map(|p| &p.ty).chain(f.ret.iter()).collect();
    annotated(&f.body, &mut spelled);
    for t in spelled {
        note_expr(t, unit, units, &mut types);
    }

    // A type needs the types inside it; a variant's name stands for its type.
    let find_type = |u: usize, n: &str| units[u].program.types.iter().find(|d| d.name == n || matches!(&d.body, TypeBody::Variants(v) if v.iter().any(|x| x.name == n)));
    let mut work: Vec<(usize, String)> = types.iter().cloned().collect();
    let mut decls: Vec<(usize, &TypeDecl)> = Vec::new();
    while let Some((u, n)) = work.pop() {
        let Some(d) = find_type(u, &n) else { continue };
        if units[u].is_std || decls.iter().any(|(i, x)| *i == u && x.name == d.name) {
            continue;
        }
        decls.push((u, d));
        let mut inner = BTreeSet::new();
        let fields: Vec<&FieldDecl> = match &d.body {
            TypeBody::Struct(fs) => fs.iter().collect(),
            TypeBody::Variants(vs) => vs.iter().flat_map(|v| v.fields.iter()).collect(),
        };
        for fd in fields {
            note_expr(&fd.ty, u, units, &mut inner);
        }
        work.extend(inner);
    }
    decls.sort_by_key(|(u, d)| (*u, d.span.start));

    let mut callers: Vec<String> = Vec::new();
    for (i, u) in units.iter().enumerate().filter(|(_, u)| !u.is_std) {
        for g in u.program.funcs.iter().filter(|g| visible(g)) {
            let mut calls = false;
            walk_block(&g.body, &mut |x| calls |= finder.callee(i, x).is_some_and(|t| t.0 == unit && t.1 == f.name));
            if calls {
                callers.push(g.name.clone());
            }
        }
    }

    let mut out = format!("// {} {}:{}", f.name, base[f.name_span.file as usize], f.name_span.line);
    if !callers.is_empty() {
        out += &format!("; called by {}", callers.join(", "));
    }
    out.push('\n');
    for (_, d) in decls {
        out += &type_decl(d);
    }
    for (u, n) in &consts {
        if let Some(k) = units[*u].program.consts.iter().find(|k| k.name == *n) {
            out += &const_decl(k);
        }
    }
    for (u, n) in callees.iter().filter(|(u, n)| !units[*u].is_std && !(*u == unit && *n == f.name)) {
        let p = &units[*u].program;
        if let Some(g) = p.funcs.iter().find(|g| g.name == *n) {
            out += &header(g, &effects_of(g, &units[*u].prefix, report));
        } else if let Some(x) = p.externs.iter().find(|x| x.name == *n) {
            out += &extern_decl(x);
        }
    }
    let text = std::fs::read_to_string(&files[f.name_span.file as usize]).map_err(|e| e.to_string())?;
    let start = text[..f.name_span.start as usize].rfind('\n').map_or(0, |i| i + 1);
    out += &text[start..f.body.end.end as usize];
    out.push('\n');
    Ok(out)
}

/// What the unsafe parts of a `claw fn` actually do.
fn raw_ops(f: &Func, c: &Checked) -> String {
    let mut seen: Vec<&str> = Vec::new();
    let mut add = |word: &'static str| {
        if !seen.contains(&word) {
            seen.push(word);
        }
    };
    walk_block(&f.body, &mut |x| match &x.kind {
        ExprKind::Call { name, .. } if matches!(name.as_str(), "alloc" | "free" | "cast") && matches!(*x.res.borrow(), Res::None) => {
            add(match name.as_str() {
                "alloc" => "alloc",
                "free" => "free",
                _ => "cast",
            })
        }
        ExprKind::Deref(_) => add("deref"),
        ExprKind::Index { e, .. } if c.types.is_ptr(e.ty.get()) => add("index"),
        ExprKind::Binary { l, .. } if c.types.is_ptr(l.ty.get()) && c.types.is_ptr(x.ty.get()) => add("arith"),
        _ => {}
    });
    seen.join(" ")
}

fn is_extern_call(x: &Expr, sigs: &std::collections::HashMap<String, FnSig>) -> bool {
    matches!(&*x.res.borrow(), Res::Func(full) if sigs.get(full).is_some_and(|s| s.ext.is_some()))
}

/// Every place where the compiler takes the programmer's word for it.
pub fn audit(units: &[Unit], c: &Checked, base: &[String]) -> String {
    let at = |s: crate::diag::Span| format!("{}:{}", base[s.file as usize], s.line);
    let (mut out, mut raw, mut outside, mut unbuilt, mut trusted) = (String::new(), 0, 0, 0, 0);
    for u in units {
        if u.is_std {
            trusted += u.program.funcs.iter().filter(|f| f.claw).count();
            continue;
        }
        for x in &u.program.externs {
            outside += 1;
            out += &format!("extern   {}  {}  effects taken on trust\n", at(x.span), x.name);
        }
        for f in u.program.funcs.iter().filter(|f| f.claw) {
            raw += 1;
            let mut calls = false;
            walk_block(&f.body, &mut |x| calls |= is_extern_call(x, &c.sigs));
            let mut ops = raw_ops(f, c);
            if calls {
                ops = format!("{ops} extern").trim().to_string();
            }
            out += &format!("claw fn  {}  {}  {}\n", at(f.name_span), f.name, if ops.is_empty() { "no raw operations" } else { &ops });
        }
        for f in u.program.funcs.iter().filter(|f| visible(f)) {
            for k in f.contracts.iter().filter(|k| effects::has_call(&k.e)) {
                unbuilt += 1;
                out += &format!("no build {}  {} {}  only in `claw run` and `claw test`\n", at(k.e.span), if k.sure { "sure" } else { "need" }, k.text);
            }
            let mut sures = Vec::new();
            sures_with_calls(&f.body, &mut sures);
            for (e, text) in sures {
                unbuilt += 1;
                out += &format!("no build {}  sure {text}  only in `claw run` and `claw test`\n", at(e.span));
            }
        }
    }
    out += &format!("audit: {raw} claw fn, {outside} extern, {unbuilt} checks not in `claw build`; std has {trusted} claw fn, not listed\n");
    out
}

fn sures_with_calls<'e>(b: &'e Block, out: &mut Vec<(&'e Expr, &'e str)>) {
    for s in &b.stmts {
        sures_in_stmt(s, out);
    }
}

fn sures_in_stmt<'e>(s: &'e Stmt, out: &mut Vec<(&'e Expr, &'e str)>) {
    match &s.kind {
        StmtKind::Sure(e, text) if effects::has_call(e) => out.push((e, text)),
        StmtKind::If { then, els, .. } => {
            sures_with_calls(then, out);
            match els {
                Some(Else::Block(e)) => sures_with_calls(e, out),
                Some(Else::If(i)) => sures_in_stmt(i, out),
                None => {}
            }
        }
        StmtKind::ForRange { body, .. } | StmtKind::ForIn { body, .. } | StmtKind::ForCond { body, .. } | StmtKind::ForEver { body } => sures_with_calls(body, out),
        StmtKind::Match { arms, .. } => {
            for a in arms {
                sures_with_calls(&a.body, out);
            }
        }
        _ => {}
    }
}
