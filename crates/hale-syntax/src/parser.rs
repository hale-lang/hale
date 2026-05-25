//! Recursive-descent parser for Hale.
//!
//! Produces an [`ast::Program`] from a token stream. Hand-written
//! per the team's choice (better diagnostic quality and direct
//! control over edge cases like mode-keywords-post-dot).
//!
//! Coverage: targets the v0 example ladder. Not all grammar
//! productions are implemented yet; missing ones produce a clear
//! "not yet implemented" error.

use crate::ast::*;
use crate::error::Diag;
use crate::lexer::{FStringPart, Token, TokenKind};
use crate::span::Span;

/// Parse a token stream into a Program. Source string is needed
/// for error rendering and for slicing literal text.
pub fn parse(tokens: Vec<Token>, _source: &str) -> Result<Program, Vec<Diag>> {
    let mut p = Parser::new(tokens);
    let prog = p.parse_program();
    match prog {
        Ok(prog) if p.diags.is_empty() => Ok(prog),
        Ok(_) => Err(std::mem::take(&mut p.diags)),
        Err(d) => {
            p.diags.push(d);
            Err(std::mem::take(&mut p.diags))
        }
    }
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    diags: Vec<Diag>,
    /// v1.x-FORM-1: tracks whether we're inside the body of a
    /// fallible fn. When true, leading-statement `fail` Ident is
    /// recognized as the fail-keyword. Outside that context,
    /// `fail` lexes and parses as an ordinary identifier (so
    /// `let fail = 0;` outside a fallible body stays admissible).
    in_fallible_body: bool,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Parser {
            tokens,
            pos: 0,
            diags: Vec::new(),
            in_fallible_body: false,
        }
    }

    // === helpers ==========================================

    fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn peek_token(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn peek_at(&self, n: usize) -> &TokenKind {
        let i = self.pos + n;
        if i < self.tokens.len() {
            &self.tokens[i].kind
        } else {
            &TokenKind::Eof
        }
    }

    fn bump(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if !matches!(t.kind, TokenKind::Eof) {
            self.pos += 1;
        }
        t
    }

    fn at(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(self.peek()) == std::mem::discriminant(kind)
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: TokenKind, what: &str) -> Result<Token, Diag> {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(&kind) {
            Ok(self.bump())
        } else {
            let span = self.peek_token().span;
            Err(Diag::parse(
                span,
                format!("expected {}, got {:?}", what, self.peek()),
            ))
        }
    }

    /// m66: close a generic-args list. Accepts `>` directly, OR
    /// splits a `>>` token in-place (consumes one `>`, rewrites
    /// the current token to a single `>` so the enclosing parser
    /// frame can close the next generic args list with the second
    /// `>`). This lifts the parser-only nested-generics ambiguity
    /// `Box<Box<Int>>` without changing the lexer (which still
    /// emits `Shr` for `>>` because shift-right needs it in
    /// expression position). Returns a Token whose span is the
    /// first half of the `>>` byte range when split.
    fn expect_gt_or_split_shr(&mut self) -> Result<Token, Diag> {
        if matches!(self.peek(), TokenKind::Gt) {
            return Ok(self.bump());
        }
        if matches!(self.peek(), TokenKind::Shr) {
            let span = self.peek_token().span;
            let start = span.start.0 as usize;
            let end = span.end.0 as usize;
            let mid = start + 1;
            // Synthesize the inner closer's `>` token (first half
            // of `>>`) and rewrite the current slot to the outer
            // closer's `>` (second half).
            let first_gt = Token::new(
                TokenKind::Gt,
                crate::span::Span::new(start, mid),
            );
            self.tokens[self.pos] = Token::new(
                TokenKind::Gt,
                crate::span::Span::new(mid, end),
            );
            return Ok(first_gt);
        }
        let span = self.peek_token().span;
        Err(Diag::parse(
            span,
            format!("expected `>`, got {:?}", self.peek()),
        ))
    }

    fn expect_ident(&mut self, what: &str) -> Result<Ident, Diag> {
        match self.peek().clone() {
            TokenKind::Ident(name) => {
                let span = self.peek_token().span;
                self.bump();
                Ok(Ident { name, span })
            }
            other => {
                let span = self.peek_token().span;
                Err(Diag::parse(
                    span,
                    format!("expected {}, got {:?}", what, other),
                ))
            }
        }
    }

    /// Like [`expect_ident`] but also accepts mode keywords as
    /// member names (per F.10) plus framework keywords that
    /// conflict with field names of built-in struct values
    /// (notably `closure` on a `ClosureViolation` value). The
    /// post-`.` position is unambiguous, so admitting reserved
    /// words here is always safe.
    fn expect_member_name(&mut self) -> Result<Ident, Diag> {
        if let Some(name) = try_member_keyword_as_name(self.peek()) {
            let span = self.peek_token().span;
            self.bump();
            return Ok(Ident { name: name.to_string(), span });
        }
        match self.peek().clone() {
            TokenKind::Ident(name) => {
                let span = self.peek_token().span;
                self.bump();
                Ok(Ident { name, span })
            }
            other => {
                let span = self.peek_token().span;
                Err(Diag::parse(
                    span,
                    format!("expected member name, got {:?}", other),
                ))
            }
        }
    }

    /// Accept either a regular Ident or a keyword that is permitted
    /// as an identifier in expression / path position (primitive
    /// type names — `time`, `int`, etc. — and mode names —
    /// `bulk` / `harmonic` / `resolution`).
    fn expect_ident_or_kw_name(&mut self, what: &str) -> Result<Ident, Diag> {
        if let Some(name) = try_keyword_as_name(self.peek()) {
            let span = self.peek_token().span;
            self.bump();
            return Ok(Ident { name: name.to_string(), span });
        }
        self.expect_ident(what)
    }

    // === top level ========================================

    fn parse_program(&mut self) -> Result<Program, Diag> {
        let start = self.peek_token().span.start;
        let mut imports = Vec::new();
        let mut items = Vec::new();

        // Imports may appear at the start.
        while self.at(&TokenKind::Import) {
            imports.push(self.parse_import()?);
        }

        while !matches!(self.peek(), TokenKind::Eof) {
            // Tolerate stray semicolons (defensive).
            if self.eat(&TokenKind::Semi) {
                continue;
            }
            // An `import` keyword here means imports were mis-ordered:
            // the grammar (spec/grammar.ebnf) requires every import to
            // precede every top-level decl. Emit a typed error and
            // consume the bad import so the parser can recover instead
            // of looping on the same token (recover_to_top_level used
            // to list `Import` as a stop token and never advanced past
            // it — that produced an unbounded-allocation infinite loop
            // surfaced as a multi-GB OOM on real source).
            if matches!(self.peek(), TokenKind::Import) {
                let span = self.peek_token().span;
                self.diags.push(Diag::parse(
                    span,
                    "`import` statements must appear before any top-level \
                     declaration (per spec/grammar.ebnf `program = \
                     { import_decl } , { top_decl }`); move this import \
                     to the top of the file."
                        .to_string(),
                ));
                // Consume the offending `import "..." [as <ident>];`
                // shape so parsing makes forward progress.
                let _ = self.parse_import();
                continue;
            }
            match self.parse_top_decl() {
                Ok(item) => items.push(item),
                Err(d) => {
                    self.diags.push(d);
                    self.recover_to_top_level();
                }
            }
        }

        let end = self.peek_token().span.end;
        Ok(Program {
            imports,
            items,
            span: Span {
                start,
                end,
            },
        })
    }

    fn recover_to_top_level(&mut self) {
        // Skip until we find a likely top-level start. `Import` is
        // intentionally NOT in this set: imports are only valid at
        // the top of the file, so an `import` keyword encountered
        // during recovery is itself a mis-ordering error, not a
        // valid resume point. Including it caused an infinite-loop
        // OOM when a parse error landed in front of a mis-ordered
        // import (the second `parse_program` loop kept failing on
        // the same import token and recovery kept stopping at it).
        // The mis-ordered case is now caught explicitly in
        // `parse_program`.
        while !matches!(
            self.peek(),
            TokenKind::Eof
                | TokenKind::Locus
                | TokenKind::Perspective
                | TokenKind::Type
                | TokenKind::Const
                | TokenKind::Fn
                | TokenKind::Module
        ) {
            self.bump();
        }
    }

    fn parse_import(&mut self) -> Result<Import, Diag> {
        let kw = self.expect(TokenKind::Import, "import")?;
        let path = match self.peek().clone() {
            TokenKind::StringLit(s) => {
                self.bump();
                s
            }
            other => {
                return Err(Diag::parse(
                    self.peek_token().span,
                    format!("expected import path string, got {:?}", other),
                ));
            }
        };
        // v1.x-IMPORT: the `as <alias>` clause is required. The
        // alias names the namespace at the import site so every
        // cross-seed reference reads as `alias::Name` — the same
        // forcing-function discipline as v1.x-3's no-default-sub-
        // mode rule and v1.x-FORM-2's two-channel rule.
        if !self.peek_is_kw_as() {
            return Err(Diag::parse(
                self.peek_token().span,
                format!(
                    "import \"{}\" must declare an alias: `import \"{}\" as <name>;` \
                     (v1.x-IMPORT requires the namespace to be named at the import site)",
                    path, path,
                ),
            ));
        }
        self.bump();
        let alias = self.expect_ident("import alias")?.name;
        let semi = self.expect(TokenKind::Semi, ";")?;
        Ok(Import {
            path,
            alias: Some(alias),
            span: kw.span.merge(semi.span),
        })
    }

    fn parse_top_decl(&mut self) -> Result<TopDecl, Diag> {
        // Annotation prefix dispatch. `@form(...)` and
        // `@locality(...)` precede `locus` (and may stack on
        // one another); `@ffi("c")` precedes `fn`. Peek the
        // ident after `@` to decide which annotation parser to
        // call. After consuming all leading annotations the
        // remaining token is either `Locus` / `Ident("main")`
        // (for `main locus`) — locus path — or `Fn` (for `@ffi`).
        let mut form: Option<FormAnnotation> = None;
        let mut locality: Option<LocalityAnnotation> = None;
        let mut leading_span: Option<Span> = None;
        loop {
            if !matches!(self.peek(), TokenKind::At) {
                break;
            }
            let kind_tok = self.peek_at(1);
            let is_ffi = matches!(&kind_tok, TokenKind::Ident(s) if s == "ffi");
            let is_form = matches!(&kind_tok, TokenKind::Ident(s) if s == "form");
            let is_locality =
                matches!(&kind_tok, TokenKind::Ident(s) if s == "locality");
            if is_ffi {
                if form.is_some() || locality.is_some() {
                    return Err(Diag::parse(
                        self.peek_token().span,
                        "`@ffi(...)` is fn-only; it can't stack with \
                         `@form(...)` or `@locality(...)` (those precede \
                         `locus`)",
                    ));
                }
                let ffi = self.parse_ffi_annotation()?;
                if !matches!(self.peek(), TokenKind::Fn) {
                    return Err(Diag::parse(
                        self.peek_token().span,
                        "expected `fn` after `@ffi(...)` annotation",
                    ));
                }
                let mut fn_decl = self.parse_fn_decl_with_ffi(Some(ffi.clone()))?;
                fn_decl.span = ffi.span.merge(fn_decl.span);
                return Ok(TopDecl::Fn(fn_decl));
            }
            if is_form {
                if form.is_some() {
                    return Err(Diag::parse(
                        self.peek_token().span,
                        "duplicate `@form(...)` annotation; one form per \
                         locus",
                    ));
                }
                let f = self.parse_form_annotation()?;
                leading_span = Some(match leading_span {
                    Some(s) => s.merge(f.span),
                    None => f.span,
                });
                form = Some(f);
                continue;
            }
            if is_locality {
                if locality.is_some() {
                    return Err(Diag::parse(
                        self.peek_token().span,
                        "duplicate `@locality(...)` annotation; one per \
                         locus",
                    ));
                }
                let l = self.parse_locality_annotation()?;
                leading_span = Some(match leading_span {
                    Some(s) => s.merge(l.span),
                    None => l.span,
                });
                locality = Some(l);
                continue;
            }
            return Err(Diag::parse(
                self.peek_token().span,
                "expected `form`, `locality`, or `ffi` after `@`",
            ));
        }
        if form.is_some() || locality.is_some() {
            // Verify a `locus` (or contextual `main locus`)
            // follows.
            let next_is_locus = matches!(self.peek(), TokenKind::Locus);
            let next_is_main = matches!(
                self.peek(),
                TokenKind::Ident(s) if s == "main"
            );
            if !next_is_locus && !next_is_main {
                return Err(Diag::parse(
                    self.peek_token().span,
                    "expected `locus` (or `main locus`) after `@form(...)` \
                     / `@locality(...)` annotation",
                ));
            }
            let mut locus = self.parse_locus_decl()?;
            if let Some(s) = leading_span {
                locus.span = s.merge(locus.span);
            }
            locus.form = form;
            locus.locality = locality;
            return Ok(TopDecl::Locus(locus));
        }
        match self.peek() {
            TokenKind::Locus => self.parse_locus_decl().map(TopDecl::Locus),
            TokenKind::Perspective => self.parse_perspective_decl().map(TopDecl::Perspective),
            TokenKind::Type => self.parse_type_decl().map(TopDecl::Type),
            TokenKind::Const => self.parse_const_decl().map(TopDecl::Const),
            TokenKind::Fn => self.parse_fn_decl().map(TopDecl::Fn),
            TokenKind::Module => self.parse_module_decl().map(TopDecl::Module),
            TokenKind::Interface => self.parse_interface_decl().map(TopDecl::Interface),
            // `topic` is a contextual keyword recognized only here
            // at top-level decl position. Lexes as `Ident("topic")`
            // everywhere else (struct field names, vars, etc.).
            TokenKind::Ident(s) if s == "topic" => {
                self.parse_topic_decl().map(TopDecl::Topic)
            }
            // FUv0.8.2 #7 (2026-05-25): `target <name> { ... }` —
            // names the substrate + its capability profile.
            // Contextual keyword recognized only at top-level decl
            // position.
            TokenKind::Ident(s) if s == "target" => {
                self.parse_target_decl().map(TopDecl::Target)
            }
            // `main locus Foo { ... }` — Phase 2 entry-point
            // marker. Same contextual-keyword pattern. The
            // following token must be `locus`.
            TokenKind::Ident(s) if s == "main" => {
                self.parse_locus_decl().map(TopDecl::Locus)
            }
            other => Err(Diag::parse(
                self.peek_token().span,
                format!("expected top-level declaration, got {:?}", other),
            )),
        }
    }

    /// FUv0.8.2 #7: parse `target <name> { cap.path, ... }`.
    ///
    /// Grammar:
    ///   target_decl       = 'target' , IDENT , '{' ,
    ///                       [ capability { ',' capability } [ ',' ] ] , '}'
    ///   capability        = IDENT { '.' IDENT }
    ///
    /// `target` is a contextual keyword recognized only at top-
    /// level decl position (same as `topic` / `main`).
    fn parse_target_decl(&mut self) -> Result<TargetDecl, Diag> {
        let kw_tok = self.peek_token().clone();
        let kw = match &kw_tok.kind {
            TokenKind::Ident(s) if s == "target" => {
                self.bump();
                kw_tok
            }
            _ => {
                return Err(Diag::parse(
                    kw_tok.span,
                    "expected `target` keyword",
                ));
            }
        };
        let name = self.expect_ident("target name")?;
        self.expect(TokenKind::LBrace, "{")?;
        let mut capabilities: Vec<Capability> = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace) {
            let cap = self.parse_capability()?;
            capabilities.push(cap);
            // Trailing comma optional; comma separator required
            // between caps. `}` ends the list.
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        let close = self.expect(TokenKind::RBrace, "}")?;
        Ok(TargetDecl {
            name,
            capabilities,
            span: kw.span.merge(close.span),
        })
    }

    fn parse_capability(&mut self) -> Result<Capability, Diag> {
        let head = self.expect_ident("capability name segment")?;
        let mut segments = vec![head.clone()];
        let mut end_span = head.span;
        while self.eat(&TokenKind::Dot) {
            let seg = self.expect_ident("capability path segment after `.`")?;
            end_span = seg.span;
            segments.push(seg);
        }
        Ok(Capability {
            segments,
            span: head.span.merge(end_span),
        })
    }

    /// `topic Foo { payload: T; }` — Phase 1 carries only the
    /// payload type. Later phases extend with `transport:`,
    /// `bindings`, etc. Parser keeps the body shape open-ended:
    /// any `key: <expr-or-type>;` line is recognized; unknown
    /// keys are rejected by typecheck. Per-field validation
    /// (payload required, payload exactly once, etc.) lives
    /// downstream so the parser stays simple.
    fn parse_topic_decl(&mut self) -> Result<TopicDecl, Diag> {
        // `topic` is a contextual keyword — consumed here as
        // `Ident("topic")`, not `TokenKind::Topic`. See the
        // dispatch in `parse_top_decl`.
        let kw_tok = self.peek_token().clone();
        let kw = match &kw_tok.kind {
            TokenKind::Ident(s) if s == "topic" => {
                self.bump();
                kw_tok
            }
            _ => {
                return Err(Diag::parse(
                    kw_tok.span,
                    "expected `topic`",
                ));
            }
        };
        let name = self.expect_ident("topic name")?;
        // Optional declarative parent: `topic Login : Events { ... }`.
        let parent = if self.eat(&TokenKind::Colon) {
            Some(self.expect_ident("parent topic name")?)
        } else {
            None
        };
        self.expect(TokenKind::LBrace, "{")?;
        let mut payload: Option<TypeExpr> = None;
        let mut subject: Option<String> = None;
        while !matches!(self.peek(), TokenKind::RBrace) {
            let field_name = self.expect_ident("topic field name")?;
            self.expect(TokenKind::Colon, ":")?;
            match field_name.name.as_str() {
                "payload" => {
                    let ty = self.parse_type_expr()?;
                    if payload.is_some() {
                        return Err(Diag::parse(
                            field_name.span,
                            "duplicate `payload:` in topic declaration",
                        ));
                    }
                    payload = Some(ty);
                }
                "subject" => {
                    let tok = self.peek_token().clone();
                    let s = match tok.kind {
                        TokenKind::StringLit(s) => {
                            self.bump();
                            s
                        }
                        _ => {
                            return Err(Diag::parse(
                                tok.span,
                                "topic `subject:` must be a string literal",
                            ));
                        }
                    };
                    if subject.is_some() {
                        return Err(Diag::parse(
                            field_name.span,
                            "duplicate `subject:` in topic declaration",
                        ));
                    }
                    subject = Some(s);
                }
                other => {
                    return Err(Diag::parse(
                        field_name.span,
                        format!(
                            "unknown topic field `{}` (recognized: \
                             `payload:`, `subject:`)",
                            other
                        ),
                    ));
                }
            }
            self.expect(TokenKind::Semi, ";")?;
        }
        let close = self.expect(TokenKind::RBrace, "}")?;
        let payload = payload.ok_or_else(|| {
            Diag::parse(
                name.span,
                "topic declaration missing required `payload: T;` field",
            )
        })?;
        Ok(TopicDecl {
            name,
            parent,
            payload,
            subject,
            span: kw.span.merge(close.span),
        })
    }

    /// v1.x-FORM-1: `@form(<name>, <args>...)`.
    ///
    /// The `form` keyword is contextual — recognized only after
    /// `@` in annotation-prefix position. Outside that position
    /// it lexes as ordinary Ident.
    fn parse_form_annotation(&mut self) -> Result<FormAnnotation, Diag> {
        let at = self.expect(TokenKind::At, "@")?;
        let next = self.peek_token().clone();
        let is_form = matches!(&next.kind, TokenKind::Ident(s) if s == "form");
        if !is_form {
            return Err(Diag::parse(
                next.span,
                "expected `form` or `ffi` after `@` (recognized annotation prefixes)",
            ));
        }
        self.bump();
        self.expect(TokenKind::LParen, "(")?;
        let form_name = self.expect_ident("form name")?;
        let mut args = Vec::new();
        while self.eat(&TokenKind::Comma) {
            let arg_name = self.expect_ident("form argument name")?;
            self.expect(TokenKind::Eq, "=")?;
            let value = self.parse_expr()?;
            let span = arg_name.span.merge(value.span());
            args.push(FormArg {
                name: arg_name,
                value,
                span,
            });
        }
        let close = self.expect(TokenKind::RParen, ")")?;
        Ok(FormAnnotation {
            name: form_name,
            args,
            span: at.span.merge(close.span),
        })
    }

    /// F.32-2 v0.2 (2026-05-25): `@locality(L1|L2|L3|any)` on a
    /// locus declaration. The tier name is a contextual ident —
    /// accepted only inside this annotation.
    ///
    /// Grammar: `'@' 'locality' '(' ('L1'|'L2'|'L3'|'any') ')'`.
    fn parse_locality_annotation(
        &mut self,
    ) -> Result<LocalityAnnotation, Diag> {
        let at = self.expect(TokenKind::At, "@")?;
        let next = self.peek_token().clone();
        let is_locality = matches!(
            &next.kind,
            TokenKind::Ident(s) if s == "locality"
        );
        if !is_locality {
            return Err(Diag::parse(
                next.span,
                "expected `locality` after `@`",
            ));
        }
        self.bump();
        self.expect(TokenKind::LParen, "(")?;
        let tier_tok = self.peek_token().clone();
        let tier = match &tier_tok.kind {
            TokenKind::Ident(s) => match s.as_str() {
                "L1" | "l1" => LocalityTier::L1,
                "L2" | "l2" => LocalityTier::L2,
                "L3" | "l3" => LocalityTier::L3,
                "any" | "Any" => LocalityTier::Any,
                _ => {
                    return Err(Diag::parse(
                        tier_tok.span,
                        format!(
                            "unknown locality tier `{}`; expected one of \
                             `L1` / `L2` / `L3` / `any`",
                            s,
                        ),
                    ));
                }
            },
            _ => {
                return Err(Diag::parse(
                    tier_tok.span,
                    "expected locality tier identifier (L1 / L2 / L3 / any)",
                ));
            }
        };
        self.bump();
        let close = self.expect(TokenKind::RParen, ")")?;
        Ok(LocalityAnnotation {
            tier,
            span: at.span.merge(close.span),
        })
    }

    /// Stage-1 FFI (2026-05-22): parse `@ffi("c")` — the
    /// annotation prefix that marks the following `fn` declaration
    /// as an external C-ABI binding. Stage 1 accepts only the
    /// literal `"c"`; future ABIs (e.g. `"system"`) would extend
    /// the set. The annotation is only valid in front of a
    /// top-level free fn at Stage 1 (the dispatch in
    /// `parse_top_decl` enforces this).
    ///
    /// Grammar: `'@' 'ffi' '(' STRING ')'`.
    fn parse_ffi_annotation(&mut self) -> Result<FfiAnnotation, Diag> {
        let at = self.expect(TokenKind::At, "@")?;
        let next = self.peek_token().clone();
        let is_ffi = matches!(&next.kind, TokenKind::Ident(s) if s == "ffi");
        if !is_ffi {
            return Err(Diag::parse(
                next.span,
                "expected `ffi` after `@`",
            ));
        }
        self.bump();
        self.expect(TokenKind::LParen, "(")?;
        let abi_tok = self.peek_token().clone();
        let abi = match &abi_tok.kind {
            TokenKind::StringLit(s) => {
                self.bump();
                s.clone()
            }
            _ => {
                return Err(Diag::parse(
                    abi_tok.span,
                    "expected ABI string literal — `@ffi(\"c\")` is the only \
                     form accepted at Stage 1",
                ));
            }
        };
        if abi != "c" {
            return Err(Diag::parse(
                abi_tok.span,
                format!(
                    "unsupported FFI ABI {:?} — Stage 1 accepts only `\"c\"`",
                    abi
                ),
            ));
        }
        let close = self.expect(TokenKind::RParen, ")")?;
        Ok(FfiAnnotation {
            abi,
            span: at.span.merge(close.span),
        })
    }

    // === interface =======================================

    fn parse_interface_decl(&mut self) -> Result<InterfaceDecl, Diag> {
        let kw = self.expect(TokenKind::Interface, "interface")?;
        let name = self.expect_ident("interface name")?;
        self.expect(TokenKind::LBrace, "{")?;
        let mut methods = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace) {
            // Each method: `fn name(params...) -> ret;` — bodyless.
            // Default methods (with bodies) are deferred.
            let kw_fn = self.expect(TokenKind::Fn, "fn")?;
            let mname = self.expect_ident("method name")?;
            self.expect(TokenKind::LParen, "(")?;
            let mut params = Vec::new();
            if !matches!(self.peek(), TokenKind::RParen) {
                params.push(self.parse_param()?);
                while self.eat(&TokenKind::Comma) {
                    params.push(self.parse_param()?);
                }
            }
            let close = self.expect(TokenKind::RParen, ")")?;
            let ret = if self.eat(&TokenKind::Arrow) {
                Some(self.parse_type_expr()?)
            } else {
                None
            };
            self.expect(TokenKind::Semi, ";")?;
            methods.push(InterfaceMethodSig {
                name: mname,
                params,
                ret,
                span: kw_fn.span.merge(close.span),
            });
        }
        let close = self.expect(TokenKind::RBrace, "}")?;
        Ok(InterfaceDecl {
            name,
            methods,
            span: kw.span.merge(close.span),
        })
    }

    // === locus ===========================================

    fn parse_locus_decl(&mut self) -> Result<LocusDecl, Diag> {
        // Phase 2: optional `main` modifier — `main locus App { ... }`.
        // `main` is a contextual keyword (lexes as Ident); accepting
        // it here at locus-decl prefix position frees the identifier
        // for use as a struct field / local elsewhere.
        let is_main = matches!(
            self.peek(),
            TokenKind::Ident(s) if s == "main"
        );
        if is_main {
            self.bump();
        }
        let kw = self.expect(TokenKind::Locus, "locus")?;
        let name = self.expect_ident("locus name")?;
        // m63: optional `<K, V, ...>` generic param list right
        // after the locus name. Same shape as fn / type generic
        // params — codegen monomorphizes on use sites.
        let generics = self.parse_generic_params_opt()?;

        let mut annotations = Vec::new();
        if self.eat(&TokenKind::Colon) {
            annotations.push(self.parse_locus_annotation()?);
            while self.eat(&TokenKind::Comma) {
                annotations.push(self.parse_locus_annotation()?);
            }
        }

        self.expect(TokenKind::LBrace, "{")?;
        let mut members = Vec::new();
        while !self.at(&TokenKind::RBrace) && !matches!(self.peek(), TokenKind::Eof) {
            members.push(self.parse_locus_member()?);
        }
        let close = self.expect(TokenKind::RBrace, "}")?;
        // Reject `bindings { }` and `placement { }` blocks in
        // non-main loci at parse time so the diagnostic cites the
        // offending span directly. Both are deployment seams (per
        // F.31) and live at main only.
        if !is_main {
            for m in &members {
                if let LocusMember::Bindings(bb) = m {
                    return Err(Diag::parse(
                        bb.span,
                        format!(
                            "`bindings` block is only valid inside `main \
                             locus`; locus `{}` is not declared with the \
                             `main` modifier",
                            name.name
                        ),
                    ));
                }
                if let LocusMember::Placement(pb) = m {
                    return Err(Diag::parse(
                        pb.span,
                        format!(
                            "`placement` block is only valid inside `main \
                             locus`; locus `{}` is not declared with the \
                             `main` modifier",
                            name.name
                        ),
                    ));
                }
            }
        }
        Ok(LocusDecl {
            name,
            is_main,
            generics,
            annotations,
            form: None,
            locality: None,
            members,
            span: kw.span.merge(close.span),
        })
    }

    fn parse_locus_annotation(&mut self) -> Result<LocusAnnotation, Diag> {
        match self.peek() {
            TokenKind::Tier => {
                self.bump();
                match self.peek().clone() {
                    TokenKind::IntLit(n) => {
                        self.bump();
                        Ok(LocusAnnotation::Tier(n))
                    }
                    other => Err(Diag::parse(
                        self.peek_token().span,
                        format!("expected tier integer, got {:?}", other),
                    )),
                }
            }
            TokenKind::Projection => {
                self.bump();
                let class = match self.peek() {
                    TokenKind::Rich => {
                        self.bump();
                        ProjectionClass::Rich
                    }
                    TokenKind::Chunked => {
                        self.bump();
                        ProjectionClass::Chunked
                    }
                    TokenKind::Recognition => {
                        let kw_span = self.bump().span;
                        // v1.x-3: recognition REQUIRES
                        // `(cap=N, <sub_mode>)`. No default sub-mode;
                        // bare `: projection recognition` is rejected.
                        // Same forcing-function discipline as the
                        // 2026-05-12 two-channel rule — the user
                        // names the storage commitment at the
                        // declaration site.
                        if !matches!(self.peek(), TokenKind::LParen) {
                            return Err(Diag::parse(
                                kw_span,
                                "`: projection recognition` requires a sub-mode \
                                 commitment. Spell one of \
                                 `recognition(cap=N, fixed_cell(bytes=K))`, \
                                 `recognition(cap=N, shared_slab(bytes=K))`, \
                                 `recognition(cap=N, spillover(bytes=K))`, \
                                 `recognition(cap=N, summary_only)`. \
                                 v1.x-3 locks this in as a forcing function — \
                                 the substrate doesn't pick a default for you."
                                    .to_string(),
                            ));
                        }
                        let params = self.parse_recognition_params()?;
                        ProjectionClass::Recognition(Some(params))
                    }
                    other => {
                        return Err(Diag::parse(
                            self.peek_token().span,
                            format!("expected projection class, got {:?}", other),
                        ));
                    }
                };
                Ok(LocusAnnotation::Projection(class))
            }
            other => Err(Diag::parse(
                self.peek_token().span,
                format!(
                    "expected tier / projection annotation, got {:?}. \
                     (Schedule moved to main's `placement {{ }}` block in F.31.)",
                    other
                ),
            )),
        }
    }

    /// v1.x-3: parse the `(cap=N, <sub_mode>)` arg block of a
    /// `: projection recognition(...)` locus annotation. Caller has
    /// confirmed the next token is `LParen`.
    ///
    /// Grammar:
    ///   `(` `cap` `=` IntLit `,` recognition_sub_mode `)`
    ///   recognition_sub_mode :=
    ///       `fixed_cell` `(` `bytes` `=` IntLit `)`
    ///     | `shared_slab` `(` `bytes` `=` IntLit `)`
    ///     | `spillover`  `(` `bytes` `=` IntLit `)`
    ///     | `summary_only`
    ///
    /// `cap`, `bytes`, and the four sub-mode names lex as Ident —
    /// they're contextual keywords only valid inside this block,
    /// keeping the math-shaped identifier pool free outside it
    /// (same F.10-style discipline as `approx` / `within`).
    fn parse_recognition_params(&mut self) -> Result<RecognitionParams, Diag> {
        self.expect(TokenKind::LParen, "`(` after `recognition`")?;

        // cap = <IntLit>
        let cap_tok = self.peek_token().clone();
        let cap_name = match self.peek() {
            TokenKind::Ident(s) => s.clone(),
            other => {
                return Err(Diag::parse(
                    cap_tok.span,
                    format!(
                        "expected `cap` inside `recognition(...)`, got {:?}",
                        other
                    ),
                ));
            }
        };
        if cap_name != "cap" {
            return Err(Diag::parse(
                cap_tok.span,
                format!(
                    "expected `cap` inside `recognition(...)`, got `{}`",
                    cap_name
                ),
            ));
        }
        self.bump();
        self.expect(TokenKind::Eq, "`=` after `cap`")?;
        let cap_val = self.expect_positive_int_lit("cap")?;

        self.expect(TokenKind::Comma, "`,` between `cap` and sub-mode")?;

        // <sub_mode>
        let sub_tok = self.peek_token().clone();
        let sub_name = match self.peek() {
            TokenKind::Ident(s) => s.clone(),
            other => {
                return Err(Diag::parse(
                    sub_tok.span,
                    format!(
                        "expected recognition sub-mode (one of `fixed_cell`, \
                         `shared_slab`, `spillover`, `summary_only`), got {:?}",
                        other
                    ),
                ));
            }
        };
        self.bump();
        let sub_mode = match sub_name.as_str() {
            "fixed_cell" => RecognitionSubMode::FixedCell,
            "shared_slab" => RecognitionSubMode::SharedSlab,
            "spillover" => RecognitionSubMode::Spillover,
            "summary_only" => RecognitionSubMode::SummaryOnly,
            other => {
                return Err(Diag::parse(
                    sub_tok.span,
                    format!(
                        "unknown recognition sub-mode `{}`; expected one of \
                         `fixed_cell`, `shared_slab`, `spillover`, `summary_only`",
                        other
                    ),
                ));
            }
        };

        self.expect(TokenKind::RParen, "`)` closing `recognition(...)`")?;
        Ok(RecognitionParams {
            cap: cap_val,
            sub_mode,
        })
    }

    fn expect_positive_int_lit(&mut self, field: &str) -> Result<u64, Diag> {
        let tok = self.peek_token().clone();
        match self.peek() {
            TokenKind::IntLit(n) => {
                let n = *n;
                self.bump();
                if n <= 0 {
                    return Err(Diag::parse(
                        tok.span,
                        format!("`{}` must be a positive integer literal", field),
                    ));
                }
                Ok(n as u64)
            }
            other => Err(Diag::parse(
                tok.span,
                format!(
                    "expected positive integer literal for `{}`, got {:?}",
                    field, other
                ),
            )),
        }
    }

    fn parse_locus_member(&mut self) -> Result<LocusMember, Diag> {
        match self.peek() {
            TokenKind::Params => self.parse_params_block().map(LocusMember::Params),
            TokenKind::Contract => self.parse_contract_block().map(LocusMember::Contract),
            TokenKind::Bus => self.parse_bus_block().map(LocusMember::Bus),
            TokenKind::Capacity => self.parse_capacity_block().map(LocusMember::Capacity),
            TokenKind::Birth | TokenKind::Accept | TokenKind::Run | TokenKind::Drain | TokenKind::Dissolve => {
                self.parse_lifecycle_decl().map(LocusMember::Lifecycle)
            }
            // `mode` is contextual; recognized as a member-
            // introducer here. Frees the identifier for use as a
            // param/field name (raylib's `cam.mode` etc.).
            TokenKind::Ident(s) if s == "mode" => {
                self.parse_mode_decl().map(LocusMember::Mode)
            }
            TokenKind::OnFailure => self.parse_failure_decl().map(LocusMember::Failure),
            TokenKind::Closure => self.parse_closure_decl().map(LocusMember::Closure),
            TokenKind::Fn => self.parse_fn_decl().map(LocusMember::Fn),
            TokenKind::Const => self.parse_const_decl().map(LocusMember::Const),
            TokenKind::Type => self.parse_type_decl().map(LocusMember::Type),
            // Phase 2 contextual keyword — `bindings { ... }`.
            // Lexes as Ident; recognized as a member-introducer
            // here. The "must be inside `main locus`" check fires
            // in `parse_locus_decl` after the body is parsed.
            TokenKind::Ident(s) if s == "bindings" => {
                self.parse_bindings_block().map(LocusMember::Bindings)
            }
            // F.31 contextual keyword — `placement { ... }`.
            // Lexes as Ident; recognized as a member-introducer
            // here. The "must be inside `main locus`" check
            // fires in `parse_locus_decl` (parallel to bindings).
            TokenKind::Ident(s) if s == "placement" => {
                self.parse_placement_block().map(LocusMember::Placement)
            }
            // F.27 v2 contextual keyword — `birth_check { EXPR }
            // -> violate NAME;`. Lexes as Ident; recognized here.
            TokenKind::Ident(s) if s == "birth_check" => {
                self.parse_birth_check_decl().map(LocusMember::BirthCheck)
            }
            other => Err(Diag::parse(
                self.peek_token().span,
                format!("expected locus member, got {:?}", other),
            )),
        }
    }

    /// F.27 v2: `birth_check { COND_EXPR } -> violate NAME[(PAYLOAD)];`.
    /// Parses as a declarative invariant check that runs after
    /// the locus's birth() body (and birth-epoch closures) at
    /// instantiation. If COND_EXPR returns true, NAME's
    /// closure violates with the locus's fully-constructed
    /// state. Multiple birth_check clauses are evaluated in
    /// declaration order; the first to fire short-circuits the
    /// rest.
    fn parse_birth_check_decl(&mut self) -> Result<BirthCheckDecl, Diag> {
        let kw_tok = self.peek_token().clone();
        self.bump(); // consume `birth_check` ident
        self.expect(TokenKind::LBrace, "{")?;
        let cond = self.parse_expr()?;
        self.expect(TokenKind::RBrace, "}")?;
        // `->` arrow. Lex's `Arrow` covers `->` if present;
        // otherwise the dash + > combo is what the existing
        // fail-disposition syntax uses. We require `->`.
        self.expect(TokenKind::Arrow, "->")?;
        let violate_kw = self.peek_token().clone();
        match &violate_kw.kind {
            TokenKind::Ident(s) if s == "violate" => {
                self.bump();
            }
            _ => {
                return Err(Diag::parse(
                    violate_kw.span,
                    format!(
                        "expected `violate` after `->` in birth_check, got {:?}",
                        violate_kw.kind
                    ),
                ));
            }
        }
        let closure_name = self.expect_ident("closure name")?;
        let payload = if matches!(self.peek(), TokenKind::LParen) {
            self.bump();
            let p = self.parse_expr()?;
            self.expect(TokenKind::RParen, ")")?;
            Some(p)
        } else {
            None
        };
        let semi = self.expect(TokenKind::Semi, ";")?;
        Ok(BirthCheckDecl {
            cond,
            closure_name,
            payload,
            span: kw_tok.span.merge(semi.span),
        })
    }

    /// `bindings { Topic: <transport> [where <constraint>, ...]; ... }`
    /// — Phase 2 surface, extended Form K (2026-05-20) with the
    /// optional `where`-clause carrying operational constraints
    /// (`intra_process`, `intra_machine`, `cross_machine`,
    /// `zero_copy`).
    fn parse_bindings_block(&mut self) -> Result<BindingsBlock, Diag> {
        let kw_tok = self.peek_token().clone();
        self.bump(); // consume `bindings` ident
        self.expect(TokenKind::LBrace, "{")?;
        let mut entries = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace) {
            let topic = self.expect_ident("topic name")?;
            self.expect(TokenKind::Colon, ":")?;
            let transport = self.parse_transport_spec()?;
            let constraints = self.parse_binding_constraints_opt()?;
            let semi = self.expect(TokenKind::Semi, ";")?;
            entries.push(BindingEntry {
                topic: topic.clone(),
                transport,
                constraints,
                span: topic.span.merge(semi.span),
            });
        }
        let close = self.expect(TokenKind::RBrace, "}")?;
        Ok(BindingsBlock {
            entries,
            span: kw_tok.span.merge(close.span),
        })
    }

    /// F.31 (2026-05-23): parse a `placement { field: SPEC; }`
    /// block. Caller has the `placement` ident as `peek()`.
    /// The "must be inside main locus" check fires later in
    /// `parse_locus_decl` (parallel to `bindings`).
    fn parse_placement_block(&mut self) -> Result<PlacementBlock, Diag> {
        let kw_tok = self.peek_token().clone();
        self.bump(); // consume `placement` ident
        self.expect(TokenKind::LBrace, "{")?;
        let mut entries = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace) {
            let field = self.expect_ident("field name")?;
            self.expect(TokenKind::Colon, ":")?;
            let spec = self.parse_placement_spec()?;
            let semi = self.expect(TokenKind::Semi, ";")?;
            entries.push(PlacementEntry {
                field: field.clone(),
                spec,
                span: field.span.merge(semi.span),
            });
        }
        let close = self.expect(TokenKind::RBrace, "}")?;
        Ok(PlacementBlock {
            entries,
            span: kw_tok.span.merge(close.span),
        })
    }

    /// F.31: parse one placement spec — `cooperative` /
    /// `cooperative(pool = X)` / `pinned` / `pinned(core = N)`.
    /// Both head keywords are contextual Idents.
    fn parse_placement_spec(&mut self) -> Result<PlacementSpec, Diag> {
        let head_tok = self.peek_token().clone();
        let head = match &head_tok.kind {
            TokenKind::Ident(s) => s.clone(),
            other => {
                return Err(Diag::parse(
                    head_tok.span,
                    format!(
                        "expected placement spec (`cooperative` or `pinned`), \
                         got {:?}",
                        other
                    ),
                ));
            }
        };
        self.bump();
        match head.as_str() {
            "cooperative" => {
                let pool = if matches!(self.peek(), TokenKind::LParen) {
                    self.bump();
                    let kw_tok = self.peek_token().clone();
                    let kw = match &kw_tok.kind {
                        TokenKind::Ident(s) => s.clone(),
                        other => {
                            return Err(Diag::parse(
                                kw_tok.span,
                                format!(
                                    "expected `pool` inside `cooperative(...)`, got {:?}",
                                    other
                                ),
                            ));
                        }
                    };
                    if kw != "pool" {
                        return Err(Diag::parse(
                            kw_tok.span,
                            format!(
                                "unknown cooperative attribute `{}`; only `pool` \
                                 is recognized",
                                kw
                            ),
                        ));
                    }
                    self.bump();
                    self.expect(TokenKind::Eq, "expected `=` after `pool`")?;
                    let name = self.expect_ident("pool name")?;
                    self.expect(
                        TokenKind::RParen,
                        "expected `)` after cooperative(pool = X)",
                    )?;
                    Some(name)
                } else {
                    None
                };
                Ok(PlacementSpec::Cooperative { pool })
            }
            "pinned" => {
                let core = if matches!(self.peek(), TokenKind::LParen) {
                    self.bump();
                    let kw_tok = self.peek_token().clone();
                    let kw = match &kw_tok.kind {
                        TokenKind::Ident(s) => s.clone(),
                        other => {
                            return Err(Diag::parse(
                                kw_tok.span,
                                format!(
                                    "expected `core` inside `pinned(...)`, got {:?}",
                                    other
                                ),
                            ));
                        }
                    };
                    if kw != "core" {
                        return Err(Diag::parse(
                            kw_tok.span,
                            format!(
                                "unknown pinned attribute `{}`; only `core` \
                                 is recognized",
                                kw
                            ),
                        ));
                    }
                    self.bump();
                    self.expect(TokenKind::Eq, "expected `=` after `core`")?;
                    let n_tok = self.peek_token().clone();
                    let n = match &n_tok.kind {
                        TokenKind::IntLit(v) => *v,
                        other => {
                            return Err(Diag::parse(
                                n_tok.span,
                                format!(
                                    "expected integer CPU index after `core =`, got {:?}",
                                    other
                                ),
                            ));
                        }
                    };
                    self.bump();
                    self.expect(
                        TokenKind::RParen,
                        "expected `)` after pinned(core = N)",
                    )?;
                    Some(n)
                } else {
                    None
                };
                Ok(PlacementSpec::Pinned { core })
            }
            other => Err(Diag::parse(
                head_tok.span,
                format!(
                    "expected `cooperative` or `pinned` as placement spec, got `{}`",
                    other
                ),
            )),
        }
    }

    /// Form K (2026-05-20): the optional `where <c>, <c>, ...`
    /// suffix on a binding entry. `where` is a reserved keyword
    /// (`TokenKind::Where`). Each constraint is a bare
    /// identifier matched against
    /// `BindingConstraint::from_ident`. Unknown idents produce
    /// a diagnostic citing the constraint span.
    fn parse_binding_constraints_opt(
        &mut self,
    ) -> Result<Vec<SpannedBindingConstraint>, Diag> {
        if !self.eat(&TokenKind::Where) {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        out.push(self.parse_binding_constraint()?);
        while self.eat(&TokenKind::Comma) {
            out.push(self.parse_binding_constraint()?);
        }
        Ok(out)
    }

    fn parse_binding_constraint(
        &mut self,
    ) -> Result<SpannedBindingConstraint, Diag> {
        let tok = self.peek_token().clone();
        let name = match &tok.kind {
            TokenKind::Ident(s) => s.clone(),
            other => {
                return Err(Diag::parse(
                    tok.span,
                    format!(
                        "expected binding constraint name (intra_process / \
                         intra_machine / cross_machine / zero_copy), got {:?}",
                        other
                    ),
                ));
            }
        };
        self.bump();
        match BindingConstraint::from_ident(&name) {
            Some(kind) => Ok(SpannedBindingConstraint {
                kind,
                span: tok.span,
            }),
            None => Err(Diag::parse(
                tok.span,
                format!(
                    "unknown binding constraint `{}` — expected one of \
                     intra_process, intra_machine, cross_machine, zero_copy",
                    name
                ),
            )),
        }
    }

    /// Transport constructor.
    ///
    /// Two shapes:
    /// - `unix("/path/to/sock")` or `unix("/path", role: listen)` —
    ///   the substrate-provided transport. The optional `role:`
    ///   kwarg overrides the typechecker's role inference from the
    ///   bus block.
    /// - `MyNatsAdapter { url: "...", ... }` — user-supplied
    ///   protocol-layer transport (Wave B of the bus-transport
    ///   redesign). The named locus must structurally satisfy
    ///   `__StdBusAdapter`; the field-init block matches the
    ///   locus's params block. Detected by the head being a
    ///   capitalized identifier (locus naming convention).
    fn parse_transport_spec(&mut self) -> Result<TransportSpec, Diag> {
        let head_tok = self.peek_token().clone();
        let head_name = match &head_tok.kind {
            TokenKind::Ident(s) => s.clone(),
            other => {
                return Err(Diag::parse(
                    head_tok.span,
                    format!(
                        "expected transport constructor `unix` or adapter locus name, got {:?}",
                        other
                    ),
                ));
            }
        };
        self.bump();
        // Wave B: a capitalized head is an adapter locus literal.
        // unix is the only lowercase keyword; everything else
        // capitalized routes to the Adapter branch.
        if head_name.chars().next().map_or(false, |c| c.is_ascii_uppercase()) {
            let lb = self.expect(TokenKind::LBrace, "{")?;
            let mut inits = Vec::new();
            if !self.at(&TokenKind::RBrace) {
                inits.push(self.parse_struct_init()?);
                while self.eat(&TokenKind::Comma) {
                    if self.at(&TokenKind::RBrace) {
                        break;
                    }
                    inits.push(self.parse_struct_init()?);
                }
            }
            let close = self.expect(TokenKind::RBrace, "}")?;
            let _ = lb;
            return Ok(TransportSpec::Adapter {
                locus: Ident {
                    name: head_name,
                    span: head_tok.span,
                },
                inits,
                span: head_tok.span.merge(close.span),
            });
        }
        match head_name.as_str() {
            "unix" => {
                self.expect(TokenKind::LParen, "(")?;
                let path = self.expect_string_literal("unix path")?;
                let mut role: Option<TransportRole> = None;
                while self.eat(&TokenKind::Comma) {
                    let key = self.expect_ident("unix kwarg name")?;
                    self.expect(TokenKind::Colon, ":")?;
                    match key.name.as_str() {
                        "role" => {
                            let tok = self.peek_token().clone();
                            let role_name = match &tok.kind {
                                TokenKind::Ident(s) => s.clone(),
                                other => {
                                    return Err(Diag::parse(
                                        tok.span,
                                        format!(
                                            "expected `connect` or `listen` for `role:`, \
                                             got {:?}",
                                            other
                                        ),
                                    ));
                                }
                            };
                            self.bump();
                            role = Some(match role_name.as_str() {
                                "connect" => TransportRole::Connect,
                                "listen" => TransportRole::Listen,
                                other => {
                                    return Err(Diag::parse(
                                        tok.span,
                                        format!(
                                            "expected `connect` or `listen` for `role:`, \
                                             got `{}`",
                                            other
                                        ),
                                    ));
                                }
                            });
                        }
                        other => {
                            return Err(Diag::parse(
                                key.span,
                                format!(
                                    "unknown `unix` kwarg `{}` (recognized: `role`)",
                                    other
                                ),
                            ));
                        }
                    }
                }
                let close = self.expect(TokenKind::RParen, ")")?;
                Ok(TransportSpec::Unix {
                    path,
                    role,
                    span: head_tok.span.merge(close.span),
                })
            }
            "shm_ring" => {
                // Form K4b + K7 (2026-05-20): SHM ring transport.
                // `shm_ring("/name", slot_count: N, on_overflow: <policy>)`.
                // Name is required; slot_count defaults to 128;
                // on_overflow is REQUIRED (no default) — Form K7
                // forces the user to think about back-pressure
                // semantics for each high-throughput topic.
                self.expect(TokenKind::LParen, "(")?;
                let name = self.expect_string_literal("shm_ring name")?;
                let mut slot_count: u64 = 128;
                let mut overflow: Option<ShmRingOverflow> = None;
                while self.eat(&TokenKind::Comma) {
                    let key = self.expect_ident("shm_ring kwarg name")?;
                    self.expect(TokenKind::Colon, ":")?;
                    match key.name.as_str() {
                        "slot_count" => {
                            let tok = self.peek_token().clone();
                            match tok.kind {
                                TokenKind::IntLit(n) if n > 0 => {
                                    self.bump();
                                    slot_count = n as u64;
                                }
                                TokenKind::IntLit(n) => {
                                    return Err(Diag::parse(
                                        tok.span,
                                        format!(
                                            "shm_ring slot_count must be positive, got {}",
                                            n
                                        ),
                                    ));
                                }
                                other => {
                                    return Err(Diag::parse(
                                        tok.span,
                                        format!(
                                            "expected positive integer for shm_ring \
                                             `slot_count:`, got {:?}",
                                            other
                                        ),
                                    ));
                                }
                            }
                        }
                        "on_overflow" => {
                            let tok = self.peek_token().clone();
                            let policy_name = match &tok.kind {
                                TokenKind::Ident(s) => s.clone(),
                                other => {
                                    return Err(Diag::parse(
                                        tok.span,
                                        format!(
                                            "expected `block`, `drop`, or `fail` for \
                                             `on_overflow:`, got {:?}",
                                            other
                                        ),
                                    ));
                                }
                            };
                            self.bump();
                            overflow = Some(match ShmRingOverflow::from_ident(&policy_name) {
                                Some(p) => p,
                                None => {
                                    return Err(Diag::parse(
                                        tok.span,
                                        format!(
                                            "unknown `on_overflow` policy `{}` \
                                             (expected `block`, `drop`, or `fail`)",
                                            policy_name
                                        ),
                                    ));
                                }
                            });
                        }
                        other => {
                            return Err(Diag::parse(
                                key.span,
                                format!(
                                    "unknown `shm_ring` kwarg `{}` (recognized: \
                                     `slot_count`, `on_overflow`)",
                                    other
                                ),
                            ));
                        }
                    }
                }
                let close = self.expect(TokenKind::RParen, ")")?;
                let overflow = overflow.ok_or_else(|| {
                    Diag::parse(
                        head_tok.span.merge(close.span),
                        "shm_ring binding requires `on_overflow:` (one of \
                         `block`, `drop`, `fail`) — Form K7 has no default \
                         back-pressure policy; pick one explicitly. `block` \
                         waits for the consumer; `drop` overwrites (silent \
                         data loss); `fail` panics on ring-full"
                            .to_string(),
                    )
                })?;
                Ok(TransportSpec::ShmRing {
                    name,
                    slot_count,
                    overflow,
                    span: head_tok.span.merge(close.span),
                })
            }
            other => Err(Diag::parse(
                head_tok.span,
                format!(
                    "unknown transport constructor `{}` (recognized: `unix(...)`, \
                     `shm_ring(...)`, or a capitalized adapter locus name with a \
                     `{{ ... }}` block; in-memory delivery is absence-of-entry)",
                    other
                ),
            )),
        }
    }

    fn expect_string_literal(&mut self, ctx: &str) -> Result<String, Diag> {
        let tok = self.peek_token().clone();
        match tok.kind {
            TokenKind::StringLit(s) => {
                self.bump();
                Ok(s)
            }
            _ => Err(Diag::parse(
                tok.span,
                format!("expected string literal for {}", ctx),
            )),
        }
    }

    /// F.22 `capacity { pool X of T; heap Y of T; ... }`.
    /// `pool` and `heap` lex as plain idents (not reserved); the
    /// parser recognizes them contextually so the surrounding
    /// identifier pool stays unreserved.
    fn parse_capacity_block(&mut self) -> Result<CapacityBlock, Diag> {
        let kw = self.expect(TokenKind::Capacity, "capacity")?;
        self.expect(TokenKind::LBrace, "{")?;
        let mut slots = Vec::new();
        while !self.at(&TokenKind::RBrace)
            && !matches!(self.peek(), TokenKind::Eof)
        {
            slots.push(self.parse_capacity_slot()?);
        }
        let close = self.expect(TokenKind::RBrace, "}")?;
        Ok(CapacityBlock {
            slots,
            span: kw.span.merge(close.span),
        })
    }

    fn parse_capacity_slot(&mut self) -> Result<CapacitySlot, Diag> {
        let kind_ident =
            self.expect_ident("slot kind (`pool` or `heap`)")?;
        let kind = match kind_ident.name.as_str() {
            "pool" => CapacitySlotKind::Pool,
            "heap" => CapacitySlotKind::Heap,
            other => {
                return Err(Diag::parse(
                    kind_ident.span,
                    format!(
                        "expected `pool` or `heap` slot kind, got `{}`",
                        other
                    ),
                ));
            }
        };
        let name = self.expect_ident("slot name")?;
        self.expect(TokenKind::Of, "of")?;
        let elem_ty = self.parse_type_expr()?;
        // v1.x-FORM-4 optional trailing clause:
        //   `indexed_by <fieldname>`
        // Names a field of the cell type as the hashmap key.
        // Only meaningful on `@form(hashmap)` loci; ignored
        // elsewhere (typecheck flags misuse).
        let indexed_by = if self.eat(&TokenKind::IndexedBy) {
            Some(self.expect_ident("field name after `indexed_by`")?)
        } else {
            None
        };
        // F.22 v1.x-4 optional trailing clause:
        //   `as_parent_for <ChildLocus>`
        let as_parent_for = if self.eat(&TokenKind::AsParentFor) {
            Some(self.expect_ident("child locus name after `as_parent_for`")?)
        } else {
            None
        };
        let semi = self.expect(TokenKind::Semi, ";")?;
        Ok(CapacitySlot {
            span: kind_ident.span.merge(semi.span),
            name,
            kind,
            elem_ty,
            as_parent_for,
            indexed_by,
        })
    }

    fn parse_params_block(&mut self) -> Result<ParamsBlock, Diag> {
        let kw = self.expect(TokenKind::Params, "params")?;
        self.expect(TokenKind::LBrace, "{")?;
        let mut params = Vec::new();
        while !self.at(&TokenKind::RBrace) && !matches!(self.peek(), TokenKind::Eof) {
            params.push(self.parse_param_decl()?);
        }
        let close = self.expect(TokenKind::RBrace, "}")?;
        Ok(ParamsBlock {
            params,
            span: kw.span.merge(close.span),
        })
    }

    fn parse_param_decl(&mut self) -> Result<ParamDecl, Diag> {
        let name = self.expect_ident("param name")?;
        let ty = if self.eat(&TokenKind::Colon) {
            // Could be inferred or a type expression. Check next.
            if self.at(&TokenKind::Inferred) {
                // Form: name : inferred ;
                self.bump();
                let semi = self.expect(TokenKind::Semi, ";")?;
                return Ok(ParamDecl {
                    span: name.span.merge(semi.span),
                    name,
                    ty: None,
                    init: ParamInit::Inferred,
                });
            }
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        // Either `= expr ;`, `: inferred ;`, or just `;` (no default
        // — required at instantiation; no value declared at the
        // params block).
        let init = if self.eat(&TokenKind::Eq) {
            ParamInit::Value(self.parse_expr()?)
        } else if self.eat(&TokenKind::Colon) {
            // `name: T : inferred ;` form
            self.expect(TokenKind::Inferred, "inferred")?;
            ParamInit::Inferred
        } else if self.at(&TokenKind::Semi) {
            // No default; the value must be supplied at instantiation.
            // We model this as Inferred for now (treating "must come
            // from outside" the same way as "compiler/runtime
            // resolves"). Future: distinguish required-at-instantiation
            // from inferred-by-system.
            ParamInit::Inferred
        } else {
            return Err(Diag::parse(
                self.peek_token().span,
                "expected `=`, `: inferred`, or `;` for param init",
            ));
        };
        let semi = self.expect(TokenKind::Semi, ";")?;
        Ok(ParamDecl {
            span: name.span.merge(semi.span),
            name,
            ty,
            init,
        })
    }

    fn parse_contract_block(&mut self) -> Result<ContractBlock, Diag> {
        let kw = self.expect(TokenKind::Contract, "contract")?;
        if self.eat(&TokenKind::Colon) {
            self.expect(TokenKind::Inferred, "inferred")?;
            let semi = self.expect(TokenKind::Semi, ";")?;
            return Ok(ContractBlock {
                kind: ContractKind::Inferred,
                span: kw.span.merge(semi.span),
            });
        }
        self.expect(TokenKind::LBrace, "{")?;
        let mut members = Vec::new();
        while !self.at(&TokenKind::RBrace) && !matches!(self.peek(), TokenKind::Eof) {
            members.push(self.parse_contract_member()?);
        }
        let close = self.expect(TokenKind::RBrace, "}")?;
        Ok(ContractBlock {
            kind: ContractKind::Members(members),
            span: kw.span.merge(close.span),
        })
    }

    fn parse_contract_member(&mut self) -> Result<ContractMember, Diag> {
        let direction_tok = self.bump();
        let direction = match direction_tok.kind {
            TokenKind::Expose => ContractDirection::Expose,
            TokenKind::Consume => ContractDirection::Consume,
            other => {
                return Err(Diag::parse(
                    direction_tok.span,
                    format!("expected `expose` or `consume`, got {:?}", other),
                ));
            }
        };
        // Either: NAME : TYPE ;  or  inferred ;
        if self.eat(&TokenKind::Inferred) {
            let semi = self.expect(TokenKind::Semi, ";")?;
            return Ok(ContractMember {
                direction,
                name: ContractName::Inferred,
                ty: None,
                span: direction_tok.span.merge(semi.span),
            });
        }
        let name = self.expect_ident("contract field name")?;
        self.expect(TokenKind::Colon, ":")?;
        let ty = self.parse_type_expr()?;
        let semi = self.expect(TokenKind::Semi, ";")?;
        Ok(ContractMember {
            direction,
            name: ContractName::Named(name),
            ty: Some(ty),
            span: direction_tok.span.merge(semi.span),
        })
    }

    fn parse_bus_block(&mut self) -> Result<BusBlock, Diag> {
        let kw = self.expect(TokenKind::Bus, "bus")?;
        self.expect(TokenKind::LBrace, "{")?;
        let mut members = Vec::new();
        while !self.at(&TokenKind::RBrace) && !matches!(self.peek(), TokenKind::Eof) {
            members.push(self.parse_bus_member()?);
        }
        let close = self.expect(TokenKind::RBrace, "}")?;
        Ok(BusBlock {
            members,
            span: kw.span.merge(close.span),
        })
    }

    fn parse_bus_member(&mut self) -> Result<BusMember, Diag> {
        match self.peek() {
            TokenKind::Subscribe => {
                let kw = self.bump();
                let subject = self.parse_bus_subject("subscribe")?;
                // `as IDENT`
                self.expect_kw_as()?;
                let handler = self.expect_ident("handler name")?;
                let ty = if self.eat(&TokenKind::Of) {
                    self.expect(TokenKind::Type, "type")?;
                    Some(self.parse_type_expr()?)
                } else {
                    None
                };
                let semi = self.expect(TokenKind::Semi, ";")?;
                Ok(BusMember::Subscribe {
                    subject,
                    handler,
                    ty,
                    span: kw.span.merge(semi.span),
                })
            }
            TokenKind::Publish => {
                let kw = self.bump();
                let subject = self.parse_bus_subject("publish")?;
                let ty = if self.eat(&TokenKind::Of) {
                    self.expect(TokenKind::Type, "type")?;
                    Some(self.parse_type_expr()?)
                } else {
                    None
                };
                let alias = if self.peek_is_kw_as() {
                    self.bump();
                    Some(self.expect_ident("alias")?)
                } else {
                    None
                };
                let semi = self.expect(TokenKind::Semi, ";")?;
                Ok(BusMember::Publish {
                    subject,
                    ty,
                    alias,
                    span: kw.span.merge(semi.span),
                })
            }
            other => Err(Diag::parse(
                self.peek_token().span,
                format!("expected subscribe or publish, got {:?}", other),
            )),
        }
    }

    /// Parse a bus subscribe/publish subject — either a string
    /// literal (legacy form) or a topic-name identifier (new form
    /// per `topic Foo { ... }` decls). Typecheck enforces the
    /// "of type T" constraint per form.
    fn parse_bus_subject(&mut self, ctx: &str) -> Result<BusSubject, Diag> {
        let tok = self.peek_token().clone();
        match tok.kind {
            // Literal-string subjects (`subscribe "log.**" as h of
            // type T;`) remain accepted because the log namespace
            // lotus relies on wildcard publish + runtime-computed
            // subject strings, and the topic-decl form has no
            // equivalent at v1. The topic form is preferred for
            // simple 1:1 subject-payload bindings; literal subjects
            // stay for wildcard / dynamic cases.
            TokenKind::StringLit(s) => {
                self.bump();
                Ok(BusSubject::Literal {
                    subject: s,
                    span: tok.span,
                })
            }
            TokenKind::Ident(_) => {
                // A7 (G16): admit `alias::Foo` (qualified path) so
                // cross-seed subscribe / publish over an imported
                // lib's topic decl parses. Single-segment paths
                // remain `BusSubject::Topic(Ident)` so the existing
                // desugar pass handles them unchanged.
                let qn = self.parse_qualified_name()?;
                if qn.segments.len() == 1 {
                    Ok(BusSubject::Topic(qn.segments.into_iter().next().unwrap()))
                } else {
                    Ok(BusSubject::QualifiedTopic(qn))
                }
            }
            other => Err(Diag::parse(
                tok.span,
                format!(
                    "expected subject string or topic name after `{}`, got {:?}",
                    ctx, other
                ),
            )),
        }
    }

    /// `as` is not a reserved keyword in our token set. The grammar
    /// uses it in import aliases and bus subscriptions. We accept
    /// the identifier `as` here.
    fn expect_kw_as(&mut self) -> Result<(), Diag> {
        match self.peek().clone() {
            TokenKind::Ident(name) if name == "as" => {
                self.bump();
                Ok(())
            }
            _ => Err(Diag::parse(
                self.peek_token().span,
                "expected `as`",
            )),
        }
    }

    fn peek_is_kw_as(&self) -> bool {
        matches!(self.peek(), TokenKind::Ident(name) if name == "as")
    }

    fn parse_lifecycle_decl(&mut self) -> Result<LifecycleDecl, Diag> {
        let kw_tok = self.bump();
        let kind = match kw_tok.kind {
            TokenKind::Birth => LifecycleKind::Birth,
            TokenKind::Accept => LifecycleKind::Accept,
            TokenKind::Run => LifecycleKind::Run,
            TokenKind::Drain => LifecycleKind::Drain,
            TokenKind::Dissolve => LifecycleKind::Dissolve,
            _ => unreachable!(),
        };
        let params = self.parse_paren_params()?;
        let ret = if self.eat(&TokenKind::Arrow) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        let body = self.parse_block()?;
        Ok(LifecycleDecl {
            kind,
            params,
            ret,
            span: kw_tok.span.merge(body.span),
            body,
        })
    }

    fn parse_paren_params(&mut self) -> Result<Vec<Param>, Diag> {
        if !self.eat(&TokenKind::LParen) {
            return Ok(Vec::new());
        }
        let mut params = Vec::new();
        if !self.at(&TokenKind::RParen) {
            params.push(self.parse_param()?);
            while self.eat(&TokenKind::Comma) {
                params.push(self.parse_param()?);
            }
        }
        self.expect(TokenKind::RParen, ")")?;
        Ok(params)
    }

    fn parse_param(&mut self) -> Result<Param, Diag> {
        let name = self.expect_ident("parameter name")?;
        self.expect(TokenKind::Colon, ":")?;
        let ty = self.parse_type_expr()?;
        let default = if self.eat(&TokenKind::Eq) {
            Some(self.parse_expr()?)
        } else {
            None
        };
        let span = name.span.merge(ty.span());
        Ok(Param {
            name,
            ty,
            default,
            span,
        })
    }

    fn parse_mode_decl(&mut self) -> Result<ModeDecl, Diag> {
        // Caller guarantees current token is `Ident("mode")`.
        let kw = self.peek_token().clone();
        self.bump();
        let kind_tok = self.bump();
        let kind = match kind_tok.kind {
            TokenKind::Bulk => ModeKind::Bulk,
            TokenKind::Harmonic => ModeKind::Harmonic,
            TokenKind::Resolution => ModeKind::Resolution,
            other => {
                return Err(Diag::parse(
                    kind_tok.span,
                    format!("expected mode name (bulk/harmonic/resolution), got {:?}", other),
                ));
            }
        };
        let params = self.parse_paren_params()?;
        let ret = if self.eat(&TokenKind::Arrow) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        let body = self.parse_block()?;
        Ok(ModeDecl {
            kind,
            params,
            ret,
            span: kw.span.merge(body.span),
            body,
        })
    }

    fn parse_failure_decl(&mut self) -> Result<FailureDecl, Diag> {
        let kw = self.expect(TokenKind::OnFailure, "on_failure")?;
        let params = self.parse_paren_params()?;
        let body = self.parse_block()?;
        Ok(FailureDecl {
            params,
            span: kw.span.merge(body.span),
            body,
        })
    }

    fn parse_closure_decl(&mut self) -> Result<ClosureDecl, Diag> {
        let kw = self.expect(TokenKind::Closure, "closure")?;
        let name = self.expect_ident("closure name")?;
        self.expect(TokenKind::LBrace, "{")?;
        // v1.x-VIOLATE (F.27): the assertion is optional. If the
        // body opens with a clause leader (epoch /
        // persists_through / resets_on / captures), there is no
        // assertion. Otherwise the first item is an assertion.
        // Typecheck (not parse) enforces "assertion required
        // unless epoch inline".
        let assertion = if self.at_closure_clause_leader() {
            None
        } else {
            Some(self.parse_closure_assertion()?)
        };
        let mut clauses = Vec::new();
        while !self.at(&TokenKind::RBrace) && !matches!(self.peek(), TokenKind::Eof) {
            clauses.push(self.parse_closure_clause()?);
        }
        let close = self.expect(TokenKind::RBrace, "}")?;
        Ok(ClosureDecl {
            name,
            assertion,
            clauses,
            span: kw.span.merge(close.span),
        })
    }

    /// True when peek begins a closure-clause production
    /// (`epoch`, `persists_through`, `resets_on`, or the
    /// contextual `captures`) OR is the body's closing brace
    /// (empty body — assertion-less). Used by parse_closure_decl
    /// to decide whether the first body item is an assertion or
    /// not. Typecheck (not parse) enforces "assertion required
    /// unless epoch inline".
    fn at_closure_clause_leader(&self) -> bool {
        match self.peek() {
            TokenKind::Epoch
            | TokenKind::PersistsThrough
            | TokenKind::ResetsOn
            | TokenKind::RBrace => true,
            TokenKind::Ident(s) if s == "captures" => true,
            _ => false,
        }
    }

    fn parse_closure_assertion(&mut self) -> Result<ClosureAssertion, Diag> {
        let left = self.parse_expr()?;
        // Either ~~ or the contextual `approx` ident-keyword.
        // `approx` is intentionally NOT a lexer-level keyword
        // (it lexes as Ident) so `fn approx(...)` is admissible
        // outside closure bodies; here we recognize it by name.
        let approx_kw = matches!(
            self.peek(),
            TokenKind::Ident(s) if s == "approx"
        );
        if approx_kw {
            self.bump();
        } else if !self.eat(&TokenKind::TildeTilde) {
            return Err(Diag::parse(
                self.peek_token().span,
                "expected `~~` or `approx` in closure assertion",
            ));
        }
        let right = self.parse_expr()?;
        // `within` is the contextual tolerance-keyword. Same
        // rationale as `approx`.
        let within_kw = matches!(
            self.peek(),
            TokenKind::Ident(s) if s == "within"
        );
        if !within_kw {
            return Err(Diag::parse(
                self.peek_token().span,
                "expected `within` after closure-assertion right-hand side",
            ));
        }
        self.bump();
        let tolerance = self.parse_expr()?;
        let semi = self.expect(TokenKind::Semi, ";")?;
        Ok(ClosureAssertion {
            span: left.span().merge(semi.span),
            left,
            right,
            tolerance,
        })
    }

    fn parse_closure_clause(&mut self) -> Result<ClosureClause, Diag> {
        match self.peek().clone() {
            TokenKind::Epoch => {
                self.bump();
                let spec = self.parse_epoch_spec()?;
                self.expect(TokenKind::Semi, ";")?;
                Ok(ClosureClause::Epoch(spec))
            }
            TokenKind::PersistsThrough => {
                self.bump();
                let names = self.parse_paren_recovery_event_list()?;
                self.expect(TokenKind::Semi, ";")?;
                Ok(ClosureClause::PersistsThrough(names))
            }
            TokenKind::ResetsOn => {
                self.bump();
                let names = self.parse_paren_recovery_event_list()?;
                self.expect(TokenKind::Semi, ";")?;
                Ok(ClosureClause::ResetsOn(names))
            }
            // v1.x-VIOLATE (F.27): `captures: f1, f2, ... ;`
            // Contextual — only recognized inside a closure body
            // by peek matching on the Ident name.
            TokenKind::Ident(s) if s == "captures" => {
                self.bump(); // consume `captures` ident
                self.expect(TokenKind::Colon, ":")?;
                let mut names = Vec::new();
                names.push(self.expect_ident("captured field name")?);
                while self.eat(&TokenKind::Comma) {
                    names.push(self.expect_ident("captured field name")?);
                }
                self.expect(TokenKind::Semi, ";")?;
                Ok(ClosureClause::Captures(names))
            }
            other => Err(Diag::parse(
                self.peek_token().span,
                format!("expected closure clause, got {:?}", other),
            )),
        }
    }

    fn parse_epoch_spec(&mut self) -> Result<EpochSpec, Diag> {
        match self.peek().clone() {
            TokenKind::Ident(s) if s == "tick" => {
                self.bump();
                Ok(EpochSpec::Tick)
            }
            TokenKind::Ident(s) if s == "explicit" => {
                self.bump();
                Ok(EpochSpec::Explicit)
            }
            // v1.x-VIOLATE (F.27): `epoch inline` — pull-only,
            // fires only via `violate NAME;`. Contextual ident.
            TokenKind::Ident(s) if s == "inline" => {
                self.bump();
                Ok(EpochSpec::Inline)
            }
            TokenKind::Birth => {
                self.bump();
                Ok(EpochSpec::Birth)
            }
            TokenKind::Dissolve => {
                self.bump();
                Ok(EpochSpec::Dissolve)
            }
            TokenKind::Ident(s) if s == "duration" => {
                self.bump();
                self.expect(TokenKind::LParen, "(")?;
                let e = self.parse_expr()?;
                self.expect(TokenKind::RParen, ")")?;
                Ok(EpochSpec::Duration(e))
            }
            other => Err(Diag::parse(
                self.peek_token().span,
                format!("expected epoch spec, got {:?}", other),
            )),
        }
    }

    /// Parses a comma-separated paren list of recovery-event
    /// names. The closure clauses `persists_through(...)` and
    /// `resets_on(...)` take these names as bare keywords per
    /// the spec example `persists_through(restart_in_place,
    /// quarantine);` — each event spelling is a reserved
    /// keyword token, not a plain identifier. Each keyword
    /// surfaces here as an `Ident` whose `name` matches the
    /// keyword spelling so downstream code can match on the
    /// string.
    fn parse_paren_recovery_event_list(&mut self) -> Result<Vec<Ident>, Diag> {
        self.expect(TokenKind::LParen, "(")?;
        let mut names = Vec::new();
        if !self.at(&TokenKind::RParen) {
            names.push(self.parse_recovery_event_name()?);
            while self.eat(&TokenKind::Comma) {
                names.push(self.parse_recovery_event_name()?);
            }
        }
        self.expect(TokenKind::RParen, ")")?;
        Ok(names)
    }

    fn parse_recovery_event_name(&mut self) -> Result<Ident, Diag> {
        let tok = self.peek_token().clone();
        let name: &'static str = match tok.kind {
            TokenKind::Restart => "restart",
            TokenKind::RestartInPlace => "restart_in_place",
            TokenKind::Quarantine => "quarantine",
            TokenKind::Dissolve => "dissolve",
            _ => return self.expect_ident("recovery event name"),
        };
        self.bump();
        Ok(Ident {
            name: name.to_string(),
            span: tok.span,
        })
    }

    fn parse_perspective_decl(&mut self) -> Result<PerspectiveDecl, Diag> {
        let kw = self.expect(TokenKind::Perspective, "perspective")?;
        let name = self.expect_ident("perspective name")?;
        let generics = self.parse_generic_params_opt()?;
        self.expect(TokenKind::LBrace, "{")?;
        let mut members = Vec::new();
        while !self.at(&TokenKind::RBrace) && !matches!(self.peek(), TokenKind::Eof) {
            members.push(self.parse_perspective_member()?);
        }
        let close = self.expect(TokenKind::RBrace, "}")?;
        Ok(PerspectiveDecl {
            name,
            generics,
            members,
            span: kw.span.merge(close.span),
        })
    }

    fn parse_perspective_member(&mut self) -> Result<PerspectiveMember, Diag> {
        match self.peek() {
            TokenKind::Params => self.parse_params_block().map(PerspectiveMember::Params),
            TokenKind::StableWhen => {
                self.bump();
                let block = self.parse_block()?;
                Ok(PerspectiveMember::StableWhen(block))
            }
            TokenKind::SerializeAs => {
                self.bump();
                let ty = self.parse_type_expr()?;
                self.expect(TokenKind::Semi, ";")?;
                Ok(PerspectiveMember::SerializeAs(ty))
            }
            TokenKind::Fn => self.parse_fn_decl().map(PerspectiveMember::Fn),
            other => Err(Diag::parse(
                self.peek_token().span,
                format!("expected perspective member, got {:?}", other),
            )),
        }
    }

    fn parse_type_decl(&mut self) -> Result<TypeDecl, Diag> {
        let kw = self.expect(TokenKind::Type, "type")?;
        let name = self.expect_ident("type name")?;
        let generics = self.parse_generic_params_opt()?;

        // Three forms:
        //   type X = type_expr ;
        //   type X { struct_fields }
        //   type X = enum { variants } ;
        if self.eat(&TokenKind::Eq) {
            // alias or enum
            if self.eat(&TokenKind::Ident("enum".to_string())) {
                self.expect(TokenKind::LBrace, "{")?;
                let mut variants = Vec::new();
                if !self.at(&TokenKind::RBrace) {
                    variants.push(self.parse_enum_variant()?);
                    while self.eat(&TokenKind::Comma) {
                        if self.at(&TokenKind::RBrace) {
                            break;
                        }
                        variants.push(self.parse_enum_variant()?);
                    }
                }
                let close = self.expect(TokenKind::RBrace, "}")?;
                self.expect(TokenKind::Semi, ";")?;
                return Ok(TypeDecl {
                    name,
                    generics,
                    body: TypeDeclBody::Enum(variants),
                    span: kw.span.merge(close.span),
                });
            }
            // alias
            let ty = self.parse_type_expr()?;
            let semi = self.expect(TokenKind::Semi, ";")?;
            Ok(TypeDecl {
                name,
                generics,
                body: TypeDeclBody::Alias(ty),
                span: kw.span.merge(semi.span),
            })
        } else {
            // struct form: type X { fields }
            self.expect(TokenKind::LBrace, "{")?;
            let mut fields = Vec::new();
            while !self.at(&TokenKind::RBrace) && !matches!(self.peek(), TokenKind::Eof) {
                fields.push(self.parse_struct_field()?);
            }
            let close = self.expect(TokenKind::RBrace, "}")?;
            Ok(TypeDecl {
                name,
                generics,
                body: TypeDeclBody::Struct(fields),
                span: kw.span.merge(close.span),
            })
        }
    }

    fn parse_struct_field(&mut self) -> Result<StructField, Diag> {
        // v1.x-8: admit framework keywords as field names —
        // `type Cmd { run: fn(); birth: fn(); }` should parse.
        // Mirrors expect_member_name's post-dot admittance —
        // inside a struct-decl body the parsing position is
        // unambiguous, so reserving these words at field-name
        // position would just block useful patterns.
        let name = self.expect_member_name()?;
        self.expect(TokenKind::Colon, ":")?;
        let ty = self.parse_type_expr()?;
        let default = if self.eat(&TokenKind::Eq) {
            Some(self.parse_expr()?)
        } else {
            None
        };
        let semi = self.expect(TokenKind::Semi, ";")?;
        Ok(StructField {
            span: name.span.merge(semi.span),
            name,
            ty,
            default,
        })
    }

    fn parse_enum_variant(&mut self) -> Result<EnumVariant, Diag> {
        let name = self.expect_ident("variant name")?;
        let mut fields = Vec::new();
        if self.eat(&TokenKind::LParen) {
            if !self.at(&TokenKind::RParen) {
                fields.push(self.parse_type_expr()?);
                while self.eat(&TokenKind::Comma) {
                    fields.push(self.parse_type_expr()?);
                }
            }
            self.expect(TokenKind::RParen, ")")?;
        }
        let span = name.span;
        Ok(EnumVariant { name, fields, span })
    }

    fn parse_generic_params_opt(&mut self) -> Result<Vec<GenericParam>, Diag> {
        if !self.at(&TokenKind::Lt) {
            return Ok(Vec::new());
        }
        self.bump();
        let mut params = Vec::new();
        params.push(self.parse_generic_param()?);
        while self.eat(&TokenKind::Comma) {
            params.push(self.parse_generic_param()?);
        }
        self.expect_gt_or_split_shr()?;
        Ok(params)
    }

    fn parse_generic_param(&mut self) -> Result<GenericParam, Diag> {
        let name = self.expect_ident("generic param name")?;
        let bound = if self.eat(&TokenKind::Colon) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        let span = name.span;
        Ok(GenericParam { name, bound, span })
    }

    fn parse_const_decl(&mut self) -> Result<ConstDecl, Diag> {
        let kw = self.expect(TokenKind::Const, "const")?;
        let name = self.expect_ident("const name")?;
        self.expect(TokenKind::Colon, ":")?;
        let ty = self.parse_type_expr()?;
        self.expect(TokenKind::Eq, "=")?;
        let value = self.parse_expr()?;
        let semi = self.expect(TokenKind::Semi, ";")?;
        Ok(ConstDecl {
            span: kw.span.merge(semi.span),
            name,
            ty,
            value,
        })
    }

    fn parse_fn_decl(&mut self) -> Result<FnDecl, Diag> {
        self.parse_fn_decl_with_ffi(None)
    }

    /// Stage-1 FFI: when `ffi` is `Some(_)`, the fn declaration
    /// terminates with `;` instead of a `{ ... }` body. A
    /// synthesized empty `Block` is stored so downstream consumers
    /// (typecheck, codegen) can keep the existing `body: Block`
    /// shape; they branch on `fd.ffi.is_some()` to take the FFI
    /// code paths. Fallible markers are rejected on FFI fns —
    /// failure crosses the C-ABI boundary as an error sentinel,
    /// not via Hale's fallible channel.
    fn parse_fn_decl_with_ffi(
        &mut self,
        ffi: Option<FfiAnnotation>,
    ) -> Result<FnDecl, Diag> {
        let kw = self.expect(TokenKind::Fn, "fn")?;
        let name = self.expect_ident("function name")?;
        let generics = self.parse_generic_params_opt()?;
        self.expect(TokenKind::LParen, "(")?;
        let mut params = Vec::new();
        if !self.at(&TokenKind::RParen) {
            params.push(self.parse_param()?);
            while self.eat(&TokenKind::Comma) {
                params.push(self.parse_param()?);
            }
        }
        self.expect(TokenKind::RParen, ")")?;
        let ret = if self.eat(&TokenKind::Arrow) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        // v1.x-FORM-1: optional `fallible(T)` marker between
        // return type and body. The keyword is contextual —
        // recognized only here, as a bare ident named "fallible"
        // followed immediately by `(`. Outside this position,
        // `fallible` is an ordinary ident.
        let fallible = self.parse_fallible_marker_opt()?;
        if let Some(ffi_marker) = &ffi {
            if !generics.is_empty() {
                return Err(Diag::parse(
                    ffi_marker.span,
                    "`@ffi` fn must not be generic — the C-ABI boundary \
                     is monomorphic",
                ));
            }
            if let Some(fallible_ty) = &fallible {
                return Err(Diag::parse(
                    fallible_ty.span(),
                    "`@ffi` fn must not be `fallible(...)` — C functions \
                     return an error sentinel, the Hale wrapper above \
                     translates to `fallible(E)` if needed",
                ));
            }
            let semi = self.expect(
                TokenKind::Semi,
                "; (an `@ffi` fn declaration has no body)",
            )?;
            // Synthesize an empty block so downstream consumers
            // keep the existing `body: Block` shape. The block's
            // span points at the trailing `;` of the declaration.
            let body = Block {
                stmts: Vec::new(),
                tail: None,
                span: semi.span,
            };
            return Ok(FnDecl {
                name,
                generics,
                params,
                ret,
                fallible,
                ffi,
                span: kw.span.merge(semi.span),
                body,
            });
        }
        // Push/pop fallible-body context around the body so
        // `fail <expr>;` is recognized inside (and only inside)
        // a fallible fn's body.
        let prev_fallible = self.in_fallible_body;
        self.in_fallible_body = fallible.is_some();
        let body = self.parse_block()?;
        self.in_fallible_body = prev_fallible;
        Ok(FnDecl {
            name,
            generics,
            params,
            ret,
            fallible,
            ffi,
            span: kw.span.merge(body.span),
            body,
        })
    }

    /// v1.x-FORM-1: contextual `fallible(T)` marker after a fn's
    /// return type. Returns the payload TypeExpr when present.
    fn parse_fallible_marker_opt(&mut self) -> Result<Option<TypeExpr>, Diag> {
        let is_fallible = matches!(
            self.peek(),
            TokenKind::Ident(s) if s == "fallible"
        );
        if !is_fallible {
            return Ok(None);
        }
        // Must be followed immediately by `(` — otherwise the
        // ident is just a stray identifier and not a marker.
        if !matches!(self.peek_at(1), TokenKind::LParen) {
            return Ok(None);
        }
        self.bump(); // consume `fallible`
        self.expect(TokenKind::LParen, "(")?;
        let payload_ty = self.parse_type_expr()?;
        self.expect(TokenKind::RParen, ")")?;
        Ok(Some(payload_ty))
    }

    fn parse_module_decl(&mut self) -> Result<ModuleDecl, Diag> {
        let kw = self.expect(TokenKind::Module, "module")?;
        let name = self.expect_ident("module name")?;
        self.expect(TokenKind::LBrace, "{")?;
        let mut items = Vec::new();
        while !self.at(&TokenKind::RBrace) && !matches!(self.peek(), TokenKind::Eof) {
            items.push(self.parse_top_decl()?);
        }
        let close = self.expect(TokenKind::RBrace, "}")?;
        Ok(ModuleDecl {
            name,
            items,
            span: kw.span.merge(close.span),
        })
    }

    // === type expressions ================================

    fn parse_type_expr(&mut self) -> Result<TypeExpr, Diag> {
        let start = self.peek_token().span;
        match self.peek() {
            // Primitive type names are predefined identifiers; the
            // parser recognizes the canonical PascalCase spellings
            // and produces TypeExpr::Primitive. Lowercase / other
            // names fall through to the named-type path.
            TokenKind::Ident(name) if PRIMITIVE_TYPE_NAMES.contains(&name.as_str()) => {
                let prim = primitive_from_name(name).expect("prim_type lookup");
                self.bump();
                Ok(TypeExpr::Primitive(prim, start))
            }
            TokenKind::Rich | TokenKind::Chunked | TokenKind::Recognition => {
                let class_tok = self.bump();
                let class = match class_tok.kind {
                    TokenKind::Rich => ProjectionClass::Rich,
                    TokenKind::Chunked => ProjectionClass::Chunked,
                    // v1.x-3: `Recognition<T>` as a *type expression*
                    // (in a signature, not a locus annotation) carries
                    // no sub-mode commitment — the allocator choice
                    // lives at the locus declaration site, not the
                    // type use site. Use None here.
                    TokenKind::Recognition => ProjectionClass::Recognition(None),
                    _ => unreachable!(),
                };
                self.expect(TokenKind::Lt, "<")?;
                let inner = self.parse_type_expr()?;
                let close = self.expect_gt_or_split_shr()?;
                Ok(TypeExpr::Projection {
                    class,
                    inner: Box::new(inner),
                    span: class_tok.span.merge(close.span),
                })
            }
            TokenKind::LBracket => {
                let lb = self.bump();
                let elem = self.parse_type_expr()?;
                let size = if self.eat(&TokenKind::Semi) {
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                let rb = self.expect(TokenKind::RBracket, "]")?;
                Ok(TypeExpr::Array {
                    elem: Box::new(elem),
                    size,
                    span: lb.span.merge(rb.span),
                })
            }
            TokenKind::LParen => {
                let lp = self.bump();
                let mut elems = Vec::new();
                if !self.at(&TokenKind::RParen) {
                    elems.push(self.parse_type_expr()?);
                    while self.eat(&TokenKind::Comma) {
                        elems.push(self.parse_type_expr()?);
                    }
                }
                let rp = self.expect(TokenKind::RParen, ")")?;
                let span = lp.span.merge(rp.span);
                if elems.len() == 1 {
                    Ok(elems.pop().unwrap())
                } else {
                    Ok(TypeExpr::Tuple(elems, span))
                }
            }
            TokenKind::Ident(_) => {
                let qn = self.parse_qualified_name()?;
                let mut generic_args = Vec::new();
                let mut span = qn.span;
                if self.at(&TokenKind::Lt) {
                    self.bump();
                    generic_args.push(self.parse_type_expr()?);
                    while self.eat(&TokenKind::Comma) {
                        generic_args.push(self.parse_type_expr()?);
                    }
                    let gt = self.expect_gt_or_split_shr()?;
                    span = span.merge(gt.span);
                }
                Ok(TypeExpr::Named {
                    path: qn,
                    generic_args,
                    span,
                })
            }
            // m80: function-pointer type — `fn(T1, T2) -> R` or
            // `fn(T1, T2)` for void-returning. Parses as a
            // TypeExpr::Function the codegen layer maps to
            // CodegenTy::FnPtr.
            TokenKind::Fn => {
                let kw = self.bump();
                self.expect(TokenKind::LParen, "(")?;
                let mut params = Vec::new();
                if !self.at(&TokenKind::RParen) {
                    params.push(self.parse_type_expr()?);
                    while self.eat(&TokenKind::Comma) {
                        params.push(self.parse_type_expr()?);
                    }
                }
                let rp = self.expect(TokenKind::RParen, ")")?;
                let mut span = kw.span.merge(rp.span);
                let ret = if self.eat(&TokenKind::Arrow) {
                    let r = self.parse_type_expr()?;
                    span = span.merge(r.span());
                    Some(Box::new(r))
                } else {
                    None
                };
                Ok(TypeExpr::Function { params, ret, span })
            }
            other => Err(Diag::parse(
                self.peek_token().span,
                format!("expected type expression, got {:?}", other),
            )),
        }
    }

    fn parse_qualified_name(&mut self) -> Result<QualifiedName, Diag> {
        let first = self.expect_ident_or_kw_name("name")?;
        let mut span = first.span;
        let mut segments = vec![first];
        while self.eat(&TokenKind::ColonColon) {
            let next = self.expect_member_name()?;
            span = span.merge(next.span);
            segments.push(next);
        }
        Ok(QualifiedName { segments, span })
    }

    // === blocks / statements / expressions ===============

    fn parse_block(&mut self) -> Result<Block, Diag> {
        let lb = self.expect(TokenKind::LBrace, "{")?;
        let mut stmts = Vec::new();
        let mut tail: Option<Box<Expr>> = None;
        while !self.at(&TokenKind::RBrace) && !matches!(self.peek(), TokenKind::Eof) {
            if let Some(t) = self.parse_block_item(&mut stmts)? {
                tail = Some(Box::new(t));
                break;
            }
        }
        let rb = self.expect(TokenKind::RBrace, "}")?;
        Ok(Block {
            stmts,
            tail,
            span: lb.span.merge(rb.span),
        })
    }

    /// Parse a single item inside a block. Pushes the result onto
    /// `stmts` if it's a statement, or returns the trailing
    /// expression when an expression-led item ends without `;`
    /// immediately before the closing `}`. Keyword-led items
    /// (let / if / match / loops / control flow / nested blocks /
    /// recovery) always parse as statements — using `if` / `match`
    /// in tail position is spelled `let x = if ...` at the
    /// outer let.
    fn parse_block_item(
        &mut self,
        stmts: &mut Vec<Stmt>,
    ) -> Result<Option<Expr>, Diag> {
        // v1.x-FORM-1: `fail <expr>;` inside a fallible fn body.
        // The keyword is contextual — only triggers when the
        // parser is in a fallible-body scope. Outside that
        // scope, leading-statement `fail` Ident falls through
        // to expression parsing (so `fail();` is a call,
        // `let fail = 0;` is a binding, etc.).
        if self.in_fallible_body && self.peek_is_fail_kw() {
            stmts.push(self.parse_fail_stmt()?);
            return Ok(None);
        }
        // v1.x-VIOLATE (F.27): `violate NAME [with EXPR];`. The
        // keyword is contextual at the statement-leading
        // position. We accept it at parse-time anywhere a
        // statement is legal; typecheck enforces the rejection
        // contexts (free fn, on_failure body). This means
        // `let violate = 0;` and `violate();` (call expr) still
        // work — the violate-stmt only triggers when the *next*
        // token after `violate` is a bare ident-then-`;`/`with`,
        // distinguishing it from a function call. See
        // peek_is_violate_stmt for the disambiguation rule.
        if self.peek_is_violate_stmt() {
            stmts.push(self.parse_violate_stmt()?);
            return Ok(None);
        }
        match self.peek() {
            TokenKind::Let
            | TokenKind::If
            | TokenKind::Match
            | TokenKind::While
            | TokenKind::For
            | TokenKind::Return
            | TokenKind::Break
            | TokenKind::Continue
            | TokenKind::Yield
            | TokenKind::LBrace
            | TokenKind::Restart
            | TokenKind::RestartInPlace
            | TokenKind::Quarantine
            | TokenKind::Reorganize
            | TokenKind::Bubble => {
                stmts.push(self.parse_stmt()?);
                Ok(None)
            }
            _ => self.parse_expr_or_tail(stmts),
        }
    }

    /// True when peek is the contextual `fail` keyword in
    /// statement-leading position. Used by parse_block_item to
    /// recognize `fail <expr>;` inside a fallible fn body.
    fn peek_is_fail_kw(&self) -> bool {
        matches!(self.peek(), TokenKind::Ident(s) if s == "fail")
    }

    /// v1.x-FORM-1: `fail <expr>;`. Symmetric to `return` but
    /// exits via the error path of the enclosing fallible fn.
    /// Only reachable inside a fallible-body scope; the caller
    /// (parse_block_item) gates that.
    fn parse_fail_stmt(&mut self) -> Result<Stmt, Diag> {
        let kw = self.bump(); // consume `fail` Ident
        let value = self.parse_expr()?;
        let semi = self.expect(TokenKind::Semi, ";")?;
        Ok(Stmt::Fail {
            value,
            span: kw.span.merge(semi.span),
        })
    }

    /// True when peek is the contextual `violate` keyword at
    /// statement-leading position AND the lookahead disambiguates
    /// it from a function call. The violate-stmt grammar is
    /// `violate IDENT (`with` EXPR)? ;` — so after `violate` we
    /// require an Ident followed by either `;` (no payload) or
    /// `with` (payload). Otherwise we fall through to expression
    /// parsing so `violate();`, `violate.foo`, `let x = violate;`
    /// etc. still work.
    fn peek_is_violate_stmt(&self) -> bool {
        if !matches!(self.peek(), TokenKind::Ident(s) if s == "violate") {
            return false;
        }
        if !matches!(self.peek_at(1), TokenKind::Ident(_)) {
            return false;
        }
        match self.peek_at(2) {
            TokenKind::Semi => true,
            TokenKind::Ident(s) if s == "with" => true,
            _ => false,
        }
    }

    /// v1.x-VIOLATE (F.27): `violate NAME [with EXPR];`.
    /// Statement-level, divergent. The closure-name lookup and
    /// rejection-context enforcement happen at typecheck.
    fn parse_violate_stmt(&mut self) -> Result<Stmt, Diag> {
        let kw = self.bump(); // consume `violate` Ident
        let name = self.expect_ident("closure name after `violate`")?;
        let payload = if matches!(self.peek(), TokenKind::Ident(s) if s == "with") {
            self.bump(); // consume `with`
            Some(self.parse_expr()?)
        } else {
            None
        };
        let semi = self.expect(TokenKind::Semi, ";")?;
        Ok(Stmt::Violate {
            name,
            payload,
            span: kw.span.merge(semi.span),
        })
    }

    fn parse_stmt(&mut self) -> Result<Stmt, Diag> {
        match self.peek() {
            TokenKind::Let => self.parse_let_stmt(),
            TokenKind::If => {
                let s = self.parse_if_stmt()?;
                Ok(Stmt::If(s))
            }
            TokenKind::Match => {
                let s = self.parse_match_stmt()?;
                Ok(Stmt::Match(s))
            }
            TokenKind::While => self.parse_while_stmt(),
            TokenKind::For => self.parse_for_stmt(),
            TokenKind::Return => {
                let kw = self.bump();
                let value = if !self.at(&TokenKind::Semi) {
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                let semi = self.expect(TokenKind::Semi, ";")?;
                Ok(Stmt::Return(value, kw.span.merge(semi.span)))
            }
            TokenKind::Break => {
                let kw = self.bump();
                let semi = self.expect(TokenKind::Semi, ";")?;
                Ok(Stmt::Break(kw.span.merge(semi.span)))
            }
            TokenKind::Continue => {
                let kw = self.bump();
                let semi = self.expect(TokenKind::Semi, ";")?;
                Ok(Stmt::Continue(kw.span.merge(semi.span)))
            }
            TokenKind::Yield => {
                let kw = self.bump();
                let semi = self.expect(TokenKind::Semi, ";")?;
                Ok(Stmt::Yield(kw.span.merge(semi.span)))
            }
            TokenKind::LBrace => Ok(Stmt::Block(self.parse_block()?)),
            // Recovery primitives. m55: per The Design, the
            // vocabulary is restart / restart_in_place /
            // quarantine / bubble + reorganize. drain and
            // dissolve are lifecycle methods only — using them
            // here would overlap with bubble(err).
            TokenKind::Restart
            | TokenKind::RestartInPlace
            | TokenKind::Quarantine
            | TokenKind::Reorganize
            | TokenKind::Bubble => self.parse_recovery_stmt(),
            // Expression-led items go through parse_block_item /
            // parse_expr_or_tail at the call site so they can become
            // a trailing-tail expression instead of a stmt. Reaching
            // this fallthrough means parse_stmt was called with an
            // expression-led peek, which the dispatcher should have
            // routed elsewhere.
            other => Err(Diag::parse(
                self.peek_token().span,
                format!(
                    "internal: parse_stmt called with non-statement token {:?}",
                    other
                ),
            )),
        }
    }

    fn parse_let_stmt(&mut self) -> Result<Stmt, Diag> {
        let kw = self.expect(TokenKind::Let, "let")?;
        let is_mut = self.eat(&TokenKind::Mut);
        // Tuple destructure form: `let (a, b) = expr;`.
        if self.at(&TokenKind::LParen) {
            self.bump();
            let mut names = vec![self.expect_ident("variable name")?];
            while self.eat(&TokenKind::Comma) {
                if self.at(&TokenKind::RParen) {
                    break;
                }
                names.push(self.expect_ident("variable name")?);
            }
            self.expect(TokenKind::RParen, ")")?;
            let ty = if self.eat(&TokenKind::Colon) {
                Some(self.parse_type_expr()?)
            } else {
                None
            };
            self.expect(TokenKind::Eq, "=")?;
            let value = self.parse_expr()?;
            let semi = self.expect(TokenKind::Semi, ";")?;
            return Ok(Stmt::LetTuple {
                is_mut,
                names,
                ty,
                value,
                span: kw.span.merge(semi.span),
            });
        }
        let name = self.expect_ident("variable name")?;
        let ty = if self.eat(&TokenKind::Colon) {
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        self.expect(TokenKind::Eq, "=")?;
        let value = self.parse_expr()?;
        let semi = self.expect(TokenKind::Semi, ";")?;
        Ok(Stmt::Let {
            is_mut,
            name,
            ty,
            value,
            span: kw.span.merge(semi.span),
        })
    }

    fn parse_if_stmt(&mut self) -> Result<IfStmt, Diag> {
        let kw = self.expect(TokenKind::If, "if")?;
        let cond = self.parse_expr()?;
        let then_block = self.parse_block()?;
        let else_block = if self.eat(&TokenKind::Else) {
            if self.at(&TokenKind::If) {
                let elif = self.parse_if_stmt()?;
                Some(Box::new(ElseBranch::ElseIf(elif)))
            } else {
                let block = self.parse_block()?;
                Some(Box::new(ElseBranch::Else(block)))
            }
        } else {
            None
        };
        let span_end = match &else_block {
            Some(b) => match b.as_ref() {
                ElseBranch::Else(blk) => blk.span,
                ElseBranch::ElseIf(if_) => if_.span,
            },
            None => then_block.span,
        };
        Ok(IfStmt {
            cond,
            then_block,
            else_block,
            span: kw.span.merge(span_end),
        })
    }

    fn parse_match_stmt(&mut self) -> Result<MatchStmt, Diag> {
        let kw = self.expect(TokenKind::Match, "match")?;
        let scrutinee = self.parse_expr()?;
        self.expect(TokenKind::LBrace, "{")?;
        let mut arms = Vec::new();
        if !self.at(&TokenKind::RBrace) {
            arms.push(self.parse_match_arm()?);
            while self.eat(&TokenKind::Comma) {
                if self.at(&TokenKind::RBrace) {
                    break;
                }
                arms.push(self.parse_match_arm()?);
            }
        }
        let close = self.expect(TokenKind::RBrace, "}")?;
        Ok(MatchStmt {
            scrutinee,
            arms,
            span: kw.span.merge(close.span),
        })
    }

    fn parse_match_arm(&mut self) -> Result<MatchArm, Diag> {
        let pattern = self.parse_pattern()?;
        let guard = if self.eat(&TokenKind::If) {
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.expect(TokenKind::Arrow, "->")?;
        let body = if self.at(&TokenKind::LBrace) {
            MatchArmBody::Block(self.parse_block()?)
        } else {
            let e = self.parse_expr()?;
            self.eat(&TokenKind::Semi);
            MatchArmBody::Expr(e)
        };
        let span = match &body {
            MatchArmBody::Expr(e) => e.span(),
            MatchArmBody::Block(b) => b.span,
        };
        Ok(MatchArm {
            pattern,
            guard,
            body,
            span,
        })
    }

    fn parse_pattern(&mut self) -> Result<Pattern, Diag> {
        let span = self.peek_token().span;
        match self.peek().clone() {
            TokenKind::IntLit(n) => {
                self.bump();
                Ok(Pattern::Literal(Literal::Int(n), span))
            }
            TokenKind::FloatLit(f) => {
                self.bump();
                Ok(Pattern::Literal(Literal::Float(f), span))
            }
            TokenKind::DecimalLit(s) => {
                self.bump();
                Ok(Pattern::Literal(Literal::Decimal(s), span))
            }
            TokenKind::DurationLit(ns) => {
                self.bump();
                Ok(Pattern::Literal(Literal::Duration(ns), span))
            }
            TokenKind::StringLit(s) => {
                self.bump();
                Ok(Pattern::Literal(Literal::String(s), span))
            }
            TokenKind::True => {
                self.bump();
                Ok(Pattern::Literal(Literal::Bool(true), span))
            }
            TokenKind::False => {
                self.bump();
                Ok(Pattern::Literal(Literal::Bool(false), span))
            }
            TokenKind::Nil => {
                self.bump();
                Ok(Pattern::Literal(Literal::Nil, span))
            }
            TokenKind::Ident(name) if name == "_" => {
                self.bump();
                Ok(Pattern::Wildcard(span))
            }
            TokenKind::Ident(_) => {
                let path = self.parse_qualified_name()?;
                if self.eat(&TokenKind::LParen) {
                    let mut args = Vec::new();
                    if !self.at(&TokenKind::RParen) {
                        args.push(self.parse_pattern()?);
                        while self.eat(&TokenKind::Comma) {
                            args.push(self.parse_pattern()?);
                        }
                    }
                    let close = self.expect(TokenKind::RParen, ")")?;
                    Ok(Pattern::Constructor {
                        span: path.span.merge(close.span),
                        path,
                        args,
                    })
                } else if path.segments.len() == 1 {
                    Ok(Pattern::Binding(path.segments.into_iter().next().unwrap()))
                } else {
                    Ok(Pattern::Constructor {
                        path: path.clone(),
                        args: Vec::new(),
                        span: path.span,
                    })
                }
            }
            TokenKind::LParen => {
                let lp = self.bump();
                let mut elems = Vec::new();
                if !self.at(&TokenKind::RParen) {
                    elems.push(self.parse_pattern()?);
                    while self.eat(&TokenKind::Comma) {
                        elems.push(self.parse_pattern()?);
                    }
                }
                let rp = self.expect(TokenKind::RParen, ")")?;
                Ok(Pattern::Tuple(elems, lp.span.merge(rp.span)))
            }
            other => Err(Diag::parse(
                span,
                format!("expected pattern, got {:?}", other),
            )),
        }
    }

    fn parse_while_stmt(&mut self) -> Result<Stmt, Diag> {
        let kw = self.expect(TokenKind::While, "while")?;
        let cond = self.parse_expr()?;
        let body = self.parse_block()?;
        Ok(Stmt::While {
            cond,
            span: kw.span.merge(body.span),
            body,
        })
    }

    fn parse_for_stmt(&mut self) -> Result<Stmt, Diag> {
        let kw = self.expect(TokenKind::For, "for")?;
        let name = self.expect_ident("loop variable")?;
        self.expect(TokenKind::In, "in")?;
        let iter = self.parse_expr()?;
        let body = self.parse_block()?;
        Ok(Stmt::For {
            name,
            iter,
            span: kw.span.merge(body.span),
            body,
        })
    }

    fn parse_recovery_stmt(&mut self) -> Result<Stmt, Diag> {
        let kw_tok = self.bump();
        let op = match kw_tok.kind {
            TokenKind::Restart => RecoveryOp::Restart,
            TokenKind::RestartInPlace => RecoveryOp::RestartInPlace,
            TokenKind::Quarantine => RecoveryOp::Quarantine,
            TokenKind::Reorganize => RecoveryOp::Reorganize,
            TokenKind::Bubble => RecoveryOp::Bubble,
            _ => unreachable!(),
        };
        self.expect(TokenKind::LParen, "(")?;
        let mut args = Vec::new();
        if !self.at(&TokenKind::RParen) {
            args.push(self.parse_expr()?);
            while self.eat(&TokenKind::Comma) {
                args.push(self.parse_expr()?);
            }
        }
        let _rp = self.expect(TokenKind::RParen, ")")?;
        let modifier = if self.peek_is_kw_for() {
            self.bump();
            Some(RecoveryModifier::For(self.parse_expr()?))
        } else if self.peek_is_kw_until() {
            self.bump();
            Some(RecoveryModifier::Until(self.parse_expr()?))
        } else {
            None
        };
        let semi = self.expect(TokenKind::Semi, ";")?;
        Ok(Stmt::Recovery {
            op,
            args,
            modifier,
            span: kw_tok.span.merge(semi.span),
        })
    }

    fn peek_is_kw_for(&self) -> bool {
        matches!(self.peek(), TokenKind::For)
    }

    fn peek_is_kw_until(&self) -> bool {
        matches!(self.peek(), TokenKind::Ident(s) if s == "until")
    }

    /// Expression-led block-item: returns Some(expr) if the item turned
    /// out to be the block's trailing tail (no `;`, immediately before
    /// `}`); otherwise pushes a Send / Assign / Expr stmt onto `stmts`.
    fn parse_expr_or_tail(
        &mut self,
        stmts: &mut Vec<Stmt>,
    ) -> Result<Option<Expr>, Diag> {
        let expr = self.parse_expr()?;
        if matches!(self.peek(), TokenKind::LeftArrow) {
            self.bump();
            let value = self.parse_expr()?;
            let semi = self.expect(TokenKind::Semi, ";")?;
            stmts.push(Stmt::Send {
                span: expr.span().merge(semi.span),
                subject: expr,
                value,
            });
            return Ok(None);
        }
        if let Some(op) = self.peek_assign_op() {
            let op_span = self.peek_token().span;
            self.bump();
            let value = self.parse_expr()?;
            let semi = self.expect(TokenKind::Semi, ";")?;
            let target = expr_to_lvalue(expr, op_span)?;
            stmts.push(Stmt::Assign {
                span: target.span.merge(semi.span),
                target,
                op,
                value,
            });
            return Ok(None);
        }
        if self.eat(&TokenKind::Semi) {
            stmts.push(Stmt::Expr(expr));
            return Ok(None);
        }
        if self.at(&TokenKind::RBrace) {
            return Ok(Some(expr));
        }
        // Neither `;` nor `}` — same error shape as the prior
        // `expect(Semi)` so error messages stay consistent.
        let _ = self.expect(TokenKind::Semi, ";")?;
        unreachable!("expect(Semi) at non-Semi non-RBrace token must error")
    }

    fn peek_assign_op(&self) -> Option<AssignOp> {
        Some(match self.peek() {
            TokenKind::Eq => AssignOp::Eq,
            TokenKind::PlusEq => AssignOp::PlusEq,
            TokenKind::MinusEq => AssignOp::MinusEq,
            TokenKind::StarEq => AssignOp::StarEq,
            TokenKind::SlashEq => AssignOp::SlashEq,
            TokenKind::PercentEq => AssignOp::PercentEq,
            TokenKind::AmpEq => AssignOp::AmpEq,
            TokenKind::PipeEq => AssignOp::PipeEq,
            TokenKind::CaretEq => AssignOp::CaretEq,
            _ => return None,
        })
    }

    // === expressions: Pratt parser =======================

    fn parse_expr(&mut self) -> Result<Expr, Diag> {
        let lhs = self.parse_expr_bp(0)?;
        // Range operator binds at the lowest precedence: `for i in
        // 0 .. n + 1` should parse as `0 .. (n + 1)`. v0 only
        // surfaces ranges in for-loop iterator position; the
        // typechecker / codegen rejects them elsewhere.
        let inclusive = if self.eat(&TokenKind::DotDot) {
            false
        } else if self.eat(&TokenKind::DotDotEq) {
            true
        } else {
            // No range; check for the v1.x-FORM-1 `or`-disposition
            // postfix instead. Right-associative so
            // `a() or b() or raise` chains correctly.
            return self.parse_or_disposition_tail(lhs);
        };
        let rhs = self.parse_expr_bp(0)?;
        let span = lhs.span().merge(rhs.span());
        Ok(Expr::Range {
            lo: Box::new(lhs),
            hi: Box::new(rhs),
            inclusive,
            span,
        })
    }

    /// v1.x-FORM-1: `<expr> or <disposition>` postfix. Addresses
    /// a fallible call site's error. The `or` keyword is
    /// contextual — recognized in this position only (outside
    /// of it, `or` lexes and parses as an ordinary identifier).
    /// Right-associative: a chain `a() or b() or raise` parses
    /// as `a() or (b() or raise)`.
    ///
    /// `raise` is also contextual — only as the immediate RHS
    /// of `or`. Outside that position, `raise` is an ordinary
    /// identifier.
    fn parse_or_disposition_tail(&mut self, lhs: Expr) -> Result<Expr, Diag> {
        let is_or = matches!(self.peek(), TokenKind::Ident(s) if s == "or");
        if !is_or {
            return Ok(lhs);
        }
        self.bump(); // consume `or`
        let is_raise = matches!(self.peek(), TokenKind::Ident(s) if s == "raise");
        let is_discard = matches!(self.peek(), TokenKind::Ident(s) if s == "discard");
        // B3 / G6 — `or fail <payload>` as an or_clause RHS. `fail`
        // is a contextual ident (same narrowing pattern as `raise`
        // / `discard`); recognized here in expression position.
        let is_fail = matches!(self.peek(), TokenKind::Ident(s) if s == "fail");
        let (disposition, end_span) = if is_raise {
            let raise_tok = self.bump();
            (OrDisposition::Raise(raise_tok.span), raise_tok.span)
        } else if is_discard {
            let discard_tok = self.bump();
            (OrDisposition::Discard(discard_tok.span), discard_tok.span)
        } else if is_fail {
            let fail_tok = self.bump();
            let payload = self.parse_expr()?;
            let span = fail_tok.span.merge(payload.span());
            (OrDisposition::Fail(Box::new(payload), span), span)
        } else {
            // Substitute: RHS is itself a full expression (which
            // may chain another `or` — that's how we get
            // right-associativity).
            let rhs = self.parse_expr()?;
            let rhs_span = rhs.span();
            (OrDisposition::Substitute(Box::new(rhs)), rhs_span)
        };
        let span = lhs.span().merge(end_span);
        Ok(Expr::Or {
            inner: Box::new(lhs),
            disposition,
            span,
        })
    }

    /// Pratt-style parse with a minimum binding power. Returns
    /// when the next operator's left binding power is less than
    /// `min_bp`.
    fn parse_expr_bp(&mut self, min_bp: u8) -> Result<Expr, Diag> {
        let mut lhs = self.parse_unary()?;

        loop {
            let op = match self.peek_binop() {
                Some(op) => op,
                None => break,
            };
            let (l_bp, r_bp) = bin_op_bp(op);
            if l_bp < min_bp {
                break;
            }
            // Non-associative ops disallow chaining at same level.
            self.bump();
            let rhs = self.parse_expr_bp(r_bp)?;
            let span = lhs.span().merge(rhs.span());
            lhs = Expr::Binary {
                op,
                left: Box::new(lhs),
                right: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, Diag> {
        match self.peek() {
            TokenKind::Minus => {
                let kw = self.bump();
                let operand = self.parse_unary()?;
                Ok(Expr::Unary {
                    op: UnaryOp::Neg,
                    span: kw.span.merge(operand.span()),
                    operand: Box::new(operand),
                })
            }
            TokenKind::Bang => {
                let kw = self.bump();
                let operand = self.parse_unary()?;
                Ok(Expr::Unary {
                    op: UnaryOp::Not,
                    span: kw.span.merge(operand.span()),
                    operand: Box::new(operand),
                })
            }
            TokenKind::Tilde => {
                let kw = self.bump();
                let operand = self.parse_unary()?;
                Ok(Expr::Unary {
                    op: UnaryOp::BitNot,
                    span: kw.span.merge(operand.span()),
                    operand: Box::new(operand),
                })
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, Diag> {
        let mut expr = self.parse_primary()?;
        loop {
            match self.peek() {
                TokenKind::Dot => {
                    self.bump();
                    // Allow numeric tuple-field access: `t.0`,
                    // `t.1`, etc. The token shows up as IntLit;
                    // we treat it as an Ident with the digit
                    // string as the name, so codegen / typecheck
                    // can detect tuple-shaped field access by
                    // checking `name.parse::<usize>()`.
                    let name = if let TokenKind::IntLit(n) = self.peek().clone() {
                        let span = self.peek_token().span;
                        self.bump();
                        Ident { name: n.to_string(), span }
                    } else {
                        self.expect_member_name()?
                    };
                    let span = expr.span().merge(name.span);
                    expr = Expr::Field {
                        receiver: Box::new(expr),
                        name,
                        span,
                    };
                }
                TokenKind::ColonColon => {
                    self.bump();
                    let name = self.expect_member_name()?;
                    let span = expr.span().merge(name.span);
                    expr = Expr::Path2 {
                        receiver: Box::new(expr),
                        name,
                        span,
                    };
                }
                TokenKind::LParen => {
                    let lp = self.bump();
                    let mut args = Vec::new();
                    if !self.at(&TokenKind::RParen) {
                        args.push(self.parse_expr()?);
                        while self.eat(&TokenKind::Comma) {
                            args.push(self.parse_expr()?);
                        }
                    }
                    let rp = self.expect(TokenKind::RParen, ")")?;
                    let span = expr.span().merge(rp.span);
                    expr = Expr::Call {
                        callee: Box::new(expr),
                        args,
                        span,
                    };
                    let _ = lp;
                }
                TokenKind::LBracket => {
                    let lb = self.bump();
                    let index = self.parse_expr()?;
                    let rb = self.expect(TokenKind::RBracket, "]")?;
                    let span = expr.span().merge(rb.span);
                    expr = Expr::Index {
                        receiver: Box::new(expr),
                        index: Box::new(index),
                        span,
                    };
                    let _ = lb;
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, Diag> {
        let span = self.peek_token().span;
        match self.peek().clone() {
            TokenKind::IntLit(n) => {
                self.bump();
                Ok(Expr::Literal(Literal::Int(n), span))
            }
            TokenKind::FloatLit(f) => {
                self.bump();
                Ok(Expr::Literal(Literal::Float(f), span))
            }
            TokenKind::DecimalLit(s) => {
                self.bump();
                Ok(Expr::Literal(Literal::Decimal(s), span))
            }
            TokenKind::StringLit(s) => {
                self.bump();
                Ok(Expr::Literal(Literal::String(s), span))
            }
            TokenKind::FStringLit(parts) => {
                self.bump();
                self.lower_fstring_parts(parts, span)
            }
            TokenKind::True => {
                self.bump();
                Ok(Expr::Literal(Literal::Bool(true), span))
            }
            TokenKind::False => {
                self.bump();
                Ok(Expr::Literal(Literal::Bool(false), span))
            }
            TokenKind::Nil => {
                self.bump();
                Ok(Expr::Literal(Literal::Nil, span))
            }
            TokenKind::DurationLit(d) => {
                self.bump();
                Ok(Expr::Literal(Literal::Duration(d), span))
            }
            TokenKind::TimeLit(s) => {
                self.bump();
                Ok(Expr::Literal(Literal::Time(s), span))
            }
            TokenKind::BytesLit(b) => {
                self.bump();
                Ok(Expr::Literal(Literal::Bytes(b), span))
            }
            TokenKind::KwSelf => {
                self.bump();
                Ok(Expr::KwSelf(span))
            }
            TokenKind::LParen => {
                self.bump();
                let mut elems = Vec::new();
                elems.push(self.parse_expr()?);
                let mut is_tuple = false;
                while self.eat(&TokenKind::Comma) {
                    is_tuple = true;
                    if self.at(&TokenKind::RParen) {
                        break;
                    }
                    elems.push(self.parse_expr()?);
                }
                let close = self.expect(TokenKind::RParen, ")")?;
                if is_tuple {
                    Ok(Expr::Tuple(elems, span.merge(close.span)))
                } else {
                    Ok(elems.into_iter().next().unwrap())
                }
            }
            TokenKind::LBracket => {
                self.bump();
                let mut elems = Vec::new();
                if !self.at(&TokenKind::RBracket) {
                    let first = self.parse_expr()?;
                    // `[val; N]` repetition form. The semicolon
                    // disambiguates from the comma-list shape.
                    // N must be a const Int literal at v0 — no
                    // const-eval engine yet; that's a follow-up
                    // when a workload needs computed sizes.
                    if self.eat(&TokenKind::Semi) {
                        let count_expr = self.parse_expr()?;
                        let count = match &count_expr {
                            Expr::Literal(Literal::Int(n), _) if *n >= 0 => {
                                *n as u64
                            }
                            _ => {
                                return Err(Diag::parse(
                                    count_expr.span(),
                                    "array-repeat count must be a non-negative integer literal at v0",
                                ));
                            }
                        };
                        let close = self.expect(TokenKind::RBracket, "]")?;
                        return Ok(Expr::ArrayRepeat {
                            val: Box::new(first),
                            count,
                            span: span.merge(close.span),
                        });
                    }
                    elems.push(first);
                    while self.eat(&TokenKind::Comma) {
                        // Allow a trailing comma — the multi-line
                        // form is idiomatic. Mirrors the same
                        // shape adapter/struct_init blocks already
                        // accept.
                        if self.at(&TokenKind::RBracket) {
                            break;
                        }
                        elems.push(self.parse_expr()?);
                    }
                }
                let close = self.expect(TokenKind::RBracket, "]")?;
                Ok(Expr::Array(elems, span.merge(close.span)))
            }
            TokenKind::LBrace => {
                let block = self.parse_block()?;
                Ok(Expr::Block(block))
            }
            TokenKind::If => {
                let if_ = self.parse_if_stmt()?;
                Ok(Expr::If(Box::new(if_)))
            }
            TokenKind::Match => {
                let m = self.parse_match_stmt()?;
                Ok(Expr::Match(Box::new(m)))
            }
            // (Previously this dispatch had a fallthrough for
            // primitive-type keywords and bus keywords used as
            // expression-position identifiers. After the
            // capitalize-types + bus-send-operator refactor,
            // primitive types are predefined identifiers and
            // `publish`/`subscribe` are no longer used as
            // expression-position names. Mode keywords appear
            // only post-dot, never as expression heads.)

            // Identifier — might be ident, path, struct expression, or call.
            TokenKind::Ident(name) => {
                // Look-ahead for "sum(" or "prod(" to recognize as builtins.
                if name == "sum" && matches!(self.peek_at(1), TokenKind::LParen) {
                    self.bump();
                    self.expect(TokenKind::LParen, "(")?;
                    let inner = self.parse_expr()?;
                    let close = self.expect(TokenKind::RParen, ")")?;
                    return Ok(Expr::Sum(Box::new(inner), span.merge(close.span)));
                }
                if name == "prod" && matches!(self.peek_at(1), TokenKind::LParen) {
                    self.bump();
                    self.expect(TokenKind::LParen, "(")?;
                    let inner = self.parse_expr()?;
                    let close = self.expect(TokenKind::RParen, ")")?;
                    return Ok(Expr::Prod(Box::new(inner), span.merge(close.span)));
                }
                let qn = self.parse_qualified_name()?;
                // Struct literal: NAME { fields }
                // We must distinguish from block expressions; treat
                // `IDENT {` followed by struct-init shape as struct.
                if self.at(&TokenKind::LBrace) && self.looks_like_struct_lit() {
                    return self.parse_struct_literal(qn);
                }
                if qn.segments.len() == 1 {
                    Ok(Expr::Ident(qn.segments.into_iter().next().unwrap()))
                } else {
                    Ok(Expr::Path(qn))
                }
            }
            other => Err(Diag::parse(
                span,
                format!("expected expression, got {:?}", other),
            )),
        }
    }

    fn looks_like_struct_lit(&self) -> bool {
        // We've just parsed an ident path and see `{`. This could be
        // a struct literal or a block. We use a simple heuristic:
        // a `{` here in expression position is a struct literal if
        // either the brace is followed by `}` (empty struct) or the
        // next non-trivial token is IDENT and then `:`.
        if !matches!(self.peek(), TokenKind::LBrace) {
            return false;
        }
        // Peek inside the brace.
        match self.peek_at(1) {
            TokenKind::RBrace => true,
            TokenKind::Ident(_) => matches!(self.peek_at(2), TokenKind::Colon),
            _ => false,
        }
    }

    fn parse_struct_literal(&mut self, qn: QualifiedName) -> Result<Expr, Diag> {
        let lb = self.expect(TokenKind::LBrace, "{")?;
        let mut inits = Vec::new();
        if !self.at(&TokenKind::RBrace) {
            inits.push(self.parse_struct_init()?);
            while self.eat(&TokenKind::Comma) {
                if self.at(&TokenKind::RBrace) {
                    break;
                }
                inits.push(self.parse_struct_init()?);
            }
        }
        let close = self.expect(TokenKind::RBrace, "}")?;
        let _ = lb;
        Ok(Expr::Struct {
            span: qn.span.merge(close.span),
            path: qn,
            inits,
        })
    }

    fn parse_struct_init(&mut self) -> Result<StructInit, Diag> {
        // v1.x-8: parity with parse_struct_field — admit
        // framework keywords as field names in struct literals
        // so `Cmd { run: my_fn }` parses.
        let name = self.expect_member_name()?;
        self.expect(TokenKind::Colon, ":")?;
        let value = self.parse_expr()?;
        let span = name.span.merge(value.span());
        Ok(StructInit { name, value, span })
    }

    fn peek_binop(&self) -> Option<BinOp> {
        Some(match self.peek() {
            TokenKind::Plus => BinOp::Add,
            TokenKind::Minus => BinOp::Sub,
            TokenKind::Star => BinOp::Mul,
            TokenKind::Slash => BinOp::Div,
            TokenKind::Percent => BinOp::Mod,
            TokenKind::EqEq => BinOp::Eq,
            TokenKind::NotEq => BinOp::NotEq,
            TokenKind::Lt => BinOp::Lt,
            TokenKind::Gt => BinOp::Gt,
            TokenKind::LtEq => BinOp::LtEq,
            TokenKind::GtEq => BinOp::GtEq,
            TokenKind::AndAnd => BinOp::And,
            TokenKind::OrOr => BinOp::Or,
            TokenKind::Amp => BinOp::BitAnd,
            TokenKind::Pipe => BinOp::BitOr,
            TokenKind::Caret => BinOp::BitXor,
            TokenKind::Shl => BinOp::Shl,
            TokenKind::Shr => BinOp::Shr,
            _ => return None,
        })
    }

    /// v1.x-10: lower a pre-split f-string into a chain of
    /// `Lit + to_string(expr) + Lit + ...` concatenations.
    ///
    /// Each `Interp(body)` substring is re-lexed + re-parsed as an
    /// Hale expression via a fresh inner parser. That lets `f"{a + b * 2}"`
    /// and `f"{user.name}"` work without growing this routine.
    /// Empty bodies are rejected at the lexer; an Interp here is
    /// always non-empty.
    fn lower_fstring_parts(
        &mut self,
        parts: Vec<FStringPart>,
        span: Span,
    ) -> Result<Expr, Diag> {
        let mut pieces: Vec<Expr> = Vec::new();
        for part in parts {
            match part {
                FStringPart::Lit(s) => {
                    if !s.is_empty() {
                        pieces.push(Expr::Literal(Literal::String(s), span));
                    }
                }
                FStringPart::Interp(body) => {
                    // Sub-parse the interpolation body as an expression.
                    let tokens = crate::lexer::lex(&body).map_err(|diags| {
                        let msg = diags
                            .iter()
                            .map(|d| d.message.clone())
                            .collect::<Vec<_>>()
                            .join("; ");
                        Diag::parse(
                            span,
                            format!("f-string interpolation `{{{}}}`: {}", body, msg),
                        )
                    })?;
                    let mut sub = Parser::new(tokens);
                    let expr = sub.parse_expr().map_err(|d| {
                        Diag::parse(
                            span,
                            format!(
                                "f-string interpolation `{{{}}}`: {}",
                                body, d.message
                            ),
                        )
                    })?;
                    // Reject trailing tokens — interpolation must be one expr.
                    if !sub.at_eof() {
                        return Err(Diag::parse(
                            span,
                            format!(
                                "f-string interpolation `{{{}}}`: unexpected trailing tokens",
                                body
                            ),
                        ));
                    }
                    // Wrap in `to_string(expr)` so any printable
                    // type renders the same way println would render it.
                    let to_str_callee = Expr::Ident(Ident {
                        name: "to_string".to_string(),
                        span,
                    });
                    pieces.push(Expr::Call {
                        callee: Box::new(to_str_callee),
                        args: vec![expr],
                        span,
                    });
                }
            }
        }

        // Ensure there is at least one String piece so the type of
        // the whole expression is String — `f""` → "", `f"{x}"` →
        // "" + to_string(x).
        if pieces.is_empty()
            || !matches!(&pieces[0], Expr::Literal(Literal::String(_), _))
        {
            pieces.insert(
                0,
                Expr::Literal(Literal::String(String::new()), span),
            );
        }

        // Fold via left-associative `+`.
        let mut iter = pieces.into_iter();
        let mut acc = iter.next().unwrap();
        for next in iter {
            acc = Expr::Binary {
                op: BinOp::Add,
                left: Box::new(acc),
                right: Box::new(next),
                span,
            };
        }
        Ok(acc)
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), TokenKind::Eof)
    }
}

/// Convert an expression we just parsed (in a stmt position) to an
/// LValue if possible; else error.
///
/// `self.x = ...` is supported by treating `self` as the LValue head
/// (synthetic Ident with the name "self"). The downstream type
/// checker is responsible for resolving the synthetic head against
/// the enclosing locus's params.
fn expr_to_lvalue(expr: Expr, op_span: Span) -> Result<LValue, Diag> {
    match expr {
        Expr::Ident(i) => Ok(LValue {
            span: i.span,
            head: i,
            tail: Vec::new(),
        }),
        Expr::KwSelf(span) => {
            // A bare `self = ...` is rejected; `self` must be
            // followed by at least one field access. We surface this
            // when no tail is built up.
            Err(Diag::parse(
                op_span,
                "cannot assign to `self` directly; use `self.field = ...`",
            ))
            .map_err(|d| {
                let _ = span;
                d
            })
        }
        Expr::Field { receiver, name, .. } => {
            // Accept `self.field = ...` by synthesizing a
            // self-headed LValue.
            let mut lv = match *receiver {
                Expr::KwSelf(s) => LValue {
                    head: Ident {
                        name: "self".to_string(),
                        span: s,
                    },
                    tail: Vec::new(),
                    span: s,
                },
                other => expr_to_lvalue(other, op_span)?,
            };
            let span = lv.span.merge(name.span);
            lv.tail.push(LValueSeg::Field(name));
            lv.span = span;
            Ok(lv)
        }
        Expr::Index { receiver, index, span } => {
            let mut lv = match *receiver {
                Expr::KwSelf(s) => LValue {
                    head: Ident {
                        name: "self".to_string(),
                        span: s,
                    },
                    tail: Vec::new(),
                    span: s,
                },
                other => expr_to_lvalue(other, op_span)?,
            };
            lv.tail.push(LValueSeg::Index(*index));
            lv.span = lv.span.merge(span);
            Ok(lv)
        }
        other => Err(Diag::parse(
            other.span(),
            "expression is not assignable",
        )),
    }
}

/// Pratt binding powers for binary operators. Higher = tighter
/// binding. Returns (left, right) bp. For non-associative ops we
/// return (n, n+1) to disallow chaining at the same level.
fn bin_op_bp(op: BinOp) -> (u8, u8) {
    use BinOp::*;
    match op {
        Or => (3, 4),
        And => (5, 6),
        Eq | NotEq => (7, 7),       // non-assoc
        Lt | Gt | LtEq | GtEq => (9, 9), // non-assoc
        BitOr => (11, 12),
        BitXor => (13, 14),
        BitAnd => (15, 16),
        Shl | Shr => (17, 18),
        Add | Sub => (19, 20),
        Mul | Div | Mod => (21, 22),
    }
}

/// If the given keyword token is one we permit as an identifier
/// in expression / path / member position, return its textual
/// name. Otherwise None.
///
/// As of v0.2 (capitalize-types + bus-send-operator refactor), the
/// only keywords still in this fallback are mode names — they need
/// to be available as member names per F.10 (e.g., `self.bulk()`).
/// Primitive types are no longer keywords (PascalCase predefined
/// identifiers); `publish`/`subscribe` are no longer overloaded
/// (the runtime publish action is the `<-` operator, not a builtin).
fn try_keyword_as_name(k: &TokenKind) -> Option<&'static str> {
    Some(match k {
        TokenKind::Bulk => "bulk",
        TokenKind::Harmonic => "harmonic",
        TokenKind::Resolution => "resolution",
        _ => return None,
    })
}

/// Broader keyword-as-name set permitted in member-name
/// position (post-`.`). The post-dot position is
/// unambiguous, so it's safe to admit framework-vocabulary
/// keywords here as field names. This unlocks fields like
/// `err.closure`, `err.locus`, `err.params` on built-in
/// struct values without renaming.
fn try_member_keyword_as_name(k: &TokenKind) -> Option<&'static str> {
    if let Some(name) = try_keyword_as_name(k) {
        return Some(name);
    }
    Some(match k {
        TokenKind::Closure => "closure",
        TokenKind::Locus => "locus",
        TokenKind::Params => "params",
        TokenKind::Contract => "contract",
        TokenKind::Bus => "bus",
        TokenKind::Capacity => "capacity",
        TokenKind::Tier => "tier",
        TokenKind::Projection => "projection",
        TokenKind::Perspective => "perspective",
        TokenKind::Type => "type",
        // v1.x-8: lifecycle keywords admissible as field names
        // inside type decls + struct literals. Friction:
        // `type Cmd { run: fn(CliCtxL); }` couldn't parse
        // because `run` lexed as TokenKind::Run.
        TokenKind::Birth => "birth",
        TokenKind::Accept => "accept",
        TokenKind::Run => "run",
        TokenKind::Drain => "drain",
        TokenKind::Dissolve => "dissolve",
        _ => return None,
    })
}

/// Canonical primitive type names. Recognized in type position;
/// unreserved elsewhere.
pub const PRIMITIVE_TYPE_NAMES: &[&str] = &[
    "Int", "Uint", "Float", "Decimal", "String", "Bool", "Time",
    "Duration", "Bytes", "BytesView", "StringView",
];

fn primitive_from_name(name: &str) -> Option<PrimType> {
    Some(match name {
        "Int" => PrimType::Int,
        "Uint" => PrimType::Uint,
        "Float" => PrimType::Float,
        "Decimal" => PrimType::Decimal,
        "String" => PrimType::String,
        "Bool" => PrimType::Bool,
        "Time" => PrimType::Time,
        "Duration" => PrimType::Duration,
        "Bytes" => PrimType::Bytes,
        "BytesView" => PrimType::BytesView,
        "StringView" => PrimType::StringView,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    fn parse_str(s: &str) -> Result<Program, Vec<Diag>> {
        let tokens = lex(s)?;
        parse(tokens, s)
    }

    #[test]
    fn parse_hello_world() {
        let src = r#"
locus HelloL {
    params {
        greeting: string = "hello, world";
    }
    birth() {
        println(self.greeting);
    }
}

fn main() {
    HelloL { };
}
"#;
        let prog = parse_str(src).expect("parse failed");
        assert_eq!(prog.items.len(), 2);
        match &prog.items[0] {
            TopDecl::Locus(l) => {
                assert_eq!(l.name.name, "HelloL");
                assert_eq!(l.members.len(), 2);
            }
            _ => panic!("expected locus"),
        }
        match &prog.items[1] {
            TopDecl::Fn(f) => {
                assert_eq!(f.name.name, "main");
            }
            _ => panic!("expected fn"),
        }
    }

    // === v1.x-FORM-1 PR1 tests ============================

    #[test]
    fn parse_form_annotation_no_args() {
        let src = r#"
@form(vec)
locus ItemListL {
    capacity { heap items of Int; }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        match &prog.items[0] {
            TopDecl::Locus(l) => {
                let form = l.form.as_ref().expect("expected form annotation");
                assert_eq!(form.name.name, "vec");
                assert!(form.args.is_empty());
            }
            _ => panic!("expected locus"),
        }
    }

    #[test]
    fn parse_form_annotation_with_args() {
        let src = r#"
@form(ring_buffer, cap = 64)
locus RecentL {
    capacity { pool history of Int; }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        match &prog.items[0] {
            TopDecl::Locus(l) => {
                let form = l.form.as_ref().expect("expected form annotation");
                assert_eq!(form.name.name, "ring_buffer");
                assert_eq!(form.args.len(), 1);
                assert_eq!(form.args[0].name.name, "cap");
            }
            _ => panic!("expected locus"),
        }
    }

    #[test]
    fn parse_form_rejects_non_locus_target() {
        let src = r#"
@form(vec)
fn not_a_locus() { }
"#;
        let err = parse_str(src).expect_err("expected parse error");
        let msg = format!("{:?}", err);
        assert!(
            msg.contains("expected `locus`"),
            "wrong error: {}",
            msg
        );
    }

    #[test]
    fn parse_form_rejects_unknown_at_keyword() {
        let src = r#"
@derive(vec)
locus L { }
"#;
        let err = parse_str(src).expect_err("expected parse error");
        let msg = format!("{:?}", err);
        assert!(
            msg.contains("expected `form`"),
            "wrong error: {}",
            msg
        );
    }

    // === F.32-2 v0.2 @locality annotation tests ===========

    #[test]
    fn parse_locality_annotation_each_tier() {
        for (spelling, want) in [
            ("L1", LocalityTier::L1),
            ("L2", LocalityTier::L2),
            ("L3", LocalityTier::L3),
            ("any", LocalityTier::Any),
        ] {
            let src = format!(
                "@locality({spelling}) locus L {{ }}",
                spelling = spelling
            );
            let prog = parse_str(&src).expect("parse failed");
            match &prog.items[0] {
                TopDecl::Locus(l) => {
                    let loc = l
                        .locality
                        .as_ref()
                        .unwrap_or_else(|| panic!("expected locality for {}", spelling));
                    assert_eq!(loc.tier, want, "wrong tier for {}", spelling);
                }
                _ => panic!("expected locus for {}", spelling),
            }
        }
    }

    #[test]
    fn parse_locality_stacks_with_form() {
        let src = r#"
type Entry { k: Int; v: Int; }
@form(hashmap, sync = lockfree, cap = 64)
@locality(L2)
locus Reg {
    capacity { pool entries of Entry indexed_by k; }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        match &prog.items[1] {
            TopDecl::Locus(l) => {
                assert!(l.form.is_some(), "expected form");
                assert_eq!(
                    l.locality.as_ref().expect("expected locality").tier,
                    LocalityTier::L2,
                );
            }
            _ => panic!("expected locus"),
        }
    }

    #[test]
    fn parse_locality_stack_order_is_either() {
        // @form before @locality and @locality before @form
        // are both valid stackings.
        for src in [
            r#"
type E { k: Int; v: Int; }
@locality(L1)
@form(hashmap, sync = lockfree, cap = 8)
locus R { capacity { pool entries of E indexed_by k; } }
"#,
            r#"
type E { k: Int; v: Int; }
@form(hashmap, sync = lockfree, cap = 8)
@locality(L1)
locus R { capacity { pool entries of E indexed_by k; } }
"#,
        ] {
            let prog = parse_str(src).expect("parse failed");
            match &prog.items[1] {
                TopDecl::Locus(l) => {
                    assert!(l.form.is_some());
                    assert_eq!(
                        l.locality.as_ref().unwrap().tier,
                        LocalityTier::L1
                    );
                }
                _ => panic!("expected locus"),
            }
        }
    }

    #[test]
    fn parse_locality_rejects_unknown_tier() {
        let src = "@locality(L4) locus L { }";
        let err = parse_str(src).expect_err("expected parse error");
        let msg = format!("{:?}", err);
        assert!(
            msg.contains("unknown locality tier"),
            "wrong error: {}",
            msg
        );
    }

    #[test]
    fn parse_locality_rejects_duplicate() {
        let src = "@locality(L1) @locality(L2) locus L { }";
        let err = parse_str(src).expect_err("expected parse error");
        let msg = format!("{:?}", err);
        assert!(
            msg.contains("duplicate `@locality"),
            "wrong error: {}",
            msg
        );
    }

    #[test]
    fn parse_locality_main_locus_works() {
        // Annotations must precede the `main` contextual
        // keyword too.
        let src = r#"
@locality(L2)
main locus App {
    run() { }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        match &prog.items[0] {
            TopDecl::Locus(l) => {
                assert!(l.is_main);
                assert_eq!(
                    l.locality.as_ref().unwrap().tier,
                    LocalityTier::L2
                );
            }
            _ => panic!("expected locus"),
        }
    }

    #[test]
    fn parse_fallible_marker() {
        let src = r#"
type ParseError { message: string; }

fn parse_int(s: string) -> Int fallible(ParseError) {
    return 0;
}
"#;
        let prog = parse_str(src).expect("parse failed");
        // items[0] = type, items[1] = fn
        match &prog.items[1] {
            TopDecl::Fn(f) => {
                assert_eq!(f.name.name, "parse_int");
                let payload = f.fallible.as_ref().expect("expected fallible marker");
                match payload {
                    TypeExpr::Named { path, .. } => {
                        assert_eq!(path.segments[0].name, "ParseError");
                    }
                    _ => panic!("expected named payload type, got {:?}", payload),
                }
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn parse_fallible_without_return_type() {
        // `-> T` is optional; `fallible(E)` can stand alone.
        let src = r#"
type E { }

fn f() fallible(E) {
    return;
}
"#;
        let prog = parse_str(src).expect("parse failed");
        match &prog.items[1] {
            TopDecl::Fn(f) => {
                assert!(f.ret.is_none());
                assert!(f.fallible.is_some());
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn parse_fallible_as_ident_outside_fn_signature() {
        // `fallible` is contextual — usable as an ordinary
        // identifier outside fn signature return position.
        let src = r#"
fn main() {
    let fallible = 42;
}
"#;
        parse_str(src).expect("parse failed");
    }

    #[test]
    fn parse_fail_stmt_inside_fallible_body() {
        let src = r#"
type E { code: Int; }

fn f() -> Int fallible(E) {
    fail E { code: 1 };
    return 0;
}
"#;
        let prog = parse_str(src).expect("parse failed");
        match &prog.items[1] {
            TopDecl::Fn(f) => {
                let body = &f.body;
                assert!(body.stmts.iter().any(|s| matches!(s, Stmt::Fail { .. })));
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn parse_fail_as_ident_outside_fallible_body() {
        // Outside a fallible fn, `fail` lexes as ident and the
        // parser treats it as an ordinary name.
        let src = r#"
fn main() {
    let fail = 42;
}
"#;
        parse_str(src).expect("parse failed");
    }

    #[test]
    fn parse_or_raise_disposition() {
        let src = r#"
type E { }
fn get(i: Int) -> Int fallible(E) { return 0; }

fn main() {
    let v = get(0) or raise;
}
"#;
        let prog = parse_str(src).expect("parse failed");
        match &prog.items[2] {
            TopDecl::Fn(f) => {
                let stmt = &f.body.stmts[0];
                match stmt {
                    Stmt::Let { value, .. } => match value {
                        Expr::Or {
                            disposition: OrDisposition::Raise(_),
                            ..
                        } => {}
                        other => panic!("expected raise disposition, got {:?}", other),
                    },
                    _ => panic!("expected let"),
                }
            }
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn parse_or_substitute_disposition() {
        let src = r#"
type E { }
fn get(i: Int) -> Int fallible(E) { return 0; }

fn main() {
    let v = get(0) or 99;
}
"#;
        let prog = parse_str(src).expect("parse failed");
        match &prog.items[2] {
            TopDecl::Fn(f) => match &f.body.stmts[0] {
                Stmt::Let {
                    value:
                        Expr::Or {
                            disposition: OrDisposition::Substitute(rhs),
                            ..
                        },
                    ..
                } => {
                    assert!(matches!(rhs.as_ref(), Expr::Literal(Literal::Int(99), _)));
                }
                _ => panic!("expected substitute(99) disposition"),
            },
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn parse_or_handler_call_with_err_binding() {
        // `err` is just an ident in the AST; typecheck (PR2) is
        // responsible for binding it to the payload. Parser
        // accepts any expression on the substitute RHS.
        let src = r#"
type E { msg: string; }
fn get(i: Int) -> Int fallible(E) { return 0; }
fn handle(err: E) -> Int { return -1; }

fn main() {
    let v = get(0) or handle(err);
}
"#;
        parse_str(src).expect("parse failed");
    }

    #[test]
    fn parse_or_right_associative_chain() {
        // a() or b() or raise → a() or (b() or raise)
        let src = r#"
type E { }
fn a() -> Int fallible(E) { return 0; }
fn b() -> Int fallible(E) { return 0; }

fn main() {
    let v = a() or b() or raise;
}
"#;
        let prog = parse_str(src).expect("parse failed");
        // Walk: outer is Or { inner: a(), disposition: Substitute(Or { inner: b(), disposition: Raise }) }
        // items[0]=type, items[1]=fn a, items[2]=fn b, items[3]=main
        match &prog.items[3] {
            TopDecl::Fn(f) => match &f.body.stmts[0] {
                Stmt::Let {
                    value:
                        Expr::Or {
                            disposition: OrDisposition::Substitute(rhs),
                            ..
                        },
                    ..
                } => match rhs.as_ref() {
                    Expr::Or {
                        disposition: OrDisposition::Raise(_),
                        ..
                    } => {}
                    other => panic!("inner should be raise disposition, got {:?}", other),
                },
                _ => panic!("expected outer substitute(inner) disposition"),
            },
            _ => panic!("expected fn"),
        }
    }

    #[test]
    fn parse_or_as_ident_in_non_postfix_position() {
        // `or` is contextual — usable as an ordinary
        // identifier when not in postfix-on-expression position.
        let src = r#"
fn main() {
    let or = 5;
}
"#;
        parse_str(src).expect("parse failed");
    }

    // === v1.x-3 recognition sub-mode parser tests ==========

    fn parse_locus_recognition(src: &str) -> ProjectionClass {
        let prog = parse_str(src).expect("parse failed");
        let l = match &prog.items[0] {
            TopDecl::Locus(l) => l,
            other => panic!("expected locus, got {:?}", other),
        };
        for ann in &l.annotations {
            if let LocusAnnotation::Projection(pc) = ann {
                return *pc;
            }
        }
        panic!("no projection annotation found");
    }

    #[test]
    fn parse_recognition_fixed_cell() {
        let pc = parse_locus_recognition(
            r#"
locus L : projection recognition(cap=4, fixed_cell) {
    accept(c: ChildL) { }
}
locus ChildL { }
"#,
        );
        match pc {
            ProjectionClass::Recognition(Some(p)) => {
                assert_eq!(p.cap, 4);
                assert_eq!(p.sub_mode, RecognitionSubMode::FixedCell);
            }
            other => panic!("expected Recognition(Some), got {:?}", other),
        }
    }

    #[test]
    fn parse_recognition_shared_slab() {
        let pc = parse_locus_recognition(
            r#"
locus L : projection recognition(cap=8, shared_slab) {
    accept(c: ChildL) { }
}
locus ChildL { }
"#,
        );
        match pc {
            ProjectionClass::Recognition(Some(p)) => {
                assert_eq!(p.cap, 8);
                assert_eq!(p.sub_mode, RecognitionSubMode::SharedSlab);
            }
            other => panic!("expected Recognition(Some), got {:?}", other),
        }
    }

    #[test]
    fn parse_recognition_spillover() {
        let pc = parse_locus_recognition(
            r#"
locus L : projection recognition(cap=2, spillover) {
    accept(c: ChildL) { }
}
locus ChildL { }
"#,
        );
        match pc {
            ProjectionClass::Recognition(Some(p)) => {
                assert_eq!(p.cap, 2);
                assert_eq!(p.sub_mode, RecognitionSubMode::Spillover);
            }
            other => panic!("expected Recognition(Some), got {:?}", other),
        }
    }

    #[test]
    fn parse_recognition_summary_only() {
        let pc = parse_locus_recognition(
            r#"
locus L : projection recognition(cap=16, summary_only) {
    accept(c: ChildL) { }
}
locus ChildL { }
"#,
        );
        match pc {
            ProjectionClass::Recognition(Some(p)) => {
                assert_eq!(p.cap, 16);
                assert_eq!(p.sub_mode, RecognitionSubMode::SummaryOnly);
            }
            other => panic!("expected Recognition(Some), got {:?}", other),
        }
    }

    #[test]
    fn parse_recognition_bare_rejected() {
        // v1.x-3 forcing function: bare `: projection recognition`
        // is a parse error, not a class with a default sub-mode.
        let src = r#"
locus L : projection recognition {
    accept(c: ChildL) { }
}
locus ChildL { }
"#;
        let diags = parse_str(src).expect_err("bare recognition must reject");
        let msg = diags
            .iter()
            .map(|d| d.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            msg.contains("requires a sub-mode commitment"),
            "diag should explain the forcing function, got: {msg}"
        );
    }

    #[test]
    fn parse_recognition_missing_sub_mode_rejected() {
        // `cap=N` alone (no sub-mode argument) is also rejected.
        let src = r#"
locus L : projection recognition(cap=4) {
    accept(c: ChildL) { }
}
locus ChildL { }
"#;
        let diags = parse_str(src).expect_err("missing sub-mode must reject");
        let msg = diags
            .iter()
            .map(|d| d.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        // Either the `,` between cap and sub-mode or the closing
        // paren without sub-mode is reported — accept either as
        // long as we got a parse error rooted in the missing arg.
        assert!(
            !msg.is_empty(),
            "expected non-empty parse diag for missing sub-mode"
        );
    }

    #[test]
    fn parse_recognition_unknown_sub_mode_rejected() {
        let src = r#"
locus L : projection recognition(cap=4, fancy_mode) {
    accept(c: ChildL) { }
}
locus ChildL { }
"#;
        let diags = parse_str(src).expect_err("unknown sub-mode must reject");
        let msg = diags
            .iter()
            .map(|d| d.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            msg.contains("unknown recognition sub-mode"),
            "diag should name the unknown sub-mode, got: {msg}"
        );
    }

    #[test]
    fn parse_recognition_zero_cap_rejected() {
        let src = r#"
locus L : projection recognition(cap=0, fixed_cell) {
    accept(c: ChildL) { }
}
locus ChildL { }
"#;
        let diags =
            parse_str(src).expect_err("cap=0 must reject (positive int literal)");
        let msg = diags
            .iter()
            .map(|d| d.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            msg.contains("`cap` must be a positive integer"),
            "diag should report cap must be positive, got: {msg}"
        );
    }

    // === v1.x-IMPORT PR1 tests ============================
    //
    // Forcing-function rule: `import "path";` requires `as <alias>`.
    // Same discipline as v1.x-3 (no default sub-mode on `recognition`)
    // and v1.x-FORM-2 (locus methods can't declare `fallible(E)`):
    // the user names the namespace at the import site.

    #[test]
    fn parse_import_with_alias_ok() {
        let src = r#"
import "lib/foo" as foo;

fn main() { }
"#;
        let prog = parse_str(src).expect("parse failed");
        assert_eq!(prog.imports.len(), 1);
        assert_eq!(prog.imports[0].path, "lib/foo");
        assert_eq!(prog.imports[0].alias.as_deref(), Some("foo"));
    }

    #[test]
    fn parse_import_rejects_bare() {
        let src = r#"
import "lib/foo";

fn main() { }
"#;
        let diags = parse_str(src).expect_err("bare import must reject");
        let msg = diags
            .iter()
            .map(|d| d.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            msg.contains("must declare an alias"),
            "diag should explain the alias requirement, got: {msg}"
        );
        assert!(
            msg.contains("lib/foo"),
            "diag should quote the offending path, got: {msg}"
        );
    }

    #[test]
    fn parse_import_rejects_missing_alias_ident() {
        let src = r#"
import "lib/foo" as ;

fn main() { }
"#;
        let diags =
            parse_str(src).expect_err("missing alias ident must reject");
        let msg = diags
            .iter()
            .map(|d| d.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            msg.contains("import alias"),
            "diag should mention the missing alias identifier, got: {msg}"
        );
    }

    #[test]
    fn parse_import_after_top_decl_errors_and_terminates() {
        // Regression: an `import` placed after any top-level decl
        // used to wedge the parser in an unbounded-allocation loop
        // (recover_to_top_level treated `Import` as a stop token,
        // so the second `parse_program` loop kept failing on the
        // same import token and pushing a Diag forever — surfaced
        // as a ~27 GB OOM on real-world source).
        let src = r#"
type Foo { x: Int; }
import "lib/foo" as foo;

fn main() { }
"#;
        let diags = parse_str(src)
            .expect_err("import after top decl must reject");
        let msg = diags
            .iter()
            .map(|d| d.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            msg.contains("must appear before any top-level declaration"),
            "diag should explain the ordering rule, got: {msg}"
        );
    }

    // === v1.x-FORM-4 PR1 tests ===========================
    //
    // Capacity-slot `indexed_by <fieldname>` clause. Parser-
    // level only — typecheck enforcement that `indexed_by` is
    // meaningful only on `@form(hashmap)` lands in PR2.

    #[test]
    fn parse_indexed_by_clause() {
        let src = r#"
@form(hashmap)
locus Registry {
    capacity { pool entries of Entry indexed_by name; }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        let cap = match &prog.items[0] {
            TopDecl::Locus(l) => l.members.iter().find_map(|m| match m {
                LocusMember::Capacity(c) => Some(c),
                _ => None,
            }).expect("capacity block"),
            _ => panic!("expected locus"),
        };
        let slot = &cap.slots[0];
        assert_eq!(slot.name.name, "entries");
        assert_eq!(slot.kind, CapacitySlotKind::Pool);
        assert_eq!(slot.indexed_by.as_ref().map(|i| i.name.as_str()), Some("name"));
        assert!(slot.as_parent_for.is_none());
    }

    #[test]
    fn parse_slot_without_indexed_by_clause() {
        // Existing plain slots stay unchanged — indexed_by is None.
        let src = r#"
locus Bag {
    capacity { heap items of Int; }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        let cap = match &prog.items[0] {
            TopDecl::Locus(l) => l.members.iter().find_map(|m| match m {
                LocusMember::Capacity(c) => Some(c),
                _ => None,
            }).expect("capacity block"),
            _ => panic!("expected locus"),
        };
        assert!(cap.slots[0].indexed_by.is_none());
    }

    #[test]
    fn parse_indexed_by_missing_ident_rejected() {
        let src = r#"
locus Registry {
    capacity { pool entries of Entry indexed_by ; }
}
"#;
        let diags =
            parse_str(src).expect_err("missing fieldname must reject");
        let msg = diags
            .iter()
            .map(|d| d.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            msg.contains("field name after `indexed_by`"),
            "diag should name the missing fieldname, got: {msg}"
        );
    }

    #[test]
    fn parse_indexed_by_outside_keyword_position_is_ident() {
        // `indexed_by` outside the slot clause position should
        // not lex as a keyword. We don't have a great place to
        // test "is an ident" without a synthetic decl, but we
        // can at least verify a normal slot decl after one
        // doesn't get confused. Smoke-test: two slots, first
        // uses indexed_by, second doesn't, both parse.
        let src = r#"
locus Two {
    capacity {
        pool keyed of Entry indexed_by name;
        heap log of Int;
    }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        let cap = match &prog.items[0] {
            TopDecl::Locus(l) => l.members.iter().find_map(|m| match m {
                LocusMember::Capacity(c) => Some(c),
                _ => None,
            }).expect("capacity block"),
            _ => panic!("expected locus"),
        };
        assert_eq!(cap.slots.len(), 2);
        assert_eq!(cap.slots[0].indexed_by.as_ref().map(|i| i.name.as_str()), Some("name"));
        assert!(cap.slots[1].indexed_by.is_none());
    }

    // v1.x-VIOLATE (F.27) phase 2 — parser tests.

    #[test]
    fn parse_inline_closure_no_assertion() {
        let src = r#"
locus L {
    params { last_error: String = ""; }
    closure fatal_io { captures: last_error; epoch inline; }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        let cl = match &prog.items[0] {
            TopDecl::Locus(l) => l.members.iter().find_map(|m| match m {
                LocusMember::Closure(c) => Some(c),
                _ => None,
            }).expect("closure decl"),
            _ => panic!("expected locus"),
        };
        assert_eq!(cl.name.name, "fatal_io");
        assert!(cl.assertion.is_none());
        assert_eq!(cl.clauses.len(), 2);
        let mut saw_captures = false;
        let mut saw_inline = false;
        for cls in &cl.clauses {
            match cls {
                ClosureClause::Captures(names) => {
                    assert_eq!(names.len(), 1);
                    assert_eq!(names[0].name, "last_error");
                    saw_captures = true;
                }
                ClosureClause::Epoch(EpochSpec::Inline) => {
                    saw_inline = true;
                }
                _ => panic!("unexpected clause: {:?}", cls),
            }
        }
        assert!(saw_captures && saw_inline);
    }

    #[test]
    fn parse_assertion_bearing_closure_still_works() {
        let src = r#"
locus L {
    params { x: Int = 0; }
    closure invariant { self.x ~~ self.x within 0; epoch tick; }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        match &prog.items[0] {
            TopDecl::Locus(l) => {
                let cl = l.members.iter().find_map(|m| match m {
                    LocusMember::Closure(c) => Some(c),
                    _ => None,
                }).expect("closure decl");
                assert!(cl.assertion.is_some());
            }
            _ => panic!("expected locus"),
        }
    }

    #[test]
    fn parse_violate_stmt_bare() {
        let src = r#"
locus L {
    params { x: Int = 0; }
    closure fatal { epoch inline; }
    fn step() { violate fatal; }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        let f = match &prog.items[0] {
            TopDecl::Locus(l) => l.members.iter().find_map(|m| match m {
                LocusMember::Fn(f) => Some(f),
                _ => None,
            }).expect("fn"),
            _ => panic!("expected locus"),
        };
        assert!(matches!(
            f.body.stmts[0],
            Stmt::Violate { ref name, payload: None, .. } if name.name == "fatal"
        ));
    }

    #[test]
    fn parse_violate_stmt_with_payload() {
        let src = r#"
locus L {
    closure fatal { epoch inline; }
    fn step() { violate fatal with 42; }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        let f = match &prog.items[0] {
            TopDecl::Locus(l) => l.members.iter().find_map(|m| match m {
                LocusMember::Fn(f) => Some(f),
                _ => None,
            }).expect("fn"),
            _ => panic!("expected locus"),
        };
        match &f.body.stmts[0] {
            Stmt::Violate { name, payload: Some(_), .. } => {
                assert_eq!(name.name, "fatal");
            }
            other => panic!("expected violate stmt with payload, got {:?}", other),
        }
    }

    #[test]
    fn parse_violate_as_ident_when_no_stmt_shape() {
        // `violate` outside the violate-stmt grammar lexes /
        // parses as an ordinary identifier. `let violate = 1;`
        // must remain admissible, as must `violate()` (call).
        let src = r#"
fn main() {
    let violate = 1;
    let _ = violate;
}
"#;
        parse_str(src).expect("parse failed");
    }

    #[test]
    fn parse_with_as_ident_outside_violate_stmt() {
        // `with` is no longer reserved; ordinary identifier
        // elsewhere.
        let src = r#"
fn main() {
    let with = 1;
    let _ = with;
}
"#;
        parse_str(src).expect("parse failed");
    }

    #[test]
    fn parse_binding_with_where_constraints() {
        // Form K (2026-05-20): the optional `where ...` clause
        // after a transport spec carries operational constraints.
        let src = r#"
type Ping { n: Int; }
topic Evt { payload: Ping; }

main locus App {
    bindings {
        Evt: unix("/tmp/evt.sock") where intra_machine, zero_copy;
    }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        let locus = prog
            .items
            .iter()
            .find_map(|it| match it {
                TopDecl::Locus(l) if l.is_main => Some(l),
                _ => None,
            })
            .expect("main locus");
        let bb = locus
            .members
            .iter()
            .find_map(|m| match m {
                LocusMember::Bindings(b) => Some(b),
                _ => None,
            })
            .expect("bindings block");
        assert_eq!(bb.entries.len(), 1);
        let cs: Vec<BindingConstraint> = bb.entries[0]
            .constraints
            .iter()
            .map(|sc| sc.kind)
            .collect();
        assert_eq!(
            cs,
            vec![BindingConstraint::IntraMachine, BindingConstraint::ZeroCopy]
        );
    }

    #[test]
    fn parse_binding_without_constraints_keeps_empty_vec() {
        // Backwards-compat: a binding entry with no `where`
        // clause parses to `constraints: vec![]`. Existing
        // bindings tests rely on this.
        let src = r#"
type Ping { n: Int; }
topic Evt { payload: Ping; }

main locus App {
    bindings {
        Evt: unix("/tmp/evt.sock");
    }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        let locus = prog
            .items
            .iter()
            .find_map(|it| match it {
                TopDecl::Locus(l) if l.is_main => Some(l),
                _ => None,
            })
            .expect("main locus");
        let bb = locus
            .members
            .iter()
            .find_map(|m| match m {
                LocusMember::Bindings(b) => Some(b),
                _ => None,
            })
            .expect("bindings block");
        assert!(bb.entries[0].constraints.is_empty());
    }

    #[test]
    fn parse_binding_unknown_constraint_errors() {
        let src = r#"
type Ping { n: Int; }
topic Evt { payload: Ping; }

main locus App {
    bindings {
        Evt: unix("/tmp/evt.sock") where banana;
    }
}
"#;
        let err = parse_str(src).expect_err("expected parse error");
        assert!(
            err.iter()
                .any(|d| d.message.contains("unknown binding constraint")),
            "expected unknown-constraint diag, got: {:?}",
            err
        );
    }

    #[test]
    fn parse_binding_where_works_on_adapter_transport() {
        // `where` clause sits after the transport regardless of
        // which variant — unix or adapter.
        let src = r#"
type Ping { n: Int; }
topic Evt { payload: Ping; }

main locus App {
    bindings {
        Evt: MyAdapter { url: "nats://x" } where cross_machine;
    }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        let locus = prog
            .items
            .iter()
            .find_map(|it| match it {
                TopDecl::Locus(l) if l.is_main => Some(l),
                _ => None,
            })
            .expect("main locus");
        let bb = locus
            .members
            .iter()
            .find_map(|m| match m {
                LocusMember::Bindings(b) => Some(b),
                _ => None,
            })
            .expect("bindings block");
        let cs: Vec<BindingConstraint> = bb.entries[0]
            .constraints
            .iter()
            .map(|sc| sc.kind)
            .collect();
        assert_eq!(cs, vec![BindingConstraint::CrossMachine]);
    }

    #[test]
    fn parse_shm_ring_transport_with_default_slot_count() {
        // Form K4b + K7 (2026-05-20): `shm_ring("/name", on_overflow: X)`
        // parses; slot_count defaults to 128; on_overflow is
        // required.
        let src = r#"
type Ping { n: Int; }
topic Evt { payload: Ping; }

main locus App {
    bindings {
        Evt: shm_ring("/hale_evt", on_overflow: drop) where zero_copy;
    }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        let locus = prog
            .items
            .iter()
            .find_map(|it| match it {
                TopDecl::Locus(l) if l.is_main => Some(l),
                _ => None,
            })
            .expect("main locus");
        let bb = locus
            .members
            .iter()
            .find_map(|m| match m {
                LocusMember::Bindings(b) => Some(b),
                _ => None,
            })
            .expect("bindings block");
        assert_eq!(bb.entries.len(), 1);
        match &bb.entries[0].transport {
            TransportSpec::ShmRing { name, slot_count, overflow, .. } => {
                assert_eq!(name, "/hale_evt");
                assert_eq!(*slot_count, 128);
                assert_eq!(*overflow, ShmRingOverflow::Drop);
            }
            other => panic!("expected ShmRing, got {:?}", other),
        }
    }

    #[test]
    fn parse_shm_ring_with_explicit_slot_count() {
        let src = r#"
type Ping { n: Int; }
topic Evt { payload: Ping; }

main locus App {
    bindings {
        Evt: shm_ring("/hale_evt", slot_count: 256, on_overflow: block);
    }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        let locus = prog
            .items
            .iter()
            .find_map(|it| match it {
                TopDecl::Locus(l) if l.is_main => Some(l),
                _ => None,
            })
            .expect("main locus");
        let bb = locus
            .members
            .iter()
            .find_map(|m| match m {
                LocusMember::Bindings(b) => Some(b),
                _ => None,
            })
            .expect("bindings block");
        match &bb.entries[0].transport {
            TransportSpec::ShmRing { name, slot_count, overflow, .. } => {
                assert_eq!(name, "/hale_evt");
                assert_eq!(*slot_count, 256);
                assert_eq!(*overflow, ShmRingOverflow::Block);
            }
            other => panic!("expected ShmRing, got {:?}", other),
        }
    }

    #[test]
    fn parse_shm_ring_zero_slot_count_rejected() {
        let src = r#"
type Ping { n: Int; }
topic Evt { payload: Ping; }

main locus App {
    bindings {
        Evt: shm_ring("/hale_evt", slot_count: 0, on_overflow: drop);
    }
}
"#;
        let err = parse_str(src).expect_err("expected error");
        assert!(
            err.iter()
                .any(|d| d.message.contains("slot_count must be positive")),
            "expected positive-slot-count diag, got: {:?}",
            err
        );
    }

    #[test]
    fn parse_shm_ring_missing_on_overflow_rejected() {
        // Form K7: on_overflow is REQUIRED — no default. Forces
        // the user to think about back-pressure semantics.
        let src = r#"
type Ping { n: Int; }
topic Evt { payload: Ping; }

main locus App {
    bindings {
        Evt: shm_ring("/hale_evt", slot_count: 64) where zero_copy;
    }
}
"#;
        let err = parse_str(src).expect_err("expected error");
        assert!(
            err.iter()
                .any(|d| d.message.contains("requires `on_overflow:`")),
            "expected missing-on_overflow diag, got: {:?}",
            err
        );
    }

    #[test]
    fn parse_shm_ring_unknown_overflow_policy_rejected() {
        let src = r#"
type Ping { n: Int; }
topic Evt { payload: Ping; }

main locus App {
    bindings {
        Evt: shm_ring("/hale_evt", on_overflow: ignore);
    }
}
"#;
        let err = parse_str(src).expect_err("expected error");
        assert!(
            err.iter()
                .any(|d| d.message.contains("unknown `on_overflow` policy")),
            "expected unknown-policy diag, got: {:?}",
            err
        );
    }

    #[test]
    fn parse_shm_ring_overflow_fail_policy() {
        let src = r#"
type Ping { n: Int; }
topic Evt { payload: Ping; }

main locus App {
    bindings {
        Evt: shm_ring("/hale_evt", on_overflow: fail);
    }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        let locus = prog
            .items
            .iter()
            .find_map(|it| match it {
                TopDecl::Locus(l) if l.is_main => Some(l),
                _ => None,
            })
            .expect("main locus");
        let bb = locus
            .members
            .iter()
            .find_map(|m| match m {
                LocusMember::Bindings(b) => Some(b),
                _ => None,
            })
            .expect("bindings block");
        match &bb.entries[0].transport {
            TransportSpec::ShmRing { overflow, .. } => {
                assert_eq!(*overflow, ShmRingOverflow::Fail);
            }
            other => panic!("expected ShmRing, got {:?}", other),
        }
    }

    #[test]
    fn parse_array_literal_with_trailing_comma() {
        // v1.x polish (2026-05-20): multi-line array literals
        // with trailing commas now parse. Single-line form
        // works too — same code path. Reported as a cosmetic
        // friction by a downstream consumer; idiomatic Hale
        // mirrors Rust's trailing-comma allowance on collection
        // literals.
        let multi_line = r#"
            type B { v: Int; }
            fn main() {
                let xs = [
                    B { v: 1 },
                    B { v: 2 },
                ];
            }
        "#;
        parse_str(multi_line).expect("multi-line array literal");

        let single_line_trailing = r#"
            type B { v: Int; }
            fn main() {
                let xs = [B { v: 1 }, B { v: 2 },];
            }
        "#;
        parse_str(single_line_trailing).expect("single-line trailing comma");

        // No-trailing-comma stays admissible (was the only
        // shape that worked pre-fix).
        let no_trailing = r#"
            type B { v: Int; }
            fn main() {
                let xs = [B { v: 1 }, B { v: 2 }];
            }
        "#;
        parse_str(no_trailing).expect("no trailing comma");
    }

    #[test]
    fn parse_shm_ring_unknown_kwarg_rejected() {
        let src = r#"
type Ping { n: Int; }
topic Evt { payload: Ping; }

main locus App {
    bindings {
        Evt: shm_ring("/hale_evt", slot_size: 80);
    }
}
"#;
        let err = parse_str(src).expect_err("expected error");
        assert!(
            err.iter()
                .any(|d| d.message.contains("unknown") && d.message.contains("shm_ring")),
            "expected unknown-kwarg diag, got: {:?}",
            err
        );
    }

    #[test]
    fn parse_adapter_binding_transport() {
        // Wave B: a capitalized head followed by `{ ... }` parses
        // as TransportSpec::Adapter with the field inits captured.
        let src = r#"
type Ping { n: Int; }
topic Evt { payload: Ping; }

main locus App {
    bindings {
        Evt: MyAdapter { url: "nats://localhost", retries: 3 };
    }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        let locus = prog
            .items
            .iter()
            .find_map(|it| match it {
                TopDecl::Locus(l) if l.is_main => Some(l),
                _ => None,
            })
            .expect("main locus");
        let bb = locus
            .members
            .iter()
            .find_map(|m| match m {
                LocusMember::Bindings(b) => Some(b),
                _ => None,
            })
            .expect("bindings block");
        assert_eq!(bb.entries.len(), 1);
        let entry = &bb.entries[0];
        assert_eq!(entry.topic.name, "Evt");
        match &entry.transport {
            TransportSpec::Adapter { locus, inits, .. } => {
                assert_eq!(locus.name, "MyAdapter");
                assert_eq!(inits.len(), 2);
                assert_eq!(inits[0].name.name, "url");
                assert_eq!(inits[1].name.name, "retries");
            }
            other => panic!("expected Adapter transport, got {:?}", other),
        }
    }

    // F.31 (2026-05-23): placement block parser tests.

    fn placement_block_of(prog: &Program) -> &PlacementBlock {
        let locus = prog
            .items
            .iter()
            .find_map(|it| match it {
                TopDecl::Locus(l) if l.is_main => Some(l),
                _ => None,
            })
            .expect("main locus");
        locus
            .members
            .iter()
            .find_map(|m| match m {
                LocusMember::Placement(p) => Some(p),
                _ => None,
            })
            .expect("placement block")
    }

    #[test]
    fn parse_placement_pinned_bare() {
        let src = r#"
locus Job { run() { } }

main locus App {
    params { job: Job = Job { }; }
    placement { job: pinned; }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        let pb = placement_block_of(&prog);
        assert_eq!(pb.entries.len(), 1);
        assert_eq!(pb.entries[0].field.name, "job");
        match &pb.entries[0].spec {
            PlacementSpec::Pinned { core: None } => {}
            other => panic!("expected Pinned{{ core: None }}, got {:?}", other),
        }
    }

    #[test]
    fn parse_placement_pinned_with_core() {
        let src = r#"
locus Worker { run() { } }

main locus App {
    params { w: Worker = Worker { }; }
    placement { w: pinned(core = 3); }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        let pb = placement_block_of(&prog);
        match &pb.entries[0].spec {
            PlacementSpec::Pinned { core: Some(3) } => {}
            other => panic!("expected Pinned{{ core: Some(3) }}, got {:?}", other),
        }
    }

    #[test]
    fn parse_placement_cooperative_bare() {
        let src = r#"
locus Worker { run() { } }

main locus App {
    params { w: Worker = Worker { }; }
    placement { w: cooperative; }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        let pb = placement_block_of(&prog);
        match &pb.entries[0].spec {
            PlacementSpec::Cooperative { pool: None } => {}
            other => panic!(
                "expected Cooperative{{ pool: None }}, got {:?}", other
            ),
        }
    }

    #[test]
    fn parse_placement_cooperative_with_pool() {
        let src = r#"
locus Worker { run() { } }

main locus App {
    params { w: Worker = Worker { }; }
    placement { w: cooperative(pool = io); }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        let pb = placement_block_of(&prog);
        match &pb.entries[0].spec {
            PlacementSpec::Cooperative { pool: Some(p) } => {
                assert_eq!(p.name, "io");
            }
            other => panic!(
                "expected Cooperative{{ pool: Some(io) }}, got {:?}", other
            ),
        }
    }

    #[test]
    fn parse_placement_multiple_entries() {
        let src = r#"
locus A { run() { } }
locus B { run() { } }
locus C { run() { } }

main locus App {
    params {
        a: A = A { };
        b: B = B { };
        c: C = C { };
    }
    placement {
        a: pinned(core = 1);
        b: pinned(core = 2);
        c: cooperative(pool = io);
    }
}
"#;
        let prog = parse_str(src).expect("parse failed");
        let pb = placement_block_of(&prog);
        assert_eq!(pb.entries.len(), 3);
        assert_eq!(pb.entries[0].field.name, "a");
        assert_eq!(pb.entries[1].field.name, "b");
        assert_eq!(pb.entries[2].field.name, "c");
    }

    #[test]
    fn parse_placement_outside_main_rejected() {
        let src = r#"
locus NotMain {
    placement { x: pinned; }
}
"#;
        let err = parse_str(src).expect_err("should reject");
        let msg = format!("{:?}", err);
        assert!(
            msg.contains("placement") && msg.contains("main"),
            "expected diagnostic about placement being main-only; got: {}",
            msg
        );
    }

    #[test]
    fn parse_old_schedule_annotation_rejected() {
        // F.31: `: schedule pinned` is no longer accepted on
        // a locus declaration. Diag should hint at the new
        // placement-block shape.
        let src = r#"
locus Old : schedule pinned {
    run() { }
}
"#;
        let err = parse_str(src).expect_err("should reject");
        let msg = format!("{:?}", err);
        assert!(
            msg.contains("placement") || msg.contains("F.31"),
            "expected diag to mention placement / F.31 migration; got: {}",
            msg
        );
    }

    // === FUv0.8.2 #7 target capability-block tests =========

    #[test]
    fn parse_target_decl_with_capabilities() {
        let src = r#"
target browser_js {
    arenas.epoch_view,
    time.monotonic,
    time.wallclock,
    random.csprng,
    gfx.canvas2d,
}
"#;
        let prog = parse_str(src).expect("parse failed");
        assert_eq!(prog.items.len(), 1);
        match &prog.items[0] {
            TopDecl::Target(t) => {
                assert_eq!(t.name.name, "browser_js");
                assert_eq!(t.capabilities.len(), 5);
                // First cap is `arenas.epoch_view` — 2 segments.
                assert_eq!(t.capabilities[0].segments.len(), 2);
                assert_eq!(t.capabilities[0].segments[0].name, "arenas");
                assert_eq!(t.capabilities[0].segments[1].name, "epoch_view");
                // Last cap is `gfx.canvas2d`.
                assert_eq!(t.capabilities[4].segments[0].name, "gfx");
                assert_eq!(t.capabilities[4].segments[1].name, "canvas2d");
            }
            other => panic!("expected Target decl, got: {:?}", other),
        }
    }

    #[test]
    fn parse_target_decl_with_no_capabilities() {
        // Empty body is legal — a target with no capabilities
        // is a target the program can only do pure computation
        // on (no I/O, no time, no anything). Useful for
        // sandbox-target builds.
        let src = "target empty { }";
        let prog = parse_str(src).expect("parse failed");
        match &prog.items[0] {
            TopDecl::Target(t) => {
                assert_eq!(t.name.name, "empty");
                assert!(t.capabilities.is_empty());
            }
            _ => panic!("expected Target"),
        }
    }

    #[test]
    fn parse_target_decl_trailing_comma_allowed() {
        let src = "target native { time.monotonic, io.fs, }";
        let prog = parse_str(src).expect("parse failed");
        match &prog.items[0] {
            TopDecl::Target(t) => {
                assert_eq!(t.capabilities.len(), 2);
            }
            _ => panic!("expected Target"),
        }
    }

    #[test]
    fn parse_target_decl_single_segment_capability() {
        // A bare identifier (no dot) is also a valid capability
        // — useful for top-level "the whole subsystem" gates
        // like `target wasm { wasi }`.
        let src = "target wasm { wasi, time.monotonic }";
        let prog = parse_str(src).expect("parse failed");
        match &prog.items[0] {
            TopDecl::Target(t) => {
                assert_eq!(t.capabilities[0].segments.len(), 1);
                assert_eq!(t.capabilities[0].segments[0].name, "wasi");
                assert_eq!(t.capabilities[1].segments.len(), 2);
            }
            _ => panic!("expected Target"),
        }
    }

    #[test]
    fn parse_target_decl_alongside_other_top_decls() {
        let src = r#"
target native { time.monotonic }
locus L { run() { } }
fn main() { L { }; }
"#;
        let prog = parse_str(src).expect("parse failed");
        assert_eq!(prog.items.len(), 3);
        assert!(matches!(prog.items[0], TopDecl::Target(_)));
        assert!(matches!(prog.items[1], TopDecl::Locus(_)));
        assert!(matches!(prog.items[2], TopDecl::Fn(_)));
    }
}
