//! The type table: every type is interned, so equal types have equal ids.

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ty(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Int { bits: u8, signed: bool },
    Float { bits: u8 },
    Bool,
    Void,
    StrLit,
    Err,
    Struct(usize),
    Enum(usize),
    Array(Ty, u64),
    Pointer(Ty),
    /// A growable array, or a read-only view of part of one.
    Slice(Ty),
    Str,
    /// A `&T` parameter.
    Ref(Ty),
    Option(Ty),
    Result(Ty, Ty),
    /// A function as a value: an index into `Types::fns`.
    Fn(usize),
    /// A 128-bit SIMD value; it lives in a vector register and only exists inside `claw fn`.
    Vec(Vk),
}

/// The three SIMD types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Vk {
    F64x2,
    F32x4,
    I32x4,
}

impl Vk {
    pub const ALL: [Vk; 3] = [Vk::F64x2, Vk::F32x4, Vk::I32x4];

    pub fn name(self) -> &'static str {
        match self {
            Vk::F64x2 => "f64x2",
            Vk::F32x4 => "f32x4",
            Vk::I32x4 => "i32x4",
        }
    }

    pub fn lanes(self) -> u32 {
        match self {
            Vk::F64x2 => 2,
            Vk::F32x4 | Vk::I32x4 => 4,
        }
    }

    /// The type of one lane.
    pub fn elem(self) -> Ty {
        match self {
            Vk::F64x2 => F64,
            Vk::F32x4 => F32,
            Vk::I32x4 => I32,
        }
    }

    pub fn is_float(self) -> bool {
        self != Vk::I32x4
    }

    pub fn from_name(n: &str) -> Option<Vk> {
        Vk::ALL.into_iter().find(|v| v.name() == n)
    }

    /// The vector whose lanes are `t`, for `vload` and `vstore`.
    pub fn of_elem(t: Ty) -> Option<Vk> {
        Vk::ALL.into_iter().find(|v| v.elem() == t)
    }
}

pub const MEM: u8 = 1;
pub const IO: u8 = 2;

/// The two effects as written in a signature.
pub fn spell_effects(mask: u8) -> String {
    let mut parts = Vec::new();
    if mask & MEM != 0 {
        parts.push("+mem");
    }
    if mask & IO != 0 {
        parts.push("+io");
    }
    parts.join(" ")
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FnType {
    pub params: Vec<Ty>,
    pub own: Vec<bool>,
    pub ret: Ty,
    pub effects: u8,
}

pub const I8: Ty = Ty(0);
pub const I16: Ty = Ty(1);
pub const I32: Ty = Ty(2);
pub const INT: Ty = Ty(3);
pub const U8: Ty = Ty(4);
pub const U16: Ty = Ty(5);
pub const U32: Ty = Ty(6);
pub const U64: Ty = Ty(7);
pub const F32: Ty = Ty(8);
pub const F64: Ty = Ty(9);
pub const BOOL: Ty = Ty(10);
pub const VOID: Ty = Ty(11);
pub const STRLIT: Ty = Ty(12);
pub const ERR: Ty = Ty(13);
pub const STR: Ty = Ty(14);

#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    pub ty: Ty,
    pub offset: u64,
}

#[derive(Clone, Debug)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<Field>,
    pub size: u64,
    pub align: u64,
    /// The module that declared it, for finding its functions.
    pub module: usize,
    /// Layout progress: 0 not started, 1 in progress, 2 done.
    pub state: u8,
}

#[derive(Clone, Debug)]
pub struct Variant {
    pub name: String,
    /// Offsets are from the start of the value, after the 8-byte tag.
    pub fields: Vec<Field>,
}

#[derive(Clone, Debug)]
pub struct EnumDef {
    pub name: String,
    pub variants: Vec<Variant>,
    pub size: u64,
    pub module: usize,
    pub state: u8,
}

pub const TAG_SIZE: u64 = 8;

pub struct Types {
    kinds: Vec<Kind>,
    map: HashMap<Kind, Ty>,
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    /// Types with a user-written `drop`, and the function that does it.
    pub user_drop: HashMap<Ty, String>,
    pub fns: Vec<FnType>,
}

/// `[]T` and `str` in memory: pointer, length, capacity. Capacity 0 means the
/// bytes are not ours to free (static data or a view).
pub const SEQ_SIZE: u64 = 24;
pub const SEQ_PTR: u64 = 0;
pub const SEQ_LEN: u64 = 8;
pub const SEQ_CAP: u64 = 16;

pub fn align_up(v: u64, a: u64) -> u64 {
    (v + a - 1) & !(a - 1)
}

impl Types {
    pub fn new() -> Types {
        let mut t = Types { kinds: Vec::new(), map: HashMap::new(), structs: Vec::new(), enums: Vec::new(), user_drop: HashMap::new(), fns: Vec::new() };
        for k in [
            Kind::Int { bits: 8, signed: true },
            Kind::Int { bits: 16, signed: true },
            Kind::Int { bits: 32, signed: true },
            Kind::Int { bits: 64, signed: true },
            Kind::Int { bits: 8, signed: false },
            Kind::Int { bits: 16, signed: false },
            Kind::Int { bits: 32, signed: false },
            Kind::Int { bits: 64, signed: false },
            Kind::Float { bits: 32 },
            Kind::Float { bits: 64 },
            Kind::Bool,
            Kind::Void,
            Kind::StrLit,
            Kind::Err,
            Kind::Str,
        ] {
            t.intern(k);
        }
        t
    }

    pub fn intern(&mut self, k: Kind) -> Ty {
        if let Some(t) = self.map.get(&k) {
            return *t;
        }
        let t = Ty(self.kinds.len() as u32);
        self.kinds.push(k);
        self.map.insert(k, t);
        t
    }

    pub fn fn_type(&mut self, f: FnType) -> Ty {
        let id = match self.fns.iter().position(|x| *x == f) {
            Some(i) => i,
            None => {
                self.fns.push(f);
                self.fns.len() - 1
            }
        };
        self.intern(Kind::Fn(id))
    }

    pub fn kind(&self, t: Ty) -> Kind {
        self.kinds[t.0 as usize]
    }

    /// How many types have been interned.
    pub fn count(&self) -> usize {
        self.kinds.len()
    }

    pub fn is_int(&self, t: Ty) -> bool {
        matches!(self.kind(t), Kind::Int { .. })
    }

    pub fn is_float(&self, t: Ty) -> bool {
        matches!(self.kind(t), Kind::Float { .. })
    }

    pub fn is_vec(&self, t: Ty) -> bool {
        matches!(self.kind(t), Kind::Vec(_))
    }

    /// Whether a SIMD value is, or sits inside, `t`.
    pub fn holds_vec(&self, t: Ty) -> bool {
        self.holds_vec_seen(t, &mut Vec::new())
    }

    /// `seen` only holds named types: they are the only way back to a type already being
    /// looked at. Arrays of arrays of arrays (a template that keeps wrapping its argument)
    /// stay linear.
    fn holds_vec_seen(&self, t: Ty, seen: &mut Vec<(bool, usize)>) -> bool {
        match self.kind(t) {
            Kind::Vec(_) => true,
            Kind::Array(e, _) | Kind::Slice(e) | Kind::Option(e) | Kind::Pointer(e) | Kind::Ref(e) => self.holds_vec_seen(e, seen),
            Kind::Result(a, b) => self.holds_vec_seen(a, seen) || self.holds_vec_seen(b, seen),
            Kind::Struct(id) => {
                if seen.contains(&(false, id)) {
                    return false;
                }
                seen.push((false, id));
                self.structs[id].fields.iter().any(|f| self.holds_vec_seen(f.ty, seen))
            }
            Kind::Enum(id) => {
                if seen.contains(&(true, id)) {
                    return false;
                }
                seen.push((true, id));
                self.enums[id].variants.iter().any(|v| v.fields.iter().any(|f| self.holds_vec_seen(f.ty, seen)))
            }
            Kind::Fn(id) => self.fns[id].params.iter().any(|p| self.holds_vec_seen(*p, seen)) || self.holds_vec_seen(self.fns[id].ret, seen),
            _ => false,
        }
    }

    /// How many array, slice, option, pointer and result layers `t` is wrapped in.
    pub fn nest(&self, t: Ty) -> u32 {
        match self.kind(t) {
            Kind::Array(e, _) | Kind::Slice(e) | Kind::Option(e) | Kind::Pointer(e) | Kind::Ref(e) => 1 + self.nest(e),
            Kind::Result(a, b) => 1 + self.nest(a).max(self.nest(b)),
            _ => 0,
        }
    }

    pub fn is_ref(&self, t: Ty) -> bool {
        matches!(self.kind(t), Kind::Ref(_))
    }

    pub fn is_ptr(&self, t: Ty) -> bool {
        matches!(self.kind(t), Kind::Pointer(_))
    }

    pub fn is_num(&self, t: Ty) -> bool {
        self.is_int(t) || self.is_float(t)
    }

    /// Lives in memory and is handled by address.
    pub fn is_aggregate(&self, t: Ty) -> bool {
        matches!(self.kind(t), Kind::Struct(_) | Kind::Enum(_) | Kind::Array(..) | Kind::Slice(_) | Kind::Str | Kind::Option(_) | Kind::Result(..))
    }

    /// Owns something that must be released when it goes away.
    pub fn needs_drop(&self, t: Ty) -> bool {
        match self.kind(t) {
            Kind::Slice(_) | Kind::Str => true,
            Kind::Struct(id) => self.user_drop.contains_key(&t) || self.structs[id].fields.iter().any(|f| self.needs_drop(f.ty)),
            Kind::Enum(id) => self.enums[id].variants.iter().any(|v| v.fields.iter().any(|f| self.needs_drop(f.ty))),
            Kind::Array(e, _) | Kind::Option(e) => self.needs_drop(e),
            Kind::Result(a, b) => self.needs_drop(a) || self.needs_drop(b),
            _ => false,
        }
    }

    /// Somewhere inside is a type whose copy the compiler cannot write for itself.
    pub fn has_user_drop(&self, t: Ty) -> bool {
        self.user_drop_in(t, &mut Vec::new())
    }

    /// Every user-written `drop` that releasing a `t` could run.
    pub fn user_drops(&self, t: Ty) -> Vec<String> {
        let (mut out, mut seen) = (Vec::new(), Vec::new());
        self.collect_drops(t, &mut out, &mut seen);
        out
    }

    fn collect_drops(&self, t: Ty, out: &mut Vec<String>, seen: &mut Vec<Ty>) {
        if seen.contains(&t) {
            return;
        }
        seen.push(t);
        out.extend(self.user_drop.get(&t).cloned());
        let fields: Vec<Ty> = match self.kind(t) {
            Kind::Struct(id) => self.structs[id].fields.iter().map(|f| f.ty).collect(),
            Kind::Enum(id) => self.enums[id].variants.iter().flat_map(|v| v.fields.iter().map(|f| f.ty)).collect(),
            Kind::Array(e, _) | Kind::Option(e) | Kind::Slice(e) => vec![e],
            Kind::Result(a, b) => vec![a, b],
            _ => Vec::new(),
        };
        for f in fields {
            self.collect_drops(f, out, seen);
        }
    }

    /// `[]Tree` inside `Tree` is legal, so remember what has been looked at.
    fn user_drop_in(&self, t: Ty, seen: &mut Vec<Ty>) -> bool {
        if seen.contains(&t) {
            return false;
        }
        seen.push(t);
        match self.kind(t) {
            Kind::Struct(id) => self.user_drop.contains_key(&t) || self.structs[id].fields.iter().any(|f| self.user_drop_in(f.ty, seen)),
            Kind::Enum(id) => self.enums[id].variants.iter().any(|v| v.fields.iter().any(|f| self.user_drop_in(f.ty, seen))),
            Kind::Array(e, _) | Kind::Option(e) | Kind::Slice(e) => self.user_drop_in(e, seen),
            Kind::Result(a, b) => self.user_drop_in(a, seen) || self.user_drop_in(b, seen),
            _ => false,
        }
    }

    /// Can a value of `t` hold another value of `t`, through `[]`?
    pub fn contains_itself(&self, t: Ty) -> bool {
        self.reaches_itself(t, &mut Vec::new())
    }

    fn reaches_itself(&self, t: Ty, path: &mut Vec<Ty>) -> bool {
        if path.contains(&t) {
            return true;
        }
        let parts: Vec<Ty> = match self.kind(t) {
            Kind::Struct(id) => self.structs[id].fields.iter().map(|f| f.ty).collect(),
            Kind::Enum(id) => self.enums[id].variants.iter().flat_map(|v| v.fields.iter().map(|f| f.ty)).collect(),
            Kind::Array(e, _) | Kind::Option(e) | Kind::Slice(e) => vec![e],
            Kind::Result(a, b) => vec![a, b],
            _ => return false,
        };
        path.push(t);
        let found = parts.into_iter().any(|p| self.reaches_itself(p, path));
        path.pop();
        found
    }

    pub fn size(&self, t: Ty) -> u64 {
        self.layout(t).0
    }

    pub fn layout(&self, t: Ty) -> (u64, u64) {
        match self.kind(t) {
            Kind::Int { bits, .. } | Kind::Float { bits } => ((bits / 8) as u64, (bits / 8) as u64),
            Kind::Bool => (1, 1),
            Kind::Vec(_) => (16, 16),
            Kind::Void | Kind::StrLit | Kind::Err => (0, 1),
            Kind::Pointer(_) | Kind::Ref(_) | Kind::Fn(_) => (8, 8),
            Kind::Slice(_) | Kind::Str => (SEQ_SIZE, 8),
            Kind::Struct(id) => (self.structs[id].size, self.structs[id].align),
            Kind::Enum(id) => (self.enums[id].size, 8),
            Kind::Array(e, n) => {
                let (s, a) = self.layout(e);
                (s * n, a)
            }
            Kind::Option(p) => (TAG_SIZE + align_up(self.size(p), 8), 8),
            Kind::Result(ok, err) => (TAG_SIZE + align_up(self.size(ok).max(self.size(err)), 8), 8),
        }
    }

    pub fn name(&self, t: Ty) -> String {
        match self.kind(t) {
            Kind::Int { bits: 64, signed: true } => "int".to_string(),
            Kind::Int { bits, signed } => format!("{}{}", if signed { "i" } else { "u" }, bits),
            Kind::Float { bits } => format!("f{bits}"),
            Kind::Bool => "bool".to_string(),
            Kind::Vec(v) => v.name().to_string(),
            Kind::Void => "nothing".to_string(),
            Kind::StrLit => "string literal".to_string(),
            Kind::Err => "<error>".to_string(),
            Kind::Struct(id) => self.structs[id].name.clone(),
            Kind::Enum(id) => self.enums[id].name.clone(),
            Kind::Array(e, n) => format!("[{}]{}", n, self.name(e)),
            Kind::Pointer(p) => format!("*{}", self.name(p)),
            Kind::Slice(e) => format!("[]{}", self.name(e)),
            Kind::Str => "str".to_string(),
            Kind::Ref(p) => format!("&{}", self.name(p)),
            Kind::Option(p) => format!("?{}", self.name(p)),
            Kind::Fn(id) => {
                let f = &self.fns[id];
                let params: Vec<String> = f.params.iter().zip(&f.own).map(|(p, own)| format!("{}{}", if *own { "own " } else { "" }, self.name(*p))).collect();
                let mut out = format!("fn({})", params.join(", "));
                if f.ret != VOID {
                    out += &format!(" {}", self.name(f.ret));
                }
                if f.effects != 0 {
                    out += &format!(" {}", spell_effects(f.effects));
                }
                out
            }
            Kind::Result(ok, err) => {
                if ok == VOID {
                    format!("! {}", self.name(err))
                } else {
                    format!("{} ! {}", self.name(ok), self.name(err))
                }
            }
        }
    }
}
