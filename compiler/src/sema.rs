//! Name resolution, type checking and the strictness rules.

use crate::ast::*;
use crate::diag::{Diag, Span};
use crate::modules::Unit;
use crate::types::*;
use std::collections::{HashMap, HashSet};

#[derive(Clone)]
pub struct FnSig {
    pub params: Vec<Ty>,
    /// Which parameters take the value over (`own`).
    pub own: Vec<bool>,
    pub ret: Ty,
    /// The C symbol, for `extern fn`.
    pub ext: Option<String>,
    pub variadic: bool,
    /// Has a `need`, which only a direct call can check.
    pub has_need: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum ConstVal {
    Int(i128),
    Float(f64),
    Bool(bool),
    /// Worked out by running code after checking: an index into `Checked::computed`.
    Computed(usize),
}

/// A `const` whose value needs a function call.
pub struct ComputedConst {
    pub name: String,
    /// The hidden function that returns it.
    pub full: String,
    pub ty: Ty,
    pub span: Span,
}

#[derive(Clone, Copy)]
pub enum NameDef {
    Type(Ty),
    Variant(usize, usize, Ty),
}

pub struct Checked {
    pub types: Types,
    pub sigs: HashMap<String, FnSig>,
    /// One checked copy of a template per set of type arguments.
    pub instances: Vec<Func>,
    pub computed: Vec<ComputedConst>,
}

struct Var {
    ty: Ty,
    /// Borrowed from elsewhere, so it can be read but never moved out.
    view: bool,
    /// A `&` parameter: writing to it is a use.
    by_ref: bool,
    mutable: bool,
    read: bool,
    written: bool,
    span: Span,
    decl: Span,
}

const BUILTINS: &[&str] = &[
    "print", "len", "copy", "alloc", "free", "cast", "sizeof", "int", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "bool",
];

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

struct Mod<'a> {
    prefix: String,
    imports: &'a HashMap<String, usize>,
    used: HashSet<String>,
    /// Each entry remembers whether it is `pub`.
    names: HashMap<String, (NameDef, bool)>,
    consts: HashMap<String, (ConstVal, Ty, bool)>,
    funcs: HashMap<String, (Option<&'a Func>, bool)>,
    gtypes: HashMap<String, (&'a TypeDecl, bool)>,
    /// A variant of a template type, and its position in that type.
    gvariants: HashMap<String, (&'a TypeDecl, usize, bool)>,
}

struct Global<'a> {
    computed: Vec<ComputedConst>,
    types: Types,
    mods: Vec<Mod<'a>>,
    cur: usize,
    /// Type parameters currently in force, inside a template instance.
    tenv: HashMap<String, Ty>,
    sigs: HashMap<String, FnSig>,
    struct_insts: HashMap<(usize, String, Vec<Ty>), Ty>,
    /// The type arguments behind each instance name: two types can print alike (`a.P` and `b.P`).
    fn_targs: HashMap<String, Vec<Ty>>,
    /// Instances made after layout; laid out together once the outermost one has all its fields.
    late: Vec<(Ty, usize, String)>,
    instance_of: HashMap<Ty, (usize, String, Vec<Ty>)>,
    spans: HashMap<Ty, Span>,
    layout_ready: bool,
    funcs_ready: bool,
    core: usize,
    too_many: bool,
    depth: u32,
    work: Vec<(&'a Func, usize, Vec<Ty>, String)>,
}

const MAX_INSTANCE_DEPTH: u32 = 32;
const MAX_INSTANCES: usize = 20_000;

fn private_msg(name: &str) -> String {
    format!("`{name}` is private to its module; mark it `pub` there")
}

fn type_list(types: &Types, args: &[Ty]) -> String {
    args.iter().map(|t| types.name(*t)).collect::<Vec<_>>().join(", ")
}

impl<'a> Global<'a> {
    fn import(&mut self, alias: &str, span: Span, diags: &mut Vec<Diag>) -> Option<usize> {
        let m = &mut self.mods[self.cur];
        match m.imports.get(alias) {
            Some(&i) => {
                m.used.insert(alias.to_string());
                Some(i)
            }
            None => {
                diags.push(Diag::new("E0146", span, format!("no imported module named `{alias}`; add `use {alias}`")));
                None
            }
        }
    }

    fn resolve(&mut self, t: &TypeExpr, diags: &mut Vec<Diag>) -> Ty {
        match &t.kind {
            TypeKind::Name { module, name, targs } => self.resolve_name(t.span, module.as_deref(), name, targs, diags),
            TypeKind::Pointer(inner) => {
                let p = self.resolve(inner, diags);
                if p == ERR {
                    return ERR;
                }
                self.types.intern(Kind::Pointer(p))
            }
            TypeKind::Ref(inner) => {
                let p = self.resolve(inner, diags);
                if p == ERR {
                    return ERR;
                }
                self.types.intern(Kind::Ref(p))
            }
            TypeKind::Slice(inner) => {
                let p = self.resolve(inner, diags);
                if p == ERR {
                    return ERR;
                }
                self.types.intern(Kind::Slice(p))
            }
            TypeKind::Array(size, elem) => {
                let n = match size {
                    Size::Lit(v) => *v,
                    Size::Name(name) => match self.mods[self.cur].consts.get(name) {
                        Some((ConstVal::Int(v), ty, _)) if *ty != ERR => *v,
                        Some((ConstVal::Computed(_), _, _)) => {
                            diags.push(Diag::new("E0126", t.span, format!("array size `{name}` comes from a function call, so it is not known while types are checked")));
                            return ERR;
                        }
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
            TypeKind::Fn { params, ret, effects } => {
                let own = params.iter().map(|p| p.0).collect();
                let params: Vec<Ty> = params.iter().map(|p| self.resolve(&p.1, diags)).collect();
                let ret = ret.as_ref().map(|r| self.resolve(r, diags)).unwrap_or(VOID);
                if ret == ERR || params.contains(&ERR) {
                    return ERR;
                }
                let effects = effects.iter().fold(0, |m, e| m | if e == "mem" { MEM } else { IO });
                self.types.fn_type(FnType { params, own, ret, effects })
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

    fn resolve_name(&mut self, span: Span, module: Option<&str>, name: &str, targs: &[TypeExpr], diags: &mut Vec<Diag>) -> Ty {
        if module.is_none() && targs.is_empty() {
            if let Some(t) = self.tenv.get(name) {
                return *t;
            }
            if let Some(p) = prim(name) {
                return p;
            }
            if name == "str" {
                return STR;
            }
        }
        let m = match module {
            Some(alias) => match self.import(alias, span, diags) {
                Some(m) => m,
                None => return ERR,
            },
            None => self.cur,
        };
        let local = m == self.cur;
        if let Some(&(d, public)) = self.mods[m].gtypes.get(name) {
            if !public && !local {
                diags.push(Diag::new("E0147", span, private_msg(name)));
                return ERR;
            }
            if targs.len() != d.tparams.len() {
                diags.push(Diag::new("E0144", span, format!("`{name}` takes {} type arguments, found {}", d.tparams.len(), targs.len())));
                return ERR;
            }
            let mut args = Vec::with_capacity(targs.len());
            for a in targs {
                let t = self.resolve(a, diags);
                if t == ERR {
                    return ERR;
                }
                args.push(t);
            }
            return self.instantiate_type(d, m, args, diags);
        }
        // A template from the always-available module, when nothing nearer has this name.
        if module.is_none() && !self.mods[m].names.contains_key(name) {
            if let Some(&(d, true)) = self.mods[self.core].gtypes.get(name) {
                let core = self.core;
                if targs.len() != d.tparams.len() {
                    diags.push(Diag::new("E0144", span, format!("`{name}` takes {} type arguments, found {}", d.tparams.len(), targs.len())));
                    return ERR;
                }
                let args: Vec<Ty> = targs.iter().map(|a| self.resolve(a, diags)).collect();
                if args.contains(&ERR) {
                    return ERR;
                }
                return self.instantiate_type(d, core, args, diags);
            }
        }
        match self.mods[m].names.get(name).copied() {
            Some((NameDef::Type(ty), public)) => {
                if !public && !local {
                    diags.push(Diag::new("E0147", span, private_msg(name)));
                    return ERR;
                }
                if !targs.is_empty() {
                    diags.push(Diag::new("E0144", span, format!("`{name}` takes no type arguments")));
                    return ERR;
                }
                ty
            }
            Some((NameDef::Variant(..), _)) => {
                diags.push(Diag::new("E0112", span, format!("`{name}` is a variant, not a type")));
                ERR
            }
            None => {
                // The always-available module, when nothing nearer has this name.
                let core = self.core;
                if module.is_none() && m != core {
                    if let Some((NameDef::Type(ty), true)) = self.mods[core].names.get(name).copied() {
                        return ty;
                    }
                }
                diags.push(Diag::new("E0112", span, format!("unknown type `{name}`")));
                ERR
            }
        }
    }

    /// Hooks up a template's `drop` for one instance of its type.
    fn attach_generic_drop(&mut self, ty: Ty, m: usize, type_name: &str, targs: Vec<Ty>, diags: &mut Vec<Diag>) {
        let Some(&(Some(f), _)) = self.mods[m].funcs.get("drop") else { return };
        let matches_type = f.params.first().is_some_and(|p| match &p.ty.kind {
            TypeKind::Ref(inner) => matches!(&inner.kind, TypeKind::Name { name, .. } if name == type_name),
            _ => false,
        });
        if matches_type && f.tparams.len() == targs.len() {
            let full = self.instantiate_fn(f, m, targs, diags);
            self.types.user_drop.insert(ty, full);
        }
    }

    fn instantiate_type(&mut self, d: &'a TypeDecl, m: usize, targs: Vec<Ty>, diags: &mut Vec<Diag>) -> Ty {
        let key = (m, d.name.clone(), targs.clone());
        if let Some(t) = self.struct_insts.get(&key) {
            return *t;
        }
        if self.depth >= MAX_INSTANCE_DEPTH {
            diags.push(Diag::new("E0145", d.span, format!("`{}` keeps instantiating itself with new types", d.name)));
            return ERR;
        }
        let name = format!("{}[{}]", d.name, type_list(&self.types, &targs));
        let ty = match &d.body {
            TypeBody::Struct(_) => {
                self.types.structs.push(StructDef { name, fields: Vec::new(), size: 0, align: 1, module: m, state: 0 });
                self.types.intern(Kind::Struct(self.types.structs.len() - 1))
            }
            TypeBody::Variants(vars) => {
                let variants = vars.iter().map(|v| Variant { name: v.name.clone(), fields: Vec::new() }).collect();
                self.types.enums.push(EnumDef { name, variants, size: TAG_SIZE, module: m, state: 0 });
                self.types.intern(Kind::Enum(self.types.enums.len() - 1))
            }
        };
        self.struct_insts.insert(key, ty);
        self.instance_of.insert(ty, (m, d.name.clone(), targs.clone()));
        self.spans.insert(ty, d.span);

        let tenv: HashMap<String, Ty> = d.tparams.iter().map(|p| p.0.clone()).zip(targs).collect();
        let saved_env = std::mem::replace(&mut self.tenv, tenv);
        let saved_cur = std::mem::replace(&mut self.cur, m);
        self.depth += 1;
        match (&d.body, self.types.kind(ty)) {
            (TypeBody::Struct(fields), Kind::Struct(id)) => {
                let resolved: Vec<Field> = fields.iter().map(|f| Field { name: f.name.clone(), ty: self.resolve(&f.ty, diags), offset: 0 }).collect();
                self.types.structs[id].fields = resolved;
            }
            (TypeBody::Variants(vars), Kind::Enum(id)) => {
                for (i, v) in vars.iter().enumerate() {
                    let resolved: Vec<Field> = v.fields.iter().map(|f| Field { name: f.name.clone(), ty: self.resolve(&f.ty, diags), offset: 0 }).collect();
                    self.types.enums[id].variants[i].fields = resolved;
                }
            }
            _ => {}
        }
        self.depth -= 1;
        self.tenv = saved_env;
        self.cur = saved_cur;
        if self.layout_ready || self.funcs_ready {
            self.late.push((ty, m, d.name.clone()));
        }
        if self.depth == 0 {
            let late = std::mem::take(&mut self.late);
            for (t, tm, type_name) in late {
                if self.layout_ready {
                    self.ensure_layout(t, diags);
                }
                if self.funcs_ready {
                    let args = self.instance_of[&t].2.clone();
                    self.attach_generic_drop(t, tm, &type_name, args, diags);
                }
            }
        }
        ty
    }

    /// Lays out `t` after everything it contains by value.
    fn ensure_layout(&mut self, t: Ty, diags: &mut Vec<Diag>) {
        let cycle = |g: &Global, t: Ty, diags: &mut Vec<Diag>| {
            let span = g.spans.get(&t).copied().unwrap_or_default();
            diags.push(Diag::new("E0127", span, format!("type `{}` contains itself, so it would be infinitely large; use a pointer", g.types.name(t))));
        };
        match self.types.kind(t) {
            Kind::Struct(id) => {
                match self.types.structs[id].state {
                    2 => return,
                    1 => return cycle(self, t, diags),
                    _ => {}
                }
                self.types.structs[id].state = 1;
                let fields = self.types.structs[id].fields.clone();
                for f in &fields {
                    self.ensure_layout(f.ty, diags);
                }
                let (placed, end, al) = self.place_fields(&fields, 0);
                let def = &mut self.types.structs[id];
                def.fields = placed;
                def.size = align_up(end, al);
                def.align = al;
                def.state = 2;
            }
            Kind::Enum(id) => {
                match self.types.enums[id].state {
                    2 => return,
                    1 => return cycle(self, t, diags),
                    _ => {}
                }
                self.types.enums[id].state = 1;
                let vars = self.types.enums[id].variants.clone();
                for v in &vars {
                    for f in &v.fields {
                        self.ensure_layout(f.ty, diags);
                    }
                }
                let mut size = TAG_SIZE;
                let mut built = Vec::new();
                for v in &vars {
                    let (placed, end, _) = self.place_fields(&v.fields, TAG_SIZE);
                    size = size.max(end);
                    built.push(Variant { name: v.name.clone(), fields: placed });
                }
                let def = &mut self.types.enums[id];
                def.variants = built;
                def.size = align_up(size, 8);
                def.state = 2;
            }
            Kind::Array(e, _) | Kind::Option(e) => self.ensure_layout(e, diags),
            Kind::Result(a, b) => {
                self.ensure_layout(a, diags);
                self.ensure_layout(b, diags);
            }
            _ => {}
        }
    }

    fn place_fields(&self, fields: &[Field], start: u64) -> (Vec<Field>, u64, u64) {
        let (mut off, mut al) = (start, 1u64);
        let mut out = Vec::with_capacity(fields.len());
        for f in fields {
            let (s, a) = self.types.layout(f.ty);
            off = align_up(off, a);
            out.push(Field { name: f.name.clone(), ty: f.ty, offset: off });
            off += s;
            al = al.max(a);
        }
        (out, off, al)
    }

    fn instantiate_fn(&mut self, f: &'a Func, m: usize, targs: Vec<Ty>, diags: &mut Vec<Diag>) -> String {
        let base = format!("{}{}[{}]", self.mods[m].prefix, f.name, type_list(&self.types, &targs));
        let mut full = base.clone();
        let mut n = 1;
        while let Some(have) = self.fn_targs.get(&full) {
            if *have == targs {
                return full;
            }
            n += 1;
            full = format!("{base}#{n}");
        }
        if self.work.len() >= MAX_INSTANCES || self.sigs.len() >= MAX_INSTANCES {
            if !std::mem::replace(&mut self.too_many, true) {
                diags.push(Diag::new("E0145", f.name_span, format!("`{}` keeps instantiating itself with new types", f.name)));
            }
            return full;
        }
        let tenv: HashMap<String, Ty> = f.tparams.iter().map(|p| p.0.clone()).zip(targs.iter().copied()).collect();
        let saved_env = std::mem::replace(&mut self.tenv, tenv);
        let saved_cur = std::mem::replace(&mut self.cur, m);
        let params = f.params.iter().map(|p| self.resolve(&p.ty, diags)).collect();
        let ret = f.ret.as_ref().map(|t| self.resolve(t, diags)).unwrap_or(VOID);
        self.tenv = saved_env;
        self.cur = saved_cur;
        let own = f.params.iter().map(|p| p.own).collect();
        let has_need = f.contracts.iter().any(|c| !c.sure);
        self.sigs.insert(full.clone(), FnSig { params, own, ret, ext: None, variadic: false, has_need });
        self.fn_targs.insert(full.clone(), targs.clone());
        self.work.push((f, m, targs, full.clone()));
        full
    }

    fn unify(&self, p: &TypeExpr, a: Ty, bind: &mut HashMap<String, Ty>, tparams: &[(String, Span)]) {
        if a == ERR {
            return;
        }
        match &p.kind {
            TypeKind::Name { module: None, name, targs } if targs.is_empty() && tparams.iter().any(|t| t.0 == *name) => {
                bind.entry(name.clone()).or_insert(a);
            }
            TypeKind::Name { targs, .. } if !targs.is_empty() => {
                if let Some((_, _, args)) = self.instance_of.get(&a) {
                    for (x, y) in targs.iter().zip(args.clone()) {
                        self.unify(x, y, bind, tparams);
                    }
                }
            }
            TypeKind::Pointer(x) => {
                if let Kind::Pointer(t) = self.types.kind(a) {
                    self.unify(x, t, bind, tparams);
                }
            }
            TypeKind::Ref(x) => {
                if let Kind::Ref(t) | Kind::Pointer(t) = self.types.kind(a) {
                    self.unify(x, t, bind, tparams);
                }
            }
            TypeKind::Slice(x) => {
                if let Kind::Slice(t) | Kind::Array(t, _) = self.types.kind(a) {
                    self.unify(x, t, bind, tparams);
                }
            }
            TypeKind::Array(_, x) => {
                if let Kind::Array(t, _) = self.types.kind(a) {
                    self.unify(x, t, bind, tparams);
                }
            }
            TypeKind::Option(x) => match self.types.kind(a) {
                Kind::Option(t) => self.unify(x, t, bind, tparams),
                _ => self.unify(x, a, bind, tparams),
            },
            TypeKind::Fn { params, ret, .. } => {
                if let Kind::Fn(id) = self.types.kind(a) {
                    let f = self.types.fns[id].clone();
                    for (x, y) in params.iter().zip(&f.params) {
                        self.unify(&x.1, *y, bind, tparams);
                    }
                    if let Some(r) = ret {
                        self.unify(r, f.ret, bind, tparams);
                    }
                }
            }
            TypeKind::Result(ok, err) => match self.types.kind(a) {
                Kind::Result(o, e) => {
                    if let Some(k) = ok {
                        self.unify(k, o, bind, tparams);
                    }
                    self.unify(err, e, bind, tparams);
                }
                _ => {
                    if let Some(k) = ok {
                        self.unify(k, a, bind, tparams);
                    }
                }
            },
            _ => {}
        }
    }

    fn owner_of(&self, t: Ty) -> Option<usize> {
        match self.types.kind(t) {
            Kind::Struct(id) => Some(self.types.structs[id].module),
            Kind::Enum(id) => Some(self.types.enums[id].module),
            Kind::Pointer(p) | Kind::Ref(p) => self.owner_of(p),
            _ => None,
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
        diags.push(Diag::new("E0139", span, "a `const` must be computable from literals and earlier constants; with a stated type it may call functions: `const x int = f()`"));
        None
    };
    match &e.kind {
        ExprKind::Int(v) => Some(ConstVal::Int(*v)),
        ExprKind::Float(v) => Some(ConstVal::Float(*v)),
        ExprKind::Char(c) => Some(ConstVal::Int(*c as i128)),
        ExprKind::Bool(b) => Some(ConstVal::Bool(*b)),
        ExprKind::Var(n) => match g.mods[g.cur].consts.get(n) {
            Some((ConstVal::Computed(_), _, _)) | None => bad(diags, e.span),
            Some((v, _, _)) => Some(*v),
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
        if g.mods[g.cur].consts.contains_key(&c.name) || BUILTINS.contains(&c.name.as_str()) {
            diags.push(Diag::new("E0102", c.span, format!("`{}` is already defined", c.name)));
            continue;
        }
        if c.computed {
            // Its type may be declared further down; `resolve_computed` fills it in.
            let id = g.computed.len();
            g.computed.push(ComputedConst { name: c.name.clone(), full: format!("{}const {}", g.mods[g.cur].prefix, c.name), ty: ERR, span: c.span });
            g.mods[g.cur].consts.insert(c.name.clone(), (ConstVal::Computed(id), ERR, c.public));
            continue;
        }
        let want = c.ty.as_ref().map(|t| g.resolve(t, diags));
        let v = match eval_const(g, &c.value, diags) {
            Some(v) => v,
            None => {
                // Already reported: uses of this name should not report again.
                g.mods[g.cur].consts.insert(c.name.clone(), (ConstVal::Int(0), ERR, c.public));
                continue;
            }
        };
        let (v, ty) = match (v, want) {
            (ConstVal::Int(n), None) => (ConstVal::Int(n), INT),
            (ConstVal::Float(n), None) => (ConstVal::Float(n), F64),
            (ConstVal::Bool(b), None) => (ConstVal::Bool(b), BOOL),
            (ConstVal::Computed(_), None) => unreachable!("eval_const never returns this"),
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
        g.mods[g.cur].consts.insert(c.name.clone(), (v, ty, c.public));
    }
}

/// Now that every type is known, gives the computed constants theirs.
fn resolve_computed(g: &mut Global, p: &Program, diags: &mut Vec<Diag>) {
    for c in p.consts.iter().filter(|c| c.computed) {
        let Some((ConstVal::Computed(id), _, public)) = g.mods[g.cur].consts.get(&c.name).copied() else { continue };
        let ty = g.resolve(c.ty.as_ref().expect("only typed constants are computed"), diags);
        if ty != ERR && !const_data(&g.types, ty) {
            diags.push(Diag::new("E0173", c.span, format!("a computed `const` cannot be {}: it must be plain data or `str`", g.types.name(ty))));
        }
        g.computed[id].ty = ty;
        g.mods[g.cur].consts.insert(c.name.clone(), (ConstVal::Computed(id), ty, public));
    }
}

/// Can the value be written into the program as bytes?
fn const_data(t: &Types, ty: Ty) -> bool {
    fn plain(t: &Types, ty: Ty) -> bool {
        match t.kind(ty) {
            Kind::Int { .. } | Kind::Float { .. } | Kind::Bool => true,
            Kind::Array(e, _) => plain(t, e),
            Kind::Struct(id) => t.structs[id].fields.iter().all(|f| plain(t, f.ty)) && !t.user_drop.contains_key(&ty),
            _ => false,
        }
    }
    ty == STR || plain(t, ty)
}

fn define_name(g: &mut Global, name: &str, def: NameDef, public: bool, span: Span, diags: &mut Vec<Diag>) -> bool {
    let m = &mut g.mods[g.cur];
    if m.names.contains_key(name) || m.gtypes.contains_key(name) || prim(name).is_some() {
        diags.push(Diag::new("E0110", span, format!("`{name}` is defined twice")));
        return false;
    }
    m.names.insert(name.to_string(), (def, public));
    true
}

/// Declares the names of a module's types, so they can refer to each other freely.
fn declare_types<'a>(g: &mut Global<'a>, p: &'a Program, diags: &mut Vec<Diag>) -> Vec<Option<Ty>> {
    let mut ids = Vec::new();
    let module = g.cur;
    for d in &p.types {
        check_case(&d.name, true, "type", d.span, diags);
        if !d.tparams.is_empty() {
            for (n, sp) in &d.tparams {
                check_case(n, true, "type parameter", *sp, diags);
            }
            let m = &mut g.mods[g.cur];
            if m.names.contains_key(&d.name) || m.gtypes.contains_key(&d.name) {
                diags.push(Diag::new("E0110", d.span, format!("`{}` is defined twice", d.name)));
            } else {
                m.gtypes.insert(d.name.clone(), (d, d.public));
                if let TypeBody::Variants(vars) = &d.body {
                    for (i, v) in vars.iter().enumerate() {
                        check_case(&v.name, true, "variant", v.span, diags);
                        let m = &mut g.mods[g.cur];
                        if m.names.contains_key(&v.name) || m.gtypes.contains_key(&v.name) || m.gvariants.contains_key(&v.name) {
                            diags.push(Diag::new("E0110", v.span, format!("`{}` is defined twice", v.name)));
                        } else {
                            m.gvariants.insert(v.name.clone(), (d, i, d.public));
                        }
                    }
                }
            }
            ids.push(None);
            continue;
        }
        let ty = match &d.body {
            TypeBody::Struct(_) => {
                g.types.structs.push(StructDef { name: d.name.clone(), fields: Vec::new(), size: 0, align: 1, module, state: 0 });
                g.types.intern(Kind::Struct(g.types.structs.len() - 1))
            }
            TypeBody::Variants(vars) => {
                let variants = vars.iter().map(|v| Variant { name: v.name.clone(), fields: Vec::new() }).collect();
                g.types.enums.push(EnumDef { name: d.name.clone(), variants, size: TAG_SIZE, module, state: 0 });
                g.types.intern(Kind::Enum(g.types.enums.len() - 1))
            }
        };
        g.spans.insert(ty, d.span);
        if !define_name(g, &d.name, NameDef::Type(ty), d.public, d.span, diags) {
            ids.push(None);
            continue;
        }
        if let (TypeBody::Variants(vars), Kind::Enum(eid)) = (&d.body, g.types.kind(ty)) {
            for (i, v) in vars.iter().enumerate() {
                check_case(&v.name, true, "variant", v.span, diags);
                define_name(g, &v.name, NameDef::Variant(eid, i, ty), d.public, v.span, diags);
            }
        }
        ids.push(Some(ty));
    }
    ids
}

fn resolve_field_list(g: &mut Global, fields: &[FieldDecl], diags: &mut Vec<Diag>) -> Vec<Field> {
    let mut out: Vec<Field> = Vec::new();
    for f in fields {
        check_case(&f.name, false, "field", f.span, diags);
        if matches!(f.ty.kind, TypeKind::Ref(_)) {
            diags.push(Diag::new("E0157", f.span, "`&` is only for parameters; a struct cannot hold one"));
        }
        if out.iter().any(|x| x.name == f.name) {
            diags.push(Diag::new("E0137", f.span, format!("field `{}` appears twice", f.name)));
        }
        let ty = g.resolve(&f.ty, diags);
        out.push(Field { name: f.name.clone(), ty, offset: 0 });
    }
    out
}

fn fill_types(g: &mut Global, p: &Program, ids: &[Option<Ty>], diags: &mut Vec<Diag>) {
    for (d, id) in p.types.iter().zip(ids) {
        let Some(ty) = *id else { continue };
        match (&d.body, g.types.kind(ty)) {
            (TypeBody::Struct(fields), Kind::Struct(sid)) => {
                g.types.structs[sid].fields = resolve_field_list(g, fields, diags);
            }
            (TypeBody::Variants(vars), Kind::Enum(eid)) => {
                for (i, v) in vars.iter().enumerate() {
                    g.types.enums[eid].variants[i].fields = resolve_field_list(g, &v.fields, diags);
                }
            }
            _ => {}
        }
    }
}

fn is_c_type(types: &Types, t: Ty) -> bool {
    t == ERR || !types.is_aggregate(t)
}

fn collect_funcs<'a>(g: &mut Global<'a>, p: &'a Program, diags: &mut Vec<Diag>) {
    let prefix = g.mods[g.cur].prefix.clone();
    for f in &p.funcs {
        check_case(&f.name, false, "function", f.name_span, diags);
        if BUILTINS.contains(&f.name.as_str()) {
            diags.push(Diag::new("E0102", f.name_span, format!("`{}` is a built-in name", f.name)));
            continue;
        }
        let taken = g.mods[g.cur].funcs.contains_key(&f.name) || g.mods[g.cur].consts.contains_key(&f.name);
        if taken {
            diags.push(Diag::new("E0110", f.name_span, format!("function `{}` is defined twice", f.name)));
            continue;
        }
        if !f.tparams.is_empty() {
            for (n, sp) in &f.tparams {
                check_case(n, true, "type parameter", *sp, diags);
            }
            g.mods[g.cur].funcs.insert(f.name.clone(), (Some(f), f.public));
            continue;
        }
        let params: Vec<Ty> = f.params.iter().map(|x| g.resolve(&x.ty, diags)).collect();
        let ret = f.ret.as_ref().map(|t| g.resolve(t, diags)).unwrap_or(VOID);
        if g.types.is_ref(ret) {
            diags.push(Diag::new("E0157", f.name_span, "a function cannot return a `&`; return the value"));
        }
        if f.name == "drop" {
            match params.first().map(|t| g.types.kind(*t)) {
                Some(Kind::Ref(t)) if params.len() == 1 && matches!(g.types.kind(t), Kind::Struct(_) | Kind::Enum(_)) => {
                    g.types.user_drop.insert(t, format!("{prefix}drop"));
                }
                _ => diags.push(Diag::new("E0156", f.name_span, "`drop` takes exactly one `&` parameter of a struct or variant type")),
            }
        }
        if !f.claw {
            let bad = f.params.iter().zip(&params).find(|(_, t)| g.types.is_ptr(**t)).map(|(x, _)| x.span);
            if let Some(span) = bad.or(if g.types.is_ptr(ret) { Some(f.name_span) } else { None }) {
                diags.push(Diag::new("E0140", span, "pointers only exist inside `claw fn`").suggest(Span { start: f.name_span.start.saturating_sub(3), end: f.name_span.start.saturating_sub(3), ..f.name_span }, "claw ", "make it a `claw fn`"));
            }
        }
        g.mods[g.cur].funcs.insert(f.name.clone(), (None, f.public));
        let own = f.params.iter().map(|p| p.own).collect();
        let has_need = f.contracts.iter().any(|c| !c.sure);
        g.sigs.insert(format!("{prefix}{}", f.name), FnSig { params, own, ret, ext: None, variadic: false, has_need });
    }
    for x in &p.externs {
        if g.mods[g.cur].funcs.contains_key(&x.name) || BUILTINS.contains(&x.name.as_str()) {
            diags.push(Diag::new("E0110", x.span, format!("function `{}` is defined twice", x.name)));
            continue;
        }
        let params: Vec<Ty> = x.params.iter().map(|a| g.resolve(&a.ty, diags)).collect();
        let ret = x.ret.as_ref().map(|t| g.resolve(t, diags)).unwrap_or(VOID);
        for (a, t) in x.params.iter().zip(&params) {
            if !is_c_type(&g.types, *t) {
                diags.push(Diag::new("E0149", a.span, "`extern fn` takes only numbers, bool and pointers; pass a pointer to the struct"));
            }
        }
        if !is_c_type(&g.types, ret) {
            diags.push(Diag::new("E0149", x.span, "`extern fn` returns only numbers, bool and pointers"));
        }
        g.mods[g.cur].funcs.insert(x.name.clone(), (None, x.public));
        let symbol = x.symbol.clone().unwrap_or_else(|| x.name.clone());
        let own = vec![false; params.len()];
        g.sigs.insert(format!("{prefix}{}", x.name), FnSig { params, own, ret, ext: Some(symbol), variadic: x.variadic, has_need: false });
    }
}

fn check_body<'a>(g: &mut Global<'a>, diags: &mut Vec<Diag>, f: &Func, module: usize, full: &str, is_main: bool) {
    let Some(sig) = g.sigs.get(full).cloned() else { return };
    g.cur = module;
    let mut cx = Ctx {
        g,
        func: f,
        scopes: vec![HashMap::new()],
        diags,
        ret: sig.ret,
        loops: 0,
        is_main,
        locks: Vec::new(),
        scope_locks: Vec::new(),
        in_text: false,
        in_defer: false,
        moved: HashMap::new(),
        flows: Vec::new(),
        in_contract: false,
        sure_clause: false,
        in_old: false,
    };
    for (x, ty) in f.params.iter().zip(sig.params) {
        if x.name == "_" {
            continue;
        }
        let (t, view, mutable) = match cx.kind(ty) {
            Kind::Ref(inner) => (inner, true, true),
            _ => (ty, !x.own, false),
        };
        cx.declare(&x.name, t, mutable, x.span, x.span);
        if let Some(v) = cx.lookup(&x.name) {
            v.view = view;
            v.by_ref = mutable;
        }
    }
    if !f.contracts.is_empty() && (is_main || f.name == "drop") {
        cx.diags.push(Diag::new("E0166", f.name_span, format!("`{}` is called by the compiler, so it cannot have contracts", f.name)));
    }
    cx.in_contract = true;
    for c in &f.contracts {
        cx.sure_clause = c.sure;
        cx.check(&c.e, BOOL);
    }
    cx.in_contract = false;
    cx.sure_clause = false;
    let ends = cx.block(&f.body);
    cx.pop_scope();
    if !ends && sig.ret != VOID && sig.ret != ERR && !matches!(cx.kind(sig.ret), Kind::Result(VOID, _)) {
        let msg = format!("`{}` can reach its end without `ret`", f.name);
        cx.diags.push(Diag::new("E0107", f.body.end, msg));
    }
}

/// `tests_only`: there is no program to start, so `main` is optional.
pub fn check(units: &[Unit], core: usize, tests_only: bool, diags: &mut Vec<Diag>) -> Checked {
    let mut g = Global {
        types: Types::new(),
        mods: units
            .iter()
            .map(|u| Mod {
                prefix: u.prefix.clone(),
                imports: &u.imports,
                used: HashSet::new(),
                names: HashMap::new(),
                consts: HashMap::new(),
                funcs: HashMap::new(),
                gtypes: HashMap::new(), gvariants: HashMap::new(),
            })
            .collect(),
        cur: 0,
        tenv: HashMap::new(),
        sigs: HashMap::new(),
        struct_insts: HashMap::new(),
        fn_targs: HashMap::new(),
        late: Vec::new(),
        instance_of: HashMap::new(),
        spans: HashMap::new(),
        layout_ready: false,
        funcs_ready: false,
        core,
        too_many: false,
        depth: 0,
        work: Vec::new(),
        computed: Vec::new(),
    };

    for (i, u) in units.iter().enumerate() {
        g.cur = i;
        collect_consts(&mut g, &u.program, diags);
    }
    let mut ids = Vec::new();
    for (i, u) in units.iter().enumerate() {
        g.cur = i;
        ids.push(declare_types(&mut g, &u.program, diags));
    }
    for (i, u) in units.iter().enumerate() {
        g.cur = i;
        fill_types(&mut g, &u.program, &ids[i], diags);
    }
    // By type id, not hash order: which type a cycle is reported on must not vary from run to run.
    let mut made: Vec<Ty> = g.instance_of.keys().copied().collect();
    made.sort_by_key(|t| t.0);
    let every: Vec<Ty> = made.into_iter().chain(ids.iter().flatten().flatten().copied()).collect();
    for t in every {
        g.ensure_layout(t, diags);
    }
    g.layout_ready = true;

    for (i, u) in units.iter().enumerate() {
        g.cur = i;
        resolve_computed(&mut g, &u.program, diags);
    }
    for (i, u) in units.iter().enumerate() {
        g.cur = i;
        collect_funcs(&mut g, &u.program, diags);
    }

    g.funcs_ready = true;
    let mut existing: Vec<(Ty, (usize, String, Vec<Ty>))> = g.instance_of.iter().map(|(t, i)| (*t, i.clone())).collect();
    // The order instances are made in decides the order of the program's symbols.
    existing.sort_by_key(|x| x.0 .0);
    for (ty, (m, name, targs)) in existing {
        g.attach_generic_drop(ty, m, &name, targs, diags);
    }

    let root = units.len() - 1;
    match units[root].program.funcs.iter().find(|f| f.name == "main") {
        None if tests_only => {}
        None => {
            let at = Span { start: 0, end: 0, line: 1, col: 1, file: 0 };
            diags.push(Diag::new("E0109", at, "no `fn main()`; every program needs one"));
        }
        Some(f) => {
            if !f.params.is_empty() || f.ret.is_some() || !f.tparams.is_empty() {
                diags.push(Diag::new("E0109", f.name_span, "`main` takes no parameters and returns nothing"));
            }
        }
    }

    let mut titles: HashSet<&str> = HashSet::new();
    for f in &units[root].program.funcs {
        if let Some(t) = &f.test {
            if !titles.insert(t) {
                diags.push(Diag::new("E0167", f.name_span, format!("there is already a test named \"{t}\"")));
            }
        }
    }

    for (i, u) in units.iter().enumerate() {
        for f in u.program.funcs.iter().filter(|f| f.tparams.is_empty()) {
            let full = format!("{}{}", u.prefix, f.name);
            check_body(&mut g, diags, f, i, &full, (i == root && f.name == "main") || f.test.is_some());
        }
    }

    // Each template instance is checked as ordinary code with its types filled in.
    let mut instances = Vec::new();
    while let Some((f, m, targs, full)) = g.work.pop() {
        let mut inst = f.clone();
        inst.name = full.clone();
        g.tenv = f.tparams.iter().map(|p| p.0.clone()).zip(targs).collect();
        check_body(&mut g, diags, &inst, m, &full, false);
        g.tenv.clear();
        instances.push(inst);
    }

    for (i, u) in units.iter().enumerate() {
        for (alias, span) in &u.import_spans {
            if !g.mods[i].used.contains(alias) {
                diags.push(Diag::new("E0148", *span, format!("module `{alias}` is imported but not used")));
            }
        }
    }

    Checked { types: g.types, sigs: g.sigs, instances, computed: g.computed }
}

struct Ctx<'a, 'b> {
    g: &'b mut Global<'a>,
    func: &'b Func,
    scopes: Vec<HashMap<String, Var>>,
    diags: &'b mut Vec<Diag>,
    ret: Ty,
    loops: u32,
    is_main: bool,
    /// Arrays being walked by an enclosing `for ... in`.
    locks: Vec<String>,
    /// Locks held until a block ends: (scope depth, how many locks there were then).
    scope_locks: Vec<(usize, usize)>,
    /// Inside `{...}` of a string.
    in_text: bool,
    in_defer: bool,
    /// Variables whose value has left, and where it left.
    moved: HashMap<String, Span>,
    flows: Vec<LoopFlow>,
    /// Inside a `need`, a `sure` or a `sure` statement: nothing may change.
    in_contract: bool,
    /// Inside a `sure` clause of a signature, where `ret` and `old` mean something.
    sure_clause: bool,
    in_old: bool,
}

/// What `moved` looked like at each jump out of a loop body.
#[derive(Default)]
struct LoopFlow {
    breaks: Vec<HashMap<String, Span>>,
    conts: Vec<HashMap<String, Span>>,
}

/// One step of a path like `a.b[i]`, for spotting overlapping uses.
#[derive(Clone, PartialEq)]
enum Comp {
    Field(String),
    Index,
}

fn path_of(e: &Expr) -> Option<(String, Vec<Comp>)> {
    match &e.kind {
        ExprKind::Var(n) => Some((n.clone(), Vec::new())),
        ExprKind::Field { e: b, name, .. } => {
            let (r, mut c) = path_of(b)?;
            c.push(Comp::Field(name.clone()));
            Some((r, c))
        }
        ExprKind::Index { e: b, .. } | ExprKind::Slice { e: b, .. } => {
            let (r, mut c) = path_of(b)?;
            c.push(Comp::Index);
            Some((r, c))
        }
        _ => None,
    }
}

/// Could these two paths touch the same memory?
fn overlap(a: &(String, Vec<Comp>), b: &(String, Vec<Comp>)) -> bool {
    a.0 == b.0 && a.1.iter().zip(&b.1).all(|(x, y)| x == y)
}

/// Visits an expression and everything nested in it.
pub fn walk<'e>(e: &'e Expr, f: &mut dyn FnMut(&'e Expr)) {
    f(e);
    match &e.kind {
        ExprKind::Old(x) | ExprKind::Unary { e: x, .. } | ExprKind::AddrOf(x) | ExprKind::Deref(x) | ExprKind::Field { e: x, .. } | ExprKind::Try(x) | ExprKind::ArrayFill(x) => walk(x, f),
        ExprKind::Binary { l, r, .. } => {
            walk(l, f);
            walk(r, f);
        }
        ExprKind::Index { e: x, i } => {
            walk(x, f);
            walk(i, f);
        }
        ExprKind::Slice { e: x, lo, hi } => {
            walk(x, f);
            for b in [lo, hi].into_iter().flatten() {
                walk(b, f);
            }
        }
        ExprKind::Call { args, .. } | ExprKind::StructLit { args, .. } | ExprKind::ArrayLit(args) => {
            for a in args {
                walk(a, f);
            }
        }
        ExprKind::CallValue { f: callee, args } => {
            walk(callee, f);
            for a in args {
                walk(a, f);
            }
        }
        ExprKind::Interp(parts) => {
            for p in parts {
                if let Part::Expr(x) = p {
                    walk(x, f);
                }
            }
        }
        ExprKind::Else { e: x, handler } => {
            walk(x, f);
            if let ElseKind::Value(v) = handler {
                walk(v, f);
            }
        }
        _ => {}
    }
}

fn mutable_paths(e: &Expr, out: &mut Vec<(String, Vec<Comp>)>) {
    walk(e, &mut |x| {
        if let ExprKind::AddrOf(inner) = &x.kind {
            out.extend(path_of(inner));
        }
    });
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

/// Literals whose type depends on what is expected of them.
fn hint_dep(e: &Expr) -> bool {
    is_lit(e) || matches!(e.kind, ExprKind::None | ExprKind::ArrayLit(_) | ExprKind::ArrayFill(_) | ExprKind::Str(_))
}

/// Does this parameter type still contain a type parameter nobody has decided?
fn mentions_unbound(p: &TypeExpr, bind: &HashMap<String, Ty>, tparams: &[(String, Span)]) -> bool {
    let rec = |x: &TypeExpr| mentions_unbound(x, bind, tparams);
    match &p.kind {
        TypeKind::Name { name, targs, .. } => (tparams.iter().any(|t| t.0 == *name) && !bind.contains_key(name)) || targs.iter().any(rec),
        TypeKind::Pointer(x) | TypeKind::Ref(x) | TypeKind::Slice(x) | TypeKind::Array(_, x) | TypeKind::Option(x) => rec(x),
        TypeKind::Result(ok, err) => ok.as_deref().is_some_and(rec) || rec(err),
        TypeKind::Fn { params, ret, .. } => params.iter().any(|x| rec(&x.1)) || ret.as_deref().is_some_and(rec),
    }
}

enum Root {
    Var(String),
    /// The write lands behind a pointer, so the variable itself does not change.
    Through,
    Other,
}

fn root_of(e: &Expr, types: &Types) -> Root {
    match &e.kind {
        ExprKind::Var(n) => Root::Var(n.clone()),
        ExprKind::Deref(_) => Root::Through,
        ExprKind::Field { e: b, .. } | ExprKind::Index { e: b, .. } => {
            if types.is_ptr(b.ty.get()) {
                Root::Through
            } else {
                root_of(b, types)
            }
        }
        _ => Root::Other,
    }
}

fn root_name(e: &Expr) -> Option<&str> {
    match &e.kind {
        ExprKind::Var(n) => Some(n),
        ExprKind::Field { e, .. } | ExprKind::Index { e, .. } => root_name(e),
        _ => None,
    }
}

impl<'a, 'b> Ctx<'a, 'b> {
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
            || self.g.mods[self.g.cur].funcs.contains_key(name)
            || self.g.mods[self.g.cur].consts.contains_key(name);
        if taken {
            self.diags.push(Diag::new("E0102", span, format!("`{name}` is already defined; names cannot be reused or hidden")));
            if self.lookup(name).is_none() {
                let v = Var { ty, view: false, by_ref: false, mutable, read: true, written: true, span, decl };
                self.scopes.last_mut().unwrap().insert(name.to_string(), v);
            }
            return;
        }
        self.moved.remove(name);
        let v = Var { ty, view: false, by_ref: false, mutable, read: false, written: false, span, decl };
        self.scopes.last_mut().unwrap().insert(name.to_string(), v);
    }

    fn pop_scope(&mut self) {
        while self.scope_locks.last().is_some_and(|l| l.0 == self.scopes.len()) {
            let (_, count) = self.scope_locks.pop().unwrap();
            self.locks.truncate(count - 1);
        }
        let scope = self.scopes.pop().unwrap();
        for (name, v) in scope {
            self.moved.remove(&name);
            if !v.read && !(v.by_ref && v.written) {
                let d = Diag::new("E0105", v.span, format!("`{name}` is never read")).suggest(v.span, "_", format!("rename `{name}` to `_`"));
                self.diags.push(d);
            } else if v.mutable && !v.written && !v.by_ref {
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
        self.accept(e, got, want);
    }

    fn accept(&mut self, e: &Expr, got: Ty, want: Ty) {
        if let Kind::Ref(inner) = self.kind(want) {
            if self.kind(got) == Kind::Pointer(inner) && matches!(e.kind, ExprKind::AddrOf(_)) {
                return;
            }
            if got == inner {
                let d = Diag::new("E0157", e.span, "this parameter changes the value, so write `&` here").fix(e.span.before(), "&", "add `&`");
                self.diags.push(d);
                if let ExprKind::Var(n) = &e.kind {
                    if let Some(v) = self.lookup(n) {
                        v.written = true;
                    }
                }
                return;
            }
        }
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
            Kind::StrLit => self.err("E0900", span, "this string form only works directly inside `print`"),
            Kind::Ref(_) => self.err("E0157", span, "`&x` only works as an argument to a `&` parameter"),
            _ => ty,
        }
    }

    fn printable(&mut self, span: Span, ty: Ty) {
        self.value_type(span, ty);
        if matches!(self.kind(ty), Kind::Fn(_)) {
            self.err("E0104", span, "a function has nothing to print");
        }
        if self.g.types.contains_itself(ty) {
            let n = self.tname(ty);
            self.err("E0179", span, format!("`{n}` contains itself, so it cannot be printed whole; write a function that walks it"));
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
                let b = self.place_base(base);
                self.field_type(b, name, *name_span)
            }
            ExprKind::Index { e: base, i } => {
                let b = self.place_base(base);
                if b == STR {
                    self.expr(i, Some(INT));
                    return self.err("E0131", e.span, "a `str` cannot be changed in place");
                }
                self.index_type(b, i, base.span)
            }
            ExprKind::Deref(inner) => self.deref_type(e, inner),
            _ => {
                self.expr(e, None);
                self.err("E0131", e.span, "this cannot be assigned to")
            }
        };
        e.ty.set(t);
        t
    }

    /// The base of a field or index on the left of `=`. Going through a pointer reads it.
    fn place_base(&mut self, base: &Expr) -> Ty {
        let t = self.place(base);
        if self.g.types.is_ptr(t) {
            if let Some(n) = root_name(base) {
                let n = n.to_string();
                if let Some(v) = self.lookup(&n) {
                    v.read = true;
                }
            }
        }
        t
    }

    fn deref_type(&mut self, e: &Expr, inner: &Expr) -> Ty {
        let t = self.expr(inner, None);
        match self.kind(t) {
            Kind::Pointer(p) => p,
            Kind::Err => ERR,
            _ => {
                let n = self.tname(t);
                self.err("E0104", e.span, format!("`*` needs a pointer, found {n}"))
            }
        }
    }

    fn field_type(&mut self, base: Ty, name: &str, span: Span) -> Ty {
        // `p.x` reads through a pointer to a struct.
        let base = match self.kind(base) {
            Kind::Pointer(p) => p,
            _ => base,
        };
        match self.kind(base) {
            Kind::Slice(_) | Kind::Str if self.func.claw => match (name, self.kind(base)) {
                ("ptr", Kind::Slice(e)) => self.g.types.intern(Kind::Pointer(e)),
                ("ptr", _) => self.g.types.intern(Kind::Pointer(U8)),
                ("len", _) | ("cap", _) => INT,
                _ => self.err("E0129", span, format!("{} has only `ptr`, `len` and `cap`", self.g.types.name(base))),
            },
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
            Kind::Pointer(elem) | Kind::Slice(elem) => elem,
            Kind::Str => U8,
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
                        let mut want = self.g.resolve(t, self.diags);
                        if !self.func.claw && self.g.types.is_ptr(want) {
                            self.diags.push(Diag::new("E0140", t.span, "pointers only exist inside `claw fn`"));
                            want = ERR;
                        }
                        self.check(init, want);
                        want
                    }
                    None => {
                        let got = self.expr(init, None);
                        self.value_type(init.span, got)
                    }
                };
                let held = if *mutable || init.coerce.get() != Coerce::No { None } else { self.view_of(init) };
                match &held {
                    Some(_) => init.coerce.set(Coerce::View),
                    None => self.consume(init),
                }
                let decl = if *mutable {
                    Span { start: s.span.start, end: name_span.start, line: s.span.line, col: s.span.col, file: s.span.file }
                } else {
                    Span { start: s.span.start, end: s.span.start, line: s.span.line, col: s.span.col, file: s.span.file }
                };
                self.declare(name, final_ty, *mutable, *name_span, decl);
                if let Some(root) = held {
                    if let Some(v) = self.lookup(name) {
                        v.view = true;
                    }
                    self.locks.push(root);
                    self.scope_locks.push((self.scopes.len(), self.locks.len()));
                }
                false
            }
            StmtKind::Assign { target, op, value } => {
                let ty = self.place(target);
                match root_of(target, &self.g.types) {
                    Root::Var(root) => {
                        let info = self.lookup(&root).map(|v| (v.mutable, v.decl));
                        if let Some((mutable, decl)) = info {
                            if !mutable {
                                let d = Diag::new("E0103", target.span, format!("`{root}` cannot change")).fix(decl, "mut ", format!("declare `mut {root}`"));
                                self.diags.push(d);
                            }
                            self.lookup(&root).unwrap().written = true;
                        }
                        if self.locks.contains(&root) {
                            self.diags.push(Diag::new("E0122", target.span, format!("`{root}` cannot change while a `for` or a view is reading it")));
                        }
                    }
                    Root::Through => {}
                    Root::Other => {
                        if ty != ERR {
                            self.diags.push(Diag::new("E0131", target.span, "this cannot be assigned to"));
                        }
                    }
                }
                if !matches!(target.kind, ExprKind::Var(_)) {
                    if let Some(root) = root_name(target) {
                        let root = root.to_string();
                        self.check_live(&root, target.span);
                    }
                }
                if !self.func.claw {
                    // `a[i] = g(&a)`: g could move the very memory being assigned to.
                    let mut muts = Vec::new();
                    mutable_paths(value, &mut muts);
                    if let Some(p) = path_of(target) {
                        if muts.iter().any(|m| overlap(m, &p)) {
                            self.diags.push(Diag::new("E0158", value.span, format!("`{}` is being assigned and changed at once", p.0)));
                        }
                    }
                }
                if let Some(o) = op {
                    if self.g.types.is_ptr(ty) {
                        if !matches!(o, BinOp::Add | BinOp::Sub) {
                            self.diags.push(Diag::new("E0104", target.span, format!("`{}=` does not work on pointers", o.text())));
                        }
                        self.check(value, INT);
                        return false;
                    }
                    if !self.g.types.is_num(ty) && ty != ERR {
                        let n = self.tname(ty);
                        self.diags.push(Diag::new("E0104", target.span, format!("`{}=` needs a number, found {n}", o.text())));
                    } else if self.g.types.is_float(ty) && !matches!(o, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div) {
                        self.diags.push(Diag::new("E0104", target.span, format!("`{}=` does not work on floats", o.text())));
                    }
                }
                self.check(value, ty);
                self.consume(value);
                if let ExprKind::Var(n) = &target.kind {
                    self.moved.remove(n);
                }
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
                    ExprKind::Call { .. } | ExprKind::CallValue { .. } | ExprKind::Try(_) | ExprKind::Else { .. } => {
                        if got != VOID && got != ERR {
                            let d = Diag::new("E0111", e.span, "result is not used").suggest(e.span.before(), "_ = ", "assign it to `_` if it really does not matter");
                            self.diags.push(d);
                        }
                    }
                    _ => self.diags.push(Diag::new("E0114", e.span, "this expression does nothing")),
                }
                false
            }
            StmtKind::If { cond, then, els } => {
                self.check(cond, BOOL);
                let before = self.moved.clone();
                let a = self.block(then);
                let after_then = std::mem::replace(&mut self.moved, before);
                let c = match els {
                    None => false,
                    Some(Else::Block(b)) => self.block(b),
                    Some(Else::If(i)) => self.stmt(i),
                };
                self.join_flow(a, after_then, c);
                a && c
            }
            StmtKind::ForRange { var, var_span, start, end, body } => {
                self.check(start, INT);
                self.check(end, INT);
                self.scopes.push(HashMap::new());
                if var != "_" {
                    self.declare(var, INT, false, *var_span, *var_span);
                }
                self.loop_body(body, false);
                self.pop_scope();
                false
            }
            StmtKind::ForIn { elem, elem_span, index, subject, body } => {
                let t = self.expr(subject, None);
                let elem_ty = match self.kind(t) {
                    Kind::Array(e, _) | Kind::Slice(e) => e,
                    Kind::Str => U8,
                    Kind::Err => ERR,
                    _ => {
                        let n = self.tname(t);
                        self.err("E0104", subject.span, format!("`for ... in` needs an array, found {n}; use `a..b` for numbers"))
                    }
                };
                self.scopes.push(HashMap::new());
                if elem != "_" {
                    self.declare(elem, elem_ty, false, *elem_span, *elem_span);
                    if let Some(v) = self.lookup(elem) {
                        v.view = true;
                    }
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
                self.loop_body(body, false);
                if locked {
                    self.locks.pop();
                }
                self.pop_scope();
                false
            }
            StmtKind::ForCond { cond, body } => {
                self.check(cond, BOOL);
                self.loop_body(body, false);
                false
            }
            StmtKind::ForEver { body } => {
                self.loop_body(body, true);
                !has_break(body)
            }
            StmtKind::Match { subject, arms } => self.match_stmt(s.span, subject, arms),
            StmtKind::Sure(e, _) => {
                self.in_contract = true;
                self.check(e, BOOL);
                self.in_contract = false;
                false
            }
            StmtKind::Defer(body) => {
                if !self.func.claw {
                    self.diags.push(Diag::new("E0142", s.span, "`defer` only exists inside `claw fn`"));
                }
                let (loops, was) = (self.loops, self.in_defer);
                self.loops = 0;
                self.in_defer = true;
                let before = self.moved.clone();
                self.block(body);
                if let Some((name, span)) = self.moved.iter().find(|(n, _)| !before.contains_key(*n)).map(|(n, s)| (n.clone(), *s)) {
                    self.diags.push(Diag::new("E0151", span, format!("`defer` cannot move `{name}`: it runs after everything else")));
                }
                self.moved = before;
                self.loops = loops;
                self.in_defer = was;
                false
            }
            StmtKind::Ret(v) => {
                if self.in_defer {
                    self.diags.push(Diag::new("E0143", s.span, "`defer` cannot leave the function"));
                }
                match v {
                    Some(e) => {
                        if self.ret == VOID {
                            self.expr(e, None);
                            self.diags.push(Diag::new("E0104", e.span, "this function returns nothing; write `ret` alone"));
                        } else {
                            self.check(e, self.ret);
                            self.consume(e);
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
                if self.in_defer {
                    self.diags.push(Diag::new("E0143", s.span, "`defer` cannot leave the function"));
                }
                match self.kind(self.ret) {
                    Kind::Result(_, err) => {
                        self.check(e, err);
                        self.consume(e);
                    }
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
                let state = self.moved.clone();
                if let Some(f) = self.flows.last_mut() {
                    if matches!(s.kind, StmtKind::Break) {
                        f.breaks.push(state);
                    } else {
                        f.conts.push(state);
                    }
                }
                true
            }
        }
    }

    fn loop_body(&mut self, body: &Block, forever: bool) {
        let entry = self.moved.clone();
        self.flows.push(LoopFlow::default());
        self.loops += 1;
        let ends = self.block(body);
        self.loops -= 1;
        let flow = self.flows.pop().unwrap();

        // What has left by the time the loop comes around again.
        let mut back = if ends { HashMap::new() } else { std::mem::take(&mut self.moved) };
        for c in flow.conts {
            back.extend(c);
        }
        let mut again: Vec<(String, Span)> = back.iter().filter(|(n, _)| !entry.contains_key(*n)).map(|(n, s)| (n.clone(), *s)).collect();
        again.sort_by_key(|x| x.1.start);
        for (name, span) in again {
            if self.lookup(&name).is_some() {
                let d = Diag::new("E0151", span, format!("`{name}` moves here on every pass of the loop")).fix(span, format!("copy({name})"), format!("write `copy({name})`"));
                self.diags.push(d);
            }
        }
        let mut out = if forever { HashMap::new() } else { entry };
        if !forever {
            out.extend(back);
        }
        for b in flow.breaks {
            out.extend(b);
        }
        self.moved = out;
    }

    /// After two branches: anything moved on a path that continues counts as moved.
    fn join_flow(&mut self, first_ends: bool, first: HashMap<String, Span>, second_ends: bool) {
        if first_ends {
            return;
        }
        if second_ends {
            self.moved = first;
        } else {
            self.moved.extend(first);
        }
    }

    fn check_live(&mut self, name: &str, span: Span) {
        if let Some(at) = self.moved.remove(name) {
            let d = Diag::new("E0151", span, format!("`{name}` was moved away at line {}", at.line)).fix(at, format!("copy({name})"), format!("write `copy({name})` there"));
            self.diags.push(d);
        }
    }

    /// `x := a.b[i]`: the variable this would be a read-only view into, if it can be one.
    fn view_of(&self, e: &Expr) -> Option<String> {
        if !matches!(e.kind, ExprKind::Field { .. } | ExprKind::Index { .. } | ExprKind::Slice { .. }) || !self.g.types.needs_drop(e.ty.get()) || self.in_contract {
            return None;
        }
        let mut at = e;
        loop {
            match &at.kind {
                ExprKind::Field { e: inner, .. } | ExprKind::Index { e: inner, .. } | ExprKind::Slice { e: inner, .. } if !self.g.types.is_ptr(inner.ty.get()) => at = inner,
                ExprKind::Var(n) => return Some(n.clone()),
                _ => return None,
            }
        }
    }

    /// `e` is being taken over by something else. Only whole local variables can go.
    fn consume(&mut self, e: &Expr) {
        let t = e.ty.get();
        if t == ERR || !self.g.types.needs_drop(t) {
            return;
        }
        if self.in_contract {
            if matches!(e.kind, ExprKind::Var(_) | ExprKind::Field { .. } | ExprKind::Index { .. } | ExprKind::Deref(_) | ExprKind::Slice { .. }) {
                self.diags.push(Diag::new("E0164", e.span, "a contract only looks: it cannot move a value"));
            }
            return;
        }
        match &e.kind {
            ExprKind::Var(n) => {
                let Some(view) = self.lookup(n).map(|v| v.view) else { return };
                if view {
                    let d = Diag::new("E0152", e.span, format!("`{n}` is borrowed here, so it cannot be moved away")).fix(e.span, format!("copy({n})"), format!("write `copy({n})`"));
                    self.diags.push(d);
                    return;
                }
                if self.locks.contains(n) {
                    self.diags.push(Diag::new("E0122", e.span, format!("`{n}` cannot move while it is being walked, matched or viewed")));
                    return;
                }
                self.moved.insert(n.clone(), e.span);
            }
            ExprKind::Field { .. } | ExprKind::Index { .. } | ExprKind::Deref(_) => {
                // `module.Variant` is a new value, not a field being moved out.
                if matches!(*e.res.borrow(), Res::Variant(..)) {
                    return;
                }
                if self.func.claw && matches!(root_of(e, &self.g.types), Root::Through) {
                    return; // Raw memory: moving it out is the programmer's business.
                }
                self.diags.push(Diag::new("E0152", e.span, "only a whole variable can be moved; write `copy(...)`, or use `remove` or `swap`"));
            }
            ExprKind::Slice { .. } => {
                self.diags.push(Diag::new("E0152", e.span, "a slice only borrows; write `copy(...)` to keep it"));
            }
            _ => {}
        }
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

        let root = root_name(subject).map(|n| n.to_string()).filter(|_| self.g.types.needs_drop(t));
        let locked = root.is_some();
        if let Some(r) = root {
            self.locks.push(r);
        }
        let before = self.moved.clone();
        let mut joined: Option<HashMap<String, Span>> = None;
        for arm in arms {
            self.moved = before.clone();
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
            if !ends {
                joined.get_or_insert_with(HashMap::new).extend(self.moved.clone());
            }
        }
        self.moved = joined.unwrap_or(before);
        if locked {
            self.locks.pop();
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
                if let Some(v) = self.lookup(b) {
                    v.view = true;
                }
            }
        }
    }

    fn expr(&mut self, e: &Expr, hint: Option<Ty>) -> Ty {
        let mut t = self.expr_inner(e, hint);
        if !self.func.claw && self.g.types.is_ptr(t) {
            self.diags.push(Diag::new("E0140", e.span, "pointers only exist inside `claw fn`"));
            t = ERR;
        }
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
            ExprKind::Str(_) => {
                // Handed to C: a pointer to the NUL-terminated bytes.
                let bytes = self.g.types.intern(Kind::Pointer(U8));
                if hint == Some(bytes) {
                    bytes
                } else {
                    STR
                }
            }
            ExprKind::Interp(parts) => {
                for p in parts {
                    if let Part::Expr(x) = p {
                        self.in_text = true;
                        let t = self.expr(x, None);
                        self.in_text = false;
                        self.printable(x.span, t);
                    }
                }
                STR
            }
            ExprKind::None => match hint.map(|h| self.kind(h)) {
                Some(Kind::Option(_)) | Some(Kind::Pointer(_)) => hint.unwrap(),
                Some(Kind::Err) => ERR,
                _ => self.err("E0123", e.span, "cannot tell which `?T` this `none` is; give the variable a type"),
            },
            ExprKind::RetVal => {
                if !self.sure_clause {
                    return self.err("E0162", e.span, "`ret` only means the result inside `sure`");
                }
                match self.kind(self.ret) {
                    Kind::Void => self.err("E0162", e.span, "this function returns nothing, so `sure` has no `ret` to look at"),
                    Kind::Result(VOID, _) => self.err("E0162", e.span, "this function has no success value, so `sure` has no `ret` to look at"),
                    Kind::Option(p) => p,
                    Kind::Result(ok, _) => ok,
                    _ => self.ret,
                }
            }
            ExprKind::Old(inner) => {
                if !self.sure_clause || self.in_old {
                    self.expr(inner, None);
                    return self.err("E0163", e.span, "`old(...)` only works in `sure`, and not inside another `old`");
                }
                self.in_old = true;
                let t = self.expr(inner, None);
                self.in_old = false;
                if t != ERR && !self.g.types.is_num(t) && t != BOOL {
                    let n = self.tname(t);
                    return self.err("E0163", inner.span, format!("`old` remembers numbers and bool, not {n}; try `old(len(x))`"));
                }
                t
            }
            ExprKind::Var(name) => self.var(e, name, num_hint),
            ExprKind::AddrOf(inner) => self.addr_of(e, inner, hint),
            ExprKind::Deref(inner) => self.deref_type(e, inner),
            ExprKind::Unary { op, e: inner } => self.unary(e, *op, inner, num_hint),
            ExprKind::Binary { op, l, r } => self.binary(*op, l, r, num_hint),
            ExprKind::Call { module, name, name_span, targs, args, via } => self.call(e, module.as_deref(), name, *name_span, targs, args, via.as_deref()),
            ExprKind::CallValue { f, args } => {
                let t = self.expr(f, None);
                self.call_value(e, t, f.span, args)
            }
            ExprKind::StructLit { module, name, name_span, targs, args } => self.struct_lit(e, module.as_deref(), name, *name_span, targs, args, num_hint),
            ExprKind::ArrayLit(items) => self.array_lit(e, items, num_hint),
            ExprKind::ArrayFill(item) => match num_hint.map(|h| self.kind(h)) {
                Some(Kind::Array(elem, _)) => {
                    self.check(item, elem);
                    if self.g.types.needs_drop(elem) {
                        self.diags.push(Diag::new("E0153", item.span, "`[x...]` would share one owner between many items; build the array item by item"));
                    }
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
                if let ExprKind::Var(alias) = &base.kind {
                    if self.lookup(alias).is_none() && self.g.mods[self.g.cur].imports.contains_key(alias) {
                        let m = self.g.import(alias, base.span, self.diags).unwrap();
                        return self.member(e, m, name, *name_span, num_hint);
                    }
                }
                let b = self.expr(base, None);
                self.field_type(b, name, *name_span)
            }
            ExprKind::Index { e: base, i } => {
                let b = self.expr(base, None);
                self.index_type(b, i, base.span)
            }
            ExprKind::Slice { e: base, lo, hi } => {
                let b = self.expr(base, None);
                for x in [lo, hi].into_iter().flatten() {
                    self.check(x, INT);
                }
                match self.kind(b) {
                    Kind::Array(el, _) => self.g.types.intern(Kind::Slice(el)),
                    Kind::Slice(_) | Kind::Str | Kind::Err => b,
                    _ => {
                        let n = self.tname(b);
                        self.err("E0129", base.span, format!("{n} cannot be sliced"))
                    }
                }
            }
            ExprKind::Try(inner) => self.try_expr(e, inner),
            ExprKind::Else { e: inner, handler } => self.else_expr(e, inner, handler),
        }
    }

    fn var(&mut self, e: &Expr, name: &str, hint: Option<Ty>) -> Ty {
        if self.lookup(name).is_some() {
            self.check_live(name, e.span);
            let v = self.lookup(name).unwrap();
            v.read = true;
            let (ty, view) = (v.ty, v.view);
            if self.sure_clause && !self.in_old && !view && self.g.types.needs_drop(ty) {
                let d = Diag::new("E0165", e.span, format!("`{name}` was taken over with `own`, so it may be gone when `sure` runs; use `old(...)`"));
                self.diags.push(d);
            }
            return ty;
        }
        let m = self.g.cur;
        if self.g.mods[m].consts.contains_key(name) || self.g.mods[m].names.contains_key(name) || self.g.mods[m].gvariants.contains_key(name) || self.g.mods[m].funcs.contains_key(name) {
            return self.member(e, m, name, e.span, hint);
        }
        let core = self.g.core;
        if self.g.mods[core].funcs.get(name).is_some_and(|f| f.1) {
            return self.member(e, core, name, e.span, hint);
        }
        if name == "_" {
            return self.err("E0101", e.span, "`_` can only be assigned to, not read");
        }
        let hint = if self.in_text { "; a literal brace in a string is `{{`" } else { "" };
        self.err("E0101", e.span, format!("unknown name `{name}`{hint}"))
    }

    /// A constant or a field-less variant of module `m`.
    fn member(&mut self, e: &Expr, m: usize, name: &str, span: Span, hint: Option<Ty>) -> Ty {
        let local = m == self.g.cur;
        if self.g.mods[m].funcs.contains_key(name) {
            return self.func_value(e, m, name, span, hint);
        }
        if let Some(&(d, idx, public)) = self.g.mods[m].gvariants.get(name) {
            if !public && !local {
                return self.err("E0147", span, private_msg(name));
            }
            let TypeBody::Variants(vars) = &d.body else { return ERR };
            if !vars[idx].fields.is_empty() {
                return self.err("E0106", span, format!("`{name}` needs its fields: `{name}{{...}}`"));
            }
            let Some(args) = self.targs_from_hint(d, m, hint) else {
                return self.err("E0123", span, format!("cannot tell which `{}[...]` this `{name}` is; give the variable a type or write `{name}[...]{{}}`", d.name));
            };
            let ty = self.g.instantiate_type(d, m, args, self.diags);
            if let Kind::Enum(eid) = self.kind(ty) {
                *e.res.borrow_mut() = Res::Variant(eid, idx);
            }
            return ty;
        }
        if let Some(&(v, t, public)) = self.g.mods[m].consts.get(name) {
            if !public && !local {
                return self.err("E0147", span, private_msg(name));
            }
            *e.res.borrow_mut() = Res::Const(v, t);
            return t;
        }
        match self.g.mods[m].names.get(name).copied() {
            Some((NameDef::Variant(eid, idx, ty), public)) => {
                if !public && !local {
                    return self.err("E0147", span, private_msg(name));
                }
                if !self.g.types.enums[eid].variants[idx].fields.is_empty() {
                    return self.err("E0106", span, format!("`{name}` needs its fields: `{name}{{...}}`"));
                }
                *e.res.borrow_mut() = Res::Variant(eid, idx);
                ty
            }
            Some((NameDef::Type(_), _)) => self.err("E0101", span, format!("`{name}` is a type, not a value")),
            None => self.err("E0101", span, format!("unknown name `{name}`")),
        }
    }

    /// The type arguments of the template `d`, if `hint` is an instance of it.
    fn targs_from_hint(&self, d: &TypeDecl, m: usize, hint: Option<Ty>) -> Option<Vec<Ty>> {
        let (module, name, args) = self.g.instance_of.get(&hint?)?;
        (*module == m && *name == d.name).then(|| args.clone())
    }

    /// A function named where a value is expected: `apply(double, 3)`.
    fn func_value(&mut self, e: &Expr, m: usize, name: &str, span: Span, hint: Option<Ty>) -> Ty {
        let (generic, public) = self.g.mods[m].funcs[name];
        if !public && m != self.g.cur {
            return self.err("E0147", span, private_msg(name));
        }
        if generic.is_some() {
            return self.err("E0176", span, format!("`{name}` is a template, so it is not one function; wrap the use you want in a small `fn`"));
        }
        let full = format!("{}{name}", self.g.mods[m].prefix);
        let Some(sig) = self.g.sigs.get(&full).cloned() else { return ERR };
        if sig.ext.is_some() {
            return self.err("E0176", span, format!("`extern fn {name}` cannot be a value; wrap it in a small `claw fn`"));
        }
        if sig.has_need {
            return self.err("E0177", span, format!("`{name}` has a `need`, and a call through a value could not check it; wrap it in a `fn` that handles every input"));
        }
        let Some(Kind::Fn(id)) = hint.map(|h| self.kind(h)) else {
            if matches!(hint.map(|h| self.kind(h)), Some(Kind::Err)) {
                return ERR;
            }
            return self.err("E0123", span, format!("cannot tell which function type `{name}` should have; write `f fn(int) int = {name}`"));
        };
        let effects = self.g.types.fns[id].effects;
        *e.res.borrow_mut() = Res::Func(full);
        self.g.types.fn_type(FnType { params: sig.params, own: sig.own, ret: sig.ret, effects })
    }

    fn addr_of(&mut self, e: &Expr, inner: &Expr, hint: Option<Ty>) -> Ty {
        if self.in_contract {
            self.expr(inner, None);
            return self.err("E0164", e.span, "a contract only looks: it cannot take `&`");
        }
        if !self.func.claw {
            if !matches!(hint.map(|h| self.g.types.kind(h)), Some(Kind::Ref(_)) | None) {
                self.expr(inner, None);
                return self.err("E0157", e.span, "`&x` only works as an argument to a `&` parameter");
            }
        }
        if !matches!(inner.kind, ExprKind::Var(_) | ExprKind::Field { .. } | ExprKind::Index { .. } | ExprKind::Deref(_)) {
            self.expr(inner, None);
            return self.err("E0131", inner.span, "only variables, fields and elements have an address");
        }
        let t = self.place(inner);
        match root_of(inner, &self.g.types) {
            Root::Var(root) => {
                self.check_live(&root, inner.span);
                if self.locks.contains(&root) {
                    self.diags.push(Diag::new("E0122", inner.span, format!("`{root}` cannot change while it is being walked, matched or viewed")));
                }
                let info = self.lookup(&root).map(|v| (v.mutable, v.decl));
                if let Some((mutable, decl)) = info {
                    if !mutable {
                        let d = Diag::new("E0103", inner.span, format!("cannot take the address of `{root}`, it is not `mut`")).fix(decl, "mut ", format!("declare `mut {root}`"));
                        self.diags.push(d);
                    }
                    let v = self.lookup(&root).unwrap();
                    v.read = true;
                    v.written = true;
                }
                if matches!(inner.kind, ExprKind::Var(_)) && !self.g.types.is_aggregate(t) {
                    self.func.addressed.borrow_mut().insert(root);
                }
            }
            Root::Through => {}
            Root::Other => {
                return self.err("E0131", inner.span, "only variables, fields and elements have an address");
            }
        }
        if t == ERR {
            return ERR;
        }
        let wants_ref = matches!(hint.map(|h| self.g.types.kind(h)), Some(Kind::Ref(_)));
        if self.func.claw && !wants_ref {
            self.g.types.intern(Kind::Pointer(t))
        } else {
            self.g.types.intern(Kind::Ref(t))
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
                } else if a != ERR && !self.g.types.is_num(a) && a != BOOL && !self.g.types.is_ptr(a) && a != STR {
                    let n = self.tname(a);
                    self.diags.push(Diag::new("E0104", l.span, format!("`{}` works on numbers and bool, not {n}; use `match`", op.text())));
                }
                BOOL
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let (a, b) = self.operands(l, r, None);
                if a != b {
                    self.mismatch(r.span, b, a);
                } else if a != ERR && !self.g.types.is_num(a) && !self.g.types.is_ptr(a) && a != STR {
                    let n = self.tname(a);
                    self.diags.push(Diag::new("E0104", l.span, format!("`{}` needs numbers, found {n}", op.text())));
                }
                BOOL
            }
            _ => {
                let (a, b) = self.operands(l, r, hint);
                if self.g.types.is_ptr(a) && matches!(op, BinOp::Add | BinOp::Sub) {
                    if b == INT {
                        return a;
                    }
                    if b == a && op == BinOp::Sub {
                        return INT;
                    }
                    let want = if op == BinOp::Sub { "int or the same pointer type" } else { "int" };
                    return self.err("E0104", r.span, format!("pointer arithmetic needs {want}"));
                }
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
                        self.in_text = true;
                        let t = self.expr(e, None);
                        self.in_text = false;
                        self.printable(e.span, t);
                    }
                }
            }
            _ => {
                let t = self.expr(a, None);
                self.printable(a.span, t);
            }
        }
    }

    fn builtin(&mut self, e: &Expr, name: &str, span: Span, targs: &[TypeExpr], args: &[Expr]) -> Option<Ty> {
        Some(match name {
            "print" => {
                if args.len() != 1 {
                    self.diags.push(Diag::new("E0106", span, format!("`print` takes 1 value, found {}", args.len())));
                }
                for a in args {
                    self.print_arg(a);
                }
                VOID
            }
            "len" => {
                if args.len() != 1 {
                    return Some(self.err("E0106", span, format!("`len` takes 1 value, found {}", args.len())));
                }
                let t = self.expr(&args[0], None);
                match self.kind(t) {
                    Kind::Array(..) | Kind::Slice(_) | Kind::Str | Kind::Err => INT,
                    _ => {
                        let n = self.tname(t);
                        self.err("E0104", args[0].span, format!("`len` needs an array, found {n}"))
                    }
                }
            }
            "__state" if self.g.mods[self.g.cur].prefix == "__check." => self.g.types.intern(Kind::Pointer(INT)),
            // For `std/par.claw` only: whether tasks must run one after another, and where a task's thread starts.
            "__sequential" if self.g.mods[self.g.cur].prefix == "par." => BOOL,
            // For `std/fs.claw` only: the values of `open` flags differ between macOS and Linux.
            "__linux" if self.g.mods[self.g.cur].prefix == "fs." => BOOL,
            "__entry" if self.g.mods[self.g.cur].prefix == "par." => {
                let targs = self.resolve_targs(targs)?;
                let m = self.g.cur;
                let (Some(f), _) = self.g.mods[m].funcs.get("thread_main").copied()? else { return Some(ERR) };
                let full = self.g.instantiate_fn(f, m, targs, self.diags);
                *e.res.borrow_mut() = Res::Func(full);
                self.g.types.intern(Kind::Pointer(U8))
            }
            "copy" => {
                if args.len() != 1 {
                    return Some(self.err("E0106", span, format!("`copy` takes 1 value, found {}", args.len())));
                }
                let t = self.expr(&args[0], None);
                let t = self.value_type(args[0].span, t);
                if t != ERR && self.g.types.has_user_drop(t) {
                    let n = self.tname(t);
                    return Some(self.err("E0154", span, format!("{n} has its own `drop`, so the compiler cannot copy it")));
                }
                t
            }
            "sizeof" => {
                let Some(t) = self.one_targ(name, span, targs) else { return Some(ERR) };
                if !args.is_empty() {
                    return Some(self.err("E0106", span, "`sizeof[T]()` takes no values"));
                }
                self.g.ensure_layout(t, self.diags);
                *e.res.borrow_mut() = Res::Const(ConstVal::Int(self.g.types.size(t) as i128), INT);
                INT
            }
            "alloc" => {
                let t = self.one_targ(name, span, targs);
                if !self.func.claw {
                    return Some(self.err("E0140", span, "`alloc` only exists inside `claw fn`"));
                }
                if args.len() != 1 {
                    return Some(self.err("E0106", span, "`alloc[T](n)` takes the number of items"));
                }
                self.check(&args[0], INT);
                match t {
                    Some(t) => self.g.types.intern(Kind::Pointer(t)),
                    None => ERR,
                }
            }
            "free" => {
                if !self.func.claw {
                    return Some(self.err("E0140", span, "`free` only exists inside `claw fn`"));
                }
                if args.len() != 1 {
                    return Some(self.err("E0106", span, "`free(p)` takes one pointer"));
                }
                let t = self.expr(&args[0], None);
                if !self.g.types.is_ptr(t) && t != ERR {
                    let n = self.tname(t);
                    return Some(self.err("E0104", args[0].span, format!("`free` needs a pointer, found {n}")));
                }
                VOID
            }
            "cast" => {
                let t = self.one_targ(name, span, targs);
                if !self.func.claw {
                    return Some(self.err("E0140", span, "`cast` only exists inside `claw fn`"));
                }
                if args.len() != 1 {
                    return Some(self.err("E0106", span, "`cast[T](x)` converts 1 value"));
                }
                let from = self.expr(&args[0], None);
                let Some(t) = t else { return Some(ERR) };
                let word = |g: &Types, x: Ty| g.is_ptr(x) || x == INT || x == U64;
                if from != ERR && (!word(&self.g.types, from) || !word(&self.g.types, t)) {
                    return Some(self.err("E0104", span, "`cast` converts between pointers, `int` and `u64`; use `T(x)` for numbers"));
                }
                t
            }
            _ => {
                let target = prim(name)?;
                if args.len() != 1 {
                    return Some(self.err("E0106", span, format!("`{name}(...)` converts 1 value, found {}", args.len())));
                }
                let hint = if is_lit(&args[0]) { Some(target) } else { None };
                let t = self.expr(&args[0], hint);
                let ok = t == ERR
                    || (self.g.types.is_num(t) && self.g.types.is_num(target))
                    || (t == BOOL && self.g.types.is_int(target))
                    || (t == BOOL && target == BOOL);
                if !ok {
                    let n = self.tname(t);
                    return Some(self.err("E0104", args[0].span, format!("cannot convert {n} to {name}")));
                }
                target
            }
        })
    }

    fn one_targ(&mut self, name: &str, span: Span, targs: &[TypeExpr]) -> Option<Ty> {
        if targs.len() != 1 {
            self.diags.push(Diag::new("E0144", span, format!("write `{name}[T](...)` with one type")));
            return None;
        }
        let t = self.g.resolve(&targs[0], self.diags);
        (t != ERR).then_some(t)
    }

    fn resolve_targs(&mut self, targs: &[TypeExpr]) -> Option<Vec<Ty>> {
        let mut out = Vec::with_capacity(targs.len());
        for a in targs {
            let t = self.g.resolve(a, self.diags);
            if t == ERR {
                return None;
            }
            out.push(t);
        }
        Some(out)
    }

    /// Works out type parameters from the arguments; each argument is checked once.
    /// Only the expected type can say what this name means.
    fn waits_for_hint(&mut self, a: &Expr) -> bool {
        if hint_dep(a) {
            return true;
        }
        let ExprKind::Var(name) = &a.kind else { return false };
        if self.lookup(name).is_some() {
            return false;
        }
        let m = &self.g.mods[self.g.cur];
        m.gvariants.contains_key(name) || self.named_function(name).is_some()
    }

    /// The signature behind a plain name, if it means an ordinary function.
    fn named_function(&self, name: &str) -> Option<FnSig> {
        let cur = &self.g.mods[self.g.cur];
        if cur.consts.contains_key(name) || cur.names.contains_key(name) {
            return None;
        }
        let m = match cur.funcs.get(name) {
            Some((None, _)) => self.g.cur,
            Some(_) => return None,
            None => match self.g.mods[self.g.core].funcs.get(name) {
                Some((None, true)) => self.g.core,
                _ => return None,
            },
        };
        self.g.sigs.get(&format!("{}{name}", self.g.mods[m].prefix)).cloned()
    }

    fn infer(&mut self, tparams: &[(String, Span)], pats: &[&TypeExpr], args: &[Expr], explicit: &[Ty], first: Option<Ty>) -> (HashMap<String, Ty>, Vec<Option<Ty>>) {
        let mut bind: HashMap<String, Ty> = tparams.iter().map(|p| p.0.clone()).zip(explicit.iter().copied()).collect();
        let mut got: Vec<Option<Ty>> = vec![None; args.len()];
        got[..first.map_or(0, |_| 1).min(args.len())].fill(first);
        for (i, a) in args.iter().enumerate() {
            if self.waits_for_hint(a) {
                continue;
            }
            let t = match got[i] {
                Some(t) => t,
                None => self.expr(a, None),
            };
            got[i] = Some(t);
            if let Some(p) = pats.get(i) {
                self.g.unify(p, t, &mut bind, tparams);
            }
        }
        // A function named as an argument says what its parameter's type variables are.
        for (i, a) in args.iter().enumerate() {
            let (ExprKind::Var(name), Some(p), true) = (&a.kind, pats.get(i), got[i].is_none()) else { continue };
            if self.lookup(name).is_none() && matches!(p.kind, TypeKind::Fn { .. }) {
                if let Some(sig) = self.named_function(name) {
                    let t = self.g.types.fn_type(FnType { params: sig.params, own: sig.own, ret: sig.ret, effects: 0 });
                    self.g.unify(p, t, &mut bind, tparams);
                }
            }
        }
        for (i, a) in args.iter().enumerate() {
            if got[i].is_some() {
                continue;
            }
            // A literal decides a parameter only if nothing else has.
            let open = pats.get(i).is_some_and(|p| mentions_unbound(p, &bind, tparams));
            if open && !matches!(a.kind, ExprKind::None | ExprKind::ArrayFill(_)) {
                let t = self.expr(a, None);
                got[i] = Some(t);
                self.g.unify(pats[i], t, &mut bind, tparams);
            }
        }
        (bind, got)
    }

    /// A fixed array handed to a parameter that only reads a `[]T`.
    fn array_view(&self, got: Ty, want: Ty, own: bool) -> bool {
        !own && matches!((self.kind(got), self.kind(want)), (Kind::Array(a, _), Kind::Slice(b)) if a == b)
    }

    /// `own` says which parameters take the value over; shorter means none may be viewed.
    fn finish_args(&mut self, args: &[Expr], wants: &[Ty], got: &[Option<Ty>], own: &[bool]) {
        for (i, a) in args.iter().enumerate() {
            let is_own = own.get(i).copied().unwrap_or(true);
            match (wants.get(i), got.get(i).copied().flatten()) {
                (Some(want), Some(g)) if self.array_view(g, *want, is_own) => {}
                (Some(want), Some(g)) => self.accept(a, g, *want),
                (Some(want), None) if !is_own && matches!(self.kind(*want), Kind::Slice(_)) => {
                    let g = self.expr(a, Some(*want));
                    if !self.array_view(g, *want, false) {
                        self.accept(a, g, *want);
                    }
                }
                (Some(want), None) => self.check(a, *want),
                (None, Some(_)) => {}
                (None, None) => {
                    // Past the named parameters of a C function: a string means its bytes.
                    let bytes = self.g.types.intern(Kind::Pointer(U8));
                    let hint = matches!(a.kind, ExprKind::Str(_)).then_some(bytes);
                    self.expr(a, hint);
                }
            }
        }
    }

    /// A call through something that holds a function.
    fn call_value(&mut self, e: &Expr, ty: Ty, span: Span, args: &[Expr]) -> Ty {
        let Kind::Fn(id) = self.kind(ty) else {
            for a in args {
                self.expr(a, None);
            }
            if ty == ERR {
                return ERR;
            }
            let n = self.tname(ty);
            return self.err("E0104", span, format!("{n} cannot be called"));
        };
        let f = self.g.types.fns[id].clone();
        if args.len() != f.params.len() {
            self.diags.push(Diag::new("E0106", span, format!("this function takes {} values, found {}", f.params.len(), args.len())));
        }
        self.finish_args(args, &f.params, &vec![None; args.len()], &f.own);
        for (a, own) in args.iter().zip(&f.own) {
            if *own {
                self.consume(a);
            }
        }
        if !self.func.claw {
            self.check_exclusive(span, args);
        }
        *e.res.borrow_mut() = Res::Value(ty);
        f.ret
    }

    fn call(&mut self, e: &Expr, module: Option<&str>, name: &str, span: Span, targs: &[TypeExpr], args: &[Expr], via: Option<&Expr>) -> Ty {
        if module.is_none() {
            let held = self.lookup(name).map(|v| v.ty);
            if held == Some(ERR) {
                self.var(e, name, None);
                for a in args {
                    self.expr(a, None);
                }
                return ERR;
            }
            if let Some(t) = held.filter(|t| matches!(self.kind(*t), Kind::Fn(_))) {
                self.var(e, name, None);
                return self.call_value(e, t, span, args);
            }
            if let Some(t) = self.builtin(e, name, span, targs, args) {
                return t;
            }
        }
        let cur = self.g.cur;
        let mut first: Option<Ty> = None;
        let target = match module {
            Some(alias) => {
                if let Some(v) = self.lookup(alias) {
                    v.read = true;
                    let base = v.ty;
                    let base = match self.kind(base) {
                        Kind::Ref(t) => t,
                        _ => base,
                    };
                    let field = match self.kind(base) {
                        Kind::Struct(id) => self.g.types.structs[id].fields.iter().find(|f| f.name == name).map(|f| f.ty),
                        _ => None,
                    };
                    if let (Some(t), Some(callee)) = (field.filter(|t| matches!(self.kind(*t), Kind::Fn(_))), via) {
                        self.expr(callee, None);
                        return self.call_value(e, t, span, args);
                    }
                    for a in args {
                        self.expr(a, None);
                    }
                    return self.err("E0150", span, format!("Claw has no methods; write `{name}({alias}, ...)`"));
                }
                self.g.import(alias, span, self.diags)
            }
            None if self.g.mods[cur].funcs.contains_key(name) => Some(cur),
            None => {
                // Not here: look in the module that declares the first argument's type.
                let mut found = None;
                let plain = args.first().is_some_and(|a| !self.waits_for_hint(a));
                if let Some(a) = args.first().filter(|_| plain) {
                    let t = self.expr(a, None);
                    first = Some(t);
                    found = self.g.owner_of(t).filter(|m| self.g.mods[*m].funcs.get(name).is_some_and(|f| f.1));
                }
                // Last, the module every program can see without importing it.
                let core = self.g.core;
                found.or(self.g.mods[core].funcs.get(name).filter(|f| f.1).map(|_| core))
            }
        };
        let entry = target.and_then(|m| self.g.mods[m].funcs.get(name).copied().map(|f| (m, f)));
        let Some((m, (generic, public))) = entry else {
            if module.is_none() || target.is_some() {
                self.diags.push(Diag::new("E0101", span, format!("unknown function `{name}`")));
            }
            for (i, a) in args.iter().enumerate() {
                if !(i == 0 && first.is_some()) {
                    self.expr(a, None);
                }
            }
            return ERR;
        };
        if !public && m != cur {
            self.diags.push(Diag::new("E0147", span, private_msg(name)));
        }
        if name == "drop" {
            self.diags.push(Diag::new("E0156", span, "`drop` runs by itself; it cannot be called"));
        }

        let (full, got) = if let Some(f) = generic {
            let Some(explicit) = self.resolve_targs(targs) else { return ERR };
            if explicit.len() > f.tparams.len() {
                return self.err("E0144", span, format!("`{name}` takes {} type arguments, found {}", f.tparams.len(), explicit.len()));
            }
            if f.params.len() != args.len() {
                self.diags.push(Diag::new("E0106", span, format!("`{name}` takes {} values, found {}", f.params.len(), args.len())));
            }
            let pats: Vec<&TypeExpr> = f.params.iter().map(|p| &p.ty).collect();
            let (bind, got) = self.infer(&f.tparams, &pats, args, &explicit, first);
            let mut ordered = Vec::with_capacity(f.tparams.len());
            for (p, _) in &f.tparams {
                match bind.get(p) {
                    Some(&ERR) => return ERR,
                    Some(t) => ordered.push(*t),
                    None => return self.err("E0144", span, format!("cannot work out `{p}`; write `{name}[...](...)`")),
                }
            }
            (self.g.instantiate_fn(f, m, ordered, self.diags), got)
        } else {
            if !targs.is_empty() {
                self.diags.push(Diag::new("E0144", span, format!("`{name}` takes no type arguments")));
            }
            let mut got = vec![None; args.len()];
            if !args.is_empty() {
                got[0] = first;
            }
            (format!("{}{}", self.g.mods[m].prefix, name), got)
        };

        let Some(sig) = self.g.sigs.get(&full).cloned() else { return ERR };
        if sig.ext.is_some() && !self.func.claw {
            self.diags.push(Diag::new("E0141", span, format!("`extern fn {name}` can only be called from `claw fn`")));
        }
        let arity_ok = args.len() == sig.params.len() || (sig.variadic && args.len() > sig.params.len());
        if !arity_ok && generic.is_none() {
            self.diags.push(Diag::new("E0106", span, format!("`{name}` takes {} values, found {}", sig.params.len(), args.len())));
        }
        self.finish_args(args, &sig.params, &got, &sig.own);
        for (a, own) in args.iter().zip(&sig.own) {
            if *own {
                self.consume(a);
            }
        }
        if !self.func.claw {
            self.check_exclusive(span, args);
        }
        if sig.variadic {
            for a in args.iter().skip(sig.params.len()) {
                if !is_c_type(&self.g.types, a.ty.get()) || a.ty.get() == STRLIT {
                    self.diags.push(Diag::new("E0149", a.span, "only numbers, bool and pointers can be passed to C"));
                }
            }
        }
        *e.res.borrow_mut() = Res::Func(full);
        sig.ret
    }

    /// One call may not change something while also reading or changing it elsewhere.
    fn check_exclusive(&mut self, span: Span, args: &[Expr]) {
        let mut muts = Vec::new();
        for a in args {
            mutable_paths(a, &mut muts);
        }
        for (i, m) in muts.iter().enumerate() {
            if muts[i + 1..].iter().any(|o| overlap(o, m)) {
                self.diags.push(Diag::new("E0158", span, format!("`{}` is passed with `&` twice in one call", m.0)));
                return;
            }
        }
        for a in args {
            if matches!(a.kind, ExprKind::AddrOf(_)) || !self.g.types.is_aggregate(a.ty.get()) {
                continue;
            }
            if let Some(p) = path_of(a) {
                if muts.iter().any(|m| overlap(m, &p)) {
                    self.diags.push(Diag::new("E0158", a.span, format!("`{}` is read here and changed with `&` in the same call", p.0)));
                    return;
                }
            }
        }
    }

    fn struct_lit(&mut self, e: &Expr, module: Option<&str>, name: &str, span: Span, targs: &[TypeExpr], args: &[Expr], hint: Option<Ty>) -> Ty {
        let cur = self.g.cur;
        let m = match module {
            Some(alias) => match self.g.import(alias, span, self.diags) {
                Some(m) => m,
                None => {
                    for a in args {
                        self.expr(a, None);
                    }
                    return ERR;
                }
            },
            None => cur,
        };
        let check_all = |cx: &mut Self| {
            for a in args {
                cx.expr(a, None);
            }
        };
        if let Some(&(d, public)) = self.g.mods[m].gtypes.get(name) {
            if !public && m != cur {
                check_all(self);
                return self.err("E0147", span, private_msg(name));
            }
            let TypeBody::Struct(fields) = &d.body else { return ERR };
            let Some(explicit) = self.resolve_targs(targs) else { return ERR };
            if args.len() != fields.len() {
                self.diags.push(Diag::new("E0106", span, format!("`{name}` has {} fields, found {}", fields.len(), args.len())));
            }
            let pats: Vec<&TypeExpr> = fields.iter().map(|f| &f.ty).collect();
            let (bind, got) = self.infer(&d.tparams, &pats, args, &explicit, None);
            let mut ordered = Vec::with_capacity(d.tparams.len());
            for (p, _) in &d.tparams {
                match bind.get(p) {
                    Some(&ERR) => return ERR,
                    Some(t) => ordered.push(*t),
                    None => return self.err("E0144", span, format!("cannot work out `{p}`; write `{name}[...]{{...}}`")),
                }
            }
            let ty = self.g.instantiate_type(d, m, ordered, self.diags);
            if ty == ERR {
                return ERR;
            }
            let Kind::Struct(id) = self.kind(ty) else { return ERR };
            let wants: Vec<Ty> = self.g.types.structs[id].fields.iter().map(|f| f.ty).collect();
            self.finish_args(args, &wants, &got, &[]);
            for a in args {
                self.consume(a);
            }
            return ty;
        }
        if let Some(&(d, idx, public)) = self.g.mods[m].gvariants.get(name) {
            if !public && m != cur {
                check_all(self);
                return self.err("E0147", span, private_msg(name));
            }
            let TypeBody::Variants(vars) = &d.body else { return ERR };
            let fields = &vars[idx].fields;
            let Some(mut explicit) = self.resolve_targs(targs) else { return ERR };
            if explicit.is_empty() {
                explicit = self.targs_from_hint(d, m, hint).unwrap_or_default();
            }
            if args.len() != fields.len() {
                self.diags.push(Diag::new("E0106", span, format!("`{name}` has {} fields, found {}", fields.len(), args.len())));
            }
            let pats: Vec<&TypeExpr> = fields.iter().map(|f| &f.ty).collect();
            let (bind, got) = self.infer(&d.tparams, &pats, args, &explicit, None);
            let mut ordered = Vec::with_capacity(d.tparams.len());
            for (p, _) in &d.tparams {
                match bind.get(p) {
                    Some(&ERR) => return ERR,
                    Some(t) => ordered.push(*t),
                    None => return self.err("E0144", span, format!("cannot work out `{p}`; write `{name}[...]{{...}}`")),
                }
            }
            let ty = self.g.instantiate_type(d, m, ordered, self.diags);
            let Kind::Enum(eid) = self.kind(ty) else { return ERR };
            *e.res.borrow_mut() = Res::Variant(eid, idx);
            let wants: Vec<Ty> = self.g.types.enums[eid].variants[idx].fields.iter().map(|f| f.ty).collect();
            self.finish_args(args, &wants, &got, &[]);
            for a in args {
                self.consume(a);
            }
            return ty;
        }
        let (fields, ty, public) = match self.g.mods[m].names.get(name).copied() {
            Some((NameDef::Type(t), public)) => match self.kind(t) {
                Kind::Struct(id) => (self.g.types.structs[id].fields.clone(), t, public),
                _ => {
                    check_all(self);
                    return self.err("E0101", span, format!("`{name}` is a variant type; build one of its variants"));
                }
            },
            Some((NameDef::Variant(eid, idx, t), public)) => {
                *e.res.borrow_mut() = Res::Variant(eid, idx);
                (self.g.types.enums[eid].variants[idx].fields.clone(), t, public)
            }
            None => {
                check_all(self);
                return self.err("E0112", span, format!("unknown type `{name}`"));
            }
        };
        if !public && m != cur {
            self.diags.push(Diag::new("E0147", span, private_msg(name)));
        }
        if args.len() != fields.len() {
            self.diags.push(Diag::new("E0106", span, format!("`{name}` has {} fields, found {}", fields.len(), args.len())));
        }
        for (a, f) in args.iter().zip(&fields) {
            self.check(a, f.ty);
            self.consume(a);
        }
        for a in args.iter().skip(fields.len()) {
            self.expr(a, None);
        }
        ty
    }

    fn array_lit(&mut self, e: &Expr, items: &[Expr], hint: Option<Ty>) -> Ty {
        if let Some(h) = hint {
            match self.kind(h) {
                Kind::Array(elem, n) => {
                    if items.len() as u64 != n {
                        let name = self.tname(h);
                        self.diags.push(Diag::new("E0104", e.span, format!("`{name}` needs {n} items, found {}", items.len())));
                    }
                    self.array_items(items, elem);
                    return h;
                }
                Kind::Slice(elem) => {
                    self.array_items(items, elem);
                    return h;
                }
                _ => {}
            }
        }
        if items.is_empty() {
            return self.err("E0123", e.span, "cannot tell what this empty array holds; write `a []int = []`");
        }
        let first = self.expr(&items[0], None);
        let first = self.value_type(items[0].span, first);
        self.consume(&items[0]);
        self.array_items(&items[1..], first);
        if first == ERR {
            return ERR;
        }
        self.g.types.intern(Kind::Array(first, items.len() as u64))
    }

    fn array_items(&mut self, items: &[Expr], elem: Ty) {
        for it in items {
            self.check(it, elem);
            self.consume(it);
        }
    }

    fn try_expr(&mut self, e: &Expr, inner: &Expr) -> Ty {
        if self.in_contract {
            self.expr(inner, None);
            return self.err("E0164", e.span, "`?` would leave in the middle of a contract; get the value first");
        }
        if self.in_defer {
            self.diags.push(Diag::new("E0143", e.span, "`defer` cannot leave the function"));
        }
        let t = self.expr(inner, None);
        self.consume(inner);
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
        self.consume(inner);
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
                    self.consume(v);
                }
            }
            ElseKind::Block(binder, b) => {
                if self.in_contract {
                    self.diags.push(Diag::new("E0164", e.span, "a contract cannot contain an `else` block; use `else value`"));
                }
                let before = self.moved.clone();
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
                self.moved = before;
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
        ExprKind::Try(e) | ExprKind::Field { e, .. } | ExprKind::Unary { e, .. } | ExprKind::AddrOf(e) | ExprKind::Deref(e) => expr_breaks(e),
        ExprKind::Index { e, i } => expr_breaks(e) || expr_breaks(i),
        ExprKind::Slice { e, .. } => expr_breaks(e),
        ExprKind::Binary { l, r, .. } => expr_breaks(l) || expr_breaks(r),
        ExprKind::Call { args, .. } | ExprKind::StructLit { args, .. } | ExprKind::ArrayLit(args) => args.iter().any(expr_breaks),
        ExprKind::CallValue { f, args } => expr_breaks(f) || args.iter().any(expr_breaks),
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
