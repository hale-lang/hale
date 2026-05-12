//! Recursive-descent parser for Aperio.
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
        // Skip until we find a likely top-level start.
        while !matches!(
            self.peek(),
            TokenKind::Eof
                | TokenKind::Locus
                | TokenKind::Perspective
                | TokenKind::Type
                | TokenKind::Const
                | TokenKind::Fn
                | TokenKind::Module
                | TokenKind::Import
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
        // Optional `as IDENT` alias.
        let alias = if self.peek_is_kw_as() {
            self.bump();
            Some(self.expect_ident("import alias")?.name)
        } else {
            None
        };
        let semi = self.expect(TokenKind::Semi, ";")?;
        Ok(Import {
            path,
            alias,
            span: kw.span.merge(semi.span),
        })
    }

    fn parse_top_decl(&mut self) -> Result<TopDecl, Diag> {
        // v1.x-FORM-1: optional `@form(...)` annotation prefix.
        // v1 recognizes this only as a prefix to `locus`.
        if matches!(self.peek(), TokenKind::At) {
            let form = self.parse_form_annotation()?;
            if !matches!(self.peek(), TokenKind::Locus) {
                return Err(Diag::parse(
                    self.peek_token().span,
                    "expected `locus` after `@form(...)` annotation",
                ));
            }
            let mut locus = self.parse_locus_decl()?;
            locus.span = form.span.merge(locus.span);
            locus.form = Some(form);
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
            other => Err(Diag::parse(
                self.peek_token().span,
                format!("expected top-level declaration, got {:?}", other),
            )),
        }
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
                "expected `form` after `@` (v1 recognizes only `@form(...)` annotations)",
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
        Ok(LocusDecl {
            name,
            generics,
            annotations,
            form: None,
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
                        self.bump();
                        ProjectionClass::Recognition
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
            TokenKind::Schedule => {
                self.bump();
                let class = match self.peek() {
                    TokenKind::Cooperative => {
                        self.bump();
                        ScheduleClass::Cooperative
                    }
                    TokenKind::Pinned => {
                        self.bump();
                        // Optional `(core = N)` attribute. Expect
                        // exactly that one shape for v0; future
                        // attributes (priority, scheduler policy,
                        // etc.) plug in here.
                        let core = if matches!(self.peek(), TokenKind::LParen) {
                            self.bump();
                            let attr_tok = self.peek_token();
                            let attr_name = match self.peek() {
                                TokenKind::Ident(s) => s.clone(),
                                other => {
                                    return Err(Diag::parse(
                                        attr_tok.span,
                                        format!(
                                            "expected `core` inside `pinned(...)`, got {:?}",
                                            other
                                        ),
                                    ));
                                }
                            };
                            if attr_name != "core" {
                                return Err(Diag::parse(
                                    attr_tok.span,
                                    format!(
                                        "unknown pinned attribute `{}`; only `core` \
                                         is recognized in v0",
                                        attr_name
                                    ),
                                ));
                            }
                            self.bump();
                            self.expect(TokenKind::Eq, "expected `=` after `core`")?;
                            let n_tok = self.peek_token();
                            let n = match self.peek() {
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
                        ScheduleClass::Pinned(core)
                    }
                    other => {
                        return Err(Diag::parse(
                            self.peek_token().span,
                            format!(
                                "expected schedule class \
                                 (cooperative | pinned), got {:?}",
                                other
                            ),
                        ));
                    }
                };
                Ok(LocusAnnotation::Schedule(class))
            }
            other => Err(Diag::parse(
                self.peek_token().span,
                format!(
                    "expected tier / projection / schedule annotation, got {:?}",
                    other
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
            TokenKind::Mode => self.parse_mode_decl().map(LocusMember::Mode),
            TokenKind::OnFailure => self.parse_failure_decl().map(LocusMember::Failure),
            TokenKind::Closure => self.parse_closure_decl().map(LocusMember::Closure),
            TokenKind::Fn => self.parse_fn_decl().map(LocusMember::Fn),
            TokenKind::Const => self.parse_const_decl().map(LocusMember::Const),
            TokenKind::Type => self.parse_type_decl().map(LocusMember::Type),
            other => Err(Diag::parse(
                self.peek_token().span,
                format!("expected locus member, got {:?}", other),
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
                let subject = match self.peek().clone() {
                    TokenKind::StringLit(s) => {
                        self.bump();
                        s
                    }
                    other => {
                        return Err(Diag::parse(
                            self.peek_token().span,
                            format!("expected subject string, got {:?}", other),
                        ));
                    }
                };
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
                let subject = match self.peek().clone() {
                    TokenKind::StringLit(s) => {
                        self.bump();
                        s
                    }
                    other => {
                        return Err(Diag::parse(
                            self.peek_token().span,
                            format!("expected subject string, got {:?}", other),
                        ));
                    }
                };
                self.expect(TokenKind::Of, "of")?;
                self.expect(TokenKind::Type, "type")?;
                let ty = self.parse_type_expr()?;
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
        let kw = self.expect(TokenKind::Mode, "mode")?;
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
        // First clause: assertion (LEFT ~~ RIGHT within TOL ;)
        let assertion = self.parse_closure_assertion()?;
        // Optional clauses
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
        match self.peek() {
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
                    TokenKind::Recognition => ProjectionClass::Recognition,
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
        let (disposition, end_span) = if is_raise {
            let raise_tok = self.bump();
            (OrDisposition::Raise(raise_tok.span), raise_tok.span)
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
    /// Aperio expression via a fresh inner parser. That lets `f"{a + b * 2}"`
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
        TokenKind::Mode => "mode",
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
    "Duration", "Bytes",
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
}
