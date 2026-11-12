//! Packages. A project lists what it uses in `claw.pkg`; `claw get` fetches those into `.claw/pkg/` and pins
//! each one by the SHA-256 of its files in `claw.lock`, so the same names give the same code tomorrow.
//!
//! claw.pkg, one dependency per line (`//` starts a comment):
//!     json  https://github.com/someone/claw-json#v1.2   a git repository, optionally at a tag or branch
//!     geo   ../geo                                      a folder on this machine
//! `use json` then finds `.claw/pkg/json/json.claw` (or `lib.claw`), and `use json.parse` finds `parse.claw`.

use crate::sha256::sha256;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Dep {
    pub name: String,
    pub source: String,
}

pub fn read_deps(file: &Path) -> Result<Vec<Dep>, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        // A comment starts with `//` at the beginning of a line or after a space (`https://` is not one).
        let cut = if line.trim_start().starts_with("//") { Some(0) } else { line.find(" //") };
        let line = cut.map_or(line, |i| &line[..i]).trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.split_whitespace();
        let (Some(name), Some(source), None) = (it.next(), it.next(), it.next()) else {
            return Err(format!("{}:{}: write `name source`", file.display(), n + 1));
        };
        if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') || name.starts_with(|c: char| c.is_ascii_digit()) {
            return Err(format!("{}:{}: `{name}` is not a module name (small letters, digits, `_`)", file.display(), n + 1));
        }
        out.push(Dep { name: name.to_string(), source: source.to_string() });
    }
    Ok(out)
}

/// Hex SHA-256 over every file's relative path and bytes, in path order.
fn tree_hash(dir: &Path) -> String {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                continue;
            }
            if p.is_dir() {
                walk(&p, base, out);
            } else {
                out.push(p.strip_prefix(base).unwrap().to_path_buf());
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, dir, &mut files);
    files.sort();
    let mut blob = Vec::new();
    for f in &files {
        blob.extend_from_slice(f.to_string_lossy().as_bytes());
        blob.push(0);
        let bytes = std::fs::read(dir.join(f)).unwrap_or_default();
        blob.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        blob.extend_from_slice(&bytes);
    }
    sha256(&blob).iter().map(|b| format!("{b:02x}")).collect()
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let name = e.file_name();
        if name == ".git" || name == ".claw" {
            continue;
        }
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(&name))?;
        } else {
            std::fs::copy(&p, to.join(&name))?;
        }
    }
    Ok(())
}

fn read_lock(file: &Path) -> Vec<(String, String)> {
    let Ok(text) = std::fs::read_to_string(file) else { return Vec::new() };
    text.lines().filter_map(|l| l.split_once(' ')).map(|(a, b)| (a.to_string(), b.trim().to_string())).collect()
}

/// `claw get [--update]` in the folder that holds claw.pkg.
pub fn get(project: &Path, update: bool) -> Result<Vec<String>, String> {
    let deps = read_deps(&project.join("claw.pkg"))?;
    let lock_file = project.join("claw.lock");
    let lock = read_lock(&lock_file);
    let pkg_dir = project.join(".claw/pkg");
    let mut report = Vec::new();
    let mut new_lock = Vec::new();
    for d in &deps {
        let dest = pkg_dir.join(&d.name);
        let pinned = lock.iter().find(|(n, _)| *n == d.name).map(|(_, h)| h.clone());
        // Already here and matching the lock: nothing to fetch.
        if dest.is_dir() && !update {
            let h = tree_hash(&dest);
            match &pinned {
                Some(p) if *p == h => {
                    report.push(format!("{}: up to date ({})", d.name, &h[..12]));
                    new_lock.push((d.name.clone(), h));
                    continue;
                }
                Some(p) => return Err(format!("{}: the files in .claw/pkg/{} changed (lock {}, found {}); run `claw get --update` to take them", d.name, d.name, &p[..12], &h[..12])),
                None => {}
            }
        }
        if dest.exists() {
            std::fs::remove_dir_all(&dest).map_err(|e| format!("{}: cannot replace the old copy: {e}", d.name))?;
        }
        std::fs::create_dir_all(&pkg_dir).map_err(|e| format!("cannot create {}: {e}", pkg_dir.display()))?;
        if d.source.contains("://") || d.source.starts_with("git@") {
            let (url, rev) = d.source.split_once('#').unwrap_or((&d.source, ""));
            let mut cmd = Command::new("git");
            cmd.arg("clone").arg("--quiet").arg("--depth").arg("1");
            if !rev.is_empty() {
                cmd.arg("--branch").arg(rev);
            }
            let st = cmd.arg(url).arg(&dest).status().map_err(|e| format!("{}: cannot run git: {e}", d.name))?;
            if !st.success() {
                return Err(format!("{}: `git clone {url}` failed", d.name));
            }
        } else {
            let from = project.join(&d.source);
            if !from.is_dir() {
                return Err(format!("{}: {} is not a folder", d.name, from.display()));
            }
            copy_dir(&from, &dest).map_err(|e| format!("{}: cannot copy {}: {e}", d.name, from.display()))?;
        }
        let h = tree_hash(&dest);
        if let (Some(p), false) = (&pinned, update) {
            if *p != h {
                let _ = std::fs::remove_dir_all(&dest);
                return Err(format!("{}: the fetched files do not match claw.lock (lock {}, got {}); run `claw get --update` if the change is expected", d.name, &p[..12], &h[..12]));
            }
        }
        report.push(format!("{}: fetched ({})", d.name, &h[..12]));
        new_lock.push((d.name.clone(), h));
    }
    let text: String = new_lock.iter().map(|(n, h)| format!("{n} {h}\n")).collect();
    std::fs::write(&lock_file, text).map_err(|e| format!("cannot write claw.lock: {e}"))?;
    Ok(report)
}

/// Where `use name` or `use name.sub` lives, looking in `.claw/pkg/` of this folder and of every folder above it.
pub fn locate(from: &Path, path: &[String]) -> Option<PathBuf> {
    let mut dir = Some(from);
    while let Some(d) = dir {
        let pkg = d.join(".claw/pkg").join(&path[0]);
        if pkg.is_dir() {
            if path.len() > 1 {
                let rel: PathBuf = path[1..].iter().collect::<PathBuf>().with_extension("claw");
                let f = pkg.join(rel);
                if f.is_file() {
                    return Some(f);
                }
            } else {
                for name in [format!("{}.claw", path[0]), "lib.claw".to_string()] {
                    let f = pkg.join(name);
                    if f.is_file() {
                        return Some(f);
                    }
                }
            }
        }
        dir = d.parent();
    }
    None
}
