//! Turns a checked AST into CIR.

use crate::ast::{self, BinOp, Else, Expr, ExprKind, Stmt, StmtKind, Ty, UnOp};
use crate::cir::*;
use crate::diag::Span;
use std::collections::HashMap;

struct FnLower<'a> {
    ids: &'a HashMap<String, FuncId>,
    vars: HashMap<String, VReg>,
    nvregs: u32,
    blocks: Vec<Block>,
    cur: BlockId,
    /// (continue target, break target) of each enclosing loop.
    loops: Vec<(BlockId, BlockId)>,
}

fn loc(s: Span) -> Loc {
    Loc { line: s.line, col: s.col }
}

pub fn lower(p: &ast::Program) -> Module {
    let ids: HashMap<String, FuncId> =
        p.funcs.iter().enumerate().map(|(i, f)| (f.name.clone(), i)).collect();
    let funcs = p.funcs.iter().map(|f| lower_func(f, &ids)).collect();
    let main = ids["main"];
    Module { funcs, main }
}

fn lower_func(f: &ast::Func, ids: &HashMap<String, FuncId>) -> Func {
    let mut l = FnLower { ids, vars: HashMap::new(), nvregs: 0, blocks: Vec::new(), cur: 0, loops: Vec::new() };
    l.new_block();
    for p in &f.params {
        let v = l.vreg();
        l.vars.insert(p.name.clone(), v);
    }
    l.block(&f.body);
    // A void function falls off its end; a value function cannot (sema).
    let end = if f.ret.is_none() { Term::Ret(None) } else { Term::Unreachable };
    l.finish(end);
    Func { name: f.name.clone(), nparams: f.params.len(), nvregs: l.nvregs, blocks: l.blocks }
}

impl<'a> FnLower<'a> {
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

    fn block(&mut self, b: &ast::Block) {
        for s in &b.stmts {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match &s.kind {
            StmtKind::Let { name, init, .. } => {
                let v = self.expr_value(init);
                let d = self.vreg();
                self.emit(Inst::Mov { d, s: v });
                self.vars.insert(name.clone(), d);
            }
            StmtKind::Assign { name, name_span, op, value } => {
                let d = self.vars[name];
                let v = self.expr_value(value);
                match op {
                    None => self.emit(Inst::Mov { d, s: v }),
                    Some(op) => {
                        let t = self.vreg();
                        self.emit(Inst::Bin { op: arith(*op), d: t, a: d, b: v, loc: loc(*name_span) });
                        self.emit(Inst::Mov { d, s: t });
                    }
                }
            }
            StmtKind::Discard(e) => {
                self.expr_value(e);
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
                let head = self.new_block();
                let body_b = self.new_block();
                let latch = self.new_block();
                let exit = self.new_block();
                self.finish(Term::Jmp(head));

                self.switch(head);
                let c = self.vreg();
                self.emit(Inst::Cmp { cc: Cc::Lt, d: c, a: counter, b: lim });
                self.finish(Term::Br { c, t: body_b, f: exit });

                self.switch(body_b);
                if var != "_" {
                    let v = self.vreg();
                    self.emit(Inst::Mov { d: v, s: counter });
                    self.vars.insert(var.clone(), v);
                }
                self.loops.push((latch, exit));
                self.block(body);
                self.loops.pop();
                self.finish(Term::Jmp(latch));

                self.switch(latch);
                self.emit(Inst::Inc { d: counter });
                self.finish(Term::Jmp(head));
                self.switch(exit);
            }
            StmtKind::ForCond { cond, body } => {
                let head = self.new_block();
                let body_b = self.new_block();
                let exit = self.new_block();
                self.finish(Term::Jmp(head));
                self.switch(head);
                self.branch(cond, body_b, exit);
                self.switch(body_b);
                self.loops.push((head, exit));
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
                self.loops.push((head, exit));
                self.block(body);
                self.loops.pop();
                self.finish(Term::Jmp(head));
                self.switch(exit);
            }
            StmtKind::Ret(v) => {
                let r = v.as_ref().map(|e| self.expr_value(e));
                self.finish_dead(Term::Ret(r));
            }
            StmtKind::Break => {
                let t = self.loops.last().unwrap().1;
                self.finish_dead(Term::Jmp(t));
            }
            StmtKind::Continue => {
                let t = self.loops.last().unwrap().0;
                self.finish_dead(Term::Jmp(t));
            }
        }
    }

    fn if_stmt(&mut self, cond: &Expr, then: &ast::Block, els: Option<&Else>) {
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

    /// Branches on a condition, short-circuiting `and` / `or` / `not`.
    fn branch(&mut self, e: &Expr, t: BlockId, f: BlockId) {
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

    fn expr_value(&mut self, e: &Expr) -> VReg {
        self.expr(e).expect("sema guarantees a value")
    }

    fn expr(&mut self, e: &Expr) -> Option<VReg> {
        Some(match &e.kind {
            ExprKind::Int(v) => {
                let d = self.vreg();
                self.emit(Inst::Const { d, v: *v as i64 });
                d
            }
            ExprKind::Bool(b) => {
                let d = self.vreg();
                self.emit(Inst::Const { d, v: *b as i64 });
                d
            }
            ExprKind::Str(_) => unreachable!("string literals only occur inside print"),
            ExprKind::Var(name) => {
                // Copy so a later assignment cannot change an operand that
                // was already evaluated (left-to-right order is guaranteed).
                let d = self.vreg();
                self.emit(Inst::Mov { d, s: self.vars[name] });
                d
            }
            ExprKind::Unary { op, e: inner } => {
                let s = self.expr_value(inner);
                let d = self.vreg();
                self.emit(match op {
                    UnOp::Neg => Inst::Neg { d, s, loc: loc(e.span) },
                    UnOp::BitNot => Inst::BitNot { d, s },
                    UnOp::Not => Inst::BoolNot { d, s },
                });
                d
            }
            ExprKind::Binary { op, l, r } => match op {
                BinOp::And | BinOp::Or => return Some(self.short_circuit(e)),
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                    let a = self.expr_value(l);
                    let b = self.expr_value(r);
                    let d = self.vreg();
                    self.emit(Inst::Cmp { cc: cc(*op), d, a, b });
                    d
                }
                _ => {
                    let a = self.expr_value(l);
                    let b = self.expr_value(r);
                    let d = self.vreg();
                    self.emit(Inst::Bin { op: arith(*op), d, a, b, loc: loc(e.span) });
                    d
                }
            },
            ExprKind::Call { name, args, .. } => {
                if name == "print" {
                    self.print(&args[0]);
                    return None;
                }
                let vals: Vec<VReg> = args.iter().map(|a| self.expr_value(a)).collect();
                let d = if e.ty.get() == Ty::Void { None } else { Some(self.vreg()) };
                self.emit(Inst::Call { d, f: self.ids[name], args: vals });
                return d;
            }
        })
    }

    fn print(&mut self, a: &Expr) {
        if let ExprKind::Str(s) = &a.kind {
            self.emit(Inst::PrintStr { text: s.clone() });
            return;
        }
        let s = self.expr_value(a);
        match a.ty.get() {
            Ty::Bool => self.emit(Inst::PrintBool { s }),
            _ => self.emit(Inst::PrintInt { s }),
        }
    }

    fn short_circuit(&mut self, e: &Expr) -> VReg {
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
