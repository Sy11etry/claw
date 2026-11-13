//! The optimizer. Works on one function at a time, in SSA form.

use crate::cir::*;
use crate::ssa;

fn mask(v: i64, bits: u8) -> i64 {
    if bits >= 64 {
        v
    } else {
        v & ((1i64 << bits) - 1)
    }
}

fn fits(v: i128, bits: u8) -> bool {
    v >= -(1i128 << (bits - 1)) && v < (1i128 << (bits - 1))
}

/// The value of an operation on two known numbers; None when it would trap.
pub fn eval_bin(op: Arith, a: i64, b: i64, num: Num) -> Option<i64> {
    let signed_result = |v: i128| fits(v, num.bits).then_some(v as i64);
    if num.signed {
        let (x, y) = (a as i128, b as i128);
        return match op {
            Arith::Add => signed_result(x + y),
            Arith::Sub => signed_result(x - y),
            Arith::Mul => signed_result(x * y),
            Arith::Div if y != 0 => signed_result(x / y),
            Arith::Rem if y != 0 => signed_result(x % y),
            Arith::And => Some(a & b),
            Arith::Or => Some(a | b),
            Arith::Xor => Some(a ^ b),
            Arith::Shl if (0..num.bits as i128).contains(&y) => signed_result(x << y),
            Arith::Shr if (0..num.bits as i128).contains(&y) => Some(a >> b),
            _ => None,
        };
    }
    let (x, y) = (a as u64, b as u64);
    let r = match op {
        Arith::Add => x.wrapping_add(y),
        Arith::Sub => x.wrapping_sub(y),
        Arith::Mul => x.wrapping_mul(y),
        Arith::Div if y != 0 => x / y,
        Arith::Rem if y != 0 => x % y,
        Arith::And => x & y,
        Arith::Or => x | y,
        Arith::Xor => x ^ y,
        Arith::Shl if y < num.bits as u64 => x << y,
        Arith::Shr if y < num.bits as u64 => x >> y,
        _ => return None,
    };
    Some(mask(r as i64, num.bits))
}

pub fn eval_cmp(cc: Cc, a: i64, b: i64, unsigned: bool) -> bool {
    let ord = if unsigned { (a as u64).cmp(&(b as u64)) } else { a.cmp(&b) };
    match cc {
        Cc::Eq => ord.is_eq(),
        Cc::Ne => ord.is_ne(),
        Cc::Lt => ord.is_lt(),
        Cc::Le => ord.is_le(),
        Cc::Gt => ord.is_gt(),
        Cc::Ge => ord.is_ge(),
    }
}

fn eval_cast(s: i64, from: Num, to: Num) -> Option<i64> {
    if to.signed {
        if !from.signed && from.bits == 64 && s < 0 {
            return None;
        }
        return fits(s as i128, to.bits).then_some(s);
    }
    Some(mask(s, to.bits))
}

/// A cast between these types leaves the 64-bit register exactly as it was.
fn cast_is_noop(from: Num, to: Num) -> bool {
    if to.bits == 64 {
        return !to.signed || from.signed || from.bits < 64;
    }
    from.bits <= to.bits && from.signed == to.signed || (!from.signed && from.bits < to.bits)
}

fn find(repl: &mut [VReg], v: VReg) -> VReg {
    let mut r = v;
    while repl[r as usize] != r {
        r = repl[r as usize];
    }
    let mut c = v;
    while repl[c as usize] != r {
        let n = repl[c as usize];
        repl[c as usize] = r;
        c = n;
    }
    r
}

/// Constant folding, copy propagation, branch folding and dead code, until nothing changes.
pub fn simplify(f: &mut Func) {
    for _ in 0..16 {
        let mut changed = fold(f);
        changed |= remove_dead(f);
        changed |= merge_blocks(f);
        if !changed {
            break;
        }
    }
}

fn fold(f: &mut Func) -> bool {
    let n = f.nvregs as usize;
    let mut konst: Vec<Option<i64>> = vec![None; n];
    for b in &f.blocks {
        for i in &b.insts {
            if let Inst::Const { d, v } = i {
                konst[*d as usize] = Some(*v);
            }
        }
    }
    let mut defs_not: Vec<Option<VReg>> = vec![None; n];
    let mut repl: Vec<VReg> = (0..f.nvregs).collect();
    let mut changed = false;

    for b in &mut f.blocks {
        for i in &mut b.insts {
            let new = match &*i {
                Inst::Mov { d, s } => {
                    repl[*d as usize] = *s;
                    changed = true;
                    None
                }
                Inst::Phi { d, args } => {
                    // All operands the same (or the phi itself): it is just that value.
                    let mut one = None;
                    let mut same = true;
                    for (_, v) in args {
                        let v = find(&mut repl, *v);
                        if v == *d || Some(v) == one {
                            continue;
                        }
                        if one.is_some() {
                            same = false;
                            break;
                        }
                        one = Some(v);
                    }
                    if let (true, Some(v)) = (same, one) {
                        repl[*d as usize] = v;
                        changed = true;
                    }
                    None
                }
                Inst::BoolNot { d, s } => {
                    let s = find(&mut repl, *s);
                    if let Some(x) = defs_not[s as usize] {
                        repl[*d as usize] = x;
                        changed = true;
                        None
                    } else {
                        defs_not[*d as usize] = Some(s);
                        konst[s as usize].map(|v| Inst::Const { d: *d, v: (v == 0) as i64 })
                    }
                }
                _ => None,
            };
            if let Some(x) = new {
                if let Some(d) = x.def() {
                    if let Inst::Const { v, .. } = x {
                        konst[d as usize] = Some(v);
                    }
                }
                *i = x;
                changed = true;
            }
        }
    }

    for b in &mut f.blocks {
        for i in &mut b.insts {
            i.each_mut(&mut |v, def| {
                if !def {
                    *v = find(&mut repl, *v);
                }
            });
        }
        ssa::term_each_use(&mut b.term, &mut |v| *v = find(&mut repl, *v));
    }

    for b in &mut f.blocks {
        for i in &mut b.insts {
            let new = match &*i {
                Inst::Bin { op, d, a, b, num, .. } => {
                    let (x, y) = (konst[*a as usize], konst[*b as usize]);
                    match (x, y, op) {
                        (Some(x), Some(y), _) => eval_bin(*op, x, y, *num).map(|v| Inst::Const { d: *d, v }),
                        (_, Some(0), Arith::Add | Arith::Sub | Arith::Or | Arith::Xor | Arith::Shl | Arith::Shr) => Some(Inst::Mov { d: *d, s: *a }),
                        (Some(0), _, Arith::Add | Arith::Or | Arith::Xor) => Some(Inst::Mov { d: *d, s: *b }),
                        (_, Some(1), Arith::Mul | Arith::Div) => Some(Inst::Mov { d: *d, s: *a }),
                        (Some(1), _, Arith::Mul) => Some(Inst::Mov { d: *d, s: *b }),
                        _ => None,
                    }
                }
                Inst::Neg { d, s, num, .. } => konst[*s as usize].and_then(|v| eval_bin(Arith::Sub, 0, v, *num)).map(|v| Inst::Const { d: *d, v }),
                Inst::BitNot { d, s, num } => konst[*s as usize].map(|v| Inst::Const { d: *d, v: if num.signed { !v } else { mask(!v, num.bits) } }),
                Inst::Cmp { cc, d, a, b, unsigned } => match (konst[*a as usize], konst[*b as usize]) {
                    (Some(x), Some(y)) => Some(Inst::Const { d: *d, v: eval_cmp(*cc, x, y, *unsigned) as i64 }),
                    _ if a == b => Some(Inst::Const { d: *d, v: matches!(cc, Cc::Eq | Cc::Le | Cc::Ge) as i64 }),
                    _ => None,
                },
                Inst::IntCast { d, s, from, to, .. } => match konst[*s as usize].and_then(|v| eval_cast(v, *from, *to)) {
                    Some(v) => Some(Inst::Const { d: *d, v }),
                    None if cast_is_noop(*from, *to) => Some(Inst::Mov { d: *d, s: *s }),
                    None => None,
                },
                Inst::AddImm { d, s, imm } => match konst[*s as usize] {
                    Some(v) => Some(Inst::Const { d: *d, v: v.wrapping_add(*imm as i64) }),
                    None if *imm == 0 => Some(Inst::Mov { d: *d, s: *s }),
                    None => None,
                },
                Inst::IndexAddr { d, base, idx, size } => match konst[*idx as usize] {
                    Some(v) if v >= 0 && (v as u64).checked_mul(*size).is_some() => Some(Inst::AddImm { d: *d, s: *base, imm: v as u64 * *size }),
                    _ => None,
                },
                Inst::Bounds { idx, len, .. } => match konst[*idx as usize] {
                    Some(v) if v >= 0 && (v as u64) < *len => Some(Inst::Mov { d: u32::MAX, s: 0 }),
                    _ => None,
                },
                Inst::BoundsDyn { idx, len, .. } => match (konst[*idx as usize], konst[*len as usize]) {
                    (Some(i), Some(n)) if i >= 0 && i < n => Some(Inst::Mov { d: u32::MAX, s: 0 }),
                    _ => None,
                },
                _ => None,
            };
            if let Some(x) = new {
                if let Inst::Const { d, v } = x {
                    konst[d as usize] = Some(v);
                }
                *i = x;
                changed = true;
            }
        }
        b.insts.retain(|i| !matches!(i, Inst::Mov { d: u32::MAX, .. }));
    }

    // Branches whose condition is known, or is a negation.
    let mut pruned = false;
    for id in 0..f.blocks.len() {
        if let Term::Br { c, t, f: e } = f.blocks[id].term {
            if let Some(x) = defs_not[c as usize] {
                f.blocks[id].term = Term::Br { c: x, t: e, f: t };
                changed = true;
                continue;
            }
            if t == e {
                f.blocks[id].term = Term::Jmp(t);
                changed = true;
                continue;
            }
            if let Some(v) = konst[c as usize] {
                let (keep, drop) = if v != 0 { (t, e) } else { (e, t) };
                f.blocks[id].term = Term::Jmp(keep);
                for i in &mut f.blocks[drop].insts {
                    if let Inst::Phi { args, .. } = i {
                        args.retain(|(p, _)| *p != id);
                    }
                }
                pruned = true;
                changed = true;
            }
        }
    }
    if pruned {
        ssa::compact(f);
    }
    changed
}

fn remove_dead(f: &mut Func) -> bool {
    let n = f.nvregs as usize;
    let mut live = vec![false; n];
    let mut work: Vec<VReg> = Vec::new();
    // Where each register comes from.
    let mut def_at: Vec<Option<(usize, usize)>> = vec![None; n];
    for (bi, b) in f.blocks.iter_mut().enumerate() {
        for (ii, i) in b.insts.iter().enumerate() {
            i.each(&mut |v, def| {
                if def {
                    def_at[v as usize] = Some((bi, ii));
                }
            });
            if !i.removable() {
                i.each(&mut |v, def| {
                    if !def {
                        work.push(v);
                    }
                });
            }
        }
        ssa::term_each_use(&mut b.term, &mut |v| work.push(*v));
    }
    while let Some(v) = work.pop() {
        if std::mem::replace(&mut live[v as usize], true) {
            continue;
        }
        if let Some((b, i)) = def_at[v as usize] {
            f.blocks[b].insts[i].each(&mut |u, def| {
                if !def {
                    work.push(u);
                }
            });
        }
    }
    let mut changed = false;
    for b in &mut f.blocks {
        let before = b.insts.len();
        b.insts.retain(|i| {
            if !i.removable() {
                return true;
            }
            let mut any = false;
            i.each(&mut |v, def| any |= def && live[v as usize]);
            any
        });
        changed |= b.insts.len() != before;
    }
    changed
}

/// A block whose only successor has no other predecessor can absorb it.
fn merge_blocks(f: &mut Func) -> bool {
    let mut changed = false;
    let mut pred_count = vec![0u32; f.blocks.len()];
    for b in &f.blocks {
        for s in ssa::succs(&b.term) {
            pred_count[s] += 1;
        }
    }
    for a in 0..f.blocks.len() {
        while let Term::Jmp(s) = f.blocks[a].term {
            if s == a || s == 0 || pred_count[s] != 1 || f.blocks[s].insts.iter().any(|i| matches!(i, Inst::Phi { .. })) {
                break;
            }
            let mut tail = std::mem::take(&mut f.blocks[s].insts);
            let term = std::mem::replace(&mut f.blocks[s].term, Term::Unreachable);
            f.blocks[a].insts.append(&mut tail);
            for t in ssa::succs(&term) {
                for i in &mut f.blocks[t].insts {
                    if let Inst::Phi { args, .. } = i {
                        for (p, _) in args.iter_mut() {
                            if *p == s {
                                *p = a;
                            }
                        }
                    }
                }
            }
            f.blocks[a].term = term;
            pred_count[s] = 0;
            changed = true;
        }
    }
    if changed {
        ssa::compact(f);
    }
    changed
}

/// What the optimizer achieved, for `claw build --stats`.
#[derive(Default)]
pub struct Stats {
    pub inlined: usize,
    pub checks_before: usize,
    pub checks_after: usize,
}

fn count_checks(f: &Func) -> usize {
    let mut n = 0;
    for b in &f.blocks {
        for i in &b.insts {
            n += match i {
                Inst::Bounds { .. } | Inst::BoundsDyn { .. } => 1,
                Inst::Bin { op, num, .. } if num.checked && (num.signed || matches!(op, Arith::Div | Arith::Rem | Arith::Shl | Arith::Shr)) && !matches!(op, Arith::And | Arith::Or | Arith::Xor) => 1,
                Inst::Neg { num, .. } if num.checked => 1,
                _ => 0,
            };
        }
        if matches!(b.term, Term::Unreachable) && b.insts.iter().any(|i| matches!(i, Inst::Exit { code: 101 })) {
            n += 1;
        }
    }
    n
}

/// Callees first, so what gets inlined is already optimized.
fn bottom_up(m: &Module) -> Vec<FuncId> {
    let n = m.funcs.len();
    let mut seen = vec![false; n];
    let mut out = Vec::with_capacity(n);
    let callees = |f: FuncId| -> Vec<FuncId> {
        let mut c = Vec::new();
        for b in &m.funcs[f].blocks {
            for i in &b.insts {
                if let Inst::Call { f: g, .. } = i {
                    c.push(*g);
                }
            }
        }
        c
    };
    for root in 0..n {
        if seen[root] {
            continue;
        }
        seen[root] = true;
        let mut stack = vec![(root, callees(root), 0usize)];
        while let Some((f, list, next)) = stack.last_mut() {
            if *next < list.len() {
                let g = list[*next];
                *next += 1;
                if !seen[g] {
                    seen[g] = true;
                    let c = callees(g);
                    stack.push((g, c, 0));
                }
            } else {
                out.push(*f);
                stack.pop();
            }
        }
    }
    out
}

const INLINE_LIMIT: usize = 40;
const CALLER_LIMIT: usize = 4000;
const INLINE_FRAME: u64 = 4096;

fn size_of(f: &Func) -> usize {
    f.blocks.iter().map(|b| b.insts.iter().filter(|i| !matches!(i, Inst::Const { .. } | Inst::Phi { .. })).count() + 1).sum()
}

fn inline_calls(f: &mut Func, id: FuncId, funcs: &[Func], done: &[bool], stats: &mut Stats) {
    let mut budget = CALLER_LIMIT.saturating_sub(size_of(f));
    let mut b = 0;
    let original = f.blocks.len();
    let mut queue: Vec<BlockId> = (0..original).collect();
    while b < queue.len() {
        let block = queue[b];
        b += 1;
        let found = f.blocks[block].insts.iter().enumerate().find_map(|(k, i)| match i {
            Inst::Call { f: g, .. } if *g != id && done[*g] => {
                let c = &funcs[*g];
                let size = size_of(c);
                let frame: u64 = c.objects.iter().map(|o| o.0).sum();
                (size <= INLINE_LIMIT && size <= budget && frame <= INLINE_FRAME).then_some((k, *g, size))
            }
            _ => None,
        });
        // Calls that are too big stay put, but a later call in the block may still fit.
        let Some((k, g, size)) = found else { continue };
        budget -= size;
        let cont = inline_one(f, block, k, &funcs[g]);
        stats.inlined += 1;
        queue.push(cont);
    }
}

/// Replaces the call at `insts[k]`; returns the block holding what came after it.
fn inline_one(f: &mut Func, block: BlockId, k: usize, c: &Func) -> BlockId {
    let Inst::Call { d, args, .. } = f.blocks[block].insts[k].clone() else { unreachable!("a call") };
    let base = f.blocks.len();
    let cont = base + c.blocks.len();
    let shift = f.nvregs;
    let obj_shift = f.objects.len();
    f.nvregs += c.nvregs;
    f.objects.extend(c.objects.iter().copied());

    let mut map: Vec<VReg> = (0..c.nvregs).map(|v| v + shift).collect();
    for (p, a) in c.params.iter().zip(&args) {
        map[*p as usize] = *a;
    }

    let mut rets: Vec<(BlockId, Option<VReg>)> = Vec::new();
    for (i, cb) in c.blocks.iter().enumerate() {
        let mut nb = cb.clone();
        for inst in &mut nb.insts {
            inst.each_mut(&mut |v, _| *v = map[*v as usize]);
            match inst {
                Inst::Phi { args, .. } => {
                    for (p, _) in args.iter_mut() {
                        *p += base;
                    }
                }
                Inst::ObjAddr { obj, .. } => *obj += obj_shift,
                _ => {}
            }
        }
        match &nb.term {
            Term::Ret(v) => {
                let v = v.map(|x| map[x as usize]);
                rets.push((base + i, v));
                nb.term = Term::Jmp(cont);
            }
            _ => {
                ssa::term_each_use(&mut nb.term, &mut |v| *v = map[*v as usize]);
                ssa::term_map_blocks(&mut nb.term, &mut |x| x + base);
            }
        }
        f.blocks.push(nb);
    }

    let tail = f.blocks[block].insts.split_off(k + 1);
    f.blocks[block].insts.pop();
    let term = std::mem::replace(&mut f.blocks[block].term, Term::Jmp(base));
    for s in ssa::succs(&term) {
        for i in &mut f.blocks[s].insts {
            if let Inst::Phi { args, .. } = i {
                for (p, _) in args.iter_mut() {
                    if *p == block {
                        *p = cont;
                    }
                }
            }
        }
    }
    let mut insts = Vec::with_capacity(tail.len() + 1);
    if let Some(d) = d {
        let args = rets.iter().filter_map(|(b, v)| v.map(|v| (*b, v))).collect();
        insts.push(Inst::Phi { d, args });
    }
    insts.extend(tail);
    f.blocks.push(Block { insts, term });
    cont
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Base {
    Obj(usize),
    Param(VReg),
}

fn mem_size(m: Mem) -> u64 {
    match m {
        Mem::I8 | Mem::U8 => 1,
        Mem::I16 | Mem::U16 => 2,
        Mem::I32 | Mem::U32 => 4,
        Mem::X64 => 8,
    }
}

/// Small fixed copies become loads and stores, which the passes below can see through.
fn expand_copies(f: &mut Func) {
    let mut next = f.nvregs;
    for b in &mut f.blocks {
        if !b.insts.iter().any(|i| matches!(i, Inst::MemCopy { size, .. } if *size <= 64)) {
            continue;
        }
        let old = std::mem::take(&mut b.insts);
        for i in old {
            let Inst::MemCopy { dst, src, size } = i else {
                b.insts.push(i);
                continue;
            };
            if size > 64 {
                b.insts.push(i);
                continue;
            }
            let mut off = 0;
            while off < size {
                let mem = if size - off >= 8 { Mem::X64 } else { Mem::U8 };
                let d = next;
                next += 1;
                b.insts.push(Inst::Load { d, p: src, off, mem });
                b.insts.push(Inst::Store { p: dst, off, s: d, mem });
                off += mem_size(mem);
            }
        }
    }
    f.nvregs = next;
}

/// Treats each field of a stack object (or of a parameter, in safe code) as a variable.
/// A loop that never changes `a.len` then reads it once, and range analysis can reason
/// about it. Objects nothing else can see stop existing in memory altogether.
fn promote_fields(f: &mut Func) {
    let n = f.nvregs as usize;
    let mut addr: Vec<Option<(Base, u64)>> = vec![None; n];
    if !f.claw {
        for p in &f.params {
            addr[*p as usize] = Some((Base::Param(*p), 0));
        }
    }
    for b in &f.blocks {
        for i in &b.insts {
            match i {
                Inst::ObjAddr { d, obj } => addr[*d as usize] = Some((Base::Obj(*obj), 0)),
                Inst::AddImm { d, s, imm } => addr[*d as usize] = addr[*s as usize].map(|(x, o)| (x, o + imm)),
                _ => {}
            }
        }
    }

    let mut captured: Vec<Base> = Vec::new();
    let mut passed: Vec<Base> = Vec::new();
    let mut fields: Vec<(Base, u64, Mem)> = Vec::new();
    for b in &f.blocks {
        for i in &b.insts {
            match i {
                Inst::Load { p, off, mem, .. } => {
                    if let Some((x, o)) = addr[*p as usize] {
                        fields.push((x, o + off, *mem));
                    }
                }
                Inst::Store { p, off, s, mem } => {
                    if let Some((x, o)) = addr[*p as usize] {
                        fields.push((x, o + off, *mem));
                    }
                    if let Some((x, _)) = addr[*s as usize] {
                        captured.push(x);
                    }
                }
                Inst::AddImm { .. } | Inst::ObjAddr { .. } => {}
                Inst::Call { args, .. } | Inst::CallInd { args, .. } => {
                    for a in args {
                        if let Some((x, _)) = addr[*a as usize] {
                            passed.push(x);
                        }
                    }
                }
                _ => i.each(&mut |v, def| {
                    if !def {
                        if let Some((x, _)) = addr[v as usize] {
                            captured.push(x);
                        }
                    }
                }),
            }
        }
        if let Term::Ret(Some(v)) = b.term {
            if let Some((x, _)) = addr[v as usize] {
                captured.push(x);
            }
        }
    }
    if f.claw {
        // An unsafe callee could keep the address and write through it later.
        captured.extend(passed.iter().copied());
    }
    fields.sort_by_key(|(x, o, m)| (format!("{x:?}"), *o, mem_size(*m), *m as u8));
    fields.dedup();
    // Fields that overlap without being the same cannot be separate variables.
    for w in fields.windows(2) {
        if w[0].0 == w[1].0 && w[0].1 + mem_size(w[0].2) > w[1].1 {
            captured.push(w[0].0);
        }
    }
    fields.retain(|(x, _, _)| !captured.contains(x));
    if fields.is_empty() {
        return;
    }

    let mut next = f.nvregs;
    let vars: std::collections::HashMap<(Base, u64, Mem), VReg> = fields
        .iter()
        .map(|k| {
            next += 1;
            (*k, next - 1)
        })
        .collect();
    let of_base = |x: Base| -> Vec<(u64, Mem, VReg)> { fields.iter().filter(|(b, _, _)| *b == x).map(|k| (k.1, k.2, vars[k])).collect() };
    let private = |x: Base| matches!(x, Base::Obj(_)) && !passed.contains(&x);

    let reload = |insts: &mut Vec<Inst>, x: Base, off: u64, mem: Mem, var: VReg, next: &mut VReg| match x {
        Base::Obj(obj) => {
            let t = *next;
            *next += 1;
            insts.push(Inst::ObjAddr { d: t, obj });
            insts.push(Inst::Load { d: var, p: t, off, mem });
        }
        Base::Param(p) => insts.push(Inst::Load { d: var, p, off, mem }),
    };

    for b in &mut f.blocks {
        let old = std::mem::take(&mut b.insts);
        let mut out = Vec::with_capacity(old.len());
        for i in old {
            match &i {
                Inst::Load { d, p, off, mem } => {
                    if let Some((x, o)) = addr[*p as usize] {
                        if let Some(var) = vars.get(&(x, o + off, *mem)) {
                            out.push(Inst::Mov { d: *d, s: *var });
                            continue;
                        }
                    }
                }
                Inst::Store { p, off, s, mem } => {
                    if let Some((x, o)) = addr[*p as usize] {
                        if let Some(var) = vars.get(&(x, o + off, *mem)) {
                            let (var, value) = (*var, *s);
                            if !private(x) {
                                out.push(i);
                            }
                            out.push(Inst::Mov { d: var, s: value });
                            continue;
                        }
                    }
                }
                Inst::Call { args, .. } | Inst::CallInd { args, .. } => {
                    let mut hit: Vec<Base> = args.iter().filter_map(|a| addr[*a as usize].map(|t| t.0)).collect();
                    hit.sort_by_key(|x| format!("{x:?}"));
                    hit.dedup();
                    out.push(i);
                    for x in hit {
                        for (off, mem, var) in of_base(x) {
                            reload(&mut out, x, off, mem, var, &mut next);
                        }
                    }
                    continue;
                }
                _ => {}
            }
            out.push(i);
        }
        b.insts = out;
    }

    // Parameters arrive already holding their values.
    let mut entry = Vec::new();
    for ((x, off, mem), var) in fields.iter().map(|k| (*k, vars[k])) {
        if let Base::Param(_) = x {
            reload(&mut entry, x, off, mem, var, &mut next);
        }
    }
    f.blocks[0].insts.splice(0..0, entry);
    f.nvregs = next;
    let extra: Vec<VReg> = vars.values().copied().collect();
    ssa::build(f, &extra);
}

/// Children of each block in the dominator tree.
fn dom_children(idom: &[usize]) -> Vec<Vec<BlockId>> {
    let mut out = vec![Vec::new(); idom.len()];
    for b in 1..idom.len() {
        if idom[b] != usize::MAX {
            out[idom[b]].push(b);
        }
    }
    out
}

type Key = (u8, u64, u64, u64);

fn commutative(op: Arith) -> bool {
    matches!(op, Arith::Add | Arith::Mul | Arith::And | Arith::Or | Arith::Xor)
}

fn num_bits(n: Num) -> u64 {
    n.bits as u64 | (n.signed as u64) << 8 | (n.checked as u64) << 9
}

fn key_of(i: &Inst, float: &[bool]) -> Option<(Key, Option<VReg>)> {
    let (a, b): (u64, u64);
    Some(match i {
        Inst::Bin { op, d, a: x, b: y, num, .. } => {
            (a, b) = if commutative(*op) && x > y { (*y as u64, *x as u64) } else { (*x as u64, *y as u64) };
            ((1, a, b, *op as u64 | num_bits(*num) << 8), Some(*d))
        }
        Inst::Cmp { cc, d, a: x, b: y, unsigned } => ((2, *x as u64, *y as u64, *cc as u64 | (*unsigned as u64) << 8), Some(*d)),
        Inst::AddImm { d, s, imm } => ((3, *s as u64, *imm, 0), Some(*d)),
        Inst::IndexAddr { d, base, idx, size } => ((4, *base as u64, *idx as u64, *size), Some(*d)),
        Inst::FBin { op, d, a: x, b: y, bits } => ((5, *x as u64, *y as u64, *op as u64 | (*bits as u64) << 8), Some(*d)),
        Inst::Neg { d, s, num, .. } => ((6, *s as u64, num_bits(*num), 0), Some(*d)),
        Inst::BitNot { d, s, num } => ((7, *s as u64, num_bits(*num), 0), Some(*d)),
        Inst::BoolNot { d, s } => ((8, *s as u64, 0, 0), Some(*d)),
        Inst::IntCast { d, s, from, to, .. } => ((9, *s as u64, num_bits(*from), num_bits(*to)), Some(*d)),
        Inst::IntToFloat { d, s, signed, bits } => ((10, *s as u64, *signed as u64, *bits as u64), Some(*d)),
        Inst::FloatCast { d, s, to_bits } => ((11, *s as u64, *to_bits as u64, 0), Some(*d)),
        Inst::FNeg { d, s, bits } => ((12, *s as u64, *bits as u64, 0), Some(*d)),
        Inst::FCmp { cc, d, a: x, b: y, bits } => ((13, *x as u64, *y as u64, *cc as u64 | (*bits as u64) << 8), Some(*d)),
        Inst::Bounds { idx, len, .. } => ((14, *idx as u64, *len, 0), None),
        Inst::BoundsDyn { idx, len, slice, .. } => ((15, *idx as u64, *len as u64, *slice as u64), None),
        // 0 and 0.0 are the same bits but live in different registers.
        Inst::Const { d, v } => ((16, *v as u64, float[*d as usize] as u64, 0), Some(*d)),
        Inst::ObjAddr { d, obj } => ((17, *obj as u64, 0, 0), Some(*d)),
        _ => return None,
    })
}

/// Removes a computation or check already done on every path here.
fn gvn(f: &mut Func) {
    let pred = ssa::preds(f);
    let idom = ssa::idoms(f, &pred);
    let kids = dom_children(&idom);
    let mut table: std::collections::HashMap<Key, Option<VReg>> = std::collections::HashMap::new();
    let mut repl: Vec<VReg> = (0..f.nvregs).collect();
    let float = crate::regalloc::float_classes(f);
    enum Step {
        Enter(BlockId),
        Leave(Vec<Key>),
    }
    let mut stack = vec![Step::Enter(0)];
    while let Some(s) = stack.pop() {
        let b = match s {
            Step::Leave(keys) => {
                for k in keys {
                    table.remove(&k);
                }
                continue;
            }
            Step::Enter(b) => b,
        };
        let mut added = Vec::new();
        let old = std::mem::take(&mut f.blocks[b].insts);
        let mut out = Vec::with_capacity(old.len());
        for mut i in old {
            if !matches!(i, Inst::Phi { .. }) {
                i.each_mut(&mut |v, def| {
                    if !def {
                        *v = find(&mut repl, *v);
                    }
                });
            }
            if let Some((key, d)) = key_of(&i, &float) {
                if let Some(prev) = table.get(&key) {
                    if let (Some(d), Some(p)) = (d, prev) {
                        repl[d as usize] = *p;
                    }
                    continue;
                }
                table.insert(key, d);
                added.push(key);
            }
            out.push(i);
        }
        f.blocks[b].insts = out;
        stack.push(Step::Leave(added));
        for k in &kids[b] {
            stack.push(Step::Enter(*k));
        }
    }
    for b in &mut f.blocks {
        for i in &mut b.insts {
            i.each_mut(&mut |v, def| {
                if !def {
                    *v = find(&mut repl, *v);
                }
            });
        }
        ssa::term_each_use(&mut b.term, &mut |v| *v = find(&mut repl, *v));
    }
}

/// Moves work whose inputs do not change in a loop out in front of it.
fn licm(f: &mut Func) {
    let pred = ssa::preds(f);
    let idom = ssa::idoms(f, &pred);
    let dominates = |a: usize, mut b: usize| {
        while b != a && b != 0 {
            b = idom[b];
        }
        a == b
    };
    let mut loops: Vec<(BlockId, Vec<BlockId>)> = Vec::new();
    for b in 0..f.blocks.len() {
        for h in ssa::succs(&f.blocks[b].term) {
            if !dominates(h, b) {
                continue;
            }
            let mut body = vec![h];
            let mut work = vec![b];
            while let Some(x) = work.pop() {
                if !body.contains(&x) {
                    body.push(x);
                    work.extend(pred[x].iter().copied());
                }
            }
            loops.push((h, body));
        }
    }
    // Inner loops first, so code climbs out one level at a time.
    loops.sort_by_key(|(_, b)| b.len());
    if loops.is_empty() {
        return;
    }
    let float = crate::regalloc::float_classes(f);
    let mut def_block = vec![usize::MAX; f.nvregs as usize];
    for (bi, b) in f.blocks.iter().enumerate() {
        for i in &b.insts {
            i.each(&mut |v, def| {
                if def {
                    def_block[v as usize] = bi;
                }
            });
        }
    }
    for p in &f.params {
        def_block[*p as usize] = 0;
    }
    for (h, body) in loops {
        let outside: Vec<BlockId> = pred[h].iter().copied().filter(|p| !body.contains(p)).collect();
        let [pre] = outside[..] else { continue };
        if ssa::succs(&f.blocks[pre].term).len() != 1 {
            continue;
        }
        let mut order = body.clone();
        order.sort();
        for b in order {
            let old = std::mem::take(&mut f.blocks[b].insts);
            let mut keep = Vec::with_capacity(old.len());
            for i in old {
                let movable = i.removable() && !matches!(i, Inst::Phi { .. } | Inst::Pi { .. } | Inst::Load { .. } | Inst::VLoad { .. } | Inst::Cmp { .. } | Inst::FCmp { .. } | Inst::ObjAddr { .. } | Inst::StrAddr { .. } | Inst::RtState { .. } | Inst::Mov { .. });
                // A float constant costs two instructions each time; an integer one is usually an immediate.
                let movable = movable || matches!(&i, Inst::Const { d, v } if float[*d as usize] || !crate::regalloc::cheap_const(*v));
                let mut inside = false;
                i.each(&mut |v, def| {
                    if !def && (def_block[v as usize] == usize::MAX || body.contains(&def_block[v as usize])) {
                        inside = true;
                    }
                });
                if movable && !inside {
                    if let Some(d) = i.def() {
                        def_block[d as usize] = pre;
                    }
                    f.blocks[pre].insts.push(i);
                } else {
                    keep.push(i);
                }
            }
            f.blocks[b].insts = keep;
        }
    }
}

/// Frame slots of objects nothing refers to any more can go.
fn drop_unused_objects(f: &mut Func) {
    let mut used = vec![false; f.objects.len()];
    for b in &f.blocks {
        for i in &b.insts {
            if let Inst::ObjAddr { obj, .. } = i {
                used[*obj] = true;
            }
        }
    }
    let mut map = vec![usize::MAX; f.objects.len()];
    let mut kept = Vec::new();
    for (o, s) in f.objects.iter().enumerate() {
        if used[o] {
            map[o] = kept.len();
            kept.push(*s);
        }
    }
    for b in &mut f.blocks {
        for i in &mut b.insts {
            if let Inst::ObjAddr { obj, .. } = i {
                *obj = map[*obj];
            }
        }
    }
    f.objects = kept;
}

/// In `--fast` builds nothing is checked: the programmer has taken responsibility.
fn strip_checks(f: &mut Func) {
    for b in &mut f.blocks {
        b.insts.retain(|i| !matches!(i, Inst::Bounds { .. } | Inst::BoundsDyn { .. }));
        for i in &mut b.insts {
            match i {
                Inst::Bin { num, .. } | Inst::Neg { num, .. } => num.checked = false,
                Inst::IntCast { to, .. } | Inst::FloatToInt { to, .. } => to.checked = false,
                _ => {}
            }
        }
    }
}

pub fn optimize(m: &mut Module, fast: bool) -> Stats {
    let mut stats = Stats::default();
    shake(m);
    let order = bottom_up(m);
    let mut done = vec![false; m.funcs.len()];
    for f in &m.funcs {
        if !f.dead {
            stats.checks_before += count_checks(f);
        }
    }
    for id in order {
        if m.funcs[id].dead {
            continue;
        }
        let mut f = std::mem::replace(&mut m.funcs[id], placeholder());
        ssa::build(&mut f, &[]);
        if fast {
            strip_checks(&mut f);
        }
        simplify(&mut f);
        inline_calls(&mut f, id, &m.funcs, &done, &mut stats);
        ssa::compact(&mut f);
        simplify(&mut f);
        expand_copies(&mut f);
        promote_fields(&mut f);
        simplify(&mut f);
        gvn(&mut f);
        simplify(&mut f);
        licm(&mut f);
        crate::ranges::run(&mut f);
        simplify(&mut f);
        gvn(&mut f);
        simplify(&mut f);
        m.funcs[id] = f;
        done[id] = true;
    }
    shake(m);
    for f in m.funcs.iter_mut().filter(|f| !f.dead) {
        stats.checks_after += count_checks(f);
        ssa::destruct(f);
        rotate_loops(f);
        drop_unused_objects(f);
        f.optimized = true;
    }
    stats
}

/// `for` tests its condition at the top, so every pass costs a jump back as well. When the test is a
/// few harmless instructions, the end of the loop gets its own copy and branches straight to the body.
fn rotate_loops(f: &mut Func) {
    const MAX_COPIED: usize = 6;
    let n = f.blocks.len();
    let mut uses = vec![0u32; f.nvregs as usize];
    let mut defs = vec![0u32; f.nvregs as usize];
    for b in &mut f.blocks {
        for i in &b.insts {
            i.each(&mut |v, def| if def { defs[v as usize] += 1 } else { uses[v as usize] += 1 });
        }
        ssa::term_each_use(&mut b.term, &mut |v| uses[*v as usize] += 1);
    }
    let copyable = |i: &Inst| {
        i.removable() && !i.is_call() && !matches!(i, Inst::Phi { .. } | Inst::Pi { .. })
            || matches!(i, Inst::Bin { .. } | Inst::IntCast { .. } | Inst::Bounds { .. } | Inst::BoundsDyn { .. })
    };
    for l in 0..n {
        let Term::Jmp(h) = f.blocks[l].term else { continue };
        if h > l || h == l || !matches!(f.blocks[h].term, Term::Br { .. }) {
            continue;
        }
        if f.blocks[h].insts.len() > MAX_COPIED || !f.blocks[h].insts.iter().all(copyable) {
            continue;
        }
        // Values the test makes and only the test reads can be renamed in the copy.
        let mut local_uses = std::collections::HashMap::<VReg, u32>::new();
        for i in &f.blocks[h].insts {
            i.each(&mut |v, def| if !def { *local_uses.entry(v).or_insert(0) += 1 });
        }
        let mut term = f.blocks[h].term.clone();
        ssa::term_each_use(&mut term, &mut |v| *local_uses.entry(*v).or_insert(0) += 1);
        let mut rename = std::collections::HashMap::<VReg, VReg>::new();
        let mut copy = Vec::new();
        let mut carried = false;
        for i in f.blocks[h].insts.clone() {
            let mut i = i;
            i.each_mut(&mut |v, def| {
                let local = defs[*v as usize] == 1 && uses[*v as usize] == local_uses.get(v).copied().unwrap_or(0);
                if def {
                    if local {
                        let fresh = f.nvregs;
                        f.nvregs += 1;
                        rename.insert(*v, fresh);
                        *v = fresh;
                    }
                } else if let Some(r) = rename.get(v) {
                    *v = *r;
                } else if local && f.blocks[h].insts.iter().any(|x| x.def() == Some(*v)) {
                    // Read before this pass writes it: a value carried round the loop.
                    carried = true;
                }
            });
            copy.push(i);
        }
        if carried {
            continue;
        }
        ssa::term_each_use(&mut term, &mut |v| {
            if let Some(r) = rename.get(v) {
                *v = *r;
            }
        });
        f.blocks[l].insts.append(&mut copy);
        f.blocks[l].term = term;
    }
}

fn placeholder() -> Func {
    Func { name: String::new(), at: Loc { line: 0, col: 0, file: 0 }, nparams: 0, params: Vec::new(), vsig: 0, c_abi: false, claw: false, optimized: false, dead: true, nvregs: 0, objects: Vec::new(), blocks: Vec::new() }
}

/// Functions nothing can reach get no code at all.
pub fn shake(m: &mut Module) {
    let mut seen = vec![false; m.funcs.len()];
    let mut work = vec![m.main];
    if let Some(c) = m.check {
        work.extend([c.alloc, c.free, c.finish]);
    }
    while let Some(f) = work.pop() {
        if std::mem::replace(&mut seen[f], true) {
            continue;
        }
        for b in &m.funcs[f].blocks {
            for i in &b.insts {
                if let Inst::Call { f: g, .. } | Inst::FuncAddr { f: g, .. } = i {
                    work.push(*g);
                }
            }
        }
    }
    for (f, s) in m.funcs.iter_mut().zip(seen) {
        f.dead = !s;
    }
}
