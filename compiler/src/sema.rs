//! Name resolution, type checking and the strictness rules.

use crate::ast::*;
use crate::diag::{Diag, Span};
use crate::types::*;
use std::collections::HashMap;

pub struct FnSig {
    pub params: Vec<Ty>,
    pub ret: Ty,
}

#[derive(Clone, Copy, Debug)]
pub enum ConstVal {
    Int(i128),
    Float(f64),
    Bool(bool),
}

#[derive(Clone, Copy)]
pub enum NameDef {
    Type(Ty),
    Variant(usize, usize, Ty),
}

pub struct Checked {
    pub types: Types,
    pub sigs: HashMap<String, FnSig>,
    pub names: HashMap<String, NameDef>,
    pub consts: HashMap<String, (ConstVal, Ty)>,
}

struct Var {
    ty: Ty,
    mutable: bool,
    read: bool,
    written: bool,
    span: Span,
    decl: Span,
}

const BUILTINS: &[&str] = &["print", "len", "int", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "bool"];

fn prim(name: &str) -> Option<Ty> {
    Some(match name {
        "int" | "i64" => INT,
        "i8" => I8,
        "i16" => I16,
        "i32" => I32,
        "u8" => U8,
        "u16" => U16,
        "u32" => U32,
        "u64" => U64,
        "f32" => F32,
        "f64" => F64,
        "bool" => BOOL,
        _ => return None,
    })
}

struct Global {
    types: Types,
    names: HashMap<String, NameDef>,
    consts: HashMap<String, (ConstVal, Ty)>,
}

impl Global {
    fn resolve(&mut self, t: &TypeExpr, diags: &mut Vec<Diag>) -> Ty {
        match &t.kind {
            TypeKind::Name(n) => {
                if let Some(p) = prim(n) {
                    return p;
                }
                if n == "str" {
                    diags.push(Diag::new("E0900", t.span, "type `str` is not available in claw 0.2"));
                    return ERR;
                }
                match self.names.get(n) {
                    Some(NameDef::Type(ty)) => *ty,
                    Some(NameDef::Variant(..)) => {
                        diags.push(Diag::new("E0112", t.span, format!("`{n}` is a variant, not a type")));
                        ERR
                    }
                    None => {
                        diags.push(Diag::new("E0112", t.span, format!("unknown type `{n}`")));
                        ERR
                    }
                }
            }
            TypeKind::Array(size, elem) => {
                let n = match size {
                    Size::Lit(v) => *v,
                    Size::Name(name) => match self.consts.get(name) {
                        Some((ConstVal::Int(v), _)) => *v,
                        _ => {
                            diags.push(Diag::new("E0126", t.span, format!("array size `{name}` must be a number or an integer `const`")));
                            return ERR;
                        }
                    },
                };
                if n <= 0 || n > (1 << 40) {
                    diags.push(Diag::new("E0126", t.span, "array size must be at least 1"));
                    return ERR;
                }
                let e = self.resolve(elem, diags);
                if e == ERR {
                    return ERR;
                }
                self.types.intern(Kind::Array(e, n as u64))
            }
            TypeKind::Option(inner) => {
                let p = self.resolve(inner, diags);
                if p == ERR {
                    return ERR;
                }
                self.types.intern(Kind::Option(p))
            }
            TypeKind::Result(ok, err) => {
                let o = match ok {
                    Some(k) => self.resolve(k, diags),
                    None => VOID,
                };
                let e = self.resolve(err, diags);
                if o == ERR || e == ERR {
                    return ERR;
                }
                self.types.intern(Kind::Result(o, e))
            }
        }
    }
}

fn int_range(bits: u8, signed: bool) -> (i128, i128) {
    if signed {
        (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
    } else {
        (0, (1i128 << bits) - 1)
    }
}

fn check_case(name: &str, upper: bool, what: &str, span: Span, diags: &mut Vec<Diag>) {
    let first = name.chars().next().unwrap_or('_');
    let bad = if upper { !first.is_ascii_uppercase() } else { first.is_ascii_uppercase() };
    if bad {
        let want = if upper { "start with a capital letter" } else { "start with a small letter" };
        diags.push(Diag::new("E0128", span, format!("{what} `{name}` must {want}")));
    }
}

fn eval_const(g: &Global, e: &Expr, diags: &mut Vec<Diag>) -> Option<ConstVal> {
    let bad = |diags: &mut Vec<Diag>, span: Span| {
        diags.push(Diag::new("E0139", span, "a `const` must be computable from literals and earlier constants"));
        None
    };
    match &e.kind {
        ExprKind::Int(v) => Some(ConstVal::Int(*v)),
        ExprKind::Float(v) => Some(ConstVal::Float(*v)),
        ExprKind::Char(c) => Some(ConstVal::Int(*c as i128)),
        ExprKind::Bool(b) => Some(ConstVal::Bool(*b)),
        ExprKind::Var(n) => match g.consts.get(n) {
            Some((v, _)) => Some(*v),
            None => bad(diags, e.span),
        },
        ExprKind::Unary { op, e: inner } => match (op, eval_const(g, inner, diags)?) {
            (UnOp::Neg, ConstVal::Int(v)) => Some(ConstVal::Int(-v)),
            (UnOp::Neg, ConstVal::Float(v)) => Some(ConstVal::Float(-v)),
            (UnOp::BitNot, ConstVal::Int(v)) => Some(ConstVal::Int(!v)),
            (UnOp::Not, ConstVal::Bool(v)) => Some(ConstVal::Bool(!v)),
            _ => bad(diags, e.span),
        },
        ExprKind::Binary { op, l, r } => {
            let a = eval_const(g, l, diags)?;
            let b = eval_const(g, r, diags)?;
            let v = match (a, b) {
                (ConstVal::Int(x), ConstVal::Int(y)) => match op {
                    BinOp::Add => x.checked_add(y),
                    BinOp::Sub => x.checked_sub(y),
                    BinOp::Mul => x.checked_mul(y),
                    BinOp::Div if y != 0 => x.checked_div(y),
                    BinOp::Rem if y != 0 => x.checked_rem(y),
                    BinOp::BitAnd => Some(x & y),
                    BinOp::BitOr => Some(x | y),
                    BinOp::BitXor => Some(x ^ y),
                    BinOp::Shl if (0..64).contains(&y) => Some(x << y),
                    BinOp::Shr if (0..64).contains(&y) => Some(x >> y),
                    _ => None,
                }
                .map(ConstVal::Int),
                (ConstVal::Float(x), ConstVal::Float(y)) => match op {
                    BinOp::Add => Some(ConstVal::Float(x + y)),
                    BinOp::Sub => Some(ConstVal::Float(x - y)),
                    BinOp::Mul => Some(ConstVal::Float(x * y)),
                    BinOp::Div => Some(ConstVal::Float(x / y)),
                    _ => None,
                },
                _ => None,
            };
            match v {
                Some(v) => Some(v),
                None => bad(diags, e.span),
            }
        }
        _ => bad(diags, e.span),
    }
}

fn collect_consts(g: &mut Global, p: &Program, diags: &mut Vec<Diag>) {
    for c in &p.consts {
        check_case(&c.name, false, "constant", c.span, diags);
        if g.consts.contains_key(&c.name) || BUILTINS.contains(&c.name.as_str()) {
            diags.push(Diag::new("E0102", c.span, format!("`{}` is already defined", c.name)));
            continue;
        }
        let want = c.ty.as_ref().map(|t| g.resolve(t, diags));
        let v = match eval_const(g, &c.value, diags) {
            Some(v) => v,
            None => {
                // Already reported: uses of this name should not report again.
                g.consts.insert(c.name.clone(), (ConstVal::Int(0), ERR));
                continue;
            }
        };
        let (v, ty) = match (v, want) {
            (ConstVal::Int(n), None) => (ConstVal::Int(n), INT),
            (ConstVal::Float(n), None) => (ConstVal::Float(n), F64),
            (ConstVal::Bool(b), None) => (ConstVal::Bool(b), BOOL),
            (ConstVal::Int(n), Some(t)) if g.types.is_float(t) => (ConstVal::Float(n as f64), t),
            (ConstVal::Float(n), Some(t)) if g.types.is_float(t) => (ConstVal::Float(n), t),
            (ConstVal::Bool(b), Some(BOOL)) => (ConstVal::Bool(b), BOOL),
            (ConstVal::Int(n), Some(t)) if g.types.is_int(t) => (ConstVal::Int(n), t),
            (_, Some(ERR)) => continue,
            (_, Some(t)) => {
                diags.push(Diag::new("E0104", c.value.span, format!("this value cannot be {}", g.types.name(t))));
                continue;
            }
        };
        if let (ConstVal::Int(n), Kind::Int { bits, signed }) = (v, g.types.kind(ty)) {
            let (lo, hi) = int_range(bits, signed);
            if n < lo || n > hi {
                diags.push(Diag::new("E0115", c.value.span, format!("number does not fit in {}", g.types.name(ty))));
                continue;
            }
        }
        g.consts.insert(c.name.clone(), (v, ty));
    }
}

fn define_name(g: &mut Global, name: &str, def: NameDef, span: Span, diags: &mut Vec<Diag>) -> bool {
    if g.names.contains_key(name) || prim(name).is_some() {
        diags.push(Diag::new("E0110", span, format!("`{name}` is defined twice")));
        return false;
    }
    g.names.insert(name.to_string(), def);
    true
}

fn collect_types(g: &mut Global, p: &Program, diags: &mut Vec<Diag>) {
    // Names first, so types can refer to each other in any order.
    let mut ids: Vec<Option<Ty>> = Vec::new();
    for d in &p.types {
        check_case(&d.name, true, "type", d.span, diags);
        let ty = match &d.body {
            TypeBody::Struct(_) => {
                g.types.structs.push(StructDef { name: d.name.clone(), fields: Vec::new(), size: 0, align: 1 });
                g.types.intern(Kind::Struct(g.types.structs.len() - 1))
            }
            TypeBody::Variants(_) => {
                g.types.enums.push(EnumDef { name: d.name.clone(), variants: Vec::new(), size: TAG_SIZE });
                g.types.intern(Kind::Enum(g.types.enums.len() - 1))
            }
        };
        ids.push(if define_name(g, &d.name, NameDef::Type(ty), d.span, diags) { Some(ty) } else { None });
    }

    let mut field_types: Vec<Vec<Vec<(String, Ty)>>> = Vec::new();
    for (d, id) in p.types.iter().zip(&ids) {
        let resolve_fields = |g: &mut Global, fields: &[FieldDecl], diags: &mut Vec<Diag>| -> Vec<(String, Ty)> {
            let mut out: Vec<(String, Ty)> = Vec::new();
            for f in fields {
                check_case(&f.name, false, "field", f.span, diags);
                if out.iter().any(|(n, _)| *n == f.name) {
                    diags.push(Diag::new("E0137", f.span, format!("field `{}` appears twice", f.name)));
                }
                out.push((f.name.clone(), g.resolve(&f.ty, diags)));
            }
            out
        };
        let mut groups = Vec::new();
        match &d.body {
            TypeBody::Struct(fields) => groups.push(resolve_fields(g, fields, diags)),
            TypeBody::Variants(vars) => {
                for (i, v) in vars.iter().enumerate() {
                    check_case(&v.name, true, "variant", v.span, diags);
                    if let (Some(ty), Kind::Enum(eid)) = (*id, id.map(|t| g.types.kind(t)).unwrap_or(Kind::Err)) {
                        define_name(g, &v.name, NameDef::Variant(eid, i, ty), v.span, diags);
                    }
                    groups.push(resolve_fields(g, &v.fields, diags));
                }
            }
        }
        field_types.push(groups);
    }

    // Lay out each declaration after everything it contains by value.
    let mut state = vec![0u8; p.types.len()];
    let index: HashMap<Ty, usize> = ids.iter().enumerate().filter_map(|(i, t)| t.map(|t| (t, i))).collect();
    for i in 0..p.types.len() {
        layout_decl(g, p, &field_types, &ids, &index, &mut state, i, diags);
    }
}

#[allow(clippy::too_many_arguments)]
fn layout_decl(
    g: &mut Global,
    p: &Program,
    field_types: &[Vec<Vec<(String, Ty)>>],
    ids: &[Option<Ty>],
    index: &HashMap<Ty, usize>,
    state: &mut Vec<u8>,
    i: usize,
    diags: &mut Vec<Diag>,
) {
    if state[i] != 0 {
        return;
    }
    state[i] = 1;
    let d = &p.types[i];
    for group in &field_types[i] {
        for (_, t) in group {
            let mut deps = Vec::new();
            direct_deps(&g.types, *t, &mut deps);
            for dep in deps {
                if let Some(&j) = index.get(&dep) {
                    if state[j] == 1 {
                        diags.push(Diag::new("E0127", d.span, format!("type `{}` contains itself, so it would be infinitely large", d.name)));
                    } else {
                        layout_decl(g, p, field_types, ids, index, state, j, diags);
                    }
                }
            }
        }
    }
    state[i] = 2;
    let Some(ty) = ids[i] else { return };
    let lay = |g: &Global, group: &[(String, Ty)], start: u64| -> (Vec<Field>, u64, u64) {
        let (mut off, mut al) = (start, 1u64);
        let mut fields = Vec::new();
        for (name, t) in group {
            let (s, a) = g.types.layout(*t);
            off = align_up(off, a);
            fields.push(Field { name: name.clone(), ty: *t, offset: off });
            off += s;
            al = al.max(a);
        }
        (fields, off, al)
    };
    match g.types.kind(ty) {
        Kind::Struct(id) => {
            let (fields, end, al) = lay(g, &field_types[i][0], 0);
            let def = &mut g.types.structs[id];
            def.fields = fields;
            def.size = align_up(end, al);
            def.align = al;
        }
        Kind::Enum(id) => {
            let TypeBody::Variants(vars) = &d.body else { return };
            let mut size = TAG_SIZE;
            let mut built = Vec::new();
            for (v, group) in vars.iter().zip(&field_types[i]) {
                let (fields, end, _) = lay(g, group, TAG_SIZE);
                size = size.max(end);
                built.push(Variant { name: v.name.clone(), fields });
            }
            let def = &mut g.types.enums[id];
            def.variants = built;
            def.size = align_up(size, 8);
        }
        _ => {}
    }
}

fn direct_deps(types: &Types, t: Ty, out: &mut Vec<Ty>) {
    match types.kind(t) {
        Kind::Struct(_) | Kind::Enum(_) => out.push(t),
        Kind::Array(e, _) | Kind::Option(e) => direct_deps(types, e, out),
        Kind::Result(a, b) => {
            direct_deps(types, a, out);
            direct_deps(types, b, out);
        }
        _ => {}
    }
}

pub fn check(p: &Program, diags: &mut Vec<Diag>) -> Checked {
    let mut g = Global { types: Types::new(), names: HashMap::new(), consts: HashMap::new() };
    collect_consts(&mut g, p, diags);
    collect_types(&mut g, p, diags);

    let mut sigs: HashMap<String, FnSig> = HashMap::new();
    for f in &p.funcs {
        check_case(&f.name, false, "function", f.name_span, diags);
        if BUILTINS.contains(&f.name.as_str()) {
            diags.push(Diag::new("E0102", f.name_span, format!("`{}` is a built-in name", f.name)));
            continue;
        }
        if sigs.contains_key(&f.name) || g.consts.contains_key(&f.name) {
            diags.push(Diag::new("E0110", f.name_span, format!("function `{}` is defined twice", f.name)));
            continue;
        }
        let params = f.params.iter().map(|x| g.resolve(&x.ty, diags)).collect();
        let ret = f.ret.as_ref().map(|t| g.resolve(t, diags)).unwrap_or(VOID);
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
        let Some(sig) = sigs.get(&f.name) else { continue };
        let param_types = sig.params.clone();
        let ret = sig.ret;
        let mut cx = Ctx {
            g: &mut g,
            sigs: &sigs,
            scopes: vec![HashMap::new()],
            diags: &mut *diags,
            ret,
            loops: 0,
            is_main: f.name == "main",
            locks: Vec::new(),
        };
        for (x, ty) in f.params.iter().zip(param_types) {
            if x.name != "_" {
                cx.declare(&x.name, ty, false, x.span, x.span);
            }
        }
        let ends = cx.block(&f.body);
        cx.pop_scope();
        if !ends && ret != VOID && ret != ERR && !matches!(cx.kind(ret), Kind::Result(VOID, _)) {
            let msg = format!("`{}` can reach its end without `ret`", f.name);
            cx.diags.push(Diag::new("E0107", f.body.end, msg));
        }
    }
    Checked { types: g.types, sigs, names: g.names, consts: g.consts }
}

struct Ctx<'a> {
    g: &'a mut Global,
    sigs: &'a HashMap<String, FnSig>,
    scopes: Vec<HashMap<String, Var>>,
    diags: &'a mut Vec<Diag>,
    ret: Ty,
    loops: u32,
    is_main: bool,
    /// Arrays being walked by an enclosing `for ... in`.
    locks: Vec<String>,
}

fn is_lit(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) => true,
        ExprKind::Unary { op: UnOp::Neg | UnOp::BitNot, e } => is_lit(e),
        ExprKind::Binary { op, l, r } => {
            matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr)
                && is_lit(l)
                && is_lit(r)
        }
        _ => false,
    }
}

fn root_name(e: &Expr) -> Option<&str> {
    match &e.kind {
        ExprKind::Var(n) => Some(n),
        ExprKind::Field { e, .. } | ExprKind::Index { e, .. } => root_name(e),
        _ => None,
    }
}

impl<'a> Ctx<'a> {
    fn kind(&self, t: Ty) -> Kind {
        self.g.types.kind(t)
    }

    fn tname(&self, t: Ty) -> String {
        self.g.types.name(t)
    }

    fn err(&mut self, code: &'static str, span: Span, msg: impl Into<String>) -> Ty {
        self.diags.push(Diag::new(code, span, msg));
        ERR
    }

    fn lookup(&mut self, name: &str) -> Option<&mut Var> {
        self.scopes.iter_mut().rev().find_map(|s| s.get_mut(name))
    }

    fn declare(&mut self, name: &str, ty: Ty, mutable: bool, span: Span, decl: Span) {
        check_case(name, false, "name", span, self.diags);
        let taken = self.lookup(name).is_some()
            || BUILTINS.contains(&name)
            || self.sigs.contains_key(name)
            || self.g.consts.contains_key(name);
        if taken {
            self.diags.push(Diag::new("E0102", span, format!("`{name}` is already defined; names cannot be reused or hidden")));
            if self.lookup(name).is_none() {
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
                let d = Diag::new("E0105", v.span, format!("`{name}` is never read")).fix(v.span, "_", format!("rename `{name}` to `_`"));
                self.diags.push(d);
            } else if v.mutable && !v.written {
                let d = Diag::new("E0116", v.span, format!("`{name}` is `mut` but never changes")).fix(v.decl, "", "remove `mut`");
                self.diags.push(d);
            }
        }
    }

    /// Returns true if control can never run past the end of the block.
    fn block(&mut self, b: &Block) -> bool {
        self.scopes.push(HashMap::new());
        let ends = self.stmts(b);
        self.pop_scope();
        ends
    }

    fn stmts(&mut self, b: &Block) -> bool {
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
        ends
    }

    /// Checks `e` against `want`, allowing `T` where `?T` or `T ! E` is expected.
    fn check(&mut self, e: &Expr, want: Ty) {
        let got = self.expr(e, Some(want));
        if got == want || got == ERR || want == ERR {
            return;
        }
        match self.kind(want) {
            Kind::Option(p) if p == got => {
                e.coerce.set(Coerce::Some(want));
                return;
            }
            Kind::Result(ok, _) if ok == got => {
                e.coerce.set(Coerce::Ok(want));
                return;
            }
            _ => {}
        }
        self.mismatch(e.span, got, want);
    }

    fn mismatch(&mut self, span: Span, got: Ty, want: Ty) {
        if got == ERR || want == ERR {
            return;
        }
        let mut msg = format!("expected {}, found {}", self.tname(want), self.tname(got));
        if self.g.types.is_num(got) && self.g.types.is_num(want) {
            msg.push_str(&format!("; convert with `{}(...)`", self.tname(want)));
        }
        self.diags.push(Diag::new("E0104", span, msg));
    }

    fn value_type(&mut self, span: Span, ty: Ty) -> Ty {
        match self.kind(ty) {
            Kind::Void => self.err("E0104", span, "this has no value"),
            Kind::StrLit => self.err("E0900", span, "strings outside `print` are not available in claw 0.2"),
            _ => ty,
        }
    }

    /// The type a literal should aim for, looking through `?T` and `T ! E`.
    fn strip(&self, hint: Option<Ty>) -> Option<Ty> {
        hint.map(|h| match self.kind(h) {
            Kind::Option(p) => p,
            Kind::Result(ok, _) => ok,
            _ => h,
        })
    }

    fn place(&mut self, e: &Expr) -> Ty {
        let t = match &e.kind {
            ExprKind::Var(name) => match self.lookup(name).map(|v| v.ty) {
                Some(t) => t,
                None => self.err("E0101", e.span, format!("unknown name `{name}`; declare it with `{name} := ...`")),
            },
            ExprKind::Field { e: base, name, name_span } => {
                let b = self.place(base);
                self.field_type(b, name, *name_span)
            }
            ExprKind::Index { e: base, i } => {
                let b = self.place(base);
                self.index_type(b, i, base.span)
            }
            _ => self.err("E0131", e.span, "this cannot be assigned to"),
        };
        e.ty.set(t);
        t
    }

    fn field_type(&mut self, base: Ty, name: &str, span: Span) -> Ty {
        match self.kind(base) {
            Kind::Struct(id) => match self.g.types.structs[id].fields.iter().find(|f| f.name == name) {
                Some(f) => f.ty,
                None => {
                    let t = self.tname(base);
                    self.err("E0129", span, format!("`{t}` has no field `{name}`"))
                }
            },
            Kind::Err => ERR,
            _ => {
                let t = self.tname(base);
                self.err("E0129", span, format!("{t} has no fields"))
            }
        }
    }

    fn index_type(&mut self, base: Ty, i: &Expr, base_span: Span) -> Ty {
        self.check(i, INT);
        match self.kind(base) {
            Kind::Array(elem, n) => {
                if let ExprKind::Int(v) = i.kind {
                    if v < 0 || v >= n as i128 {
                        self.diags.push(Diag::new("E0130", i.span, format!("index {v} is outside `{}`", self.tname(base))));
                    }
                }
                elem
            }
            Kind::Err => ERR,
            _ => {
                let t = self.tname(base);
                self.err("E0129", base_span, format!("{t} cannot be indexed"))
            }
        }
    }

    fn stmt(&mut self, s: &Stmt) -> bool {
        match &s.kind {
            StmtKind::Let { mutable, name, name_span, ty, init } => {
                let final_ty = match ty {
                    Some(t) => {
                        let want = self.g.resolve(t, self.diags);
                        self.check(init, want);
                        want
                    }
                    None => {
                        let got = self.expr(init, None);
                        self.value_type(init.span, got)
                    }
                };
                let decl = if *mutable {
                    Span { start: s.span.start, end: name_span.start, line: s.span.line, col: s.span.col }
                } else {
                    Span { start: s.span.start, end: s.span.start, line: s.span.line, col: s.span.col }
                };
                self.declare(name, final_ty, *mutable, *name_span, decl);
                false
            }
            StmtKind::Assign { target, op, value } => {
                let ty = self.place(target);
                match root_name(target) {
                    Some(root) => {
                        let root = root.to_string();
                        let info = self.lookup(&root).map(|v| (v.mutable, v.decl));
                        if let Some((mutable, decl)) = info {
                            if !mutable {
                                let d = Diag::new("E0103", target.span, format!("`{root}` cannot change")).fix(decl, "mut ", format!("declare `mut {root}`"));
                                self.diags.push(d);
                            }
                            self.lookup(&root).unwrap().written = true;
                        }
                        if self.locks.contains(&root) {
                            self.diags.push(Diag::new("E0122", target.span, format!("`{root}` cannot change while a `for` is walking it")));
                        }
                    }
                    None if ty != ERR => {
                        self.diags.push(Diag::new("E0131", target.span, "this cannot be assigned to"));
                    }
                    None => {}
                }
                if let Some(o) = op {
                    if !self.g.types.is_num(ty) && ty != ERR {
                        let n = self.tname(ty);
                        self.diags.push(Diag::new("E0104", target.span, format!("`{}=` needs a number, found {n}", o.text())));
                    } else if self.g.types.is_float(ty) && !matches!(o, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div) {
                        self.diags.push(Diag::new("E0104", target.span, format!("`{}=` does not work on floats", o.text())));
                    }
                }
                self.check(value, ty);
                false
            }
            StmtKind::Discard(e) => {
                let got = self.expr(e, None);
                let got = self.value_type(e.span, got);
                if matches!(self.kind(got), Kind::Result(..)) {
                    self.diags.push(Diag::new("E0125", e.span, "an error cannot be ignored; use `?` or `else`"));
                }
                false
            }
            StmtKind::Expr(e) => {
                let got = self.expr(e, None);
                match &e.kind {
                    ExprKind::Call { .. } | ExprKind::Try(_) | ExprKind::Else { .. } => {
                        if got != VOID && got != ERR {
                            let d = Diag::new("E0111", e.span, "result is not used").fix(e.span, "_ = ", "assign it to `_` if it really does not matter");
                            self.diags.push(d);
                        }
                    }
                    _ => self.diags.push(Diag::new("E0114", e.span, "this expression does nothing")),
                }
                false
            }
            StmtKind::If { cond, then, els } => {
                self.check(cond, BOOL);
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
                self.check(start, INT);
                self.check(end, INT);
                self.scopes.push(HashMap::new());
                if var != "_" {
                    self.declare(var, INT, false, *var_span, *var_span);
                }
                self.loop_body(body);
                self.pop_scope();
                false
            }
            StmtKind::ForIn { elem, elem_span, index, subject, body } => {
                let t = self.expr(subject, None);
                let elem_ty = match self.kind(t) {
                    Kind::Array(e, _) => e,
                    Kind::Err => ERR,
                    _ => {
                        let n = self.tname(t);
                        self.err("E0104", subject.span, format!("`for ... in` needs an array, found {n}; use `a..b` for numbers"))
                    }
                };
                self.scopes.push(HashMap::new());
                if elem != "_" {
                    self.declare(elem, elem_ty, false, *elem_span, *elem_span);
                }
                if let Some((n, sp)) = index {
                    if n != "_" {
                        self.declare(n, INT, false, *sp, *sp);
                    }
                }
                let root = root_name(subject).map(|s| s.to_string());
                let locked = root.is_some();
                if let Some(r) = root {
                    self.locks.push(r);
                }
                self.loop_body(body);
                if locked {
                    self.locks.pop();
                }
                self.pop_scope();
                false
            }
            StmtKind::ForCond { cond, body } => {
                self.check(cond, BOOL);
                self.loop_body(body);
                false
            }
            StmtKind::ForEver { body } => {
                self.loop_body(body);
                !has_break(body)
            }
            StmtKind::Match { subject, arms } => self.match_stmt(s.span, subject, arms),
            StmtKind::Ret(v) => {
                match v {
                    Some(e) => {
                        if self.ret == VOID {
                            self.expr(e, None);
                            self.diags.push(Diag::new("E0104", e.span, "this function returns nothing; write `ret` alone"));
                        } else {
                            self.check(e, self.ret);
                        }
                    }
                    None => {
                        let ok_void = matches!(self.kind(self.ret), Kind::Result(VOID, _));
                        if self.ret != VOID && self.ret != ERR && !ok_void {
                            let n = self.tname(self.ret);
                            self.diags.push(Diag::new("E0104", s.span, format!("this function must return {n}")));
                        }
                    }
                }
                true
            }
            StmtKind::Fail(e) => {
                match self.kind(self.ret) {
                    Kind::Result(_, err) => self.check(e, err),
                    Kind::Err => {
                        self.expr(e, None);
                    }
                    _ => {
                        self.expr(e, None);
                        self.diags.push(Diag::new("E0124", s.span, "`fail` needs a function returning `T ! E`"));
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

    fn loop_body(&mut self, body: &Block) {
        self.loops += 1;
        self.block(body);
        self.loops -= 1;
    }

    fn match_stmt(&mut self, span: Span, subject: &Expr, arms: &[Arm]) -> bool {
        let t = self.expr(subject, None);
        let t = self.value_type(subject.span, t);
        let kind = self.kind(t);
        let mut seen_variants: Vec<usize> = Vec::new();
        let mut seen_ints: Vec<i128> = Vec::new();
        let mut seen_bools: Vec<bool> = Vec::new();
        let mut wild = false;
        let mut all_end = true;

        match kind {
            Kind::Enum(_) | Kind::Int { .. } | Kind::Bool | Kind::Err => {}
            _ => {
                let n = self.tname(t);
                self.diags.push(Diag::new("E0104", subject.span, format!("`match` works on variants, integers and bool, not {n}")));
            }
        }

        for arm in arms {
            if wild {
                self.diags.push(Diag::new("E0113", arm.span, "this arm can never run: `_` above catches everything"));
            }
            self.scopes.push(HashMap::new());
            match &arm.pat {
                Pat::Wild => wild = true,
                Pat::Bool(b) => {
                    if kind != Kind::Bool && kind != Kind::Err {
                        self.diags.push(Diag::new("E0118", arm.span, "this pattern does not match the type being matched"));
                    }
                    if seen_bools.contains(b) {
                        self.diags.push(Diag::new("E0121", arm.span, "this case is already handled"));
                    }
                    seen_bools.push(*b);
                }
                Pat::Int(v) => {
                    match kind {
                        Kind::Int { bits, signed } => {
                            let (lo, hi) = int_range(bits, signed);
                            if *v < lo || *v > hi {
                                let n = self.tname(t);
                                self.diags.push(Diag::new("E0115", arm.span, format!("number does not fit in {n}")));
                            }
                        }
                        Kind::Err => {}
                        _ => self.diags.push(Diag::new("E0118", arm.span, "this pattern does not match the type being matched")),
                    }
                    if seen_ints.contains(v) {
                        self.diags.push(Diag::new("E0121", arm.span, "this case is already handled"));
                    }
                    seen_ints.push(*v);
                }
                Pat::Variant { name, binds } => self.variant_pat(arm.span, kind, t, name, binds, &mut seen_variants),
            }
            let ends = self.block(&arm.body);
            all_end &= ends;
            self.pop_scope();
        }

        let exhaustive = wild
            || match kind {
                Kind::Enum(id) => {
                    let missing: Vec<String> = self.g.types.enums[id]
                        .variants
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| !seen_variants.contains(i))
                        .map(|(_, v)| v.name.clone())
                        .collect();
                    if !missing.is_empty() {
                        self.diags.push(Diag::new("E0120", span, format!("`match` misses: {}", missing.join(", "))));
                    }
                    true
                }
                Kind::Bool => {
                    if seen_bools.len() < 2 {
                        self.diags.push(Diag::new("E0120", span, "`match` must handle both true and false"));
                    }
                    true
                }
                Kind::Int { .. } => {
                    self.diags.push(Diag::new("E0120", span, "`match` on a number needs a `_:` arm"));
                    true
                }
                _ => true,
            };
        exhaustive && all_end && !arms.is_empty()
    }

    fn variant_pat(&mut self, span: Span, kind: Kind, t: Ty, name: &str, binds: &[(String, Span)], seen: &mut Vec<usize>) {
        let Kind::Enum(id) = kind else {
            if kind != Kind::Err {
                self.diags.push(Diag::new("E0118", span, "this pattern does not match the type being matched"));
            }
            return;
        };
        let def = self.g.types.enums[id].clone();
        let Some(idx) = def.variants.iter().position(|v| v.name == name) else {
            let n = self.tname(t);
            self.diags.push(Diag::new("E0118", span, format!("`{n}` has no variant `{name}`")));
            return;
        };
        if seen.contains(&idx) {
            self.diags.push(Diag::new("E0121", span, "this case is already handled"));
        }
        seen.push(idx);
        let fields = &def.variants[idx].fields;
        if !binds.is_empty() && binds.len() != fields.len() {
            self.diags.push(Diag::new("E0119", span, format!("`{name}` has {} fields, found {} names", fields.len(), binds.len())));
            for (b, sp) in binds {
                if b != "_" {
                    self.declare(b, ERR, false, *sp, *sp);
                    self.lookup(b).unwrap().read = true;
                }
            }
            return;
        }
        for ((b, sp), f) in binds.iter().zip(fields) {
            if b != "_" {
                self.declare(b, f.ty, false, *sp, *sp);
            }
        }
    }

    fn expr(&mut self, e: &Expr, hint: Option<Ty>) -> Ty {
        let t = self.expr_inner(e, hint);
        e.ty.set(t);
        t
    }

    fn expr_inner(&mut self, e: &Expr, hint: Option<Ty>) -> Ty {
        let num_hint = self.strip(hint);
        match &e.kind {
            ExprKind::Int(v) => {
                let t = match num_hint {
                    Some(h) if self.g.types.is_int(h) || self.g.types.is_float(h) => h,
                    _ => INT,
                };
                if let Kind::Int { bits, signed } = self.kind(t) {
                    let (lo, hi) = int_range(bits, signed);
                    if *v < lo || *v > hi {
                        let n = self.tname(t);
                        return self.err("E0115", e.span, format!("number does not fit in {n}"));
                    }
                }
                t
            }
            ExprKind::Float(_) => match num_hint {
                Some(h) if self.g.types.is_float(h) => h,
                _ => F64,
            },
            ExprKind::Char(_) => U8,
            ExprKind::Bool(_) => BOOL,
            ExprKind::Str(_) | ExprKind::Interp(_) => STRLIT,
            ExprKind::None => match hint.map(|h| self.kind(h)) {
                Some(Kind::Option(_)) => hint.unwrap(),
                Some(Kind::Err) => ERR,
                _ => self.err("E0123", e.span, "cannot tell which `?T` this `none` is; give the variable a type"),
            },
            ExprKind::RetVal => ERR,
            ExprKind::Var(name) => self.var(e, name),
            ExprKind::Unary { op, e: inner } => self.unary(e, *op, inner, num_hint),
            ExprKind::Binary { op, l, r } => self.binary(*op, l, r, num_hint),
            ExprKind::Call { name, name_span, args } => self.call(name, *name_span, args),
            ExprKind::StructLit { name, name_span, args } => self.struct_lit(name, *name_span, args),
            ExprKind::ArrayLit(items) => self.array_lit(e, items, num_hint),
            ExprKind::ArrayFill(item) => match num_hint.map(|h| self.kind(h)) {
                Some(Kind::Array(elem, _)) => {
                    self.check(item, elem);
                    num_hint.unwrap()
                }
                Some(Kind::Err) => {
                    self.expr(item, None);
                    ERR
                }
                _ => {
                    self.expr(item, None);
                    self.err("E0123", e.span, "`[x...]` needs a known array type; write `a [10]int = [0...]`")
                }
            },
            ExprKind::Field { e: base, name, name_span } => {
                let b = self.expr(base, None);
                self.field_type(b, name, *name_span)
            }
            ExprKind::Index { e: base, i } => {
                let b = self.expr(base, None);
                self.index_type(b, i, base.span)
            }
            ExprKind::Try(inner) => self.try_expr(e, inner),
            ExprKind::Else { e: inner, handler } => self.else_expr(e, inner, handler),
        }
    }

    fn var(&mut self, e: &Expr, name: &str) -> Ty {
        if let Some(v) = self.lookup(name) {
            v.read = true;
            return v.ty;
        }
        if let Some((_, t)) = self.g.consts.get(name) {
            return *t;
        }
        match self.g.names.get(name).copied() {
            Some(NameDef::Variant(eid, idx, ty)) => {
                if !self.g.types.enums[eid].variants[idx].fields.is_empty() {
                    return self.err("E0106", e.span, format!("`{name}` needs its fields: `{name}{{...}}`"));
                }
                ty
            }
            Some(NameDef::Type(_)) => self.err("E0101", e.span, format!("`{name}` is a type, not a value")),
            None if name == "_" => self.err("E0101", e.span, "`_` can only be assigned to, not read"),
            None => self.err("E0101", e.span, format!("unknown name `{name}`")),
        }
    }

    fn unary(&mut self, e: &Expr, op: UnOp, inner: &Expr, hint: Option<Ty>) -> Ty {
        match op {
            UnOp::Not => {
                self.check(inner, BOOL);
                BOOL
            }
            UnOp::Neg => {
                let t = self.expr(inner, hint);
                match self.kind(t) {
                    Kind::Int { signed: true, .. } | Kind::Float { .. } | Kind::Err => t,
                    _ => {
                        let n = self.tname(t);
                        self.err("E0104", e.span, format!("`-` needs a signed number, found {n}"))
                    }
                }
            }
            UnOp::BitNot => {
                let t = self.expr(inner, hint);
                if self.g.types.is_int(t) || t == ERR {
                    t
                } else {
                    let n = self.tname(t);
                    self.err("E0104", e.span, format!("`~` needs an integer, found {n}"))
                }
            }
        }
    }

    fn operands(&mut self, l: &Expr, r: &Expr, hint: Option<Ty>) -> (Ty, Ty) {
        if is_lit(l) && !is_lit(r) {
            let b = self.expr(r, hint);
            let a = self.expr(l, Some(b));
            (a, b)
        } else {
            let a = self.expr(l, hint);
            let b = self.expr(r, Some(a));
            (a, b)
        }
    }

    fn binary(&mut self, op: BinOp, l: &Expr, r: &Expr, hint: Option<Ty>) -> Ty {
        match op {
            BinOp::And | BinOp::Or => {
                self.check(l, BOOL);
                self.check(r, BOOL);
                BOOL
            }
            BinOp::Eq | BinOp::Ne => {
                let (a, b) = self.operands(l, r, None);
                if a != b {
                    self.mismatch(r.span, b, a);
                } else if a != ERR && !self.g.types.is_num(a) && a != BOOL {
                    let n = self.tname(a);
                    self.diags.push(Diag::new("E0104", l.span, format!("`{}` works on numbers and bool, not {n}; use `match`", op.text())));
                }
                BOOL
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let (a, b) = self.operands(l, r, None);
                if a != b {
                    self.mismatch(r.span, b, a);
                } else if a != ERR && !self.g.types.is_num(a) {
                    let n = self.tname(a);
                    self.diags.push(Diag::new("E0104", l.span, format!("`{}` needs numbers, found {n}", op.text())));
                }
                BOOL
            }
            _ => {
                let (a, b) = self.operands(l, r, hint);
                if a != b {
                    self.mismatch(r.span, b, a);
                    return ERR;
                }
                let int_only = !matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div);
                let ok = if int_only { self.g.types.is_int(a) } else { self.g.types.is_num(a) };
                if !ok && a != ERR {
                    let n = self.tname(a);
                    let want = if int_only { "integers" } else { "numbers" };
                    return self.err("E0104", l.span, format!("`{}` needs {want}, found {n}", op.text()));
                }
                a
            }
        }
    }

    fn print_arg(&mut self, a: &Expr) {
        match &a.kind {
            ExprKind::Str(_) => {
                a.ty.set(STRLIT);
            }
            ExprKind::Interp(parts) => {
                a.ty.set(STRLIT);
                for p in parts {
                    if let Part::Expr(e) = p {
                        let t = self.expr(e, None);
                        self.value_type(e.span, t);
                    }
                }
            }
            _ => {
                let t = self.expr(a, None);
                self.value_type(a.span, t);
            }
        }
    }

    fn call(&mut self, name: &str, span: Span, args: &[Expr]) -> Ty {
        if name == "print" {
            if args.len() != 1 {
                self.diags.push(Diag::new("E0106", span, format!("`print` takes 1 value, found {}", args.len())));
            }
            for a in args {
                self.print_arg(a);
            }
            return VOID;
        }
        if name == "len" {
            if args.len() != 1 {
                return self.err("E0106", span, format!("`len` takes 1 value, found {}", args.len()));
            }
            let t = self.expr(&args[0], None);
            return match self.kind(t) {
                Kind::Array(..) | Kind::Err => INT,
                _ => {
                    let n = self.tname(t);
                    self.err("E0104", args[0].span, format!("`len` needs an array, found {n}"))
                }
            };
        }
        if let Some(target) = prim(name) {
            if args.len() != 1 {
                return self.err("E0106", span, format!("`{name}(...)` converts 1 value, found {}", args.len()));
            }
            let hint = if is_lit(&args[0]) { Some(target) } else { None };
            let t = self.expr(&args[0], hint);
            let ok = t == ERR
                || (self.g.types.is_num(t) && self.g.types.is_num(target))
                || (t == BOOL && self.g.types.is_int(target))
                || (t == BOOL && target == BOOL);
            if !ok {
                let n = self.tname(t);
                return self.err("E0104", args[0].span, format!("cannot convert {n} to {name}"));
            }
            return target;
        }
        let sigs = self.sigs;
        let Some(sig) = sigs.get(name) else {
            for a in args {
                self.expr(a, None);
            }
            return self.err("E0101", span, format!("unknown function `{name}`"));
        };
        if sig.params.len() != args.len() {
            self.diags.push(Diag::new("E0106", span, format!("`{name}` takes {} values, found {}", sig.params.len(), args.len())));
        }
        for (i, a) in args.iter().enumerate() {
            match sig.params.get(i) {
                Some(want) => self.check(a, *want),
                None => {
                    self.expr(a, None);
                }
            }
        }
        sig.ret
    }

    fn struct_lit(&mut self, name: &str, span: Span, args: &[Expr]) -> Ty {
        let (fields, ty) = match self.g.names.get(name).copied() {
            Some(NameDef::Type(t)) => match self.kind(t) {
                Kind::Struct(id) => (self.g.types.structs[id].fields.clone(), t),
                _ => {
                    for a in args {
                        self.expr(a, None);
                    }
                    return self.err("E0101", span, format!("`{name}` is a variant type; build one of its variants"));
                }
            },
            Some(NameDef::Variant(eid, idx, t)) => (self.g.types.enums[eid].variants[idx].fields.clone(), t),
            None => {
                for a in args {
                    self.expr(a, None);
                }
                return self.err("E0112", span, format!("unknown type `{name}`"));
            }
        };
        if args.len() != fields.len() {
            self.diags.push(Diag::new("E0106", span, format!("`{name}` has {} fields, found {}", fields.len(), args.len())));
        }
        for (a, f) in args.iter().zip(&fields) {
            self.check(a, f.ty);
        }
        for a in args.iter().skip(fields.len()) {
            self.expr(a, None);
        }
        ty
    }

    fn array_lit(&mut self, e: &Expr, items: &[Expr], hint: Option<Ty>) -> Ty {
        if let Some(h) = hint {
            if let Kind::Array(elem, n) = self.kind(h) {
                if items.len() as u64 != n {
                    let name = self.tname(h);
                    self.diags.push(Diag::new("E0104", e.span, format!("`{name}` needs {n} items, found {}", items.len())));
                }
                for it in items {
                    self.check(it, elem);
                }
                return h;
            }
        }
        if items.is_empty() {
            return self.err("E0123", e.span, "an empty array has no size");
        }
        let first = self.expr(&items[0], None);
        let first = self.value_type(items[0].span, first);
        for it in &items[1..] {
            self.check(it, first);
        }
        if first == ERR {
            return ERR;
        }
        self.g.types.intern(Kind::Array(first, items.len() as u64))
    }

    fn try_expr(&mut self, e: &Expr, inner: &Expr) -> Ty {
        let t = self.expr(inner, None);
        match self.kind(t) {
            Kind::Option(p) => {
                if !self.is_main && !matches!(self.kind(self.ret), Kind::Option(_) | Kind::Err) {
                    return self.err("E0124", e.span, "`?` on `?T` needs a function returning `?U`; or handle it with `else`");
                }
                p
            }
            Kind::Result(ok, err) => {
                if !self.is_main {
                    match self.kind(self.ret) {
                        Kind::Result(_, my_err) if my_err == err => {}
                        Kind::Err => {}
                        _ => {
                            let n = self.tname(err);
                            return self.err("E0124", e.span, format!("`?` needs a function that can fail with {n}; or handle it with `else`"));
                        }
                    }
                }
                ok
            }
            Kind::Err => ERR,
            _ => {
                let n = self.tname(t);
                self.err("E0104", inner.span, format!("`?` needs `?T` or `T ! E`, found {n}"))
            }
        }
    }

    fn else_expr(&mut self, e: &Expr, inner: &Expr, handler: &ElseKind) -> Ty {
        let t = self.expr(inner, None);
        let (payload, err) = match self.kind(t) {
            Kind::Option(p) => (p, None),
            Kind::Result(ok, err) => (ok, Some(err)),
            Kind::Err => (ERR, None),
            _ => {
                let n = self.tname(t);
                self.err("E0104", inner.span, format!("`else` needs `?T` or `T ! E`, found {n}"));
                (ERR, None)
            }
        };
        match handler {
            ElseKind::Value(v) => {
                if payload == VOID {
                    self.diags.push(Diag::new("E0104", e.span, "there is no value to replace here; use `else { ... }`"));
                    self.expr(v, None);
                } else {
                    self.check(v, payload);
                }
            }
            ElseKind::Block(binder, b) => {
                self.scopes.push(HashMap::new());
                if let Some((n, sp)) = binder {
                    match err {
                        Some(et) => self.declare(n, et, false, *sp, *sp),
                        None => {
                            if t != ERR {
                                self.diags.push(Diag::new("E0138", *sp, "`?T` has no error value to name; write `else { ... }`"));
                            }
                            self.declare(n, ERR, false, *sp, *sp);
                            self.lookup(n).unwrap().read = true;
                        }
                    }
                }
                let ends = self.block(b);
                self.pop_scope();
                if !ends {
                    self.diags.push(Diag::new("E0117", b.end, "an `else` block must leave: end it with `ret`, `fail`, `break` or `continue`"));
                }
            }
        }
        payload
    }
}

/// Does a `break` inside this block leave *this* loop?
fn has_break(b: &Block) -> bool {
    b.stmts.iter().any(stmt_breaks)
}

fn expr_breaks(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Else { e, handler } => {
            expr_breaks(e)
                || match handler {
                    ElseKind::Block(_, b) => has_break(b),
                    ElseKind::Value(v) => expr_breaks(v),
                }
        }
        ExprKind::Try(e) | ExprKind::Field { e, .. } | ExprKind::Unary { e, .. } => expr_breaks(e),
        ExprKind::Index { e, i } => expr_breaks(e) || expr_breaks(i),
        ExprKind::Binary { l, r, .. } => expr_breaks(l) || expr_breaks(r),
        ExprKind::Call { args, .. } | ExprKind::StructLit { args, .. } | ExprKind::ArrayLit(args) => args.iter().any(expr_breaks),
        ExprKind::ArrayFill(e) => expr_breaks(e),
        _ => false,
    }
}

fn stmt_breaks(s: &Stmt) -> bool {
    match &s.kind {
        StmtKind::Break => true,
        StmtKind::If { cond, then, els } => {
            expr_breaks(cond)
                || has_break(then)
                || match els {
                    Some(Else::Block(b)) => has_break(b),
                    Some(Else::If(i)) => stmt_breaks(i),
                    None => false,
                }
        }
        StmtKind::Match { subject, arms } => expr_breaks(subject) || arms.iter().any(|a| has_break(&a.body)),
        StmtKind::Let { init, .. } => expr_breaks(init),
        StmtKind::Assign { value, target, .. } => expr_breaks(value) || expr_breaks(target),
        StmtKind::Expr(e) | StmtKind::Discard(e) | StmtKind::Fail(e) => expr_breaks(e),
        StmtKind::Ret(Some(e)) => expr_breaks(e),
        _ => false,
    }
}
