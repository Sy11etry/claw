//! `claw lsp`: a language server over stdin and stdout, for any editor that speaks LSP.
//!
//! It runs the same front end as `claw check` on the text in the editor, not on the saved file, and
//! reports errors as diagnostics with the compiler's own patch as a quick fix. Hover shows a
//! declaration with its worked-out effects, go-to-definition finds it, the outline lists the file's
//! declarations, and formatting is `claw fmt`.

use crate::diag::{Diag, Span};
use crate::{effects, fmt, info, modules, sema};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;

// ---------------------------------------------------------------- JSON

#[derive(Clone, Debug, PartialEq)]
pub enum J {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

impl J {
    fn get(&self, key: &str) -> &J {
        if let J::Obj(items) = self {
            for (k, v) in items {
                if k == key {
                    return v;
                }
            }
        }
        &J::Null
    }
    fn str(&self) -> &str {
        match self {
            J::Str(s) => s,
            _ => "",
        }
    }
    fn int(&self) -> i64 {
        match self {
            J::Num(n) => *n as i64,
            _ => 0,
        }
    }
    fn items(&self) -> &[J] {
        match self {
            J::Arr(a) => a,
            _ => &[],
        }
    }
    fn write(&self, out: &mut String) {
        match self {
            J::Null => out.push_str("null"),
            J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            J::Num(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    out.push_str(&format!("{}", *n as i64));
                } else {
                    out.push_str(&format!("{n}"));
                }
            }
            J::Str(s) => write_str(s, out),
            J::Arr(a) => {
                out.push('[');
                for (i, x) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    x.write(out);
                }
                out.push(']');
            }
            J::Obj(items) => {
                out.push('{');
                for (i, (k, v)) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_str(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

struct JsonParser<'a> {
    b: &'a [u8],
    i: usize,
}

impl JsonParser<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> Option<()> {
        self.ws();
        (self.b.get(self.i) == Some(&c)).then(|| self.i += 1)
    }
    fn value(&mut self) -> Option<J> {
        self.ws();
        match *self.b.get(self.i)? {
            b'n' => self.word("null", J::Null),
            b't' => self.word("true", J::Bool(true)),
            b'f' => self.word("false", J::Bool(false)),
            b'"' => self.string().map(J::Str),
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                if self.eat(b']').is_some() {
                    return Some(J::Arr(items));
                }
                loop {
                    items.push(self.value()?);
                    if self.eat(b',').is_some() {
                        continue;
                    }
                    self.eat(b']')?;
                    return Some(J::Arr(items));
                }
            }
            b'{' => {
                self.i += 1;
                let mut items = Vec::new();
                if self.eat(b'}').is_some() {
                    return Some(J::Obj(items));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.eat(b':')?;
                    items.push((k, self.value()?));
                    if self.eat(b',').is_some() {
                        continue;
                    }
                    self.eat(b'}')?;
                    return Some(J::Obj(items));
                }
            }
            _ => {
                let start = self.i;
                while self.i < self.b.len() && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                std::str::from_utf8(&self.b[start..self.i]).ok()?.parse().ok().map(J::Num)
            }
        }
    }
    fn word(&mut self, w: &str, v: J) -> Option<J> {
        if self.b[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            Some(v)
        } else {
            None
        }
    }
    fn hex4(&mut self) -> Option<u32> {
        let h = std::str::from_utf8(self.b.get(self.i..self.i + 4)?).ok()?;
        self.i += 4;
        u32::from_str_radix(h, 16).ok()
    }
    fn string(&mut self) -> Option<String> {
        if self.b.get(self.i) != Some(&b'"') {
            return None;
        }
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = *self.b.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = *self.b.get(self.i)?;
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) && self.b[self.i..].starts_with(b"\\u") {
                                self.i += 2;
                                let lo = self.hex4()?;
                                cp = 0x10000 + ((cp - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
                            out.extend_from_slice(ch.to_string().as_bytes());
                        }
                        other => out.push(other),
                    }
                }
                c => out.push(c),
            }
        }
        Some(String::from_utf8_lossy(&out).into_owned())
    }
}

pub fn parse_json(text: &str) -> Option<J> {
    let mut p = JsonParser { b: text.as_bytes(), i: 0 };
    let v = p.value()?;
    p.ws();
    (p.i == p.b.len()).then_some(v)
}

fn obj(items: Vec<(&str, J)>) -> J {
    J::Obj(items.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}
fn num(n: usize) -> J {
    J::Num(n as f64)
}
fn text(s: &str) -> J {
    J::Str(s.to_string())
}

// ---------------------------------------------------------------- positions

/// The UTF-16 column of byte `at` within `line`: what the protocol counts.
fn utf16_col(line: &str, at: usize) -> usize {
    line[..at.min(line.len())].chars().map(|c| c.len_utf16()).sum()
}

/// (line, UTF-16 column), both from 0, of a byte offset.
fn position(src: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(src.len());
    let before = &src[..offset];
    let line = before.matches('\n').count();
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, utf16_col(&src[start..], offset - start))
}

/// The byte offset of (line, UTF-16 column).
fn offset_of(src: &str, line: usize, col: usize) -> usize {
    let mut start = 0;
    for _ in 0..line {
        match src[start..].find('\n') {
            Some(i) => start += i + 1,
            None => return src.len(),
        }
    }
    let rest = &src[start..];
    let end = rest.find('\n').unwrap_or(rest.len());
    let mut units = 0;
    for (i, c) in rest[..end].char_indices() {
        if units >= col {
            return start + i;
        }
        units += c.len_utf16();
    }
    start + end
}

fn range(src: &str, from: usize, to: usize) -> J {
    let (l0, c0) = position(src, from);
    let (l1, c1) = position(src, to.max(from));
    obj(vec![("start", obj(vec![("line", num(l0)), ("character", num(c0))])), ("end", obj(vec![("line", num(l1)), ("character", num(c1))]))])
}

/// The range a span covers; an empty span still marks one character, so the editor can underline it.
fn span_range(src: &str, s: Span) -> J {
    let end = if s.end > s.start { s.end as usize } else { s.start as usize + 1 };
    range(src, s.start as usize, end)
}

// ---------------------------------------------------------------- uris

fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&rest[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    Some(PathBuf::from(String::from_utf8_lossy(&out).into_owned()))
}

fn path_to_uri(path: &str) -> String {
    let mut out = String::from("file://");
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------- the server

struct Analysis {
    /// Per file: the diagnostics to show.
    diags: HashMap<String, Vec<(Diag, J)>>,
    /// Declarations, when the program got as far as being checked.
    front: Option<Front>,
}

struct Front {
    loaded: modules::Loaded,
    report: effects::Report,
}

struct Server {
    docs: HashMap<String, String>,
    /// Files that have diagnostics showing right now, so they can be cleared.
    shown: HashMap<String, Vec<String>>,
    last: HashMap<String, Analysis>,
    out: std::io::Stdout,
    shutdown: bool,
}

impl Server {
    fn send(&mut self, v: J) {
        let mut body = String::new();
        v.write(&mut body);
        let mut o = self.out.lock();
        let _ = write!(o, "Content-Length: {}\r\n\r\n{}", body.len(), body);
        let _ = o.flush();
    }

    fn reply(&mut self, id: &J, result: J) {
        self.send(obj(vec![("jsonrpc", text("2.0")), ("id", id.clone()), ("result", result)]));
    }

    fn fail(&mut self, id: &J, code: i64, msg: &str) {
        self.send(obj(vec![("jsonrpc", text("2.0")), ("id", id.clone()), ("error", obj(vec![("code", J::Num(code as f64)), ("message", text(msg))]))]));
    }

    fn notify(&mut self, method: &str, params: J) {
        self.send(obj(vec![("jsonrpc", text("2.0")), ("method", text(method)), ("params", params)]));
    }

    /// The text of a file as the editor has it, or as the disk has it.
    fn source(&self, path: &str) -> String {
        for (uri, t) in &self.docs {
            if uri_to_path(uri).is_some_and(|p| p.canonicalize().unwrap_or(p).to_string_lossy() == path) {
                return t.clone();
            }
        }
        std::fs::read_to_string(path).unwrap_or_default()
    }

    fn overlay(&self) {
        let mut files = HashMap::new();
        for (uri, t) in &self.docs {
            if let Some(p) = uri_to_path(uri) {
                files.insert(p.canonicalize().unwrap_or(p), t.clone());
            }
        }
        modules::set_overlay(files);
    }

    fn analyze(&mut self, uri: &str) {
        let Some(path) = uri_to_path(uri) else { return };
        self.overlay();
        let root = path.to_string_lossy().into_owned();
        let result = std::panic::catch_unwind(|| analyze_file(&root));
        let (raw, front) = match result {
            Ok(r) => r,
            Err(_) => ((Vec::new(), Vec::new()), None),
        };
        let files: Vec<String> = match &front {
            Some(f) => f.loaded.files.clone(),
            None => raw.1.clone(),
        };
        let mut by_file: HashMap<String, Vec<(Diag, J)>> = HashMap::new();
        for d in raw.0 {
            let Some(name) = files.get(d.span.file as usize) else { continue };
            let canon = std::path::Path::new(name).canonicalize().map_or(name.clone(), |p| p.to_string_lossy().into_owned());
            let src = self.source(&canon);
            let range = span_range(&src, d.span);
            by_file.entry(canon).or_default().push((d, range));
        }
        // Publish for every file with problems, and empty lists where there were some before.
        let mut now: Vec<String> = by_file.keys().cloned().collect();
        now.sort();
        let before = self.shown.remove(uri).unwrap_or_default();
        let mut targets: Vec<String> = now.clone();
        for old in before {
            if !targets.contains(&old) {
                targets.push(old);
            }
        }
        if !targets.contains(&root_canon(&root)) {
            targets.push(root_canon(&root));
        }
        for file in targets {
            let items: Vec<J> = by_file
                .get(&file)
                .map(|v| {
                    v.iter()
                        .map(|(d, r)| {
                            obj(vec![
                                ("range", r.clone()),
                                ("severity", num(1)),
                                ("code", text(d.code)),
                                ("source", text("claw")),
                                ("message", J::Str(match &d.fix {
                                    Some(f) => format!("{}; fix: {}", d.msg, f.label),
                                    None => d.msg.clone(),
                                })),
                            ])
                        })
                        .collect()
                })
                .unwrap_or_default();
            self.notify("textDocument/publishDiagnostics", obj(vec![("uri", J::Str(path_to_uri(&file))), ("diagnostics", J::Arr(items))]));
        }
        self.shown.insert(uri.to_string(), now);
        self.last.insert(uri.to_string(), Analysis { diags: by_file, front });
    }

    fn word_at(&self, uri: &str, params: &J) -> Option<String> {
        let src = self.docs.get(uri)?;
        let pos = params.get("position");
        let at = offset_of(src, pos.get("line").int() as usize, pos.get("character").int() as usize);
        let b = src.as_bytes();
        let ident = |c: u8| c == b'_' || c.is_ascii_alphanumeric();
        let (mut s, mut e) = (at.min(b.len()), at.min(b.len()));
        while s > 0 && ident(b[s - 1]) {
            s -= 1;
        }
        while e < b.len() && ident(b[e]) {
            e += 1;
        }
        (s < e).then(|| src[s..e].to_string())
    }

    fn handle(&mut self, msg: &J) {
        let method = msg.get("method").str().to_string();
        let id = msg.get("id").clone();
        let params = msg.get("params");
        let has_id = *msg.get("id") != J::Null;
        match method.as_str() {
            "initialize" => {
                let caps = obj(vec![
                    ("textDocumentSync", obj(vec![("openClose", J::Bool(true)), ("change", num(1)), ("save", J::Bool(true))])),
                    ("hoverProvider", J::Bool(true)),
                    ("definitionProvider", J::Bool(true)),
                    ("documentSymbolProvider", J::Bool(true)),
                    ("documentFormattingProvider", J::Bool(true)),
                    ("codeActionProvider", obj(vec![("codeActionKinds", J::Arr(vec![text("quickfix")]))])),
                ]);
                self.reply(&id, obj(vec![("capabilities", caps), ("serverInfo", obj(vec![("name", text("claw")), ("version", text(env!("CARGO_PKG_VERSION")))]))]));
            }
            "initialized" => {}
            "shutdown" => {
                self.shutdown = true;
                self.reply(&id, J::Null);
            }
            "exit" => std::process::exit(if self.shutdown { 0 } else { 1 }),
            "textDocument/didOpen" => {
                let d = params.get("textDocument");
                let uri = d.get("uri").str().to_string();
                self.docs.insert(uri.clone(), d.get("text").str().to_string());
                self.analyze(&uri);
            }
            "textDocument/didChange" => {
                let uri = params.get("textDocument").get("uri").str().to_string();
                // Whole-document sync: the last change holds the full text.
                if let Some(last) = params.get("contentChanges").items().last() {
                    self.docs.insert(uri.clone(), last.get("text").str().to_string());
                }
                self.analyze(&uri);
            }
            "textDocument/didSave" => {
                let uri = params.get("textDocument").get("uri").str().to_string();
                self.analyze(&uri);
            }
            "textDocument/didClose" => {
                let uri = params.get("textDocument").get("uri").str().to_string();
                self.docs.remove(&uri);
                self.last.remove(&uri);
                if let Some(files) = self.shown.remove(&uri) {
                    for f in files {
                        self.notify("textDocument/publishDiagnostics", obj(vec![("uri", J::Str(path_to_uri(&f))), ("diagnostics", J::Arr(vec![]))]));
                    }
                }
            }
            "textDocument/hover" => {
                let uri = params.get("textDocument").get("uri").str().to_string();
                let found = self.word_at(&uri, params).and_then(|w| {
                    let a = self.last.get(&uri)?;
                    let f = a.front.as_ref()?;
                    info::describe(&f.loaded.units, &f.report, &w).map(|(decl, _)| decl)
                });
                match found {
                    Some(decl) => self.reply(&id, obj(vec![("contents", obj(vec![("kind", text("markdown")), ("value", J::Str(format!("```claw\n{decl}\n```")))]))])),
                    None => self.reply(&id, J::Null),
                }
            }
            "textDocument/definition" => {
                let uri = params.get("textDocument").get("uri").str().to_string();
                let found = self.word_at(&uri, params).and_then(|w| {
                    let a = self.last.get(&uri)?;
                    let f = a.front.as_ref()?;
                    let (_, span) = info::describe(&f.loaded.units, &f.report, &w)?;
                    Some((f.loaded.files.get(span.file as usize)?.clone(), span))
                });
                match found {
                    Some((file, span)) => {
                        let canon = std::path::Path::new(&file).canonicalize().map_or(file, |p| p.to_string_lossy().into_owned());
                        let src = self.source(&canon);
                        let r = span_range(&src, span);
                        self.reply(&id, obj(vec![("uri", J::Str(path_to_uri(&canon))), ("range", r)]));
                    }
                    None => self.reply(&id, J::Null),
                }
            }
            "textDocument/documentSymbol" => {
                let uri = params.get("textDocument").get("uri").str().to_string();
                let symbols = self.docs.get(&uri).map(|src| outline(src)).unwrap_or_default();
                self.reply(&id, J::Arr(symbols));
            }
            "textDocument/formatting" => {
                let uri = params.get("textDocument").get("uri").str().to_string();
                let edits = match self.docs.get(&uri) {
                    Some(src) => match fmt::format(src) {
                        Ok(out) if out != *src => vec![obj(vec![("range", range(src, 0, src.len())), ("newText", J::Str(out))])],
                        _ => vec![],
                    },
                    None => vec![],
                };
                self.reply(&id, J::Arr(edits));
            }
            "textDocument/codeAction" => {
                let uri = params.get("textDocument").get("uri").str().to_string();
                let mut actions = Vec::new();
                if let (Some(a), Some(path)) = (self.last.get(&uri), uri_to_path(&uri)) {
                    let canon = path.canonicalize().unwrap_or(path).to_string_lossy().into_owned();
                    let (rs, re) = (params.get("range").get("start"), params.get("range").get("end"));
                    let asked = ((rs.get("line").int(), rs.get("character").int()), (re.get("line").int(), re.get("character").int()));
                    for (d, r) in a.diags.get(&canon).map(|v| v.as_slice()).unwrap_or(&[]) {
                        let Some(fix) = &d.fix else { continue };
                        let (s, e) = (r.get("start"), r.get("end"));
                        let theirs = ((s.get("line").int(), s.get("character").int()), (e.get("line").int(), e.get("character").int()));
                        if theirs.1 < asked.0 || asked.1 < theirs.0 {
                            continue;
                        }
                        let src = self.docs.get(&uri).cloned().unwrap_or_default();
                        let edit = obj(vec![("range", range(&src, fix.span.start as usize, fix.span.end as usize)), ("newText", J::Str(fix.text.clone()))]);
                        actions.push(obj(vec![
                            ("title", J::Str(format!("fix: {}", fix.label))),
                            ("kind", text("quickfix")),
                            ("isPreferred", J::Bool(fix.safe)),
                            ("edit", obj(vec![("changes", obj(vec![(uri.as_str(), J::Arr(vec![edit]))]))])),
                        ]));
                    }
                }
                self.reply(&id, J::Arr(actions));
            }
            _ => {
                if has_id {
                    self.fail(&id, -32601, &format!("method not supported: {method}"));
                }
            }
        }
    }
}

fn root_canon(root: &str) -> String {
    std::path::Path::new(root).canonicalize().map_or(root.to_string(), |p| p.to_string_lossy().into_owned())
}

type Raw = (Vec<Diag>, Vec<String>);

/// The front end on one file: the errors, and the program when it is good enough to ask about.
fn analyze_file(root: &str) -> (Raw, Option<Front>) {
    let loaded = match modules::load(root, false) {
        Ok(l) => l,
        Err((diags, files)) => return ((diags, files), None),
    };
    let mut diags = Vec::new();
    let checked = sema::check(&loaded.units, loaded.core, true, &mut diags);
    let report = effects::check(&loaded.units, &checked, &mut diags);
    crate::diag::sort(&mut diags);
    let files = loaded.files.clone();
    ((diags, files), Some(Front { loaded, report }))
}

/// The declarations of a file, for the editor's outline. Parses only.
fn outline(src: &str) -> Vec<J> {
    let mut diags = Vec::new();
    let toks = crate::lexer::lex(src, 0, &mut diags);
    if !diags.is_empty() {
        return Vec::new();
    }
    let program = crate::parser::Parser::new(toks, &mut diags).with_source(src).parse_program();
    if !diags.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut sym = |name: &str, kind: usize, span: Span| {
        let r = span_range(src, Span { end: span.start + name.len() as u32, ..span });
        out.push((span.start, obj(vec![("name", text(name)), ("kind", num(kind)), ("range", r.clone()), ("selectionRange", r)])));
    };
    for d in &program.types {
        let kind = if matches!(d.body, crate::ast::TypeBody::Struct(_)) { 23 } else { 10 };
        sym(&d.name, kind, d.span);
    }
    for c in &program.consts {
        sym(&c.name, 14, c.span);
    }
    for x in &program.externs {
        sym(&x.name, 12, x.span);
    }
    for f in program.funcs.iter().filter(|f| f.test.is_none() && !f.name.starts_with("const ")) {
        sym(&f.name, 12, f.name_span);
    }
    out.sort_by_key(|(at, _)| *at);
    out.into_iter().map(|(_, v)| v).collect()
}

fn read_message(input: &mut impl BufRead) -> Option<String> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.strip_prefix("Content-Length:") {
            length = v.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0u8; length?];
    input.read_exact(&mut body).ok()?;
    Some(String::from_utf8_lossy(&body).into_owned())
}

pub fn serve() -> ! {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut s = Server { docs: HashMap::new(), shown: HashMap::new(), last: HashMap::new(), out: std::io::stdout(), shutdown: false };
    while let Some(body) = read_message(&mut input) {
        if let Some(msg) = parse_json(&body) {
            s.handle(&msg);
        }
    }
    std::process::exit(if s.shutdown { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip() {
        let v = parse_json(r#"{"a":[1,2.5,"x\né😀"],"b":null,"c":true}"#).unwrap();
        let mut out = String::new();
        v.write(&mut out);
        assert_eq!(parse_json(&out).unwrap(), v);
        assert_eq!(v.get("a").items()[2].str(), "x\né😀");
    }

    #[test]
    fn positions_count_utf16() {
        let src = "ab\né😀x\n";
        assert_eq!(position(src, 0), (0, 0));
        assert_eq!(position(src, 3 + 2 + 4), (1, 3));
        assert_eq!(offset_of(src, 1, 3), 3 + 2 + 4);
    }
}
