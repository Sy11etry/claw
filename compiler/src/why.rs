//! Turns a failure into its cause: the lines that fed the bad value, and a patch if one is plain.

use crate::ast::*;
use crate::diag::Span;
use crate::effects::short_name;
use crate::interp::{Failure, Kind, Machine};
use crate::modules::Unit;
use crate::sema::walk;
use crate::types::{self, Types};
use std::collections::{HashMap, HashSet};

/// Lines of cause in one report.
const MAX_LINES: usize = 10;
const MAX_TEXT: usize = 70;

pub struct Sources<'a> {
    pub units: &'a [Unit],
    pub base: &'a [String],
    pub types: &'a Types,
    text: Vec<String>,
    library: HashSet<u32>,
}

impl<'a> Sources<'a> {
    pub fn new(units: &'a [Unit], files: &[String], base: &'a [String], types: &'a Types) -> Sources<'a> {
        let text = files.iter().map(|f| std::fs::read_to_string(f).unwrap_or_default()).collect();
        let library = units.iter().filter(|u| u.is_std).flat_map(|u| u.program.funcs.iter().map(|f| f.name_span.file)).collect();
        Sources { units, base, types, text, library }
    }

    fn mine(&self, line: u32) -> bool {
        line != 0 && !self.library.contains(&(line >> 22))
    }

    fn line_text(&self, key: u32) -> String {
        let n = (key & 0x3FFFFF) as usize;
        let text = self.text[(key >> 22) as usize].lines().nth(n.wrapping_sub(1)).unwrap_or("").trim();
        if text.chars().count() > MAX_TEXT {
            let cut: String = text.chars().take(MAX_TEXT).collect();
            return format!("{cut}...");
        }
        text.to_string()
    }

    fn of(&self, s: Span) -> &str {
        self.text[s.file as usize].get(s.start as usize..s.end as usize).unwrap_or("")
    }
}

/// `file:line:col` at the end of a message's first line.
fn parse_at(msg: &str, base: &[String]) -> Option<(u32, u32, u32)> {
    let first = msg.lines().next()?;
    let place = first.rsplit(" at ").next()?;
    let mut parts = place.rsplit(':');
    let col = parts.next()?.parse().ok()?;
    let line = parts.next()?.parse().ok()?;
    let file: Vec<&str> = parts.collect();
    let name = file.into_iter().rev().collect::<Vec<_>>().join(":");
    Some((base.iter().position(|b| *b == name)? as u32, line, col))
}

/// The lines under a failure's headline: `why` and, if there is an obvious one, `fix`.
pub fn report(s: &Sources, mc: &Machine, f: &Failure, input: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(text) = input {
        out.push(format!("input  {text}"));
    }
    let (mut causes, lost) = mc.causes(&f.seeds, MAX_LINES, &|l| s.mine(l));
    // The line that failed comes last and says which values did it.
    let at = parse_at(&f.msg, s.base).map_or(f.line, |(file, line, _)| (file << 22) | line);
    let mut last = Vec::new();
    if !f.detail.is_empty() {
        last.push(f.detail.clone());
    }
    if s.mine(at) {
        match causes.iter().position(|c| c.line == at) {
            Some(i) => {
                let mut c = causes.remove(i);
                c.values.extend(last);
                causes.push(c);
            }
            None => causes.push(crate::interp::Cause { line: at, values: last }),
        }
    }
    for (i, c) in causes.iter().enumerate() {
        let place = format!("{}:{}", s.base[(c.line >> 22) as usize], c.line & 0x3FFFFF);
        let label = if i == 0 { "why " } else { "    " };
        let values = if c.values.is_empty() { String::new() } else { format!("    {}", c.values.join(", ")) };
        out.push(format!("{label}   {place}  {}{values}", s.line_text(c.line)));
    }
    if lost {
        out.push("       (earlier history was not kept)".to_string());
    }
    if let Some(fix) = suggest(s, f) {
        out.push(format!("fix    {fix}"));
    }
    out
}

/// Everything a function body says, in one pass.
struct Facts<'e> {
    exprs: Vec<&'e Expr>,
    /// Immutable locals and the expression that made them.
    lets: HashMap<&'e str, &'e Expr>,
    /// Compound assignments, whose position is the target's.
    updates: Vec<(&'e Expr, BinOp, &'e Expr)>,
}

fn gather<'e>(b: &'e Block, x: &mut Facts<'e>) {
    for st in &b.stmts {
        gather_stmt(st, x);
    }
}

fn gather_stmt<'e>(st: &'e Stmt, x: &mut Facts<'e>) {
    let see = |e: &'e Expr, x: &mut Facts<'e>| walk(e, &mut |y| x.exprs.push(y));
    match &st.kind {
        StmtKind::Let { mutable, name, init, .. } => {
            see(init, x);
            if !mutable {
                x.lets.insert(name, init);
            }
        }
        StmtKind::Assign { target, op, value } => {
            see(target, x);
            see(value, x);
            if let Some(op) = op {
                x.updates.push((target, *op, value));
            }
        }
        StmtKind::Discard(e) | StmtKind::Expr(e) | StmtKind::Fail(e) | StmtKind::Sure(e, _) => see(e, x),
        StmtKind::Ret(e) => {
            if let Some(e) = e {
                see(e, x);
            }
        }
        StmtKind::If { cond, then, els } => {
            see(cond, x);
            gather(then, x);
            match els {
                Some(Else::Block(b)) => gather(b, x),
                Some(Else::If(i)) => gather_stmt(i, x),
                None => {}
            }
        }
        StmtKind::ForRange { start, end, body, .. } => {
            see(start, x);
            see(end, x);
            gather(body, x);
        }
        StmtKind::ForIn { subject, body, .. } => {
            see(subject, x);
            gather(body, x);
        }
        StmtKind::ForCond { cond, body } => {
            see(cond, x);
            gather(body, x);
        }
        StmtKind::ForEver { body } | StmtKind::Defer(body) => gather(body, x),
        StmtKind::Match { subject, arms } => {
            see(subject, x);
            for a in arms {
                gather(&a.body, x);
            }
        }
        StmtKind::Break | StmtKind::Continue => {}
    }
}

/// Walks the names in `text`; `f` may replace one, or give up on the whole rewrite with `None`.
fn rewrite(text: &str, f: &mut dyn FnMut(&str) -> Option<Option<String>>) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'"' || c == b'\'' {
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != c {
                i += if bytes[i] == b'\\' { 2 } else { 1 };
            }
            i = (i + 1).min(bytes.len());
            out.push_str(&text[start..i]);
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &text[start..i];
            let field = start > 0 && bytes[start - 1] == b'.';
            let call = text[i..].trim_start().starts_with('(') || text[i..].trim_start().starts_with('[') && word.chars().next().is_some_and(|x| x.is_ascii_uppercase());
            if field || call || matches!(word, "and" | "or" | "not" | "true" | "false" | "none") {
                out.push_str(word);
            } else {
                match f(word)? {
                    Some(r) => out.push_str(&r),
                    None => out.push_str(word),
                }
            }
        } else if c.is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            out.push_str(&text[start..i]);
        } else {
            let ch = text[i..].chars().next()?;
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    Some(out)
}

fn wrap(text: String) -> String {
    if text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') || (text.ends_with(')') && !text[..text.len() - 1].contains(')') && !text.contains(' ')) {
        text
    } else {
        format!("({text})")
    }
}

/// Rewrites a condition so that it mentions only the function's own parameters.
fn in_terms_of_params(s: &Sources, text: &str, f: &Func, x: &Facts, depth: u32) -> Option<String> {
    if depth > 6 {
        return None;
    }
    rewrite(text, &mut |word| {
        if f.params.iter().any(|p| p.name == word && !matches!(p.ty.kind, TypeKind::Ref(_))) {
            return Some(None);
        }
        let init = x.lets.get(word)?;
        let inner = s.of(init.span).to_string();
        if inner.is_empty() || inner.contains('?') || inner.contains(" else ") {
            return None;
        }
        Some(Some(wrap(in_terms_of_params(s, &inner, f, x, depth + 1)?)))
    })
}

fn simple_int(e: &Expr) -> bool {
    matches!(e.kind, ExprKind::Int(_))
}

/// A `need` that would have kept the bad input out, when one can be read off the code.
fn suggest(s: &Sources, fail: &Failure) -> Option<String> {
    if !matches!(fail.kind, Kind::DivZero | Kind::Bounds | Kind::Shift | Kind::Convert | Kind::AllocSize | Kind::Need) {
        return None;
    }
    let (file, line, col) = parse_at(&fail.msg, s.base)?;
    let hit = |sp: Span| sp.file == file && sp.line == line && sp.col == col;
    for u in s.units.iter().filter(|u| !u.is_std) {
        for f in &u.program.funcs {
            if f.name_span.file != file || f.name == "main" || f.test.is_some() || f.name.starts_with("const ") {
                continue;
            }
            let mut x = Facts { exprs: Vec::new(), lets: HashMap::new(), updates: Vec::new() };
            gather(&f.body, &mut x);
            let mut wanted: Option<String> = None;
            for (target, op, value) in &x.updates {
                if hit(target.span) && matches!(op, BinOp::Div | BinOp::Rem) {
                    wanted = Some(format!("{} != 0", s.of(value.span)));
                }
            }
            for e in x.exprs.iter().filter(|e| hit(e.span)) {
                let got = match &e.kind {
                    ExprKind::Binary { op: BinOp::Div | BinOp::Rem, r, .. } if fail.kind == Kind::DivZero => Some(format!("{} != 0", s.of(r.span))),
                    ExprKind::Binary { op: BinOp::Shl | BinOp::Shr, l, r } if fail.kind == Kind::Shift => {
                        let bits = s.types.size(l.ty.get()) * 8;
                        Some(format!("{} >= 0 and {} < {bits}", s.of(r.span), s.of(r.span)))
                    }
                    ExprKind::Index { e: base, i } if fail.kind == Kind::Bounds => {
                        let (bt, it) = (s.of(base.span), s.of(i.span));
                        Some(if simple_int(i) { format!("{it} < len({bt})") } else { format!("{it} >= 0 and {it} < len({bt})") })
                    }
                    ExprKind::Call { name, args, .. } if fail.kind == Kind::Convert && args.len() == 1 => match s.types.kind(e.ty.get()) {
                        types::Kind::Int { bits, signed: true } if bits < 64 => {
                            let hi = (1i128 << (bits - 1)) - 1;
                            let a = wrap(s.of(args[0].span).to_string());
                            let _ = name;
                            Some(format!("{a} >= {} and {a} <= {hi}", -hi - 1))
                        }
                        _ => None,
                    },
                    ExprKind::Call { name, args, .. } if fail.kind == Kind::AllocSize && name == "alloc" && args.len() == 1 => Some(format!("{} >= 0", wrap(s.of(args[0].span).to_string()))),
                    ExprKind::Call { args, .. } if fail.kind == Kind::Need => need_of_callee(s, fail, e, args),
                    _ => None,
                };
                if got.is_some() {
                    wanted = got;
                }
            }
            let Some(cond) = wanted else { continue };
            let cond = in_terms_of_params(s, &cond, f, &x, 0)?;
            let cond = drop_obvious(cond);
            return Some(format!("add `need {cond}` to {}", short_name(&f.name)));
        }
    }
    None
}

/// `3 >= 0 and 3 < len(a)` says more than it needs to.
fn drop_obvious(cond: String) -> String {
    if let Some((first, rest)) = cond.split_once(" >= 0 and ") {
        if first.chars().all(|c| c.is_ascii_digit()) {
            return rest.to_string();
        }
    }
    cond
}

/// The callee's failed `need`, rewritten in the caller's words.
fn need_of_callee(s: &Sources, fail: &Failure, call: &Expr, args: &[Expr]) -> Option<String> {
    let Res::Func(full) = &*call.res.borrow() else { return None };
    let text = fail.msg.lines().next()?.split('`').nth(1)?.to_string();
    let callee = s.units.iter().flat_map(|u| u.program.funcs.iter().map(move |f| (format!("{}{}", u.prefix, f.name), f))).find(|(n, _)| n == full.split('[').next().unwrap_or(full))?.1;
    rewrite(&text, &mut |word| {
        let i = callee.params.iter().position(|p| p.name == word)?;
        Some(Some(wrap(s.of(args.get(i)?.span).to_string())))
    })
}
