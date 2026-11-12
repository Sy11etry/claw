//! `claw fmt`: one canonical layout, no options.
//!
//! Works on tokens rather than the syntax tree, so comments survive. After
//! formatting, the token stream is compared with the original: the formatter
//! can only ever change whitespace, or it refuses to write anything.

use crate::diag::Diag;
use crate::lexer::{self, Tok, Token};

fn is_comment_line(line: &str) -> Option<&str> {
    let t = line.trim_start();
    t.strip_prefix("//").map(|_| t)
}

/// Splits a source line into its code and its trailing comment, respecting strings.
fn split_comment(line: &str) -> (&str, Option<&str>) {
    let b = line.as_bytes();
    let mut i = 0;
    let mut in_str = false;
    let mut in_char = false;
    while i < b.len() {
        match b[i] {
            b'\\' if in_str || in_char => i += 1,
            b'"' if !in_char => in_str = !in_str,
            b'\'' if !in_str => in_char = !in_char,
            b'/' if !in_str && !in_char && b.get(i + 1) == Some(&b'/') => {
                return (line[..i].trim_end(), Some(&line[i..]));
            }
            _ => {}
        }
        i += 1;
    }
    (line.trim_end(), None)
}

fn is_type_name(s: &str) -> bool {
    s.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false)
}

fn value_end(t: &Tok) -> bool {
    match t {
        Tok::Int(_) | Tok::Float(_) | Tok::Char(_) | Tok::Str(_) | Tok::Ident(_) => true,
        Tok::Kw(k) => matches!(*k, "true" | "false" | "none"),
        Tok::Op(o) => matches!(*o, ")" | "]" | "}" | "?"),
        _ => false,
    }
}

/// What sits between two tokens on one line.
fn needs_space(prev: &Tok, cur: &Tok, prev2: Option<&Tok>, adjacent: bool, gap_before_prev: bool) -> bool {
    use Tok::*;
    // `s *u8`: glued to what follows but not to what precedes, so a prefix.
    if matches!(prev, Op("*") | Op("&")) && adjacent && gap_before_prev {
        return false;
    }
    if matches!((prev, cur), (Op(","), Op("..."))) {
        return true;
    }
    let unary_prev = matches!(prev, Op("-") | Op("~") | Op("&") | Op("*") | Op("!"))
        && !prev2.map(value_end).unwrap_or(false);
    if unary_prev {
        return false;
    }
    match (prev, cur) {
        // `?` is a postfix after a value and a prefix on a type; the source says which.
        (_, Op("?")) | (Op("?"), _) => !adjacent,
        // Effects: `+mem`, `+io`.
        (Op("+"), Ident(n)) if adjacent && (n == "mem" || n == "io") => false,
        (_, Op(",")) | (_, Op(")")) | (_, Op("]")) | (_, Op("...")) => false,
        (_, Op(":")) => false,
        (Op("(") | Op("["), _) => false,
        (Op("."), _) | (_, Op(".")) => false,
        (Op(".."), _) | (_, Op("..")) => false,
        // `[4]int`: nothing between the bracket and the element type.
        (Op("]"), Ident(_)) | (Op("]"), Op("[")) => false,
        (Ident(_), Op("(")) => false,
        // `fn(int) int`, `[2]fn(int) int`, `pick(true)(4, 5)`.
        (Kw("fn"), Op("(")) | (Op("]"), Kw("fn")) => false,
        (Op(")"), Op("(")) => !adjacent,
        (Ident(_), Op("[")) | (Op(")"), Op("[")) | (Op("]"), Op("(")) => !adjacent,
        (Ident(n), Op("{")) if is_type_name(n) => !adjacent,
        (Op("]"), Op("{")) => !adjacent,
        (Op("{"), Op("}")) => false,
        _ => true,
    }
}

fn format_code(line: &str, diags: &mut Vec<Diag>) -> (String, Vec<Token>) {
    let toks = lexer::lex(line, 0, diags);
    let mut out = String::new();
    let real: Vec<&Token> = toks.iter().filter(|t| !matches!(t.tok, Tok::Newline | Tok::Eof)).collect();
    let type_decl = matches!(real.first().map(|t| &t.tok), Some(Tok::Kw("type"))) && matches!(real.get(2).map(|t| &t.tok), Some(Tok::Op("{")));
    // For each open `{` on this line: is it a literal like `Point{1, 2}`?
    let mut braces: Vec<bool> = Vec::new();
    // In a signature or `if`/`for`/`match` head a bare `Name {` opens a block.
    let head = match (real.first().map(|t| &t.tok), real.get(1).map(|t| &t.tok)) {
        (Some(Tok::Kw("fn" | "if" | "for" | "match" | "claw" | "pub" | "extern")), _) => true,
        (Some(Tok::Op("}")), Some(Tok::Kw("else"))) => true,
        _ => false,
    };
    let mut parens = 0;
    for (i, t) in real.iter().enumerate() {
        if i > 0 {
            let adjacent = real[i - 1].span.end == t.span.start;
            let prev2 = if i >= 2 { Some(&real[i - 2].tok) } else { None };
            let gap_before_prev = i < 2 || real[i - 2].span.end != real[i - 1].span.start;
            let mut space = needs_space(&real[i - 1].tok, &t.tok, prev2, adjacent, gap_before_prev);
            let empty = real[i - 1].tok == Tok::Op("{") && t.tok == Tok::Op("}");
            if !empty {
                if real[i - 1].tok == Tok::Op("{") {
                    space = !braces.last().copied().unwrap_or(false);
                }
                if t.tok == Tok::Op("}") {
                    space = !braces.last().copied().unwrap_or(false);
                }
            }
            if head && parens == 0 && t.tok == Tok::Op("{") {
                space = true;
            }
            // The result type of a signature: `fn f() []int`.
            let signature = matches!(real.first().map(|x| &x.tok), Some(Tok::Kw("fn" | "claw" | "pub" | "extern")));
            if signature && parens == 0 && real[i - 1].tok == Tok::Op(")") && matches!(t.tok, Tok::Op("[") | Tok::Op("?")) {
                space = true;
            }
            if type_decl && (i == 2 || i == 3 || (i + 1 == real.len() && t.tok == Tok::Op("}"))) {
                space = true;
            }
            if space {
                out.push(' ');
            }
        }
        match t.tok {
            Tok::Op("{") => {
                let literal = i > 0
                    && real[i - 1].span.end == t.span.start
                    && (matches!(&real[i - 1].tok, Tok::Ident(n) if is_type_name(n)) || real[i - 1].tok == Tok::Op("]"))
                    && !(type_decl && i == 2)
                    && !(head && parens == 0);
                braces.push(literal);
            }
            Tok::Op("(") | Tok::Op("[") => parens += 1,
            Tok::Op(")") | Tok::Op("]") => parens -= 1,
            Tok::Op("}") => {
                braces.pop();
            }
            _ => {}
        }
        out.push_str(&line[t.span.start as usize..t.span.end as usize]);
    }
    let kept = real.iter().map(|t| (*t).clone()).collect();
    (out, kept)
}

/// Net change in bracket depth; `opens_first` counts closers at the line's start.
fn depth_change(toks: &[Token]) -> (i32, i32) {
    let mut leading_close = 0;
    let mut seen_other = false;
    let mut delta = 0;
    for t in toks {
        match t.tok {
            Tok::Op("{") | Tok::Op("(") | Tok::Op("[") => {
                delta += 1;
                seen_other = true;
            }
            Tok::Op("}") | Tok::Op(")") | Tok::Op("]") => {
                delta -= 1;
                if !seen_other {
                    leading_close += 1;
                }
            }
            _ => seen_other = true,
        }
    }
    (leading_close, delta)
}

fn ends_continued(toks: &[Token]) -> bool {
    match toks.last().map(|t| &t.tok) {
        Some(Tok::Kw(k)) => matches!(*k, "and" | "or" | "not"),
        Some(Tok::Op(o)) => !matches!(*o, ")" | "]" | "}" | "?" | "!" | "." | "..." | "(" | "[" | "{" | ","),
        _ => false,
    }
}

pub fn format(src: &str) -> Result<String, Vec<Diag>> {
    let mut diags = Vec::new();
    let original = lexer::lex(src, 0, &mut diags);
    if !diags.is_empty() {
        return Err(diags);
    }

    let mut out = String::with_capacity(src.len());
    let mut depth: i32 = 0;
    let mut continued = false;
    let mut blank_pending = false;
    let mut wrote_any = false;

    for (n, raw) in src.split('\n').enumerate() {
        let raw = raw.trim_end_matches('\r');
        // The `#!` line of a script is for the shell: keep it as it is.
        if n == 0 && raw.starts_with("#!") {
            out.push_str(raw.trim_end());
            out.push('\n');
            wrote_any = true;
            continue;
        }
        if raw.trim().is_empty() {
            blank_pending = wrote_any;
            continue;
        }
        if blank_pending {
            out.push('\n');
            blank_pending = false;
        }
        wrote_any = true;

        if let Some(comment) = is_comment_line(raw) {
            out.push_str(&"    ".repeat(depth.max(0) as usize));
            out.push_str(comment.trim_end());
            out.push('\n');
            continue;
        }

        let (code, comment) = split_comment(raw);
        let mut sub = Vec::new();
        let (text, toks) = format_code(code.trim_start(), &mut sub);
        if !sub.is_empty() {
            return Err(sub);
        }
        let (leading_close, delta) = depth_change(&toks);
        // A clause of a signature, not a `sure` statement in a body.
        let contract = depth == 0 && matches!(toks.first().map(|t| &t.tok), Some(Tok::Kw("need")) | Some(Tok::Kw("sure")));
        let indent = (depth - leading_close).max(0) + (continued || contract) as i32;
        out.push_str(&"    ".repeat(indent as usize));
        out.push_str(&text);
        if let Some(c) = comment {
            if !text.is_empty() {
                out.push(' ');
            }
            out.push_str(c.trim_end());
        }
        out.push('\n');
        depth += delta;
        continued = ends_continued(&toks);
    }

    // The safety net: only whitespace may ever change.
    let mut after_diags = Vec::new();
    let after = lexer::lex(&out, 0, &mut after_diags);
    let same = after_diags.is_empty()
        && original.len() == after.len()
        && original.iter().zip(&after).all(|(a, b)| a.tok == b.tok);
    if !same {
        let at = crate::diag::Span { start: 0, end: 0, line: 1, col: 1, file: 0 };
        return Err(vec![Diag::new("E0902", at, "formatter would have changed this program; file left untouched")]);
    }
    Ok(out)
}
