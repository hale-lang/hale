//! Type checking — milestone 2 cut.
//!
//! Walks every program in the bundle and verifies a tractable
//! subset of the type rules:
//!
//! - Literal expressions get their natural primitive type.
//! - Binary / unary operator operand-type compatibility.
//! - `let x: T = e;` — e's inferred type assignable to T.
//! - Struct-literal field names + types match the type
//!   declaration.
//! - Bus send (`"subject" <- v`): subject is declared in the
//!   enclosing locus's bus block, payload type matches.
//! - `~~` closure assertion: left and right have compatible
//!   types; tolerance is numeric-ish (we don't enforce strictly
//!   in milestone 2 — just that something is there).
//! - `self.field`: resolves against enclosing locus's params.
//!
//! Names referenced via paths the bundle can't see (stdlib,
//! `time::sleep`, `println`) resolve to `Ty::Unknown`, which
//! is bidirectionally compatible — milestone 2 does not error
//! on these. Milestone 3 will tighten.

use std::collections::BTreeMap;

use aperio_syntax::ast::*;
use aperio_syntax::{Diag, Span};

use crate::resolve::{resolve_type_expr, TopScope};
use crate::symbol::*;
use crate::ty::Ty;

fn method_to_fn_ty(m: &MethodInfo) -> Ty {
    Ty::Function {
        params: m.params.clone(),
        ret: Box::new(m.ret.clone()),
    }
}

/// True if the match arms cover every possible scrutinee
/// value. v0 rules:
///   - Any arm without a guard whose pattern is wildcard `_`
///     or a bare binding makes the match exhaustive.
///   - For Bool scrutinee: literal `true` AND literal `false`
///     arms (both unguarded) is also exhaustive.
///   - For an enum-typed scrutinee (m47): every declared variant
///     must be covered by an unguarded `EnumName::Variant`
///     constructor pattern.
///   - For everything else: a wildcard / binding is required.
fn match_is_exhaustive(scrut_ty: &Ty, arms: &[MatchArm], top: &TopScope) -> bool {
    let unguarded = |a: &&MatchArm| a.guard.is_none();
    let has_catchall = arms.iter().filter(unguarded).any(|a| {
        matches!(a.pattern, Pattern::Wildcard(_) | Pattern::Binding(_))
    });
    if has_catchall {
        return true;
    }
    if matches!(scrut_ty, Ty::Prim(PrimType::Bool)) {
        let mut has_true = false;
        let mut has_false = false;
        for arm in arms.iter().filter(unguarded) {
            if let Pattern::Literal(Literal::Bool(b), _) = &arm.pattern {
                if *b {
                    has_true = true;
                } else {
                    has_false = true;
                }
            }
        }
        return has_true && has_false;
    }
    if let Ty::Named(name) = scrut_ty {
        if let Some(TopSymbol::Type(TypeInfo {
            kind: TypeKind::Enum(variants),
            ..
        })) = top.symbols.get(name)
        {
            let mut covered: std::collections::BTreeSet<&str> =
                std::collections::BTreeSet::new();
            // m68: also accept arms whose enum_seg is a
            // synthesized monomorph of `name` — e.g. arms
            // written as `Result_Int_String::Ok` count as
            // covering `Ok` for a scrutinee typed as the
            // generic `Result` template. Codegen monomorphizes
            // generic enums into mangled-name decls
            // (`Result_Int_String`) but the typechecker only
            // sees the original template, so the user's match
            // arms (which use the mangled names that codegen
            // recognizes) would otherwise false-positive as
            // non-exhaustive. The mangle convention is
            // `<template>_<arg>_<arg>...` so the prefix check
            // is unambiguous.
            let mangle_prefix = format!("{}_", name);
            for arm in arms.iter().filter(unguarded) {
                if let Pattern::Constructor { path, .. } = &arm.pattern {
                    if let [enum_seg, variant_seg] = path.segments.as_slice() {
                        let matches_template_or_monomorph =
                            enum_seg.name == *name
                                || enum_seg
                                    .name
                                    .starts_with(&mangle_prefix);
                        if matches_template_or_monomorph {
                            // m47-payloads: a Constructor arm
                            // covers its variant whether the
                            // sub-patterns are wildcards / bindings
                            // (catch-all over the payload) or
                            // empty (no-payload variant). Literal
                            // sub-patterns are narrower and
                            // wouldn't cover all values of the
                            // variant; we still treat them as
                            // covering for v0.1 — same permissive
                            // policy the Bool literal arms get.
                            covered.insert(variant_seg.name.as_str());
                        }
                    }
                }
            }
            return variants.iter().all(|v| covered.contains(v.name.as_str()));
        }
        // m68: a named type the typechecker doesn't know about
        // at all (commonly: a fully-mangled monomorph that
        // somehow flows in — codegen synthesizes those, the
        // typechecker doesn't see them) should be permissive
        // for exhaustiveness, same as Ty::Unknown. Narrowed
        // to "name not in top.symbols" so known structs / loci
        // / perspectives still require a wildcard / binding arm.
        if !top.symbols.contains_key(name) {
            return true;
        }
    }
    // Be permissive on Unknown — we genuinely can't say.
    matches!(scrut_ty, Ty::Unknown)
}

/// True if `e` is composed entirely of literals — no
/// identifiers, no `self`, no calls, no field access. Used by
/// closure-cycle-existence: a closure assertion with pure-
/// literal sides has nothing to audit.
fn is_pure_literal(e: &Expr) -> bool {
    match e {
        Expr::Literal(_, _) => true,
        Expr::Unary { operand, .. } => is_pure_literal(operand),
        Expr::Binary { left, right, .. } => {
            is_pure_literal(left) && is_pure_literal(right)
        }
        Expr::Tuple(parts, _) | Expr::Array(parts, _) => {
            parts.iter().all(is_pure_literal)
        }
        _ => false,
    }
}

pub fn check_bundle(bundle: &Bundle<'_>, top: &TopScope) -> Vec<Diag> {
    let mut diags = Vec::new();
    let known = collect_known_names(top);
    for program in bundle.programs.values() {
        let mut cx = Checker {
            top,
            known: &known,
            diags: &mut diags,
            locals: ScopeStack::new(),
            current_locus: None,
            in_lifecycle: false,
            in_closure: false,
        };
        for item in &program.items {
            cx.check_top_decl(item);
        }
    }
    diags
}

fn collect_known_names(top: &TopScope) -> BTreeMap<String, Span> {
    let mut m = BTreeMap::new();
    for (name, sym) in &top.symbols {
        if matches!(
            sym,
            TopSymbol::Locus(_) | TopSymbol::Type(_) | TopSymbol::Perspective(_)
        ) {
            m.insert(name.clone(), sym.span());
        }
    }
    m
}

struct Checker<'a> {
    top: &'a TopScope,
    known: &'a BTreeMap<String, Span>,
    diags: &'a mut Vec<Diag>,
    locals: ScopeStack,
    current_locus: Option<&'a LocusInfo>,
    in_lifecycle: bool,
    in_closure: bool,
}

#[derive(Default)]
struct ScopeStack {
    frames: Vec<BTreeMap<String, LocalSym>>,
}

#[derive(Debug, Clone)]
struct LocalSym {
    ty: Ty,
    /// m50: tracks whether the binding was declared with `mut`.
    /// `let x = ...` is immutable; `let mut x = ...` permits
    /// reassignment. Per spec/types.md "Mutability" + design-
    /// rationale §E. Locus state on `self` is mutable
    /// independently (locus fields aren't bindings — they're
    /// state — and lifecycle methods update them through
    /// `self.field = ...` regardless of any binding's is_mut).
    /// Fn params, loop variables, and pattern bindings default
    /// to false: the surface spec says params are immutable,
    /// loop vars rebind fresh each iteration, and pattern arm
    /// bindings exist only for the duration of the arm body.
    is_mut: bool,
}

impl ScopeStack {
    fn new() -> Self {
        Self {
            frames: vec![BTreeMap::new()],
        }
    }
    fn push(&mut self) {
        self.frames.push(BTreeMap::new());
    }
    fn pop(&mut self) {
        self.frames.pop();
    }
    fn insert(&mut self, name: &str, sym: LocalSym) {
        self.frames
            .last_mut()
            .expect("at least one scope")
            .insert(name.to_string(), sym);
    }
    fn lookup(&self, name: &str) -> Option<&LocalSym> {
        for frame in self.frames.iter().rev() {
            if let Some(s) = frame.get(name) {
                return Some(s);
            }
        }
        None
    }
}

impl<'a> Checker<'a> {
    fn check_top_decl(&mut self, decl: &'a TopDecl) {
        match decl {
            TopDecl::Locus(l) => self.check_locus(l),
            TopDecl::Fn(f) => self.check_fn(f, None),
            TopDecl::Const(c) => {
                let want = resolve_type_expr(&c.ty, self.known);
                let got = self.check_expr(&c.value);
                if !want.assignable_from(&got) {
                    self.diags.push(Diag::ty(
                        c.value.span(),
                        format!(
                            "const `{}`: expected `{}`, got `{}`",
                            c.name.name,
                            want.display(),
                            got.display()
                        ),
                    ));
                }
            }
            TopDecl::Module(m) => {
                for item in &m.items {
                    self.check_top_decl(item);
                }
            }
            TopDecl::Type(_) | TopDecl::Perspective(_) => {
                // Structure already validated by resolver; field
                // types are checked when something instantiates
                // them via struct literal.
            }
            TopDecl::Interface(_) => {
                // Interface declarations are pure type-level —
                // method signatures only, no bodies. The resolver
                // collected them; the structural impl-check fires
                // at the use site (call expression where the
                // expected type is an interface).
            }
        }
    }

    fn check_locus(&mut self, decl: &'a LocusDecl) {
        let info = match self.top.lookup(&decl.name.name) {
            Some(TopSymbol::Locus(info)) => info,
            _ => return,
        };
        let prev = self.current_locus.replace(info);

        // Validate that bus-subscribe handlers are declared on
        // the locus body (as fn members).
        let fn_members: BTreeMap<String, &FnDecl> = decl
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Fn(f) => Some((f.name.name.clone(), f)),
                _ => None,
            })
            .collect();
        for sub in &info.bus_subscribes {
            if !fn_members.contains_key(&sub.handler) {
                self.diags.push(Diag::ty(
                    sub.span,
                    format!(
                        "bus subscribe `{}` references handler `{}` which is \
                         not declared on locus `{}`",
                        sub.subject, sub.handler, info.name
                    ),
                ));
            }
        }

        // F.8: contract compatibility. If this locus consumes
        // fields from coordinatees, the accept-child type
        // must expose each consumed field at a compatible
        // type. The check fires once per parent locus; the
        // child's expose-set must be a superset (by name) of
        // the parent's consume-set, with assignable types.
        if !info.contract_consume.is_empty() {
            self.check_contract_compatibility(info);
        }

        for member in &decl.members {
            self.check_locus_member(member);
        }

        self.current_locus = prev;
    }

    fn check_contract_compatibility(&mut self, parent: &LocusInfo) {
        let child_name = match &parent.accept_param {
            Some((_, Ty::Named(n))) => n.clone(),
            Some((_, _)) => return, // non-named child type → can't statically resolve
            None => {
                // Parent declares consume but doesn't accept any
                // child. Static error per F.8 — the consume
                // surface has nothing to bind against.
                for entry in &parent.contract_consume {
                    self.diags.push(Diag::ty(
                        entry.span,
                        format!(
                            "locus `{}`: contract consumes `{}` but declares no \
                             `accept(_: ChildType)` to bind against",
                            parent.name, entry.name
                        ),
                    ));
                }
                return;
            }
        };
        let child = match self.top.lookup(&child_name) {
            Some(TopSymbol::Locus(c)) => c,
            _ => return, // unresolved child type — separate error already raised
        };
        for need in &parent.contract_consume {
            match child
                .contract_expose
                .iter()
                .find(|e| e.name == need.name)
            {
                Some(have) => {
                    if !need.ty.assignable_from(&have.ty) {
                        self.diags.push(Diag::ty(
                            need.span,
                            format!(
                                "contract: locus `{}` consumes `{}: {}`, but child \
                                 locus `{}` exposes it as `{}`",
                                parent.name,
                                need.name,
                                need.ty.display(),
                                child.name,
                                have.ty.display()
                            ),
                        ));
                    }
                }
                None => {
                    self.diags.push(Diag::ty(
                        need.span,
                        format!(
                            "contract: locus `{}` consumes `{}` but child locus \
                             `{}` does not expose it",
                            parent.name, need.name, child.name
                        ),
                    ));
                }
            }
        }
    }

    fn check_locus_member(&mut self, member: &'a LocusMember) {
        match member {
            LocusMember::Params(_) | LocusMember::Contract(_) | LocusMember::Bus(_) => {
                // Already lowered by resolver; param defaults are
                // checked against declared types implicitly when
                // the param is referenced. (Milestone-2 cut: no
                // default-vs-declared-type re-check here.)
            }
            LocusMember::Lifecycle(lc) => {
                self.in_lifecycle = true;
                self.locals.push();
                for p in &lc.params {
                    let ty = resolve_type_expr(&p.ty, self.known);
                    self.locals.insert(&p.name.name, LocalSym { ty, is_mut: false });
                }
                self.check_block(&lc.body);
                self.locals.pop();
                self.in_lifecycle = false;
            }
            LocusMember::Mode(md) => {
                self.in_lifecycle = true;
                self.locals.push();
                for p in &md.params {
                    let ty = resolve_type_expr(&p.ty, self.known);
                    self.locals.insert(&p.name.name, LocalSym { ty, is_mut: false });
                }
                self.check_block(&md.body);
                self.locals.pop();
                self.in_lifecycle = false;
            }
            LocusMember::Failure(fd) => {
                self.in_lifecycle = true;
                self.locals.push();
                for p in &fd.params {
                    let ty = resolve_type_expr(&p.ty, self.known);
                    self.locals.insert(&p.name.name, LocalSym { ty, is_mut: false });
                }
                self.check_block(&fd.body);
                self.locals.pop();
                self.in_lifecycle = false;
            }
            LocusMember::Closure(cd) => {
                self.in_closure = true;
                self.in_lifecycle = true;
                let lt = self.check_expr(&cd.assertion.left);
                let rt = self.check_expr(&cd.assertion.right);
                if !lt.assignable_from(&rt) && !rt.assignable_from(&lt) {
                    self.diags.push(Diag::ty(
                        cd.assertion.span,
                        format!(
                            "closure `{}`: assertion sides have incompatible types \
                             `{}` and `{}`",
                            cd.name.name,
                            lt.display(),
                            rt.display()
                        ),
                    ));
                }
                // Cycle-existence: at least one side of the
                // assertion must observe runtime-varying state
                // (self, locals, method calls). Two pure-literal
                // sides means the assertion has nothing to
                // audit — either always passes or always fails.
                if is_pure_literal(&cd.assertion.left)
                    && is_pure_literal(&cd.assertion.right)
                {
                    self.diags.push(Diag::ty(
                        cd.assertion.span,
                        format!(
                            "closure `{}`: both assertion sides are pure literals; \
                             a closure must observe at least one runtime-varying \
                             value (e.g. `self.x`) to audit anything",
                            cd.name.name
                        ),
                    ));
                }
                let _ = self.check_expr(&cd.assertion.tolerance);
                self.in_lifecycle = false;
                self.in_closure = false;
            }
            LocusMember::Fn(f) => {
                self.in_lifecycle = true;
                self.check_fn(f, self.current_locus);
                self.in_lifecycle = false;
            }
            LocusMember::Const(c) => {
                let want = resolve_type_expr(&c.ty, self.known);
                let got = self.check_expr(&c.value);
                if !want.assignable_from(&got) {
                    self.diags.push(Diag::ty(
                        c.value.span(),
                        format!(
                            "const `{}`: expected `{}`, got `{}`",
                            c.name.name,
                            want.display(),
                            got.display()
                        ),
                    ));
                }
            }
            LocusMember::Type(_) => {}
        }
    }

    fn check_fn(&mut self, decl: &'a FnDecl, locus: Option<&'a LocusInfo>) {
        let prev_locus = self.current_locus;
        if locus.is_some() {
            self.current_locus = locus;
        }
        self.locals.push();
        for p in &decl.params {
            let ty = resolve_type_expr(&p.ty, self.known);
            self.locals.insert(&p.name.name, LocalSym { ty, is_mut: false });
        }
        self.check_block(&decl.body);
        self.locals.pop();
        self.current_locus = prev_locus;
    }

    fn check_block(&mut self, block: &Block) {
        self.locals.push();
        for stmt in &block.stmts {
            self.check_stmt(stmt);
        }
        self.locals.pop();
    }

    fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { is_mut, name, ty, value, .. } => {
                let got = self.check_expr(value);
                let bound = match ty {
                    Some(te) => {
                        let want = resolve_type_expr(te, self.known);
                        if !want.assignable_from(&got) {
                            self.diags.push(Diag::ty(
                                value.span(),
                                format!(
                                    "let `{}`: expected `{}`, got `{}`",
                                    name.name,
                                    want.display(),
                                    got.display()
                                ),
                            ));
                        }
                        want
                    }
                    None => got,
                };
                self.locals.insert(
                    &name.name,
                    LocalSym { ty: bound, is_mut: *is_mut },
                );
            }
            Stmt::LetTuple { is_mut, names, ty, value, .. } => {
                let got = self.check_expr(value);
                let elem_tys: Vec<Ty> = match (&got, ty) {
                    (Ty::Tuple(parts), _) if parts.len() == names.len() => {
                        parts.clone()
                    }
                    (Ty::Tuple(parts), _) => {
                        self.diags.push(Diag::ty(
                            value.span(),
                            format!(
                                "let-tuple: expected {} elements, got `{}`",
                                names.len(),
                                got.display()
                            ),
                        ));
                        // Best-effort: pad / truncate so subsequent
                        // typechecking can still proceed.
                        let mut v = parts.clone();
                        v.resize(names.len(), Ty::Unknown);
                        v
                    }
                    (other, _) => {
                        self.diags.push(Diag::ty(
                            value.span(),
                            format!(
                                "let-tuple: rhs is `{}`, not a tuple",
                                other.display()
                            ),
                        ));
                        vec![Ty::Unknown; names.len()]
                    }
                };
                for (n, t) in names.iter().zip(elem_tys.iter()) {
                    self.locals.insert(
                        &n.name,
                        LocalSym { ty: t.clone(), is_mut: *is_mut },
                    );
                }
            }
            Stmt::Assign { target, value, span, .. } => {
                let got = self.check_expr(value);
                let want = self.lvalue_ty(target);
                if !want.assignable_from(&got) {
                    self.diags.push(Diag::ty(
                        value.span(),
                        format!(
                            "assignment: target type `{}` not assignable from `{}`",
                            want.display(),
                            got.display()
                        ),
                    ));
                }
                // m50: bare-head reassignment to a non-mut local is
                // a compile-time error per spec/types.md "Mutability"
                // + design-rationale §E. Field/index segments
                // (`x.field = ...`, `x[i] = ...`) don't rebind the
                // local — they mutate state through it — so they
                // stay allowed even when the head binding is
                // immutable. `self.field = ...` is also allowed
                // because `self` is locus state, not a binding.
                if target.tail.is_empty() && target.head.name != "self" {
                    if let Some(sym) = self.locals.lookup(&target.head.name) {
                        if !sym.is_mut {
                            self.diags.push(Diag::ty(
                                *span,
                                format!(
                                    "cannot assign to `{}`: binding is \
                                     immutable. Declare with `let mut {}` \
                                     to permit reassignment.",
                                    target.head.name, target.head.name
                                ),
                            ));
                        }
                    }
                }
            }
            Stmt::Send { subject, value, span } => {
                self.check_send(subject, value, *span);
            }
            Stmt::If(if_stmt) => self.check_if(if_stmt),
            Stmt::Match(m) => self.check_match(m),
            Stmt::For { name, iter, body, .. } => {
                let _ = self.check_expr(iter);
                self.locals.push();
                self.locals.insert(&name.name, LocalSym { ty: Ty::Unknown, is_mut: false });
                self.check_block(body);
                self.locals.pop();
            }
            Stmt::While { cond, body, .. } => {
                let ct = self.check_expr(cond);
                if !ct.assignable_from(&Ty::Prim(PrimType::Bool)) {
                    self.diags.push(Diag::ty(
                        cond.span(),
                        format!(
                            "while condition must be Bool; got `{}`",
                            ct.display()
                        ),
                    ));
                }
                self.check_block(body);
            }
            Stmt::Return(expr, _) => {
                if let Some(e) = expr {
                    let _ = self.check_expr(e);
                }
            }
            Stmt::Break(_) | Stmt::Continue(_) | Stmt::Yield(_) => {}
            Stmt::Block(b) => self.check_block(b),
            Stmt::Recovery { args, modifier, .. } => {
                for a in args {
                    let _ = self.check_expr(a);
                }
                if let Some(RecoveryModifier::For(e) | RecoveryModifier::Until(e)) = modifier {
                    let _ = self.check_expr(e);
                }
            }
            Stmt::Expr(e) => {
                let _ = self.check_expr(e);
            }
        }
    }

    fn check_if(&mut self, stmt: &IfStmt) {
        let ct = self.check_expr(&stmt.cond);
        if !ct.assignable_from(&Ty::Prim(PrimType::Bool)) {
            self.diags.push(Diag::ty(
                stmt.cond.span(),
                format!("if condition must be Bool; got `{}`", ct.display()),
            ));
        }
        self.check_block(&stmt.then_block);
        if let Some(else_branch) = &stmt.else_block {
            match else_branch.as_ref() {
                ElseBranch::Else(b) => self.check_block(b),
                ElseBranch::ElseIf(s) => self.check_if(s),
            }
        }
    }

    fn check_match(&mut self, stmt: &MatchStmt) {
        let scrut_ty = self.check_expr(&stmt.scrutinee);
        for arm in &stmt.arms {
            if let Some(g) = &arm.guard {
                let _ = self.check_expr(g);
            }
            match &arm.body {
                MatchArmBody::Expr(e) => {
                    let _ = self.check_expr(e);
                }
                MatchArmBody::Block(b) => self.check_block(b),
            }
        }
        if !match_is_exhaustive(&scrut_ty, &stmt.arms, self.top) {
            self.diags.push(Diag::ty(
                stmt.span,
                format!(
                    "match is not exhaustive; add a `_` arm or cover all \
                     cases of `{}`",
                    scrut_ty.display()
                ),
            ));
        }
    }

    fn check_send(&mut self, subject: &Expr, value: &Expr, span: Span) {
        let payload_ty = self.check_expr(value);
        let subject_str = match subject {
            Expr::Literal(Literal::String(s), _) => Some(s.clone()),
            _ => None,
        };
        let locus = match self.current_locus {
            Some(l) => l,
            None => {
                self.diags.push(Diag::ty(
                    span,
                    "bus send (`<-`) only valid inside a locus body".to_string(),
                ));
                return;
            }
        };
        // m94: a non-literal subject is allowed when the locus
        // declares a wildcard `publish` whose payload matches.
        // The wildcard declaration acts as the authorization +
        // type-binding for any concrete subject computed at
        // runtime that matches the pattern. Static subject-pattern
        // verification is impossible by definition; we trust the
        // declaration and let runtime dispatch route to whichever
        // subscribers (exact or wildcard) match.
        let subject_str = match subject_str {
            Some(s) => s,
            None => {
                let wildcard_match = locus.bus_publishes.iter().find(|p| {
                    p.subject.contains("**")
                        && p.payload.assignable_from(&payload_ty)
                });
                if wildcard_match.is_none() {
                    let any_wildcard = locus
                        .bus_publishes
                        .iter()
                        .any(|p| p.subject.contains("**"));
                    if any_wildcard {
                        self.diags.push(Diag::ty(
                            value.span(),
                            format!(
                                "bus send (computed subject): payload `{}` does \
                                 not match any wildcard publish declaration in \
                                 locus `{}`",
                                payload_ty.display(),
                                locus.name
                            ),
                        ));
                    } else {
                        self.diags.push(Diag::ty(
                            subject.span(),
                            format!(
                                "bus send with computed subject requires a \
                                 wildcard `publish` declaration (e.g. \
                                 `publish \"log.**\" of type T`) in locus `{}`",
                                locus.name
                            ),
                        ));
                    }
                }
                return;
            }
        };
        let pub_decl = locus
            .bus_publishes
            .iter()
            .find(|p| p.subject == subject_str);
        match pub_decl {
            Some(decl) => {
                if !decl.payload.assignable_from(&payload_ty) {
                    self.diags.push(Diag::ty(
                        value.span(),
                        format!(
                            "bus send `{}`: payload `{}` not assignable to declared `{}`",
                            subject_str,
                            payload_ty.display(),
                            decl.payload.display()
                        ),
                    ));
                }
            }
            None => {
                // m94: an exact-literal subject is also valid when
                // it matches a wildcard publish declaration of the
                // right type. This lets a locus declare
                // `publish "log.**" of type LogEvent` once and
                // then send on `"log.app"` etc. literally.
                let wildcard_match = locus.bus_publishes.iter().find(|p| {
                    p.subject.contains("**")
                        && super::wildcard_match(&p.subject, &subject_str)
                        && p.payload.assignable_from(&payload_ty)
                });
                if wildcard_match.is_none() {
                    self.diags.push(Diag::ty(
                        subject.span(),
                        format!(
                            "bus send subject `{}` is not declared in locus `{}`'s bus block",
                            subject_str, locus.name
                        ),
                    ));
                }
            }
        }
    }

    fn lvalue_ty(&mut self, lv: &LValue) -> Ty {
        let mut ty = if lv.head.name == "self" {
            self.self_ty()
        } else if let Some(s) = self.locals.lookup(&lv.head.name) {
            s.ty.clone()
        } else {
            Ty::Unknown
        };
        for seg in &lv.tail {
            match seg {
                LValueSeg::Field(f) => {
                    ty = self.field_ty(&ty, &f.name).unwrap_or(Ty::Unknown);
                }
                LValueSeg::Index(idx) => {
                    let _ = self.check_expr(idx);
                    ty = match ty {
                        Ty::Array(elem, _) => *elem,
                        _ => Ty::Unknown,
                    };
                }
            }
        }
        ty
    }

    fn self_ty(&self) -> Ty {
        match self.current_locus {
            Some(l) => Ty::Named(l.name.clone()),
            None => Ty::Unknown,
        }
    }

    /// Look up a named field on a type. Resolves struct fields,
    /// locus params (when accessing a locus handle's exposed
    /// state — milestone 2 just exposes all params), and
    /// perspective params.
    /// Verify that a locus structurally implements an interface:
    /// for every method the interface declares, the locus has a
    /// method with the same name, same arity, compatible param
    /// types, and a compatible return type. Returns Err with a
    /// human-readable message on the first mismatch.
    ///
    /// Both arguments are top-symbol names. Caller has already
    /// verified that `iface_name` resolves to a TopSymbol::Interface.
    /// `locus_name` may be any TopSymbol — non-locus returns Err.
    fn check_structural_impl(
        &self,
        locus_name: &str,
        iface_name: &str,
    ) -> Result<(), String> {
        let iface = match self.top.lookup(iface_name) {
            Some(TopSymbol::Interface(i)) => i,
            _ => return Ok(()),
        };
        let locus = match self.top.lookup(locus_name) {
            Some(TopSymbol::Locus(l)) => l,
            _ => {
                return Err(format!(
                    "type `{}` cannot satisfy interface `{}` — only loci satisfy interfaces",
                    locus_name, iface_name
                ));
            }
        };
        for im in &iface.methods {
            let lm = locus.methods.iter().find(|lm| lm.name == im.name);
            let lm = match lm {
                Some(m) => m,
                None => {
                    return Err(format!(
                        "locus `{}` does not satisfy interface `{}`: missing method `{}`",
                        locus_name, iface_name, im.name
                    ));
                }
            };
            if lm.params.len() != im.params.len() {
                return Err(format!(
                    "locus `{}` method `{}` arity does not match interface `{}`: expected {} arg(s), locus has {}",
                    locus_name,
                    im.name,
                    iface_name,
                    im.params.len(),
                    lm.params.len()
                ));
            }
            for (i, (lp, ip)) in
                lm.params.iter().zip(im.params.iter()).enumerate()
            {
                let want = &ip.1;
                if !want.assignable_from(lp) {
                    return Err(format!(
                        "locus `{}` method `{}` arg #{} type mismatch: interface `{}` requires `{}`, locus has `{}`",
                        locus_name,
                        im.name,
                        i,
                        iface_name,
                        want.display(),
                        lp.display()
                    ));
                }
            }
            if !im.ret.assignable_from(&lm.ret) {
                return Err(format!(
                    "locus `{}` method `{}` return type mismatch: interface `{}` requires `{}`, locus returns `{}`",
                    locus_name,
                    im.name,
                    iface_name,
                    im.ret.display(),
                    lm.ret.display()
                ));
            }
        }
        Ok(())
    }

    fn field_ty(&self, ty: &Ty, name: &str) -> Option<Ty> {
        match ty {
            // Numeric tuple field access: `t.0`, `t.1`. Parser
            // stores the digit string as the field name, so we
            // recognize it as a usize index here.
            Ty::Tuple(parts) => {
                if let Ok(i) = name.parse::<usize>() {
                    if i < parts.len() {
                        return Some(parts[i].clone());
                    }
                }
                None
            }
            Ty::Named(n) => match self.top.lookup(n)? {
                TopSymbol::Type(info) => match &info.kind {
                    TypeKind::Struct(fields) => fields
                        .iter()
                        .find(|f| f.name == name)
                        .map(|f| f.ty.clone()),
                    TypeKind::Alias(t) => self.field_ty(t, name),
                    TypeKind::Enum(_) => None,
                },
                TopSymbol::Locus(info) => {
                    if name == "children" {
                        return Some(match &info.accept_param {
                            Some((_, t)) => Ty::Array(Box::new(t.clone()), None),
                            None => Ty::Array(Box::new(Ty::Unknown), None),
                        });
                    }
                    if name == "k_max" {
                        // F.1: k_max = B / [(1-phi)c + phi*sigma].
                        // Fractional in general; Float regardless of
                        // whether B/c/sigma are Int (the divisor is
                        // a phi-weighted blend).
                        return Some(Ty::Prim(PrimType::Float));
                    }
                    if let Some(p) = info.params.iter().find(|p| p.name == name) {
                        return Some(p.ty.clone());
                    }
                    info.methods
                        .iter()
                        .find(|m| m.name == name)
                        .map(method_to_fn_ty)
                }
                TopSymbol::Perspective(info) => {
                    if let Some(p) = info.params.iter().find(|p| p.name == name) {
                        return Some(p.ty.clone());
                    }
                    info.methods
                        .iter()
                        .find(|m| m.name == name)
                        .map(method_to_fn_ty)
                }
                _ => None,
            },
            Ty::Unknown => Some(Ty::Unknown),
            _ => None,
        }
    }

    fn check_expr(&mut self, expr: &Expr) -> Ty {
        match expr {
            Expr::Literal(lit, _) => lit_ty(lit),
            Expr::Ident(id) => {
                if let Some(s) = self.locals.lookup(&id.name) {
                    s.ty.clone()
                } else if let Some(sym) = self.top.lookup(&id.name) {
                    match sym {
                        TopSymbol::Const(c) => c.ty.clone(),
                        TopSymbol::Fn(sig) => Ty::Function {
                            params: sig.params.iter().map(|(_, t)| t.clone()).collect(),
                            ret: Box::new(sig.ret.clone()),
                        },
                        // Locus / Type / Perspective / Interface
                        // names used in expression position resolve
                        // to the type (struct-literal, call site,
                        // or interface-typed binding).
                        TopSymbol::Locus(_)
                        | TopSymbol::Type(_)
                        | TopSymbol::Perspective(_)
                        | TopSymbol::Interface(_) => Ty::Named(id.name.clone()),
                    }
                } else {
                    Ty::Unknown
                }
            }
            Expr::Path(qn) => {
                // m47-followup: 2-segment path may be an enum
                // variant construction (`EnumName::VariantName`).
                // Resolve to the enum type so let-bindings,
                // tuple/array literals, and struct fields can
                // unify against the declared shape rather than
                // falling through to Unknown (which made `let x:
                // Color = Color::Red;` fail with `expected Color,
                // got ?`).
                if qn.segments.len() == 2 {
                    let enum_name = &qn.segments[0].name;
                    let variant_name = &qn.segments[1].name;
                    if let Some(TopSymbol::Type(TypeInfo {
                        kind: TypeKind::Enum(variants),
                        ..
                    })) = self.top.symbols.get(enum_name)
                    {
                        if variants.iter().any(|v| v.name == *variant_name) {
                            return Ty::Named(enum_name.clone());
                        }
                    }
                }
                Ty::Unknown
            }
            Expr::Path2 { .. } => Ty::Unknown,
            Expr::KwSelf(span) => {
                if self.current_locus.is_none() {
                    self.diags.push(Diag::ty(
                        *span,
                        "`self` used outside a locus body".to_string(),
                    ));
                }
                self.self_ty()
            }
            Expr::Binary { op, left, right, span } => {
                let lt = self.check_expr(left);
                let rt = self.check_expr(right);
                self.binop_ty(*op, &lt, &rt, *span)
            }
            Expr::Unary { op, operand, .. } => {
                let t = self.check_expr(operand);
                match op {
                    UnaryOp::Neg | UnaryOp::BitNot => t,
                    UnaryOp::Not => Ty::Prim(PrimType::Bool),
                }
            }
            Expr::Call { callee, args, .. } => {
                // m47-payloads: enum-variant construction with
                // args. `EnumName::Variant(..)` resolves to the
                // enum's named type. We still walk the args to
                // surface their own type errors, but don't unify
                // them against declared field types yet — codegen
                // performs that strict check, and the typechecker
                // is permissive on Unknowns elsewhere.
                if let Expr::Path(qn) = callee.as_ref() {
                    if qn.segments.len() == 2 {
                        let enum_name = &qn.segments[0].name;
                        let variant_name = &qn.segments[1].name;
                        if let Some(TopSymbol::Type(TypeInfo {
                            kind: TypeKind::Enum(variants),
                            ..
                        })) = self.top.symbols.get(enum_name)
                        {
                            if variants.iter().any(|v| v.name == *variant_name) {
                                for a in args {
                                    let _ = self.check_expr(a);
                                }
                                return Ty::Named(enum_name.clone());
                            }
                        }
                    }
                }
                let callee_ty = self.check_expr(callee);
                let arg_tys: Vec<Ty> = args.iter().map(|a| self.check_expr(a)).collect();
                // F.20: when a fn param is an interface type, the
                // arg's locus type must structurally satisfy the
                // interface (have the required methods with
                // compatible signatures). Permissive on Unknown,
                // permissive on shape mismatch — the existing
                // checker doesn't enforce arg-vs-param positional
                // typing in general; this fires *only* when the
                // param is an interface, so we don't widen the
                // call-site checking surface beyond that.
                if let Ty::Function { params, .. } = &callee_ty {
                    for (i, (param_ty, arg_ty)) in
                        params.iter().zip(arg_tys.iter()).enumerate()
                    {
                        if let (Ty::Named(iface_name), Ty::Named(arg_name)) =
                            (param_ty, arg_ty)
                        {
                            // Look up param-named symbol; only
                            // check if it actually resolves to an
                            // interface (not a locus / type /
                            // perspective).
                            let is_iface = matches!(
                                self.top.lookup(iface_name),
                                Some(TopSymbol::Interface(_))
                            );
                            if is_iface {
                                if let Err(msg) =
                                    self.check_structural_impl(arg_name, iface_name)
                                {
                                    let span = args
                                        .get(i)
                                        .map(|e| e.span())
                                        .unwrap_or_else(|| Span::new(0, 0));
                                    self.diags.push(Diag::ty(span, msg));
                                }
                            }
                        }
                    }
                }
                match callee_ty {
                    Ty::Function { ret, .. } => *ret,
                    _ => Ty::Unknown,
                }
            }
            Expr::Field { receiver, name, span } => {
                let rt = self.check_expr(receiver);
                match self.field_ty(&rt, &name.name) {
                    Some(t) => t,
                    None => {
                        // Permissive on Unknown — stdlib paths
                        // and externally-typed values pass
                        // through. Strict when the receiver
                        // is a known type and the field
                        // doesn't exist on it: catches typos
                        // statically.
                        if !matches!(rt, Ty::Unknown) {
                            self.diags.push(Diag::ty(
                                *span,
                                format!(
                                    "no field `{}` on `{}`",
                                    name.name,
                                    rt.display()
                                ),
                            ));
                        }
                        Ty::Unknown
                    }
                }
            }
            Expr::Index { receiver, index, .. } => {
                let rt = self.check_expr(receiver);
                let _ = self.check_expr(index);
                match rt {
                    Ty::Array(elem, _) => *elem,
                    _ => Ty::Unknown,
                }
            }
            Expr::Tuple(parts, _) => {
                Ty::Tuple(parts.iter().map(|e| self.check_expr_local(e)).collect())
            }
            Expr::Array(parts, _) => {
                let elem = if let Some(first) = parts.first() {
                    self.check_expr_local(first)
                } else {
                    Ty::Unknown
                };
                for e in parts.iter().skip(1) {
                    let _ = self.check_expr(e);
                }
                Ty::Array(Box::new(elem), Some(parts.len() as u64))
            }
            Expr::Struct { path, inits, span } => self.check_struct_literal(path, inits, *span),
            Expr::Block(b) => {
                self.check_block(b);
                Ty::Unit
            }
            Expr::If(s) => {
                self.check_if(s);
                Ty::Unit
            }
            Expr::Match(m) => {
                self.check_match(m);
                Ty::Unit
            }
            Expr::Sum(inner, _) | Expr::Prod(inner, _) => self.check_expr(inner),
            Expr::Approx { left, right, tolerance, span } => {
                if !self.in_closure {
                    self.diags.push(Diag::ty(
                        *span,
                        "approximate-equality (`~~`) only valid inside a closure block"
                            .to_string(),
                    ));
                }
                let _ = self.check_expr(left);
                let _ = self.check_expr(right);
                let _ = self.check_expr(tolerance);
                Ty::Prim(PrimType::Bool)
            }
            Expr::Range { lo, hi, .. } => {
                // v0 ranges are integer iterators only. Both sides
                // must be Int. The expression itself doesn't have a
                // first-class type beyond "iterator over Int" — the
                // for-stmt handler is the only consumer that
                // recognizes it. Returning Unknown lets callers in
                // non-iterator positions still typecheck without
                // the result being used as a value.
                let _ = self.check_expr(lo);
                let _ = self.check_expr(hi);
                Ty::Unknown
            }
        }
    }

    /// Same as check_expr but used when we need a type without
    /// risking borrow conflicts with the recursion. (In practice
    /// it's identical; named to mark intent at the call sites.)
    fn check_expr_local(&mut self, expr: &Expr) -> Ty {
        self.check_expr(expr)
    }

    /// Whether a value of type `t` can be auto-coerced to String
    /// inside a `String + <t>` expression. Mirrors the codegen
    /// `value_to_string_supports` set: every primitive that
    /// `to_string(...)` accepts, plus enums (which render as their
    /// variant name).
    fn ty_is_printable(t: &Ty) -> bool {
        match t {
            Ty::Prim(p) => matches!(
                p,
                PrimType::String
                    | PrimType::Int
                    | PrimType::Bool
                    | PrimType::Float
                    | PrimType::Decimal
                    | PrimType::Duration
                    | PrimType::Time
            ),
            // Named types: enums render via to_string at codegen.
            // The typechecker doesn't distinguish enum vs struct
            // here without more lookup work; permit and let
            // codegen reject if the type isn't actually printable
            // (struct with no Display rendering would still error
            // there).
            Ty::Named(_) => true,
            _ => false,
        }
    }

    fn binop_ty(&mut self, op: BinOp, lt: &Ty, rt: &Ty, span: Span) -> Ty {
        use BinOp::*;
        // Ergonomics arc: `String + <printable>` and the symmetric
        // form auto-coerce in codegen via value_to_string. The
        // typechecker mirrors that by short-circuiting on the
        // mixed-String add as a permitted shape that yields String.
        if matches!(op, Add) {
            let l_str = matches!(lt, Ty::Prim(PrimType::String));
            let r_str = matches!(rt, Ty::Prim(PrimType::String));
            if (l_str && Self::ty_is_printable(rt))
                || (r_str && Self::ty_is_printable(lt))
            {
                return Ty::Prim(PrimType::String);
            }
        }
        match op {
            Add | Sub | Mul | Div | Mod | BitAnd | BitOr | BitXor | Shl | Shr => {
                if !lt.assignable_from(rt) && !rt.assignable_from(lt) {
                    self.diags.push(Diag::ty(
                        span,
                        format!(
                            "binary op: incompatible operand types `{}` and `{}`",
                            lt.display(),
                            rt.display()
                        ),
                    ));
                }
                if matches!(lt, Ty::Unknown) {
                    rt.clone()
                } else {
                    lt.clone()
                }
            }
            Eq | NotEq | Lt | Gt | LtEq | GtEq => {
                if !lt.assignable_from(rt) && !rt.assignable_from(lt) {
                    self.diags.push(Diag::ty(
                        span,
                        format!(
                            "comparison: incompatible operand types `{}` and `{}`",
                            lt.display(),
                            rt.display()
                        ),
                    ));
                }
                Ty::Prim(PrimType::Bool)
            }
            And | Or => Ty::Prim(PrimType::Bool),
        }
    }

    fn check_struct_literal(
        &mut self,
        path: &QualifiedName,
        inits: &[StructInit],
        span: Span,
    ) -> Ty {
        if path.segments.len() != 1 {
            for init in inits {
                let _ = self.check_expr(&init.value);
            }
            return Ty::Unknown;
        }
        let name = &path.segments[0].name;
        let sym = match self.top.lookup(name) {
            Some(s) => s,
            None => {
                self.diags.push(Diag::ty(
                    span,
                    format!("unknown type `{}` in struct/locus literal", name),
                ));
                for init in inits {
                    let _ = self.check_expr(&init.value);
                }
                return Ty::Unknown;
            }
        };

        let (fields, kind_label, requires_all): (Vec<(String, Ty, bool)>, &str, bool) = match sym {
            TopSymbol::Type(info) => match &info.kind {
                TypeKind::Struct(fields) => (
                    fields
                        .iter()
                        .map(|f| (f.name.clone(), f.ty.clone(), f.has_default))
                        .collect(),
                    "type",
                    true,
                ),
                _ => {
                    self.diags.push(Diag::ty(
                        span,
                        format!("`{}` is not a struct type", name),
                    ));
                    return Ty::Unknown;
                }
            },
            TopSymbol::Locus(info) => (
                info.params
                    .iter()
                    .map(|p| (p.name.clone(), p.ty.clone(), p.has_default))
                    .collect(),
                "locus",
                false,
            ),
            TopSymbol::Perspective(info) => (
                info.params
                    .iter()
                    .map(|p| (p.name.clone(), p.ty.clone(), p.has_default))
                    .collect(),
                "perspective",
                false,
            ),
            _ => {
                self.diags.push(Diag::ty(
                    span,
                    format!("`{}` cannot be instantiated with `{{...}}`", name),
                ));
                return Ty::Unknown;
            }
        };

        let mut seen: BTreeMap<String, ()> = BTreeMap::new();
        for init in inits {
            let got = self.check_expr(&init.value);
            match fields.iter().find(|(n, _, _)| n == &init.name.name) {
                Some((_, want, _)) => {
                    if !want.assignable_from(&got) {
                        self.diags.push(Diag::ty(
                            init.value.span(),
                            format!(
                                "{} `{}`: field `{}` expects `{}`, got `{}`",
                                kind_label,
                                name,
                                init.name.name,
                                want.display(),
                                got.display()
                            ),
                        ));
                    }
                }
                None => {
                    self.diags.push(Diag::ty(
                        init.span,
                        format!(
                            "{} `{}` has no field `{}`",
                            kind_label, name, init.name.name
                        ),
                    ));
                }
            }
            seen.insert(init.name.name.clone(), ());
        }
        if requires_all {
            for (fname, _, has_default) in &fields {
                if !seen.contains_key(fname) && !has_default {
                    self.diags.push(Diag::ty(
                        span,
                        format!(
                            "{} `{}`: missing field `{}`",
                            kind_label, name, fname
                        ),
                    ));
                }
            }
        }

        Ty::Named(name.clone())
    }
}

fn lit_ty(lit: &Literal) -> Ty {
    match lit {
        Literal::Int(_) => Ty::Prim(PrimType::Int),
        Literal::Float(_) => Ty::Prim(PrimType::Float),
        Literal::Decimal(_) => Ty::Prim(PrimType::Decimal),
        Literal::String(_) => Ty::Prim(PrimType::String),
        Literal::Bool(_) => Ty::Prim(PrimType::Bool),
        Literal::Nil => Ty::Unknown,
        Literal::Duration(_) => Ty::Prim(PrimType::Duration),
        Literal::Time(_) => Ty::Prim(PrimType::Time),
        Literal::Bytes(_) => Ty::Prim(PrimType::Bytes),
    }
}
