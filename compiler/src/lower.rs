//! Turns a checked AST into CIR.

use crate::ast::{self, BinOp, Coerce, Else, ElseKind, Expr, ExprKind, Part, Pat, Res, Stmt, StmtKind, UnOp};
use crate::cir::*;
use crate::diag::Span;
use crate::modules::Unit;
use crate::sema::{Checked, ConstVal};
use crate::types::*;
use std::collections::{HashMap, HashSet};

struct FnLower<'a, 'i> {
    c: &'a Checked,
    files: &'a [String],
    ids: &'i HashMap<String, FuncId>,
    vars: HashMap<String, VReg>,
    /// Scalars that had their address taken: `vars` holds their memory address.
    addressed: HashSet<String>,
    nvregs: u32,
    objects: Vec<(u64, u64)>,
    blocks: Vec<Block>,
    cur: BlockId,
    /// (continue target, break target, defer depth) of each enclosing loop.
    loops: Vec<(BlockId, BlockId, usize)>,
    /// Pending `defer` bodies of each open block, innermost last.
    defers: Vec<Vec<&'a ast::Block>>,
    ret: Ty,
    sret: Option<VReg>,
    is_main: bool,
}

enum Place {
    Reg(VReg),
    Mem(VReg, u64),
}

fn loc(s: Span) -> Loc {
    Loc { line: s.line, col: s.col, file: s.file }
}

pub fn lower<'a>(units: &'a [Unit], c: &'a Checked, files: &'a [String]) -> Module {
    let mut list: Vec<(String, &'a ast::Func)> = Vec::new();
    for u in units {
        for f in u.program.funcs.iter().filter(|f| f.tparams.is_empty()) {
            list.push((format!("{}{}", u.prefix, f.name), f));
        }
    }
    for f in &c.instances {
        list.push((f.name.clone(), f));
    }
    let ids: HashMap<String, FuncId> = list.iter().enumerate().map(|(i, (n, _))| (n.clone(), i)).collect();
    let funcs = list.iter().map(|(n, f)| lower_func(n, f, c, &ids, files)).collect();
    Module { funcs, main: ids["main"] }
}

fn lower_func<'a>(full: &str, f: &'a ast::Func, c: &'a Checked, ids: &HashMap<String, FuncId>, files: &'a [String]) -> Func {
    let sig = &c.sigs[full];
    let mut l = FnLower {
        c,
        files,
        ids,
        vars: HashMap::new(),
        addressed: f.addressed.borrow().clone(),
        nvregs: 0,
        objects: Vec::new(),
        blocks: Vec::new(),
        cur: 0,
        loops: Vec::new(),
        defers: Vec::new(),
        ret: sig.ret,
        sret: None,
        is_main: full == "main",
    };
    l.new_block();
    if c.types.is_aggregate(sig.ret) {
        l.sret = Some(l.vreg());
    }
    for p in &f.params {
        let v = l.vreg();
        l.vars.insert(p.name.clone(), v);
    }
    let nparams = f.params.len() + l.sret.is_some() as usize;
    l.block(&f.body);
    if sig.ret == VOID {
        l.finish(Term::Ret(None));
    } else if matches!(c.types.kind(sig.ret), Kind::Result(VOID, _)) {
        l.return_ok_void();
    } else {
        l.finish(Term::Unreachable);
    }
    Func { name: full.to_string(), nparams, nvregs: l.nvregs, objects: l.objects, blocks: l.blocks }
}

impl<'a, 'i> FnLower<'a, 'i> {
    fn t(&self) -> &'a Types {
        &self.c.types
    }

    fn vreg(&mut self) -> VReg {
        self.nvregs += 1;
        self.nvregs - 1
    }

    fn new_block(&mut self) -> BlockId {
        self.blocks.push(Block { insts: Vec::new(), term: Term::Unreachable });
        self.blocks.len() - 1
    }

    fn emit(&mut self, i: Inst) {
        self.blocks[self.cur].insts.push(i);
    }

    fn finish(&mut self, t: Term) {
        self.blocks[self.cur].term = t;
    }

    fn switch(&mut self, b: BlockId) {
        self.cur = b;
    }

    /// Ends the current block with `t` and continues in a fresh dead block,
    /// so statements after a jump still have somewhere to go.
    fn finish_dead(&mut self, t: Term) {
        self.finish(t);
        let dead = self.new_block();
        self.switch(dead);
    }

    fn constant(&mut self, v: i64) -> VReg {
        let d = self.vreg();
        self.emit(Inst::Const { d, v });
        d
    }

    fn temp_obj(&mut self, ty: Ty) -> VReg {
        let (size, align) = self.t().layout(ty);
        self.objects.push((size.max(1), align));
        let d = self.vreg();
        self.emit(Inst::ObjAddr { d, obj: self.objects.len() - 1 });
        d
    }

    fn mem_of(&self, ty: Ty) -> Mem {
        match self.t().kind(ty) {
            Kind::Int { bits: 8, signed: true } => Mem::I8,
            Kind::Int { bits: 16, signed: true } => Mem::I16,
            Kind::Int { bits: 32, signed: true } => Mem::I32,
            Kind::Int { bits: 8, signed: false } | Kind::Bool => Mem::U8,
            Kind::Int { bits: 16, signed: false } => Mem::U16,
            Kind::Int { bits: 32, signed: false } | Kind::Float { bits: 32 } => Mem::U32,
            _ => Mem::X64,
        }
    }

    fn num_of(&self, ty: Ty) -> Num {
        match self.t().kind(ty) {
            Kind::Int { bits, signed } => Num { bits, signed },
            Kind::Pointer(_) => Num { bits: 64, signed: false },
            _ => Num { bits: 64, signed: true },
        }
    }

    fn float_bits(&self, ty: Ty) -> Option<u8> {
        match self.t().kind(ty) {
            Kind::Float { bits } => Some(bits),
            _ => None,
        }
    }

    fn offset_addr(&mut self, base: VReg, off: u64) -> VReg {
        if off == 0 {
            return base;
        }
        let d = self.vreg();
        self.emit(Inst::AddImm { d, s: base, imm: off });
        d
    }

    fn store(&mut self, addr: VReg, off: u64, v: VReg, ty: Ty) {
        if self.t().is_aggregate(ty) {
            let dst = self.offset_addr(addr, off);
            self.emit(Inst::MemCopy { dst, src: v, size: self.t().size(ty) });
        } else {
            self.emit(Inst::Store { p: addr, off, s: v, mem: self.mem_of(ty) });
        }
    }

    fn load(&mut self, addr: VReg, off: u64, ty: Ty) -> VReg {
        if self.t().is_aggregate(ty) {
            return self.offset_addr(addr, off);
        }
        let d = self.vreg();
        self.emit(Inst::Load { d, p: addr, off, mem: self.mem_of(ty) });
        d
    }

    /// A fresh local holding a copy of `v`.
    fn new_local(&mut self, v: VReg, ty: Ty) -> VReg {
        if self.t().is_aggregate(ty) {
            let a = self.temp_obj(ty);
            self.store(a, 0, v, ty);
            a
        } else {
            let d = self.vreg();
            self.emit(Inst::Mov { d, s: v });
            d
        }
    }

    fn set_tag(&mut self, addr: VReg, tag: i64) {
        let t = self.constant(tag);
        self.emit(Inst::Store { p: addr, off: 0, s: t, mem: Mem::X64 });
    }

    fn return_ok_void(&mut self) {
        let sret = self.sret.expect("Result is returned in memory");
        self.set_tag(sret, 0);
        self.finish(Term::Ret(None));
    }

    fn block(&mut self, b: &'a ast::Block) {
        self.defers.push(Vec::new());
        for s in &b.stmts {
            self.stmt(s);
        }
        let depth = self.defers.len() - 1;
        self.run_defers(depth);
        self.defers.pop();
    }

    /// Emits the pending `defer` bodies of every block from `from` inward, newest first.
    fn run_defers(&mut self, from: usize) {
        for level in (from..self.defers.len()).rev() {
            let pending = self.defers[level].clone();
            for body in pending.into_iter().rev() {
                self.block(body);
            }
        }
    }

    fn place(&mut self, e: &'a Expr) -> Place {
        match &e.kind {
            ExprKind::Var(name) => {
                let v = self.vars[name];
                if self.t().is_aggregate(e.ty.get()) || self.addressed.contains(name) {
                    Place::Mem(v, 0)
                } else {
                    Place::Reg(v)
                }
            }
            ExprKind::Field { e: base, name, .. } => {
                let off = self.field_offset(base.ty.get(), name);
                if self.t().is_ptr(base.ty.get()) {
                    return Place::Mem(self.expr_value(base), off);
                }
                match self.place(base) {
                    Place::Mem(a, o) => Place::Mem(a, o + off),
                    Place::Reg(_) => unreachable!("structs live in memory"),
                }
            }
            ExprKind::Deref(inner) => Place::Mem(self.expr_value(inner), 0),
            ExprKind::Index { e: base, i } if self.t().is_ptr(base.ty.get()) => {
                let a = self.expr_value(base);
                Place::Mem(self.element_addr(base.ty.get(), a, i, e.span), 0)
            }
            ExprKind::Index { e: base, i } => {
                let Place::Mem(a, o) = self.place(base) else { unreachable!("arrays live in memory") };
                let addr = self.offset_addr(a, o);
                Place::Mem(self.element_addr(base.ty.get(), addr, i, e.span), 0)
            }
            _ => unreachable!("sema checked the assignment target"),
        }
    }

    fn field_offset(&self, ty: Ty, name: &str) -> u64 {
        let ty = match self.t().kind(ty) {
            Kind::Pointer(p) => p,
            _ => ty,
        };
        let Kind::Struct(id) = self.t().kind(ty) else { unreachable!("field of a non-struct") };
        self.t().structs[id].fields.iter().find(|f| f.name == name).unwrap().offset
    }

    fn element_addr(&mut self, array: Ty, addr: VReg, i: &'a Expr, span: Span) -> VReg {
        if let Kind::Pointer(elem) = self.t().kind(array) {
            // A raw pointer has no length, so there is nothing to check.
            let idx = self.expr_value(i);
            let d = self.vreg();
            self.emit(Inst::IndexAddr { d, base: addr, idx, size: self.t().size(elem) });
            return d;
        }
        let Kind::Array(elem, n) = self.t().kind(array) else { unreachable!("index of a non-array") };
        let idx = self.expr_value(i);
        self.emit(Inst::Bounds { idx, len: n, loc: loc(span) });
        let d = self.vreg();
        self.emit(Inst::IndexAddr { d, base: addr, idx, size: self.t().size(elem) });
        d
    }

    fn stmt(&mut self, s: &'a Stmt) {
        match &s.kind {
            StmtKind::Let { name, init, .. } => {
                let ty = init.ty.get();
                let ty = match init.coerce.get() {
                    Coerce::Some(t) | Coerce::Ok(t) => t,
                    Coerce::No => ty,
                };
                let v = self.expr_value(init);
                // A value just built in its own temporary needs no second copy.
                let fresh = init.coerce.get() != Coerce::No
                    || matches!(init.kind, ExprKind::ArrayLit(_) | ExprKind::ArrayFill(_) | ExprKind::StructLit { .. } | ExprKind::Call { .. } | ExprKind::None);
                let d = if self.addressed.contains(name) && !self.t().is_aggregate(ty) {
                    let a = self.temp_obj(ty);
                    self.store(a, 0, v, ty);
                    a
                } else if fresh && self.t().is_aggregate(ty) {
                    v
                } else {
                    self.new_local(v, ty)
                };
                self.vars.insert(name.clone(), d);
            }
            StmtKind::Assign { target, op, value } => self.assign(target, *op, value),
            StmtKind::Discard(e) => {
                self.expr(e);
            }
            StmtKind::Expr(e) => {
                self.expr(e);
            }
            StmtKind::If { cond, then, els } => self.if_stmt(cond, then, els.as_ref()),
            StmtKind::ForRange { var, start, end, body, .. } => {
                let a = self.expr_value(start);
                let limit = self.expr_value(end);
                // Separate hidden copies: the bounds are evaluated exactly once.
                let counter = self.vreg();
                self.emit(Inst::Mov { d: counter, s: a });
                let lim = self.vreg();
                self.emit(Inst::Mov { d: lim, s: limit });
                self.counted_loop(counter, lim, |l| {
                    if var != "_" {
                        let v = l.vreg();
                        l.emit(Inst::Mov { d: v, s: counter });
                        l.vars.insert(var.clone(), v);
                    }
                    l.block(body);
                });
            }
            StmtKind::ForIn { elem, index, subject, body, .. } => self.for_in(elem, index.as_ref(), subject, body),
            StmtKind::ForCond { cond, body } => {
                let head = self.new_block();
                let body_b = self.new_block();
                let exit = self.new_block();
                self.finish(Term::Jmp(head));
                self.switch(head);
                self.branch(cond, body_b, exit);
                self.switch(body_b);
                self.loops.push((head, exit, self.defers.len()));
                self.block(body);
                self.loops.pop();
                self.finish(Term::Jmp(head));
                self.switch(exit);
            }
            StmtKind::ForEver { body } => {
                let head = self.new_block();
                let exit = self.new_block();
                self.finish(Term::Jmp(head));
                self.switch(head);
                self.loops.push((head, exit, self.defers.len()));
                self.block(body);
                self.loops.pop();
                self.finish(Term::Jmp(head));
                self.switch(exit);
            }
            StmtKind::Match { subject, arms } => self.match_stmt(subject, arms),
            StmtKind::Ret(v) => self.ret_stmt(v.as_ref()),
            StmtKind::Fail(e) => {
                let Kind::Result(_, err) = self.t().kind(self.ret) else { unreachable!("sema checked `fail`") };
                let v = self.expr_value(e);
                let sret = self.sret.expect("Result is returned in memory");
                self.set_tag(sret, 1);
                self.store(sret, TAG_SIZE, v, err);
                self.run_defers(0);
                self.finish_dead(Term::Ret(None));
            }
            StmtKind::Defer(body) => self.defers.last_mut().unwrap().push(body),
            StmtKind::Break => {
                let (_, t, depth) = *self.loops.last().unwrap();
                self.run_defers(depth);
                self.finish_dead(Term::Jmp(t));
            }
            StmtKind::Continue => {
                let (t, _, depth) = *self.loops.last().unwrap();
                self.run_defers(depth);
                self.finish_dead(Term::Jmp(t));
            }
        }
    }

    /// `for` over `counter..lim`; the body may use `break` and `continue`.
    fn counted_loop(&mut self, counter: VReg, lim: VReg, body: impl FnOnce(&mut Self)) {
        let head = self.new_block();
        let body_b = self.new_block();
        let latch = self.new_block();
        let exit = self.new_block();
        self.finish(Term::Jmp(head));

        self.switch(head);
        let c = self.vreg();
        self.emit(Inst::Cmp { cc: Cc::Lt, d: c, a: counter, b: lim, unsigned: false });
        self.finish(Term::Br { c, t: body_b, f: exit });

        self.switch(body_b);
        self.loops.push((latch, exit, self.defers.len()));
        body(self);
        self.loops.pop();
        self.finish(Term::Jmp(latch));

        self.switch(latch);
        self.emit(Inst::Inc { d: counter });
        self.finish(Term::Jmp(head));
        self.switch(exit);
    }

    fn for_in(&mut self, elem: &str, index: Option<&(String, Span)>, subject: &'a Expr, body: &'a ast::Block) {
        let Kind::Array(elem_ty, n) = self.t().kind(subject.ty.get()) else { unreachable!("sema checked `for in`") };
        let base = self.expr_value(subject);
        let counter = self.constant(0);
        let lim = self.constant(n as i64);
        let size = self.t().size(elem_ty);
        self.counted_loop(counter, lim, |l| {
            if elem != "_" {
                let a = l.vreg();
                l.emit(Inst::IndexAddr { d: a, base, idx: counter, size });
                // The array cannot change during the loop, so aggregates can be read in place.
                let v = l.load(a, 0, elem_ty);
                let d = if l.t().is_aggregate(elem_ty) { v } else { l.new_local(v, elem_ty) };
                l.vars.insert(elem.to_string(), d);
            }
            if let Some((name, _)) = index {
                if name != "_" {
                    let v = l.vreg();
                    l.emit(Inst::Mov { d: v, s: counter });
                    l.vars.insert(name.clone(), v);
                }
            }
            l.block(body);
        });
    }

    fn ret_stmt(&mut self, v: Option<&'a Expr>) {
        match v {
            None => {
                self.run_defers(0);
                if matches!(self.t().kind(self.ret), Kind::Result(VOID, _)) {
                    self.return_ok_void();
                    let dead = self.new_block();
                    self.switch(dead);
                } else {
                    self.finish_dead(Term::Ret(None));
                }
            }
            Some(e) => {
                let r = self.expr_value(e);
                if self.t().is_aggregate(self.ret) {
                    let sret = self.sret.expect("aggregates are returned in memory");
                    self.store(sret, 0, r, self.ret);
                    self.run_defers(0);
                    self.finish_dead(Term::Ret(None));
                } else {
                    // The value is computed first: a `defer` must not change what is returned.
                    self.run_defers(0);
                    self.finish_dead(Term::Ret(Some(r)));
                }
            }
        }
    }

    fn assign(&mut self, target: &'a Expr, op: Option<BinOp>, value: &'a Expr) {
        let ty = target.ty.get();
        let p = self.place(target);
        match op {
            None => {
                let v = self.expr_value(value);
                self.write_place(p, v, ty);
            }
            Some(op) => {
                let old = match p {
                    Place::Reg(r) => r,
                    Place::Mem(a, o) => self.load(a, o, ty),
                };
                let v = self.expr_value(value);
                let d = self.arith(op, old, v, ty, target.span);
                self.write_place(p, d, ty);
            }
        }
    }

    fn write_place(&mut self, p: Place, v: VReg, ty: Ty) {
        match p {
            Place::Reg(r) => self.emit(Inst::Mov { d: r, s: v }),
            Place::Mem(a, o) => self.store(a, o, v, ty),
        }
    }

    fn if_stmt(&mut self, cond: &'a Expr, then: &'a ast::Block, els: Option<&'a Else>) {
        let t = self.new_block();
        let join = self.new_block();
        let f = if els.is_some() { self.new_block() } else { join };
        self.branch(cond, t, f);
        self.switch(t);
        self.block(then);
        self.finish(Term::Jmp(join));
        if let Some(e) = els {
            self.switch(f);
            match e {
                Else::Block(b) => self.block(b),
                Else::If(i) => self.stmt(i),
            }
            self.finish(Term::Jmp(join));
        }
        self.switch(join);
    }

    fn eq_branch(&mut self, a: VReg, v: i64, t: BlockId, f: BlockId) {
        let k = self.constant(v);
        let c = self.vreg();
        self.emit(Inst::Cmp { cc: Cc::Eq, d: c, a, b: k, unsigned: false });
        self.finish(Term::Br { c, t, f });
    }

    fn match_stmt(&mut self, subject: &'a Expr, arms: &'a [ast::Arm]) {
        let ty = subject.ty.get();
        let v = self.expr_value(subject);
        let enum_id = match self.t().kind(ty) {
            Kind::Enum(id) => Some(id),
            _ => None,
        };
        let key = if enum_id.is_some() {
            let d = self.vreg();
            self.emit(Inst::Load { d, p: v, off: 0, mem: Mem::X64 });
            d
        } else {
            v
        };
        let join = self.new_block();
        for arm in arms {
            let body = self.new_block();
            let next = self.new_block();
            match &arm.pat {
                Pat::Wild => self.finish(Term::Jmp(body)),
                Pat::Bool(b) => self.eq_branch(key, *b as i64, body, next),
                Pat::Int(n) => self.eq_branch(key, *n as i64, body, next),
                Pat::Variant { name, .. } => {
                    let tag = self.variant_index(enum_id.unwrap(), name);
                    self.eq_branch(key, tag as i64, body, next);
                }
            }
            self.switch(body);
            if let Pat::Variant { name, binds } = &arm.pat {
                let id = enum_id.unwrap();
                let idx = self.variant_index(id, name);
                let fields = self.t().enums[id].variants[idx].fields.clone();
                for ((b, _), f) in binds.iter().zip(&fields) {
                    if b != "_" {
                        let x = self.load(v, f.offset, f.ty);
                        let d = self.new_local(x, f.ty);
                        self.vars.insert(b.clone(), d);
                    }
                }
            }
            self.block(&arm.body);
            self.finish(Term::Jmp(join));
            self.switch(next);
        }
        self.finish(Term::Jmp(join));
        self.switch(join);
    }

    fn variant_index(&self, id: usize, name: &str) -> usize {
        self.t().enums[id].variants.iter().position(|v| v.name == name).unwrap()
    }

    /// Branches on a condition, short-circuiting `and` / `or` / `not`.
    fn branch(&mut self, e: &'a Expr, t: BlockId, f: BlockId) {
        match &e.kind {
            ExprKind::Binary { op: BinOp::And, l, r } => {
                let mid = self.new_block();
                self.branch(l, mid, f);
                self.switch(mid);
                self.branch(r, t, f);
            }
            ExprKind::Binary { op: BinOp::Or, l, r } => {
                let mid = self.new_block();
                self.branch(l, t, mid);
                self.switch(mid);
                self.branch(r, t, f);
            }
            ExprKind::Unary { op: UnOp::Not, e } => self.branch(e, f, t),
            _ => {
                let c = self.expr_value(e);
                self.finish(Term::Br { c, t, f });
            }
        }
    }

    fn expr_value(&mut self, e: &'a Expr) -> VReg {
        self.expr(e).expect("sema guarantees a value")
    }

    fn expr(&mut self, e: &'a Expr) -> Option<VReg> {
        let v = self.expr_raw(e);
        match e.coerce.get() {
            Coerce::No => v,
            Coerce::Some(t) => {
                let a = self.temp_obj(t);
                self.set_tag(a, 1);
                self.store(a, TAG_SIZE, v.expect("a value to wrap"), e.ty.get());
                Some(a)
            }
            Coerce::Ok(t) => {
                let a = self.temp_obj(t);
                self.set_tag(a, 0);
                if let Some(v) = v {
                    self.store(a, TAG_SIZE, v, e.ty.get());
                }
                Some(a)
            }
        }
    }

    fn float_const(&mut self, v: f64, ty: Ty) -> VReg {
        let raw = if self.float_bits(ty) == Some(32) { (v as f32).to_bits() as i64 } else { v.to_bits() as i64 };
        self.constant(raw)
    }

    fn pointer_step(&mut self, op: BinOp, a: VReg, b: VReg, ty: Ty) -> VReg {
        let Kind::Pointer(elem) = self.t().kind(ty) else { unreachable!("pointer arithmetic") };
        let idx = if op == BinOp::Sub {
            let zero = self.constant(0);
            let n = self.vreg();
            let at = Loc { line: 0, col: 0, file: 0 };
            self.emit(Inst::Bin { op: Arith::Sub, d: n, a: zero, b, num: Num { bits: 64, signed: false }, loc: at });
            n
        } else {
            b
        };
        let d = self.vreg();
        self.emit(Inst::IndexAddr { d, base: a, idx, size: self.t().size(elem) });
        d
    }

    fn pointer_diff(&mut self, a: VReg, b: VReg, ty: Ty, span: Span) -> VReg {
        let Kind::Pointer(elem) = self.t().kind(ty) else { unreachable!("pointer difference") };
        let bytes = self.vreg();
        self.emit(Inst::Bin { op: Arith::Sub, d: bytes, a, b, num: Num { bits: 64, signed: false }, loc: loc(span) });
        let size = self.constant(self.t().size(elem).max(1) as i64);
        let d = self.vreg();
        self.emit(Inst::Bin { op: Arith::Div, d, a: bytes, b: size, num: Num { bits: 64, signed: true }, loc: loc(span) });
        d
    }

    fn arith(&mut self, op: BinOp, a: VReg, b: VReg, ty: Ty, span: Span) -> VReg {
        if self.t().is_ptr(ty) {
            return self.pointer_step(op, a, b, ty);
        }
        let d = self.vreg();
        if let Some(bits) = self.float_bits(ty) {
            let fop = match op {
                BinOp::Add => FArith::Add,
                BinOp::Sub => FArith::Sub,
                BinOp::Mul => FArith::Mul,
                BinOp::Div => FArith::Div,
                _ => unreachable!("sema rejects this float operator"),
            };
            self.emit(Inst::FBin { op: fop, d, a, b, bits });
        } else {
            self.emit(Inst::Bin { op: arith(op), d, a, b, num: self.num_of(ty), loc: loc(span) });
        }
        d
    }

    fn expr_raw(&mut self, e: &'a Expr) -> Option<VReg> {
        let ty = e.ty.get();
        Some(match &e.kind {
            ExprKind::Int(v) => {
                if self.float_bits(ty).is_some() {
                    self.float_const(*v as f64, ty)
                } else {
                    self.constant(*v as i64)
                }
            }
            ExprKind::Float(v) => self.float_const(*v, ty),
            ExprKind::Char(c) => self.constant(*c as i64),
            ExprKind::Bool(b) => self.constant(*b as i64),
            ExprKind::Str(text) => {
                let d = self.vreg();
                self.emit(Inst::StrAddr { d, text: text.clone() });
                d
            }
            ExprKind::Interp(_) => unreachable!("interpolation only occurs inside print"),
            ExprKind::AddrOf(inner) => {
                let Place::Mem(a, o) = self.place(inner) else { unreachable!("sema made this addressable") };
                self.offset_addr(a, o)
            }
            ExprKind::Deref(inner) => {
                let p = self.expr_value(inner);
                self.load(p, 0, ty)
            }
            ExprKind::RetVal => unreachable!("contracts are not lowered in 0.2"),
            ExprKind::None if self.t().is_ptr(ty) => self.constant(0),
            ExprKind::None => {
                let a = self.temp_obj(ty);
                self.set_tag(a, 0);
                a
            }
            ExprKind::Var(name) => return Some(self.var(e, name, ty)),
            ExprKind::Unary { op, e: inner } => self.unary(*op, inner, ty, e.span),
            ExprKind::Binary { op, l, r } => match op {
                BinOp::And | BinOp::Or => return Some(self.short_circuit(e)),
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                    let operand = l.ty.get();
                    let a = self.expr_value(l);
                    let b = self.expr_value(r);
                    let d = self.vreg();
                    if let Some(bits) = self.float_bits(operand) {
                        self.emit(Inst::FCmp { cc: cc(*op), d, a, b, bits });
                    } else {
                        let unsigned = !self.num_of(operand).signed;
                        self.emit(Inst::Cmp { cc: cc(*op), d, a, b, unsigned });
                    }
                    d
                }
                _ => {
                    let a = self.expr_value(l);
                    let b = self.expr_value(r);
                    if self.t().is_ptr(l.ty.get()) && self.t().is_ptr(r.ty.get()) {
                        self.pointer_diff(a, b, l.ty.get(), e.span)
                    } else {
                        self.arith(*op, a, b, l.ty.get(), e.span)
                    }
                }
            },
            ExprKind::Call { name, args, .. } => return self.call(e, name, args),
            ExprKind::StructLit { args, .. } => self.struct_lit(e, args, ty),
            ExprKind::ArrayLit(items) => {
                let Kind::Array(elem, _) = self.t().kind(ty) else { unreachable!("array literal type") };
                let a = self.temp_obj(ty);
                let size = self.t().size(elem);
                for (i, it) in items.iter().enumerate() {
                    let v = self.expr_value(it);
                    self.store(a, i as u64 * size, v, elem);
                }
                a
            }
            ExprKind::ArrayFill(item) => {
                let Kind::Array(elem, n) = self.t().kind(ty) else { unreachable!("array fill type") };
                let a = self.temp_obj(ty);
                let v = self.expr_value(item);
                let counter = self.constant(0);
                let lim = self.constant(n as i64);
                let size = self.t().size(elem);
                self.counted_loop(counter, lim, |l| {
                    let d = l.vreg();
                    l.emit(Inst::IndexAddr { d, base: a, idx: counter, size });
                    l.store(d, 0, v, elem);
                });
                a
            }
            ExprKind::Field { .. } if !matches!(*e.res.borrow(), Res::None) => self.resolved(e, ty),
            ExprKind::Field { e: base, name, .. } => {
                let off = self.field_offset(base.ty.get(), name);
                let a = self.expr_value(base);
                self.load(a, off, ty)
            }
            ExprKind::Index { e: base, i } => {
                let a = self.expr_value(base);
                let d = self.element_addr(base.ty.get(), a, i, e.span);
                self.load(d, 0, ty)
            }
            ExprKind::Try(inner) => return self.try_expr(e, inner),
            ExprKind::Else { e: inner, handler } => return self.else_expr(inner, handler, ty),
        })
    }

    fn var(&mut self, e: &Expr, name: &str, ty: Ty) -> VReg {
        if let Some(&v) = self.vars.get(name) {
            if self.t().is_aggregate(ty) {
                return v;
            }
            if self.addressed.contains(name) {
                return self.load(v, 0, ty);
            }
            // Copy so a later assignment cannot change an operand that was
            // already evaluated (left-to-right order is guaranteed).
            let d = self.vreg();
            self.emit(Inst::Mov { d, s: v });
            return d;
        }
        self.resolved(e, ty)
    }

    /// A constant or a field-less variant, as sema resolved it.
    fn resolved(&mut self, e: &Expr, ty: Ty) -> VReg {
        match *e.res.borrow() {
            Res::Const(ConstVal::Int(n), _) => self.constant(n as i64),
            Res::Const(ConstVal::Bool(b), _) => self.constant(b as i64),
            Res::Const(ConstVal::Float(x), t) => self.float_const(x, t),
            Res::Variant(_, idx) => {
                let a = self.temp_obj(ty);
                self.set_tag(a, idx as i64);
                a
            }
            _ => unreachable!("sema resolved this name"),
        }
    }

    fn unary(&mut self, op: UnOp, inner: &'a Expr, ty: Ty, span: Span) -> VReg {
        let s = self.expr_value(inner);
        let d = self.vreg();
        match op {
            UnOp::Not => self.emit(Inst::BoolNot { d, s }),
            UnOp::BitNot => self.emit(Inst::BitNot { d, s, num: self.num_of(ty) }),
            UnOp::Neg => match self.float_bits(ty) {
                Some(bits) => self.emit(Inst::FNeg { d, s, bits }),
                None => self.emit(Inst::Neg { d, s, num: self.num_of(ty), loc: loc(span) }),
            },
        }
        d
    }

    fn call(&mut self, e: &'a Expr, name: &str, args: &'a [Expr]) -> Option<VReg> {
        let ty = e.ty.get();
        let res = e.res.borrow().clone();
        let full = match res {
            Res::Const(..) => return Some(self.resolved(e, ty)),
            Res::Func(full) => full,
            _ => return self.builtin_call(e, name, args),
        };
        let sig = &self.c.sigs[&full];
        if let Some(sym) = &sig.ext {
            return self.call_c(sym.clone(), sig.params.len(), sig.variadic, args, ty);
        }
        let aggregate = self.t().is_aggregate(ty);
        // Results land in a fresh temporary, so `x = f(x)` never overlaps.
        let mut vals = Vec::with_capacity(args.len() + 1);
        let sret = if aggregate { Some(self.temp_obj(ty)) } else { None };
        vals.extend(sret);
        for a in args {
            let v = self.expr_value(a);
            vals.push(v);
        }
        let d = if ty == VOID || aggregate { None } else { Some(self.vreg()) };
        self.emit(Inst::Call { d, f: self.ids[&full], args: vals });
        if aggregate {
            sret
        } else {
            d
        }
    }

    fn builtin_call(&mut self, e: &'a Expr, name: &str, args: &'a [Expr]) -> Option<VReg> {
        let ty = e.ty.get();
        match name {
            "print" => {
                self.print(&args[0]);
                None
            }
            "len" => {
                let Kind::Array(_, n) = self.t().kind(args[0].ty.get()) else { unreachable!("sema checked `len`") };
                self.expr(&args[0]);
                Some(self.constant(n as i64))
            }
            "alloc" => {
                let Kind::Pointer(elem) = self.t().kind(ty) else { unreachable!("sema checked `alloc`") };
                let n = self.expr_value(&args[0]);
                let d = self.vreg();
                self.emit(Inst::Alloc { d, n, elem: self.t().size(elem), loc: loc(e.span) });
                Some(d)
            }
            "free" => {
                let p = self.expr_value(&args[0]);
                self.emit(Inst::Free { p });
                None
            }
            "cast" => {
                let s = self.expr_value(&args[0]);
                let d = self.vreg();
                self.emit(Inst::Mov { d, s });
                Some(d)
            }
            _ => Some(self.cast(&args[0], ty, e.span)),
        }
    }

    fn call_c(&mut self, sym: String, fixed: usize, variadic: bool, args: &'a [Expr], ty: Ty) -> Option<VReg> {
        let mut out = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            let mut v = self.expr_value(a);
            let mut at = a.ty.get();
            // C's default promotion: a variadic float travels as a double.
            if variadic && i >= fixed && at == F32 {
                let d = self.vreg();
                self.emit(Inst::FloatCast { d, s: v, to_bits: 64 });
                v = d;
                at = F64;
            }
            out.push(CArg { v, float: self.float_bits(at), size: self.t().size(at) as u32 });
        }
        let ret = match self.t().kind(ty) {
            Kind::Void => CRet::None,
            Kind::Float { bits } => CRet::Float(bits),
            _ => CRet::Int(self.mem_of(ty)),
        };
        let d = if ty == VOID { None } else { Some(self.vreg()) };
        self.emit(Inst::CallC { d, sym, args: out, variadic: variadic.then_some(fixed), ret });
        d
    }

    fn cast(&mut self, arg: &'a Expr, to: Ty, span: Span) -> VReg {
        let from = arg.ty.get();
        let s = self.expr_value(arg);
        let d = self.vreg();
        let t = self.t();
        match (t.kind(from), t.kind(to)) {
            (Kind::Bool, _) => self.emit(Inst::Mov { d, s }),
            (Kind::Int { .. }, Kind::Int { .. }) => {
                self.emit(Inst::IntCast { d, s, from: self.num_of(from), to: self.num_of(to), loc: loc(span) });
            }
            (Kind::Int { signed, .. }, Kind::Float { bits }) => self.emit(Inst::IntToFloat { d, s, signed, bits }),
            (Kind::Float { bits }, Kind::Int { .. }) => {
                self.emit(Inst::FloatToInt { d, s, bits, to: self.num_of(to), loc: loc(span) });
            }
            (Kind::Float { bits: a }, Kind::Float { bits: b }) if a != b => self.emit(Inst::FloatCast { d, s, to_bits: b }),
            _ => self.emit(Inst::Mov { d, s }),
        }
        d
    }

    fn struct_lit(&mut self, e: &'a Expr, args: &'a [Expr], ty: Ty) -> VReg {
        let a = self.temp_obj(ty);
        let res = e.res.borrow().clone();
        let fields = match res {
            Res::Variant(eid, idx) => {
                self.set_tag(a, idx as i64);
                self.t().enums[eid].variants[idx].fields.clone()
            }
            _ => {
                let Kind::Struct(id) = self.t().kind(ty) else { unreachable!("struct literal type") };
                self.t().structs[id].fields.clone()
            }
        };
        for (arg, f) in args.iter().zip(&fields) {
            let v = self.expr_value(arg);
            self.store(a, f.offset, v, f.ty);
        }
        a
    }

    fn success_tag(&self, ty: Ty) -> i64 {
        match self.t().kind(ty) {
            Kind::Option(_) => 1,
            _ => 0,
        }
    }

    fn payload_of(&self, ty: Ty) -> Ty {
        match self.t().kind(ty) {
            Kind::Option(p) => p,
            Kind::Result(ok, _) => ok,
            _ => unreachable!("only ?T and T ! E have a payload"),
        }
    }

    fn try_expr(&mut self, e: &'a Expr, inner: &'a Expr) -> Option<VReg> {
        let ty = inner.ty.get();
        let v = self.expr_value(inner);
        let tag = self.vreg();
        self.emit(Inst::Load { d: tag, p: v, off: 0, mem: Mem::X64 });
        let ok = self.new_block();
        let bad = self.new_block();
        let want = self.success_tag(ty);
        self.eq_branch(tag, want, ok, bad);

        self.switch(bad);
        if self.is_main {
            self.run_defers(0);
            self.emit(Inst::BufStr { text: "error: ".to_string() });
            match self.t().kind(ty) {
                Kind::Result(_, err) => {
                    let x = self.load(v, TAG_SIZE, err);
                    self.print_value(x, err);
                }
                _ => self.emit(Inst::BufStr { text: "none".to_string() }),
            }
            let text = format!(" at {}:{}:{}\n", self.files[e.span.file as usize], e.span.line, e.span.col);
            self.emit(Inst::BufStr { text });
            self.emit(Inst::Flush { fd: 2 });
            self.emit(Inst::Exit { code: 1 });
            self.finish(Term::Unreachable);
        } else {
            let sret = self.sret.expect("`?` needs a function returning in memory");
            match self.t().kind(ty) {
                Kind::Result(_, err) => {
                    self.set_tag(sret, 1);
                    let x = self.load(v, TAG_SIZE, err);
                    self.store(sret, TAG_SIZE, x, err);
                }
                _ => self.set_tag(sret, 0),
            }
            self.run_defers(0);
            self.finish(Term::Ret(None));
        }

        self.switch(ok);
        let payload = self.payload_of(ty);
        if payload == VOID {
            None
        } else {
            Some(self.load(v, TAG_SIZE, payload))
        }
    }

    fn else_expr(&mut self, inner: &'a Expr, handler: &'a ElseKind, payload: Ty) -> Option<VReg> {
        let ty = inner.ty.get();
        let v = self.expr_value(inner);
        let tag = self.vreg();
        self.emit(Inst::Load { d: tag, p: v, off: 0, mem: Mem::X64 });
        let ok = self.new_block();
        let bad = self.new_block();
        let join = self.new_block();
        let want = self.success_tag(ty);
        self.eq_branch(tag, want, ok, bad);

        // One result location, written on whichever path runs.
        let result = if payload == VOID {
            None
        } else if self.t().is_aggregate(payload) {
            Some(self.temp_obj(payload))
        } else {
            Some(self.vreg())
        };

        self.switch(ok);
        if let Some(r) = result {
            let x = self.load(v, TAG_SIZE, payload);
            self.write_result(r, x, payload);
        }
        self.finish(Term::Jmp(join));

        self.switch(bad);
        match handler {
            ElseKind::Value(d) => {
                let x = self.expr_value(d);
                self.write_result(result.expect("sema checked the fallback"), x, payload);
                self.finish(Term::Jmp(join));
            }
            ElseKind::Block(binder, b) => {
                if let (Some((name, _)), Kind::Result(_, err)) = (binder, self.t().kind(ty)) {
                    let x = self.load(v, TAG_SIZE, err);
                    let d = self.new_local(x, err);
                    self.vars.insert(name.clone(), d);
                }
                self.block(b);
                self.finish(Term::Unreachable);
            }
        }
        self.switch(join);
        result
    }

    fn write_result(&mut self, r: VReg, x: VReg, ty: Ty) {
        if self.t().is_aggregate(ty) {
            self.store(r, 0, x, ty);
        } else {
            self.emit(Inst::Mov { d: r, s: x });
        }
    }

    fn print(&mut self, a: &'a Expr) {
        // Evaluate first: a trap in an argument must not leave half a line.
        enum Piece<'b> {
            Lit(&'b str),
            Val(VReg, Ty),
        }
        let mut pieces = Vec::new();
        match &a.kind {
            ExprKind::Str(s) => pieces.push(Piece::Lit(s)),
            ExprKind::Interp(parts) => {
                for p in parts {
                    match p {
                        Part::Lit(s) => pieces.push(Piece::Lit(s)),
                        Part::Expr(e) => pieces.push(Piece::Val(self.expr_value(e), e.ty.get())),
                    }
                }
            }
            _ => pieces.push(Piece::Val(self.expr_value(a), a.ty.get())),
        }
        for p in pieces {
            match p {
                Piece::Lit(s) => self.emit(Inst::BufStr { text: s.to_string() }),
                Piece::Val(v, t) => self.print_value(v, t),
            }
        }
        self.emit(Inst::BufStr { text: "\n".to_string() });
        self.emit(Inst::Flush { fd: 1 });
    }

    fn lit(&mut self, s: &str) {
        self.emit(Inst::BufStr { text: s.to_string() });
    }

    fn print_fields(&mut self, name: &str, addr: VReg, fields: &[Field]) {
        self.lit(name);
        if fields.is_empty() {
            return;
        }
        self.lit("{");
        for (i, f) in fields.iter().enumerate() {
            if i > 0 {
                self.lit(", ");
            }
            let x = self.load(addr, f.offset, f.ty);
            self.print_value(x, f.ty);
        }
        self.lit("}");
    }

    fn print_value(&mut self, v: VReg, ty: Ty) {
        match self.t().kind(ty) {
            Kind::Int { signed, .. } => self.emit(Inst::BufInt { s: v, signed }),
            Kind::Pointer(_) => self.emit(Inst::BufInt { s: v, signed: false }),
            Kind::Bool => self.emit(Inst::BufBool { s: v }),
            Kind::Float { bits } => self.emit(Inst::BufFloat { s: v, bits }),
            Kind::Struct(id) => {
                let def = self.t().structs[id].clone();
                self.print_fields(&def.name, v, &def.fields);
            }
            Kind::Enum(id) => {
                let def = self.t().enums[id].clone();
                let tag = self.vreg();
                self.emit(Inst::Load { d: tag, p: v, off: 0, mem: Mem::X64 });
                let join = self.new_block();
                for (i, var) in def.variants.iter().enumerate() {
                    let body = self.new_block();
                    let next = self.new_block();
                    self.eq_branch(tag, i as i64, body, next);
                    self.switch(body);
                    self.print_fields(&var.name, v, &var.fields);
                    self.finish(Term::Jmp(join));
                    self.switch(next);
                }
                self.finish(Term::Jmp(join));
                self.switch(join);
            }
            Kind::Array(elem, n) => {
                self.lit("[");
                let counter = self.constant(0);
                let lim = self.constant(n as i64);
                let size = self.t().size(elem);
                self.counted_loop(counter, lim, |l| {
                    let first = l.new_block();
                    let sep = l.new_block();
                    let item = l.new_block();
                    l.eq_branch(counter, 0, first, sep);
                    l.switch(sep);
                    l.lit(", ");
                    l.finish(Term::Jmp(item));
                    l.switch(first);
                    l.finish(Term::Jmp(item));
                    l.switch(item);
                    let d = l.vreg();
                    l.emit(Inst::IndexAddr { d, base: v, idx: counter, size });
                    let x = l.load(d, 0, elem);
                    l.print_value(x, elem);
                });
                self.lit("]");
            }
            Kind::Option(p) => {
                let (some, none, join) = (self.new_block(), self.new_block(), self.new_block());
                let tag = self.vreg();
                self.emit(Inst::Load { d: tag, p: v, off: 0, mem: Mem::X64 });
                self.eq_branch(tag, 1, some, none);
                self.switch(some);
                let x = self.load(v, TAG_SIZE, p);
                self.print_value(x, p);
                self.finish(Term::Jmp(join));
                self.switch(none);
                self.lit("none");
                self.finish(Term::Jmp(join));
                self.switch(join);
            }
            Kind::Result(ok, err) => {
                let (good, bad, join) = (self.new_block(), self.new_block(), self.new_block());
                let tag = self.vreg();
                self.emit(Inst::Load { d: tag, p: v, off: 0, mem: Mem::X64 });
                self.eq_branch(tag, 0, good, bad);
                self.switch(good);
                if ok == VOID {
                    self.lit("ok");
                } else {
                    let x = self.load(v, TAG_SIZE, ok);
                    self.print_value(x, ok);
                }
                self.finish(Term::Jmp(join));
                self.switch(bad);
                self.lit("fail(");
                let x = self.load(v, TAG_SIZE, err);
                self.print_value(x, err);
                self.lit(")");
                self.finish(Term::Jmp(join));
                self.switch(join);
            }
            Kind::Void | Kind::StrLit | Kind::Err => unreachable!("sema rejects printing these"),
        }
    }

    fn short_circuit(&mut self, e: &'a Expr) -> VReg {
        let d = self.vreg();
        let t = self.new_block();
        let f = self.new_block();
        let join = self.new_block();
        self.branch(e, t, f);
        self.switch(t);
        self.emit(Inst::Const { d, v: 1 });
        self.finish(Term::Jmp(join));
        self.switch(f);
        self.emit(Inst::Const { d, v: 0 });
        self.finish(Term::Jmp(join));
        self.switch(join);
        d
    }
}

fn arith(op: BinOp) -> Arith {
    match op {
        BinOp::Add => Arith::Add,
        BinOp::Sub => Arith::Sub,
        BinOp::Mul => Arith::Mul,
        BinOp::Div => Arith::Div,
        BinOp::Rem => Arith::Rem,
        BinOp::BitAnd => Arith::And,
        BinOp::BitOr => Arith::Or,
        BinOp::BitXor => Arith::Xor,
        BinOp::Shl => Arith::Shl,
        BinOp::Shr => Arith::Shr,
        _ => unreachable!("not an arithmetic operator"),
    }
}

fn cc(op: BinOp) -> Cc {
    match op {
        BinOp::Eq => Cc::Eq,
        BinOp::Ne => Cc::Ne,
        BinOp::Lt => Cc::Lt,
        BinOp::Le => Cc::Le,
        BinOp::Gt => Cc::Gt,
        BinOp::Ge => Cc::Ge,
        _ => unreachable!("not a comparison"),
    }
}
