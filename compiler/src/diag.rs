//! Compiler diagnostics: one compact line per error, or JSON for tools.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub line: u32,
    pub col: u32,
    /// Index into the list of source files.
    pub file: u32,
}

#[derive(Clone, Debug)]
pub struct Fix {
    pub span: Span,
    pub text: String,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct Diag {
    pub code: &'static str,
    pub span: Span,
    pub msg: String,
    pub fix: Option<Fix>,
}

impl Diag {
    pub fn new(code: &'static str, span: Span, msg: impl Into<String>) -> Diag {
        Diag { code, span, msg: msg.into(), fix: None }
    }

    pub fn fix(mut self, span: Span, text: impl Into<String>, label: impl Into<String>) -> Diag {
        self.fix = Some(Fix { span, text: text.into(), label: label.into() });
        self
    }
}

pub const MAX_ERRORS: usize = 20;

/// Orders errors by position and drops repeats (one template body, several instances).
pub fn sort(diags: &mut Vec<Diag>) {
    diags.sort_by_key(|d| (d.span.file, d.span.line, d.span.col));
    diags.dedup_by_key(|d| (d.code, d.span.file, d.span.start, d.msg.clone()));
}

pub fn render_text(files: &[String], diags: &[Diag]) -> String {
    let mut out = String::new();
    for d in diags.iter().take(MAX_ERRORS) {
        let file = &files[d.span.file as usize];
        out.push_str(&format!("{}:{}:{} {} {}", file, d.span.line, d.span.col, d.code, d.msg));
        if let Some(f) = &d.fix {
            out.push_str(&format!("; fix: {}", f.label));
        }
        out.push('\n');
    }
    if diags.len() > MAX_ERRORS {
        out.push_str(&format!("{} more errors not shown\n", diags.len() - MAX_ERRORS));
    }
    out
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn render_json(files: &[String], diags: &[Diag]) -> String {
    let items: Vec<String> = diags
        .iter()
        .take(MAX_ERRORS)
        .map(|d| {
            let fix = match &d.fix {
                Some(f) => format!(
                    "{{\"start\":{},\"end\":{},\"text\":{},\"label\":{}}}",
                    f.span.start,
                    f.span.end,
                    json_str(&f.text),
                    json_str(&f.label)
                ),
                None => "null".to_string(),
            };
            format!(
                "{{\"file\":{},\"line\":{},\"col\":{},\"code\":{},\"msg\":{},\"fix\":{}}}",
                json_str(&files[d.span.file as usize]),
                d.span.line,
                d.span.col,
                json_str(d.code),
                json_str(&d.msg),
                fix
            )
        })
        .collect();
    format!("[{}]\n", items.join(","))
}
