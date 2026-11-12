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
                // `__has_include(x)` and friends: swallow the call, answer 0.
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
}

#[derive(Default)]
struct Decls {
    typedefs: HashMap<String, C>,
    /// Structs with fields that can be written in Claw, in order of appearance.
    structs: Vec<(String, Vec<(String, C)>)>,
    /// Struct names we saw defined, even if their fields cannot be written.
    opaque: HashSet<String>,
    protos: Vec<Proto>,
    enums: Vec<(String, i64)>,
}

struct Parser<'a> {
    t: &'a [T],
    /// For each token: does it come from a file whose names the program should see?
    owns: &'a [bool],
    i: usize,
    d: Decls,
    consts: HashMap<String, i64>,
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

    fn skip_attrs(&mut self) {
        loop {
            match self.peek().and_then(|t| t.id()) {
                Some("__attribute__" | "__attribute" | "__asm" | "__asm__" | "asm" | "__declspec" | "_Alignas" | "__alignof__") => {
                    self.i += 1;
                    if self.at("(") {
                        self.skip_group();
                    }
                }
                Some(q) if QUALIFIERS.contains(&q) => self.i += 1,
                _ => break,
            }
        }
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

    fn base_type(&mut self) -> Option<C> {
        self.skip_attrs();
        let (mut unsigned, mut signed, mut longs, mut short, mut ch, mut int, mut float, mut double, mut void, mut boolean) = (false, false, 0, false, false, false, false, false, false, false);
        let mut named: Option<C> = None;
        let mut any = false;
        loop {
            self.skip_attrs();
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
                    self.i += 1;
                    self.skip_attrs();
                    let tag = self.peek().and_then(|t| t.id()).map(|s| s.to_string());
                    if tag.is_some() {
                        self.i += 1;
                    }
                    let kind = w.clone();
                    let name = tag.clone().unwrap_or_else(|| format!("anon{}", self.i));
                    if self.at("{") {
                        if kind == "enum" {
                            self.enum_body();
                        } else {
                            self.struct_body(&name, kind == "union");
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

    fn struct_body(&mut self, name: &str, is_union: bool) {
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
        if ok && !fields.is_empty() && !self.d.structs.iter().any(|s| s.0 == name) {
            self.d.structs.push((name.to_string(), fields));
        }
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
            let n = eval_expr(&self.t[s..self.i - 1], &self.consts, true).unwrap_or(0);
            ty = C::Array(Box::new(ty), n);
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
        // Stray macro names in front of the type (`CURL_EXTERN CURLcode f(...)`).
        loop {
            self.skip_attrs();
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
                if !is_static && owned {
                    self.d.protos.push(Proto { name, ret: ty, params, variadic });
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

pub struct Header {
    pub source: String,
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
    let mut pre = Pre { macros: HashMap::new(), defined_here: Vec::new(), incdirs: dirs, out: Vec::new(), seen: HashSet::new(), depth: 0 };
    pre.predefine();
    pre.file(root, true, true);
    // Pull the ownership markers out of the stream.
    let mut toks = Vec::with_capacity(pre.out.len());
    let mut owns = Vec::with_capacity(pre.out.len());
    let mut own = true;
    for t in &pre.out {
        match t {
            T::P(m) if m == "@1" => own = true,
            T::P(m) if m == "@0" => own = false,
            t => {
                toks.push(t.clone());
                owns.push(own);
            }
        }
    }
    let mut p = Parser { t: &toks, owns: &owns, i: 0, d: Decls::default(), consts: HashMap::new() };
    p.top();
    let consts_from_enums = p.consts.clone();
    let mut d = p.d;
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
    d.structs.retain(|s| seen_want.contains(&s.0));

    // Names of the Claw types written for structs.
    let mut type_names = HashMap::new();
    let mut used_names: HashSet<String> = HashSet::new();
    let alias_of: HashMap<String, String> = d.typedefs.iter().filter_map(|(k, v)| if let C::Struct(n) = v { Some((n.clone(), k.clone())) } else { None }).collect();
    for (name, _) in &d.structs {
        let shown = alias_of.get(name).cloned().unwrap_or_else(|| name.clone());
        let mut t = upper_first(shown.trim_start_matches('_'));
        if t.is_empty() || claw_keywords().contains(&t.as_str()) {
            t.push('T');
        }
        while !used_names.insert(t.clone()) {
            t.push('_');
        }
        type_names.insert(name.clone(), t);
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
        let body = expand(&m.body, &pre.macros, &[], &mut budget);
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
    for pr in &d.protos {
        if !done.insert(pr.name.clone()) {
            continue;
        }
        let mut params = Vec::new();
        let mut ok = true;
        for (k, p) in pr.params.iter().enumerate() {
            match out.ty(&p.ty, false) {
                Some(t) => {
                    let mut n = if p.name.is_empty() { format!("a{k}") } else { claw_ident(&p.name) };
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
            r => out.ty(r, false).map(|t| format!(" {t}")),
        };
        let Some(ret) = ret.filter(|_| ok) else {
            skipped.push(pr.name.clone());
            continue;
        };
        let id = claw_ident(&pr.name);
        if !fn_ids.insert(id.clone()) {
            continue;
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
        s.push_str(&format!("// not translated (struct by value, union or unknown type): {}\n", skipped.join(" ")));
    }
    Header { source: s }
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
