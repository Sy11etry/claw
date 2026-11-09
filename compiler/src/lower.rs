//! Turns a checked AST into CIR.

use crate::ast::{self, BinOp, Coerce, Else, ElseKind, Expr, ExprKind, Part, Pat, Res, Stmt, StmtKind, UnOp};
use crate::cir::*;
use crate::diag::Span;
use crate::effects::{has_call, short_name};
use crate::modules::Unit;
use crate::sema::{walk, Checked, ConstVal};
use crate::types::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// Something to do when a scope ends.
#[derive(Clone, Copy)]
enum Clean<'a> {
    Defer(&'a ast::Block),
    /// Release the value at `addr` if `flag` is still set.
    Drop { addr: VReg, ty: Ty, flag: VReg },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum JobKind {
    Drop,
    Copy,
}

/// Functions the compiler writes itself, one per type that needs them.
struct Synth {
    base: usize,
    jobs: Vec<(JobKind, Ty)>,
    known: HashMap<(JobKind, Ty), FuncId>,
}

impl Synth {
    fn id(&mut self, kind: JobKind, ty: Ty) -> FuncId {
        if let Some(id) = self.known.get(&(kind, ty)) {
            return *id;
        }
        let id = self.base + self.jobs.len();
        self.jobs.push((kind, ty));
        self.known.insert((kind, ty), id);
        id
    }
}

/// What a build is for.
#[derive(Clone, Copy)]
pub struct Opts<'c> {
    /// Every contract, and the memory checker.
    pub check: bool,
    /// Build the `test` blocks of the root file.
    pub test: bool,
    /// `--fast`: contracts are not compiled at all.
    pub fast: bool,
    /// For the interpreter: it watches memory itself and wants to know the source lines.
    pub interp: bool,
    /// Values of the `const`s that need a call; empty while they are being worked out.
    pub consts: &'c [ConstData],
}

/// The result of running a `const` at compile time.
#[derive(Clone, Debug)]
pub enum ConstData {
    Scalar(u64),
    Bytes(Vec<u8>),
    Str(Vec<u8>),
}

/// What every function being lowered shares.
struct Env<'a, 'i> {
    c: &'a Checked,
    files: &'a [String],
    ids: &'i HashMap<String, FuncId>,
    funcs: &'i HashMap<String, &'a ast::Func>,
    synth: &'i RefCell<Synth>,
    check: bool,
    fast: bool,
    interp: bool,
    consts: &'i [ConstData],
}

/// Word 0 of the runtime's state: set while a contract with calls is running.
const GUARD: u64 = 0;

struct FnLower<'a, 'i> {
    c: &'a Checked,
    files: &'a [String],
    ids: &'i HashMap<String, FuncId>,
    funcs: &'i HashMap<String, &'a ast::Func>,
    synth: &'i RefCell<Synth>,
    /// Every contract runs, not only the cheap ones.
    check: bool,
    /// No contract runs.
    fast: bool,
    interp: bool,
    consts: &'i [ConstData],
    /// The last source position announced to the interpreter.
    mark_at: Option<Span>,
    /// Raw memory goes through the checker.
    instrument: bool,
    is_claw: bool,
    in_contract: bool,
    ret_val: Option<VReg>,
    /// `old(e)` values saved on entry.
    olds: HashMap<*const Expr, VReg>,
    sures: Vec<&'a ast::Contract>,
    cur_name: String,
    vars: HashMap<String, VReg>,
    /// Is this local still holding its value? Only for values that need dropping.
    flags: HashMap<String, VReg>,
    all_flags: Vec<VReg>,
    /// Scalars that had their address taken: `vars` holds their memory address.
    addressed: HashSet<String>,
    nvregs: u32,
    objects: Vec<(u64, u64)>,
    blocks: Vec<Block>,
    cur: BlockId,
    /// (continue target, break target, defer depth) of each enclosing loop.
    loops: Vec<(BlockId, BlockId, usize)>,
    /// Pending `defer` bodies of each open block, innermost last.
    defers: Vec<Vec<Clean<'a>>>,
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

pub fn lower<'a>(units: &'a [Unit], c: &'a Checked, files: &'a [String], opts: Opts<'a>) -> Module {
    let mut list: Vec<(String, &'a ast::Func)> = Vec::new();
    let root = units.len() - 1;
    for (i, u) in units.iter().enumerate() {
        for f in u.program.funcs.iter().filter(|f| f.tparams.is_empty() && (f.test.is_none() || (opts.test && i == root))) {
            list.push((format!("{}{}", u.prefix, f.name), f));
        }
    }
    for f in &c.instances {
        list.push((f.name.clone(), f));
    }
    let ids: HashMap<String, FuncId> = list.iter().enumerate().map(|(i, (n, _))| (n.clone(), i)).collect();
    // Argument checks for the fuzzer take the ids right after the ordinary functions.
    let root_prefix = &units[root].prefix;
    let wanted: Vec<(&String, &'a ast::Func)> = if opts.interp {
        units[root].program.funcs.iter().filter(|f| is_auto(f)).map(|f| (list.iter().find(|(n, g)| std::ptr::eq(*g, f) && n.starts_with(root_prefix.as_str())).map(|(n, _)| n).expect("listed above"), f)).collect()
    } else {
        Vec::new()
    };
    let filtered: Vec<(&String, &'a ast::Func)> = wanted.iter().filter(|(_, f)| f.contracts.iter().any(|x| !x.sure)).cloned().collect();
    let synth = RefCell::new(Synth { base: list.len() + filtered.len(), jobs: Vec::new(), known: HashMap::new() });
    let by_name: HashMap<String, &'a ast::Func> = list.iter().map(|(n, f)| (n.clone(), *f)).collect();
    let env = Env { c, files, ids: &ids, funcs: &by_name, synth: &synth, check: opts.check, fast: opts.fast, interp: opts.interp, consts: opts.consts };
    let mut funcs: Vec<Func> = list.iter().map(|(n, f)| lower_func(n, f, &env)).collect();
    for (n, f) in &filtered {
        funcs.push(build_filter(n, f, &env));
    }
    // Each generated function may ask for more, so the queue can grow as it runs.
    let mut next = 0;
    loop {
        let job = synth.borrow().jobs.get(next).copied();
        let Some((kind, ty)) = job else { break };
        funcs.push(build_synth(kind, ty, &env));
        next += 1;
    }
    let check = (opts.check && !opts.interp).then(|| CheckRt { alloc: ids["__check.on_alloc"], free: ids["__check.on_free"], finish: ids["__check.finish"] });
    let tests = list.iter().filter_map(|(n, f)| f.test.clone().map(|t| (t, ids[n]))).collect();
    let autos = wanted
        .iter()
        .map(|(n, f)| {
            let sig = &c.sigs[*n];
            let filter = filtered.iter().position(|(m, _)| m == n).map(|i| list.len() + i);
            let params = f.params.iter().zip(&sig.params).zip(&sig.own).map(|((p, t), o)| (p.name.clone(), *t, *o)).collect();
            Auto { full: (*n).clone(), name: f.name.clone(), func: ids[*n], filter, params, ret: sig.ret }
        })
        .collect();
    let const_funcs = c.computed.iter().map(|x| ids[&x.full]).collect();
    Module { funcs, main: ids.get("main").copied().unwrap_or(0), check, tests, autos, const_funcs }
}

/// Could the fuzzer be pointed at this function at all?
fn is_auto(f: &ast::Func) -> bool {
    f.test.is_none() && f.tparams.is_empty() && f.name != "main" && f.name != "drop" && !f.name.starts_with("const ")
}

/// A function saying whether its arguments satisfy `f`'s `need` clauses.
fn build_filter<'a>(full: &str, f: &'a ast::Func, env: &Env<'a, '_>) -> Func {
    let c = env.c;
    let sig = &c.sigs[full];
    let mut l = new_lower(env, BOOL, false);
    l.is_claw = f.claw;
    l.cur_name = full.to_string();
    l.in_contract = true;
    l.defers.push(Vec::new());
    for (p, ty) in f.params.iter().zip(&sig.params) {
        let v = l.vreg();
        l.vars.insert(p.name.clone(), v);
        if c.types.is_ref(*ty) {
            l.addressed.insert(p.name.clone());
        }
    }
    for x in f.contracts.iter().filter(|x| !x.sure) {
        l.defers.push(Vec::new());
        let (v, _) = l.eval_cond(&x.e);
        let (ok, bad) = (l.new_block(), l.new_block());
        l.finish(Term::Br { c: v, t: ok, f: bad });
        l.switch(bad);
        let no = l.constant(0);
        l.finish(Term::Ret(Some(no)));
        l.switch(ok);
        let depth = l.defers.len() - 1;
        l.run_defers(depth);
        l.defers.pop();
    }
    l.defers.pop();
    let yes = l.constant(1);
    l.finish(Term::Ret(Some(yes)));
    l.into_func(format!("need of {full}"), f.params.len())
}

fn new_lower<'a, 'i>(env: &Env<'a, 'i>, ret: Ty, is_main: bool) -> FnLower<'a, 'i> {
    let mut l = FnLower {
        c: env.c,
        files: env.files,
        ids: env.ids,
        funcs: env.funcs,
        synth: env.synth,
        check: env.check,
        fast: env.fast,
        interp: env.interp,
        consts: env.consts,
        mark_at: None,
        instrument: env.check && !env.interp,
        is_claw: false,
        in_contract: false,
        ret_val: None,
        olds: HashMap::new(),
        sures: Vec::new(),
        cur_name: String::new(),
        vars: HashMap::new(),
        flags: HashMap::new(),
        all_flags: Vec::new(),
        addressed: HashSet::new(),
        nvregs: 0,
        objects: Vec::new(),
        blocks: Vec::new(),
        cur: 0,
        loops: Vec::new(),
        defers: Vec::new(),
        ret,
        sret: None,
        is_main,
    };
    l.new_block();
    l
}

fn lower_func<'a>(full: &str, f: &'a ast::Func, env: &Env<'a, '_>) -> Func {
    let c = env.c;
    let sig = &c.sigs[full];
    let mut l = new_lower(env, sig.ret, full == "main" || f.test.is_some());
    l.instrument = env.check && !env.interp && !full.starts_with("__check.");
    l.is_claw = f.claw;
    l.cur_name = full.to_string();
    l.addressed = f.addressed.borrow().clone();
    if c.types.is_aggregate(sig.ret) {
        l.sret = Some(l.vreg());
    }
    l.defers.push(Vec::new());
    // Parameters must take the first registers, in order, before anything else does.
    let regs: Vec<VReg> = f.params.iter().map(|_| l.vreg()).collect();
    l.mark(f.name_span);
    for ((p, ty), v) in f.params.iter().zip(&sig.params).zip(regs) {
        l.vars.insert(p.name.clone(), v);
        l.note(&p.name, v, *ty);
        if c.types.is_ref(*ty) {
            l.addressed.insert(p.name.clone());
        } else if p.own && c.types.needs_drop(*ty) {
            let flag = l.new_flag();
            l.flags.insert(p.name.clone(), flag);
            l.defers[0].push(Clean::Drop { addr: v, ty: *ty, flag });
        }
    }
    let nparams = f.params.len() + l.sret.is_some() as usize;
    l.sures = f.contracts.iter().filter(|x| x.sure && !env.fast && (env.check || !has_call(&x.e))).collect();
    l.save_olds();
    l.block(&f.body);
    l.run_defers(0);
    if sig.ret == VOID || matches!(c.types.kind(sig.ret), Kind::Result(VOID, _)) {
        l.check_sures(None);
    }
    l.defers.pop();
    if sig.ret == VOID {
        l.finish(Term::Ret(None));
    } else if matches!(c.types.kind(sig.ret), Kind::Result(VOID, _)) {
        l.return_ok_void();
    } else {
        l.finish(Term::Unreachable);
    }
    l.into_func(full.to_string(), nparams)
}

fn build_synth<'a>(kind: JobKind, ty: Ty, env: &Env<'a, '_>) -> Func {
    let c = env.c;
    let mut l = new_lower(env, VOID, false);
    let (name, nparams) = match kind {
        JobKind::Drop => {
            let v = l.vreg();
            l.build_drop(v, ty);
            (format!("__drop[{}]", c.types.name(ty)), 1)
        }
        JobKind::Copy => {
            let dst = l.vreg();
            let src = l.vreg();
            l.build_copy(dst, src, ty);
            (format!("__copy[{}]", c.types.name(ty)), 2)
        }
    };
    l.finish(Term::Ret(None));
    l.into_func(name, nparams)
}

impl<'a, 'i> FnLower<'a, 'i> {
    fn t(&self) -> &'a Types {
        &self.c.types
    }

    /// Tells the interpreter which source line runs next.
    fn mark(&mut self, s: Span) {
        if self.interp {
            self.mark_at = Some(s);
            self.emit(Inst::Line { loc: loc(s) });
        }
    }

    /// Back to the line of the current statement, after code from somewhere else.
    fn remark(&mut self) {
        if let Some(s) = self.mark_at {
            self.emit(Inst::Line { loc: loc(s) });
        }
    }

    /// Tells the interpreter a variable took a value, if the value can be shown.
    fn note(&mut self, name: &str, v: VReg, ty: Ty) {
        if !self.interp || name == "_" {
            return;
        }
        let show = match self.t().kind(ty) {
            Kind::Int { signed: true, .. } => Show::Int,
            Kind::Int { signed: false, .. } => Show::Uint,
            Kind::Bool => Show::Bool,
            Kind::Float { bits: 32 } => Show::F32,
            Kind::Float { .. } => Show::F64,
            _ => return,
        };
        self.emit(Inst::Note { name: name.to_string(), v, show });
    }

    fn place_text(&self, s: Span) -> String {
        format!("{}:{}:{}", self.files[s.file as usize], s.line, s.col)
    }

    /// Tells the checker where the next allocation or release comes from.
    fn set_site(&mut self, s: Span) {
        if self.instrument {
            let text = self.place_text(s);
            self.emit(Inst::SetSite { text });
        }
    }

    /// In check mode: `size` bytes at `addr` must belong to the program.
    fn guard_mem(&mut self, addr: VReg, size: u64, s: Span) {
        if !self.instrument {
            return;
        }
        let site = self.vreg();
        let text = self.place_text(s);
        self.emit(Inst::StrAddr { d: site, text });
        let n = self.constant(size as i64);
        self.emit(Inst::Call { d: None, f: self.ids["__check.access"], args: vec![addr, n, site] });
    }

    /// Does this assignment target or read land in memory only a `claw fn` can get wrong?
    fn is_raw(&self, e: &Expr) -> bool {
        through_pointer(e, self.t())
            || (self.is_claw && matches!(&e.kind, ExprKind::Index { e: b, .. } if matches!(self.t().kind(b.ty.get()), Kind::Slice(_) | Kind::Str)))
    }

    fn guard_on(&mut self) -> (VReg, BlockId) {
        let a = self.vreg();
        self.emit(Inst::RtState { d: a });
        let v = self.vreg();
        self.emit(Inst::Load { d: v, p: a, off: GUARD, mem: Mem::X64 });
        let (run, done) = (self.new_block(), self.new_block());
        self.finish(Term::Br { c: v, t: done, f: run });
        self.switch(run);
        let one = self.constant(1);
        self.emit(Inst::Store { p: a, off: GUARD, s: one, mem: Mem::X64 });
        (a, done)
    }

    fn guard_off(&mut self, (a, done): (VReg, BlockId)) {
        let zero = self.constant(0);
        self.emit(Inst::Store { p: a, off: GUARD, s: zero, mem: Mem::X64 });
        self.finish(Term::Jmp(done));
        self.switch(done);
    }

    /// Records `old(...)` values before the body can change anything.
    fn save_olds(&mut self) {
        for c in self.sures.clone() {
            let mut found: Vec<&'a Expr> = Vec::new();
            walk(&c.e, &mut |x| {
                if matches!(x.kind, ExprKind::Old(_)) {
                    found.push(x);
                }
            });
            for x in found {
                let ExprKind::Old(inner) = &x.kind else { continue };
                let slot = self.vreg();
                self.all_flags.push(slot);
                let guard = has_call(inner).then(|| self.guard_on());
                self.defers.push(Vec::new());
                let v = self.expr_value(inner);
                self.emit(Inst::Mov { d: slot, s: v });
                let depth = self.defers.len() - 1;
                self.run_defers(depth);
                self.defers.pop();
                if let Some(g) = guard {
                    self.guard_off(g);
                }
                self.olds.insert(x as *const Expr, slot);
            }
        }
    }

    /// `r` is the scalar result, if any; aggregates are already in the result slot.
    fn check_sures(&mut self, r: Option<VReg>) {
        if self.sures.is_empty() {
            return;
        }
        let payload = match self.t().kind(self.ret) {
            Kind::Option(p) => Some(p),
            Kind::Result(ok, _) if ok != VOID => Some(ok),
            _ => None,
        };
        let Some(p) = payload else {
            self.ret_val = r.or(self.sret);
            return self.run_sures();
        };
        // A `sure` speaks about a value that was returned, not about `none` or `fail`.
        let sret = self.sret.expect("?T and T ! E return in memory");
        let tag = self.load_tag(sret);
        let want = self.success_tag(self.ret);
        let (yes, join) = (self.new_block(), self.new_block());
        self.eq_branch(tag, want, yes, join);
        self.switch(yes);
        self.ret_val = Some(self.load(sret, TAG_SIZE, p));
        self.run_sures();
        self.finish(Term::Jmp(join));
        self.switch(join);
    }

    fn run_sures(&mut self) {
        for c in self.sures.clone() {
            let at = self.place_text(c.e.span);
            let name = short_name(&self.cur_name).to_string();
            self.mark(c.e.span);
            self.check_cond(&c.e, format!("trap: sure failed: `{}` in {name} at {at}\n", c.text));
        }
    }

    /// Evaluates the callee's `need` clauses here, so a failure names this call.
    fn check_needs(&mut self, full: &str, args: &[VReg], at: Span) {
        let Some(&f) = self.funcs.get(full) else { return };
        if self.fast {
            return;
        }
        let check = self.check;
        let needs: Vec<&'a ast::Contract> = f.contracts.iter().filter(|c| !c.sure && (check || !has_call(&c.e))).collect();
        if needs.is_empty() {
            return;
        }
        let sig = &self.c.sigs[full];
        let saved = (std::mem::take(&mut self.vars), std::mem::take(&mut self.addressed), std::mem::take(&mut self.flags));
        for ((p, t), v) in f.params.iter().zip(&sig.params).zip(args) {
            self.vars.insert(p.name.clone(), *v);
            if self.t().is_ref(*t) {
                self.addressed.insert(p.name.clone());
            }
        }
        let place = self.place_text(at);
        for c in needs {
            let was = self.mark_at;
            self.mark(c.e.span);
            self.check_cond(&c.e, format!("trap: need failed: `{}` in {} at {place}\n", c.text, short_name(full)));
            self.mark_at = was;
            self.remark();
        }
        (self.vars, self.addressed, self.flags) = saved;
    }

    /// Stops the program with a report if `e` is false.
    fn check_cond(&mut self, e: &'a Expr, header: String) {
        let guard = has_call(e).then(|| self.guard_on());
        let was = std::mem::replace(&mut self.in_contract, true);
        self.defers.push(Vec::new());
        let (c, shown) = self.eval_cond(e);
        let (ok, bad) = (self.new_block(), self.new_block());
        self.finish(Term::Br { c, t: ok, f: bad });

        self.switch(bad);
        self.emit(Inst::BufStr { text: header });
        for (label, v, ty) in shown {
            self.lit(&format!("  {label} = "));
            self.print_value(v, ty);
            self.lit("\n");
        }
        for x in self.names_in(e) {
            let ty = x.ty.get();
            let (label, v) = match &x.kind {
                ExprKind::Var(n) => (n.clone(), self.var(x, n, ty)),
                _ => ("ret".to_string(), self.ret_val.expect("`ret` was bound")),
            };
            self.lit(&format!("  {label} = "));
            self.print_value(v, ty);
            self.lit("\n");
        }
        self.emit(Inst::Flush { fd: 2 });
        self.emit(Inst::Exit { code: 101 });
        self.finish(Term::Unreachable);

        self.switch(ok);
        let depth = self.defers.len() - 1;
        self.run_defers(depth);
        self.defers.pop();
        self.in_contract = was;
        if let Some(g) = guard {
            self.guard_off(g);
        }
    }

    /// Variables a failed condition should show, each once; what `old` saw is not there now.
    fn names_in(&self, e: &'a Expr) -> Vec<&'a Expr> {
        let mut under_old: HashSet<*const Expr> = HashSet::new();
        walk(e, &mut |x| {
            if let ExprKind::Old(inner) = &x.kind {
                walk(inner, &mut |y| {
                    under_old.insert(y as *const Expr);
                });
            }
        });
        let mut out: Vec<&'a Expr> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        walk(e, &mut |x| {
            let key = match &x.kind {
                ExprKind::Var(n) if self.vars.contains_key(n) => n.clone(),
                ExprKind::RetVal => "ret".to_string(),
                _ => return,
            };
            if !under_old.contains(&(x as *const Expr)) && seen.insert(key) {
                out.push(x);
            }
        });
        out
    }

    /// The condition, and the values of a comparison worth showing if it fails.
    fn eval_cond(&mut self, e: &'a Expr) -> (VReg, Vec<(&'static str, VReg, Ty)>) {
        if let ExprKind::Binary { op, l, r } = &e.kind {
            if is_comparison(*op) {
                let (d, operands) = self.compare_exprs(*op, l, r);
                let mut shown = Vec::new();
                if let Some((a, b)) = operands {
                    if !plain(l) {
                        shown.push(("left", a, l.ty.get()));
                    }
                    if !plain(r) {
                        shown.push(("right", b, r.ty.get()));
                    }
                }
                return (d, shown);
            }
        }
        (self.expr_value(e), Vec::new())
    }

    /// Compares; in a contract, integer `+ - *` is exact rather than overflowing.
    fn compare_exprs(&mut self, op: BinOp, l: &'a Expr, r: &'a Expr) -> (VReg, Option<(VReg, VReg)>) {
        let operand = l.ty.get();
        if self.in_contract && self.t().is_int(operand) && (self.is_wide(l) || self.is_wide(r)) {
            let (alo, ahi) = self.wide(l);
            let (blo, bhi) = self.wide(r);
            let d = self.vreg();
            self.emit(Inst::WideCmp { cc: cc(op), d, alo, ahi, blo, bhi });
            return (d, None);
        }
        let a = self.expr_value(l);
        let b = self.expr_value(r);
        (self.compare(op, a, b, operand), Some((a, b)))
    }

    fn compare(&mut self, op: BinOp, a: VReg, b: VReg, operand: Ty) -> VReg {
        if operand == STR {
            return self.compare_str(op, a, b);
        }
        let d = self.vreg();
        if let Some(bits) = self.float_bits(operand) {
            self.emit(Inst::FCmp { cc: cc(op), d, a, b, bits });
        } else {
            let unsigned = !self.num_of(operand).signed;
            self.emit(Inst::Cmp { cc: cc(op), d, a, b, unsigned });
        }
        d
    }

    fn is_wide(&self, e: &Expr) -> bool {
        matches!(&e.kind, ExprKind::Binary { op: BinOp::Add | BinOp::Sub | BinOp::Mul, .. } | ExprKind::Unary { op: UnOp::Neg, .. }) && self.t().is_int(e.ty.get())
    }

    fn wide(&mut self, e: &'a Expr) -> (VReg, VReg) {
        if self.is_wide(e) {
            let (alo, ahi, blo, bhi, op) = match &e.kind {
                ExprKind::Binary { op, l, r } => {
                    let (alo, ahi) = self.wide(l);
                    let (blo, bhi) = self.wide(r);
                    let op = match op {
                        BinOp::Add => WideOp::Add,
                        BinOp::Sub => WideOp::Sub,
                        _ => WideOp::Mul,
                    };
                    (alo, ahi, blo, bhi, op)
                }
                ExprKind::Unary { e: x, .. } => {
                    let zero = self.constant(0);
                    let (blo, bhi) = self.wide(x);
                    (zero, zero, blo, bhi, WideOp::Sub)
                }
                _ => unreachable!("is_wide checked this"),
            };
            let (dlo, dhi) = (self.vreg(), self.vreg());
            self.emit(Inst::WideBin { op, dlo, dhi, alo, ahi, blo, bhi, loc: loc(e.span) });
            return (dlo, dhi);
        }
        if let ExprKind::Int(v) = e.kind {
            return (self.constant(v as i64), self.constant((v >> 64) as i64));
        }
        let v = self.expr_value(e);
        let hi = if self.num_of(e.ty.get()).signed {
            let k = self.constant(63);
            let d = self.vreg();
            self.emit(Inst::Bin { op: Arith::Shr, d, a: v, b: k, num: Num { bits: 64, signed: true, checked: true }, loc: loc(e.span) });
            d
        } else {
            self.constant(0)
        };
        (v, hi)
    }

    fn needs(&self, ty: Ty) -> bool {
        self.t().needs_drop(ty)
    }

    fn into_func(self, name: String, nparams: usize) -> Func {
        // Every flag starts cleared, on every path, before anything can set it.
        let mut blocks = self.blocks;
        let zeros: Vec<Inst> = self.all_flags.iter().map(|d| Inst::Const { d: *d, v: 0 }).collect();
        blocks[0].insts.splice(0..0, zeros);
        let params = (0..nparams as VReg).collect();
        Func { name, nparams, params, claw: self.is_claw, optimized: false, dead: false, nvregs: self.nvregs, objects: self.objects, blocks }
    }

    /// A new liveness flag, set right away.
    fn new_flag(&mut self) -> VReg {
        let flag = self.vreg();
        self.all_flags.push(flag);
        self.emit(Inst::Const { d: flag, v: 1 });
        flag
    }

    fn clear_flag(&mut self, name: &str) {
        if let Some(&flag) = self.flags.get(name) {
            self.emit(Inst::Const { d: flag, v: 0 });
        }
    }

    fn drop_now(&mut self, addr: VReg, ty: Ty) {
        let f = self.synth.borrow_mut().id(JobKind::Drop, ty);
        self.emit(Inst::Call { d: None, f, args: vec![addr] });
    }

    fn copy_into(&mut self, dst: VReg, src: VReg, ty: Ty) {
        let f = self.synth.borrow_mut().id(JobKind::Copy, ty);
        self.emit(Inst::Call { d: None, f, args: vec![dst, src] });
    }

    /// A local takes ownership; it goes in the block around the current statement.
    fn register_local(&mut self, name: &str, addr: VReg, ty: Ty) {
        let flag = self.new_flag();
        self.flags.insert(name.to_string(), flag);
        let n = self.defers.len();
        self.defers[n - 2].push(Clean::Drop { addr, ty, flag });
    }

    /// A temporary lives until the end of the current statement.
    fn register_temp(&mut self, addr: VReg, ty: Ty) {
        let flag = self.new_flag();
        self.defers.last_mut().unwrap().push(Clean::Drop { addr, ty, flag });
    }

    /// `(pointer to first item, length)` of an array, slice or string.
    fn seq(&mut self, ty: Ty, addr: VReg) -> (VReg, VReg) {
        if let Kind::Array(_, n) = self.t().kind(ty) {
            let len = self.constant(n as i64);
            return (addr, len);
        }
        let p = self.vreg();
        self.emit(Inst::Load { d: p, p: addr, off: SEQ_PTR, mem: Mem::X64 });
        let n = self.vreg();
        self.emit(Inst::Load { d: n, p: addr, off: SEQ_LEN, mem: Mem::X64 });
        (p, n)
    }

    fn elem_of(&self, ty: Ty) -> Ty {
        match self.t().kind(ty) {
            Kind::Array(e, _) | Kind::Slice(e) => e,
            Kind::Str => U8,
            _ => unreachable!("not a sequence"),
        }
    }

    fn store_seq(&mut self, a: VReg, p: VReg, len: VReg, cap: VReg) {
        self.emit(Inst::Store { p: a, off: SEQ_PTR, s: p, mem: Mem::X64 });
        self.emit(Inst::Store { p: a, off: SEQ_LEN, s: len, mem: Mem::X64 });
        self.emit(Inst::Store { p: a, off: SEQ_CAP, s: cap, mem: Mem::X64 });
    }

    /// Calls `each` with the address of every item.
    fn for_each(&mut self, p: VReg, n: VReg, size: u64, each: impl FnOnce(&mut Self, VReg)) {
        let counter = self.constant(0);
        let lim = self.vreg();
        self.emit(Inst::Mov { d: lim, s: n });
        self.counted_loop(counter, lim, |l| {
            let d = l.vreg();
            l.emit(Inst::IndexAddr { d, base: p, idx: counter, size });
            each(l, d);
        });
    }

    fn on_tag(&mut self, tag: VReg, value: i64, body: impl FnOnce(&mut Self)) {
        let (yes, join) = (self.new_block(), self.new_block());
        self.eq_branch(tag, value, yes, join);
        self.switch(yes);
        body(self);
        self.finish(Term::Jmp(join));
        self.switch(join);
    }

    fn load_tag(&mut self, addr: VReg) -> VReg {
        let d = self.vreg();
        self.emit(Inst::Load { d, p: addr, off: 0, mem: Mem::X64 });
        d
    }

    fn drop_fields(&mut self, v: VReg, fields: &[Field]) {
        for f in fields {
            if self.needs(f.ty) {
                let a = self.offset_addr(v, f.offset);
                self.drop_now(a, f.ty);
            }
        }
    }

    fn build_drop(&mut self, v: VReg, ty: Ty) {
        match self.t().kind(ty) {
            Kind::Slice(_) | Kind::Str => {
                let elem = self.elem_of(ty);
                let cap = self.vreg();
                self.emit(Inst::Load { d: cap, p: v, off: SEQ_CAP, mem: Mem::X64 });
                let (free_it, done) = (self.new_block(), self.new_block());
                self.eq_branch(cap, 0, done, free_it);
                self.switch(free_it);
                let (p, n) = self.seq(ty, v);
                if self.needs(elem) {
                    let size = self.t().size(elem);
                    self.for_each(p, n, size, |l, item| l.drop_now(item, elem));
                }
                if self.instrument {
                    self.emit(Inst::SetSite { text: "automatic release".to_string() });
                }
                self.emit(Inst::Free { p });
                self.finish(Term::Jmp(done));
                self.switch(done);
            }
            Kind::Array(elem, n) => {
                let len = self.constant(n as i64);
                let size = self.t().size(elem);
                self.for_each(v, len, size, |l, item| l.drop_now(item, elem));
            }
            Kind::Struct(id) => {
                if let Some(full) = self.t().user_drop.get(&ty) {
                    self.emit(Inst::Call { d: None, f: self.ids[full], args: vec![v] });
                }
                let fields = self.t().structs[id].fields.clone();
                self.drop_fields(v, &fields);
            }
            Kind::Enum(id) => {
                if let Some(full) = self.t().user_drop.get(&ty) {
                    self.emit(Inst::Call { d: None, f: self.ids[full], args: vec![v] });
                }
                let tag = self.load_tag(v);
                let variants = self.t().enums[id].variants.clone();
                for (i, var) in variants.iter().enumerate() {
                    self.on_tag(tag, i as i64, |l| l.drop_fields(v, &var.fields));
                }
            }
            Kind::Option(p) => {
                let tag = self.load_tag(v);
                self.on_tag(tag, 1, |l| {
                    let a = l.offset_addr(v, TAG_SIZE);
                    l.drop_now(a, p);
                });
            }
            Kind::Result(ok, err) => {
                let tag = self.load_tag(v);
                let a = self.offset_addr(v, TAG_SIZE);
                if self.needs(ok) {
                    self.on_tag(tag, 0, |l| l.drop_now(a, ok));
                }
                if self.needs(err) {
                    self.on_tag(tag, 1, |l| l.drop_now(a, err));
                }
            }
            _ => unreachable!("nothing to drop"),
        }
    }

    fn copy_fields(&mut self, dst: VReg, src: VReg, fields: &[Field]) {
        for f in fields {
            if self.needs(f.ty) {
                let d = self.offset_addr(dst, f.offset);
                let s = self.offset_addr(src, f.offset);
                self.copy_into(d, s, f.ty);
            }
        }
    }

    /// Writes a deep copy of `src` into the uninitialized memory at `dst`.
    fn build_copy(&mut self, dst: VReg, src: VReg, ty: Ty) {
        if let Kind::Slice(_) | Kind::Str = self.t().kind(ty) {
            let elem = self.elem_of(ty);
            let size = self.t().size(elem);
            let (p, n) = self.seq(ty, src);
            let (empty, some, done) = (self.new_block(), self.new_block(), self.new_block());
            self.eq_branch(n, 0, empty, some);
            self.switch(empty);
            let zero = self.constant(0);
            self.store_seq(dst, zero, zero, zero);
            self.finish(Term::Jmp(done));
            self.switch(some);
            let fresh = self.vreg();
            let at = Loc { line: 0, col: 0, file: 0 };
            self.emit(Inst::Alloc { d: fresh, n, elem: size, loc: at });
            if self.needs(elem) {
                let counter = self.constant(0);
                let lim = self.vreg();
                self.emit(Inst::Mov { d: lim, s: n });
                self.counted_loop(counter, lim, |l| {
                    let (to, from) = (l.vreg(), l.vreg());
                    l.emit(Inst::IndexAddr { d: to, base: fresh, idx: counter, size });
                    l.emit(Inst::IndexAddr { d: from, base: p, idx: counter, size });
                    l.copy_into(to, from, elem);
                });
            } else {
                let k = self.constant(size as i64);
                let bytes = self.vreg();
                self.emit(Inst::Bin { op: Arith::Mul, d: bytes, a: n, b: k, num: Num { bits: 64, signed: false, checked: true }, loc: at });
                self.emit(Inst::MemCopyDyn { dst: fresh, src: p, n: bytes });
            }
            self.store_seq(dst, fresh, n, n);
            self.finish(Term::Jmp(done));
            self.switch(done);
            return;
        }
        self.emit(Inst::MemCopy { dst, src, size: self.t().size(ty) });
        match self.t().kind(ty) {
            Kind::Array(elem, n) => {
                let size = self.t().size(elem);
                let counter = self.constant(0);
                let lim = self.constant(n as i64);
                self.counted_loop(counter, lim, |l| {
                    let (to, from) = (l.vreg(), l.vreg());
                    l.emit(Inst::IndexAddr { d: to, base: dst, idx: counter, size });
                    l.emit(Inst::IndexAddr { d: from, base: src, idx: counter, size });
                    l.copy_into(to, from, elem);
                });
            }
            Kind::Struct(id) => {
                let fields = self.t().structs[id].fields.clone();
                self.copy_fields(dst, src, &fields);
            }
            Kind::Enum(id) => {
                let tag = self.load_tag(src);
                let variants = self.t().enums[id].variants.clone();
                for (i, var) in variants.iter().enumerate() {
                    self.on_tag(tag, i as i64, |l| l.copy_fields(dst, src, &var.fields));
                }
            }
            Kind::Option(p) => {
                let tag = self.load_tag(src);
                self.on_tag(tag, 1, |l| {
                    let (d, s) = (l.offset_addr(dst, TAG_SIZE), l.offset_addr(src, TAG_SIZE));
                    l.copy_into(d, s, p);
                });
            }
            Kind::Result(ok, err) => {
                let tag = self.load_tag(src);
                let (d, s) = (self.offset_addr(dst, TAG_SIZE), self.offset_addr(src, TAG_SIZE));
                if self.needs(ok) {
                    self.on_tag(tag, 0, |l| l.copy_into(d, s, ok));
                }
                if self.needs(err) {
                    self.on_tag(tag, 1, |l| l.copy_into(d, s, err));
                }
            }
            _ => unreachable!("nothing to copy"),
        }
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
            Kind::Int { bits, signed } => Num { bits, signed, checked: true },
            Kind::Pointer(_) => Num { bits: 64, signed: false, checked: true },
            _ => Num { bits: 64, signed: true, checked: true },
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
            for item in pending.into_iter().rev() {
                match item {
                    Clean::Defer(body) => self.block(body),
                    Clean::Drop { addr, ty, flag } => {
                        let (yes, join) = (self.new_block(), self.new_block());
                        self.finish(Term::Br { c: flag, t: yes, f: join });
                        self.switch(yes);
                        self.drop_now(addr, ty);
                        self.emit(Inst::Const { d: flag, v: 0 });
                        self.finish(Term::Jmp(join));
                        self.switch(join);
                    }
                }
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
        if matches!(self.t().kind(ty), Kind::Slice(_) | Kind::Str) {
            return match name {
                "ptr" => SEQ_PTR,
                "len" => SEQ_LEN,
                _ => SEQ_CAP,
            };
        }
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
        if matches!(self.t().kind(array), Kind::Slice(_) | Kind::Str) {
            let (p, len) = self.seq(array, addr);
            let idx = self.expr_value(i);
            self.emit(Inst::BoundsDyn { idx, len, loc: loc(span) });
            let d = self.vreg();
            let size = self.t().size(self.elem_of(array));
            self.emit(Inst::IndexAddr { d, base: p, idx, size });
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
        self.defers.push(Vec::new());
        self.stmt_inner(s);
        let depth = self.defers.len() - 1;
        self.run_defers(depth);
        self.defers.pop();
    }

    fn stmt_inner(&mut self, s: &'a Stmt) {
        self.mark(s.span);
        match &s.kind {
            StmtKind::Let { name, init, .. } => {
                let ty = init.ty.get();
                let ty = match init.coerce.get() {
                    Coerce::Some(t) | Coerce::Ok(t) => t,
                    Coerce::No => ty,
                };
                let v = self.owned_value(init);
                self.note(name, v, ty);
                // A value just built in its own temporary needs no second copy.
                let fresh = init.coerce.get() != Coerce::No
                    || matches!(init.kind, ExprKind::ArrayLit(_) | ExprKind::ArrayFill(_) | ExprKind::StructLit { .. } | ExprKind::Call { .. } | ExprKind::None | ExprKind::Str(_) | ExprKind::Interp(_));
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
                if self.needs(ty) {
                    self.register_local(name, d, ty);
                }
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
                        l.note(var, v, INT);
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
                self.mark(s.span);
                if makes_temporaries(cond, self.t()) {
                    // The temporaries must go on every pass, not once at the end.
                    self.defers.push(Vec::new());
                    let c = self.expr_value(cond);
                    let depth = self.defers.len() - 1;
                    self.run_defers(depth);
                    self.defers.pop();
                    self.finish(Term::Br { c, t: body_b, f: exit });
                } else {
                    self.branch(cond, body_b, exit);
                }
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
            StmtKind::Sure(e, text) => {
                if !self.fast && (self.check || !has_call(e)) {
                    let at = self.place_text(s.span);
                    self.check_cond(e, format!("trap: sure failed: `{text}` at {at}\n"));
                }
            }
            StmtKind::Ret(v) => self.ret_stmt(v.as_ref()),
            StmtKind::Fail(e) => {
                let Kind::Result(_, err) = self.t().kind(self.ret) else { unreachable!("sema checked `fail`") };
                let v = self.owned_value(e);
                let sret = self.sret.expect("Result is returned in memory");
                self.set_tag(sret, 1);
                self.store(sret, TAG_SIZE, v, err);
                self.run_defers(0);
                self.finish_dead(Term::Ret(None));
            }
            StmtKind::Defer(body) => {
                let n = self.defers.len();
                self.defers[n - 2].push(Clean::Defer(body));
            }
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
        self.remark();
        let c = self.vreg();
        self.emit(Inst::Cmp { cc: Cc::Lt, d: c, a: counter, b: lim, unsigned: false });
        self.finish(Term::Br { c, t: body_b, f: exit });

        self.switch(body_b);
        self.loops.push((latch, exit, self.defers.len()));
        body(self);
        self.loops.pop();
        self.finish(Term::Jmp(latch));

        self.switch(latch);
        self.remark();
        self.emit(Inst::AddImm { d: counter, s: counter, imm: 1 });
        self.finish(Term::Jmp(head));
        self.switch(exit);
    }

    fn for_in(&mut self, elem: &str, index: Option<&(String, Span)>, subject: &'a Expr, body: &'a ast::Block) {
        let ty = subject.ty.get();
        let elem_ty = self.elem_of(ty);
        let v = self.expr_value(subject);
        let (base, n) = self.seq(ty, v);
        let counter = self.constant(0);
        let lim = self.vreg();
        self.emit(Inst::Mov { d: lim, s: n });
        let size = self.t().size(elem_ty);
        self.counted_loop(counter, lim, |l| {
            if elem != "_" {
                let a = l.vreg();
                l.emit(Inst::IndexAddr { d: a, base, idx: counter, size });
                // The sequence cannot change during the loop, so aggregates can be read in place.
                let x = l.load(a, 0, elem_ty);
                let d = if l.t().is_aggregate(elem_ty) { x } else { l.new_local(x, elem_ty) };
                l.vars.insert(elem.to_string(), d);
                l.note(elem, d, elem_ty);
            }
            if let Some((name, _)) = index {
                if name != "_" {
                    let x = l.vreg();
                    l.emit(Inst::Mov { d: x, s: counter });
                    l.vars.insert(name.clone(), x);
                    l.note(name, x, INT);
                }
            }
            l.block(body);
        });
    }

    fn ret_stmt(&mut self, v: Option<&'a Expr>) {
        match v {
            None => {
                self.run_defers(0);
                self.check_sures(None);
                if matches!(self.t().kind(self.ret), Kind::Result(VOID, _)) {
                    self.return_ok_void();
                    let dead = self.new_block();
                    self.switch(dead);
                } else {
                    self.finish_dead(Term::Ret(None));
                }
            }
            Some(e) => {
                let r = self.owned_value(e);
                if self.t().is_aggregate(self.ret) {
                    let sret = self.sret.expect("aggregates are returned in memory");
                    self.store(sret, 0, r, self.ret);
                    self.run_defers(0);
                    self.check_sures(None);
                    self.finish_dead(Term::Ret(None));
                } else {
                    // The value is computed first: a `defer` must not change what is returned.
                    self.run_defers(0);
                    self.check_sures(Some(r));
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
                let v = self.owned_value(value);
                self.guard_target(&p, target, ty);
                self.note_target(target, v, ty);
                if self.needs(ty) && !through_pointer(target, self.t()) {
                    let old = match p {
                        Place::Mem(a, o) => self.offset_addr(a, o),
                        Place::Reg(_) => unreachable!("owning values live in memory"),
                    };
                    let flag = match &target.kind {
                        ExprKind::Var(n) => self.flags.get(n).copied(),
                        _ => None,
                    };
                    match flag {
                        Some(flag) => {
                            // The old value may already have moved away.
                            let (yes, join) = (self.new_block(), self.new_block());
                            self.finish(Term::Br { c: flag, t: yes, f: join });
                            self.switch(yes);
                            self.drop_now(old, ty);
                            self.finish(Term::Jmp(join));
                            self.switch(join);
                            self.write_place(p, v, ty);
                            self.emit(Inst::Const { d: flag, v: 1 });
                        }
                        None => {
                            self.drop_now(old, ty);
                            self.write_place(p, v, ty);
                        }
                    }
                    return;
                }
                self.write_place(p, v, ty);
            }
            Some(op) => {
                self.guard_target(&p, target, ty);
                let old = match p {
                    Place::Reg(r) => r,
                    Place::Mem(a, o) => self.load(a, o, ty),
                };
                let v = self.expr_value(value);
                let d = self.arith(op, old, v, ty, target.span);
                self.note_target(target, d, ty);
                self.write_place(p, d, ty);
            }
        }
    }

    fn note_target(&mut self, target: &Expr, v: VReg, ty: Ty) {
        if self.interp {
            if let Some(name) = target_name(target) {
                self.note(&name, v, ty);
            }
        }
    }

    fn guard_target(&mut self, p: &Place, target: &Expr, ty: Ty) {
        if self.instrument && self.is_raw(target) {
            if let Place::Mem(a, o) = *p {
                let addr = self.offset_addr(a, o);
                self.guard_mem(addr, self.t().size(ty), target.span);
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
                        // An owning field is only looked at; the subject keeps it.
                        let d = if self.needs(f.ty) { x } else { self.new_local(x, f.ty) };
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

    /// Evaluates for reading. A temporary that owns memory is released after the statement.
    fn expr(&mut self, e: &'a Expr) -> Option<VReg> {
        let v = self.expr_raw(e);
        if let Some(x) = v {
            let ty = e.ty.get();
            if self.needs(ty) && is_fresh(e) {
                self.register_temp(x, ty);
            }
        }
        self.wrap(e, v)
    }

    /// Evaluates for taking over: the caller becomes responsible for the value.
    fn owned(&mut self, e: &'a Expr) -> Option<VReg> {
        let ty = e.ty.get();
        let v = match &e.kind {
            ExprKind::Var(n) if self.needs(ty) => {
                let x = self.var(e, n, ty);
                self.clear_flag(n);
                Some(x)
            }
            _ => self.expr_raw(e),
        };
        self.wrap(e, v)
    }

    fn owned_value(&mut self, e: &'a Expr) -> VReg {
        self.owned(e).expect("sema guarantees a value")
    }

    fn wrap(&mut self, e: &Expr, v: Option<VReg>) -> Option<VReg> {
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
            self.emit(Inst::Bin { op: Arith::Sub, d: n, a: zero, b, num: Num { bits: 64, signed: false, checked: true }, loc: at });
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
        self.emit(Inst::Bin { op: Arith::Sub, d: bytes, a, b, num: Num { bits: 64, signed: false, checked: true }, loc: loc(span) });
        let size = self.constant(self.t().size(elem).max(1) as i64);
        let d = self.vreg();
        self.emit(Inst::Bin { op: Arith::Div, d, a: bytes, b: size, num: Num { bits: 64, signed: true, checked: true }, loc: loc(span) });
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
                if self.t().is_ptr(ty) {
                    d
                } else {
                    let a = self.temp_obj(STR);
                    let n = self.constant(text.len() as i64);
                    let zero = self.constant(0);
                    self.store_seq(a, d, n, zero);
                    a
                }
            }
            ExprKind::Interp(parts) => {
                let pieces = self.pieces(parts);
                self.emit(Inst::CaptureStart);
                self.emit_pieces(pieces);
                self.set_site(e.span);
                let (ptr, len) = (self.vreg(), self.vreg());
                self.emit(Inst::CaptureTake { ptr, len });
                let a = self.temp_obj(STR);
                self.store_seq(a, ptr, len, len);
                a
            }
            ExprKind::AddrOf(inner) => {
                let Place::Mem(a, o) = self.place(inner) else { unreachable!("sema made this addressable") };
                self.offset_addr(a, o)
            }
            ExprKind::Deref(inner) => {
                let p = self.expr_value(inner);
                self.guard_mem(p, self.t().size(ty), e.span);
                self.load(p, 0, ty)
            }
            ExprKind::RetVal => return self.ret_val,
            ExprKind::Old(_) => return self.olds.get(&(e as *const Expr)).copied(),
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
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => self.compare_exprs(*op, l, r).0,
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
            ExprKind::ArrayLit(items) if matches!(self.t().kind(ty), Kind::Slice(_)) => {
                let elem = self.elem_of(ty);
                let a = self.temp_obj(ty);
                let size = self.t().size(elem);
                let at = loc(e.span);
                let n = self.constant(items.len() as i64);
                if items.is_empty() {
                    let zero = self.constant(0);
                    self.store_seq(a, zero, zero, zero);
                } else {
                    self.set_site(e.span);
                    let p = self.vreg();
                    self.emit(Inst::Alloc { d: p, n, elem: size, loc: at });
                    for (i, it) in items.iter().enumerate() {
                        let v = self.owned_value(it);
                        self.store(p, i as u64 * size, v, elem);
                    }
                    self.store_seq(a, p, n, n);
                }
                a
            }
            ExprKind::ArrayLit(items) => {
                let Kind::Array(elem, _) = self.t().kind(ty) else { unreachable!("array literal type") };
                let a = self.temp_obj(ty);
                let size = self.t().size(elem);
                for (i, it) in items.iter().enumerate() {
                    let v = self.owned_value(it);
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
                if self.instrument && self.t().is_ptr(base.ty.get()) {
                    let at = self.offset_addr(a, off);
                    self.guard_mem(at, self.t().size(ty), e.span);
                }
                self.load(a, off, ty)
            }
            ExprKind::Index { e: base, i } => {
                let a = self.expr_value(base);
                let d = self.element_addr(base.ty.get(), a, i, e.span);
                if self.instrument && (self.t().is_ptr(base.ty.get()) || self.is_raw(e)) {
                    self.guard_mem(d, self.t().size(ty), e.span);
                }
                self.load(d, 0, ty)
            }
            ExprKind::Slice { e: base, lo, hi } => self.slice_expr(base, lo.as_deref(), hi.as_deref(), ty, e.span),
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
            Res::Const(ConstVal::Computed(id), _) => self.computed(id),
            Res::Variant(_, idx) => {
                let a = self.temp_obj(ty);
                self.set_tag(a, idx as i64);
                a
            }
            _ => unreachable!("sema resolved this name"),
        }
    }

    /// A `const` that needed a function call: now plain data.
    fn computed(&mut self, id: usize) -> VReg {
        let d = self.vreg();
        match self.consts.get(id) {
            None => self.emit(Inst::ConstRef { d, id }),
            Some(ConstData::Scalar(v)) => self.emit(Inst::Const { d, v: *v as i64 }),
            Some(ConstData::Bytes(b)) => self.emit(Inst::DataAddr { d, bytes: b.clone() }),
            Some(ConstData::Str(b)) => {
                let mut bytes = b.clone();
                bytes.push(0);
                self.emit(Inst::DataAddr { d, bytes });
                let a = self.temp_obj(STR);
                let n = self.constant(b.len() as i64);
                let zero = self.constant(0);
                self.store_seq(a, d, n, zero);
                return a;
            }
        }
        d
    }

    fn compare_str(&mut self, op: BinOp, a: VReg, b: VReg) -> VReg {
        let d = self.vreg();
        if matches!(op, BinOp::Eq | BinOp::Ne) {
            self.emit(Inst::Call { d: Some(d), f: self.ids["core.str_eq"], args: vec![a, b] });
            if op == BinOp::Eq {
                return d;
            }
            let n = self.vreg();
            self.emit(Inst::BoolNot { d: n, s: d });
            return n;
        }
        self.emit(Inst::Call { d: Some(d), f: self.ids["core.str_cmp"], args: vec![a, b] });
        let zero = self.constant(0);
        let r = self.vreg();
        self.emit(Inst::Cmp { cc: cc(op), d: r, a: d, b: zero, unsigned: false });
        r
    }

    fn slice_expr(&mut self, base: &'a Expr, lo: Option<&'a Expr>, hi: Option<&'a Expr>, ty: Ty, span: Span) -> VReg {
        let base_ty = base.ty.get();
        let a = self.expr_value(base);
        let (p, n) = self.seq(base_ty, a);
        let start = match lo {
            Some(x) => self.expr_value(x),
            None => self.constant(0),
        };
        let end = match hi {
            Some(x) => self.expr_value(x),
            None => n,
        };
        // 0 <= start <= end <= len, checked as two unsigned comparisons.
        let n_plus = self.vreg();
        self.emit(Inst::AddImm { d: n_plus, s: n, imm: 1 });
        self.emit(Inst::BoundsDyn { idx: end, len: n_plus, loc: loc(span) });
        let end_plus = self.vreg();
        self.emit(Inst::AddImm { d: end_plus, s: end, imm: 1 });
        self.emit(Inst::BoundsDyn { idx: start, len: end_plus, loc: loc(span) });

        let size = self.t().size(self.elem_of(base_ty));
        let first = self.vreg();
        self.emit(Inst::IndexAddr { d: first, base: p, idx: start, size });
        let count = self.vreg();
        self.emit(Inst::Bin { op: Arith::Sub, d: count, a: end, b: start, num: Num { bits: 64, signed: false, checked: true }, loc: loc(span) });
        let view = self.temp_obj(ty);
        let zero = self.constant(0);
        self.store_seq(view, first, count, zero);
        view
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
        for (a, own) in args.iter().zip(&sig.own) {
            let v = if *own { self.owned_value(a) } else { self.expr_value(a) };
            vals.push(v);
        }
        self.check_needs(&full, &vals[sret.is_some() as usize..], e.span);
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
                let at = args[0].ty.get();
                let v = self.expr_value(&args[0]);
                Some(self.seq(at, v).1)
            }
            "copy" => {
                let src = self.expr_value(&args[0]);
                if !self.t().is_aggregate(ty) {
                    return Some(src);
                }
                let dst = self.temp_obj(ty);
                if self.needs(ty) {
                    self.copy_into(dst, src, ty);
                } else {
                    self.store(dst, 0, src, ty);
                }
                Some(dst)
            }
            "alloc" => {
                let Kind::Pointer(elem) = self.t().kind(ty) else { unreachable!("sema checked `alloc`") };
                let n = self.expr_value(&args[0]);
                self.set_site(e.span);
                let d = self.vreg();
                self.emit(Inst::Alloc { d, n, elem: self.t().size(elem), loc: loc(e.span) });
                Some(d)
            }
            "free" => {
                let p = self.expr_value(&args[0]);
                self.set_site(e.span);
                self.emit(Inst::Free { p });
                None
            }
            "__state" => {
                let d = self.vreg();
                self.emit(Inst::RtState { d });
                Some(d)
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
            let v = self.owned_value(arg);
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
        let v = self.owned_value(inner);
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
        let v = self.owned_value(inner);
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
                self.drop_unused_error(v, ty);
                let x = self.owned_value(d);
                self.write_result(result.expect("sema checked the fallback"), x, payload);
                self.finish(Term::Jmp(join));
            }
            ElseKind::Block(binder, b) => {
                self.defers.push(Vec::new());
                match (binder, self.t().kind(ty)) {
                    (Some((name, _)), Kind::Result(_, err)) => {
                        let x = self.load(v, TAG_SIZE, err);
                        let d = self.new_local(x, err);
                        self.vars.insert(name.clone(), d);
                        if self.needs(err) {
                            let flag = self.new_flag();
                            self.flags.insert(name.clone(), flag);
                            self.defers.last_mut().unwrap().push(Clean::Drop { addr: d, ty: err, flag });
                        }
                    }
                    _ => self.drop_unused_error(v, ty),
                }
                self.block(b);
                self.defers.pop();
                self.finish(Term::Unreachable);
            }
        }
        self.switch(join);
        result
    }

    /// Nobody asked for the error in `v`, so it ends here.
    fn drop_unused_error(&mut self, v: VReg, ty: Ty) {
        if let Kind::Result(_, err) = self.t().kind(ty) {
            if self.needs(err) {
                let a = self.offset_addr(v, TAG_SIZE);
                self.drop_now(a, err);
            }
        }
    }

    fn write_result(&mut self, r: VReg, x: VReg, ty: Ty) {
        if self.t().is_aggregate(ty) {
            self.store(r, 0, x, ty);
        } else {
            self.emit(Inst::Mov { d: r, s: x });
        }
    }

    fn pieces(&mut self, parts: &'a [Part]) -> Vec<Piece<'a>> {
        let mut out = Vec::with_capacity(parts.len());
        for p in parts {
            match p {
                Part::Lit(s) => out.push(Piece::Lit(s)),
                Part::Expr(e) => out.push(Piece::Val(self.expr_value(e), e.ty.get())),
            }
        }
        out
    }

    fn emit_pieces(&mut self, pieces: Vec<Piece<'a>>) {
        for p in pieces {
            match p {
                Piece::Lit(s) => self.emit(Inst::BufStr { text: s.to_string() }),
                Piece::Val(v, t) => self.print_value(v, t),
            }
        }
    }

    fn print(&mut self, a: &'a Expr) {
        // Evaluate first: a trap in an argument must not leave half a line.
        let pieces = match &a.kind {
            ExprKind::Str(s) => vec![Piece::Lit(s)],
            ExprKind::Interp(parts) => self.pieces(parts),
            _ => vec![Piece::Val(self.expr_value(a), a.ty.get())],
        };
        self.emit_pieces(pieces);
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
            Kind::Str => {
                let (p, n) = self.seq(ty, v);
                self.emit(Inst::BufPtr { p, n });
            }
            Kind::Array(..) | Kind::Slice(_) => {
                let elem = self.elem_of(ty);
                self.lit("[");
                let (base, lim) = self.seq(ty, v);
                let counter = self.constant(0);
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
                    l.emit(Inst::IndexAddr { d, base, idx: counter, size });
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
            Kind::Void | Kind::StrLit | Kind::Err | Kind::Ref(_) => unreachable!("sema rejects printing these"),
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

enum Piece<'a> {
    Lit(&'a str),
    Val(VReg, Ty),
}

fn is_comparison(op: BinOp) -> bool {
    matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
}

/// `x` or `p.x`: what an assignment to this should be called in a report.
fn target_name(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Var(n) => Some(n.clone()),
        ExprKind::Field { e: b, name, .. } => Some(format!("{}.{name}", target_name(b)?)),
        _ => None,
    }
}

/// Its value is already on the page or shown under its own name.
fn plain(e: &Expr) -> bool {
    matches!(e.kind, ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Char(_) | ExprKind::Bool(_) | ExprKind::Str(_) | ExprKind::Var(_) | ExprKind::RetVal)
}

/// Produces a value nobody owns yet.
fn is_fresh(e: &Expr) -> bool {
    matches!(e.kind, ExprKind::Call { .. } | ExprKind::StructLit { .. } | ExprKind::ArrayLit(_) | ExprKind::Interp(_) | ExprKind::Try(_) | ExprKind::Else { .. })
}

fn through_pointer(e: &Expr, types: &Types) -> bool {
    match &e.kind {
        ExprKind::Deref(_) => true,
        ExprKind::Field { e: b, .. } | ExprKind::Index { e: b, .. } => types.is_ptr(b.ty.get()) || through_pointer(b, types),
        _ => false,
    }
}

fn makes_temporaries(e: &Expr, types: &Types) -> bool {
    let here = types.needs_drop(e.ty.get()) && is_fresh(e);
    let any = |list: &[Expr]| list.iter().any(|x| makes_temporaries(x, types));
    here || match &e.kind {
        ExprKind::Unary { e: x, .. } | ExprKind::Deref(x) | ExprKind::AddrOf(x) | ExprKind::Field { e: x, .. } | ExprKind::Try(x) => makes_temporaries(x, types),
        ExprKind::Binary { l, r, .. } => makes_temporaries(l, types) || makes_temporaries(r, types),
        ExprKind::Index { e: x, i } => makes_temporaries(x, types) || makes_temporaries(i, types),
        ExprKind::Slice { e: x, .. } => makes_temporaries(x, types),
        ExprKind::Call { args, .. } | ExprKind::StructLit { args, .. } | ExprKind::ArrayLit(args) => any(args),
        ExprKind::Else { e: x, .. } => makes_temporaries(x, types),
        _ => false,
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
