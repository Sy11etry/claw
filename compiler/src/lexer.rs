//! Tokens. Newlines are significant: a line ends a statement unless it ends
//! in an operator, a comma or an open bracket, or sits inside () or [].

use crate::diag::{Diag, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Int(i128),
    Float(f64),
    Char(u8),
    Str(String),
    Ident(String),
    Kw(&'static str),
    Op(&'static str),
    Newline,
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

pub const KEYWORDS: &[&str] = &[
    "fn", "type", "use", "pub", "const", "mut", "own", "if", "else", "for", "in", "break",
    "continue", "match", "ret", "fail", "need", "sure", "test", "claw", "defer", "extern", "and",
    "or", "not", "true", "false", "none",
];

const OPS: &[&str] = &[
    "<<=", ">>=", "...", ":=", "==", "!=", "<=", ">=", "<<", ">>", "+=", "-=", "*=", "/=", "%=", "&=",
    "|=", "^=", "..", "+", "-", "*", "/", "%", "&", "|", "^", "~", "<", ">", "=", "(", ")", "{",
    "}", "[", "]", ",", ".", ":", "!", "?",
];

fn continues_line(t: &Tok) -> bool {
    match t {
        Tok::Kw(k) => matches!(*k, "and" | "or" | "not"),
        Tok::Op(o) => !matches!(*o, ")" | "]" | "}" | "?" | "!" | "." | "..."),
        _ => false,
    }
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
    col: u32,
    file: u32,
}

impl<'a> Lexer<'a> {
    fn peek(&self, off: usize) -> u8 {
        *self.src.get(self.pos + off).unwrap_or(&0)
    }

    fn bump(&mut self) -> u8 {
        let c = self.peek(0);
        self.pos += 1;
        if c == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        c
    }

    fn span_from(&self, start: usize, line: u32, col: u32) -> Span {
        Span { start: start as u32, end: self.pos as u32, line, col, file: self.file }
    }
}

pub fn lex(src: &str, file: u32, diags: &mut Vec<Diag>) -> Vec<Token> {
    let mut l = Lexer { src: src.as_bytes(), pos: 0, line: 1, col: 1, file };
    let mut out: Vec<Token> = Vec::with_capacity(src.len() / 3);
    let mut brackets: Vec<u8> = Vec::new();

    loop {
        let c = l.peek(0);
        if l.pos >= l.src.len() {
            break;
        }
        let (start, line, col) = (l.pos, l.line, l.col);
        match c {
            b' ' | b'\t' | b'\r' => {
                l.bump();
            }
            b'\n' => {
                l.bump();
                let inside = matches!(brackets.last(), Some(b'(') | Some(b'['));
                let keep = match out.last() {
                    None => false,
                    Some(t) => !continues_line(&t.tok) && t.tok != Tok::Newline,
                };
                if keep && !inside {
                    out.push(Token { tok: Tok::Newline, span: l.span_from(start, line, col) });
                }
            }
            b'/' if l.peek(1) == b'/' => {
                while l.pos < l.src.len() && l.peek(0) != b'\n' {
                    l.bump();
                }
            }
            b'0'..=b'9' => out.push(lex_number(&mut l, diags)),
            b'"' => out.push(lex_string(&mut l, diags)),
            b'\'' => out.push(lex_char(&mut l, diags)),
            c if c == b'_' || c.is_ascii_alphabetic() => {
                while l.peek(0) == b'_' || l.peek(0).is_ascii_alphanumeric() {
                    l.bump();
                }
                let text = &src[start..l.pos];
                let tok = match KEYWORDS.iter().find(|k| **k == text) {
                    Some(k) => Tok::Kw(k),
                    None => Tok::Ident(text.to_string()),
                };
                out.push(Token { tok, span: l.span_from(start, line, col) });
            }
            _ => {
                let rest = &l.src[l.pos..];
                match OPS.iter().find(|o| rest.starts_with(o.as_bytes())) {
                    Some(op) => {
                        for _ in 0..op.len() {
                            l.bump();
                        }
                        match *op {
                            "(" => brackets.push(b'('),
                            "[" => brackets.push(b'['),
                            "{" => brackets.push(b'{'),
                            ")" | "]" | "}" => {
                                brackets.pop();
                            }
                            _ => {}
                        }
                        out.push(Token { tok: Tok::Op(op), span: l.span_from(start, line, col) });
                    }
                    None => {
                        // Consume a whole UTF-8 character so the report points at it.
                        l.bump();
                        while l.peek(0) & 0xC0 == 0x80 {
                            l.bump();
                        }
                        let span = l.span_from(start, line, col);
                        let text = &src[start..l.pos];
                        diags.push(Diag::new("E0001", span, format!("unexpected character `{text}`")));
                    }
                }
            }
        }
    }
    let span = Span { start: l.pos as u32, end: l.pos as u32, line: l.line, col: l.col, file };
    if out.last().map(|t| t.tok != Tok::Newline).unwrap_or(false) {
        out.push(Token { tok: Tok::Newline, span });
    }
    out.push(Token { tok: Tok::Eof, span });
    out
}

fn lex_number(l: &mut Lexer, diags: &mut Vec<Diag>) -> Token {
    let (start, line, col) = (l.pos, l.line, l.col);
    let radix = match (l.peek(0), l.peek(1)) {
        (b'0', b'x') => 16,
        (b'0', b'b') => 2,
        _ => 10,
    };
    if radix != 10 {
        l.bump();
        l.bump();
    }
    let mut value: i128 = 0;
    let mut digits = 0;
    let mut overflow = false;
    loop {
        let c = l.peek(0);
        if c == b'_' {
            l.bump();
            continue;
        }
        let d = match (c as char).to_digit(radix) {
            Some(d) => d,
            None => break,
        };
        l.bump();
        digits += 1;
        match value.checked_mul(radix as i128).and_then(|v| v.checked_add(d as i128)) {
            Some(v) => value = v,
            None => overflow = true,
        }
    }
    if radix == 10 {
        let frac = l.peek(0) == b'.' && l.peek(1).is_ascii_digit();
        let next_after_e = if matches!(l.peek(1), b'+' | b'-') { l.peek(2) } else { l.peek(1) };
        let exp_now = matches!(l.peek(0), b'e' | b'E') && next_after_e.is_ascii_digit();
        if frac || exp_now {
            if frac {
                l.bump();
                while l.peek(0).is_ascii_digit() || l.peek(0) == b'_' {
                    l.bump();
                }
            }
            let next_after_e = if matches!(l.peek(1), b'+' | b'-') { l.peek(2) } else { l.peek(1) };
            if matches!(l.peek(0), b'e' | b'E') && next_after_e.is_ascii_digit() {
                l.bump();
                if matches!(l.peek(0), b'+' | b'-') {
                    l.bump();
                }
                while l.peek(0).is_ascii_digit() {
                    l.bump();
                }
            }
            let span = l.span_from(start, line, col);
            let text: String = std::str::from_utf8(&l.src[start..l.pos]).unwrap_or("0").chars().filter(|c| *c != '_').collect();
            return Token { tok: Tok::Float(text.parse().unwrap_or(0.0)), span };
        }
    }
    let span = l.span_from(start, line, col);
    if digits == 0 {
        diags.push(Diag::new("E0003", span, "number has no digits"));
    } else if l.peek(0) == b'_' || l.peek(0).is_ascii_alphabetic() {
        diags.push(Diag::new("E0003", span, "number runs into a name; add a space"));
    }
    if overflow {
        value = i128::MAX;
    }
    Token { tok: Tok::Int(value), span }
}

fn lex_string(l: &mut Lexer, diags: &mut Vec<Diag>) -> Token {
    let (start, line, col) = (l.pos, l.line, l.col);
    l.bump();
    let mut bytes = Vec::new();
    loop {
        if l.pos >= l.src.len() || l.peek(0) == b'\n' {
            let span = l.span_from(start, line, col);
            diags.push(Diag::new("E0002", span, "string is not closed; add `\"`"));
            break;
        }
        let c = l.bump();
        match c {
            b'"' => break,
            b'\\' => {
                let e = l.bump();
                match e {
                    b'n' => bytes.push(b'\n'),
                    b't' => bytes.push(b'\t'),
                    b'r' => bytes.push(b'\r'),
                    b'0' => bytes.push(0),
                    b'\\' => bytes.push(b'\\'),
                    b'"' => bytes.push(b'"'),
                    _ => {
                        let span = l.span_from(l.pos - 2, l.line, l.col.saturating_sub(2));
                        diags.push(Diag::new("E0004", span, "unknown escape; use \\n \\t \\r \\0 \\\\ \\\""));
                    }
                }
            }
            c => bytes.push(c),
        }
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Token { tok: Tok::Str(text), span: l.span_from(start, line, col) }
}

fn lex_char(l: &mut Lexer, diags: &mut Vec<Diag>) -> Token {
    let (start, line, col) = (l.pos, l.line, l.col);
    l.bump();
    let c = l.bump();
    let value = if c == b'\\' {
        match l.bump() {
            b'n' => b'\n',
            b't' => b'\t',
            b'r' => b'\r',
            b'0' => 0,
            b'\\' => b'\\',
            b'\'' => b'\'',
            _ => {
                let span = l.span_from(start, line, col);
                diags.push(Diag::new("E0004", span, "unknown escape; use \\n \\t \\r \\0 \\\\ \\'"));
                0
            }
        }
    } else {
        c
    };
    if l.peek(0) != b'\'' || c >= 0x80 {
        let span = l.span_from(start, line, col);
        diags.push(Diag::new("E0005", span, "a character literal holds exactly one ASCII character; close it with `'`"));
        while l.pos < l.src.len() && l.peek(0) != b'\'' && l.peek(0) != b'\n' {
            l.bump();
        }
    }
    if l.peek(0) == b'\'' {
        l.bump();
    }
    Token { tok: Tok::Char(value), span: l.span_from(start, line, col) }
}
