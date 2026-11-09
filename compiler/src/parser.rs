//! Hand-written recursive descent with precedence climbing for expressions.

use crate::ast::*;
use crate::diag::{Diag, Span};
use crate::lexer::{self, Tok, Token};
use std::cell::RefCell;
use std::collections::HashSet;

type PResult<T> = Result<T, ()>;

pub struct Parser<'a> {
    toks: Vec<Token>,
    pos: usize,
    diags: &'a mut Vec<Diag>,
    /// In `if`/`for`/`match` heads a bare `Name{` or `else` would be ambiguous.
    in_head: bool,
    in_contract: bool,
    /// The source text, for quoting contracts in their failure messages.
    src: String,
    tests: usize,
}

fn join(a: Span, b: Span) -> Span {
    Span { start: a.start, end: b.end, line: a.line, col: a.col, file: a.file }
}

fn is_type_name(s: &str) -> bool {
    s.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false)
}

impl<'a> Parser<'a> {
    pub fn new(toks: Vec<Token>, diags: &'a mut Vec<Diag>) -> Parser<'a> {
        Parser { toks, pos: 0, diags, in_head: false, in_contract: false, src: String::new(), tests: 0 }
    }

    pub fn with_source(mut self, src: &str) -> Parser<'a> {
        self.src = src.to_string();
        self
    }

    /// The source between a starting span and the last token consumed, on one line.
    fn text_since(&self, start: Span) -> String {
        let end = self.toks[self.pos.saturating_sub(1)].span.end as usize;
        let raw = self.src.get(start.start as usize..end).unwrap_or("");
        raw.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn parse_test(&mut self, index: usize) -> PResult<Func> {
        let span = self.bump().span;
        let Tok::Str(title) = self.peek().clone() else { return self.expected("the test's name in quotes") };
        self.bump();
        let body = self.parse_block()?;
        Ok(Func {
            name: format!("test {index}"),
            name_span: span,
            public: false,
            claw: false,
            tparams: Vec::new(),
            addressed: RefCell::new(HashSet::new()),
            params: Vec::new(),
            ret: None,
            contracts: Vec::new(),
            effects: Vec::new(),
            sig_end: span,
            test: Some(title),
            body,
        })
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn peek_at(&self, off: usize) -> &Tok {
        let i = (self.pos + off).min(self.toks.len() - 1);
        &self.toks[i].tok
    }

    fn span(&self) -> Span {
        self.toks[self.pos].span
    }

    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn at_op(&self, op: &str) -> bool {
        matches!(self.peek(), Tok::Op(o) if *o == op)
    }

    fn at_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Tok::Kw(k) if *k == kw)
    }

    fn eat_op(&mut self, op: &str) -> bool {
        if self.at_op(op) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn describe(&self) -> String {
        match self.peek() {
            Tok::Int(v) => format!("number `{v}`"),
            Tok::Float(v) => format!("number `{v}`"),
            Tok::Char(_) => "a character".to_string(),
            Tok::Str(_) => "a string".to_string(),
            Tok::Ident(s) => format!("name `{s}`"),
            Tok::Kw(k) => format!("keyword `{k}`"),
            Tok::Op(o) => format!("`{o}`"),
            Tok::Newline => "end of line".to_string(),
            Tok::Eof => "end of file".to_string(),
        }
    }

    fn error<T>(&mut self, code: &'static str, span: Span, msg: String) -> PResult<T> {
        self.diags.push(Diag::new(code, span, msg));
        Err(())
    }

    fn expected<T>(&mut self, what: &str) -> PResult<T> {
        let msg = format!("expected {what}, found {}", self.describe());
        let span = self.span();
        self.error("E0201", span, msg)
    }

    fn expect_op(&mut self, op: &str) -> PResult<Token> {
        if self.at_op(op) {
            Ok(self.bump())
        } else {
            self.expected(&format!("`{op}`"))
        }
    }

    fn expect_ident(&mut self, what: &str) -> PResult<(String, Span)> {
        if let Tok::Ident(s) = self.peek() {
            let s = s.clone();
            let span = self.bump().span;
            Ok((s, span))
        } else if let Tok::Kw(k) = self.peek() {
            let msg = format!("`{k}` is a keyword and cannot be used as {what}");
            let span = self.span();
            self.error("E0202", span, msg)
        } else {
            self.expected(what)
        }
    }

    fn skip_newlines(&mut self) {
        while *self.peek() == Tok::Newline {
            self.bump();
        }
    }

    pub fn parse_program(&mut self) -> Program {
        let mut p = Program::default();
        loop {
            self.skip_newlines();
            if *self.peek() == Tok::Eof {
                break;
            }
            let public = self.at_kw("pub");
            if public {
                self.bump();
            }
            let result: PResult<()> = match self.peek().clone() {
                Tok::Kw("fn") => self.parse_func(public, false).map(|f| p.funcs.push(f)),
                Tok::Kw("claw") => {
                    self.bump();
                    if self.at_kw("fn") {
                        self.parse_func(public, true).map(|f| p.funcs.push(f))
                    } else {
                        self.expected("`fn` after `claw`")
                    }
                }
                Tok::Kw("type") => self.parse_type_decl(public).map(|t| p.types.push(t)),
                Tok::Kw("const") => self.parse_const(public).map(|c| p.consts.push(c)),
                Tok::Kw("extern") => self.parse_extern(public).map(|e| p.externs.push(e)),
                Tok::Kw("use") if !public => self.parse_use().map(|u| p.uses.push(u)),
                Tok::Kw("test") if !public => {
                    self.tests += 1;
                    let index = self.tests;
                    self.parse_test(index).map(|f| p.funcs.push(f))
                }
                _ => self.expected("`fn`, `claw fn`, `type`, `const`, `extern`, `use` or `test`"),
            };
            if result.is_err() {
                self.bump();
                self.sync_top();
            }
        }
        p
    }

    /// Recover at the next declaration that starts a line.
    fn sync_top(&mut self) {
        loop {
            match self.peek() {
                Tok::Eof => return,
                Tok::Kw("fn") | Tok::Kw("type") | Tok::Kw("const") | Tok::Kw("claw") | Tok::Kw("extern") | Tok::Kw("use") | Tok::Kw("pub") | Tok::Kw("test")
                    if self.pos == 0 || self.toks[self.pos - 1].tok == Tok::Newline =>
                {
                    return
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    fn parse_use(&mut self) -> PResult<UseDecl> {
        let span = self.bump().span;
        let mut path = vec![self.expect_ident("a module name")?.0];
        while self.eat_op(".") {
            path.push(self.expect_ident("a module name")?.0);
        }
        Ok(UseDecl { path, span })
    }

    fn parse_extern(&mut self, public: bool) -> PResult<ExternDecl> {
        self.bump();
        let symbol = if let Tok::Str(s) = self.peek().clone() {
            self.bump();
            Some(s)
        } else {
            None
        };
        if !self.at_kw("fn") {
            return self.expected("`fn` after `extern`");
        }
        self.bump();
        let (name, span) = self.expect_ident("a function name")?;
        self.expect_op("(")?;
        let mut params = Vec::new();
        let mut variadic = false;
        while !self.at_op(")") {
            if self.eat_op("...") {
                variadic = true;
                break;
            }
            let (pname, pspan) = self.expect_ident("a parameter name")?;
            let ty = self.parse_type()?;
            params.push(Param { own: false, name: pname, span: pspan, ty });
            if !self.eat_op(",") {
                break;
            }
        }
        self.expect_op(")")?;
        let ret = if matches!(self.peek(), Tok::Ident(_)) || self.at_op("*") { Some(self.parse_type()?) } else { None };
        Ok(ExternDecl { symbol, name, span, public, params, variadic, ret })
    }

    fn parse_tparams(&mut self) -> PResult<Vec<(String, Span)>> {
        let mut out = Vec::new();
        if self.eat_op("[") {
            while !self.at_op("]") {
                out.push(self.expect_ident("a type parameter")?);
                if !self.eat_op(",") {
                    break;
                }
            }
            self.expect_op("]")?;
        }
        Ok(out)
    }

    fn parse_const(&mut self, public: bool) -> PResult<ConstDecl> {
        self.bump();
        let (name, span) = self.expect_ident("a constant name")?;
        let ty = if self.at_op("=") { None } else { Some(self.parse_type()?) };
        self.expect_op("=")?;
        let start = self.span();
        let value = self.parse_expr(0)?;
        let text = self.text_since(start);
        Ok(ConstDecl { name, span, public, ty, value, computed: false, text })
    }

    fn parse_fields(&mut self) -> PResult<Vec<FieldDecl>> {
        self.expect_op("{")?;
        let mut fields = Vec::new();
        loop {
            self.skip_newlines();
            if self.at_op("}") {
                break;
            }
            let (name, span) = self.expect_ident("a field name")?;
            let ty = self.parse_type()?;
            fields.push(FieldDecl { name, span, ty });
            self.skip_newlines();
            if !self.eat_op(",") && !self.at_op("}") && !matches!(self.peek(), Tok::Ident(_)) {
                return self.expected("`,` or `}`");
            }
        }
        self.bump();
        Ok(fields)
    }

    fn parse_type_decl(&mut self, public: bool) -> PResult<TypeDecl> {
        self.bump();
        let (name, span) = self.expect_ident("a type name")?;
        let tparams = self.parse_tparams()?;
        if self.at_op("{") {
            let fields = self.parse_fields()?;
            return Ok(TypeDecl { name, span, public, tparams, body: TypeBody::Struct(fields) });
        }
        self.expect_op("=")?;
        let mut variants = Vec::new();
        loop {
            let (vname, vspan) = self.expect_ident("a variant name")?;
            let fields = if self.at_op("{") { self.parse_fields()? } else { Vec::new() };
            variants.push(VariantDecl { name: vname, span: vspan, fields });
            if !self.eat_op("|") {
                break;
            }
        }
        Ok(TypeDecl { name, span, public, tparams, body: TypeBody::Variants(variants) })
    }

    fn parse_type_base(&mut self) -> PResult<TypeExpr> {
        let span = self.span();
        if self.at_op("?") {
            self.bump();
            let inner = self.parse_type_base()?;
            if matches!(inner.kind, TypeKind::Option(_)) {
                return self.error("E0134", span, "`??T` is not allowed".to_string());
            }
            return Ok(TypeExpr { kind: TypeKind::Option(Box::new(inner)), span });
        }
        if self.at_op("[") {
            self.bump();
            let size = match self.peek().clone() {
                Tok::Int(v) => {
                    self.bump();
                    Size::Lit(v)
                }
                Tok::Ident(n) => {
                    self.bump();
                    Size::Name(n)
                }
                Tok::Op("]") => {
                    self.bump();
                    let elem = self.parse_type_base()?;
                    return Ok(TypeExpr { kind: TypeKind::Slice(Box::new(elem)), span });
                }
                _ => return self.expected("an array size"),
            };
            self.expect_op("]")?;
            let elem = self.parse_type_base()?;
            return Ok(TypeExpr { kind: TypeKind::Array(size, Box::new(elem)), span });
        }
        if self.eat_op("*") {
            let inner = self.parse_type_base()?;
            return Ok(TypeExpr { kind: TypeKind::Pointer(Box::new(inner)), span });
        }
        if self.eat_op("&") {
            let inner = self.parse_type_base()?;
            return Ok(TypeExpr { kind: TypeKind::Ref(Box::new(inner)), span });
        }
        if self.at_kw("fn") {
            return self.parse_fn_type(span);
        }
        let (first, _) = self.expect_ident("a type")?;
        let (module, name) = if self.at_op(".") {
            self.bump();
            (Some(first), self.expect_ident("a type")?.0)
        } else {
            (None, first)
        };
        let targs = if self.at_op("[") { self.parse_targs()? } else { Vec::new() };
        Ok(TypeExpr { kind: TypeKind::Name { module, name, targs }, span })
    }

    fn at_type_start(&self) -> bool {
        matches!(self.peek(), Tok::Ident(_)) || self.at_op("[") || self.at_op("?") || self.at_op("*") || self.at_op("!") || self.at_op("&") || self.at_kw("fn")
    }

    fn parse_effects(&mut self) -> PResult<Vec<String>> {
        let mut effects = Vec::new();
        while self.at_op("+") {
            self.bump();
            let (e, span) = self.expect_ident("an effect name")?;
            if e != "mem" && e != "io" {
                return self.error("E0132", span, format!("unknown effect `{e}`; use `+mem` or `+io`"));
            }
            effects.push(e);
        }
        Ok(effects)
    }

    /// `fn(int, own str) int +mem`, at the word `fn`.
    fn parse_fn_type(&mut self, span: Span) -> PResult<TypeExpr> {
        self.bump();
        self.expect_op("(")?;
        let mut params = Vec::new();
        while !self.at_op(")") {
            let own = self.at_kw("own");
            if own {
                self.bump();
            }
            params.push((own, self.parse_type()?));
            if !self.eat_op(",") {
                break;
            }
        }
        self.expect_op(")")?;
        let ret = if self.at_type_start() { Some(Box::new(self.parse_type()?)) } else { None };
        let effects = self.parse_effects()?;
        Ok(TypeExpr { kind: TypeKind::Fn { params, ret, effects }, span })
    }

    fn parse_targs(&mut self) -> PResult<Vec<TypeExpr>> {
        self.expect_op("[")?;
        let mut out = Vec::new();
        while !self.at_op("]") {
            out.push(self.parse_type()?);
            if !self.eat_op(",") {
                break;
            }
        }
        self.expect_op("]")?;
        Ok(out)
    }

    /// At `[`: does this bracket hold type arguments, as in `max[int](1, 2)`?
    fn looks_like_targs(&self, name: &str) -> bool {
        // `table[i](x)` indexes; only a type can start `max[int](x)`.
        let is_type = match self.peek_at(1) {
            Tok::Ident(t) => is_type_name(t) || matches!(t.as_str(), "int" | "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "f32" | "f64" | "bool" | "str") || matches!(self.peek_at(2), Tok::Op(".")),
            Tok::Op("[") | Tok::Op("*") | Tok::Op("?") | Tok::Op("&") | Tok::Kw("fn") => true,
            _ => false,
        };
        if !is_type {
            return false;
        }
        let mut i = 1;
        let mut depth = 1;
        while depth > 0 {
            match self.peek_at(i) {
                Tok::Op("[") => depth += 1,
                Tok::Op("]") => depth -= 1,
                Tok::Eof | Tok::Newline => return false,
                _ => {}
            }
            i += 1;
        }
        match self.peek_at(i) {
            Tok::Op("(") => true,
            Tok::Op("{") => is_type_name(name) && !self.in_head,
            _ => false,
        }
    }

    fn parse_type(&mut self) -> PResult<TypeExpr> {
        let span = self.span();
        if self.at_op("!") {
            self.bump();
            let e = self.parse_type_base()?;
            return Ok(TypeExpr { kind: TypeKind::Result(None, Box::new(e)), span });
        }
        let t = self.parse_type_base()?;
        if self.at_op("!") {
            self.bump();
            let e = self.parse_type_base()?;
            return Ok(TypeExpr { kind: TypeKind::Result(Some(Box::new(t)), Box::new(e)), span });
        }
        Ok(t)
    }

    fn parse_func(&mut self, public: bool, claw: bool) -> PResult<Func> {
        self.bump();
        let (name, name_span) = self.expect_ident("a function name")?;
        let tparams = self.parse_tparams()?;
        self.expect_op("(")?;
        let mut params = Vec::new();
        while !self.at_op(")") {
            let own = self.at_kw("own");
            if own {
                self.bump();
            }
            let (pname, span) = self.expect_ident("a parameter name")?;
            let ty = self.parse_type()?;
            params.push(Param { own, name: pname, span, ty });
            if !self.eat_op(",") {
                break;
            }
        }
        self.expect_op(")")?;
        let ret = if self.at_type_start() { Some(self.parse_type()?) } else { None };
        let effects = self.parse_effects()?;
        let last = self.toks[self.pos.saturating_sub(1)].span;
        let sig_end = Span { start: last.end, end: last.end, line: last.line, col: last.col, file: last.file };

        let mut contracts = Vec::new();
        self.skip_newlines();
        while self.at_kw("need") || self.at_kw("sure") {
            let sure = self.at_kw("sure");
            self.bump();
            let start = self.span();
            self.in_contract = true;
            // `need x == Empty {`: the brace opens the body, not a struct.
            let head = std::mem::replace(&mut self.in_head, true);
            let e = self.parse_expr(0);
            self.in_head = head;
            self.in_contract = false;
            let text = self.text_since(start);
            contracts.push(Contract { sure, e: e?, text });
            self.skip_newlines();
        }
        let body = self.parse_block()?;
        Ok(Func { name, name_span, public, claw, tparams, addressed: RefCell::new(HashSet::new()), params, ret, contracts, effects, sig_end, test: None, body })
    }

    fn parse_block(&mut self) -> PResult<Block> {
        let was_head = std::mem::replace(&mut self.in_head, false);
        let result = self.parse_block_inner();
        self.in_head = was_head;
        result
    }

    fn parse_block_inner(&mut self) -> PResult<Block> {
        self.expect_op("{")?;
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if self.at_op("}") {
                break;
            }
            if *self.peek() == Tok::Eof {
                let span = self.span();
                return self.error("E0204", span, "block is not closed; add `}`".to_string());
            }
            let stmt = self.parse_stmt()?;
            stmts.push(stmt);
            if !self.at_op("}") && *self.peek() != Tok::Newline {
                let msg = format!("one statement per line; found {} after the statement", self.describe());
                let span = self.span();
                return self.error("E0205", span, msg);
            }
        }
        let end = self.bump().span;
        Ok(Block { stmts, end })
    }

    /// `x [4]int = ...` and `x ?int = ...` declare; `x[4] = ...` assigns.
    fn is_typed_let(&self) -> bool {
        match self.peek_at(1) {
            Tok::Ident(_) | Tok::Kw("fn") => true,
            Tok::Op("?") => matches!(self.peek_at(2), Tok::Ident(_) | Tok::Op("[") | Tok::Op("?") | Tok::Op("*") | Tok::Kw("fn")),
            // `x * y` alone does nothing, so at the start of a line this is a pointer type.
            Tok::Op("*") => true,
            Tok::Op("[") => {
                // Skip every `[...]` group; a type follows only in a declaration.
                let mut i = 1;
                while matches!(self.peek_at(i), Tok::Op("[")) {
                    let mut depth = 1;
                    i += 1;
                    while depth > 0 {
                        match self.peek_at(i) {
                            Tok::Op("[") => depth += 1,
                            Tok::Op("]") => depth -= 1,
                            Tok::Eof | Tok::Newline => return false,
                            _ => {}
                        }
                        i += 1;
                    }
                }
                matches!(self.peek_at(i), Tok::Ident(_) | Tok::Op("?") | Tok::Op("*") | Tok::Kw("fn"))
            }
            _ => false,
        }
    }

    fn parse_stmt(&mut self) -> PResult<Stmt> {
        let start = self.span();
        match self.peek().clone() {
            Tok::Kw("mut") => {
                self.bump();
                self.parse_let(start, true)
            }
            Tok::Kw("if") => self.parse_if(),
            Tok::Kw("for") => self.parse_for(),
            Tok::Kw("match") => self.parse_match(),
            Tok::Kw("ret") => {
                self.bump();
                let value = if *self.peek() == Tok::Newline || self.at_op("}") {
                    None
                } else {
                    Some(self.parse_expr(0)?)
                };
                Ok(Stmt { kind: StmtKind::Ret(value), span: start })
            }
            Tok::Kw("fail") => {
                self.bump();
                let e = self.parse_expr(0)?;
                Ok(Stmt { kind: StmtKind::Fail(e), span: start })
            }
            Tok::Kw("break") => {
                self.bump();
                Ok(Stmt { kind: StmtKind::Break, span: start })
            }
            Tok::Kw("continue") => {
                self.bump();
                Ok(Stmt { kind: StmtKind::Continue, span: start })
            }
            Tok::Kw("else") => self.error(
                "E0203",
                start,
                "`else` must be on the same line as the closing `}`".to_string(),
            ),
            Tok::Kw("defer") => {
                self.bump();
                let body = if self.at_op("{") {
                    self.parse_block()?
                } else {
                    let stmt = self.parse_stmt()?;
                    Block { stmts: vec![stmt], end: start }
                };
                Ok(Stmt { kind: StmtKind::Defer(body), span: start })
            }
            Tok::Kw("sure") => {
                self.bump();
                let at = self.span();
                let e = self.parse_expr(0)?;
                let text = self.text_since(at);
                Ok(Stmt { kind: StmtKind::Sure(e, text), span: start })
            }
            Tok::Kw("need") => self.error("E0168", start, "`need` belongs in a signature; inside a body write `sure`".to_string()),
            Tok::Ident(name) => {
                if matches!(self.peek_at(1), Tok::Op(":=")) || self.is_typed_let() {
                    return self.parse_let(start, false);
                }
                if name == "_" && matches!(self.peek_at(1), Tok::Op("=")) {
                    self.bump();
                    self.bump();
                    let e = self.parse_expr(0)?;
                    return Ok(Stmt { kind: StmtKind::Discard(e), span: start });
                }
                self.parse_expr_or_assign(start)
            }
            _ => self.parse_expr_or_assign(start),
        }
    }

    fn parse_expr_or_assign(&mut self, start: Span) -> PResult<Stmt> {
        let e = self.parse_expr(0)?;
        if let Tok::Op(op) = self.peek().clone() {
            if let Some(op) = assign_op(op) {
                self.bump();
                let value = self.parse_expr(0)?;
                return Ok(Stmt { kind: StmtKind::Assign { target: e, op, value }, span: start });
            }
        }
        Ok(Stmt { kind: StmtKind::Expr(e), span: start })
    }

    fn parse_let(&mut self, start: Span, mutable: bool) -> PResult<Stmt> {
        let (name, name_span) = self.expect_ident("a variable name")?;
        let ty = if self.at_op(":=") {
            self.bump();
            None
        } else {
            let ty = self.parse_type()?;
            self.expect_op("=")?;
            Some(ty)
        };
        let init = self.parse_expr(0)?;
        Ok(Stmt { kind: StmtKind::Let { mutable, name, name_span, ty, init }, span: start })
    }

    fn parse_head(&mut self) -> PResult<Expr> {
        let was = std::mem::replace(&mut self.in_head, true);
        let e = self.parse_expr(0);
        self.in_head = was;
        e
    }

    fn parse_if(&mut self) -> PResult<Stmt> {
        let start = self.bump().span;
        let cond = self.parse_head()?;
        let then = self.parse_block()?;
        let els = if self.at_kw("else") {
            self.bump();
            if self.at_kw("if") {
                Some(Else::If(Box::new(self.parse_if()?)))
            } else {
                Some(Else::Block(self.parse_block()?))
            }
        } else {
            None
        };
        Ok(Stmt { kind: StmtKind::If { cond, then, els }, span: start })
    }

    fn parse_for(&mut self) -> PResult<Stmt> {
        let start = self.bump().span;
        if self.at_op("{") {
            let body = self.parse_block()?;
            return Ok(Stmt { kind: StmtKind::ForEver { body }, span: start });
        }
        let binds_two = matches!(self.peek(), Tok::Ident(_))
            && matches!(self.peek_at(1), Tok::Op(","))
            && matches!(self.peek_at(2), Tok::Ident(_))
            && matches!(self.peek_at(3), Tok::Kw("in"));
        let binds_one = matches!(self.peek(), Tok::Ident(_)) && matches!(self.peek_at(1), Tok::Kw("in"));
        if binds_one || binds_two {
            let (var, var_span) = self.expect_ident("a loop variable")?;
            let index = if binds_two {
                self.bump();
                Some(self.expect_ident("an index variable")?)
            } else {
                None
            };
            self.bump();
            let from = self.parse_head()?;
            if self.at_op("..") {
                if binds_two {
                    let span = self.span();
                    return self.error("E0135", span, "a range gives one value per step; use one loop variable".to_string());
                }
                self.bump();
                let to = self.parse_head()?;
                let body = self.parse_block()?;
                return Ok(Stmt { kind: StmtKind::ForRange { var, var_span, start: from, end: to, body }, span: start });
            }
            let body = self.parse_block()?;
            return Ok(Stmt { kind: StmtKind::ForIn { elem: var, elem_span: var_span, index, subject: from, body }, span: start });
        }
        let cond = self.parse_head()?;
        let body = self.parse_block()?;
        Ok(Stmt { kind: StmtKind::ForCond { cond, body }, span: start })
    }

    fn parse_pattern(&mut self) -> PResult<Pat> {
        match self.peek().clone() {
            Tok::Ident(n) if n == "_" => {
                self.bump();
                Ok(Pat::Wild)
            }
            Tok::Ident(n) => {
                self.bump();
                let mut binds = Vec::new();
                if self.eat_op("{") {
                    while !self.at_op("}") {
                        binds.push(self.expect_ident("a name to bind")?);
                        if !self.eat_op(",") {
                            break;
                        }
                    }
                    self.expect_op("}")?;
                }
                Ok(Pat::Variant { name: n, binds })
            }
            Tok::Int(v) => {
                self.bump();
                Ok(Pat::Int(v))
            }
            Tok::Char(c) => {
                self.bump();
                Ok(Pat::Int(c as i128))
            }
            Tok::Op("-") => {
                self.bump();
                if let Tok::Int(v) = self.peek().clone() {
                    self.bump();
                    Ok(Pat::Int(-v))
                } else {
                    self.expected("a number")
                }
            }
            Tok::Kw("true") => {
                self.bump();
                Ok(Pat::Bool(true))
            }
            Tok::Kw("false") => {
                self.bump();
                Ok(Pat::Bool(false))
            }
            _ => self.expected("a pattern"),
        }
    }

    fn parse_match(&mut self) -> PResult<Stmt> {
        let start = self.bump().span;
        let subject = self.parse_head()?;
        self.expect_op("{")?;
        let was_head = std::mem::replace(&mut self.in_head, false);
        let mut arms = Vec::new();
        loop {
            self.skip_newlines();
            if self.at_op("}") {
                break;
            }
            let span = self.span();
            let pat = self.parse_pattern()?;
            self.expect_op(":")?;
            let body = if self.at_op("{") {
                self.parse_block()?
            } else {
                let stmt = self.parse_stmt()?;
                Block { stmts: vec![stmt], end: span }
            };
            arms.push(Arm { pat, span, body });
            if !self.at_op("}") && *self.peek() != Tok::Newline {
                return self.expected("a new line after the arm");
            }
        }
        self.in_head = was_head;
        self.bump();
        Ok(Stmt { kind: StmtKind::Match { subject, arms }, span: start })
    }

    fn peek_binop(&self) -> Option<(BinOp, u8)> {
        Some(match self.peek() {
            Tok::Kw("or") => (BinOp::Or, 1),
            Tok::Kw("and") => (BinOp::And, 2),
            Tok::Op(o) => match *o {
                "==" => (BinOp::Eq, 3),
                "!=" => (BinOp::Ne, 3),
                "<" => (BinOp::Lt, 3),
                "<=" => (BinOp::Le, 3),
                ">" => (BinOp::Gt, 3),
                ">=" => (BinOp::Ge, 3),
                "+" => (BinOp::Add, 4),
                "-" => (BinOp::Sub, 4),
                "|" => (BinOp::BitOr, 4),
                "^" => (BinOp::BitXor, 4),
                "*" => (BinOp::Mul, 5),
                "/" => (BinOp::Div, 5),
                "%" => (BinOp::Rem, 5),
                "<<" => (BinOp::Shl, 5),
                ">>" => (BinOp::Shr, 5),
                "&" => (BinOp::BitAnd, 5),
                _ => return None,
            },
            _ => return None,
        })
    }

    fn parse_expr(&mut self, min_prec: u8) -> PResult<Expr> {
        let mut left = self.parse_unary()?;
        while let Some((op, prec)) = self.peek_binop() {
            if prec < min_prec {
                break;
            }
            self.bump();
            let right = self.parse_expr(prec + 1)?;
            let span = join(left.span, right.span);
            left = Expr::new(ExprKind::Binary { op, l: Box::new(left), r: Box::new(right) }, span);
        }
        if min_prec == 0 && self.at_kw("else") && !self.in_head {
            return self.parse_else(left);
        }
        Ok(left)
    }

    fn parse_else(&mut self, value: Expr) -> PResult<Expr> {
        let start = value.span;
        self.bump();
        let binder = match self.peek() {
            Tok::Ident(n) if !is_type_name(n) && matches!(self.peek_at(1), Tok::Op("{")) => Some(self.expect_ident("a name")?),
            _ => None,
        };
        let handler = if binder.is_some() || self.at_op("{") {
            ElseKind::Block(binder, self.parse_block()?)
        } else {
            ElseKind::Value(Box::new(self.parse_expr(1)?))
        };
        let span = join(start, self.toks[self.pos.saturating_sub(1)].span);
        Ok(Expr::new(ExprKind::Else { e: Box::new(value), handler }, span))
    }

    fn parse_unary(&mut self) -> PResult<Expr> {
        let start = self.span();
        if self.at_kw("not") {
            self.bump();
            // Like Python: `not a == b` means `not (a == b)`.
            let e = self.parse_expr(3)?;
            let span = join(start, e.span);
            return Ok(Expr::new(ExprKind::Unary { op: UnOp::Not, e: Box::new(e) }, span));
        }
        if self.at_op("-") {
            self.bump();
            match self.peek().clone() {
                Tok::Int(v) => {
                    let tok_span = self.bump().span;
                    return self.parse_postfix(Expr::new(ExprKind::Int(-v), join(start, tok_span)));
                }
                Tok::Float(v) => {
                    let tok_span = self.bump().span;
                    return self.parse_postfix(Expr::new(ExprKind::Float(-v), join(start, tok_span)));
                }
                _ => {}
            }
            let e = self.parse_unary()?;
            let span = join(start, e.span);
            return Ok(Expr::new(ExprKind::Unary { op: UnOp::Neg, e: Box::new(e) }, span));
        }
        if self.at_op("~") {
            self.bump();
            let e = self.parse_unary()?;
            let span = join(start, e.span);
            return Ok(Expr::new(ExprKind::Unary { op: UnOp::BitNot, e: Box::new(e) }, span));
        }
        if self.at_op("&") {
            self.bump();
            let e = self.parse_unary()?;
            let span = join(start, e.span);
            return Ok(Expr::new(ExprKind::AddrOf(Box::new(e)), span));
        }
        if self.at_op("*") {
            self.bump();
            let e = self.parse_unary()?;
            let span = join(start, e.span);
            return Ok(Expr::new(ExprKind::Deref(Box::new(e)), span));
        }
        let e = self.parse_primary()?;
        self.parse_postfix(e)
    }

    fn parse_postfix(&mut self, mut e: Expr) -> PResult<Expr> {
        loop {
            if self.at_op(".") {
                self.bump();
                let (name, name_span) = self.expect_ident("a field name")?;
                let targs = if self.at_op("[") && self.looks_like_targs(&name) { self.parse_targs()? } else { Vec::new() };
                let module = if let ExprKind::Var(m) = &e.kind { Some(m.clone()) } else { None };
                if self.at_op("(") {
                    self.bump();
                    let (args, close) = self.parse_args(")")?;
                    let span = join(e.span, close);
                    let field = Expr::new(ExprKind::Field { e: Box::new(e), name: name.clone(), name_span }, span);
                    e = match module {
                        Some(module) => Expr::new(ExprKind::Call { module: Some(module), name, name_span, targs, args, via: Some(Box::new(field)) }, span),
                        None => Expr::new(ExprKind::CallValue { f: Box::new(field), args }, span),
                    };
                } else if self.at_op("{") && module.is_some() && is_type_name(&name) && !self.in_head {
                    self.bump();
                    let (args, close) = self.parse_args("}")?;
                    let span = join(e.span, close);
                    e = Expr::new(ExprKind::StructLit { module, name, name_span, targs, args }, span);
                } else {
                    let span = join(e.span, name_span);
                    e = Expr::new(ExprKind::Field { e: Box::new(e), name, name_span }, span);
                }
            } else if self.at_op("[") {
                self.bump();
                let was = std::mem::replace(&mut self.in_head, false);
                let result = self.parse_index_or_slice(e);
                self.in_head = was;
                e = result?;
            } else if self.at_op("(") {
                self.bump();
                let (args, close) = self.parse_args(")")?;
                let span = join(e.span, close);
                e = Expr::new(ExprKind::CallValue { f: Box::new(e), args }, span);
            } else if self.at_op("?") {
                let close = self.bump();
                let span = join(e.span, close.span);
                e = Expr::new(ExprKind::Try(Box::new(e)), span);
            } else {
                return Ok(e);
            }
        }
    }

    fn parse_index_or_slice(&mut self, e: Expr) -> PResult<Expr> {
        let lo = if self.at_op("..") { None } else { Some(Box::new(self.parse_expr(0)?)) };
        if self.eat_op("..") {
            let hi = if self.at_op("]") { None } else { Some(Box::new(self.parse_expr(0)?)) };
            let close = self.expect_op("]")?;
            let span = join(e.span, close.span);
            return Ok(Expr::new(ExprKind::Slice { e: Box::new(e), lo, hi }, span));
        }
        let close = self.expect_op("]")?;
        let span = join(e.span, close.span);
        Ok(Expr::new(ExprKind::Index { e: Box::new(e), i: lo.expect("an index was parsed") }, span))
    }

    fn parse_args(&mut self, close: &str) -> PResult<(Vec<Expr>, Span)> {
        let was = std::mem::replace(&mut self.in_head, false);
        let mut args = Vec::new();
        let result = loop {
            self.skip_newlines();
            if self.at_op(close) {
                break self.expect_op(close);
            }
            match self.parse_expr(0) {
                Ok(e) => args.push(e),
                Err(()) => break Err(()),
            }
            self.skip_newlines();
            if !self.eat_op(",") {
                break self.expect_op(close);
            }
        };
        self.in_head = was;
        Ok((args, result?.span))
    }

    fn parse_interp(&mut self, text: &str, span: Span) -> PResult<Expr> {
        let mut parts = Vec::new();
        let mut lit = String::new();
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '{' if chars.get(i + 1) == Some(&'{') => {
                    lit.push('{');
                    i += 2;
                }
                '}' if chars.get(i + 1) == Some(&'}') => {
                    lit.push('}');
                    i += 2;
                }
                '{' => {
                    let mut j = i + 1;
                    let mut depth = 1;
                    while j < chars.len() {
                        match chars[j] {
                            '{' => depth += 1,
                            '}' => depth -= 1,
                            _ => {}
                        }
                        if depth == 0 {
                            break;
                        }
                        j += 1;
                    }
                    if depth != 0 {
                        return self.error("E0133", span, "`{` in this string is not closed; write `{{` for a literal brace".to_string());
                    }
                    let inner: String = chars[i + 1..j].iter().collect();
                    if !lit.is_empty() {
                        parts.push(Part::Lit(std::mem::take(&mut lit)));
                    }
                    parts.push(Part::Expr(self.parse_embedded(&inner, span)?));
                    i = j + 1;
                }
                c => {
                    lit.push(c);
                    i += 1;
                }
            }
        }
        if !lit.is_empty() {
            parts.push(Part::Lit(lit));
        }
        Ok(Expr::new(ExprKind::Interp(parts), span))
    }

    /// Parses `{expr}` from a string; errors point at the string itself.
    fn parse_embedded(&mut self, src: &str, span: Span) -> PResult<Expr> {
        let mut sub_diags = Vec::new();
        let mut toks = lexer::lex(src, span.file, &mut sub_diags);
        for t in &mut toks {
            t.span = span;
        }
        let mut p = Parser::new(toks, &mut sub_diags);
        let e = p.parse_expr(0);
        let leftover = e.is_ok() && !matches!(p.peek(), Tok::Eof | Tok::Newline);
        drop(p);
        if leftover {
            sub_diags.push(Diag::new("E0133", span, format!("cannot read `{src}` inside this string")));
        }
        for d in &mut sub_diags {
            d.span = span;
        }
        let failed = !sub_diags.is_empty();
        self.diags.append(&mut sub_diags);
        if failed {
            return Err(());
        }
        e
    }

    fn parse_primary(&mut self) -> PResult<Expr> {
        let span = self.span();
        match self.peek().clone() {
            Tok::Int(v) => {
                self.bump();
                Ok(Expr::new(ExprKind::Int(v), span))
            }
            Tok::Float(v) => {
                self.bump();
                Ok(Expr::new(ExprKind::Float(v), span))
            }
            Tok::Char(c) => {
                self.bump();
                Ok(Expr::new(ExprKind::Char(c), span))
            }
            Tok::Str(s) => {
                self.bump();
                if s.contains('{') || s.contains("}}") {
                    self.parse_interp(&s, span)
                } else {
                    Ok(Expr::new(ExprKind::Str(s), span))
                }
            }
            Tok::Kw("true") => {
                self.bump();
                Ok(Expr::new(ExprKind::Bool(true), span))
            }
            Tok::Kw("false") => {
                self.bump();
                Ok(Expr::new(ExprKind::Bool(false), span))
            }
            Tok::Kw("none") => {
                self.bump();
                Ok(Expr::new(ExprKind::None, span))
            }
            Tok::Kw("ret") if self.in_contract => {
                self.bump();
                Ok(Expr::new(ExprKind::RetVal, span))
            }
            Tok::Ident(name) if self.in_contract && name == "old" && matches!(self.peek_at(1), Tok::Op("(")) => {
                self.bump();
                self.bump();
                let inner = self.parse_expr(0)?;
                self.expect_op(")")?;
                Ok(Expr::new(ExprKind::Old(Box::new(inner)), span))
            }
            Tok::Ident(name) => {
                self.bump();
                let targs = if self.at_op("[") && self.looks_like_targs(&name) { self.parse_targs()? } else { Vec::new() };
                if self.at_op("(") {
                    self.bump();
                    let (args, close) = self.parse_args(")")?;
                    Ok(Expr::new(ExprKind::Call { module: None, name, name_span: span, targs, args, via: None }, join(span, close)))
                } else if self.at_op("{") && is_type_name(&name) && !self.in_head {
                    self.bump();
                    let (args, close) = self.parse_args("}")?;
                    Ok(Expr::new(ExprKind::StructLit { module: None, name, name_span: span, targs, args }, join(span, close)))
                } else {
                    Ok(Expr::new(ExprKind::Var(name), span))
                }
            }
            Tok::Op("(") => {
                self.bump();
                let was = std::mem::replace(&mut self.in_head, false);
                let e = self.parse_expr(0);
                self.in_head = was;
                let e = e?;
                self.expect_op(")")?;
                Ok(e)
            }
            Tok::Op("[") => {
                self.bump();
                let was = std::mem::replace(&mut self.in_head, false);
                let result = self.parse_array(span);
                self.in_head = was;
                result
            }
            _ => self.expected("an expression"),
        }
    }

    fn parse_array(&mut self, span: Span) -> PResult<Expr> {
        self.skip_newlines();
        let mut items = Vec::new();
        loop {
            self.skip_newlines();
            if self.at_op("]") {
                break;
            }
            let e = self.parse_expr(0)?;
            if self.eat_op("...") {
                if !items.is_empty() {
                    let s = self.span();
                    return self.error("E0136", s, "`x...` fills a whole array, so it must be the only item".to_string());
                }
                self.skip_newlines();
                let close = self.expect_op("]")?;
                return Ok(Expr::new(ExprKind::ArrayFill(Box::new(e)), join(span, close.span)));
            }
            items.push(e);
            self.skip_newlines();
            if !self.eat_op(",") {
                break;
            }
        }
        let close = self.expect_op("]")?;
        Ok(Expr::new(ExprKind::ArrayLit(items), join(span, close.span)))
    }
}

/// `Some(None)` is plain `=`, `Some(Some(op))` is a compound assignment.
fn assign_op(op: &str) -> Option<Option<BinOp>> {
    Some(match op {
        "=" => None,
        "+=" => Some(BinOp::Add),
        "-=" => Some(BinOp::Sub),
        "*=" => Some(BinOp::Mul),
        "/=" => Some(BinOp::Div),
        "%=" => Some(BinOp::Rem),
        "&=" => Some(BinOp::BitAnd),
        "|=" => Some(BinOp::BitOr),
        "^=" => Some(BinOp::BitXor),
        "<<=" => Some(BinOp::Shl),
        ">>=" => Some(BinOp::Shr),
        _ => return None,
    })
}
