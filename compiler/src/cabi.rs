//! C structs passed by value: which registers carry them, by the C rules of each target.
//!
//! Only structs of numbers, bool, pointers, and arrays and structs of those can go to C by value. A struct of at
//! most 16 bytes travels in registers; a bigger one only comes back from C, through memory the caller provides.
//!
//! arm64 (AAPCS64): one to four floats of the same kind (a "homogeneous float aggregate") go in float registers,
//! one per member; anything else of at most 16 bytes goes in one or two integer registers, 8 bytes each. A bigger
//! result is written where `x8` points.
//!
//! x86-64 (System V): each 8 bytes is one register: a float register if every field in it is a float, an
//! integer register otherwise. A bigger result is written where a hidden first argument points.

use crate::types::{Kind, Ty, Types};

/// One register's worth of a struct: `float` is `Some(32)` for an f32 member (arm64 only), `Some(64)` for 8 bytes
/// in a float register, `None` for 8 bytes in an integer register.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    pub float: Option<u8>,
    pub off: u64,
}

pub enum Pass {
    Regs(Vec<Piece>),
    /// Over 16 bytes.
    Memory,
}

/// Size and alignment as C lays the type out; `None` when C has no such type.
pub fn layout(types: &Types, t: Ty) -> Option<(u64, u64)> {
    Some(match types.kind(t) {
        Kind::Int { bits, .. } => (bits as u64 / 8, bits as u64 / 8),
        Kind::Float { bits } => (bits as u64 / 8, bits as u64 / 8),
        Kind::Bool => (1, 1),
        Kind::Pointer(_) => (8, 8),
        Kind::Array(e, n) => {
            let (s, a) = layout(types, e)?;
            (s * n, a)
        }
        Kind::Struct(id) => {
            let (mut size, mut align) = (0u64, 1u64);
            for f in &types.structs[id].fields {
                let (fs, fa) = layout(types, f.ty)?;
                size = size.div_ceil(fa) * fa + fs;
                align = align.max(fa);
            }
            (size.div_ceil(align) * align, align)
        }
        _ => return None,
    })
}

/// Every number in the struct with its offset: `Some(bits)` for floats, `None` for integers, bool and pointers.
fn leaves(types: &Types, t: Ty, base: u64, out: &mut Vec<(u64, Option<u8>)>) {
    match types.kind(t) {
        Kind::Float { bits } => out.push((base, Some(bits))),
        Kind::Array(e, n) => {
            let size = layout(types, e).map_or(0, |l| l.0);
            for i in 0..n {
                leaves(types, e, base + i * size, out);
            }
        }
        Kind::Struct(id) => {
            let mut off = 0u64;
            for f in &types.structs[id].fields {
                let Some((fs, fa)) = layout(types, f.ty) else { return };
                off = off.div_ceil(fa) * fa;
                leaves(types, f.ty, base + off, out);
                off += fs;
            }
        }
        _ => out.push((base, None)),
    }
}

/// Is `t` a struct that C can take or give by value?
pub fn is_c_struct(types: &Types, t: Ty) -> bool {
    matches!(types.kind(t), Kind::Struct(_)) && layout(types, t).is_some()
}

/// How a C struct travels on the target.
pub fn classify(types: &Types, t: Ty, x86: bool) -> Pass {
    let (size, _) = layout(types, t).unwrap_or((0, 1));
    let mut ls = Vec::new();
    leaves(types, t, 0, &mut ls);
    if !x86 {
        if let Some(Some(bits)) = ls.first().map(|l| l.1) {
            if ls.len() <= 4 && ls.iter().all(|l| l.1 == Some(bits)) {
                return Pass::Regs(ls.iter().map(|l| Piece { float: Some(bits), off: l.0 }).collect());
            }
        }
    }
    if size > 16 {
        return Pass::Memory;
    }
    let words = size.div_ceil(8).max(1);
    Pass::Regs(
        (0..words)
            .map(|w| {
                let inside: Vec<&(u64, Option<u8>)> = ls.iter().filter(|l| l.0 / 8 == w).collect();
                let all_float = x86 && !inside.is_empty() && inside.iter().all(|l| l.1.is_some());
                Piece { float: if all_float { Some(64) } else { None }, off: w * 8 }
            })
            .collect(),
    )
}

/// Integer and float registers the arguments need on one target, counting each struct by its pieces.
fn registers(types: &Types, params: &[Ty], ret: Ty, x86: bool) -> (usize, usize) {
    let (mut ints, mut floats) = (0, 0);
    if is_c_struct(types, ret) && x86 && matches!(classify(types, ret, true), Pass::Memory) {
        ints += 1;
    }
    for &p in params {
        if is_c_struct(types, p) {
            if let Pass::Regs(pieces) = classify(types, p, x86) {
                for q in pieces {
                    if q.float.is_some() {
                        floats += 1;
                    } else {
                        ints += 1;
                    }
                }
            }
        } else if matches!(types.kind(p), Kind::Float { .. }) {
            floats += 1;
        } else {
            ints += 1;
        }
    }
    (ints, floats)
}

/// Why this C signature cannot be called, if it cannot: a struct argument over 16 bytes, or more values than the
/// registers hold once structs are split (on either target, so a program builds for both or for neither).
pub fn problem(types: &Types, params: &[Ty], ret: Ty) -> Option<String> {
    for &p in params {
        if is_c_struct(types, p) && layout(types, p).is_some_and(|l| l.0 > 16) {
            return Some("a struct over 16 bytes cannot go to C by value; pass a pointer to it".to_string());
        }
    }
    if !params.iter().chain(std::iter::once(&ret)).any(|t| is_c_struct(types, *t)) {
        return None;
    }
    for (x86, ints_max) in [(false, 8), (true, 6)] {
        let (ints, floats) = registers(types, params, ret, x86);
        if ints > ints_max || floats > 8 {
            return Some("with its structs split into registers, this call needs more registers than C has for arguments; pass pointers instead".to_string());
        }
    }
    None
}
