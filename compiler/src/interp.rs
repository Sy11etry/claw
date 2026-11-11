//! Runs CIR directly, in a memory of its own where every block is separate.
//! Compile-time `const`s, the fuzzer and `claw why` all stand on this. With tracing on,
//! each value remembers the last event that made it, so a failure can be walked backwards.

use crate::cir::*;
use crate::types::Vk;
use std::collections::{HashMap, VecDeque};
use std::ffi::CString;

/// Addresses are (block number + 1) << 32 | offset, so a stray pointer cannot reach another block.
const SHIFT: u32 = 32;
/// How long freed blocks stay unusable, to catch late use of them.
const QUARANTINE: usize = 65_536;
const CAPTURE_LIMIT: usize = 1 << 16;
/// Events kept for `why`; older causes are forgotten.
const TRACE_EVENTS: usize = 1 << 23;
const TRACE_NOTES: usize = 1 << 20;
const NONE: u32 = u32::MAX;
/// The runtime's words, and C's `errno`, live in one block.
const ERRNO_AT: u64 = 64;
/// A function as a value: this bit, and the function's number.
const FN_TAG: u64 = 1 << 63;

extern "C" {
    fn snprintf(buf: *mut u8, n: usize, fmt: *const u8, ...) -> i32;
    fn open(path: *const std::ffi::c_char, flags: i32, ...) -> i32;
    fn close(fd: i32) -> i32;
    fn read(fd: i32, p: *mut u8, n: usize) -> isize;
    fn write(fd: i32, p: *const u8, n: usize) -> isize;
    fn unlink(path: *const std::ffi::c_char) -> i32;
    fn __error() -> *mut i32;
    fn toupper(c: i32) -> i32;
    fn tolower(c: i32) -> i32;
    fn isdigit(c: i32) -> i32;
    fn isalpha(c: i32) -> i32;
    fn isspace(c: i32) -> i32;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Heap,
    Stack,
    Static,
    /// The runtime's own words: writable, but nobody allocated them.
    Runtime,
    Freed,
}

struct Region {
    data: Vec<u8>,
    /// With tracing: the event that last wrote each byte.
    shadow: Vec<u32>,
    state: State,
    /// Made by whoever set up this run, not by the program.
    input: bool,
    made: u32,
    freed: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Overflow,
    DivZero,
    Shift,
    Bounds,
    Convert,
    AllocSize,
    OutOfMemory,
    ContractMath,
    Memory,
    Leak,
    Need,
    Sure,
    /// `?` reached `main`, or the message-only traps.
    Other,
}

impl Kind {
    fn text(self) -> &'static str {
        match self {
            Kind::Overflow => "integer overflow",
            Kind::DivZero => "division by zero",
            Kind::Shift => "shift out of range",
            Kind::Bounds => "index out of bounds",
            Kind::Convert => "value does not fit the target type",
            Kind::AllocSize => "invalid allocation size",
            Kind::OutOfMemory => "out of memory",
            Kind::ContractMath => "a number in a contract is too large even for exact arithmetic",
            _ => "failure",
        }
    }
}

#[derive(Debug)]
pub struct Failure {
    pub kind: Kind,
    /// As the compiled program would say it; contracts add lines with values.
    pub msg: String,
    pub exit: i32,
    /// The values that made it fail, for a report.
    pub detail: String,
    /// Events that produced the bad values.
    pub seeds: Vec<u32>,
    /// `(file << 22) | line` of the source line running.
    pub line: u32,
}

#[derive(Debug)]
pub enum Stop {
    /// The function returned this.
    Done(u64),
    /// The program called C's `exit`.
    Exit(i32),
    Fail(Box<Failure>),
    /// Ran out of steps, stack or memory: no verdict.
    Limit(&'static str),
    /// Needs something only the real machine has.
    Unsupported(String),
    /// A `const` that has not been worked out yet.
    NeedConst(usize),
}

#[derive(Clone)]
pub struct Config {
    pub steps: u64,
    pub memory: u64,
    pub depth: usize,
    pub io: bool,
    /// Where `print` goes: to the real stdout, or nowhere.
    pub stdout: bool,
    pub trace: bool,
    pub cover: bool,
}

struct Frame {
    func: FuncId,
    block: BlockId,
    ip: usize,
    base: usize,
    obj_base: usize,
    ret: Option<VReg>,
    line: u32,
    ctrl_base: usize,
    /// The branch that decided this call happens at all.
    ctrl_from: u32,
}

#[derive(Clone, Copy, Default)]
struct Event {
    line: u32,
    a: u32,
    b: u32,
    ctrl: u32,
    note: u32,
}

struct NoteRec<'m> {
    event: u32,
    name: &'m str,
    value: u64,
    show: Show,
}

struct Branch {
    event: u32,
    /// Where the two sides meet again.
    join: u32,
    block: u32,
}

#[derive(Default)]
struct Trace<'m> {
    events: Vec<Event>,
    next: u32,
    notes: Vec<NoteRec<'m>>,
    note_count: u32,
    regs: Vec<u32>,
    branches: Vec<Branch>,
    joins: HashMap<FuncId, Vec<u32>>,
}

impl<'m> Trace<'m> {
    fn add(&mut self, line: u32, a: u32, b: u32, ctrl: u32) -> u32 {
        self.next += 1;
        let at = (self.next as usize - 1) & (TRACE_EVENTS - 1);
        let e = Event { line, a, b, ctrl, note: 0 };
        if at == self.events.len() {
            self.events.push(e);
        } else {
            self.events[at] = e;
        }
        self.next
    }

    /// Keeps the value a variable took, and links it to its event.
    fn add_note(&mut self, rec: NoteRec<'m>) {
        self.note_count += 1;
        let at = (self.note_count as usize - 1) & (TRACE_NOTES - 1);
        let event = (rec.event as usize - 1) & (TRACE_EVENTS - 1);
        if at == self.notes.len() {
            self.notes.push(rec);
        } else {
            self.notes[at] = rec;
        }
        self.events[event].note = self.note_count;
    }

    fn note_of(&self, e: &Event, id: u32) -> Option<&NoteRec<'m>> {
        if e.note == 0 {
            return None;
        }
        self.notes.get((e.note as usize - 1) & (TRACE_NOTES - 1)).filter(|n| n.event == id)
    }

    fn get(&self, id: u32) -> Option<&Event> {
        if id == 0 || self.next - id >= TRACE_EVENTS as u32 {
            return None;
        }
        self.events.get((id as usize - 1) & (TRACE_EVENTS - 1))
    }
}

/// One source line that mattered, for a report.
pub struct Cause {
    pub line: u32,
    /// `x = 3`, for the variables this line set on the way to the failure.
    pub values: Vec<String>,
}

pub struct Machine<'m> {
    m: &'m Module,
    files: &'m [String],
    pub cfg: Config,
    regions: Vec<Region>,
    statics: usize,
    static_at: HashMap<*const Inst, u64>,
    state_block: u64,
    free_stack: Vec<u32>,
    quarantine: VecDeque<u32>,
    frames: Vec<Frame>,
    regs: Vec<u64>,
    /// The upper 64 bits of 128-bit vector values, parallel to `regs`.
    hi: Vec<u64>,
    objs: Vec<u64>,
    /// Bytes in live heap blocks.
    used: u64,
    pub steps: u64,
    buf: Vec<u8>,
    capturing: bool,
    /// What has gone to stderr.
    pub err: Vec<u8>,
    trace: Trace<'m>,
    err_start: usize,
    pub cover: Vec<u8>,
    pub consts: Vec<Option<crate::lower::ConstData>>,
    const_at: HashMap<usize, u64>,
}

fn sext(v: u64, bits: u8) -> u64 {
    let s = 64 - bits as u32;
    (((v << s) as i64) >> s) as u64
}

/// After an operation on 64-bit values: narrow signed types trap, unsigned ones wrap.
fn normalize(x: u64, num: Num) -> Result<u64, Kind> {
    if num.signed {
        if num.bits < 64 && num.checked && sext(x, num.bits) != x {
            return Err(Kind::Overflow);
        }
        Ok(x)
    } else if num.bits < 64 {
        Ok(x & ((1u64 << num.bits) - 1))
    } else {
        Ok(x)
    }
}

fn arith(op: Arith, a: u64, b: u64, num: Num) -> Result<u64, Kind> {
    let wide = num.signed && num.bits == 64 && num.checked;
    let (sa, sb) = (a as i64, b as i64);
    match op {
        Arith::Add if wide => sa.checked_add(sb).map(|v| v as u64).ok_or(Kind::Overflow),
        Arith::Sub if wide => sa.checked_sub(sb).map(|v| v as u64).ok_or(Kind::Overflow),
        Arith::Mul if wide => sa.checked_mul(sb).map(|v| v as u64).ok_or(Kind::Overflow),
        Arith::Add => normalize(a.wrapping_add(b), num),
        Arith::Sub => normalize(a.wrapping_sub(b), num),
        Arith::Mul => normalize(a.wrapping_mul(b), num),
        Arith::Div => {
            if num.checked && b == 0 {
                return Err(Kind::DivZero);
            }
            if num.signed {
                if num.bits == 64 && num.checked && sb == -1 && sa == i64::MIN {
                    return Err(Kind::Overflow);
                }
                normalize(if b == 0 { 0 } else { sa.wrapping_div(sb) as u64 }, num)
            } else {
                Ok(a.checked_div(b).unwrap_or(0))
            }
        }
        Arith::Rem => {
            if num.checked && b == 0 {
                return Err(Kind::DivZero);
            }
            if b == 0 {
                Ok(a)
            } else if num.signed {
                Ok(sa.wrapping_rem(sb) as u64)
            } else {
                Ok(a % b)
            }
        }
        Arith::And => Ok(a & b),
        Arith::Or => Ok(a | b),
        Arith::Xor => Ok(a ^ b),
        Arith::Shl => {
            if num.checked && b >= num.bits as u64 {
                return Err(Kind::Shift);
            }
            let x = a << (b & 63);
            if wide && ((x as i64) >> (b & 63)) as u64 != a {
                return Err(Kind::Overflow);
            }
            if wide {
                Ok(x)
            } else {
                normalize(x, num)
            }
        }
        Arith::Shr => {
            if num.checked && b >= num.bits as u64 {
                return Err(Kind::Shift);
            }
            Ok(if num.signed { (sa >> (b & 63)) as u64 } else { a >> (b & 63) })
        }
    }
}

fn compare<T: PartialOrd>(cc: Cc, a: T, b: T) -> u64 {
    (match cc {
        Cc::Eq => a == b,
        Cc::Ne => a != b,
        Cc::Lt => a < b,
        Cc::Le => a <= b,
        Cc::Gt => a > b,
        Cc::Ge => a >= b,
    }) as u64
}

/// A vector as its four 32-bit or two 64-bit lanes, low lane first.
fn lanes32(v: (u64, u64)) -> [u32; 4] {
    [v.0 as u32, (v.0 >> 32) as u32, v.1 as u32, (v.1 >> 32) as u32]
}

fn from_lanes32(l: [u32; 4]) -> (u64, u64) {
    (l[0] as u64 | (l[1] as u64) << 32, l[2] as u64 | (l[3] as u64) << 32)
}

fn vec_bin(op: FArith, ty: Vk, a: (u64, u64), b: (u64, u64)) -> (u64, u64) {
    match ty {
        Vk::F64x2 => (float_bin(op, a.0, b.0, 64), float_bin(op, a.1, b.1, 64)),
        Vk::F32x4 => {
            let (x, y) = (lanes32(a), lanes32(b));
            from_lanes32(std::array::from_fn(|i| float_bin(op, x[i] as u64, y[i] as u64, 32) as u32))
        }
        Vk::I32x4 => {
            let (x, y) = (lanes32(a), lanes32(b));
            from_lanes32(std::array::from_fn(|i| match op {
                FArith::Add => x[i].wrapping_add(y[i]),
                FArith::Sub => x[i].wrapping_sub(y[i]),
                _ => x[i].wrapping_mul(y[i]),
            }))
        }
    }
}

fn vec_fma(ty: Vk, a: (u64, u64), b: (u64, u64), c: (u64, u64)) -> (u64, u64) {
    let f64s = |x: u64, y: u64, z: u64| f64::from_bits(x).mul_add(f64::from_bits(y), f64::from_bits(z)).to_bits();
    match ty {
        Vk::F64x2 => (f64s(a.0, b.0, c.0), f64s(a.1, b.1, c.1)),
        _ => {
            let (x, y, z) = (lanes32(a), lanes32(b), lanes32(c));
            from_lanes32(std::array::from_fn(|i| f32::from_bits(x[i]).mul_add(f32::from_bits(y[i]), f32::from_bits(z[i])).to_bits()))
        }
    }
}

fn vec_neg(ty: Vk, a: (u64, u64)) -> (u64, u64) {
    match ty {
        Vk::F64x2 => (a.0 ^ (1 << 63), a.1 ^ (1 << 63)),
        Vk::F32x4 => {
            let x = lanes32(a);
            from_lanes32(std::array::from_fn(|i| x[i] ^ 0x8000_0000))
        }
        Vk::I32x4 => {
            let x = lanes32(a);
            from_lanes32(std::array::from_fn(|i| (x[i] as i32).wrapping_neg() as u32))
        }
    }
}

fn float_bin(op: FArith, a: u64, b: u64, bits: u8) -> u64 {
    fn go<T: std::ops::Add<Output = T> + std::ops::Sub<Output = T> + std::ops::Mul<Output = T> + std::ops::Div<Output = T>>(op: FArith, x: T, y: T) -> T {
        match op {
            FArith::Add => x + y,
            FArith::Sub => x - y,
            FArith::Mul => x * y,
            FArith::Div => x / y,
        }
    }
    if bits == 32 {
        go(op, f32::from_bits(a as u32), f32::from_bits(b as u32)).to_bits() as u64
    } else {
        go(op, f64::from_bits(a), f64::from_bits(b)).to_bits()
    }
}

fn int_cast(s: u64, from: Num, to: Num) -> Result<u64, Kind> {
    if to.signed && !to.checked {
        return Ok(s);
    }
    if to.signed {
        if !from.signed && from.bits == 64 && (s as i64) < 0 {
            return Err(Kind::Convert);
        }
        if to.bits < 64 && sext(s, to.bits) != s {
            return Err(Kind::Convert);
        }
        return Ok(s);
    }
    Ok(if to.bits < 64 { s & ((1u64 << to.bits) - 1) } else { s })
}

fn float_to_int(x: f64, to: Num) -> Result<u64, Kind> {
    if to.checked {
        let bits = to.bits as i32;
        let (hi, lo, inclusive) = if to.signed {
            let hi = 2f64.powi(bits - 1);
            if bits == 64 {
                (hi, -hi, true)
            } else {
                (hi, -hi - 1.0, false)
            }
        } else {
            (2f64.powi(bits), -1.0, false)
        };
        if x.is_nan() || x >= hi || if inclusive { x < lo } else { x <= lo } {
            return Err(Kind::Convert);
        }
    }
    Ok(if to.signed { x as i64 as u64 } else { x as u64 })
}

/// As the compiled program prints floats.
pub fn format_float(x: f64, bits: u8) -> Vec<u8> {
    let mut out = [0u8; 64];
    let fmt: &[u8] = if bits == 64 { b"%.15g\0" } else { b"%.7g\0" };
    // SAFETY: the buffer is 64 bytes, snprintf is told so, and the format takes one double.
    let n = unsafe { snprintf(out.as_mut_ptr(), 64, fmt.as_ptr(), x) };
    out[..(n.clamp(0, 63)) as usize].to_vec()
}

/// For each block, the block where the paths leaving it meet again. Only paths that can
/// return count: a block that stops the program does not make everything after a branch "depend" on it.
fn joins_of(f: &Func) -> Vec<u32> {
    let n = f.blocks.len();
    let exit = n;
    let succs = |b: usize| -> Vec<usize> {
        match &f.blocks[b].term {
            Term::Jmp(t) => vec![*t],
            Term::Br { t, f: e, .. } => vec![*t, *e],
            Term::Ret(_) => vec![exit],
            Term::Unreachable => Vec::new(),
        }
    };
    let mut preds = vec![Vec::new(); n + 1];
    let mut forward = vec![Vec::new(); n + 1];
    for b in 0..n {
        for s in succs(b) {
            preds[s].push(b);
            forward[b].push(s);
        }
    }
    // Reverse postorder of the graph with every edge flipped, from the exit.
    let mut seen = vec![false; n + 1];
    let mut order = Vec::with_capacity(n + 1);
    let mut stack = vec![(exit, 0usize)];
    seen[exit] = true;
    while let Some((b, i)) = stack.last_mut() {
        if let Some(&p) = preds[*b].get(*i) {
            *i += 1;
            if !seen[p] {
                seen[p] = true;
                stack.push((p, 0));
            }
        } else {
            order.push(*b);
            stack.pop();
        }
    }
    order.reverse();
    let mut pos = vec![usize::MAX; n + 1];
    for (i, b) in order.iter().enumerate() {
        pos[*b] = i;
    }
    let mut idom = vec![usize::MAX; n + 1];
    idom[exit] = exit;
    let mut changed = true;
    while changed {
        changed = false;
        for &b in order.iter().skip(1) {
            let mut new = usize::MAX;
            for &s in &forward[b] {
                if idom[s] == usize::MAX {
                    continue;
                }
                if new == usize::MAX {
                    new = s;
                    continue;
                }
                let (mut x, mut y) = (s, new);
                while x != y {
                    while pos[x] > pos[y] {
                        x = idom[x];
                    }
                    while pos[y] > pos[x] {
                        y = idom[y];
                    }
                }
                new = x;
            }
            if idom[b] != new {
                idom[b] = new;
                changed = true;
            }
        }
    }
    (0..n).map(|b| if idom[b] == usize::MAX || idom[b] == exit { NONE } else { idom[b] as u32 }).collect()
}

macro_rules! fail_on {
    ($self:ident, $e:expr, $loc:expr, $seeds:expr, $detail:expr) => {
        match $e {
            Ok(v) => v,
            Err(kind) => return $self.trap(kind, $loc, $seeds, $detail),
        }
    };
}

impl<'m> Machine<'m> {
    pub fn new(m: &'m Module, files: &'m [String], cfg: Config) -> Machine<'m> {
        let mut mc = Machine {
            m,
            files,
            cover: if cfg.cover { vec![0; 1 << 16] } else { Vec::new() },
            cfg,
            regions: Vec::new(),
            statics: 0,
            static_at: HashMap::new(),
            state_block: 0,
            free_stack: Vec::new(),
            quarantine: VecDeque::new(),
            frames: Vec::new(),
            regs: Vec::new(),
            hi: Vec::new(),
            objs: Vec::new(),
            used: 0,
            steps: 0,
            buf: Vec::new(),
            capturing: false,
            err: Vec::new(),
            trace: Trace::default(),
            err_start: 0,
            consts: Vec::new(),
            const_at: HashMap::new(),
        };
        mc.state_block = mc.fresh(128, State::Runtime, 0);
        for f in &m.funcs {
            for b in &f.blocks {
                for i in &b.insts {
                    let bytes: Option<Vec<u8>> = match i {
                        Inst::StrAddr { text, .. } => {
                            let mut v = text.as_bytes().to_vec();
                            v.push(0);
                            Some(v)
                        }
                        Inst::DataAddr { bytes, .. } => Some(bytes.clone()),
                        _ => None,
                    };
                    if let Some(bytes) = bytes {
                        let at = mc.fresh(bytes.len() as u64, State::Static, 0);
                        mc.regions[(at >> SHIFT) as usize - 1].data = bytes;
                        mc.static_at.insert(i as *const Inst, at);
                    }
                }
            }
        }
        mc.statics = mc.regions.len();
        mc
    }

    /// Forgets everything a run did; compile-time data stays.
    pub fn reset(&mut self) {
        self.regions.truncate(self.statics);
        self.free_stack.clear();
        self.quarantine.clear();
        self.used = 0;
        self.buf.clear();
        self.err.clear();
        self.err_start = 0;
        self.capturing = false;
        self.const_at.clear();
        let state = (self.state_block >> SHIFT) as usize - 1;
        self.regions[state].data.iter_mut().for_each(|b| *b = 0);
    }

    fn fresh(&mut self, size: u64, state: State, made: u32) -> u64 {
        let shadow = if self.cfg.trace { vec![0; size as usize] } else { Vec::new() };
        if state == State::Stack {
            if let Some(i) = self.free_stack.pop() {
                let r = &mut self.regions[i as usize];
                r.data.clear();
                r.data.resize(size as usize, 0);
                r.shadow = shadow;
                r.state = state;
                r.made = made;
                return (i as u64 + 1) << SHIFT;
            }
        }
        if state == State::Heap && self.quarantine.len() > QUARANTINE {
            let i = self.quarantine.pop_front().expect("not empty") as usize;
            self.regions[i] = Region { data: vec![0; size as usize], shadow, state, input: false, made, freed: 0 };
            return (i as u64 + 1) << SHIFT;
        }
        self.regions.push(Region { data: vec![0; size as usize], shadow, state, input: false, made, freed: 0 });
        (self.regions.len() as u64) << SHIFT
    }

    /// A heap block prepared by the caller of this run.
    pub fn input_block(&mut self, size: u64) -> u64 {
        let at = self.fresh(size, State::Heap, 0);
        self.regions[(at >> SHIFT) as usize - 1].input = true;
        at
    }

    /// Everything alive now is the caller's to worry about.
    pub fn adopt_all(&mut self) {
        for r in self.regions.iter_mut().filter(|r| r.state == State::Heap) {
            r.input = true;
        }
    }

    pub fn put(&mut self, addr: u64, bytes: &[u8]) {
        let (r, o) = (((addr >> SHIFT) - 1) as usize, addr as u32 as usize);
        self.regions[r].data[o..o + bytes.len()].copy_from_slice(bytes);
    }

    pub fn block_bytes(&self, addr: u64, n: usize) -> Option<&[u8]> {
        let r = self.regions.get(((addr >> SHIFT) as usize).checked_sub(1)?)?;
        r.data.get(addr as u32 as usize..addr as u32 as usize + n)
    }

    /// Blocks the program made and can no longer reach from `roots` or from what it was given.
    pub fn leaked(&self, roots: &[u64]) -> (usize, u64, u32) {
        let mut marked = vec![false; self.regions.len()];
        let mut work: Vec<usize> = Vec::new();
        for (i, r) in self.regions.iter().enumerate() {
            if r.input && r.state == State::Heap {
                marked[i] = true;
                work.push(i);
            }
        }
        for a in roots {
            if let Some(i) = ((*a >> SHIFT) as usize).checked_sub(1) {
                if i < marked.len() && !marked[i] {
                    marked[i] = true;
                    work.push(i);
                }
            }
        }
        while let Some(i) = work.pop() {
            for w in self.regions[i].data.chunks_exact(8) {
                let v = u64::from_le_bytes(w.try_into().expect("8 bytes"));
                if let Some(j) = ((v >> SHIFT) as usize).checked_sub(1) {
                    if j < marked.len() && !marked[j] {
                        marked[j] = true;
                        work.push(j);
                    }
                }
            }
        }
        let (mut n, mut bytes, mut site) = (0, 0, 0);
        for (i, r) in self.regions.iter().enumerate() {
            if r.state == State::Heap && !marked[i] {
                n += 1;
                bytes += r.data.len() as u64;
                if site == 0 {
                    site = r.made;
                }
            }
        }
        (n, bytes, site)
    }

    fn place(&self, line: u32) -> String {
        if line == 0 {
            return "the compiler's own code".to_string();
        }
        format!("{}:{}", self.files[(line >> 22) as usize], line & 0x3FFFFF)
    }

    fn cur_line(&self) -> u32 {
        self.frames.last().map_or(0, |f| f.line)
    }

    fn trap(&mut self, kind: Kind, loc: Loc, seeds: Vec<u32>, detail: String) -> Stop {
        let msg = format!("trap: {} at {}:{}:{}\n", kind.text(), self.files[loc.file as usize], loc.line, loc.col);
        self.err.extend_from_slice(msg.as_bytes());
        let line = if loc.line != 0 { (loc.file << 22) | loc.line } else { self.cur_line() };
        Stop::Fail(Box::new(Failure { kind, msg, exit: 101, detail, seeds, line }))
    }

    fn memory_fault(&mut self, what: String, seeds: Vec<u32>) -> Stop {
        let line = self.cur_line();
        let msg = format!("trap: {what} at {}\n", self.place(line));
        self.err.extend_from_slice(msg.as_bytes());
        Stop::Fail(Box::new(Failure { kind: Kind::Memory, msg, exit: 101, detail: String::new(), seeds, line }))
    }

    /// Finds the block and offset of `size` bytes at `addr`, or says what is wrong with it.
    fn find(&self, addr: u64, size: u64, writing: bool) -> Result<(usize, usize), String> {
        let id = addr >> SHIFT;
        if id == 0 {
            return Err(if addr == 0 { "null pointer used".to_string() } else { format!("wild pointer {addr:#x} used") });
        }
        let i = id as usize - 1;
        let Some(r) = self.regions.get(i) else { return Err(format!("wild pointer {addr:#x} used")) };
        let off = addr as u32 as u64;
        if r.state == State::Freed {
            return Err(format!("use after free (block made at {}, released at {})", self.place(r.made), self.place(r.freed)));
        }
        if off + size > r.data.len() as u64 {
            return Err(format!("out of bounds access: {size} bytes at offset {off} of a block of {}", r.data.len()));
        }
        if writing && r.state == State::Static {
            return Err("write to read-only data".to_string());
        }
        Ok((i, off as usize))
    }

    fn load(&self, addr: u64, mem: Mem) -> Result<u64, String> {
        let size = mem_size(mem);
        let (r, o) = self.find(addr, size, false)?;
        let mut raw = [0u8; 8];
        raw[..size as usize].copy_from_slice(&self.regions[r].data[o..o + size as usize]);
        let v = u64::from_le_bytes(raw);
        Ok(match mem {
            Mem::I8 => sext(v, 8),
            Mem::I16 => sext(v, 16),
            Mem::I32 => sext(v, 32),
            _ => v,
        })
    }

    fn store(&mut self, addr: u64, v: u64, mem: Mem) -> Result<(), String> {
        let size = mem_size(mem) as usize;
        let (r, o) = self.find(addr, size as u64, true)?;
        self.regions[r].data[o..o + size].copy_from_slice(&v.to_le_bytes()[..size]);
        Ok(())
    }

    fn copy(&mut self, dst: u64, src: u64, n: u64) -> Result<(), String> {
        if n == 0 {
            return Ok(());
        }
        let (rs, os) = self.find(src, n, false)?;
        let (rd, od) = self.find(dst, n, true)?;
        let n = n as usize;
        if rs == rd {
            self.regions[rd].data.copy_within(os..os + n, od);
            if self.cfg.trace {
                self.regions[rd].shadow.copy_within(os..os + n, od);
            }
            return Ok(());
        }
        let (a, b) = if rs < rd {
            let (x, y) = self.regions.split_at_mut(rd);
            (&x[rs], &mut y[0])
        } else {
            let (x, y) = self.regions.split_at_mut(rs);
            (&y[0], &mut x[rd])
        };
        b.data[od..od + n].copy_from_slice(&a.data[os..os + n]);
        if self.cfg.trace {
            b.shadow[od..od + n].copy_from_slice(&a.shadow[os..os + n]);
        }
        Ok(())
    }

    fn bytes_at(&self, addr: u64, n: u64) -> Result<Vec<u8>, String> {
        if n == 0 {
            return Ok(Vec::new());
        }
        let (r, o) = self.find(addr, n, false)?;
        Ok(self.regions[r].data[o..o + n as usize].to_vec())
    }

    /// Bytes up to, but not including, the first zero.
    fn cstr_at(&self, addr: u64) -> Result<Vec<u8>, String> {
        let (r, o) = self.find(addr, 1, false)?;
        let data = &self.regions[r].data[o..];
        match data.iter().position(|b| *b == 0) {
            Some(n) => Ok(data[..n].to_vec()),
            None => Err("a string without its terminating zero was read".to_string()),
        }
    }

    fn malloc(&mut self, bytes: u64, loc_line: u32) -> Result<u64, Stop> {
        if bytes > (1 << 36) {
            return Ok(0);
        }
        self.used += bytes;
        if self.used > self.cfg.memory {
            return Err(Stop::Limit("memory"));
        }
        Ok(self.fresh(bytes, State::Heap, loc_line))
    }

    fn release(&mut self, p: u64) -> Result<(), String> {
        if p == 0 {
            return Ok(());
        }
        let line = self.cur_line();
        let id = p >> SHIFT;
        let Some(r) = (id as usize).checked_sub(1).and_then(|i| self.regions.get_mut(i)) else { return Err(format!("free of a wild pointer {p:#x}")) };
        match r.state {
            State::Freed => {
                let (a, b) = (r.made, r.freed);
                return Err(format!("double free (block made at {}, released at {})", self.place(a), self.place(b)));
            }
            State::Heap if p as u32 == 0 => {}
            State::Heap => return Err("free of a pointer into the middle of a block".to_string()),
            _ => return Err("free of memory that did not come from `alloc`".to_string()),
        }
        self.used -= r.data.len() as u64;
        r.data = Vec::new();
        r.shadow = Vec::new();
        r.state = State::Freed;
        r.freed = line;
        self.quarantine.push_back(id as u32 - 1);
        Ok(())
    }

    fn write_out(&mut self, fd: u32) {
        match fd {
            2 => {
                let text = std::mem::take(&mut self.buf);
                self.err_start = self.err.len();
                self.err.extend_from_slice(&text);
            }
            _ if self.cfg.stdout => {
                use std::io::Write;
                let _ = std::io::stdout().write_all(&self.buf);
                let _ = std::io::stdout().flush();
                self.buf.clear();
            }
            _ => self.buf.clear(),
        }
    }

    fn push_frame(&mut self, func: FuncId, ret: Option<VReg>, ctrl_from: u32, line: u32) -> Result<usize, Stop> {
        if self.frames.len() >= self.cfg.depth {
            return Err(Stop::Limit("stack"));
        }
        let f = &self.m.funcs[func];
        let base = self.regs.len();
        self.regs.resize(base + f.nvregs as usize, 0);
        self.hi.resize(base + f.nvregs as usize, 0);
        let obj_base = self.objs.len();
        self.objs.resize(obj_base + f.objects.len(), 0);
        let ctrl_base = self.trace.branches.len();
        if self.cfg.trace {
            self.trace.regs.resize(base + f.nvregs as usize, 0);
        }
        self.frames.push(Frame { func, block: 0, ip: 0, base, obj_base, ret, line, ctrl_base, ctrl_from });
        Ok(base)
    }

    fn pop_frame(&mut self) {
        let fr = self.frames.pop().expect("a frame to leave");
        for k in fr.obj_base..self.objs.len() {
            let a = self.objs[k];
            if a != 0 {
                let i = (a >> SHIFT) as usize - 1;
                self.regions[i].state = State::Freed;
                self.free_stack.push(i as u32);
            }
        }
        self.objs.truncate(fr.obj_base);
        self.regs.truncate(fr.base);
        self.hi.truncate(fr.base);
        if self.cfg.trace {
            self.trace.regs.truncate(fr.base);
            self.trace.branches.truncate(fr.ctrl_base);
        }
    }

    fn ctrl(&self) -> u32 {
        let fr = self.frames.last().expect("running");
        if self.trace.branches.len() > fr.ctrl_base {
            self.trace.branches.last().expect("checked").event
        } else {
            fr.ctrl_from
        }
    }

    fn sh(&self, base: usize, v: VReg) -> u32 {
        self.trace.regs[base + v as usize]
    }

    /// The event behind these registers, if tracing.
    fn seeds(&self, base: usize, vs: &[VReg]) -> Vec<u32> {
        if !self.cfg.trace {
            return Vec::new();
        }
        vs.iter().map(|v| self.sh(base, *v)).filter(|e| *e != 0).collect()
    }

    fn mem_shadow(&self, addr: u64) -> u32 {
        let (r, o) = (((addr >> SHIFT).wrapping_sub(1)) as usize, addr as u32 as usize);
        self.regions.get(r).and_then(|x| x.shadow.get(o)).copied().unwrap_or(0)
    }

    fn set_mem_shadow(&mut self, addr: u64, n: u64, e: u32) {
        let (r, o) = (((addr >> SHIFT).wrapping_sub(1)) as usize, addr as u32 as usize);
        if let Some(x) = self.regions.get_mut(r) {
            let end = (o + n as usize).min(x.shadow.len());
            if o < end {
                x.shadow[o..end].fill(e);
            }
        }
    }

    /// Records where the value an instruction writes comes from, before it runs.
    fn trace_inst(&mut self, i: &Inst, base: usize) {
        let line = self.cur_line();
        let ctrl = self.ctrl();
        match i {
            Inst::Line { .. } | Inst::SetSite { .. } | Inst::Note { .. } | Inst::MemCopy { .. } | Inst::MemCopyDyn { .. } | Inst::Call { .. } | Inst::CallInd { .. } => {}
            Inst::ObjAddr { d, .. } | Inst::StrAddr { d, .. } | Inst::RtState { d } | Inst::DataAddr { d, .. } | Inst::ConstRef { d, .. } | Inst::FuncAddr { d, .. } => self.trace.regs[base + *d as usize] = 0,
            Inst::Mov { d, s } | Inst::AddImm { d, s, .. } => self.trace.regs[base + *d as usize] = self.trace.regs[base + *s as usize],
            Inst::Load { d, p, off, .. } => {
                let m = self.mem_shadow(self.regs[base + *p as usize].wrapping_add(*off));
                let e = self.trace.add(line, m, self.sh(base, *p), ctrl);
                self.trace.regs[base + *d as usize] = e;
            }
            Inst::Store { p, off, s, mem } => {
                let e = self.trace.add(line, self.sh(base, *s), self.sh(base, *p), ctrl);
                self.set_mem_shadow(self.regs[base + *p as usize].wrapping_add(*off), mem_size(*mem), e);
            }
            _ => {
                let (mut a, mut b, mut k) = (0, 0, 0);
                i.each(&mut |v, def| {
                    if !def {
                        match k {
                            0 => a = v + 1,
                            1 => b = v + 1,
                            _ => {}
                        }
                        k += 1;
                    }
                });
                let at = |x: u32, t: &Trace| if x == 0 { 0 } else { t.regs[base + x as usize - 1] };
                let (sa, sb) = (at(a, &self.trace), at(b, &self.trace));
                let e = self.trace.add(line, sa, sb, ctrl);
                let regs = &mut self.trace.regs;
                i.each(&mut |v, def| {
                    if def {
                        regs[base + v as usize] = e;
                    }
                });
            }
        }
    }

    fn enter_block(&mut self, from: usize, to: usize, func: FuncId) {
        if self.cfg.cover {
            let h = (func.wrapping_mul(31337) ^ from.wrapping_mul(257) ^ to) & 0xFFFF;
            self.cover[h] = 1;
        }
        if self.cfg.trace {
            let floor = self.frames.last().expect("running").ctrl_base;
            while self.trace.branches.len() > floor && self.trace.branches.last().expect("checked").join == to as u32 {
                self.trace.branches.pop();
            }
        }
    }

    fn trace_branch(&mut self, c: VReg, base: usize, block: usize, func: FuncId) {
        let line = self.cur_line();
        let ctrl = self.ctrl();
        let e = self.trace.add(line, self.sh(base, c), 0, ctrl);
        let m = self.m;
        let joins = self.trace.joins.entry(func).or_insert_with(|| joins_of(&m.funcs[func]));
        let join = joins[block];
        let floor = self.frames.last().expect("running").ctrl_base;
        if self.trace.branches.len() > floor && self.trace.branches.last().expect("checked").block == block as u32 {
            self.trace.branches.pop();
        }
        self.trace.branches.push(Branch { event: e, join, block: block as u32 });
    }

    /// Calls `f` with the given register values and runs until it returns.
    pub fn call(&mut self, func: FuncId, args: &[u64]) -> Stop {
        self.frames.clear();
        self.regs.clear();
        self.objs.clear();
        self.trace.regs.clear();
        self.trace.branches.clear();
        self.steps = 0;
        let base = match self.push_frame(func, None, 0, 0) {
            Ok(b) => b,
            Err(s) => return s,
        };
        let params = &self.m.funcs[func].params;
        for (p, a) in params.iter().zip(args) {
            self.regs[base + *p as usize] = *a;
        }
        let stop = self.exec();
        while !self.frames.is_empty() {
            self.pop_frame();
        }
        stop
    }

    fn exec(&mut self) -> Stop {
        let m: &'m Module = self.m;
        'frames: loop {
            let top = self.frames.len() - 1;
            let (fid, mut block, mut ip, base) = {
                let fr = &self.frames[top];
                (fr.func, fr.block, fr.ip, fr.base)
            };
            let func = &m.funcs[fid];
            macro_rules! r {
                ($v:expr) => {
                    self.regs[base + $v as usize]
                };
            }
            macro_rules! mem {
                ($e:expr, $seeds:expr) => {
                    match $e {
                        Ok(v) => v,
                        Err(what) => {
                            let s = self.seeds(base, $seeds);
                            return self.memory_fault(what, s);
                        }
                    }
                };
            }
            loop {
                let b = &func.blocks[block];
                while ip < b.insts.len() {
                    self.steps += 1;
                    if self.steps > self.cfg.steps {
                        return Stop::Limit("steps");
                    }
                    let inst = &b.insts[ip];
                    if self.cfg.trace {
                        self.trace_inst(inst, base);
                    }
                    match inst {
                        Inst::Const { d, v } => r!(*d) = *v as u64,
                        Inst::Mov { d, s } => {
                            r!(*d) = r!(*s);
                            self.hi[base + *d as usize] = self.hi[base + *s as usize];
                        }
                        Inst::Phi { .. } | Inst::Pi { .. } => unreachable!("the interpreter runs code before SSA"),
                        Inst::VBin { op, d, a, b: c, ty } => {
                            let (x, y) = ((r!(*a), self.hi[base + *a as usize]), (r!(*c), self.hi[base + *c as usize]));
                            let (lo, hi) = vec_bin(*op, *ty, x, y);
                            r!(*d) = lo;
                            self.hi[base + *d as usize] = hi;
                        }
                        Inst::VFma { d, a, b: c, c: acc, ty } => {
                            let get = |m: &Self, v: VReg| (m.regs[base + v as usize], m.hi[base + v as usize]);
                            let (x, y, z) = (get(self, *a), get(self, *c), get(self, *acc));
                            let (lo, hi) = vec_fma(*ty, x, y, z);
                            r!(*d) = lo;
                            self.hi[base + *d as usize] = hi;
                        }
                        Inst::VNeg { d, s, ty } => {
                            let x = (r!(*s), self.hi[base + *s as usize]);
                            let (lo, hi) = vec_neg(*ty, x);
                            r!(*d) = lo;
                            self.hi[base + *d as usize] = hi;
                        }
                        Inst::VSplat { d, s, ty } => {
                            let x = r!(*s);
                            let (lo, hi) = match ty {
                                Vk::F64x2 => (x, x),
                                Vk::F32x4 | Vk::I32x4 => {
                                    let w = (x & 0xFFFF_FFFF) * 0x1_0000_0001;
                                    (w, w)
                                }
                            };
                            r!(*d) = lo;
                            self.hi[base + *d as usize] = hi;
                        }
                        Inst::VLoad { d, p, off, .. } => {
                            let a = r!(*p).wrapping_add(*off);
                            let lo = mem!(self.load(a, Mem::X64), &[*p]);
                            let hi = mem!(self.load(a.wrapping_add(8), Mem::X64), &[*p]);
                            r!(*d) = lo;
                            self.hi[base + *d as usize] = hi;
                        }
                        Inst::VStore { p, off, s, .. } => {
                            let a = r!(*p).wrapping_add(*off);
                            let (lo, hi) = (r!(*s), self.hi[base + *s as usize]);
                            mem!(self.store(a, lo, Mem::X64), &[*p]);
                            mem!(self.store(a.wrapping_add(8), hi, Mem::X64), &[*p]);
                        }
                        Inst::VLane { d, s, lane, ty } => {
                            let (lo, hi) = (r!(*s), self.hi[base + *s as usize]);
                            let word = if (*lane as u32) < ty.lanes() / 2 { lo } else { hi };
                            r!(*d) = match ty {
                                Vk::F64x2 => word,
                                Vk::F32x4 => (word >> (32 * (*lane as u32 % 2))) & 0xFFFF_FFFF,
                                Vk::I32x4 => (((word >> (32 * (*lane as u32 % 2))) as u32) as i32) as i64 as u64,
                            };
                        }
                        Inst::Bin { op, d, a, b: c, num, loc } => {
                            let (x, y) = (r!(*a), r!(*c));
                            match arith(*op, x, y, *num) {
                                Ok(v) => r!(*d) = v,
                                Err(kind) => {
                                    let (seeds, detail) = match kind {
                                        Kind::DivZero | Kind::Shift => (self.seeds(base, &[*c]), format!("right = {}", show_num(y, *num))),
                                        _ => (self.seeds(base, &[*a, *c]), format!("left = {}, right = {}", show_num(x, *num), show_num(y, *num))),
                                    };
                                    return self.trap(kind, *loc, seeds, detail);
                                }
                            }
                        }
                        Inst::Neg { d, s, num, loc } => {
                            let x = r!(*s);
                            let v = if !num.checked {
                                Ok(x.wrapping_neg())
                            } else if num.bits == 64 {
                                (x as i64).checked_neg().map(|v| v as u64).ok_or(Kind::Overflow)
                            } else {
                                normalize(x.wrapping_neg(), *num)
                            };
                            let seeds = self.seeds(base, &[*s]);
                            r!(*d) = fail_on!(self, v, *loc, seeds, format!("value = {}", x as i64));
                        }
                        Inst::BitNot { d, s, num } => {
                            let x = !r!(*s);
                            r!(*d) = if !num.signed && num.bits < 64 { x & ((1u64 << num.bits) - 1) } else { x };
                        }
                        Inst::BoolNot { d, s } => r!(*d) = r!(*s) ^ 1,
                        Inst::Cmp { cc, d, a, b: c, unsigned } => {
                            let (x, y) = (r!(*a), r!(*c));
                            r!(*d) = if *unsigned { compare(*cc, x, y) } else { compare(*cc, x as i64, y as i64) };
                        }
                        Inst::FBin { op, d, a, b: c, bits } => r!(*d) = float_bin(*op, r!(*a), r!(*c), *bits),
                        Inst::FNeg { d, s, bits } => r!(*d) = r!(*s) ^ (1u64 << (*bits - 1)),
                        Inst::FCmp { cc, d, a, b: c, bits } => {
                            let (x, y) = (r!(*a), r!(*c));
                            r!(*d) = if *bits == 32 { compare(*cc, f32::from_bits(x as u32), f32::from_bits(y as u32)) } else { compare(*cc, f64::from_bits(x), f64::from_bits(y)) };
                        }
                        Inst::IntCast { d, s, from, to, loc } => {
                            let x = r!(*s);
                            let seeds = self.seeds(base, &[*s]);
                            r!(*d) = fail_on!(self, int_cast(x, *from, *to), *loc, seeds, format!("value = {}", show_num(x, *from)));
                        }
                        Inst::IntToFloat { d, s, signed, bits } => {
                            let x = r!(*s);
                            r!(*d) = match (*signed, *bits) {
                                (true, 32) => (x as i64 as f32).to_bits() as u64,
                                (true, _) => (x as i64 as f64).to_bits(),
                                (false, 32) => (x as f32).to_bits() as u64,
                                (false, _) => (x as f64).to_bits(),
                            };
                        }
                        Inst::FloatToInt { d, s, bits, to, loc } => {
                            let x = if *bits == 32 { f32::from_bits(r!(*s) as u32) as f64 } else { f64::from_bits(r!(*s)) };
                            let seeds = self.seeds(base, &[*s]);
                            r!(*d) = fail_on!(self, float_to_int(x, *to), *loc, seeds, format!("value = {x}"));
                        }
                        Inst::FloatCast { d, s, to_bits } => {
                            let x = r!(*s);
                            r!(*d) = if *to_bits == 64 { (f32::from_bits(x as u32) as f64).to_bits() } else { (f64::from_bits(x) as f32).to_bits() as u64 };
                        }
                        Inst::FuncAddr { d, f } => r!(*d) = FN_TAG | *f as u64,
                        Inst::Call { .. } | Inst::CallInd { .. } => {
                            let (d, f, args) = match inst {
                                Inst::Call { d, f, args } => (*d, *f, args),
                                Inst::CallInd { d, callee, args } => (*d, (r!(*callee) & !FN_TAG) as usize, args),
                                _ => unreachable!("matched above"),
                            };
                            let line = self.frames[top].line;
                            let ctrl = if self.cfg.trace { self.ctrl() } else { 0 };
                            self.frames[top].block = block;
                            self.frames[top].ip = ip + 1;
                            let nb = match self.push_frame(f, d, ctrl, line) {
                                Ok(b) => b,
                                Err(s) => return s,
                            };
                            for (p, a) in m.funcs[f].params.iter().zip(args) {
                                self.regs[nb + *p as usize] = self.regs[base + *a as usize];
                                if self.cfg.trace {
                                    self.trace.regs[nb + *p as usize] = self.trace.regs[base + *a as usize];
                                }
                            }
                            continue 'frames;
                        }
                        Inst::ObjAddr { d, obj } => {
                            let slot = self.frames[top].obj_base + *obj;
                            if self.objs[slot] == 0 {
                                let line = self.frames[top].line;
                                self.objs[slot] = self.fresh(func.objects[*obj].0, State::Stack, line);
                            }
                            r!(*d) = self.objs[slot];
                        }
                        Inst::AddImm { d, s, imm } => r!(*d) = r!(*s).wrapping_add(*imm),
                        Inst::Load { d, p, off, mem } => {
                            let a = r!(*p).wrapping_add(*off);
                            r!(*d) = mem!(self.load(a, *mem), &[*p]);
                        }
                        Inst::Store { p, off, s, mem } => {
                            let a = r!(*p).wrapping_add(*off);
                            let v = r!(*s);
                            mem!(self.store(a, v, *mem), &[*p]);
                        }
                        Inst::MemCopy { dst, src, size } => {
                            let (x, y) = (r!(*dst), r!(*src));
                            mem!(self.copy(x, y, *size), &[*dst, *src]);
                        }
                        Inst::MemCopyDyn { dst, src, n } => {
                            let (x, y, k) = (r!(*dst), r!(*src), r!(*n));
                            mem!(self.copy(x, y, k), &[*dst, *src]);
                        }
                        Inst::IndexAddr { d, base: p, idx, size } => r!(*d) = r!(*p).wrapping_add(r!(*idx).wrapping_mul(*size)),
                        Inst::Bounds { idx, len, loc } => {
                            let i = r!(*idx);
                            if i >= *len {
                                let seeds = self.seeds(base, &[*idx]);
                                return self.trap(Kind::Bounds, *loc, seeds, format!("index = {}, len = {len}", i as i64));
                            }
                        }
                        Inst::BoundsDyn { idx, len, loc } => {
                            let (i, n) = (r!(*idx), r!(*len));
                            if i >= n {
                                let seeds = self.seeds(base, &[*idx, *len]);
                                return self.trap(Kind::Bounds, *loc, seeds, format!("index = {}, len = {}", i as i64, n as i64));
                            }
                        }
                        Inst::Alloc { d, n, elem, loc } => {
                            let k = r!(*n);
                            let seeds = self.seeds(base, &[*n]);
                            if (k as i64) < 0 || (k as u128 * *elem as u128) >> 64 != 0 {
                                return self.trap(Kind::AllocSize, *loc, seeds, format!("count = {}", k as i64));
                            }
                            let line = self.frames[top].line;
                            let p = match self.malloc(k * *elem, line) {
                                Ok(p) => p,
                                Err(s) => return s,
                            };
                            if p == 0 {
                                return self.trap(Kind::OutOfMemory, *loc, seeds, format!("count = {}", k as i64));
                            }
                            r!(*d) = p;
                        }
                        Inst::Free { p } => {
                            let x = r!(*p);
                            mem!(self.release(x), &[*p]);
                        }
                        Inst::StrAddr { d, .. } | Inst::DataAddr { d, .. } => r!(*d) = self.static_at[&(inst as *const Inst)],
                        Inst::RtState { d } => r!(*d) = self.state_block,
                        Inst::SetSite { .. } => {}
                        Inst::ConstRef { d, id } => {
                            let v = match self.const_value(*id) {
                                Ok(v) => v,
                                Err(s) => return s,
                            };
                            r!(*d) = v;
                        }
                        Inst::Line { loc } => self.frames[top].line = (loc.file << 22) | loc.line,
                        Inst::Note { name, v, show } => {
                            if self.cfg.trace {
                                let line = self.frames[top].line;
                                let ctrl = self.ctrl();
                                let e = self.trace.add(line, self.sh(base, *v), 0, ctrl);
                                self.trace.regs[base + *v as usize] = e;
                                let value = r!(*v);
                                self.trace.add_note(NoteRec { event: e, name, value, show: *show });
                            }
                        }
                        Inst::CallC { d, sym, args, variadic: _, ret } => {
                            let vals: Vec<u64> = args.iter().map(|a| r!(a.v)).collect();
                            let raw = match self.call_c(sym, &vals) {
                                Ok(CResult::Value(v)) => v,
                                Ok(CResult::Fault(what)) => {
                                    let list: Vec<VReg> = args.iter().map(|a| a.v).collect();
                                    let s = self.seeds(base, &list);
                                    return self.memory_fault(what, s);
                                }
                                Err(s) => return s,
                            };
                            if let Some(d) = d {
                                r!(*d) = match ret {
                                    CRet::None => 0,
                                    CRet::Float(_) => raw,
                                    CRet::Int(m) => match m {
                                        Mem::I8 => sext(raw, 8),
                                        Mem::I16 => sext(raw, 16),
                                        Mem::I32 => sext(raw, 32),
                                        Mem::U8 => raw & 0xFF,
                                        Mem::U16 => raw & 0xFFFF,
                                        Mem::U32 => raw & 0xFFFF_FFFF,
                                        Mem::X64 => raw,
                                    },
                                };
                            }
                        }
                        Inst::BufStr { text } => {
                            if let Some(s) = self.buf_add(text.as_bytes()) {
                                return s;
                            }
                        }
                        Inst::BufInt { s, signed } => {
                            let x = r!(*s);
                            let t = if *signed { (x as i64).to_string() } else { x.to_string() };
                            if let Some(s) = self.buf_add(t.as_bytes()) {
                                return s;
                            }
                        }
                        Inst::BufBool { s } => {
                            let t: &[u8] = if r!(*s) != 0 { b"true" } else { b"false" };
                            if let Some(s) = self.buf_add(t) {
                                return s;
                            }
                        }
                        Inst::BufFloat { s, bits } => {
                            let x = if *bits == 32 { f32::from_bits(r!(*s) as u32) as f64 } else { f64::from_bits(r!(*s)) };
                            let t = format_float(x, *bits);
                            if let Some(s) = self.buf_add(&t) {
                                return s;
                            }
                        }
                        Inst::BufPtr { p, n } => {
                            let bytes = mem!(self.bytes_at(r!(*p), r!(*n)), &[*p]);
                            if let Some(s) = self.buf_add(&bytes) {
                                return s;
                            }
                        }
                        Inst::CaptureStart => self.capturing = true,
                        Inst::CaptureTake { ptr, len } => {
                            self.capturing = false;
                            let n = self.buf.len() as u64;
                            if n == 0 {
                                r!(*ptr) = 0;
                            } else {
                                let line = self.frames[top].line;
                                let p = match self.malloc(n, line) {
                                    Ok(p) => p,
                                    Err(s) => return s,
                                };
                                let text = std::mem::take(&mut self.buf);
                                self.put(p, &text);
                                r!(*ptr) = p;
                            }
                            r!(*len) = n;
                        }
                        Inst::Flush { fd } => self.write_out(*fd),
                        Inst::Exit { code } => return self.contract_stop(*code as i32),
                        Inst::WideBin { op, dlo, dhi, alo, ahi, blo, bhi, loc } => {
                            let x = (((r!(*ahi) as u128) << 64) | r!(*alo) as u128) as i128;
                            let y = (((r!(*bhi) as u128) << 64) | r!(*blo) as u128) as i128;
                            let v = match op {
                                WideOp::Add => x.checked_add(y),
                                WideOp::Sub => x.checked_sub(y),
                                WideOp::Mul => {
                                    if r!(*ahi) as i64 != (r!(*alo) as i64) >> 63 || r!(*bhi) as i64 != (r!(*blo) as i64) >> 63 {
                                        None
                                    } else {
                                        x.checked_mul(y)
                                    }
                                }
                            };
                            let Some(v) = v else { return self.trap(Kind::ContractMath, *loc, Vec::new(), String::new()) };
                            r!(*dlo) = v as u64;
                            r!(*dhi) = (v >> 64) as u64;
                        }
                        Inst::WideCmp { cc, d, alo, ahi, blo, bhi } => {
                            let x = (((r!(*ahi) as u128) << 64) | r!(*alo) as u128) as i128;
                            let y = (((r!(*bhi) as u128) << 64) | r!(*blo) as u128) as i128;
                            r!(*d) = compare(*cc, x, y);
                        }
                    }
                    ip += 1;
                }
                // A block may have no instructions at all: `for { }` must still run out of steps.
                self.steps += 1;
                if self.steps > self.cfg.steps {
                    return Stop::Limit("steps");
                }
                match &b.term {
                    Term::Jmp(t) => {
                        self.enter_block(block, *t, fid);
                        block = *t;
                        ip = 0;
                    }
                    Term::Br { c, t, f } => {
                        if self.cfg.trace {
                            self.trace_branch(*c, base, block, fid);
                        }
                        let next = if r!(*c) != 0 { *t } else { *f };
                        self.enter_block(block, next, fid);
                        block = next;
                        ip = 0;
                    }
                    Term::Ret(v) => {
                        let value = v.map(|x| r!(x)).unwrap_or(0);
                        let shadow = if self.cfg.trace { v.map(|x| self.sh(base, x)).unwrap_or(0) } else { 0 };
                        let dest = self.frames[top].ret;
                        self.pop_frame();
                        if self.frames.is_empty() {
                            return Stop::Done(value);
                        }
                        if let Some(d) = dest {
                            let cb = self.frames.last().expect("checked").base;
                            self.regs[cb + d as usize] = value;
                            if self.cfg.trace {
                                self.trace.regs[cb + d as usize] = shadow;
                            }
                        }
                        continue 'frames;
                    }
                    Term::Unreachable => {
                        let s = self.memory_fault("control reached the end of a function that promised a value".to_string(), Vec::new());
                        return s;
                    }
                }
            }
        }
    }

    fn const_value(&mut self, id: usize) -> Result<u64, Stop> {
        use crate::lower::ConstData;
        if let Some(a) = self.const_at.get(&id) {
            return Ok(*a);
        }
        let data = match self.consts.get(id) {
            Some(Some(d)) => d.clone(),
            _ => return Err(Stop::NeedConst(id)),
        };
        let v = match data {
            ConstData::Scalar(v) => return Ok(v),
            ConstData::Bytes(b) => {
                let a = self.fresh(b.len() as u64, State::Static, 0);
                self.regions[(a >> SHIFT) as usize - 1].data = b;
                a
            }
            ConstData::Str(b) => {
                let a = self.fresh(b.len() as u64, State::Static, 0);
                let n = b.len() as u64;
                self.regions[(a >> SHIFT) as usize - 1].data = b;
                let h = self.fresh(24, State::Static, 0);
                let mut header = Vec::with_capacity(24);
                header.extend_from_slice(&a.to_le_bytes());
                header.extend_from_slice(&n.to_le_bytes());
                header.extend_from_slice(&0u64.to_le_bytes());
                self.regions[(h >> SHIFT) as usize - 1].data = header;
                h
            }
        };
        self.const_at.insert(id, v);
        Ok(v)
    }

    fn buf_add(&mut self, bytes: &[u8]) -> Option<Stop> {
        if self.capturing && self.buf.len() + bytes.len() > CAPTURE_LIMIT {
            let msg = "trap: interpolated string is too long (64 KB)\n".to_string();
            self.err.extend_from_slice(msg.as_bytes());
            return Some(Stop::Fail(Box::new(Failure { kind: Kind::Other, msg, exit: 101, detail: String::new(), seeds: Vec::new(), line: self.cur_line() })));
        }
        self.buf.extend_from_slice(bytes);
        None
    }

    /// `Exit` is how contracts and `?` in `main` stop the program.
    fn contract_stop(&mut self, code: i32) -> Stop {
        // The last report is the one that stopped the program.
        let msg = String::from_utf8_lossy(&self.err[self.err_start..]).into_owned();
        let kind = if msg.starts_with("trap: need failed") {
            Kind::Need
        } else if msg.starts_with("trap: sure failed") {
            Kind::Sure
        } else {
            Kind::Other
        };
        let seeds = if self.cfg.trace {
            let c = self.ctrl();
            if c != 0 {
                vec![c]
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        let mut lines = msg.lines();
        lines.next();
        let detail = lines.map(|l| l.trim()).collect::<Vec<_>>().join(", ");
        Stop::Fail(Box::new(Failure { kind, msg, exit: code, detail, seeds, line: self.cur_line() }))
    }

    /// The source lines that led to the given events, oldest first.
    pub fn causes(&self, seeds: &[u32], limit: usize, shown: &dyn Fn(u32) -> bool) -> (Vec<Cause>, bool) {
        let t = &self.trace;
        let mut seen: HashMap<u32, ()> = HashMap::new();
        let mut order: Vec<u32> = Vec::new();
        let mut lost = false;
        let mut walk = |start: &[u32], seen: &mut HashMap<u32, ()>, order: &mut Vec<u32>| {
            let mut queue: VecDeque<u32> = start.iter().copied().collect();
            while let Some(id) = queue.pop_front() {
                if id == 0 || seen.contains_key(&id) || seen.len() > 400_000 {
                    continue;
                }
                let Some(e) = t.get(id) else {
                    lost = true;
                    continue;
                };
                seen.insert(id, ());
                order.push(id);
                queue.push_back(e.a);
                queue.push_back(e.b);
            }
        };
        walk(seeds, &mut seen, &mut order);
        // Then the branches that decided those lines ran.
        let deciders: Vec<u32> = order.iter().filter_map(|id| t.get(*id).map(|e| e.ctrl)).filter(|c| *c != 0).collect();
        let data_only = order.len();
        walk(&deciders, &mut seen, &mut order);

        let mut lines: Vec<u32> = Vec::new();
        for (k, id) in order.iter().enumerate() {
            let line = t.get(*id).map_or(0, |e| e.line);
            let room = if k < data_only { limit } else { limit.saturating_sub(2) };
            if line != 0 && shown(line) && !lines.contains(&line) && lines.len() < room {
                lines.push(line);
            }
        }
        let mut out: Vec<(u32, Cause)> = Vec::new();
        for line in lines {
            let mut last = 0;
            let mut values: Vec<(u32, String, String)> = Vec::new();
            for id in order.iter().copied() {
                let Some(e) = t.get(id) else { continue };
                if e.line != line {
                    continue;
                }
                last = last.max(id);
                if let Some(n) = t.note_of(e, id) {
                    let text = show_value(n.value, n.show);
                    match values.iter_mut().find(|v| v.1 == n.name) {
                        Some(v) if v.0 < id => {
                            v.0 = id;
                            v.2 = text;
                        }
                        Some(_) => {}
                        None => values.push((id, n.name.to_string(), text)),
                    }
                }
            }
            values.sort_by_key(|v| v.0);
            out.push((last, Cause { line, values: values.into_iter().map(|v| format!("{} = {}", v.1, v.2)).collect() }));
        }
        out.sort_by_key(|c| c.0);
        (out.into_iter().map(|c| c.1).collect(), lost)
    }

    /// Blocks still alive at the end of `main`.
    pub fn check_leaks(&mut self) -> Option<Failure> {
        let (n, bytes, site) = self.leaked(&[]);
        if n == 0 {
            return None;
        }
        let msg = format!("trap: memory leak: {n} blocks, {bytes} bytes never released; first made at {}\n", self.place(site));
        Some(Failure { kind: Kind::Leak, msg, exit: 101, detail: String::new(), seeds: Vec::new(), line: site })
    }

    fn call_c(&mut self, sym: &str, a: &[u64]) -> Result<CResult, Stop> {
        let f = |i: usize| f64::from_bits(a[i]);
        let float = |x: f64| Ok(CResult::Value(x.to_bits()));
        let int = |x: i64| Ok(CResult::Value(x as u64));
        macro_rules! fault {
            ($e:expr) => {
                match $e {
                    Ok(v) => v,
                    Err(what) => return Ok(CResult::Fault(what)),
                }
            };
        }
        match sym {
            "sqrt" => float(f(0).sqrt()),
            "sin" => float(f(0).sin()),
            "cos" => float(f(0).cos()),
            "tan" => float(f(0).tan()),
            "asin" => float(f(0).asin()),
            "acos" => float(f(0).acos()),
            "atan" => float(f(0).atan()),
            "exp" => float(f(0).exp()),
            "log" => float(f(0).ln()),
            "log2" => float(f(0).log2()),
            "log10" => float(f(0).log10()),
            "floor" => float(f(0).floor()),
            "ceil" => float(f(0).ceil()),
            "round" => float(f(0).round()),
            "trunc" => float(f(0).trunc()),
            "fabs" => float(f(0).abs()),
            "pow" => float(f(0).powf(f(1))),
            "atan2" => float(f(0).atan2(f(1))),
            "fmod" => float(f(0) % f(1)),
            "ldexp" => float(f(0) * 2f64.powi(a[1] as i32)),
            "sqrtf" => Ok(CResult::Value(f32::from_bits(a[0] as u32).sqrt().to_bits() as u64)),
            "abs" => int((a[0] as i32).wrapping_abs() as i64),
            "labs" => int((a[0] as i64).wrapping_abs()),
            // SAFETY: plain character functions taking and returning an int.
            "toupper" => int(unsafe { toupper(a[0] as i32) } as i64),
            "tolower" => int(unsafe { tolower(a[0] as i32) } as i64),
            "isdigit" => int(unsafe { isdigit(a[0] as i32) } as i64),
            "isalpha" => int(unsafe { isalpha(a[0] as i32) } as i64),
            "isspace" => int(unsafe { isspace(a[0] as i32) } as i64),
            "strlen" => Ok(CResult::Value(fault!(self.cstr_at(a[0])).len() as u64)),
            "atoi" => {
                let s = fault!(self.cstr_at(a[0]));
                let text = String::from_utf8_lossy(&s).trim_start().to_string();
                let end = text.char_indices().take_while(|(i, c)| c.is_ascii_digit() || (*i == 0 && (*c == '-' || *c == '+'))).count();
                int(text[..end].parse::<i32>().unwrap_or(0) as i64)
            }
            "strcmp" => {
                let (x, y) = (fault!(self.cstr_at(a[0])), fault!(self.cstr_at(a[1])));
                int(match x.cmp(&y) {
                    std::cmp::Ordering::Less => -1,
                    std::cmp::Ordering::Equal => 0,
                    std::cmp::Ordering::Greater => 1,
                })
            }
            "memcmp" => {
                let (x, y) = (fault!(self.bytes_at(a[0], a[2])), fault!(self.bytes_at(a[1], a[2])));
                int(x.iter().zip(&y).find(|(p, q)| p != q).map_or(0, |(p, q)| *p as i64 - *q as i64))
            }
            "memmove" | "memcpy" => {
                fault!(self.copy(a[0], a[1], a[2]));
                Ok(CResult::Value(a[0]))
            }
            "memset" => {
                if a[2] != 0 {
                    let (r, o) = fault!(self.find(a[0], a[2], true));
                    self.regions[r].data[o..o + a[2] as usize].fill(a[1] as u8);
                }
                Ok(CResult::Value(a[0]))
            }
            "malloc" | "calloc" => {
                let n = if sym == "malloc" { a[0] } else { a[0].saturating_mul(a[1]) };
                let line = self.cur_line();
                Ok(CResult::Value(self.malloc(n, line)?))
            }
            "free" => {
                fault!(self.release(a[0]));
                Ok(CResult::Value(0))
            }
            "__error" => Ok(CResult::Value(self.state_block + ERRNO_AT)),
            "exit" | "write" | "read" | "open" | "close" | "unlink" if !self.cfg.io => Err(Stop::Unsupported(sym.to_string())),
            "exit" => Err(Stop::Exit(a[0] as i32)),
            "write" => {
                let bytes = fault!(self.bytes_at(a[1], a[2]));
                let fd = a[0] as i32;
                if fd == 1 || fd == 2 {
                    let saved = std::mem::replace(&mut self.buf, bytes);
                    self.write_out(fd as u32);
                    self.buf = saved;
                    return int(a[2] as i64);
                }
                // SAFETY: the pointer and length describe `bytes`, which outlives the call.
                let n = unsafe { write(fd, bytes.as_ptr(), bytes.len()) };
                self.keep_errno();
                int(n as i64)
            }
            "read" => {
                let mut tmp = vec![0u8; a[2].min(1 << 30) as usize];
                // SAFETY: the buffer holds exactly as many bytes as the call may write.
                let n = unsafe { read(a[0] as i32, tmp.as_mut_ptr(), tmp.len()) };
                self.keep_errno();
                if n > 0 {
                    let (r, o) = fault!(self.find(a[1], n as u64, true));
                    self.regions[r].data[o..o + n as usize].copy_from_slice(&tmp[..n as usize]);
                }
                int(n as i64)
            }
            "open" => {
                let path = fault!(self.cstr_at(a[0]));
                let Ok(c) = CString::new(path) else { return int(-1) };
                let mode = a.get(2).copied().unwrap_or(0) as u32;
                // SAFETY: `c` is a valid C string; the mode is only read when creating a file.
                let n = unsafe { open(c.as_ptr(), a[1] as i32, mode) };
                self.keep_errno();
                int(n as i64)
            }
            "close" => {
                // SAFETY: closing a descriptor has no memory effects.
                let n = unsafe { close(a[0] as i32) };
                self.keep_errno();
                int(n as i64)
            }
            "unlink" => {
                let path = fault!(self.cstr_at(a[0]));
                let Ok(c) = CString::new(path) else { return int(-1) };
                // SAFETY: `c` is a valid C string.
                let n = unsafe { unlink(c.as_ptr()) };
                self.keep_errno();
                int(n as i64)
            }
            _ => Err(Stop::Unsupported(sym.to_string())),
        }
    }

    /// C's `errno` must be visible to the interpreted program.
    fn keep_errno(&mut self) {
        // SAFETY: `__error` returns this thread's errno location.
        let e = unsafe { *__error() };
        let at = self.state_block + ERRNO_AT;
        self.put(at, &e.to_le_bytes());
    }
}

enum CResult {
    Value(u64),
    Fault(String),
}

fn mem_size(m: Mem) -> u64 {
    match m {
        Mem::I8 | Mem::U8 => 1,
        Mem::I16 | Mem::U16 => 2,
        Mem::I32 | Mem::U32 => 4,
        Mem::X64 => 8,
    }
}

fn show_num(v: u64, num: Num) -> String {
    if num.signed {
        (v as i64).to_string()
    } else {
        v.to_string()
    }
}

fn show_value(v: u64, s: Show) -> String {
    match s {
        Show::Int => (v as i64).to_string(),
        Show::Uint => v.to_string(),
        Show::Bool => (v != 0).to_string(),
        Show::F32 => String::from_utf8_lossy(&format_float(f32::from_bits(v as u32) as f64, 32)).into_owned(),
        Show::F64 => String::from_utf8_lossy(&format_float(f64::from_bits(v), 64)).into_owned(),
    }
}
