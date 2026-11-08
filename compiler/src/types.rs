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
    Option(Ty),
    Result(Ty, Ty),
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
}

pub fn align_up(v: u64, a: u64) -> u64 {
    (v + a - 1) & !(a - 1)
}

impl Types {
    pub fn new() -> Types {
        let mut t = Types { kinds: Vec::new(), map: HashMap::new(), structs: Vec::new(), enums: Vec::new() };
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

    pub fn kind(&self, t: Ty) -> Kind {
        self.kinds[t.0 as usize]
    }

    pub fn is_int(&self, t: Ty) -> bool {
        matches!(self.kind(t), Kind::Int { .. })
    }

    pub fn is_float(&self, t: Ty) -> bool {
        matches!(self.kind(t), Kind::Float { .. })
    }

    pub fn is_ptr(&self, t: Ty) -> bool {
        matches!(self.kind(t), Kind::Pointer(_))
    }

    pub fn is_num(&self, t: Ty) -> bool {
        self.is_int(t) || self.is_float(t)
    }

    /// Lives in memory and is handled by address.
    pub fn is_aggregate(&self, t: Ty) -> bool {
        matches!(self.kind(t), Kind::Struct(_) | Kind::Enum(_) | Kind::Array(..) | Kind::Option(_) | Kind::Result(..))
    }

    pub fn size(&self, t: Ty) -> u64 {
        self.layout(t).0
    }

    pub fn layout(&self, t: Ty) -> (u64, u64) {
        match self.kind(t) {
            Kind::Int { bits, .. } | Kind::Float { bits } => ((bits / 8) as u64, (bits / 8) as u64),
            Kind::Bool => (1, 1),
            Kind::Void | Kind::StrLit | Kind::Err => (0, 1),
            Kind::Pointer(_) => (8, 8),
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
            Kind::Void => "nothing".to_string(),
            Kind::StrLit => "string literal".to_string(),
            Kind::Err => "<error>".to_string(),
            Kind::Struct(id) => self.structs[id].name.clone(),
            Kind::Enum(id) => self.enums[id].name.clone(),
            Kind::Array(e, n) => format!("[{}]{}", n, self.name(e)),
            Kind::Pointer(p) => format!("*{}", self.name(p)),
            Kind::Option(p) => format!("?{}", self.name(p)),
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
