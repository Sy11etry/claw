//! Range analysis: works out what each integer can be, and deletes the checks and
//! branches whose outcome is therefore already known. This is what "proved" means
//! for `need`, `sure`, array bounds and overflow.

use crate::cir::*;
use crate::opt::eval_cmp;
use crate::ssa;
use std::collections::HashMap;

const MIN: i128 = i64::MIN as i128;
const MAX: i128 = i64::MAX as i128;
const FULL: (i128, i128) = (MIN, MAX);

type R = (i128, i128);

fn type_range(n: Num) -> R {
    if n.signed {
        (-(1i128 << (n.bits - 1)), (1i128 << (n.bits - 1)) - 1)
    } else if n.bits == 64 {
        FULL
    } else {
        (0, (1i128 << n.bits) - 1)
    }
}

fn mem_range(m: Mem) -> R {
    match m {
        Mem::I8 => (-128, 127),
        Mem::I16 => (-32768, 32767),
        Mem::I32 => (i32::MIN as i128, i32::MAX as i128),
        Mem::U8 => (0, 255),
        Mem::U16 => (0, 65535),
        Mem::U32 => (0, u32::MAX as i128),
        Mem::X64 => FULL,
    }
}

fn clamp(r: R, t: R) -> R {
    (r.0.max(t.0).min(t.1), r.1.min(t.1).max(t.0))
}

/// The exact range of `a op b`, before any trap or wrap, if it is simple to say.
fn raw_arith(op: Arith, a: R, b: R) -> Option<R> {
    match op {
        Arith::Add => Some((a.0 + b.0, a.1 + b.1)),
        Arith::Sub => Some((a.0 - b.1, a.1 - b.0)),
        Arith::Mul => {
            let c = [a.0 * b.0, a.0 * b.1, a.1 * b.0, a.1 * b.1];
            Some((*c.iter().min().unwrap(), *c.iter().max().unwrap()))
        }
        _ => None,
    }
}

fn result_of(op: Arith, a: R, b: R, num: Num) -> R {
    let t = type_range(num);
    if let Some(r) = raw_arith(op, a, b) {
        if num.signed {
            return clamp(r, t);
        }
        // Unsigned wraps: only a range that stays put can be trusted.
        return if r.0 >= t.0 && r.1 <= t.1 && (num.bits < 64 || r.0 >= 0) { r } else if num.bits == 64 { FULL } else { t };
    }
    match op {
        Arith::Div => {
            if a.0 >= 0 && b.0 > 0 {
                (a.0 / b.1, a.1 / b.0)
            } else if num.signed {
                let m = a.0.abs().max(a.1.abs());
                clamp((-m, m), t)
            } else if a.0 >= 0 && b.0 >= 0 {
                (0, a.1)
            } else {
                t
            }
        }
        Arith::Rem => {
            let m = b.0.abs().max(b.1.abs()) - 1;
            if !num.signed && b.0 < 0 {
                // A divisor with its top bit set is huge: nothing comes off.
                if a.0 >= 0 {
                    (0, a.1)
                } else {
                    t
                }
            } else if m < 0 {
                t
            } else if a.0 >= 0 {
                (0, m.min(a.1))
            } else if num.signed {
                (-m, m)
            } else {
                t
            }
        }
        Arith::And => {
            if a.0 >= 0 && b.0 >= 0 {
                (0, a.1.min(b.1))
            } else if a.0 >= 0 {
                (0, a.1)
            } else if b.0 >= 0 {
                (0, b.1)
            } else {
                t
            }
        }
        Arith::Or | Arith::Xor if a.0 >= 0 && b.0 >= 0 => {
            let top = a.1.max(b.1);
            let mask = if top == 0 { 0 } else { (1i128 << (128 - top.leading_zeros())) - 1 };
            clamp((0, mask), t)
        }
        Arith::Shr if a.0 >= 0 && b.0 >= 0 && b.0 < 64 => (a.0 >> b.1.clamp(0, 63), a.1 >> b.0),
        _ => t,
    }
}

fn apply_pi(s: R, cc: Cc, o: R, unsigned: bool) -> R {
    if unsigned {
        // `s < o` as unsigned, with o known non-negative, pins s into 0..o.
        if o.0 < 0 {
            return s;
        }
        return match cc {
            Cc::Lt => (0.max(s.0), s.1.min(o.1 - 1)),
            Cc::Le | Cc::Eq => (0.max(s.0), s.1.min(o.1)),
            _ => s,
        };
    }
    match cc {
        Cc::Lt => (s.0, s.1.min(o.1 - 1)),
        Cc::Le => (s.0, s.1.min(o.1)),
        Cc::Gt => (s.0.max(o.0 + 1), s.1),
        Cc::Ge => (s.0.max(o.0), s.1),
        Cc::Eq => (s.0.max(o.0), s.1.min(o.1)),
        Cc::Ne => {
            if o.0 == o.1 && s.0 == o.0 {
                (s.0 + 1, s.1)
            } else if o.0 == o.1 && s.1 == o.0 {
                (s.0, s.1 - 1)
            } else {
                s
            }
        }
    }
}

fn negate(cc: Cc) -> Cc {
    match cc {
        Cc::Eq => Cc::Ne,
        Cc::Ne => Cc::Eq,
        Cc::Lt => Cc::Ge,
        Cc::Le => Cc::Gt,
        Cc::Gt => Cc::Le,
        Cc::Ge => Cc::Lt,
    }
}

fn swap(cc: Cc) -> Cc {
    match cc {
        Cc::Lt => Cc::Gt,
        Cc::Le => Cc::Ge,
        Cc::Gt => Cc::Lt,
        Cc::Ge => Cc::Le,
        other => other,
    }
}

/// After each branch on a comparison, names the operands afresh with what is now known.
fn insert_pis(f: &mut Func) {
    let pred = ssa::preds(f);
    let idom = ssa::idoms(f, &pred);
    let mut konst = vec![false; f.nvregs as usize];
    let mut cmp_of: HashMap<VReg, (Cc, VReg, VReg, bool)> = HashMap::new();
    for b in &f.blocks {
        for i in &b.insts {
            match i {
                Inst::Const { d, .. } => konst[*d as usize] = true,
                Inst::Cmp { cc, d, a, b, unsigned } => {
                    cmp_of.insert(*d, (*cc, *a, *b, *unsigned));
                }
                _ => {}
            }
        }
    }
    let mut next = f.nvregs;
    let mut renames: Vec<Vec<(VReg, VReg)>> = vec![Vec::new(); f.blocks.len()];
    let mut heads: Vec<Vec<Inst>> = vec![Vec::new(); f.blocks.len()];
    for b in 0..f.blocks.len() {
        let Term::Br { c, t, f: e } = f.blocks[b].term else { continue };
        let Some(&(cc, x, y, unsigned)) = cmp_of.get(&c) else { continue };
        for (target, cond) in [(t, cc), (e, negate(cc))] {
            if pred[target].len() != 1 {
                continue;
            }
            if !konst[x as usize] {
                let d = next;
                next += 1;
                heads[target].push(Inst::Pi { d, s: x, cc: cond, other: y, unsigned });
                renames[target].push((x, d));
            }
            if !konst[y as usize] {
                let d = next;
                next += 1;
                heads[target].push(Inst::Pi { d, s: y, cc: swap(cond), other: x, unsigned });
                renames[target].push((y, d));
            }
        }
    }
    f.nvregs = next;
    let mut kids = vec![Vec::new(); f.blocks.len()];
    for b in 1..f.blocks.len() {
        kids[idom[b]].push(b);
    }
    let mut map: HashMap<VReg, Vec<VReg>> = HashMap::new();
    enum Step {
        Enter(BlockId),
        Leave(BlockId),
    }
    let mut stack = vec![Step::Enter(0)];
    while let Some(s) = stack.pop() {
        let b = match s {
            Step::Leave(b) => {
                for (x, _) in &renames[b] {
                    map.get_mut(x).unwrap().pop();
                }
                continue;
            }
            Step::Enter(b) => b,
        };
        let lookup = |map: &HashMap<VReg, Vec<VReg>>, v: VReg| map.get(&v).and_then(|s| s.last().copied()).unwrap_or(v);
        // The Pi's own operands see the names from before it.
        for i in heads[b].iter_mut() {
            i.each_mut(&mut |v, def| {
                if !def {
                    *v = lookup(&map, *v);
                }
            });
        }
        for (x, d) in &renames[b] {
            map.entry(*x).or_default().push(*d);
        }
        for i in &mut f.blocks[b].insts {
            if matches!(i, Inst::Phi { .. }) {
                continue;
            }
            i.each_mut(&mut |v, def| {
                if !def {
                    *v = lookup(&map, *v);
                }
            });
        }
        ssa::term_each_use(&mut f.blocks[b].term, &mut |v| *v = lookup(&map, *v));
        for s in ssa::succs(&f.blocks[b].term) {
            for i in &mut f.blocks[s].insts {
                if let Inst::Phi { args, .. } = i {
                    for (p, v) in args.iter_mut() {
                        if *p == b {
                            *v = lookup(&map, *v);
                        }
                    }
                }
            }
        }
        stack.push(Step::Leave(b));
        for k in &kids[b] {
            stack.push(Step::Enter(*k));
        }
    }
    for (b, h) in heads.into_iter().enumerate() {
        let at = f.blocks[b].insts.iter().take_while(|i| matches!(i, Inst::Phi { .. })).count();
        f.blocks[b].insts.splice(at..at, h);
    }
}

struct Solver {
    r: Vec<Option<R>>,
}

impl Solver {
    fn get(&self, v: VReg) -> R {
        self.r[v as usize].unwrap_or(FULL)
    }

    /// None: an operand is not known yet, so wait for another round.
    fn eval(&self, i: &Inst) -> Option<R> {
        let val = |v: VReg| self.r[v as usize];
        Some(match i {
            Inst::Const { v, .. } => (*v as i128, *v as i128),
            Inst::Mov { s, .. } => val(*s)?,
            Inst::Phi { args, .. } => {
                let mut out: Option<R> = None;
                for (_, v) in args {
                    if let Some(x) = val(*v) {
                        out = Some(match out {
                            Some(o) => (o.0.min(x.0), o.1.max(x.1)),
                            None => x,
                        });
                    }
                }
                out?
            }
            Inst::Pi { s, cc, other, unsigned, .. } => {
                let x = val(*s)?;
                let r = apply_pi(x, *cc, val(*other).unwrap_or(FULL), *unsigned);
                // An empty range means this path cannot run; stay sound and simple.
                if r.0 > r.1 {
                    x
                } else {
                    r
                }
            }
            Inst::Bin { op, a, b, num, .. } => result_of(*op, val(*a)?, val(*b)?, *num),
            Inst::Neg { s, num, .. } => {
                let x = val(*s)?;
                clamp((-x.1, -x.0), type_range(*num))
            }
            Inst::AddImm { s, imm, .. } => {
                let x = val(*s)?;
                let k = *imm as i128;
                if x.1 + k > MAX {
                    FULL
                } else {
                    (x.0 + k, x.1 + k)
                }
            }
            Inst::Cmp { .. } | Inst::FCmp { .. } | Inst::BoolNot { .. } | Inst::WideCmp { .. } => (0, 1),
            Inst::Load { mem, .. } => mem_range(*mem),
            Inst::IntCast { s, from, to, .. } => {
                let mut x = val(*s)?;
                if to.signed && !from.signed && from.bits == 64 {
                    // A u64 with the top bit set traps here, so what gets through is not negative.
                    x = (x.0.max(0), x.1.max(0));
                }
                let t = type_range(*to);
                if x.0 >= t.0 && x.1 <= t.1 {
                    x
                } else {
                    t
                }
            }
            Inst::FloatToInt { to, .. } => type_range(*to),
            Inst::BitNot { num, .. } => type_range(*num),
            _ => FULL,
        })
    }
}

struct Prover<'a> {
    s: &'a Solver,
    defs: &'a HashMap<VReg, Inst>,
    /// Where each register is defined, and the dominator tree.
    def_block: &'a [usize],
    idom: &'a [usize],
    /// Claims about a phi being proved right now, to spot going round a loop.
    phis: HashMap<(VReg, VReg), i128>,
    steps: u32,
}

impl Prover<'_> {
    /// Is `y` fixed before block `b` starts, so one claim about it holds on every pass?
    fn settled_before(&self, y: VReg, b: usize) -> bool {
        let mut d = self.def_block[y as usize];
        if d == usize::MAX {
            return false;
        }
        if d == b {
            return false;
        }
        let mut at = b;
        while at != d && at != 0 {
            at = self.idom[at];
        }
        d = at;
        d == self.def_block[y as usize]
    }

    fn small_const(&self, v: VReg) -> Option<i128> {
        let r = self.s.get(v);
        (r.0 == r.1).then_some(r.0)
    }

    /// Is `x <= y + c` certain? A small version of the ABCD algorithm.
    fn le(&mut self, x: VReg, y: VReg, c: i128, depth: u32) -> bool {
        if x == y {
            return c >= 0;
        }
        let (rx, ry) = (self.s.get(x), self.s.get(y));
        if rx.1 <= ry.0 + c {
            return true;
        }
        self.steps += 1;
        if depth == 0 || self.steps > 2000 {
            return false;
        }
        match self.defs.get(&x).cloned() {
            Some(Inst::Pi { s, cc, other, unsigned: false, .. }) => {
                let by_bound = match cc {
                    Cc::Lt => self.le(other, y, c + 1, depth - 1),
                    Cc::Le | Cc::Eq => self.le(other, y, c, depth - 1),
                    _ => false,
                };
                if by_bound || self.le(s, y, c, depth - 1) {
                    return true;
                }
            }
            Some(Inst::Pi { s, cc, other, unsigned: true, .. }) => {
                // Only `x < y` as unsigned with y >= 0 says anything about x as signed.
                if self.s.get(other).0 >= 0 && match cc {
                    Cc::Lt => self.le(other, y, c + 1, depth - 1),
                    Cc::Le | Cc::Eq => self.le(other, y, c, depth - 1),
                    _ => false,
                } {
                    return true;
                }
                if self.le(s, y, c, depth - 1) {
                    return true;
                }
            }
            Some(Inst::AddImm { s, imm, .. }) => {
                if rx.1 < MAX && self.le(s, y, c - imm as i128, depth - 1) {
                    return true;
                }
            }
            Some(Inst::Bin { op: Arith::Add, a, b, num, .. }) if num.signed => {
                if let Some(k) = self.small_const(b) {
                    if self.le(a, y, c - k, depth - 1) {
                        return true;
                    }
                }
                if let Some(k) = self.small_const(a) {
                    if self.le(b, y, c - k, depth - 1) {
                        return true;
                    }
                }
            }
            Some(Inst::Bin { op: Arith::Sub, a, b, num, .. }) if num.signed => {
                if let Some(k) = self.small_const(b) {
                    if self.le(a, y, c + k, depth - 1) {
                        return true;
                    }
                }
            }
            Some(Inst::Phi { d, args }) if self.settled_before(y, self.def_block[d as usize]) => {
                // Going round a loop must not be what makes the claim true.
                if let Some(first) = self.phis.get(&(d, y)) {
                    return c >= *first;
                }
                self.phis.insert((d, y), c);
                let ok = args.iter().all(|(_, v)| self.le(*v, y, c, depth - 1));
                self.phis.remove(&(d, y));
                if ok {
                    return true;
                }
            }
            _ => {}
        }
        match self.defs.get(&y).cloned() {
            Some(Inst::Pi { s, cc, other, unsigned: false, .. }) => {
                let by_bound = match cc {
                    Cc::Gt => self.le(x, other, c + 1, depth - 1),
                    Cc::Ge | Cc::Eq => self.le(x, other, c, depth - 1),
                    _ => false,
                };
                by_bound || self.le(x, s, c, depth - 1)
            }
            Some(Inst::Pi { s, .. }) => self.le(x, s, c, depth - 1),
            Some(Inst::AddImm { s, imm, .. }) => ry.1 < MAX && self.le(x, s, c + imm as i128, depth - 1),
            Some(Inst::Bin { op: Arith::Add, a, b, num, .. }) if num.signed => match self.small_const(b) {
                Some(k) => self.le(x, a, c + k, depth - 1),
                None => false,
            },
            Some(Inst::Bin { op: Arith::Sub, a, b, num, .. }) if num.signed => match self.small_const(b) {
                Some(k) => self.le(x, a, c - k, depth - 1),
                None => false,
            },
            _ => false,
        }
    }

    fn holds(&mut self, cc: Cc, a: VReg, b: VReg) -> bool {
        match cc {
            Cc::Lt => self.le(a, b, -1, 12),
            Cc::Le => self.le(a, b, 0, 12),
            Cc::Gt => self.le(b, a, -1, 12),
            Cc::Ge => self.le(b, a, 0, 12),
            Cc::Eq => self.le(a, b, 0, 12) && self.le(b, a, 0, 12),
            Cc::Ne => self.le(a, b, -1, 12) || self.le(b, a, -1, 12),
        }
    }
}

fn power_of_two(r: R) -> Option<u32> {
    (r.0 == r.1 && r.0 > 0 && (r.0 as u128).is_power_of_two()).then(|| r.0.trailing_zeros())
}

pub fn run(f: &mut Func) {
    ssa::compact(f);
    insert_pis(f);
    let mut s = Solver { r: vec![None; f.nvregs as usize] };
    for p in &f.params {
        s.r[*p as usize] = Some(FULL);
    }
    // Rise to a fixed point, giving up on bounds that keep moving...
    let mut changes = vec![0u8; f.nvregs as usize];
    for _ in 0..24 {
        let mut any = false;
        for b in &f.blocks {
            for i in &b.insts {
                let Some(d) = i.def() else { continue };
                let Some(new) = s.eval(i) else { continue };
                let cur = s.r[d as usize];
                let joined = match cur {
                    Some(o) => (o.0.min(new.0), o.1.max(new.1)),
                    None => new,
                };
                if cur != Some(joined) {
                    any = true;
                    let n = &mut changes[d as usize];
                    *n = n.saturating_add(1);
                    let wide = match cur {
                        Some(o) if *n > 4 && matches!(i, Inst::Phi { .. }) => (if joined.0 < o.0 { MIN } else { joined.0 }, if joined.1 > o.1 { MAX } else { joined.1 }),
                        _ => joined,
                    };
                    s.r[d as usize] = Some(wide);
                }
            }
        }
        if !any {
            break;
        }
    }
    // ... then let the bounds on each path pull the ranges back in.
    for _ in 0..3 {
        for b in &f.blocks {
            for i in &b.insts {
                let Some(d) = i.def() else { continue };
                let Some(new) = s.eval(i) else { continue };
                if let Some(o) = s.r[d as usize] {
                    let tight = (o.0.max(new.0), o.1.min(new.1));
                    if tight.0 <= tight.1 {
                        s.r[d as usize] = Some(tight);
                    }
                }
            }
        }
    }

    let mut defs: HashMap<VReg, Inst> = HashMap::new();
    for b in &f.blocks {
        for i in &b.insts {
            if matches!(i, Inst::Pi { .. } | Inst::Phi { .. } | Inst::AddImm { .. } | Inst::Bin { .. }) {
                if let Some(d) = i.def() {
                    defs.insert(d, i.clone());
                }
            }
        }
    }
    let pred = ssa::preds(f);
    let idom = ssa::idoms(f, &pred);
    let mut def_block = vec![usize::MAX; f.nvregs as usize];
    for p in &f.params {
        def_block[*p as usize] = 0;
    }
    for (bi, b) in f.blocks.iter().enumerate() {
        for i in &b.insts {
            i.each(&mut |v, def| {
                if def {
                    def_block[v as usize] = bi;
                }
            });
        }
    }
    let mut p = Prover { s: &s, defs: &defs, def_block: &def_block, idom: &idom, phis: HashMap::new(), steps: 0 };
    // Division results whose right-hand side must become this constant.
    let mut rewrites: HashMap<VReg, i64> = HashMap::new();

    for b in &mut f.blocks {
        b.insts.retain_mut(|i| {
            p.steps = 0;
            match i {
                Inst::Bounds { idx, len, .. } => {
                    let r = p.s.get(*idx);
                    if r.0 >= 0 && r.1 < *len as i128 {
                        return false;
                    }
                }
                Inst::BoundsDyn { idx, len, .. } => {
                    let r = p.s.get(*idx);
                    if r.0 >= 0 && p.le(*idx, *len, -1, 12) {
                        return false;
                    }
                }
                Inst::Cmp { cc, d, a, b, unsigned } => {
                    let (x, y) = (p.s.get(*a), p.s.get(*b));
                    let same_sign = x.0 >= 0 && y.0 >= 0;
                    if !*unsigned || same_sign {
                        let known = if p.holds(*cc, *a, *b) {
                            Some(true)
                        } else if p.holds(negate(*cc), *a, *b) {
                            Some(false)
                        } else {
                            None
                        };
                        if let Some(v) = known {
                            *i = Inst::Const { d: *d, v: v as i64 };
                        }
                    } else if x.0 == x.1 && y.0 == y.1 {
                        let v = eval_cmp(*cc, x.0 as i64, y.0 as i64, *unsigned);
                        *i = Inst::Const { d: *d, v: v as i64 };
                    }
                }
                Inst::Bin { op, a, b, num, d, loc } => {
                    let (x, y) = (p.s.get(*a), p.s.get(*b));
                    let t = type_range(*num);
                    match op {
                        Arith::Add | Arith::Sub | Arith::Mul if num.signed => {
                            let r = raw_arith(*op, x, y).unwrap();
                            if r.0 >= t.0 && r.1 <= t.1 {
                                num.checked = false;
                            }
                        }
                        Arith::Div | Arith::Rem => {
                            let nonzero = y.0 > 0 || y.1 < 0;
                            let no_overflow = !num.signed || *op == Arith::Rem || x.0 > t.0 || y.0 > -1 || y.1 < -1;
                            if nonzero && no_overflow {
                                num.checked = false;
                            }
                            // A non-negative number divides by 2^k with a shift.
                            if let (Some(k), true) = (power_of_two(y), x.0 >= 0 && !num.checked) {
                                let n = Num { bits: num.bits, signed: num.signed, checked: false };
                                if *op == Arith::Div {
                                    let (d, a, b, loc) = (*d, *a, *b, *loc);
                                    *i = Inst::Bin { op: Arith::Shr, d, a, b, num: n, loc };
                                    rewrites.insert(d, k as i64);
                                } else {
                                    let (d, a, b, loc) = (*d, *a, *b, *loc);
                                    *i = Inst::Bin { op: Arith::And, d, a, b, num: n, loc };
                                    rewrites.insert(d, (1i64 << k) - 1);
                                }
                            }
                        }
                        Arith::Shr => {
                            if y.0 >= 0 && y.1 < num.bits as i128 {
                                num.checked = false;
                            }
                        }
                        Arith::Shl => {
                            if y.0 >= 0 && y.1 < 62 && num.signed {
                                let top = |v: i128| v << y.1;
                                let r = (top(x.0.min(0)), top(x.1.max(0)));
                                if r.0 >= t.0 && r.1 <= t.1 {
                                    num.checked = false;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Inst::Neg { s, num, .. } => {
                    let r = p.s.get(*s);
                    let t = type_range(*num);
                    if -r.1 >= t.0 && -r.0 <= t.1 {
                        num.checked = false;
                    }
                }
                Inst::IntCast { s, from, to, .. } => {
                    let r = p.s.get(*s);
                    let t = type_range(*to);
                    let top_bit_clear = from.signed || from.bits < 64 || r.0 >= 0;
                    if to.signed && top_bit_clear && r.0 >= t.0 && r.1 <= t.1 {
                        to.checked = false;
                    }
                }
                _ => {}
            }
            true
        });
    }

    // Divisions turned into shifts need their new right-hand constants.
    let mut next = f.nvregs;
    if !rewrites.is_empty() {
        for b in &mut f.blocks {
            let old = std::mem::take(&mut b.insts);
            for mut i in old {
                if let Inst::Bin { d, b: rhs, .. } = &mut i {
                    if let Some(v) = rewrites.get(d) {
                        let k = next;
                        next += 1;
                        *rhs = k;
                        b.insts.push(Inst::Const { d: k, v: *v });
                    }
                }
                b.insts.push(i);
            }
        }
        f.nvregs = next;
    }

    for b in &mut f.blocks {
        for i in &mut b.insts {
            if let Inst::Pi { d, s, .. } = *i {
                *i = Inst::Mov { d, s };
            }
        }
    }
}
