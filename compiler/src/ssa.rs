//! SSA form: building it (Braun et al.), and leaving it again.

use crate::cir::*;
use std::collections::{HashMap, HashSet};

pub fn succs(t: &Term) -> Vec<BlockId> {
    match t {
        Term::Jmp(b) => vec![*b],
        Term::Br { t, f, .. } if t == f => vec![*t],
        Term::Br { t, f, .. } => vec![*t, *f],
        _ => Vec::new(),
    }
}

pub fn preds(f: &Func) -> Vec<Vec<BlockId>> {
    let mut out = vec![Vec::new(); f.blocks.len()];
    for (i, b) in f.blocks.iter().enumerate() {
        for s in succs(&b.term) {
            out[s].push(i);
        }
    }
    out
}

pub fn term_each_use(t: &mut Term, f: &mut dyn FnMut(&mut VReg)) {
    match t {
        Term::Br { c, .. } => f(c),
        Term::Ret(Some(v)) => f(v),
        _ => {}
    }
}

pub fn term_map_blocks(t: &mut Term, f: &mut dyn FnMut(BlockId) -> BlockId) {
    match t {
        Term::Jmp(b) => *b = f(*b),
        Term::Br { t, f: e, .. } => {
            *t = f(*t);
            *e = f(*e);
        }
        _ => {}
    }
}

/// Blocks in reverse postorder; a branch's first target directly follows it where possible.
pub fn rpo(f: &Func) -> Vec<BlockId> {
    let mut seen = vec![false; f.blocks.len()];
    let mut post = Vec::with_capacity(f.blocks.len());
    let mut stack: Vec<(BlockId, usize)> = vec![(0, 0)];
    seen[0] = true;
    while let Some(&mut (b, ref mut next)) = stack.last_mut() {
        let mut s = succs(&f.blocks[b].term);
        s.reverse(); // Visited last means placed first.
        if *next < s.len() {
            let c = s[*next];
            *next += 1;
            if !seen[c] {
                seen[c] = true;
                stack.push((c, 0));
            }
        } else {
            post.push(b);
            stack.pop();
        }
    }
    post.reverse();
    post
}

/// Drops unreachable blocks and renumbers the rest in reverse postorder (entry stays 0).
pub fn compact(f: &mut Func) {
    for b in &mut f.blocks {
        if let Term::Br { t, f: e, .. } = b.term {
            if t == e {
                b.term = Term::Jmp(t);
            }
        }
    }
    let order = rpo(f);
    let mut new_id = vec![usize::MAX; f.blocks.len()];
    for (n, b) in order.iter().enumerate() {
        new_id[*b] = n;
    }
    let mut old: Vec<Option<Block>> = std::mem::take(&mut f.blocks).into_iter().map(Some).collect();
    for b in &order {
        let mut block = old[*b].take().expect("each block once");
        term_map_blocks(&mut block.term, &mut |x| new_id[x]);
        for i in &mut block.insts {
            if let Inst::Phi { args, .. } = i {
                args.retain(|(p, _)| new_id[*p] != usize::MAX);
                for (p, _) in args.iter_mut() {
                    *p = new_id[*p];
                }
            }
        }
        f.blocks.push(block);
    }
}

/// Immediate dominators, for blocks already in reverse postorder (Cooper, Harvey, Kennedy).
pub fn idoms(f: &Func, pred: &[Vec<BlockId>]) -> Vec<usize> {
    let n = f.blocks.len();
    let mut idom = vec![usize::MAX; n];
    idom[0] = 0;
    loop {
        let mut changed = false;
        for b in 1..n {
            let mut new = usize::MAX;
            for &p in &pred[b] {
                if idom[p] == usize::MAX {
                    continue;
                }
                if new == usize::MAX {
                    new = p;
                    continue;
                }
                let (mut x, mut y) = (p, new);
                while x != y {
                    while x > y {
                        x = idom[x];
                    }
                    while y > x {
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
        if !changed {
            break;
        }
    }
    idom
}

struct Rename {
    defs: Vec<HashMap<VReg, VReg>>,
    sealed: Vec<bool>,
    incomplete: Vec<Vec<(VReg, VReg)>>,
    phis: Vec<Vec<Inst>>,
    pred: Vec<Vec<BlockId>>,
    next: VReg,
    undef: Vec<Inst>,
}

impl Rename {
    fn fresh(&mut self) -> VReg {
        self.next += 1;
        self.next - 1
    }

    fn read(&mut self, var: VReg, b: BlockId) -> VReg {
        if let Some(v) = self.defs[b].get(&var) {
            return *v;
        }
        let v = if !self.sealed[b] {
            let d = self.fresh();
            self.incomplete[b].push((var, d));
            d
        } else if self.pred[b].is_empty() {
            // Read before any write on this path: it cannot matter, so any value will do.
            let d = self.fresh();
            self.undef.push(Inst::Const { d, v: 0 });
            d
        } else if self.pred[b].len() == 1 {
            let p = self.pred[b][0];
            self.read(var, p)
        } else {
            let d = self.fresh();
            self.defs[b].insert(var, d);
            self.fill_phi(var, d, b);
            d
        };
        self.defs[b].insert(var, v);
        v
    }

    fn fill_phi(&mut self, var: VReg, d: VReg, b: BlockId) {
        let args = self.pred[b].clone().into_iter().map(|p| (p, self.read(var, p))).collect();
        self.phis[b].push(Inst::Phi { d, args });
    }

    fn seal(&mut self, b: BlockId) {
        self.sealed[b] = true;
        for (var, d) in std::mem::take(&mut self.incomplete[b]) {
            self.fill_phi(var, d, b);
        }
    }
}

/// Gives every register that is written more than once a single definition.
/// `extra` are renamed even if written once: a read may not be dominated by the write.
pub fn build(f: &mut Func, extra: &[VReg]) {
    compact(f);
    let n = f.blocks.len();
    let mut count = vec![0u32; f.nvregs as usize];
    for p in &f.params {
        count[*p as usize] += 1;
    }
    for b in &f.blocks {
        for i in &b.insts {
            i.each(&mut |v, def| {
                if def {
                    count[v as usize] += 1;
                }
            });
        }
    }
    let mut multi: HashSet<VReg> = (0..f.nvregs).filter(|v| count[*v as usize] > 1).collect();
    multi.extend(extra);
    if multi.is_empty() {
        return;
    }
    let pred = preds(f);
    let mut r = Rename { defs: vec![HashMap::new(); n], sealed: vec![false; n], incomplete: vec![Vec::new(); n], phis: vec![Vec::new(); n], pred, next: f.nvregs, undef: Vec::new() };
    for p in &f.params {
        r.defs[0].insert(*p, *p);
    }
    let mut filled = vec![false; n];
    for b in 0..n {
        if r.pred[b].iter().all(|p| filled[*p]) {
            r.seal(b);
        }
        let mut insts = std::mem::take(&mut f.blocks[b].insts);
        for i in &mut insts {
            if matches!(i, Inst::Phi { .. }) {
                // Its operands are read at the end of each predecessor, below.
                i.each_mut(&mut |v, def| {
                    if def && multi.contains(v) {
                        let new = r.fresh();
                        r.defs[b].insert(*v, new);
                        *v = new;
                    }
                });
                continue;
            }
            i.each_mut(&mut |v, def| {
                if !def && multi.contains(v) {
                    *v = r.read(*v, b);
                }
            });
            i.each_mut(&mut |v, def| {
                if def && multi.contains(v) {
                    let new = r.fresh();
                    r.defs[b].insert(*v, new);
                    *v = new;
                }
            });
        }
        f.blocks[b].insts = insts;
        term_each_use(&mut f.blocks[b].term, &mut |v| {
            if multi.contains(v) {
                *v = r.read(*v, b);
            }
        });
        filled[b] = true;
        for s in succs(&f.blocks[b].term) {
            if !r.sealed[s] && s <= b + n && r.pred[s].iter().all(|p| filled[*p]) {
                r.seal(s);
            }
        }
    }
    for b in 0..n {
        for i in 0..f.blocks[b].insts.len() {
            let Inst::Phi { args, .. } = &mut f.blocks[b].insts[i] else { continue };
            for (p, v) in args.iter_mut() {
                if multi.contains(v) {
                    *v = r.read(*v, *p);
                }
            }
        }
    }
    for (b, phis) in r.phis.drain(..).enumerate() {
        f.blocks[b].insts.splice(0..0, phis);
    }
    f.blocks[0].insts.splice(0..0, r.undef.drain(..));
    f.nvregs = r.next;
}

/// Emits copies that behave as if they all happened at once.
fn sequentialize(moves: &mut Vec<(VReg, VReg)>, next: &mut VReg, out: &mut Vec<Inst>) {
    moves.retain(|(d, s)| d != s);
    while !moves.is_empty() {
        if let Some(i) = (0..moves.len()).find(|&i| !moves.iter().any(|(_, s)| *s == moves[i].0)) {
            let (d, s) = moves.swap_remove(i);
            out.push(Inst::Mov { d, s });
        } else {
            // Only cycles are left: save one value to break it.
            let t = *next;
            *next += 1;
            let d = moves[0].0;
            out.push(Inst::Mov { d: t, s: d });
            for m in moves.iter_mut() {
                if m.1 == d {
                    m.1 = t;
                }
            }
        }
    }
}

/// Replaces every phi with copies at the end of its predecessors.
pub fn destruct(f: &mut Func) {
    compact(f);
    let pred = preds(f);
    let mut new_blocks: Vec<Block> = Vec::new();
    let base = f.blocks.len();
    let has_phi: Vec<bool> = f.blocks.iter().map(|b| b.insts.iter().any(|i| matches!(i, Inst::Phi { .. }))).collect();
    // A copy on an edge needs a place that runs only on that edge.
    for b in 0..base {
        let targets = succs(&f.blocks[b].term);
        if targets.len() < 2 {
            continue;
        }
        for s in targets {
            if pred[s].len() < 2 || !has_phi[s] {
                continue;
            }
            let id = base + new_blocks.len();
            new_blocks.push(Block { insts: Vec::new(), term: Term::Jmp(s) });
            term_map_blocks(&mut f.blocks[b].term, &mut |x| if x == s { id } else { x });
            for i in &mut f.blocks[s].insts {
                if let Inst::Phi { args, .. } = i {
                    for (p, _) in args.iter_mut() {
                        if *p == b {
                            *p = id;
                        }
                    }
                }
            }
        }
    }
    f.blocks.append(&mut new_blocks);
    let n = f.blocks.len();
    let mut copies: Vec<Vec<(VReg, VReg)>> = vec![Vec::new(); n];
    for b in &mut f.blocks {
        b.insts.retain(|i| {
            if let Inst::Phi { d, args } = i {
                for (p, v) in args {
                    copies[*p].push((*d, *v));
                }
                false
            } else {
                true
            }
        });
    }
    let mut next = f.nvregs;
    for (b, moves) in copies.iter_mut().enumerate() {
        let mut out = Vec::new();
        sequentialize(moves, &mut next, &mut out);
        f.blocks[b].insts.append(&mut out);
    }
    f.nvregs = next;
    compact(f);
    forward_into_copies(f);
}

/// `x = a + 1; d = x` with x used nowhere else becomes `d = a + 1`: no copy in the loop.
fn forward_into_copies(f: &mut Func) {
    let mut uses = vec![0u32; f.nvregs as usize];
    let mut defs = vec![0u32; f.nvregs as usize];
    for b in &mut f.blocks {
        for i in &b.insts {
            i.each(&mut |v, def| {
                if def {
                    defs[v as usize] += 1;
                } else {
                    uses[v as usize] += 1;
                }
            });
        }
        term_each_use(&mut b.term, &mut |v| uses[*v as usize] += 1);
    }
    for b in &mut f.blocks {
        let mut k = b.insts.len();
        while k > 0 {
            k -= 1;
            let Inst::Mov { d, s } = b.insts[k] else { continue };
            if uses[s as usize] != 1 || defs[s as usize] != 1 {
                continue;
            }
            let mut j = k;
            let mut found = None;
            while j > 0 {
                j -= 1;
                if b.insts[j].def() == Some(s) {
                    found = Some(j);
                    break;
                }
                let mut touches = false;
                b.insts[j].each(&mut |v, _| touches |= v == d || v == s);
                if touches {
                    break;
                }
            }
            let Some(j) = found else { continue };
            if matches!(b.insts[j], Inst::Const { .. } | Inst::ObjAddr { .. }) {
                continue;
            }
            b.insts[j].each_mut(&mut |v, def| {
                if def {
                    *v = d;
                }
            });
            b.insts.remove(k);
        }
    }
}
