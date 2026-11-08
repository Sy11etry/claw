//! Finds and parses every file a program needs. One file is one module.

use crate::ast::Program;
use crate::diag::{Diag, Span};
use crate::{lexer, parser};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct Unit {
    /// Prepended to names to make them unique: "" for the root, "geo." for `use geo`.
    pub prefix: String,
    pub program: Program,
    /// Local name of each import, and which unit it is.
    pub imports: HashMap<String, usize>,
    pub import_spans: HashMap<String, Span>,
}

pub struct Loaded {
    /// Dependencies come before the modules that use them; the root is last.
    pub units: Vec<Unit>,
    /// Full paths, for error messages.
    pub files: Vec<String>,
    /// Base names, for messages printed at run time.
    pub base_names: Vec<String>,
}

struct Loader {
    std_root: Option<PathBuf>,
    units: Vec<Unit>,
    files: Vec<String>,
    base_names: Vec<String>,
    done: HashMap<PathBuf, usize>,
    in_progress: Vec<PathBuf>,
    diags: Vec<Diag>,
}

impl Loader {
    fn locate(&self, dir: &Path, path: &[String]) -> Option<PathBuf> {
        let rel: PathBuf = path.iter().collect::<PathBuf>().with_extension("claw");
        let local = dir.join(&rel);
        if local.is_file() {
            return Some(local);
        }
        let in_std = self.std_root.as_ref()?.join(&rel);
        in_std.is_file().then_some(in_std)
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
        let src = match std::fs::read_to_string(&path) {
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
        let program = parser::Parser::new(toks, &mut self.diags).parse_program();
        if self.diags.len() > before {
            return None;
        }

        self.in_progress.push(key.clone());
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
            let Some(found) = self.locate(&dir, &u.path) else {
                self.diags.push(Diag::new("E0146", u.span, format!("cannot find module `{dotted}`")));
                continue;
            };
            let name = found.to_string_lossy().into_owned();
            if let Some(i) = self.load(name, found, format!("{dotted}."), Some(u.span)) {
                imports.insert(alias.clone(), i);
                import_spans.insert(alias, u.span);
            }
        }
        self.in_progress.pop();

        self.units.push(Unit { prefix, program, imports, import_spans });
        let idx = self.units.len() - 1;
        self.done.insert(key, idx);
        Some(idx)
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

pub fn load(root: &str) -> Result<Loaded, (Vec<Diag>, Vec<String>)> {
    let mut l = Loader {
        std_root: find_std(),
        units: Vec::new(),
        files: Vec::new(),
        base_names: Vec::new(),
        done: HashMap::new(),
        in_progress: Vec::new(),
        diags: Vec::new(),
    };
    l.load(root.to_string(), PathBuf::from(root), String::new(), None);
    if !l.diags.is_empty() {
        return Err((l.diags, l.files));
    }
    Ok(Loaded { units: l.units, files: l.files, base_names: l.base_names })
}
