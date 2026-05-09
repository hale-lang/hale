//! Recursive-descent parser for lotus.
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
use crate::lexer::{Token, TokenKind};
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
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Parser {
            tokens,
            pos: 0,
            diags: Vec::new(),
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
        match self.peek() {
            TokenKind::Locus => self.parse_locus_decl().map(TopDecl::Locus),
            TokenKind::Perspective => self.parse_perspective_decl().map(TopDecl::Perspective),
            TokenKind::Type => self.parse_type_decl().map(TopDecl::Type),
            TokenKind::Const => self.parse_const_decl().map(TopDecl::Const),
            TokenKind::Fn => self.parse_fn_decl().map(TopDecl::Fn),
            TokenKind::Module => self.parse_module_decl().map(TopDecl::Module),
            other => Err(Diag::parse(
                self.peek_token().span,
                format!("expected top-level declaration, got {:?}", other),
            )),
        }
    }

    // === locus ===========================================

    fn parse_locus_decl(&mut self) -> Result<LocusDecl, Diag> {
        let kw = self.expect(TokenKind::Locus, "locus")?;
        let name = self.expect_ident("locus name")?;

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
            annotations,
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
                        ScheduleClass::Pinned
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
        // Either ~~ or `approx` keyword
        if !(self.eat(&TokenKind::TildeTilde) || self.eat(&TokenKind::Approx)) {
            return Err(Diag::parse(
                self.peek_token().span,
                "expected `~~` or `approx` in closure assertion",
            ));
        }
        let right = self.parse_expr()?;
        self.expect(TokenKind::Within, "within")?;
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
                let names = self.parse_paren_ident_list()?;
                self.expect(TokenKind::Semi, ";")?;
                Ok(ClosureClause::PersistsThrough(names))
            }
            TokenKind::ResetsOn => {
                self.bump();
                let names = self.parse_paren_ident_list()?;
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

    fn parse_paren_ident_list(&mut self) -> Result<Vec<Ident>, Diag> {
        self.expect(TokenKind::LParen, "(")?;
        let mut names = Vec::new();
        if !self.at(&TokenKind::RParen) {
            names.push(self.expect_ident("identifier")?);
            while self.eat(&TokenKind::Comma) {
                names.push(self.expect_ident("identifier")?);
            }
        }
        self.expect(TokenKind::RParen, ")")?;
        Ok(names)
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
        let name = self.expect_ident("field name")?;
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
        self.expect(TokenKind::Gt, ">")?;
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
        let body = self.parse_block()?;
        Ok(FnDecl {
            name,
            generics,
            params,
            ret,
            span: kw.span.merge(body.span),
            body,
        })
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
                let close = self.expect(TokenKind::Gt, ">")?;
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
                    let gt = self.expect(TokenKind::Gt, ">")?;
                    span = span.merge(gt.span);
                }
                Ok(TypeExpr::Named {
                    path: qn,
                    generic_args,
                    span,
                })
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
        while !self.at(&TokenKind::RBrace) && !matches!(self.peek(), TokenKind::Eof) {
            stmts.push(self.parse_stmt()?);
        }
        let rb = self.expect(TokenKind::RBrace, "}")?;
        Ok(Block {
            stmts,
            span: lb.span.merge(rb.span),
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
            TokenKind::LBrace => Ok(Stmt::Block(self.parse_block()?)),
            // Recovery primitives
            TokenKind::Restart
            | TokenKind::RestartInPlace
            | TokenKind::Drain
            | TokenKind::Dissolve
            | TokenKind::Quarantine
            | TokenKind::Reorganize
            | TokenKind::Bubble => self.parse_recovery_stmt(),
            _ => self.parse_expr_or_assign_stmt(),
        }
    }

    fn parse_let_stmt(&mut self) -> Result<Stmt, Diag> {
        let kw = self.expect(TokenKind::Let, "let")?;
        let is_mut = self.eat(&TokenKind::Mut);
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
            TokenKind::Drain => RecoveryOp::Drain,
            TokenKind::Dissolve => RecoveryOp::Dissolve,
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

    fn parse_expr_or_assign_stmt(&mut self) -> Result<Stmt, Diag> {
        // Parse an expression; if followed by `<-`, build a send
        // statement; if followed by an assignment op, build an
        // assign statement; else expr-stmt.
        let expr = self.parse_expr()?;
        if matches!(self.peek(), TokenKind::LeftArrow) {
            self.bump();
            let value = self.parse_expr()?;
            let semi = self.expect(TokenKind::Semi, ";")?;
            return Ok(Stmt::Send {
                span: expr.span().merge(semi.span),
                subject: expr,
                value,
            });
        }
        if let Some(op) = self.peek_assign_op() {
            let op_span = self.peek_token().span;
            self.bump();
            let value = self.parse_expr()?;
            let semi = self.expect(TokenKind::Semi, ";")?;
            let target = expr_to_lvalue(expr, op_span)?;
            return Ok(Stmt::Assign {
                span: target.span.merge(semi.span),
                target,
                op,
                value,
            });
        }
        let _ = self.expect(TokenKind::Semi, ";")?;
        Ok(Stmt::Expr(expr))
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
        self.parse_expr_bp(0)
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
                    let name = self.expect_member_name()?;
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
                    elems.push(self.parse_expr()?);
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
        let name = self.expect_ident("field name")?;
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
        TokenKind::Mode => "mode",
        TokenKind::Tier => "tier",
        TokenKind::Projection => "projection",
        TokenKind::Perspective => "perspective",
        TokenKind::Type => "type",
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
}
