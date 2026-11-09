//! Automatic tests: every function without `+io` gets hit with boundary, random and mutated
//! inputs, in the interpreter. A found failure is shrunk to a small input and explained.

use crate::cir::*;
use crate::effects;
use crate::interp::{Config, Failure, Kind as Fail, Machine, Stop};
use crate::types::{self, Kind, Ty, Types, TAG_SIZE};
use crate::why::{self, Sources};
use std::collections::HashSet;

const RUN_STEPS: u64 = 200_000;
const RUNS: u32 = 4_000;
/// All steps one function may use, and all that a file may use.
const FUNC_STEPS: u64 = 20_000_000;
const FILE_STEPS: u64 = 300_000_000;
const MAX_DEPTH: u32 = 3;
const MAX_ITEMS: u64 = 8;
const MAX_ARRAY: u64 = 256;
const SHRINK_TRIES: u32 = 3_000;
const MAX_NOTES: usize = 5;

#[derive(Clone, Debug, PartialEq)]
enum Val {
    Int(i128),
    Bool(bool),
    Float(f64),
    Str(Vec<u8>),
    Items(Vec<Val>),
    Fields(Vec<Val>),
    Variant(usize, Vec<Val>),
    None,
    Some(Box<Val>),
    Ok(Box<Val>),
    Err(Box<Val>),
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*: fixed seed, so a run can be repeated exactly.
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3)) | 1
}

fn can_make(t: &Types, ty: Ty, seen: &mut Vec<Ty>) -> bool {
    if seen.contains(&ty) {
        return true;
    }
    seen.push(ty);
    let ok = match t.kind(ty) {
        Kind::Int { .. } | Kind::Float { .. } | Kind::Bool | Kind::Str => true,
        Kind::Array(e, n) => n <= MAX_ARRAY && can_make(t, e, seen),
        Kind::Slice(e) | Kind::Ref(e) | Kind::Option(e) => can_make(t, e, seen),
        Kind::Result(a, b) => a != types::VOID && can_make(t, a, seen) && can_make(t, b, seen),
        Kind::Struct(id) => t.structs[id].fields.iter().all(|f| can_make(t, f.ty, seen)),
        Kind::Enum(id) => t.enums[id].variants.iter().all(|v| v.fields.iter().all(|f| can_make(t, f.ty, seen))),
        _ => false,
    };
    seen.pop();
    ok
}

fn int_range(bits: u8, signed: bool) -> (i128, i128) {
    if signed {
        (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
    } else {
        (0, (1i128 << bits) - 1)
    }
}

enum Outcome {
    Fine,
    Discard,
    Skip,
    Failed(Box<Failure>),
}

struct Fuzzer<'a, 'm> {
    t: &'a Types,
    mc: Machine<'m>,
    rng: Rng,
    ints: Vec<i128>,
    strs: Vec<Vec<u8>>,
    /// Steps used so far; the budget counts these, never the clock, so runs repeat exactly.
    total: u64,
}

impl Fuzzer<'_, '_> {
    fn zero(&self, ty: Ty, depth: u32) -> Val {
        match self.t.kind(ty) {
            Kind::Int { .. } => Val::Int(0),
            Kind::Bool => Val::Bool(false),
            Kind::Float { .. } => Val::Float(0.0),
            Kind::Str => Val::Str(Vec::new()),
            Kind::Slice(_) => Val::Items(Vec::new()),
            Kind::Array(e, n) => Val::Items((0..n).map(|_| self.zero(e, depth)).collect()),
            Kind::Ref(e) => self.zero(e, depth),
            Kind::Option(_) => Val::None,
            Kind::Result(a, _) => Val::Ok(Box::new(self.zero(a, depth + 1))),
            Kind::Struct(id) => Val::Fields(self.t.structs[id].fields.iter().map(|f| self.zero(f.ty, depth + 1)).collect()),
            Kind::Enum(id) => {
                // The variant with the fewest fields, so that recursion bottoms out.
                let vs = &self.t.enums[id].variants;
                let best = (0..vs.len()).min_by_key(|i| vs[*i].fields.len()).unwrap_or(0);
                Val::Variant(best, vs[best].fields.iter().map(|f| self.zero(f.ty, depth + 1)).collect())
            }
            _ => unreachable!("can_make rejected this"),
        }
    }

    /// A short list of the values most likely to break something.
    fn edges(&self, ty: Ty) -> Vec<Val> {
        match self.t.kind(ty) {
            Kind::Int { bits, signed } => {
                let (lo, hi) = int_range(bits, signed);
                let mut v = vec![0, 1, 2, hi];
                if signed {
                    v.extend([-1, lo]);
                }
                v.into_iter().map(Val::Int).collect()
            }
            Kind::Bool => vec![Val::Bool(false), Val::Bool(true)],
            Kind::Float { .. } => [0.0, 1.0, -1.0, 0.5, f64::MAX, f64::INFINITY, f64::NAN].into_iter().map(Val::Float).collect(),
            Kind::Str => [&b""[..], b"a", b"0", b" ", b"-1"].iter().map(|s| Val::Str(s.to_vec())).collect(),
            Kind::Slice(e) => {
                let zero = self.zero(e, 1);
                let mut out = vec![Val::Items(Vec::new()), Val::Items(vec![zero.clone()]), Val::Items(vec![zero.clone(), zero.clone(), zero])];
                for x in self.edges(e).into_iter().take(4) {
                    out.push(Val::Items(vec![x]));
                }
                out
            }
            Kind::Ref(e) => self.edges(e),
            Kind::Option(e) => {
                let mut out = vec![Val::None];
                out.extend(self.edges(e).into_iter().take(4).map(|x| Val::Some(Box::new(x))));
                out
            }
            Kind::Enum(id) => self.t.enums[id].variants.iter().enumerate().map(|(i, v)| Val::Variant(i, v.fields.iter().map(|f| self.zero(f.ty, 1)).collect())).collect(),
            _ => vec![self.zero(ty, 0)],
        }
    }

    fn make_int(&mut self, bits: u8, signed: bool) -> Val {
        let (lo, hi) = int_range(bits, signed);
        let v = match self.rng.below(10) {
            0..=4 => self.rng.below(17) as i128 - 8,
            5 | 6 => [lo, hi, lo + 1, hi - 1, 0, hi / 2][self.rng.below(6) as usize],
            7 | 8 if !self.ints.is_empty() => self.ints[self.rng.below(self.ints.len() as u64) as usize],
            _ => self.rng.next() as i64 as i128,
        };
        Val::Int(v.clamp(lo, hi))
    }

    fn make_str(&mut self) -> Val {
        if !self.strs.is_empty() && self.rng.below(3) == 0 {
            let i = self.rng.below(self.strs.len() as u64) as usize;
            return Val::Str(self.strs[i].clone());
        }
        let n = self.rng.below(MAX_ITEMS + 1);
        const CHARS: &[u8] = b"abcXYZ0129 -_.,\n";
        Val::Str((0..n).map(|_| CHARS[self.rng.below(CHARS.len() as u64) as usize]).collect())
    }

    fn make(&mut self, ty: Ty, depth: u32) -> Val {
        match self.t.kind(ty) {
            Kind::Int { bits, signed } => self.make_int(bits, signed),
            Kind::Bool => Val::Bool(self.rng.below(2) == 0),
            Kind::Float { .. } => match self.rng.below(5) {
                0 => [0.0, 1.0, -1.0, f64::MAX, f64::MIN_POSITIVE, f64::INFINITY, f64::NAN][self.rng.below(7) as usize].into(),
                1 => (self.rng.next() as i64 as f64).into(),
                _ => (self.rng.below(2001) as f64 / 8.0 - 125.0).into(),
            },
            Kind::Str => self.make_str(),
            Kind::Slice(e) => {
                let n = if depth >= MAX_DEPTH { 0 } else { self.rng.below(MAX_ITEMS + 1) };
                Val::Items((0..n).map(|_| self.make(e, depth + 1)).collect())
            }
            Kind::Array(e, n) => Val::Items((0..n).map(|_| self.make(e, depth + 1)).collect()),
            Kind::Ref(e) => self.make(e, depth),
            Kind::Option(e) => {
                if depth >= MAX_DEPTH || self.rng.below(3) == 0 {
                    Val::None
                } else {
                    Val::Some(Box::new(self.make(e, depth + 1)))
                }
            }
            Kind::Result(a, b) => {
                if self.rng.below(2) == 0 {
                    Val::Ok(Box::new(self.make(a, depth + 1)))
                } else {
                    Val::Err(Box::new(self.make(b, depth + 1)))
                }
            }
            Kind::Struct(id) => {
                let list: Vec<Ty> = self.t.structs[id].fields.iter().map(|f| f.ty).collect();
                Val::Fields(list.into_iter().map(|f| self.make(f, depth + 1)).collect())
            }
            Kind::Enum(id) => {
                if depth >= MAX_DEPTH {
                    return self.zero(ty, depth);
                }
                let i = self.rng.below(self.t.enums[id].variants.len() as u64) as usize;
                let list: Vec<Ty> = self.t.enums[id].variants[i].fields.iter().map(|f| f.ty).collect();
                Val::Variant(i, list.into_iter().map(|f| self.make(f, depth + 1)).collect())
            }
            _ => unreachable!("can_make rejected this"),
        }
    }

    /// A small random change somewhere inside the value.
    fn mutate(&mut self, v: &Val, ty: Ty) -> Val {
        let t = self.t;
        match (v, t.kind(ty)) {
            (Val::Int(x), Kind::Int { bits, signed }) => {
                let (lo, hi) = int_range(bits, signed);
                let bit = 1i128 << self.rng.below(bits as u64);
                let new = match self.rng.below(5) {
                    0 => *x + 1,
                    1 => *x - 1,
                    2 => *x ^ bit,
                    3 => -*x,
                    _ => return self.make_int(bits, signed),
                };
                Val::Int(new.clamp(lo, hi))
            }
            (Val::Str(s), _) => {
                let mut s = s.clone();
                match self.rng.below(3) {
                    0 if !s.is_empty() => {
                        let i = self.rng.below(s.len() as u64) as usize;
                        s.remove(i);
                    }
                    1 => {
                        let i = self.rng.below(s.len() as u64 + 1) as usize;
                        s.insert(i, b"0a -"[self.rng.below(4) as usize]);
                    }
                    _ => return self.make_str(),
                }
                Val::Str(s)
            }
            (Val::Items(items), Kind::Slice(e)) => {
                let mut items = items.clone();
                match self.rng.below(3) {
                    0 if !items.is_empty() => {
                        let i = self.rng.below(items.len() as u64) as usize;
                        items.remove(i);
                    }
                    1 => {
                        let i = self.rng.below(items.len() as u64 + 1) as usize;
                        let x = self.make(e, 1);
                        items.insert(i, x);
                    }
                    _ if !items.is_empty() => {
                        let i = self.rng.below(items.len() as u64) as usize;
                        items[i] = self.mutate(&items[i].clone(), e);
                    }
                    _ => {}
                }
                Val::Items(items)
            }
            (Val::Items(items), Kind::Array(e, _)) if !items.is_empty() => {
                let mut items = items.clone();
                let i = self.rng.below(items.len() as u64) as usize;
                items[i] = self.mutate(&items[i].clone(), e);
                Val::Items(items)
            }
            (_, Kind::Ref(e)) => self.mutate(v, e),
            (Val::Some(x), Kind::Option(e)) => Val::Some(Box::new(self.mutate(x, e))),
            (Val::Fields(fs), Kind::Struct(id)) if !fs.is_empty() => {
                let mut fs = fs.clone();
                let i = self.rng.below(fs.len() as u64) as usize;
                fs[i] = self.mutate(&fs[i].clone(), t.structs[id].fields[i].ty);
                Val::Fields(fs)
            }
            _ => self.make(ty, 1),
        }
    }

    /// Writes a value into memory at `at`.
    fn write(&mut self, v: &Val, ty: Ty, at: u64) {
        let t = self.t;
        match (v, t.kind(ty)) {
            (Val::Int(x), Kind::Int { bits, .. }) => self.mc.put(at, &(*x as u64).to_le_bytes()[..bits as usize / 8]),
            (Val::Bool(x), _) => self.mc.put(at, &[*x as u8]),
            (Val::Float(x), Kind::Float { bits: 32 }) => self.mc.put(at, &(*x as f32).to_le_bytes()),
            (Val::Float(x), _) => self.mc.put(at, &x.to_le_bytes()),
            (Val::Str(s), _) => {
                let p = if s.is_empty() { 0 } else { self.mc.input_block(s.len() as u64) };
                if !s.is_empty() {
                    self.mc.put(p, s);
                }
                self.header(at, p, s.len() as u64);
            }
            (Val::Items(items), Kind::Slice(e)) => {
                let p = if items.is_empty() { 0 } else { self.mc.input_block(items.len() as u64 * t.size(e).max(1)) };
                for (i, x) in items.iter().enumerate() {
                    self.write(x, e, p + i as u64 * t.size(e));
                }
                self.header(at, p, items.len() as u64);
            }
            (Val::Items(items), Kind::Array(e, _)) => {
                for (i, x) in items.iter().enumerate() {
                    self.write(x, e, at + i as u64 * t.size(e));
                }
            }
            (_, Kind::Ref(e)) => self.write(v, e, at),
            (Val::Fields(fs), Kind::Struct(id)) => {
                for (x, f) in fs.iter().zip(&t.structs[id].fields) {
                    self.write(x, f.ty, at + f.offset);
                }
            }
            (Val::Variant(i, fs), Kind::Enum(id)) => {
                self.mc.put(at, &(*i as u64).to_le_bytes());
                for (x, f) in fs.iter().zip(&t.enums[id].variants[*i].fields) {
                    self.write(x, f.ty, at + f.offset);
                }
            }
            (Val::None, _) => self.mc.put(at, &0u64.to_le_bytes()),
            (Val::Some(x), Kind::Option(e)) => {
                self.mc.put(at, &1u64.to_le_bytes());
                self.write(x, e, at + TAG_SIZE);
            }
            (Val::Ok(x), Kind::Result(a, _)) => {
                self.mc.put(at, &0u64.to_le_bytes());
                self.write(x, a, at + TAG_SIZE);
            }
            (Val::Err(x), Kind::Result(_, b)) => {
                self.mc.put(at, &1u64.to_le_bytes());
                self.write(x, b, at + TAG_SIZE);
            }
            _ => unreachable!("a value always matches the type it was made for"),
        }
    }

    fn header(&mut self, at: u64, p: u64, n: u64) {
        self.mc.put(at, &p.to_le_bytes());
        self.mc.put(at + 8, &n.to_le_bytes());
        self.mc.put(at + 16, &n.to_le_bytes());
    }

    /// The register value a parameter of this type receives.
    fn arg(&mut self, v: &Val, ty: Ty) -> u64 {
        let t = self.t;
        if t.is_aggregate(ty) || t.is_ref(ty) {
            let inner = match t.kind(ty) {
                Kind::Ref(e) => e,
                _ => ty,
            };
            let at = self.mc.input_block(t.size(inner).max(1));
            self.write(v, inner, at);
            return at;
        }
        match v {
            Val::Int(x) => *x as u64,
            Val::Bool(x) => *x as u64,
            Val::Float(x) if t.kind(ty) == (Kind::Float { bits: 32 }) => (*x as f32).to_bits() as u64,
            Val::Float(x) => x.to_bits(),
            _ => unreachable!("only scalars get here"),
        }
    }

    fn run(&mut self, a: &Auto, vals: &[Val]) -> Outcome {
        self.mc.reset();
        let args: Vec<u64> = vals.iter().zip(&a.params).map(|(v, p)| self.arg(v, p.1)).collect();
        if let Some(filter) = a.filter {
            let stop = self.mc.call(filter, &args);
            self.total += self.mc.steps;
            match stop {
                Stop::Done(1) => {}
                Stop::Unsupported(_) => return Outcome::Skip,
                _ => return Outcome::Discard,
            }
            self.mc.adopt_all();
        }
        let t = self.t;
        let mut full = Vec::with_capacity(args.len() + 1);
        if t.is_aggregate(a.ret) {
            full.push(self.mc.input_block(t.size(a.ret).max(1)));
        }
        full.extend(&args);
        let stop = self.mc.call(a.func, &full);
        self.total += self.mc.steps;
        match stop {
            Stop::Done(v) => {
                let (n, bytes, site) = self.mc.leaked(&[v]);
                if n > 0 {
                    let line = site;
                    let msg = format!("trap: memory leak: {n} blocks, {bytes} bytes never released\n");
                    return Outcome::Failed(Box::new(Failure { kind: Fail::Leak, msg, exit: 101, detail: String::new(), seeds: Vec::new(), line }));
                }
                Outcome::Fine
            }
            Stop::Exit(_) | Stop::Limit(_) | Stop::NeedConst(_) => Outcome::Discard,
            Stop::Unsupported(_) => Outcome::Skip,
            Stop::Fail(f) => match f.kind {
                Fail::OutOfMemory | Fail::ContractMath => Outcome::Discard,
                _ => Outcome::Failed(f),
            },
        }
    }

    fn shrink(&mut self, a: &Auto, mut vals: Vec<Val>, first: &Failure) -> Vec<Val> {
        let key = (first.kind, first.line);
        let mut tries = 0;
        loop {
            let mut better = false;
            'params: for i in 0..vals.len() {
                for c in smaller(&vals[i]) {
                    tries += 1;
                    if tries > SHRINK_TRIES {
                        return vals;
                    }
                    let mut next = vals.clone();
                    next[i] = c;
                    if let Outcome::Failed(f) = self.run(a, &next) {
                        if (f.kind, f.line) == key {
                            vals = next;
                            better = true;
                            break 'params;
                        }
                    }
                }
            }
            if !better {
                return vals;
            }
        }
    }
}

impl From<f64> for Val {
    fn from(x: f64) -> Val {
        Val::Float(x)
    }
}

/// Simpler versions of a value, most drastic first.
fn smaller(v: &Val) -> Vec<Val> {
    let mut out = Vec::new();
    match v {
        Val::Int(x) if *x != 0 => {
            out.push(Val::Int(0));
            if x.abs() > 1 {
                out.push(Val::Int(x / 2));
                // Steps of shrinking size close in on the smallest failing number quickly.
                let mut step = x.abs() / 2;
                while step > 0 {
                    out.push(Val::Int(x - x.signum() * step));
                    step /= 2;
                }
            }
        }
        Val::Bool(true) => out.push(Val::Bool(false)),
        Val::Float(x) if *x != 0.0 => {
            out.push(Val::Float(0.0));
            if x.is_finite() && x.fract() != 0.0 {
                out.push(Val::Float(x.trunc()));
            }
        }
        Val::Str(s) if !s.is_empty() => {
            out.push(Val::Str(Vec::new()));
            for i in 0..s.len().min(16) {
                let mut t = s.clone();
                t.remove(i);
                out.push(Val::Str(t));
            }
        }
        Val::Items(items) if !items.is_empty() => {
            out.push(Val::Items(Vec::new()));
            for i in 0..items.len().min(16) {
                let mut t = items.clone();
                t.remove(i);
                out.push(Val::Items(t));
                for c in smaller(&items[i]) {
                    let mut t = items.clone();
                    t[i] = c;
                    out.push(Val::Items(t));
                }
            }
        }
        Val::Fields(fs) => {
            for i in 0..fs.len() {
                for c in smaller(&fs[i]) {
                    let mut t = fs.clone();
                    t[i] = c;
                    out.push(Val::Fields(t));
                }
            }
        }
        Val::Variant(k, fs) => {
            for i in 0..fs.len() {
                for c in smaller(&fs[i]) {
                    let mut t = fs.clone();
                    t[i] = c;
                    out.push(Val::Variant(*k, t));
                }
            }
        }
        Val::Some(x) => {
            out.push(Val::None);
            out.extend(smaller(x).into_iter().map(|c| Val::Some(Box::new(c))));
        }
        _ => {}
    }
    out
}

/// A value as it would be written in Claw.
fn pretty(v: &Val, ty: Ty, t: &Types) -> String {
    let list = |items: &[Val], tys: &mut dyn Iterator<Item = Ty>| items.iter().zip(tys).map(|(x, e)| pretty(x, e, t)).collect::<Vec<_>>().join(", ");
    match (v, t.kind(ty)) {
        (_, Kind::Ref(e)) => pretty(v, e, t),
        (Val::Int(x), _) => x.to_string(),
        (Val::Bool(x), _) => x.to_string(),
        (Val::Float(x), _) => {
            if x.is_nan() {
                "NaN".to_string()
            } else if x.is_infinite() {
                if *x > 0.0 { "inf" } else { "-inf" }.to_string()
            } else {
                String::from_utf8_lossy(&crate::interp::format_float(*x, 64)).into_owned()
            }
        }
        (Val::Str(s), _) => format!("{:?}", String::from_utf8_lossy(s)),
        (Val::Items(items), Kind::Slice(e) | Kind::Array(e, _)) => format!("[{}]", list(items, &mut std::iter::repeat(e))),
        (Val::Fields(fs), Kind::Struct(id)) => format!("{}{{{}}}", t.structs[id].name, list(fs, &mut t.structs[id].fields.iter().map(|f| f.ty))),
        (Val::Variant(i, fs), Kind::Enum(id)) => {
            let var = &t.enums[id].variants[*i];
            if fs.is_empty() {
                var.name.clone()
            } else {
                format!("{}{{{}}}", var.name, list(fs, &mut var.fields.iter().map(|f| f.ty)))
            }
        }
        (Val::None, _) => "none".to_string(),
        (Val::Some(x), Kind::Option(e)) => pretty(x, e, t),
        (Val::Ok(x), Kind::Result(a, _)) => pretty(x, a, t),
        (Val::Err(x), Kind::Result(_, b)) => format!("fail({})", pretty(x, b, t)),
        _ => "?".to_string(),
    }
}

fn call_text(a: &Auto, vals: &[Val], t: &Types) -> String {
    let args: Vec<String> = vals.iter().zip(&a.params).map(|(v, p)| pretty(v, p.1, t)).collect();
    format!("{}({})", a.name, args.join(", "))
}

/// Numbers and strings the function itself mentions make good inputs.
fn dictionary(m: &Module, start: FuncId) -> (Vec<i128>, Vec<Vec<u8>>) {
    let (mut ints, mut strs) = (HashSet::new(), HashSet::new());
    let mut seen = HashSet::new();
    let mut work = vec![start];
    while let Some(f) = work.pop() {
        if !seen.insert(f) || seen.len() > 64 {
            continue;
        }
        for b in &m.funcs[f].blocks {
            for i in &b.insts {
                match i {
                    Inst::Const { v, .. } if v.abs_diff(0) < (1 << 40) => {
                        for d in -1..=1 {
                            ints.insert(*v as i128 + d);
                        }
                    }
                    Inst::StrAddr { text, .. } if !text.is_empty() && text.len() < 32 && strs.len() < 32 => {
                        strs.insert(text.clone().into_bytes());
                    }
                    Inst::Call { f, .. } | Inst::FuncAddr { f, .. } => work.push(*f),
                    _ => {}
                }
            }
        }
    }
    let mut a: Vec<i128> = ints.into_iter().collect();
    a.sort_unstable();
    a.truncate(256);
    let mut b: Vec<Vec<u8>> = strs.into_iter().collect();
    b.sort_unstable();
    (a, b)
}

pub struct Summary {
    pub functions: usize,
    pub inputs: u64,
    pub failed: usize,
    pub skipped: usize,
    pub lines: Vec<String>,
}

/// What repeating the same call with tracing on says about the failure.
fn explain(m: &Module, files: &[String], src: &Sources, a: &Auto, vals: &[Val], t: &Types, first: &Failure) -> Vec<String> {
    let cfg = Config { steps: RUN_STEPS, memory: 1 << 26, depth: 2000, io: false, stdout: false, trace: true, cover: false };
    let mut fz = Fuzzer { t, mc: Machine::new(m, files, cfg), rng: Rng(1), ints: Vec::new(), strs: Vec::new(), total: 0 };
    let input = call_text(a, vals, t);
    match fz.run(a, vals) {
        Outcome::Failed(f) => why::report(src, &fz.mc, &f, Some(&input)),
        _ => why::report(src, &fz.mc, first, Some(&input)),
    }
}

pub fn run(m: &Module, files: &[String], src: &Sources, t: &Types, effects: &effects::Report) -> Summary {
    let mut out = Summary { functions: 0, inputs: 0, failed: 0, skipped: 0, lines: Vec::new() };
    let mut notes: Vec<String> = Vec::new();
    let mut spent = 0u64;
    for a in m.autos.iter().filter(|a| !effects.has_io(&a.full)) {
        if !a.params.iter().all(|p| can_make(t, p.1, &mut Vec::new())) {
            out.skipped += 1;
            continue;
        }
        if spent > FILE_STEPS {
            out.skipped += 1;
            continue;
        }
        let cfg = Config { steps: RUN_STEPS, memory: 1 << 26, depth: 2000, io: false, stdout: false, trace: false, cover: true };
        let (ints, strs) = dictionary(m, a.func);
        let mut fz = Fuzzer { t, mc: Machine::new(m, files, cfg), rng: Rng(hash(&a.full)), ints, strs, total: 0 };
        let report = fuzz_one(&mut fz, a);
        spent += fz.total;
        out.inputs += report.inputs;
        out.functions += 1;
        match report.result {
            Found::Nothing => {}
            Found::Skip => {
                out.skipped += 1;
                out.functions -= 1;
            }
            Found::Overflow(vals) => notes.push(format!("note: {} overflows for {}; add a `need` if that input is not meant to work", a.name, call_text(a, &vals, t))),
            Found::Bug(vals, f) => {
                out.failed += 1;
                let head = f.msg.lines().next().unwrap_or("").trim_start_matches("trap: ");
                out.lines.push(format!("FAIL {}: {head}", a.name));
                out.lines.extend(explain(m, files, src, a, &vals, t, &f).into_iter().map(|l| format!("  {l}")));
            }
        }
    }
    let more = notes.len().saturating_sub(MAX_NOTES);
    out.lines.extend(notes.into_iter().take(MAX_NOTES));
    if more > 0 {
        out.lines.push(format!("note: {more} more functions can overflow"));
    }
    out
}

enum Found {
    Nothing,
    Skip,
    Overflow(Vec<Val>),
    Bug(Vec<Val>, Box<Failure>),
}

struct FuncReport {
    inputs: u64,
    result: Found,
}

fn fuzz_one(fz: &mut Fuzzer, a: &Auto) -> FuncReport {
    let types: Vec<Ty> = a.params.iter().map(|p| p.1).collect();
    let mut seen = vec![0u8; 1 << 16];
    let mut corpus: Vec<Vec<Val>> = Vec::new();
    let mut inputs = 0u64;
    let mut overflow: Option<(Vec<Val>, Box<Failure>)> = None;

    // Boundary values first: all zeros, then one parameter at a time, then pairs.
    let zeros: Vec<Val> = types.iter().map(|t| fz.zero(*t, 0)).collect();
    let edges: Vec<Vec<Val>> = types.iter().map(|t| fz.edges(*t)).collect();
    let mut plan = vec![zeros.clone()];
    for (i, list) in edges.iter().enumerate() {
        for v in list {
            let mut c = zeros.clone();
            c[i] = v.clone();
            plan.push(c);
        }
    }
    for i in 0..types.len() {
        for j in i + 1..types.len() {
            for x in &edges[i] {
                for y in &edges[j] {
                    if plan.len() < 600 {
                        let mut c = zeros.clone();
                        c[i] = x.clone();
                        c[j] = y.clone();
                        plan.push(c);
                    }
                }
            }
        }
    }

    let mut next_planned = 0;
    let start = fz.total;
    let mut runs = 0;
    while runs < RUNS && fz.total - start < FUNC_STEPS {
        runs += 1;
        let vals: Vec<Val> = if next_planned < plan.len() {
            next_planned += 1;
            plan[next_planned - 1].clone()
        } else if !corpus.is_empty() && fz.rng.below(2) == 0 {
            let base = corpus[fz.rng.below(corpus.len() as u64) as usize].clone();
            let mut c = base;
            if !c.is_empty() {
                let k = fz.rng.below(c.len() as u64) as usize;
                c[k] = fz.mutate(&c[k].clone(), types[k]);
            }
            c
        } else {
            types.iter().map(|t| fz.make(*t, 0)).collect()
        };
        inputs += 1;
        fz.mc.cover.iter_mut().for_each(|x| *x = 0);
        let outcome = fz.run(a, &vals);
        let mut grew = false;
        for (s, c) in seen.iter_mut().zip(&fz.mc.cover) {
            if *c != 0 && *s == 0 {
                *s = 1;
                grew = true;
            }
        }
        match outcome {
            Outcome::Fine => {
                if grew && corpus.len() < 256 {
                    corpus.push(vals);
                }
            }
            Outcome::Discard => {}
            Outcome::Skip => return FuncReport { inputs, result: Found::Skip },
            Outcome::Failed(f) if f.kind == Fail::Overflow => {
                if overflow.is_none() {
                    overflow = Some((vals, f));
                }
            }
            Outcome::Failed(f) => {
                let small = fz.shrink(a, vals, &f);
                return FuncReport { inputs, result: Found::Bug(small, f) };
            }
        }
    }
    match overflow {
        Some((vals, f)) => {
            let small = fz.shrink(a, vals, &f);
            FuncReport { inputs, result: Found::Overflow(small) }
        }
        None => FuncReport { inputs, result: Found::Nothing },
    }
}
