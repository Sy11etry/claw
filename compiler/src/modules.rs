//! Finds and parses every file a program needs. One file is one module.

use crate::ast::{Block, Expr, ExprKind, Func, Program, Stmt, StmtKind};
use crate::diag::{Diag, Span};
use crate::{lexer, parser};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

thread_local! {
    /// Text of files an editor has open and not saved: the language server reads these instead of the disk.
    static OVERLAY: std::cell::RefCell<HashMap<PathBuf, String>> = std::cell::RefCell::new(HashMap::new());
}

thread_local! {
    /// Directories given with `-I`: where `use c "x.h"` looks for headers.
    static INCDIRS: std::cell::RefCell<Vec<PathBuf>> = std::cell::RefCell::new(Vec::new());
}

pub fn set_include_dirs(dirs: Vec<PathBuf>) {
    INCDIRS.with(|d| *d.borrow_mut() = dirs);
}

pub fn set_overlay(files: HashMap<PathBuf, String>) {
    OVERLAY.with(|o| *o.borrow_mut() = files);
}

fn read_source(path: &Path) -> std::io::Result<String> {
    let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if let Some(text) = OVERLAY.with(|o| o.borrow().get(&key).cloned()) {
        return Ok(text);
    }
    std::fs::read_to_string(path)
}

pub struct Unit {
    /// Prepended to names to make them unique: "" for the root, "geo." for `use geo`.
    pub prefix: String,
    pub program: Program,
    /// Local name of each import, and which unit it is.
    pub imports: HashMap<String, usize>,
    pub import_spans: HashMap<String, Span>,
    /// Part of the standard library rather than the user's program.
    pub is_std: bool,
    /// Structs of the C headers this module reads that Claw cannot lay out: name, then why and the header.
    pub packed: HashMap<String, (String, String)>,
}

pub struct Loaded {
    /// Dependencies come before the modules that use them; the root is last.
    pub units: Vec<Unit>,
    /// The always-available `core` module.
    pub core: usize,
    /// Full paths, for error messages.
    pub files: Vec<String>,
    /// Base names, for messages printed at run time.
    pub base_names: Vec<String>,
    /// Shared libraries named by `link "name"` in any module.
    pub links: Vec<String>,
}

struct Loader {
    std_root: Option<PathBuf>,
    /// The folder of the program being built: packages are looked up from here.
    root_dir: PathBuf,
    units: Vec<Unit>,
    files: Vec<String>,
    base_names: Vec<String>,
    done: HashMap<PathBuf, usize>,
    in_progress: Vec<PathBuf>,
    diags: Vec<Diag>,
    core: Option<usize>,
    links: Vec<String>,
    /// Also load the memory checker written in Claw.
    check: bool,
}

impl Loader {
    /// The file a `use` names: next to the program, in the standard library, or in a package. `Err` when the
    /// package does not match its lines in claw.pkg and claw.lock.
    fn locate(&self, dir: &Path, path: &[String]) -> Result<Option<PathBuf>, String> {
        let rel: PathBuf = path.iter().collect::<PathBuf>().with_extension("claw");
        let local = dir.join(&rel);
        if local.is_file() {
            return Ok(Some(local));
        }
        if let Some(in_std) = self.std_root.as_ref().map(|r| r.join(&rel)).filter(|p| p.is_file()) {
            return Ok(Some(in_std));
        }
        // A package fetched by `claw get`, found from the program's own folder upward.
        crate::pkg::locate(&self.root_dir, path)
    }

    /// `use c "x.h"`: the header's declarations join this module as if they were written here.
    fn c_headers(&mut self, program: &mut Program, here: &Path, packed: &mut HashMap<String, (String, String)>) {
        let incdirs = INCDIRS.with(|d| d.borrow().clone());
        for (name, span) in std::mem::take(&mut program.cimports) {
            let Some(h) = crate::cheader::locate(&name, here, &incdirs) else {
                self.diags.push(Diag::new("E0181", span, format!("cannot find the C header `{name}`; add its folder with -I")));
                continue;
            };
            let out = crate::cheader::translate(&h, &incdirs);
            let text = out.source;
            let base = h.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            for (ty, why) in out.packed {
                packed.insert(ty, (why, base.clone()));
            }
            let file = self.files.len() as u32;
            self.files.push(h.to_string_lossy().into_owned());
            self.base_names.push(base);
            let before = self.diags.len();
            let toks = lexer::lex(&text, file, &mut self.diags);
            if self.diags.len() > before {
                continue;
            }
            let mut extra = parser::Parser::new(toks, &mut self.diags).with_source(&text).parse_program();
            if self.diags.len() > before {
                continue;
            }
            let have: HashSet<String> = program.externs.iter().map(|e| e.name.clone()).chain(program.funcs.iter().map(|f| f.name.clone())).chain(program.consts.iter().map(|c| c.name.clone())).chain(program.types.iter().map(|t| t.name.clone())).collect();
            program.externs.extend(extra.externs.drain(..).filter(|e| !have.contains(&e.name)));
            program.consts.extend(extra.consts.drain(..).filter(|c| !have.contains(&c.name)));
            program.types.extend(extra.types.drain(..).filter(|t| !have.contains(&t.name)));
        }
    }

    fn load_core(&mut self) {
        let found = self.std_root.as_ref().map(|r| r.join("core.claw")).filter(|p| p.is_file());
        let Some(path) = found else {
            eprintln!("claw: cannot find std/core.claw next to the compiler");
            std::process::exit(2);
        };
        let name = path.to_string_lossy().into_owned();
        self.core = self.load(name, path, "core.".to_string(), Some(Span::default()));
        if self.check {
            let Some(path) = self.std_root.as_ref().map(|r| r.join("check_rt.claw")).filter(|p| p.is_file()) else {
                eprintln!("claw: cannot find std/check_rt.claw next to the compiler");
                std::process::exit(2);
            };
            let name = path.to_string_lossy().into_owned();
            self.load(name, path, "__check.".to_string(), Some(Span::default()));
        }
    }

    /// Returns the unit index, or None if the file could not be used.
    fn load(&mut self, display: String, path: PathBuf, prefix: String, from: Option<Span>) -> Option<usize> {
        let key = path.canonicalize().unwrap_or_else(|_| path.clone());
        if let Some(&i) = self.done.get(&key) {
            return Some(i);
        }
        let at = from.unwrap_or_default();
        if self.in_progress.contains(&key) {
            self.diags.push(Diag::new("E0146", at, format!("module `{prefix}` is part of an import cycle")));
            return None;
        }
        let src = match read_source(&path) {
            Ok(s) => s,
            Err(e) => {
                if from.is_none() {
                    eprintln!("claw: cannot read {display}: {e}");
                    std::process::exit(2);
                }
                self.diags.push(Diag::new("E0146", at, format!("cannot read {display}: {e}")));
                return None;
            }
        };
        let file = self.files.len() as u32;
        self.files.push(display);
        self.base_names.push(path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());

        let before = self.diags.len();
        let toks = lexer::lex(&src, file, &mut self.diags);
        if self.diags.len() > before {
            return None;
        }
        let mut program = parser::Parser::new(toks, &mut self.diags).with_source(&src).parse_program();
        if self.diags.len() > before {
            return None;
        }
        self.links.extend(program.links.iter().cloned());
        let mut packed = HashMap::new();
        self.c_headers(&mut program, &path, &mut packed);
        computed_consts(&mut program);

        self.in_progress.push(key.clone());
        if from.is_none() {
            self.load_core();
        }
        let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let mut imports = HashMap::new();
        let mut import_spans = HashMap::new();
        for u in &program.uses {
            let alias = u.path.last().unwrap().clone();
            if imports.contains_key(&alias) {
                self.diags.push(Diag::new("E0110", u.span, format!("module `{alias}` is imported twice")));
                continue;
            }
            let dotted = u.path.join(".");
            let found = match self.locate(&dir, &u.path) {
                Ok(Some(f)) => f,
                Ok(None) => {
                    self.diags.push(Diag::new("E0146", u.span, format!("cannot find module `{dotted}`")));
                    continue;
                }
                Err(msg) => {
                    self.diags.push(Diag::new("E0183", u.span, msg));
                    continue;
                }
            };
            let name = found.to_string_lossy().into_owned();
            if let Some(i) = self.load(name, found, format!("{dotted}."), Some(u.span)) {
                imports.insert(alias.clone(), i);
                import_spans.insert(alias, u.span);
            }
        }
        self.in_progress.pop();

        let is_std = self.std_root.as_ref().is_some_and(|r| r.canonicalize().map(|c| key.starts_with(c)).unwrap_or(false));
        self.units.push(Unit { prefix, program, imports, import_spans, is_std, packed });
        let idx = self.units.len() - 1;
        self.done.insert(key, idx);
        Some(idx)
    }
}

/// Literals and arithmetic on earlier simple constants: sema can do these by itself.
fn is_simple(e: &Expr, computed: &HashSet<String>) -> bool {
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Char(_) | ExprKind::Bool(_) => true,
        ExprKind::Var(n) => !computed.contains(n),
        ExprKind::Unary { e, .. } => is_simple(e, computed),
        ExprKind::Binary { l, r, .. } => is_simple(l, computed) && is_simple(r, computed),
        _ => false,
    }
}

/// `const x int = f(3)` becomes a hidden `fn const x() int { ret f(3) }`, run while compiling.
fn computed_consts(p: &mut Program) {
    // Constants can refer to later ones, so repeat until the set stops growing.
    let mut computed = HashSet::new();
    loop {
        let before = computed.len();
        let more: Vec<String> = p.consts.iter().filter(|c| c.ty.is_some() && !is_simple(&c.value, &computed)).map(|c| c.name.clone()).collect();
        computed.extend(more);
        if computed.len() == before {
            break;
        }
    }
    for c in p.consts.iter_mut().filter(|c| computed.contains(&c.name)) {
        let Some(ty) = c.ty.clone() else { continue };
        c.computed = true;
        let value = std::mem::replace(&mut c.value, Expr::new(ExprKind::Int(0), c.span));
        p.funcs.push(Func {
            name: format!("const {}", c.name),
            name_span: c.span,
            public: false,
            claw: false,
            tparams: Vec::new(),
            addressed: Default::default(),
            params: Vec::new(),
            ret: Some(ty),
            contracts: Vec::new(),
            effects: Vec::new(),
            sig_end: c.span,
            test: None,
            body: Block { stmts: vec![Stmt { span: value.span, kind: StmtKind::Ret(Some(value)) }], end: c.span },
        });
    }
}

/// Where the standard library lives relative to the compiler binary.
fn find_std() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let dir = exe.parent()?;
    let installed = dir.join("std");
    if installed.is_dir() {
        return Some(installed);
    }
    let repo = dir.parent()?.parent()?.parent()?.join("std");
    repo.is_dir().then_some(repo)
}

/// The public face of the standard library: every module but the memory checker, parsed only.
pub fn std_programs() -> Vec<(String, Program)> {
    let Some(root) = find_std() else { return Vec::new() };
    let Ok(dir) = std::fs::read_dir(&root) else { return Vec::new() };
    let mut names: Vec<PathBuf> = dir.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "claw")).collect();
    names.sort();
    let mut out = Vec::new();
    for path in names {
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if stem == "check_rt" {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&path) else { continue };
        let mut diags = Vec::new();
        let toks = lexer::lex(&src, 0, &mut diags);
        let program = parser::Parser::new(toks, &mut diags).with_source(&src).parse_program();
        out.push((stem, program));
    }
    out
}

pub fn load(root: &str, check: bool) -> Result<Loaded, (Vec<Diag>, Vec<String>)> {
    let mut l = Loader {
        std_root: find_std(),
        root_dir: Path::new(root).canonicalize().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default(),
        units: Vec::new(),
        files: Vec::new(),
        base_names: Vec::new(),
        done: HashMap::new(),
        in_progress: Vec::new(),
        diags: Vec::new(),
        core: None,
        links: Vec::new(),
        check,
    };
    l.load(root.to_string(), PathBuf::from(root), String::new(), None);
    if !l.diags.is_empty() {
        return Err((l.diags, l.files));
    }
    let core = l.core.expect("core was loaded with the root");
    Ok(Loaded { units: l.units, core, files: l.files, base_names: l.base_names, links: l.links })
}
