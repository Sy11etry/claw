//! `use c "zlib.h"`: reads a C header and writes the Claw declarations for it.
//!
//! This is not a C compiler. It runs a small preprocessor (`#define`, `#if`, macros with parameters, quote
//! includes), then picks out what a caller needs: function prototypes become `extern fn`, structs with plain
//! fields become `type`, `enum` values and integer `#define`s become `const`. Anything it cannot read
//! (a struct passed by value, bit fields, unions, inline code) is left out and listed in a comment.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
enum T {
    Id(String),
    Num(String),
    Str,
    P(String),
}

impl T {
    fn is(&self, p: &str) -> bool {
        matches!(self, T::P(x) if x == p)
    }
    fn id(&self) -> Option<&str> {
        match self {
            T::Id(s) => Some(s),
            _ => None,
        }
    }
}

// ---------- tokens ----------

fn strip_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                if b[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            i += 2;
            out.push(' ');
        } else if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b[i] == b'"' {
            // Keep strings whole so `//` inside them survives.
            out.push('"');
            i += 1;
            while i < b.len() && b[i] != b'"' && b[i] != b'\n' {
                if b[i] == b'\\' && i + 1 < b.len() {
                    out.push(b[i] as char);
                    i += 1;
                }
                out.push(b[i] as char);
                i += 1;
            }
            if i < b.len() && b[i] == b'"' {
                out.push('"');
                i += 1;
            }
        } else {
            let ch = src[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn tokenize(line: &str) -> Vec<T> {
    let b: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_alphabetic() || c == '_' || c == '$' {
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '_' || b[i] == '$') {
                i += 1;
            }
            out.push(T::Id(b[s..i].iter().collect()));
        } else if c.is_ascii_digit() || (c == '.' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '.' || ((b[i] == '+' || b[i] == '-') && matches!(b[i - 1], 'e' | 'E') && !b[s..i].iter().collect::<String>().starts_with("0x"))) {
                i += 1;
            }
            out.push(T::Num(b[s..i].iter().collect()));
        } else if c == '"' {
            i += 1;
            while i < b.len() && b[i] != '"' {
                if b[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            out.push(T::Str);
        } else if c == '\'' {
            let s = i;
            i += 1;
            while i < b.len() && b[i] != '\'' {
                if b[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            let text: String = b[s..i.min(b.len())].iter().collect();
            let inner = text.trim_matches('\'');
            let v = match inner {
                "\\n" => 10,
                "\\t" => 9,
                "\\0" => 0,
                "\\\\" => 92,
                "\\'" => 39,
                s if s.chars().count() == 1 => s.chars().next().unwrap() as i64,
                _ => 0,
            };
            out.push(T::Num(v.to_string()));
        } else {
            let two: String = b[i..(i + 2).min(b.len())].iter().collect();
            let three: String = b[i..(i + 3).min(b.len())].iter().collect();
            if three == "..." {
                out.push(T::P(three));
                i += 3;
            } else if ["<<", ">>", "&&", "||", "==", "!=", "<=", ">=", "->", "##"].contains(&two.as_str()) {
                out.push(T::P(two));
                i += 2;
            } else {
                out.push(T::P(c.to_string()));
                i += 1;
            }
        }
    }
    out
}

// ---------- preprocessor ----------

#[derive(Clone)]
struct Macro {
    params: Option<Vec<String>>,
    body: Vec<T>,
}

struct Pre {
    macros: HashMap<String, Macro>,
    /// Names from `#define` in the header files themselves (not the predefined ones).
    defined_here: Vec<String>,
    incdirs: Vec<PathBuf>,
    out: Vec<T>,
    seen: HashSet<PathBuf>,
    depth: usize,
    /// `#pragma pack` in force, and the states saved by `push`.
    pack_on: bool,
    pack_stack: Vec<bool>,
    /// What a C compiler would refuse in these files (an `#if` without `#endif`).
    problems: Vec<String>,
}

fn expand(toks: &[T], macros: &HashMap<String, Macro>, hide: &[String], budget: &mut usize) -> Vec<T> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        let Some(name) = t.id() else {
            out.push(t.clone());
            i += 1;
            continue;
        };
        let Some(m) = macros.get(name).filter(|_| !hide.iter().any(|h| h == name) && *budget > 0) else {
            out.push(t.clone());
            i += 1;
            continue;
        };
        *budget -= 1;
        let mut hid = hide.to_vec();
        hid.push(name.to_string());
        match &m.params {
            None => {
                out.extend(expand(&m.body, macros, &hid, budget));
                i += 1;
            }
            Some(params) => {
                if !toks.get(i + 1).is_some_and(|t| t.is("(")) {
                    out.push(t.clone());
                    i += 1;
                    continue;
                }
                let mut depth = 0;
                let mut j = i + 1;
                let mut args: Vec<Vec<T>> = vec![Vec::new()];
                while j < toks.len() {
                    let x = &toks[j];
                    if x.is("(") {
                        depth += 1;
                        if depth > 1 {
                            args.last_mut().unwrap().push(x.clone());
                        }
                    } else if x.is(")") {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        args.last_mut().unwrap().push(x.clone());
                    } else if x.is(",") && depth == 1 {
                        args.push(Vec::new());
                    } else {
                        args.last_mut().unwrap().push(x.clone());
                    }
                    j += 1;
                }
                if args.len() == 1 && args[0].is_empty() && params.is_empty() {
                    args.clear();
                }
                let mut body = Vec::new();
                for b in &m.body {
                    if b.is("#") || b.is("##") {
                        continue;
                    }
                    match b.id().and_then(|n| params.iter().position(|p| p == n)) {
                        Some(k) => body.extend(args.get(k).cloned().unwrap_or_default()),
                        None => body.push(b.clone()),
                    }
                }
                out.extend(expand(&body, macros, &hid, budget));
                i = j + 1;
            }
        }
    }
    out
}

/// Evaluates an integer expression; `strict` makes an unknown name an error instead of 0.
struct Eval<'a> {
    t: &'a [T],
    i: usize,
    consts: &'a HashMap<String, i64>,
    strict: bool,
    ok: bool,
}

impl Eval<'_> {
    fn peek(&self) -> Option<&T> {
        self.t.get(self.i)
    }
    fn eat(&mut self, p: &str) -> bool {
        if self.peek().is_some_and(|t| t.is(p)) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expr(&mut self) -> i64 {
        let c = self.bin(0);
        if self.eat("?") {
            let a = self.expr();
            self.eat(":");
            let b = self.expr();
            return if c != 0 { a } else { b };
        }
        c
    }
    fn prec(op: &str) -> Option<u8> {
        Some(match op {
            "||" => 1,
            "&&" => 2,
            "|" => 3,
            "^" => 4,
            "&" => 5,
            "==" | "!=" => 6,
            "<" | ">" | "<=" | ">=" => 7,
            "<<" | ">>" => 8,
            "+" | "-" => 9,
            "*" | "/" | "%" => 10,
            _ => return None,
        })
    }
    fn bin(&mut self, min: u8) -> i64 {
        let mut l = self.unary();
        while let Some(T::P(op)) = self.peek().cloned() {
            let Some(p) = Self::prec(&op).filter(|p| *p >= min) else { break };
            self.i += 1;
            let r = self.bin(p + 1);
            l = match op.as_str() {
                "||" => (l != 0 || r != 0) as i64,
                "&&" => (l != 0 && r != 0) as i64,
                "|" => l | r,
                "^" => l ^ r,
                "&" => l & r,
                "==" => (l == r) as i64,
                "!=" => (l != r) as i64,
                "<" => (l < r) as i64,
                ">" => (l > r) as i64,
                "<=" => (l <= r) as i64,
                ">=" => (l >= r) as i64,
                "<<" => l.wrapping_shl(r as u32),
                ">>" => l.wrapping_shr(r as u32),
                "+" => l.wrapping_add(r),
                "-" => l.wrapping_sub(r),
                "*" => l.wrapping_mul(r),
                "/" if r != 0 => l / r,
                "%" if r != 0 => l % r,
                _ => {
                    self.ok = false;
                    0
                }
            };
        }
        l
    }
    fn unary(&mut self) -> i64 {
        if self.eat("!") {
            return (self.unary() == 0) as i64;
        }
        if self.eat("-") {
            return self.unary().wrapping_neg();
        }
        if self.eat("+") {
            return self.unary();
        }
        if self.eat("~") {
            return !self.unary();
        }
        if self.eat("(") {
            // A cast such as `(unsigned)5` or `(int)x`: skip the type words.
            let save = self.i;
            let mut words = 0;
            while let Some(T::Id(w)) = self.peek() {
                if matches!(w.as_str(), "int" | "unsigned" | "long" | "short" | "char" | "signed" | "size_t" | "uint32_t" | "int32_t" | "uint64_t" | "int64_t") {
                    words += 1;
                    self.i += 1;
                } else {
                    break;
                }
            }
            if words > 0 && self.eat(")") {
                return self.unary();
            }
            self.i = save;
            let v = self.expr();
            if !self.eat(")") {
                self.ok = false;
            }
            return v;
        }
        match self.peek().cloned() {
            Some(T::Num(n)) => {
                self.i += 1;
                parse_int(&n).unwrap_or_else(|| {
                    self.ok = false;
                    0
                })
            }
            Some(T::Id(n)) => {
                self.i += 1;
                if let Some(v) = self.consts.get(&n) {
                    return *v;
                }
                // `__has_include(x)` and friends in an `#if`: swallow the call, answer 0. A constant that calls
                // something (a function, an unmeasured `sizeof`) has no value here.
                if self.strict && self.peek().is_some_and(|t| t.is("(")) {
                    self.ok = false;
                    return 0;
                }
                if self.eat("(") {
                    let mut d = 1;
                    while d > 0 && self.i < self.t.len() {
                        if self.t[self.i].is("(") {
                            d += 1;
                        } else if self.t[self.i].is(")") {
                            d -= 1;
                        }
                        self.i += 1;
                    }
                    return 0;
                }
                if self.strict {
                    self.ok = false;
                }
                0
            }
            _ => {
                self.ok = false;
                0
            }
        }
    }
}

fn parse_int(s: &str) -> Option<i64> {
    let t = s.trim_end_matches(['u', 'U', 'l', 'L']);
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return u64::from_str_radix(h, 16).ok().map(|v| v as i64);
    }
    if let Some(b) = t.strip_prefix("0b") {
        return i64::from_str_radix(b, 2).ok();
    }
    if t.len() > 1 && t.starts_with('0') && t.chars().all(|c| c.is_ascii_digit()) {
        return i64::from_str_radix(&t[1..], 8).ok();
    }
    t.parse::<i64>().ok()
}

fn eval_expr(toks: &[T], consts: &HashMap<String, i64>, strict: bool) -> Option<i64> {
    let mut e = Eval { t: toks, i: 0, consts, strict, ok: true };
    let v = e.expr();
    (e.ok && e.i == toks.len()).then_some(v)
}

impl Pre {
    fn predefine(&mut self) {
        for (n, v) in [("__APPLE__", "1"), ("__MACH__", "1"), ("__LP64__", "1"), ("_LP64", "1"), ("__arm64__", "1"), ("__aarch64__", "1"), ("__GNUC__", "4"), ("__STDC__", "1"), ("__STDC_VERSION__", "201112L"), ("__x86_64__", "0")] {
            self.macros.insert(n.to_string(), Macro { params: None, body: tokenize(v) });
        }
        // `__x86_64__` must read as undefined, not as 0: drop it again.
        self.macros.remove("__x86_64__");
    }

    fn flush(&mut self, pending: &mut Vec<T>) {
        if pending.is_empty() {
            return;
        }
        let mut budget = 200_000;
        let ex = expand(pending, &self.macros, &[], &mut budget);
        self.out.extend(ex);
        pending.clear();
    }

    fn defined_pass(&self, toks: &[T]) -> Vec<T> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < toks.len() {
            if toks[i].id() == Some("defined") {
                let (name, next) = if toks.get(i + 1).is_some_and(|t| t.is("(")) {
                    (toks.get(i + 2).and_then(|t| t.id()).unwrap_or(""), i + 4)
                } else {
                    (toks.get(i + 1).and_then(|t| t.id()).unwrap_or(""), i + 2)
                };
                out.push(T::Num(if self.macros.contains_key(name) { "1" } else { "0" }.to_string()));
                i = next;
            } else {
                out.push(toks[i].clone());
                i += 1;
            }
        }
        out
    }

    fn cond(&self, toks: &[T]) -> bool {
        let t = self.defined_pass(toks);
        let mut budget = 10_000;
        let ex = expand(&t, &self.macros, &[], &mut budget);
        eval_expr(&ex, &HashMap::new(), false).unwrap_or(0) != 0
    }

    /// `#pragma pack(...)` changes the packing of the structs after it. Returns false for other pragmas. Forms this
    /// does not know count as packed: a struct they touch is refused, never laid out by guesswork.
    fn pragma_pack(&mut self, rest: &str) -> bool {
        let Some(args) = rest.trim().strip_prefix("pack") else { return false };
        let args = args.trim();
        if !args.is_empty() && !args.starts_with('(') {
            return false;
        }
        let inner = args.strip_prefix('(').and_then(|a| a.strip_suffix(')')).unwrap_or("");
        let parts: Vec<&str> = inner.split(',').map(str::trim).filter(|p| !p.is_empty()).collect();
        match parts.as_slice() {
            [] => self.pack_on = false,
            ["push"] => self.pack_stack.push(self.pack_on),
            ["push", n] => {
                self.pack_stack.push(self.pack_on);
                self.pack_on = *n != "0";
            }
            ["pop"] => self.pack_on = self.pack_stack.pop().unwrap_or(false),
            ["show"] => {}
            [n] => self.pack_on = *n != "0",
            _ => self.pack_on = true,
        }
        true
    }

    fn find(&self, name: &str, here: &Path) -> Option<PathBuf> {
        let local = here.parent().unwrap_or(Path::new(".")).join(name);
        if local.is_file() {
            return Some(local);
        }
        self.incdirs.iter().map(|d| d.join(name)).find(|p| p.is_file())
    }

    fn file(&mut self, path: &Path, owned: bool, parent: bool) {
        let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if self.depth > 12 || !self.seen.insert(key) {
            return;
        }
        let Ok(text) = std::fs::read_to_string(path) else { return };
        self.depth += 1;
        // Markers tell the declaration reader which tokens came from a file whose names the program should see.
        self.out.push(T::P(format!("@{}", owned as u8)));
        let text = strip_comments(&text).replace("\\\r\n", " ").replace("\\\n", " ");
        // (this region is live, some branch of this #if was taken, the parent region is live)
        let mut stack: Vec<(bool, bool, bool)> = Vec::new();
        let mut pending: Vec<T> = Vec::new();
        for line in text.lines() {
            let live = stack.last().map_or(true, |s| s.0);
            let trimmed = line.trim_start();
            let Some(dir) = trimmed.strip_prefix('#') else {
                if live {
                    pending.extend(tokenize(line));
                }
                continue;
            };
            let dir = dir.trim_start();
            let (word, rest) = dir.split_at(dir.find(|c: char| !c.is_ascii_alphanumeric() && c != '_').unwrap_or(dir.len()));
            match word {
                "if" | "ifdef" | "ifndef" => {
                    self.flush(&mut pending);
                    let parent = live;
                    let yes = parent
                        && match word {
                            "if" => self.cond(&tokenize(rest)),
                            "ifdef" => self.macros.contains_key(rest.trim()),
                            _ => !self.macros.contains_key(rest.trim()),
                        };
                    stack.push((yes, yes, parent));
                }
                "elif" | "else" | "endif" => {
                    self.flush(&mut pending);
                    match (word, stack.last().copied()) {
                        ("endif", Some(_)) => {
                            stack.pop();
                        }
                        ("else", Some((_, taken, parent))) => {
                            *stack.last_mut().unwrap() = (parent && !taken, true, parent);
                        }
                        ("elif", Some((_, taken, parent))) => {
                            let yes = parent && !taken && self.cond(&tokenize(rest));
                            *stack.last_mut().unwrap() = (yes, taken || yes, parent);
                        }
                        _ => {}
                    }
                }
                _ if !live => {}
                "pragma" => {
                    // The packing state rides along in the token stream, so the struct reader sees it where it is.
                    if self.pragma_pack(rest) {
                        self.flush(&mut pending);
                        self.out.push(T::P(if self.pack_on { "@p1" } else { "@p0" }.to_string()));
                    }
                }
                "define" => {
                    self.flush(&mut pending);
                    let r = rest.trim_start();
                    let name_end = r.find(|c: char| !c.is_ascii_alphanumeric() && c != '_').unwrap_or(r.len());
                    let name = r[..name_end].to_string();
                    if name.is_empty() {
                        continue;
                    }
                    let after = &r[name_end..];
                    // A `(` right after the name makes it a macro with parameters.
                    let (params, body) = if after.starts_with('(') {
                        let close = after.find(')').unwrap_or(after.len() - 1);
                        let ps: Vec<String> = after[1..close].split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty() && p != "...").collect();
                        (Some(ps), tokenize(&after[close + 1..]))
                    } else {
                        (None, tokenize(after))
                    };
                    if owned {
                        self.defined_here.push(name.clone());
                    }
                    self.macros.insert(name, Macro { params, body });
                }
                "undef" => {
                    self.flush(&mut pending);
                    self.macros.remove(rest.trim());
                }
                "include" => {
                    self.flush(&mut pending);
                    let r = rest.trim();
                    // Quote includes are the library's own files. Angle includes are followed only when they sit
                    // in a directory given with -I: the C standard types are built in.
                    let (name, quoted) = if let Some(q) = r.strip_prefix('"') { (q.trim_end_matches('"').to_string(), true) } else { (r.trim_matches(|c| c == '<' || c == '>').to_string(), false) };
                    let found = if quoted { self.find(&name, path) } else { self.incdirs.iter().map(|d| d.join(&name)).find(|p| p.is_file()) };
                    if let Some(f) = found {
                        // Plumbing headers (types, availability, machine parts) are read but their names stay hidden.
                        let rel = f.to_string_lossy().to_string();
                        let plumbing = ["/sys/", "/machine/", "/arm/", "/i386/", "/mach/", "/os/", "/_types", "Availability", "/xlocale", "/libkern/", "/mach-o/", "/net/", "/netinet"].iter().any(|x| rel.contains(x)) && !quoted;
                        let own = owned && !plumbing;
                        self.file(&f, own, owned);
                    }
                }
                _ => {}
            }
        }
        self.flush(&mut pending);
        if !stack.is_empty() {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            self.problems.push(format!("{name}: {} `#if` without `#endif`; a C compiler would refuse this header", stack.len()));
        }
        self.out.push(T::P(format!("@{}", parent as u8)));
        self.depth -= 1;
    }
}

// ---------- declarations ----------

#[derive(Clone, Debug, PartialEq)]
enum C {
    Void,
    Int { bits: u8, signed: bool },
    F32,
    F64,
    Bool,
    Ptr(Box<C>),
    /// A struct (or enum, union) by name.
    Struct(String),
    Enum,
    /// A function type, or a pointer to one.
    Func,
    Array(Box<C>, i64),
    Unknown,
}

struct Param {
    name: String,
    ty: C,
}

struct Proto {
    name: String,
    ret: C,
    params: Vec<Param>,
    variadic: bool,
    /// `static` or `inline` in the header: C compiles it into each caller, so a library may not have it.
    inline: bool,
}

#[derive(Default)]
struct Decls {
    typedefs: HashMap<String, C>,
    /// Structs with fields that can be written in Claw, in order of appearance.
    structs: Vec<(String, Vec<(String, C)>)>,
    /// Struct names we saw defined, even if their fields cannot be written.
    opaque: HashSet<String>,
    /// Structs laid out differently from Claw, by C name, and why (see `Header::packed`).
    layout: Vec<(String, &'static str)>,
    /// Typedef names whose type has a layout attribute (`vector_size`, `aligned`): a struct holding one changes too.
    layout_typedefs: HashMap<String, &'static str>,
    protos: Vec<Proto>,
    enums: Vec<(String, i64)>,
}

struct Parser<'a> {
    t: &'a [T],
    /// For each token: does it come from a file whose names the program should see?
    owns: &'a [bool],
    /// For each token: is `#pragma pack` in force there?
    packs: &'a [bool],
    i: usize,
    d: Decls,
    consts: HashMap<String, i64>,
    /// The layout change the attributes skipped since the struct that is being read began (see `skip_attrs`).
    layout: Option<&'static str>,
    /// Layout attributes read by `declaration` before the type it is about to read.
    decl_layout: Option<&'static str>,
}

const QUALIFIERS: &[&str] = &["const", "volatile", "restrict", "__restrict", "__restrict__", "_Nonnull", "_Nullable", "_Null_unspecified", "__nonnull", "__nullable", "__unused", "register", "_Noreturn", "__extension__", "inline", "__inline", "__inline__", "static", "extern", "_Atomic"];

fn builtin_typedef(name: &str) -> Option<C> {
    let i = |bits, signed| Some(C::Int { bits, signed });
    match name {
        "size_t" | "uintptr_t" | "uint64_t" | "u_int64_t" | "uintmax_t" | "__uint64_t" | "UInt64" | "uintptr" => i(64, false),
        "ssize_t" | "ptrdiff_t" | "intptr_t" | "int64_t" | "off_t" | "time_t" | "intmax_t" | "__int64_t" | "SInt64" | "clock_t" | "off64_t" => i(64, true),
        "uint32_t" | "u_int32_t" | "__uint32_t" | "UInt32" | "uid_t" | "gid_t" | "mode_t" | "socklen_t" | "wchar_t" => i(32, false),
        "int32_t" | "__int32_t" | "SInt32" | "pid_t" | "OSStatus" | "BOOL_INT" => i(32, true),
        "uint16_t" | "u_int16_t" | "__uint16_t" | "UInt16" => i(16, false),
        "int16_t" | "__int16_t" | "SInt16" => i(16, true),
        "uint8_t" | "u_int8_t" | "__uint8_t" | "UInt8" | "Byte" | "Bytef" | "uch" => i(8, false),
        "int8_t" | "__int8_t" | "SInt8" => i(8, true),
        "uInt" | "uInt_t" => i(32, false),
        "uLong" | "uLongf" => i(64, false),
        "uIntf" => i(32, false),
        "bool" | "_Bool" | "boolean_t" => Some(C::Bool),
        "va_list" | "FILE" | "DIR" => Some(C::Unknown),
        _ => None,
    }
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&T> {
        self.t.get(self.i)
    }
    fn at(&self, p: &str) -> bool {
        self.peek().is_some_and(|t| t.is(p))
    }
    fn eat(&mut self, p: &str) -> bool {
        if self.at(p) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn at_id(&self, s: &str) -> bool {
        self.peek().and_then(|t| t.id()) == Some(s)
    }

    /// Skips a balanced `( ... )`, `{ ... }` or `[ ... ]` group starting at the current open bracket.
    fn skip_group(&mut self) {
        let mut d = 0i32;
        while let Some(t) = self.peek() {
            if t.is("(") || t.is("{") || t.is("[") {
                d += 1;
            } else if t.is(")") || t.is("}") || t.is("]") {
                d -= 1;
            }
            self.i += 1;
            if d <= 0 {
                break;
            }
        }
    }

    /// Skips attributes and qualifiers. A layout attribute (`packed`, `aligned`, `_Alignas`) is noted in `self.layout`.
    fn skip_attrs(&mut self) {
        loop {
            match self.peek().and_then(|t| t.id()) {
                Some("__attribute__" | "__attribute" | "__asm" | "__asm__" | "asm" | "__declspec" | "_Alignas" | "alignas" | "__alignof__") => {
                    let from = self.i;
                    self.i += 1;
                    if self.at("(") {
                        self.skip_group();
                    }
                    if self.layout.is_none() {
                        self.layout = self.layout_reason(from, self.i);
                    }
                }
                Some(q) if QUALIFIERS.contains(&q) => self.i += 1,
                _ => break,
            }
        }
    }

    /// Skips attributes, and says which layout change they make, if any. The record of the struct being read is
    /// left as it was.
    fn skip_attrs_reason(&mut self) -> Option<&'static str> {
        let outer = self.layout.take();
        let from = self.i;
        self.skip_attrs();
        let r = self.layout_reason(from, self.i);
        self.layout = outer;
        r
    }

    /// The layout change made by the tokens from `from` to `to`, if any.
    fn layout_reason(&self, from: usize, to: usize) -> Option<&'static str> {
        let end = to.min(self.t.len());
        self.t.get(from..end)?.iter().find_map(|t| match t.id()? {
            "packed" | "__packed__" => Some("packed"),
            "aligned" | "__aligned__" | "_Alignas" | "alignas" => Some("aligned"),
            "vector_size" | "__vector_size__" | "ext_vector_type" => Some("vector type"),
            _ => None,
        })
    }

    /// Skip to just past the end of the current declaration.
    fn recover(&mut self) {
        while let Some(t) = self.peek() {
            if t.is(";") {
                self.i += 1;
                return;
            }
            if t.is("{") {
                self.skip_group();
                if self.at(";") {
                    self.i += 1;
                }
                return;
            }
            if t.is("(") || t.is("[") {
                self.skip_group();
                continue;
            }
            self.i += 1;
        }
    }

    /// A C type. A layout attribute met before a plain type (`__attribute__((aligned(8))) int x;`) changes the struct
    /// around it, so it is noted there; before a struct definition it belongs to that struct.
    fn base_type(&mut self) -> Option<C> {
        let mut pre = self.decl_layout.take();
        let c = self.base_type_in(&mut pre);
        add_reason(&mut self.layout, pre);
        c
    }

    fn base_type_in(&mut self, pre: &mut Option<&'static str>) -> Option<C> {
        let r = self.skip_attrs_reason();
        add_reason(pre, r);
        let (mut unsigned, mut signed, mut longs, mut short, mut ch, mut int, mut float, mut double, mut void, mut boolean) = (false, false, 0, false, false, false, false, false, false, false);
        let mut named: Option<C> = None;
        let mut any = false;
        loop {
            let r = self.skip_attrs_reason();
            add_reason(pre, r);
            let Some(T::Id(w)) = self.peek().cloned() else { break };
            match w.as_str() {
                "unsigned" => unsigned = true,
                "signed" | "__signed" | "__signed__" => signed = true,
                "long" => longs += 1,
                "short" => short = true,
                "char" => ch = true,
                "int" => int = true,
                "float" => float = true,
                "double" => double = true,
                "void" => void = true,
                "_Bool" => boolean = true,
                "struct" | "union" | "enum" => {
                    if named.is_some() || any {
                        break;
                    }
                    let kw = self.i;
                    self.i += 1;
                    // Layout attributes from here on belong to this struct; one nested in its body keeps its own.
                    let outer = self.layout.take();
                    self.skip_attrs();
                    let tag = self.peek().and_then(|t| t.id()).map(|s| s.to_string());
                    if tag.is_some() {
                        self.i += 1;
                    }
                    let kind = w.clone();
                    let name = tag.clone().unwrap_or_else(|| format!("anon{}", self.i));
                    let defines = self.at("{");
                    let own_pre = if defines { pre.take() } else { None };
                    let mut body = None;
                    if defines {
                        if kind == "enum" {
                            self.enum_body();
                        } else {
                            body = Some(self.struct_body(&name, kind == "union"));
                            self.skip_attrs(); // after the closing brace: `} __attribute__((packed))`
                        }
                    }
                    let pack = if self.packs.get(kw).copied().unwrap_or(false) { Some("#pragma pack") } else { None };
                    let own = self.layout.take().or(own_pre).or(pack);
                    self.layout = outer;
                    if let Some((ok, fields)) = body {
                        match own {
                            Some(why) => self.d.layout.push((name.clone(), why)),
                            None if ok && !fields.is_empty() && !self.d.structs.iter().any(|s| s.0 == name) => self.d.structs.push((name.clone(), fields)),
                            None => {}
                        }
                    }
                    named = Some(if kind == "enum" { C::Enum } else { C::Struct(name) });
                    any = true;
                    continue;
                }
                _ => {
                    if named.is_some() || any {
                        break;
                    }
                    if let Some(&why) = self.d.layout_typedefs.get(&w) {
                        add_reason(pre, Some(why));
                    }
                    if let Some(c) = builtin_typedef(&w).or_else(|| self.d.typedefs.get(&w).cloned()) {
                        named = Some(c);
                    } else if self.looks_like_type_name() {
                        named = Some(C::Unknown);
                    } else {
                        break;
                    }
                    any = true;
                    self.i += 1;
                    continue;
                }
            }
            any = true;
            self.i += 1;
        }
        if let Some(n) = named {
            return Some(n);
        }
        if !any {
            return None;
        }
        Some(if void {
            C::Void
        } else if boolean {
            C::Bool
        } else if float {
            C::F32
        } else if double {
            C::F64
        } else if ch {
            C::Int { bits: 8, signed: signed && !unsigned }
        } else if short {
            C::Int { bits: 16, signed: !unsigned }
        } else if longs > 0 {
            C::Int { bits: 64, signed: !unsigned }
        } else if int || unsigned || signed {
            C::Int { bits: 32, signed: !unsigned }
        } else {
            C::Unknown
        })
    }

    /// An unknown name is a type if it is followed by something that continues a declaration.
    fn looks_like_type_name(&self) -> bool {
        let next = self.t.get(self.i + 1);
        match next {
            Some(T::Id(_)) => true,
            Some(t) if t.is("*") => true,
            _ => false,
        }
    }

    fn enum_body(&mut self) {
        self.eat("{");
        let mut next = 0i64;
        while !self.at("}") && self.peek().is_some() {
            let Some(T::Id(name)) = self.peek().cloned() else {
                self.i += 1;
                continue;
            };
            self.i += 1;
            if self.eat("=") {
                let s = self.i;
                let mut d = 0;
                while let Some(t) = self.peek() {
                    if (t.is(",") || t.is("}")) && d == 0 {
                        break;
                    }
                    if t.is("(") {
                        d += 1;
                    } else if t.is(")") {
                        d -= 1;
                    }
                    self.i += 1;
                }
                if let Some(v) = eval_expr(&self.t[s..self.i], &self.consts, true) {
                    next = v;
                }
            }
            self.consts.insert(name.clone(), next);
            if self.owns.get(self.i).copied().unwrap_or(true) {
                self.d.enums.push((name, next));
            }
            next += 1;
            self.eat(",");
        }
        self.eat("}");
    }

    /// Reads a struct body. Returns whether every field can be written in Claw, and the fields.
    fn struct_body(&mut self, name: &str, is_union: bool) -> (bool, Vec<(String, C)>) {
        self.eat("{");
        let mut fields = Vec::new();
        let mut ok = !is_union;
        while !self.at("}") && self.peek().is_some() {
            let Some(base) = self.base_type() else {
                ok = false;
                self.recover();
                continue;
            };
            loop {
                let before = self.i;
                match self.declarator(&base) {
                    Some((n, ty, _)) if !n.is_empty() => {
                        // A bit field.
                        if self.at(":") {
                            ok = false;
                        }
                        fields.push((n, ty));
                    }
                    _ => ok = false,
                }
                if self.at(":") {
                    while !self.at(",") && !self.at(";") && self.peek().is_some() {
                        self.i += 1;
                    }
                }
                if self.i == before {
                    self.i += 1;
                }
                if !self.eat(",") {
                    break;
                }
            }
            if !self.eat(";") {
                ok = false;
                self.recover();
            }
        }
        self.eat("}");
        self.d.opaque.insert(name.to_string());
        (ok, fields)
    }

    /// One declarator: pointers, a name, array or function suffixes. Returns (name, type, parameters if a function).
    fn declarator(&mut self, base: &C) -> Option<(String, C, Option<(Vec<Param>, bool)>)> {
        let mut ty = base.clone();
        loop {
            self.skip_attrs();
            if self.eat("*") {
                ty = C::Ptr(Box::new(ty));
            } else {
                break;
            }
        }
        self.skip_attrs();
        // `(*name)(params)`: a function pointer.
        if self.at("(") && self.t.get(self.i + 1).is_some_and(|t| t.is("*") || t.is("^")) {
            self.i += 2;
            while self.eat("*") {}
            self.skip_attrs();
            let name = self.peek().and_then(|t| t.id()).map(|s| s.to_string()).unwrap_or_default();
            if !name.is_empty() {
                self.i += 1;
            }
            while self.at("[") {
                self.skip_group();
            }
            if !self.eat(")") {
                return None;
            }
            if self.at("(") {
                self.skip_group();
            }
            return Some((name, C::Ptr(Box::new(C::Func)), None));
        }
        let name = match self.peek() {
            Some(T::Id(n)) => {
                let n = n.clone();
                self.i += 1;
                n
            }
            _ => String::new(),
        };
        self.skip_attrs();
        if self.at("(") {
            let (params, variadic) = self.params()?;
            self.skip_attrs();
            return Some((name, ty, Some((params, variadic))));
        }
        while self.at("[") {
            let s = self.i + 1;
            self.skip_group();
            let toks = sizeof_pass(&self.t[s..self.i - 1], &self.d.sizes());
            // A size it cannot work out makes the field unknown, so the struct is reported, never guessed.
            ty = match eval_expr(&toks, &self.consts, true) {
                Some(n) if n > 0 => C::Array(Box::new(ty), n),
                _ if s == self.i - 1 => C::Array(Box::new(ty), 0),
                _ => C::Unknown,
            };
        }
        Some((name, ty, None))
    }

    fn params(&mut self) -> Option<(Vec<Param>, bool)> {
        self.eat("(");
        let mut out = Vec::new();
        let mut variadic = false;
        if self.at_id("void") && self.t.get(self.i + 1).is_some_and(|t| t.is(")")) {
            self.i += 2;
            return Some((out, false));
        }
        while !self.at(")") {
            if self.eat("...") {
                variadic = true;
                break;
            }
            let base = self.base_type()?;
            let (name, ty, f) = self.declarator(&base)?;
            let ty = match (f, ty) {
                (Some(_), _) => C::Ptr(Box::new(C::Func)),
                (None, C::Array(e, _)) => C::Ptr(e),
                (None, t) => t,
            };
            out.push(Param { name, ty });
            if !self.eat(",") {
                break;
            }
        }
        if !self.eat(")") {
            return None;
        }
        Some((out, variadic))
    }

    fn top(&mut self) {
        while self.peek().is_some() {
            let start = self.i;
            if self.eat(";") || self.eat("}") {
                continue;
            }
            if self.at_id("extern") && matches!(self.t.get(self.i + 1), Some(T::Str)) {
                self.i += 2;
                self.eat("{");
                continue;
            }
            if self.at_id("typedef") {
                self.i += 1;
                self.typedef();
            } else {
                self.declaration();
            }
            if self.i == start {
                self.i += 1;
            }
        }
    }

    fn typedef(&mut self) {
        // A layout attribute on the typedef (`typedef int v4 __attribute__((vector_size(16)));`) is noted for its name.
        let outer = self.layout.take();
        self.typedef_names();
        self.layout = outer;
    }

    fn typedef_names(&mut self) {
        let Some(base) = self.base_type() else {
            self.recover();
            return;
        };
        loop {
            let Some((name, ty, f)) = self.declarator(&base) else {
                self.recover();
                return;
            };
            let ty = if f.is_some() { C::Func } else { ty };
            if !name.is_empty() {
                if let Some(why) = self.layout {
                    self.d.layout_typedefs.insert(name.clone(), why);
                }
                // `typedef struct tag tag;` keeps the struct name as the type.
                self.d.typedefs.insert(name, ty);
            }
            if !self.eat(",") {
                break;
            }
        }
        if !self.eat(";") {
            self.recover();
        }
    }

    fn declaration(&mut self) {
        let owned = self.owns.get(self.i).copied().unwrap_or(true);
        let is_static = {
            let mut j = self.i;
            let mut s = false;
            while let Some(T::Id(w)) = self.t.get(j) {
                if w == "static" || w == "inline" || w == "__inline" {
                    s = true;
                } else if !QUALIFIERS.contains(&w.as_str()) {
                    break;
                }
                j += 1;
            }
            s
        };
        // Stray macro names in front of the type (`CURL_EXTERN CURLcode f(...)`). Layout attributes met here
        // belong to the type that follows, which `base_type` takes over.
        let mut pre = None;
        loop {
            add_reason(&mut pre, self.skip_attrs_reason());
            let Some(T::Id(w)) = self.peek().cloned() else { break };
            let known = builtin_typedef(&w).is_some() || self.d.typedefs.contains_key(&w) || matches!(w.as_str(), "unsigned" | "signed" | "long" | "short" | "char" | "int" | "float" | "double" | "void" | "_Bool" | "struct" | "union" | "enum");
            if known {
                break;
            }
            let next_known = match self.t.get(self.i + 1) {
                Some(T::Id(n)) => builtin_typedef(n).is_some() || self.d.typedefs.contains_key(n) || matches!(n.as_str(), "unsigned" | "signed" | "long" | "short" | "char" | "int" | "float" | "double" | "void" | "_Bool" | "struct" | "union" | "enum" | "const"),
                _ => false,
            };
            if next_known {
                self.i += 1;
            } else {
                break;
            }
        }
        self.decl_layout = pre;
        let Some(base) = self.base_type() else {
            self.recover();
            return;
        };
        if self.eat(";") {
            return;
        }
        loop {
            let Some((name, ty, f)) = self.declarator(&base) else {
                self.recover();
                return;
            };
            if let (Some((params, variadic)), false) = (f, name.is_empty()) {
                if owned {
                    self.d.protos.push(Proto { name, ret: ty, params, variadic, inline: is_static });
                }
            }
            self.skip_attrs();
            if self.at("{") {
                self.skip_group();
                return;
            }
            if self.eat("=") {
                self.recover();
                return;
            }
            if !self.eat(",") {
                break;
            }
        }
        if !self.eat(";") {
            self.recover();
        }
    }
}

/// What measuring a type needs: the structs (all of them, written or not), the odd layouts and the typedefs.
struct Sizes<'a> {
    structs: &'a [(String, Vec<(String, C)>)],
    layout: &'a [(String, &'static str)],
    typedefs: &'a HashMap<String, C>,
}

impl Decls {
    fn sizes(&self) -> Sizes<'_> {
        Sizes { structs: &self.structs, layout: &self.layout, typedefs: &self.typedefs }
    }
}

/// Size and alignment in bytes of a C type on the 64-bit targets, if this reader knows it.
fn size_align(c: &C, d: &Sizes, depth: usize) -> Option<(i64, i64)> {
    if depth > 32 {
        return None;
    }
    Some(match c {
        C::Int { bits, .. } => ((*bits / 8) as i64, (*bits / 8) as i64),
        C::F32 | C::Enum => (4, 4),
        C::F64 | C::Ptr(_) | C::Func => (8, 8),
        C::Bool => (1, 1),
        C::Array(e, n) => {
            let (s, a) = size_align(e, d, depth + 1)?;
            (s * n, a)
        }
        C::Struct(n) => {
            if d.layout.iter().any(|(p, _)| p == n) {
                return None;
            }
            let (_, fields) = d.structs.iter().find(|s| s.0 == *n)?;
            let (mut size, mut align) = (0i64, 1i64);
            for (_, f) in fields {
                let (fs, fa) = size_align(f, d, depth + 1)?;
                size = (size + fa - 1) / fa * fa + fs;
                align = align.max(fa);
            }
            ((size + align - 1) / align * align, align)
        }
        C::Void | C::Unknown => return None,
    })
}

/// The type named by the tokens inside `sizeof(...)`.
fn type_of_tokens(toks: &[T], d: &Sizes) -> Option<C> {
    if toks.iter().any(|t| t.is("*")) {
        return Some(C::Ptr(Box::new(C::Void)));
    }
    let words: Vec<&str> = toks.iter().map(|t| t.id()).collect::<Option<Vec<_>>>()?;
    let words: Vec<&str> = words.into_iter().filter(|w| !matches!(*w, "const" | "volatile" | "signed")).collect();
    match words.as_slice() {
        ["struct" | "union", name] => Some(C::Struct(name.to_string())),
        ["enum", _] => Some(C::Enum),
        [one] if builtin_typedef(one).is_some() || d.typedefs.contains_key(*one) => builtin_typedef(one).or_else(|| d.typedefs.get(*one).cloned()),
        ws => {
            let has = |w: &str| ws.contains(&w);
            let longs = ws.iter().filter(|w| **w == "long").count();
            Some(if has("char") {
                C::Int { bits: 8, signed: true }
            } else if has("short") {
                C::Int { bits: 16, signed: true }
            } else if has("double") {
                C::F64
            } else if has("float") {
                C::F32
            } else if has("_Bool") {
                C::Bool
            } else if longs > 0 {
                C::Int { bits: 64, signed: true }
            } else if has("int") || has("unsigned") {
                C::Int { bits: 32, signed: true }
            } else {
                return None;
            })
        }
    }
}

/// Replaces each `sizeof(type)` it can measure with the number of bytes, for array sizes and constants.
fn sizeof_pass(toks: &[T], d: &Sizes) -> Vec<T> {
    let mut out = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        if toks[i].id() == Some("sizeof") && toks.get(i + 1).is_some_and(|t| t.is("(")) {
            let mut depth = 0;
            let mut j = i + 1;
            while j < toks.len() {
                if toks[j].is("(") {
                    depth += 1;
                } else if toks[j].is(")") {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            if let Some((size, _)) = type_of_tokens(&toks[i + 2..j.min(toks.len())], d).and_then(|c| size_align(&c, d, 0)) {
                out.push(T::Num(size.to_string()));
                i = j + 1;
                continue;
            }
        }
        out.push(toks[i].clone());
        i += 1;
    }
    out
}

// ---------- Claw output ----------

fn claw_keywords() -> &'static [&'static str] {
    &["fn", "type", "use", "pub", "const", "mut", "own", "if", "else", "for", "in", "break", "continue", "match", "ret", "fail", "need", "sure", "test", "claw", "defer", "extern", "and", "or", "not", "true", "false", "none", "len", "print", "copy", "alloc", "free", "cast", "sizeof", "push", "pop", "insert", "remove", "clear", "reserve", "swap", "sort", "has", "index_of", "find", "contains", "starts_with", "ends_with", "trim", "split", "lines", "join", "upper", "lower", "to_int", "old", "main", "link", "min", "max", "abs", "new_map", "put", "get", "has_key", "del", "hash_int", "hash_str", "int", "bool", "str", "f32", "f64", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64"]
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn upper_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// A C name as a Claw function, field or parameter name, and the C symbol if they differ.
fn claw_ident(c: &str) -> String {
    let mut s = if c.starts_with('_') { format!("c{}", lower_first(c)) } else { lower_first(c) };
    if s.is_empty() {
        s = "x".into();
    }
    if claw_keywords().contains(&s.as_str()) {
        s.push('_');
    }
    s
}

struct Out<'a> {
    d: &'a Decls,
    type_names: HashMap<String, String>,
}

impl Out<'_> {
    fn ty(&self, c: &C, by_value_ok: bool) -> Option<String> {
        Some(match c {
            C::Void => return None,
            C::Int { bits, signed } => format!("{}{}", if *signed { "i" } else { "u" }, bits),
            C::F32 => "f32".into(),
            C::F64 => "f64".into(),
            C::Bool => "bool".into(),
            C::Enum => "i32".into(),
            C::Func | C::Unknown => return None,
            C::Struct(n) => {
                if by_value_ok {
                    self.type_names.get(n)?.clone()
                } else {
                    return None;
                }
            }
            C::Array(e, n) => {
                let inner = self.ty(e, true)?;
                format!("[{}]{}", n, inner)
            }
            C::Ptr(inner) => match &**inner {
                C::Void | C::Func | C::Unknown => "*u8".into(),
                C::Int { bits: 8, .. } => "*u8".into(),
                C::Struct(n) => match self.type_names.get(n) {
                    Some(t) => format!("*{t}"),
                    None => "*u8".into(),
                },
                C::Ptr(_) => format!("*{}", self.ty(inner, false).unwrap_or_else(|| "*u8".into())),
                other => format!("*{}", self.ty(other, true).unwrap_or_else(|| "u8".into())),
            },
        })
    }
}

/// Why a name from a C header is not what a program might expect, for the compiler's messages.
#[derive(Clone)]
pub enum CNote {
    /// The C function has another name in Claw (its first letter is made small).
    Renamed(String),
    /// Declared in the header, but not translated: why.
    Skipped(String),
    /// A macro with parameters: Claw does not translate those.
    Macro,
}

pub struct Header {
    pub source: String,
    /// Names a program may call that this header knows something about (by their C name), and the header's note.
    pub notes: Vec<(String, CNote)>,
    /// C names of `static` or `inline` functions that were written as `extern fn`: a library may not have them.
    pub inline: Vec<String>,
    /// What a C compiler would refuse in the header.
    pub problems: Vec<String>,
    /// Structs that C lays out differently from Claw (packed, aligned, or under `#pragma pack`): the name a program
    /// would use, and why. They are not written, so a program that names one gets E0182.
    pub packed: Vec<(String, String)>,
}

/// Why a prototype could not be written, in words for a message.
fn skip_reason(pr: &Proto, out: &Out) -> String {
    let by_value = |c: &C| matches!(c, C::Struct(_)) || matches!(c, C::Array(..));
    if by_value(&pr.ret) || pr.params.iter().any(|p| by_value(&p.ty)) {
        "it passes or returns a union, or a struct with a field Claw cannot hold, by value; call a C function that takes a pointer, or write a small C wrapper".to_string()
    } else {
        let bad: Vec<String> = pr.params.iter().filter(|p| out.ty(&p.ty, false).is_none()).map(|p| if p.name.is_empty() { "a parameter".to_string() } else { format!("`{}`", p.name) }).collect();
        if bad.is_empty() {
            "its result has a type Claw cannot hold".to_string()
        } else {
            format!("{} has a type Claw cannot hold", bad.join(", "))
        }
    }
}

/// Adds `r` unless a reason is already there.
fn add_reason(to: &mut Option<&'static str>, r: Option<&'static str>) {
    if to.is_none() {
        *to = r;
    }
}

/// Does a value of this type hold a packed struct by value, directly or in an array?
fn holds_packed(c: &C, layout: &[(String, &'static str)]) -> bool {
    match c {
        C::Struct(n) => layout.iter().any(|(p, _)| p == n),
        C::Array(e, _) => holds_packed(e, layout),
        _ => false,
    }
}

/// The name Claw gives a C struct or typedef name.
fn claw_type_name(shown: &str) -> String {
    let mut t = upper_first(shown.trim_start_matches('_'));
    if t.is_empty() || claw_keywords().contains(&t.as_str()) {
        t.push('T');
    }
    t
}

/// Reads `path` (and the quote includes inside it) and writes Claw source for it.
pub fn translate(root: &Path, incdirs: &[PathBuf]) -> Header {
    // A header from the system folders brings its own includes along; the standard types are then read from them.
    let mut dirs = incdirs.to_vec();
    for sys in SYSTEM_DIRS {
        if root.starts_with(sys) && !dirs.iter().any(|d| d == Path::new(sys)) {
            dirs.push(PathBuf::from(sys));
        }
    }
    let mut pre = Pre { macros: HashMap::new(), defined_here: Vec::new(), incdirs: dirs, out: Vec::new(), seen: HashSet::new(), depth: 0, pack_on: false, pack_stack: Vec::new(), problems: Vec::new() };
    pre.predefine();
    pre.file(root, true, true);
    // Pull the ownership and packing markers out of the stream; each remaining token keeps their state.
    let mut toks = Vec::with_capacity(pre.out.len());
    let mut owns = Vec::with_capacity(pre.out.len());
    let mut packs = Vec::with_capacity(pre.out.len());
    let mut own = true;
    let mut pack = false;
    for t in &pre.out {
        match t {
            T::P(m) if m == "@1" => own = true,
            T::P(m) if m == "@0" => own = false,
            T::P(m) if m == "@p1" => pack = true,
            T::P(m) if m == "@p0" => pack = false,
            t => {
                toks.push(t.clone());
                owns.push(own);
                packs.push(pack);
            }
        }
    }
    let mut p = Parser { t: &toks, owns: &owns, packs: &packs, i: 0, d: Decls::default(), consts: HashMap::new(), layout: None, decl_layout: None };
    p.top();
    let consts_from_enums = p.consts.clone();
    let mut d = p.d;
    // A struct that holds a packed one by value is laid out differently too; such structs are not written either.
    loop {
        let more: Vec<(String, &'static str)> = d.structs.iter().filter(|(n, _)| !d.layout.iter().any(|(p, _)| p == n)).filter(|(_, fields)| fields.iter().any(|(_, t)| holds_packed(t, &d.layout))).map(|(n, _)| (n.clone(), "contains a packed struct")).collect();
        if more.is_empty() {
            break;
        }
        d.layout.extend(more);
    }
    let layout = d.layout.clone();
    d.structs.retain(|s| !layout.iter().any(|(p, _)| *p == s.0));
    // Only structs that some function (or another needed struct) mentions are written out.
    fn mentions(c: &C, out: &mut Vec<String>) {
        match c {
            C::Struct(n) => out.push(n.clone()),
            C::Ptr(i) | C::Array(i, _) => mentions(i, out),
            _ => {}
        }
    }
    let mut want: Vec<String> = Vec::new();
    for pr in &d.protos {
        mentions(&pr.ret, &mut want);
        for q in &pr.params {
            mentions(&q.ty, &mut want);
        }
    }
    let mut seen_want: HashSet<String> = HashSet::new();
    while let Some(n) = want.pop() {
        if !seen_want.insert(n.clone()) {
            continue;
        }
        if let Some((_, fields)) = d.structs.iter().find(|s| s.0 == n) {
            for (_, t) in fields {
                mentions(t, &mut want);
            }
        }
    }
    // Every struct read stays measurable for `sizeof`, written or not.
    let all_structs = d.structs.clone();
    d.structs.retain(|s| seen_want.contains(&s.0));

    // Names of the Claw types written for structs.
    let mut type_names = HashMap::new();
    let mut used_names: HashSet<String> = HashSet::new();
    let alias_of: HashMap<String, String> = d.typedefs.iter().filter_map(|(k, v)| if let C::Struct(n) = v { Some((n.clone(), k.clone())) } else { None }).collect();
    for (name, _) in &d.structs {
        let shown = alias_of.get(name).cloned().unwrap_or_else(|| name.clone());
        let mut t = claw_type_name(&shown);
        while !used_names.insert(t.clone()) {
            t.push('_');
        }
        type_names.insert(name.clone(), t);
    }
    // The names a program would use for the structs it cannot have: the tag, and each typedef name for it.
    let mut packed_names: Vec<(String, String)> = Vec::new();
    for (c, why) in &layout {
        let mut names: Vec<String> = Vec::new();
        if !c.starts_with("anon") {
            names.push(claw_type_name(c));
        }
        for (k, v) in &d.typedefs {
            if matches!(v, C::Struct(n) if n == c) {
                names.push(claw_type_name(k));
            }
        }
        packed_names.extend(names.into_iter().map(|n| (n, why.to_string())));
    }
    let out = Out { d: &d, type_names };
    let _ = out.d;

    let mut s = String::new();
    s.push_str("// generated from a C header by `use c`\n");
    for (name, fields) in &d.structs {
        let tn = &out.type_names[name];
        let mut parts = Vec::new();
        let mut ok = true;
        let mut seen = HashSet::new();
        for (f, ty) in fields {
            match out.ty(ty, true) {
                Some(t) => {
                    let mut n = claw_ident(f);
                    while !seen.insert(n.clone()) {
                        n.push('_');
                    }
                    parts.push(format!("{n} {t}"));
                }
                None => ok = false,
            }
        }
        if ok {
            s.push_str(&format!("type {tn} {{ {} }}\n", parts.join(", ")));
        } else {
            s.push_str(&format!("// struct {name}: some field has a type Claw cannot hold; used through `*u8`\n"));
        }
    }
    for (name, why) in &packed_names {
        s.push_str(&format!("// struct {name}: laid out differently from Claw ({why}), so it is not written; a program that names it gets E0182\n"));
    }
    // Fix up: structs that failed above are not in the usable set; rebuild the map for the prototypes.
    let mut usable = HashMap::new();
    for (name, fields) in &d.structs {
        if fields.iter().all(|(_, t)| out.ty(t, true).is_some()) {
            usable.insert(name.clone(), out.type_names[name].clone());
        }
    }
    let out = Out { d: &d, type_names: usable };

    let mut names: HashSet<String> = HashSet::new();
    fn emit_const(names: &mut HashSet<String>, s: &mut String, n: &str, v: &str) {
        let mut id = lower_first(n.trim_start_matches('_')).to_lowercase();
        if id.is_empty() || claw_keywords().contains(&id.as_str()) {
            id.push('_');
        }
        if names.insert(id.clone()) {
            // C integers are `int`, which is `i32` in Claw; bigger values get the smallest type that holds them.
            let ty = match v.parse::<i64>() {
                Ok(n) if n >= i32::MIN as i64 && n <= i32::MAX as i64 => "i32",
                Ok(n) if n >= 0 && n <= u32::MAX as i64 => "u32",
                Ok(_) => "int",
                Err(_) => "f64",
            };
            s.push_str(&format!("const {id} {ty} = {v}\n"));
        }
    }
    for (n, v) in &d.enums {
        emit_const(&mut names, &mut s, n, &v.to_string());
    }
    let mut empty = HashMap::new();
    for (k, v) in &consts_from_enums {
        empty.insert(k.clone(), *v);
    }
    for n in &pre.defined_here {
        if names.contains(&n.to_lowercase()) {
            continue;
        }
        let Some(m) = pre.macros.get(n) else { continue };
        if m.params.is_some() || m.body.is_empty() || m.body.len() > 16 {
            continue;
        }
        let mut budget = 1000;
        let sizes = Sizes { structs: &all_structs, layout: &d.layout, typedefs: &d.typedefs };
        let body = sizeof_pass(&expand(&m.body, &pre.macros, &[], &mut budget), &sizes);
        if let Some(v) = eval_expr(&body, &empty, true) {
            emit_const(&mut names, &mut s, n, &v.to_string());
        } else if body.len() == 1 {
            if let T::Num(f) = &body[0] {
                let f = f.trim_end_matches(['f', 'F', 'l', 'L']);
                if f.contains('.') && f.parse::<f64>().is_ok() {
                    emit_const(&mut names, &mut s, n, if f.ends_with('.') { "0.0" } else { f });
                }
            }
        }
    }

    let mut done: HashSet<String> = HashSet::new();
    let mut fn_ids: HashSet<String> = HashSet::new();
    let mut skipped = Vec::new();
    let mut notes: Vec<(String, CNote)> = Vec::new();
    let mut inline_names: Vec<String> = Vec::new();
    // A function declared both plainly and as `inline` is the library's: the plain prototype wins.
    let mut protos: Vec<&Proto> = d.protos.iter().filter(|p| !p.inline).collect();
    protos.extend(d.protos.iter().filter(|p| p.inline));
    for pr in protos {
        if !done.insert(pr.name.clone()) {
            continue;
        }
        let mut params = Vec::new();
        let mut ok = true;
        for (k, p) in pr.params.iter().enumerate() {
            // A struct by value is fine when Claw can write the struct (see `cabi`).
            match out.ty(&p.ty, true) {
                Some(t) => {
                    // `__s` in a system header is just `s`: the underscores only keep C macros away from it.
                    let bare = p.name.trim_start_matches('_');
                    let mut n = if p.name.is_empty() || bare.is_empty() { format!("a{k}") } else { claw_ident(bare) };
                    while params.iter().any(|(x, _): &(String, String)| *x == n) {
                        n.push('_');
                    }
                    params.push((n, t));
                }
                None => ok = false,
            }
        }
        let ret = match &pr.ret {
            C::Void => Some(String::new()),
            r => out.ty(r, true).map(|t| format!(" {t}")),
        };
        let Some(ret) = ret.filter(|_| ok) else {
            if !pr.inline {
                skipped.push(pr.name.clone());
                notes.push((pr.name.clone(), CNote::Skipped(skip_reason(pr, &out))));
            }
            continue;
        };
        let id = claw_ident(&pr.name);
        if !fn_ids.insert(id.clone()) {
            continue;
        }
        if id != pr.name {
            notes.push((pr.name.clone(), CNote::Renamed(id.clone())));
        }
        if pr.inline {
            inline_names.push(pr.name.clone());
        }
        let ps: Vec<String> = params.iter().map(|(n, t)| format!("{n} {t}")).collect();
        let mut all = ps.join(", ");
        if pr.variadic {
            if !all.is_empty() {
                all.push_str(", ");
            }
            all.push_str("...");
        }
        let sym = if id == pr.name { String::new() } else { format!("\"{}\" ", pr.name) };
        s.push_str(&format!("extern {sym}fn {id}({all}){ret}\n"));
    }
    if !skipped.is_empty() {
        s.push_str(&format!("// not translated (a union or unknown type, or a struct Claw cannot hold): {}\n", skipped.join(" ")));
    }
    if !inline_names.is_empty() {
        s.push_str(&format!("// static or inline in the header, so a library may not have them (checked when the program is built): {}\n", inline_names.join(" ")));
    }
    let mut macros: Vec<&String> = pre.defined_here.iter().filter(|n| pre.macros.get(*n).is_some_and(|m| m.params.is_some()) && !done.contains(*n)).collect();
    macros.sort();
    macros.dedup();
    for m in &macros {
        notes.push((m.to_string(), CNote::Macro));
    }
    if !macros.is_empty() {
        let shown: Vec<&str> = macros.iter().take(30).map(|m| m.as_str()).collect();
        let more = if macros.len() > 30 { format!(" and {} more", macros.len() - 30) } else { String::new() };
        s.push_str(&format!("// macros with parameters are not translated: {}{more}\n", shown.join(" ")));
    }
    Header { source: s, packed: packed_names, notes, inline: inline_names, problems: pre.problems }
}

const SYSTEM_DIRS: &[&str] = &["/opt/homebrew/include", "/usr/local/include", "/usr/include", "/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk/usr/include", "/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk/usr/include"];

/// Where `use c "name.h"` looks: next to the program, in -I directories, then the usual places.
pub fn locate(name: &str, here: &Path, incdirs: &[PathBuf]) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = vec![here.parent().unwrap_or(Path::new(".")).to_path_buf()];
    dirs.extend(incdirs.iter().cloned());
    for d in SYSTEM_DIRS {
        dirs.push(PathBuf::from(d));
    }
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}
