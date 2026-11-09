//! Linear scan register allocation over whole live intervals.

use crate::cir::*;
use crate::ssa;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// Never read.
    None,
    Reg(u32),
    Fpr(u32),
    Slot(u32),
    /// Materialized at each use instead of occupying a register.
    Const(i64),
    Obj(usize),
}

pub struct Alloc {
    pub places: Vec<Place>,
    pub saved_x: Vec<u32>,
    pub saved_d: Vec<u32>,
    pub slots: u32,
}

/// Every value on the stack: the unoptimized build's simple, predictable layout.
pub fn naive(f: &Func) -> Alloc {
    Alloc { places: (0..f.nvregs).map(Place::Slot).collect(), saved_x: Vec::new(), saved_d: Vec::new(), slots: f.nvregs }
}

const LOW_X: [u32; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
const MID_X: [u32; 3] = [8, 15, 17];
const SAVED_X: [u32; 10] = [19, 20, 21, 22, 23, 24, 25, 26, 27, 28];
const LOW_D: [u32; 6] = [2, 3, 4, 5, 6, 7];
const MID_D: [u32; 16] = [16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31];
const SAVED_D: [u32; 8] = [8, 9, 10, 11, 12, 13, 14, 15];

#[derive(Clone, Copy)]
struct Interval {
    v: VReg,
    start: u32,
    end: u32,
    float: bool,
    /// Live across a call: only callee-saved registers survive.
    crosses: bool,
    /// Touches a call or the entry: x0-x7 are in use around it.
    touches: bool,
    weight: f32,
}

fn float_classes(f: &Func) -> Vec<bool> {
    let n = f.nvregs as usize;
    let mut parent: Vec<u32> = (0..f.nvregs).collect();
    fn root(p: &mut [u32], mut x: u32) -> u32 {
        while p[x as usize] != x {
            p[x as usize] = p[p[x as usize] as usize];
            x = p[x as usize];
        }
        x
    }
    let mut float = vec![false; n];
    for b in &f.blocks {
        for i in &b.insts {
            match i {
                Inst::Mov { d, s } => {
                    let (a, c) = (root(&mut parent, *d), root(&mut parent, *s));
                    parent[a as usize] = c;
                }
                Inst::FBin { d, a, b, .. } => {
                    float[*d as usize] = true;
                    float[*a as usize] = true;
                    float[*b as usize] = true;
                }
                Inst::FNeg { d, s, .. } | Inst::FloatCast { d, s, .. } => {
                    float[*d as usize] = true;
                    float[*s as usize] = true;
                }
                Inst::FCmp { a, b, .. } => {
                    float[*a as usize] = true;
                    float[*b as usize] = true;
                }
                Inst::IntToFloat { d, .. } => float[*d as usize] = true,
                Inst::FloatToInt { s, .. } | Inst::BufFloat { s, .. } => float[*s as usize] = true,
                _ => {}
            }
        }
    }
    let mut group = vec![false; n];
    for v in 0..f.nvregs {
        if float[v as usize] {
            let r = root(&mut parent, v);
            group[r as usize] = true;
        }
    }
    (0..f.nvregs).map(|v| group[root(&mut parent, v) as usize]).collect()
}

pub fn allocate(f: &Func) -> Alloc {
    let n = f.nvregs as usize;
    let mut places = vec![Place::None; n];
    let words = n.div_ceil(64).max(1);
    let nb = f.blocks.len();
    let float = float_classes(f);

    // Single definitions by a constant or a stack address need no register.
    let mut defs = vec![0u32; n];
    for p in &f.params {
        defs[*p as usize] += 1;
    }
    for b in &f.blocks {
        for i in &b.insts {
            i.each(&mut |v, def| {
                if def {
                    defs[v as usize] += 1;
                }
            });
        }
    }
    let mut remat = vec![false; n];
    for b in &f.blocks {
        for i in &b.insts {
            match i {
                Inst::Const { d, v } if defs[*d as usize] == 1 => {
                    places[*d as usize] = Place::Const(*v);
                    remat[*d as usize] = true;
                }
                Inst::ObjAddr { d, obj } if defs[*d as usize] == 1 => {
                    places[*d as usize] = Place::Obj(*obj);
                    remat[*d as usize] = true;
                }
                _ => {}
            }
        }
    }

    // Liveness per block.
    let mut gen_set = vec![0u64; nb * words];
    let mut kill = vec![0u64; nb * words];
    let bit = |set: &mut [u64], b: usize, v: VReg| set[b * words + v as usize / 64] |= 1 << (v % 64);
    let has = |set: &[u64], b: usize, v: VReg| set[b * words + v as usize / 64] >> (v % 64) & 1 != 0;
    for (bi, b) in f.blocks.iter().enumerate() {
        for i in &b.insts {
            i.each(&mut |v, def| {
                if !def && !has(&kill, bi, v) {
                    bit(&mut gen_set, bi, v);
                }
            });
            i.each(&mut |v, def| {
                if def {
                    bit(&mut kill, bi, v);
                }
            });
        }
        let mut t = b.term.clone();
        ssa::term_each_use(&mut t, &mut |v| {
            if !has(&kill, bi, *v) {
                bit(&mut gen_set, bi, *v);
            }
        });
    }
    let mut live_in = gen_set.clone();
    let mut live_out = vec![0u64; nb * words];
    loop {
        let mut changed = false;
        for b in (0..nb).rev() {
            for s in ssa::succs(&f.blocks[b].term) {
                for w in 0..words {
                    let x = live_in[s * words + w];
                    if x & !live_out[b * words + w] != 0 {
                        live_out[b * words + w] |= x;
                        changed = true;
                    }
                }
            }
            for w in 0..words {
                let x = gen_set[b * words + w] | (live_out[b * words + w] & !kill[b * words + w]);
                if x != live_in[b * words + w] {
                    live_in[b * words + w] = x;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Loops, from jumps that go backwards in the block order.
    let mut depth = vec![0u32; nb];
    for (b, block) in f.blocks.iter().enumerate() {
        for s in ssa::succs(&block.term) {
            if s <= b {
                for d in &mut depth[s..=b] {
                    *d += 1;
                }
            }
        }
    }

    // Intervals, positions counted in instructions.
    let mut start = vec![u32::MAX; n];
    let mut end = vec![0u32; n];
    let mut weight = vec![0f32; n];
    let mut hint: Vec<Option<VReg>> = vec![None; n];
    let mut calls: Vec<u32> = vec![0];
    let mut pos = 2u32;
    let touch = |v: VReg, p: u32, start: &mut Vec<u32>, end: &mut Vec<u32>| {
        let i = v as usize;
        start[i] = start[i].min(p);
        end[i] = end[i].max(p);
    };
    for p in &f.params {
        touch(*p, 0, &mut start, &mut end);
    }
    for (bi, b) in f.blocks.iter().enumerate() {
        let head = pos;
        let w = 8f32.powi(depth[bi].min(6) as i32);
        for i in &b.insts {
            if i.is_call() {
                calls.push(pos);
            }
            if let Inst::Mov { d, s } = i {
                hint[*d as usize] = Some(*s);
                hint[*s as usize].get_or_insert(*d);
            }
            i.each(&mut |v, _| {
                touch(v, pos, &mut start, &mut end);
                weight[v as usize] += w;
            });
            pos += 2;
        }
        let mut t = b.term.clone();
        ssa::term_each_use(&mut t, &mut |v| {
            touch(*v, pos, &mut start, &mut end);
            weight[*v as usize] += w;
        });
        pos += 2;
        for w in 0..words {
            let mut bits = live_in[bi * words + w];
            while bits != 0 {
                let v = (w * 64) as u32 + bits.trailing_zeros();
                bits &= bits - 1;
                touch(v, head, &mut start, &mut end);
            }
            let mut bits = live_out[bi * words + w];
            while bits != 0 {
                let v = (w * 64) as u32 + bits.trailing_zeros();
                bits &= bits - 1;
                touch(v, pos - 1, &mut start, &mut end);
            }
        }
    }

    let mut list: Vec<Interval> = Vec::new();
    for v in 0..f.nvregs {
        let i = v as usize;
        if start[i] == u32::MAX || remat[i] {
            continue;
        }
        let at = calls.partition_point(|c| *c < start[i]);
        let touches = at < calls.len() && calls[at] <= end[i];
        let first_inside = calls.partition_point(|c| *c <= start[i]);
        let crosses = first_inside < calls.len() && calls[first_inside] < end[i];
        list.push(Interval { v, start: start[i], end: end[i], float: float[i], crosses, touches, weight: weight[i] / (end[i] - start[i] + 1) as f32 });
    }
    list.sort_by_key(|x| x.start);

    let mut active: Vec<(Interval, u32)> = Vec::new();
    let mut busy_x = 0u32;
    let mut busy_d = 0u32;
    let mut slots = 0u32;
    let mut used_x = 0u32;
    let mut used_d = 0u32;

    let candidates = |iv: &Interval| -> Vec<u32> {
        let mut out = Vec::new();
        if iv.float {
            if !iv.crosses {
                if !iv.touches {
                    out.extend(LOW_D);
                }
                out.extend(MID_D);
            }
            out.extend(SAVED_D);
        } else {
            if !iv.crosses {
                if !iv.touches {
                    out.extend(LOW_X);
                }
                out.extend(MID_X);
            }
            out.extend(SAVED_X);
        }
        out
    };

    for iv in list {
        active.retain(|(a, r)| {
            if a.end < iv.start {
                if a.float {
                    busy_d &= !(1 << r);
                } else {
                    busy_x &= !(1 << r);
                }
                false
            } else {
                true
            }
        });
        let options = candidates(&iv);
        let busy = if iv.float { busy_d } else { busy_x };
        let hinted = hint[iv.v as usize].and_then(|h| match places[h as usize] {
            Place::Reg(r) if !iv.float => Some(r),
            Place::Fpr(r) if iv.float => Some(r),
            _ => None,
        });
        let reg = hinted.filter(|r| options.contains(r) && busy >> r & 1 == 0).or_else(|| options.iter().copied().find(|r| busy >> r & 1 == 0));
        let chosen = match reg {
            Some(r) => Some(r),
            None => {
                // Take over from the cheapest value that is holding a usable register.
                let victim = active
                    .iter()
                    .enumerate()
                    .filter(|(_, (a, r))| a.float == iv.float && options.contains(r) && a.weight < iv.weight)
                    .min_by(|(_, (a, _)), (_, (b, _))| a.weight.total_cmp(&b.weight))
                    .map(|(i, _)| i);
                victim.map(|i| {
                    let (a, r) = active.swap_remove(i);
                    places[a.v as usize] = Place::Slot(slots);
                    slots += 1;
                    r
                })
            }
        };
        match chosen {
            Some(r) => {
                if iv.float {
                    busy_d |= 1 << r;
                    used_d |= 1 << r;
                    places[iv.v as usize] = Place::Fpr(r);
                } else {
                    busy_x |= 1 << r;
                    used_x |= 1 << r;
                    places[iv.v as usize] = Place::Reg(r);
                }
                active.push((iv, r));
            }
            None => {
                places[iv.v as usize] = Place::Slot(slots);
                slots += 1;
            }
        }
    }

    let saved_x = SAVED_X.iter().copied().filter(|r| used_x >> r & 1 != 0).collect();
    let saved_d = SAVED_D.iter().copied().filter(|r| used_d >> r & 1 != 0).collect();
    Alloc { places, saved_x, saved_d, slots }
}
