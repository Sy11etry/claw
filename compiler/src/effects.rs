//! Checks what signatures promise: `+mem` (allocates) and `+io` (touches the world).
//! Private functions have their effects worked out; declared ones are an upper bound.

use crate::ast::*;
use crate::diag::{Diag, Span};
use crate::modules::Unit;
use crate::sema::{walk, Checked};
use crate::types::*;
use std::collections::HashMap;

fn mask_of(names: &[String]) -> u8 {
    names.iter().fold(0, |m, n| m | if n == "mem" { MEM } else { IO })
}

fn spell(mask: u8) -> String {
    spell_effects(mask)
}

/// `core.push[int]` is `push` to the person reading the message.
pub fn short_name(full: &str) -> &str {
    let base = full.split('[').next().unwrap_or(full);
    base.rsplit('.').next().unwrap_or(base)
}

/// One thing a function does that may have an effect.
struct Use {
    effect: u8,
    callee: Option<String>,
    span: Span,
    what: String,
    /// Comes from a type's `drop`, not from the function's own code.
    from_drop: bool,
}

struct Scan<'t> {
    types: &'t Types,
    uses: Vec<Use>,
    statements: Vec<Vec<Use>>,
    /// Functions used as values: (function, what the value's type allows, where).
    values: Vec<(String, u8, Span)>,
}

impl Scan<'_> {
    fn direct(&mut self, effect: u8, span: Span, what: &str) {
        self.uses.push(Use { effect, callee: None, span, what: what.to_string(), from_drop: false });
    }

    fn drops(&mut self, t: Ty, span: Span) {
        for d in self.types.user_drops(t) {
            self.uses.push(Use { effect: 0, callee: Some(d), span, what: "releasing this value".to_string(), from_drop: true });
        }
    }

    fn through_value(&mut self, callee: Ty, result: Ty, span: Span) {
        if let Kind::Fn(id) = self.types.kind(callee) {
            self.direct(self.types.fns[id].effects, span, "this call through a value");
        }
        self.drops(result, span);
    }

    fn node(&mut self, x: &Expr) {
        let ty = x.ty.get();
        match &x.kind {
            ExprKind::Var(_) | ExprKind::Field { .. } => {
                if let (Res::Func(full), Kind::Fn(id)) = (&*x.res.borrow(), self.types.kind(ty)) {
                    self.values.push((full.clone(), self.types.fns[id].effects, x.span));
                }
            }
            ExprKind::CallValue { f, .. } => self.through_value(f.ty.get(), ty, x.span),
            ExprKind::Call { .. } if matches!(*x.res.borrow(), Res::Value(_)) => {
                if let Res::Value(t) = *x.res.borrow() {
                    self.through_value(t, ty, x.span);
                }
            }
            ExprKind::Call { name, .. } => match &*x.res.borrow() {
                Res::Func(full) => {
                    self.uses.push(Use { effect: 0, callee: Some(full.clone()), span: x.span, what: format!("`{}`", short_name(full)), from_drop: false });
                    self.drops(ty, x.span);
                }
                Res::Const(..) => {}
                _ => match name.as_str() {
                    "print" => self.direct(IO, x.span, "`print`"),
                    "alloc" => self.direct(MEM, x.span, "`alloc`"),
                    "copy" if self.types.needs_drop(ty) => self.direct(MEM, x.span, "`copy`"),
                    _ => {}
                },
            },
            ExprKind::ArrayLit(items) if matches!(self.types.kind(ty), Kind::Slice(_)) && !items.is_empty() => {
                self.direct(MEM, x.span, "this array");
            }
            ExprKind::Interp(_) if ty == STR => self.direct(MEM, x.span, "this string"),
            ExprKind::StructLit { .. } => self.drops(ty, x.span),
            _ => {}
        }
    }

    fn expr(&mut self, e: &Expr) {
        let mut blocks: Vec<&Block> = Vec::new();
        walk(e, &mut |x| {
            self.node(x);
            if let ExprKind::Else { handler: ElseKind::Block(_, b), .. } = &x.kind {
                blocks.push(b);
            }
        });
        for b in blocks {
            self.block(b);
        }
    }

    fn pure(&mut self, e: &Expr) -> Vec<Use> {
        let saved = std::mem::take(&mut self.uses);
        self.expr(e);
        std::mem::replace(&mut self.uses, saved)
    }

    fn block(&mut self, b: &Block) {
        for s in &b.stmts {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match &s.kind {
            StmtKind::Let { init, .. } => {
                self.expr(init);
                self.drops(init.ty.get(), init.span);
            }
            StmtKind::Assign { target, value, .. } => {
                self.expr(target);
                self.expr(value);
            }
            StmtKind::Discard(e) | StmtKind::Expr(e) | StmtKind::Fail(e) => self.expr(e),
            StmtKind::Ret(v) => {
                if let Some(e) = v {
                    self.expr(e);
                }
            }
            StmtKind::If { cond, then, els } => {
                self.expr(cond);
                self.block(then);
                match els {
                    Some(Else::Block(b)) => self.block(b),
                    Some(Else::If(i)) => self.stmt(i),
                    None => {}
                }
            }
            StmtKind::ForRange { start, end, body, .. } => {
                self.expr(start);
                self.expr(end);
                self.block(body);
            }
            StmtKind::ForIn { subject, body, .. } => {
                self.expr(subject);
                self.block(body);
            }
            StmtKind::ForCond { cond, body } => {
                self.expr(cond);
                self.block(body);
            }
            StmtKind::ForEver { body } | StmtKind::Defer(body) => self.block(body),
            StmtKind::Match { subject, arms } => {
                self.expr(subject);
                for a in arms {
                    self.block(&a.body);
                }
            }
            StmtKind::Sure(e, _) => {
                let uses = self.pure(e);
                self.statements.push(uses);
            }
            StmtKind::Break | StmtKind::Continue => {}
        }
    }
}

struct Node<'a> {
    full: String,
    f: &'a Func,
    /// None: worked out from the body.
    declared: Option<u8>,
    uses: Vec<Use>,
    /// What each contract and `sure` statement does; contracts must do nothing.
    pure: Vec<Vec<Use>>,
    values: Vec<(String, u8, Span)>,
}

fn node<'a>(full: String, f: &'a Func, is_entry: bool, c: &Checked) -> Node<'a> {
    let mut scan = Scan { types: &c.types, uses: Vec::new(), statements: Vec::new(), values: Vec::new() };
    scan.block(&f.body);
    if let Some(sig) = c.sigs.get(&full) {
        for (p, t) in f.params.iter().zip(&sig.params) {
            if p.own {
                scan.drops(*t, p.span);
            }
        }
    }
    let mut pure: Vec<Vec<Use>> = f.contracts.iter().map(|c| scan.pure(&c.e)).collect();
    if f.test.is_none() {
        pure.append(&mut scan.statements);
    }
    let declared = if is_entry {
        Some(MEM | IO)
    } else if f.public || !f.effects.is_empty() {
        Some(mask_of(&f.effects))
    } else {
        None
    };
    Node { full, f, declared, uses: scan.uses, pure, values: scan.values }
}

/// Is there a call to an ordinary function in this expression?
pub fn has_call(e: &Expr) -> bool {
    let mut found = false;
    walk(e, &mut |x| found |= matches!(x.kind, ExprKind::CallValue { .. }) || (matches!(x.kind, ExprKind::Call { .. }) && matches!(*x.res.borrow(), Res::Func(_) | Res::Value(_))));
    found
}

fn count_stmts(b: &Block, n: &mut usize) {
    for s in &b.stmts {
        count_stmt(s, n);
    }
}

fn count_stmt(s: &Stmt, n: &mut usize) {
    match &s.kind {
        StmtKind::Sure(e, _) => *n += has_call(e) as usize,
        StmtKind::If { then, els, .. } => {
            count_stmts(then, n);
            match els {
                Some(Else::Block(b)) => count_stmts(b, n),
                Some(Else::If(i)) => count_stmt(i, n),
                None => {}
            }
        }
        StmtKind::ForRange { body, .. } | StmtKind::ForIn { body, .. } | StmtKind::ForCond { body, .. } | StmtKind::ForEver { body } => count_stmts(body, n),
        StmtKind::Match { arms, .. } => {
            for a in arms {
                count_stmts(&a.body, n);
            }
        }
        _ => {}
    }
}

/// What the effect check learned about the program.
pub struct Report {
    /// How many checks only run in check mode.
    pub slow: usize,
    pub known: HashMap<String, u8>,
}

impl Report {
    /// What this function does, for a signature that does not say.
    pub fn effects(&self, full: &str) -> u8 {
        self.known.get(full).copied().unwrap_or(0)
    }

    /// Does this function, or anything it calls, touch the world?
    pub fn has_io(&self, full: &str) -> bool {
        self.known.get(full).is_none_or(|e| e & IO != 0)
    }
}

/// Reports every broken promise.
pub fn check(units: &[Unit], c: &Checked, diags: &mut Vec<Diag>) -> Report {
    let root = units.len() - 1;
    let mut nodes: Vec<Node> = Vec::new();
    let mut slow = 0;
    for (i, u) in units.iter().enumerate() {
        for f in u.program.funcs.iter().filter(|f| f.tparams.is_empty()) {
            let is_entry = (i == root && f.name == "main") || f.test.is_some();
            nodes.push(node(format!("{}{}", u.prefix, f.name), f, is_entry, c));
            if !u.is_std && f.test.is_none() {
                slow += f.contracts.iter().filter(|x| has_call(&x.e)).count();
                count_stmts(&f.body, &mut slow);
            }
        }
    }
    for f in &c.instances {
        nodes.push(node(f.name.clone(), f, false, c));
    }

    // Private functions: repeat until the call graph stops adding anything.
    let mut known: HashMap<String, u8> = nodes.iter().map(|n| (n.full.clone(), n.declared.unwrap_or(0))).collect();
    let total = |uses: &[Use], known: &HashMap<String, u8>| uses.iter().fold(0, |m, u| m | u.effect | u.callee.as_ref().and_then(|x| known.get(x)).copied().unwrap_or(0));
    loop {
        let mut changed = false;
        for n in &nodes {
            let now = match n.declared {
                None => total(&n.uses, &known),
                // A template promises for its own code; what `T`'s `drop` does belongs to `T`.
                Some(d) if !n.f.tparams.is_empty() => {
                    let dropped: Vec<&Use> = n.uses.iter().filter(|u| u.from_drop).collect();
                    dropped.iter().fold(d, |m, u| m | u.callee.as_ref().and_then(|x| known.get(x)).copied().unwrap_or(0))
                }
                Some(d) => d,
            };
            // Only ever adds, so two functions with one name cannot flip it back and forth forever.
            let had = known[&n.full];
            if had | now != had {
                known.insert(n.full.clone(), had | now);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    for n in &nodes {
        let display = short_name(&n.full);
        if let Some(allowed) = n.declared {
            let mut told = 0;
            for u in n.uses.iter().filter(|u| !(u.from_drop && !n.f.tparams.is_empty())) {
                let has = u.effect | u.callee.as_ref().and_then(|x| known.get(x)).copied().unwrap_or(0);
                let missing = has & !allowed & !told;
                if missing != 0 {
                    told |= missing;
                    let word = spell(missing);
                    let msg = format!("`{display}` does not declare `{word}`, but {} has it", u.what);
                    diags.push(Diag::new("E0160", u.span, msg).fix(n.f.sig_end, format!(" {word}"), format!("add `{word}` to the signature")));
                }
            }
        }
        for (full, allowed, span) in &n.values {
            let extra = known.get(full).copied().unwrap_or(0) & !allowed;
            if extra != 0 {
                let word = spell(extra);
                diags.push(Diag::new("E0178", *span, format!("`{}` is `{word}`, which this function type does not allow; add `{word}` to the type", short_name(full))));
            }
        }
        for uses in &n.pure {
            for u in uses {
                let has = u.effect | u.callee.as_ref().and_then(|x| known.get(x)).copied().unwrap_or(0);
                if has != 0 {
                    diags.push(Diag::new("E0161", u.span, format!("a contract cannot have side effects, but {} is `{}`", u.what, spell(has))));
                }
            }
        }
    }
    for x in &c.computed {
        if known.get(&x.full).is_some_and(|e| e & IO != 0) {
            diags.push(Diag::new("E0171", x.span, format!("`{}` is computed while compiling, so it cannot use `+io`", x.name)));
        }
    }
    Report { slow, known }
}
