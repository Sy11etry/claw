//! Packages. A project lists what it uses in `claw.pkg`; `claw get` fetches those into `.claw/pkg/` and pins
//! each one in `claw.lock`: where it came from, and the SHA-256 of its files. The same names then give the same
//! code tomorrow, and `claw run` and `claw build` refuse a package that no longer matches its line.
//!
//! claw.pkg, one dependency per line (`//` starts a comment):
//!     json  https://github.com/someone/claw-json#v1.2   a git repository, optionally at a tag or branch
//!     geo   ../geo                                      a folder on this machine
//! `use json` then finds `.claw/pkg/json/json.claw` (or `lib.claw`), and `use json.parse` finds `parse.claw`.
//!
//! claw.lock, one line per package: `name source hash`. A line with only `name hash` was written by an older
//! claw: its source is not known, so `claw get` fetches that package again to check it.

use crate::sha256::sha256;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Dep {
    pub name: String,
    pub source: String,
}

/// One line of claw.lock.
struct Pin {
    name: String,
    /// Where the package came from, as claw.pkg wrote it. None for a line written before that was recorded.
    source: Option<String>,
    hash: String,
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

fn read_lock(file: &Path) -> Result<Vec<Pin>, String> {
    let Ok(text) = std::fs::read_to_string(file) else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let pin = match fields.as_slice() {
            [] => continue,
            [name, source, hash] => Pin { name: name.to_string(), source: Some(source.to_string()), hash: hash.to_string() },
            [name, hash] => Pin { name: name.to_string(), source: None, hash: hash.to_string() },
            _ => return Err(format!("{}:{}: write `name source hash`", file.display(), n + 1)),
        };
        out.push(pin);
    }
    Ok(out)
}

/// The first twelve characters of a hash, for messages.
fn short(hash: &str) -> String {
    hash.chars().take(12).collect()
}

fn source_changed(name: &str, now: &str, pinned: &str) -> String {
    format!("{name}: claw.pkg says `{now}`, but claw.lock pins `{pinned}`; run `claw get --update` to take the new source")
}

fn files_changed(name: &str, pinned: &str, found: &str) -> String {
    format!("{name}: the files in .claw/pkg/{name} changed (lock {}, found {}); run `claw get --update` to take them", short(pinned), short(found))
}

/// `claw get [--update]` in the folder that holds claw.pkg.
pub fn get(project: &Path, update: bool) -> Result<Vec<String>, String> {
    let deps = read_deps(&project.join("claw.pkg"))?;
    let lock_file = project.join("claw.lock");
    let lock = read_lock(&lock_file)?;
    let pkg_dir = project.join(".claw/pkg");
    // A changed source is refused before anything is fetched, so a refusal leaves the folder as it was.
    if !update {
        for d in &deps {
            if let Some(src) = lock.iter().find(|p| p.name == d.name).and_then(|p| p.source.as_deref()) {
                if src != d.source {
                    return Err(source_changed(&d.name, &d.source, src));
                }
            }
        }
    }
    let mut report = Vec::new();
    let mut new_lock: Vec<(String, String, String)> = Vec::new();
    for d in &deps {
        let dest = pkg_dir.join(&d.name);
        let pinned = lock.iter().find(|p| p.name == d.name);
        // Already here and pinned from this very source: nothing to fetch, only the files to check.
        if dest.is_dir() && !update {
            if let Some(Pin { source: Some(_), hash: pin_hash, .. }) = pinned {
                let h = tree_hash(&dest);
                if *pin_hash != h {
                    return Err(files_changed(&d.name, pin_hash, &h));
                }
                report.push(format!("{}: up to date ({})", d.name, short(&h)));
                new_lock.push((d.name.clone(), d.source.clone(), h));
                continue;
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
        if let (Some(p), false) = (pinned, update) {
            if p.hash != h {
                let _ = std::fs::remove_dir_all(&dest);
                return Err(format!("{}: the fetched files do not match claw.lock (lock {}, got {}); run `claw get --update` if the change is expected", d.name, short(&p.hash), short(&h)));
            }
        }
        report.push(format!("{}: fetched ({})", d.name, short(&h)));
        new_lock.push((d.name.clone(), d.source.clone(), h));
    }
    let text: String = new_lock.iter().map(|(n, s, h)| format!("{n} {s} {h}\n")).collect();
    std::fs::write(&lock_file, text).map_err(|e| format!("cannot write claw.lock: {e}"))?;
    Ok(report)
}

/// Where `use name` or `use name.sub` lives, looking in `.claw/pkg/` of this folder and of every folder above it.
/// A package found there must still match its claw.pkg and claw.lock lines; `Err` says what to run when it does not.
pub fn locate(from: &Path, path: &[String]) -> Result<Option<PathBuf>, String> {
    let mut dir = Some(from);
    while let Some(d) = dir {
        let pkg = d.join(".claw/pkg").join(&path[0]);
        if pkg.is_dir() {
            let found = if path.len() > 1 {
                let rel: PathBuf = path[1..].iter().collect::<PathBuf>().with_extension("claw");
                Some(pkg.join(rel)).filter(|f| f.is_file())
            } else {
                [format!("{}.claw", path[0]), "lib.claw".to_string()].iter().map(|n| pkg.join(n)).find(|f| f.is_file())
            };
            if let Some(f) = found {
                verify(d, &path[0])?;
                return Ok(Some(f));
            }
        }
        dir = d.parent();
    }
    Ok(None)
}

/// Before `use name` loads `.claw/pkg/name` of `project`: the package is listed in claw.pkg, pinned in claw.lock
/// from that same source, and its files still have the pinned hash.
fn verify(project: &Path, name: &str) -> Result<(), String> {
    if !project.join("claw.pkg").is_file() {
        return Err(format!("{name}: it is in .claw/pkg, but there is no claw.pkg next to it; list it in claw.pkg and run `claw get`"));
    }
    let deps = read_deps(&project.join("claw.pkg")).map_err(|e| format!("{name}: {e}"))?;
    let Some(dep) = deps.iter().find(|d| d.name == name) else {
        return Err(format!("{name}: it is in .claw/pkg but not in claw.pkg; list it there and run `claw get`"));
    };
    let lock = read_lock(&project.join("claw.lock")).map_err(|e| format!("{name}: {e}"))?;
    let Some(pin) = lock.iter().find(|p| p.name == name) else {
        return Err(format!("{name}: it is not in claw.lock; run `claw get`"));
    };
    let Some(source) = &pin.source else {
        return Err(format!("{name}: claw.lock was written by an older claw and does not say where the package came from; run `claw get` to check it"));
    };
    if *source != dep.source {
        return Err(source_changed(name, &dep.source, source));
    }
    let h = tree_hash(&project.join(".claw/pkg").join(name));
    if h != pin.hash {
        return Err(files_changed(name, &pin.hash, &h));
    }
    Ok(())
}
