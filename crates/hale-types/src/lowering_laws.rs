//! The `law_backstops` family (F.40 phase 3, C7): the laws lowering used
//! to re-judge for itself.
//!
//! Lowering kept a spanless `CodegenError::Unsupported` for a rule the
//! checker states, because the test harness's snapshot
//! (`Config::harness`) lowers what it is handed without a check. Each
//! such refusal moves here once a law covers every program it refused,
//! located and reading the rows the families produce; then the refusal
//! is deleted. [`lowering_laws`] is the one entry: the check runs it
//! beside its other rules, and the harness's lowering view demands it
//! before it lowers, so a program a law refuses is refused at every
//! entry point, with the law's span and wording, and lowering judges it
//! nowhere.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{
    Block, ElseBranch, EpochSpec, Expr, FnDecl, IfStmt, LValueSeg, LifecycleKind, LocusDecl, LocusMember,
    MatchArmBody, MatchStmt, OrDisposition, ParamInit, ParamsBlock, PerspectiveMember, Program, RecoveryModifier, Stmt,
    StructInit, TopDecl, TransportSpec, TypeDecl, TypeDeclBody, TypeExpr,
};
use hale_syntax::{Diag, Span};

use crate::binding_rows::BindingRows;
use crate::ownership_graph::OwnershipGraph;
use crate::placement::{Decision, DomainKind, Origin, PerUsePosition, PlacementTable, SiteRef, SiteUniverse};
use crate::snapshot::Snapshot;
use crate::Bundle;

/// The rows the laws read.
pub struct LoweringLawInputs<'a> {
    /// The placement table: which instance runs pinned, what it
    /// realizes, and what decided it.
    pub placement: &'a PlacementTable,
    /// The binding rows: the topic an adapter's binding entry names.
    pub bindings: &'a BindingRows,
    /// The ownership graph, on request: which instantiation sites bubble
    /// to an owner on another thread. Asked for only when the placement
    /// table runs something off the main thread.
    pub ownership: &'a dyn Fn() -> Option<&'a OwnershipGraph>,
}

/// Every law that replaced a lowering backstop, over `bundle`.
pub fn lowering_laws(bundle: &Bundle<'_>, inputs: &LoweringLawInputs<'_>) -> Vec<Diag> {
    let mut diags = Vec::new();
    pinned_features(bundle, inputs, &mut diags);
    pinned_root_in_a_loop(bundle, inputs, &mut diags);
    placement_entry_consumed(bundle, inputs, &mut diags);
    cross_pool_spawn_used_as_a_value(inputs, &mut diags);
    self_containing_locus(bundle, &mut diags);
    diags
}

/// A cross-pool spawn is fire-and-forget (spec/semantics.md, "accept
/// bubbling"): a locus literal `I { }` written in a body of `B`, where
/// `B` does not accept `I` and the nearest acceptor `A` is a singleton
/// on another thread, is born on `A`'s thread through an async handoff,
/// so it may only be a bare statement; used as a value (let-bound, an
/// argument, a field, a sub-expression) it is refused.
///
/// Read off the ownership graph's cross-pool bubble plan, keyed
/// (enclosing locus, child locus) as lowering keys it while it lowers a
/// literal in that locus's own member bodies. A literal in a params
/// default is not judged here: lowering expands a default in the scope
/// that instantiates the locus, under that scope's locus, so which plan
/// entry it meets depends on the instantiation, a relation no row holds
/// yet, and lowering keeps its own refusal for that shape.
fn cross_pool_spawn_used_as_a_value(inputs: &LoweringLawInputs<'_>, diags: &mut Vec<Diag>) {
    // A cross-pool edge needs a locus placed off the main thread; a
    // table whose one domain is main has none, and the graph is not
    // built.
    if inputs.placement.domains.len() <= 1 {
        return;
    }
    let Some(ownership) = (inputs.ownership)() else { return };
    let crosspool = ownership.bubble_plans().crosspool;
    if crosspool.is_empty() {
        return;
    }
    // The ownership walk owns both the resolved child and its use
    // context. Joining by a written leaf or reconstructing the context
    // here would miss aliases/imports or misclassify nested literals.
    for site in &ownership.sites {
        if site.params_default || site.bare_statement {
            continue;
        }
        let child = &site.child_ty;
        let Some(owner) = crosspool.get(&(site.enclosing_locus.clone(), child.clone())) else { continue };
        diags.push(Diag::ty(
            site.span,
            format!(
                "cross-pool spawn `{child}{{ }}` is fire-and-forget: the instance is created on \
                 `{owner}`'s thread and cannot be used here. Write it as a bare statement \
                 (`{child} {{ ... }};`), not as a value (let-binding, sub-expression, or field)."
            ),
        ));
    }
}

/// Rule 18 (GH #890): every `placement { }` entry is consumed by the
/// locus literal lowered for its field.
///
/// A placement entry is carried to lowering as an override on the locus
/// LITERAL lowered for its field, and nothing else takes it. So a field
/// whose value arrives any other way — a factory call, a fallible call,
/// a conditional, a reference to an instance somebody else built —
/// leaves the entry untaken: no thread is spawned, no pool is joined,
/// and the entry the author wrote is dropped. Applying it afterwards is
/// not available (the pinned path spawns a thread that runs the whole
/// lifecycle, and a factory's literal has already run birth and `run()`
/// by the time the value returns), so the entry is refused at its source,
/// pointing at the literal form that carries it.
///
/// The value an entry places is the init a root literal supplies for the
/// field, or the params default when a literal leaves it (or when no
/// literal builds the root, and the entry's template takes its defaults);
/// both spellings are judged, and a default every literal overrides is
/// dead text, not a dropped placement. Read off the placement table: the
/// root is the lowering root, and its literals are every literal of the
/// root declaration as resolved (an imported seed's `main` is not the
/// root, and a literal of another locus that shares its name is not one
/// of them): the table's constructions, written in a scope's bodies, and
/// the literals it records where no body reaches, which lowering expands
/// wherever their holder is built (`locus Holder { params { app: App =
/// App { w: make_worker() }; } }` places nothing, as a construction
/// would not). The default is live exactly when some of those literals
/// leaves the field to it, or when no literal lowering emits builds the
/// root ([`crate::placement::RootRow::built_by_a_literal`]): an expanded
/// literal some lowered literal expands builds it as a construction
/// does, so `App { w: Worker { } }` in `Holder`'s default, with `Holder {
/// }` built, leaves `App`'s own default dead.
fn placement_entry_consumed(bundle: &Bundle<'_>, inputs: &LoweringLawInputs<'_>, diags: &mut Vec<Diag>) {
    let Some(root) = &inputs.placement.root else { return };
    let decls = declarations(bundle);
    let Some(main) = decls.get(&root.realizes.site).copied() else { return };
    let Some(pb) = main.members.iter().find_map(|m| match m {
        LocusMember::Placement(pb) => Some(pb),
        _ => None,
    }) else {
        return;
    };
    let Some(params) = main.members.iter().find_map(|m| match m {
        LocusMember::Params(p) => Some(p),
        _ => None,
    }) else {
        return;
    };
    // Each literal's field inits.
    let wanted: BTreeSet<SiteRef> =
        root.constructions.iter().map(|c| c.literal).chain(root.expanded.iter().map(|e| e.literal)).collect();
    let mut sites: Vec<&[StructInit]> = Vec::new();
    {
        let mut found = literals(|e, _bare| {
            if let Expr::Struct { inits, id, .. } = e {
                if bundle.snapshot.site_id(*id).is_some_and(|s| wanted.contains(&SiteRef::user(s))) {
                    sites.push(inits.as_slice());
                }
            }
        });
        for program in bundle.programs.values() {
            found.items(&program.items);
        }
    }

    for entry in &pb.entries {
        let field = entry.field.name.as_str();
        // An unknown or non-locus field is `check_placement_block`'s to
        // report; saying it twice helps nobody.
        let Some(param) = params.params.iter().find(|p| p.name.name == field) else {
            continue;
        };
        // The literal form to suggest: the declared type as written (a
        // stdlib locus is a qualified path, and its literal is spelled
        // the same way), or `T` when the type is inferred from the
        // default and there is nothing to quote.
        let ty_name = match &param.ty {
            Some(TypeExpr::Named { path, .. }) if !path.segments.is_empty() => {
                path.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::")
            }
            _ => "T".to_string(),
        };
        // The site inits for this field, and whether any site leaves the
        // field to its default.
        let mut overrides: Vec<&Expr> = Vec::new();
        let mut any_site_takes_default = false;
        for inits in &sites {
            match inits.iter().find(|i| i.name.name == field) {
                Some(init) => overrides.push(&init.value),
                None => any_site_takes_default = true,
            }
        }
        for init in overrides {
            if !matches!(init, Expr::Struct { .. }) {
                diags.push(placement_unconsumed_diag(field, &ty_name, init, entry.span, true));
            }
        }
        // The default is live when some literal of the root omits the
        // field, and when no literal lowering emits builds the root
        // (`RootRow::built_by_a_literal`). Lowering then builds no root at
        // all, but the table's entry template, or a library seed checked
        // on its own, takes the default, and the law sides with them: the
        // default is the only initialiser there is.
        if !(any_site_takes_default || !root.built_by_a_literal()) {
            continue;
        }
        // No default and no site init: the missing-required-param rule
        // owns that program, not this one.
        let ParamInit::Value(default) = &param.init else { continue };
        if !matches!(default, Expr::Struct { .. }) {
            diags.push(placement_unconsumed_diag(field, &ty_name, default, entry.span, false));
        }
    }
}

/// The rule 18 diagnostic, for an initialiser written at a root literal
/// (`at_site`) or in the params default.
fn placement_unconsumed_diag(field: &str, ty_name: &str, init: &Expr, entry_span: Span, at_site: bool) -> Diag {
    let shape = match init {
        Expr::Call { .. } => "a call",
        Expr::Or { .. } => "a fallible call",
        Expr::If(_) | Expr::Match(_) => "a conditional",
        Expr::Ident(_)
        | Expr::Path(_)
        | Expr::Field { .. }
        | Expr::Path2 { .. }
        | Expr::Index { .. }
        | Expr::KwSelf(_) => "a reference to an instance built elsewhere",
        _ => "an expression that is not a locus literal",
    };
    Diag::ty(
        init.span(),
        format!(
            "placement entry `{}` names a field no locus literal initialises: {} is {}. A placement \
             is carried by the locus LITERAL lowered for the field — a factory's literal is lowered \
             inside the factory, out of this entry's reach — so the entry would be silently dropped \
             and `{}` would run wherever an unplaced field runs. Write the literal {} (`{}`), and \
             move the factory's other work into the locus's own params or `birth()`.",
            field,
            if at_site { format!("the value supplied for `{}` here", field) } else { format!("`{}`'s default", field) },
            shape,
            field,
            if at_site { "at this site" } else { "in the field" },
            if at_site {
                format!("{}: {} {{ }}", field, ty_name)
            } else {
                format!("{}: {} = {} {{ }};", field, ty_name, ty_name)
            },
        ),
    )
    .with_related(entry_span, format!("`{}` is placed here", field))
}

/// Every expression written in a body, handed to `f` once, outermost
/// first, with whether it is a struct literal written as a bare
/// expression statement (`T { … };`, its value discarded; `false` for
/// every other expression). The bodies are where the placement table's
/// scopes find literals: every fn body (its parameters' defaults
/// included) and every locus member body, at any nesting, through every
/// statement and expression form; `items` walks the positions the table
/// records apart from them too (`PlacementTable`'s root `expanded`), and
/// `expr` walks one expression alone.
struct Literals<F> {
    f: F,
}

fn literals<'a, F: FnMut(&'a Expr, bool)>(f: F) -> Literals<F> {
    Literals { f }
}

impl<'a, F: FnMut(&'a Expr, bool)> Literals<F> {
    /// Every expression written in `items`: the bodies, and the positions
    /// no body reaches (a params default of a locus or a perspective, a
    /// const, a type's field default, a closure's assertion, an adapter's
    /// inits, a perspective's members).
    fn items(&mut self, items: &'a [TopDecl]) {
        for item in items {
            match item {
                TopDecl::Fn(fd) => self.fn_decl(fd),
                TopDecl::Locus(l) => {
                    self.locus_bodies(l);
                    for m in &l.members {
                        match m {
                            LocusMember::Params(pb) => self.params(pb),
                            LocusMember::Const(c) => self.expr(&c.value),
                            LocusMember::Type(td) => self.type_defaults(td),
                            LocusMember::Closure(c) => {
                                if let Some(a) = &c.assertion {
                                    self.expr(&a.left);
                                    self.expr(&a.right);
                                    self.expr(&a.tolerance);
                                }
                            }
                            LocusMember::Bindings(bb) => {
                                for e in &bb.entries {
                                    if let TransportSpec::Adapter { inits, .. } = &e.transport {
                                        for i in inits {
                                            self.expr(&i.value);
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                TopDecl::Perspective(p) => {
                    for m in &p.members {
                        match m {
                            PerspectiveMember::Params(pb) => self.params(pb),
                            PerspectiveMember::StableWhen(b) => self.block(b),
                            PerspectiveMember::Fn(fd) => self.fn_decl(fd),
                            PerspectiveMember::SerializeAs(_) | PerspectiveMember::Bus(_) => {}
                        }
                    }
                }
                TopDecl::Const(c) => self.expr(&c.value),
                TopDecl::Type(td) => self.type_defaults(td),
                TopDecl::Module(m) => self.items(&m.items),
                _ => {}
            }
        }
    }

    fn params(&mut self, pb: &'a ParamsBlock) {
        for p in &pb.params {
            if let ParamInit::Value(e) = &p.init {
                self.expr(e);
            }
        }
    }

    fn type_defaults(&mut self, td: &'a TypeDecl) {
        if let TypeDeclBody::Struct(fields) = &td.body {
            for f in fields {
                if let Some(d) = &f.default {
                    self.expr(d);
                }
            }
        }
    }

    /// A locus's member bodies: not its params defaults, which lowering
    /// expands in the instantiating scope, not the locus's own.
    fn locus_bodies(&mut self, l: &'a LocusDecl) {
        for m in &l.members {
            match m {
                LocusMember::Fn(fd) => self.fn_decl(fd),
                LocusMember::Lifecycle(ld) => self.block(&ld.body),
                LocusMember::Mode(md) => self.block(&md.body),
                LocusMember::Failure(fd) => self.block(&fd.body),
                LocusMember::BirthCheck(bc) => {
                    self.expr(&bc.cond);
                    if let Some(p) = &bc.payload {
                        self.expr(p);
                    }
                }
                _ => {}
            }
        }
    }

    fn literal(&mut self, e: &'a Expr, bare: bool) {
        (self.f)(e, bare);
        if let Expr::Struct { inits, .. } = e {
            for i in inits {
                self.expr(&i.value);
            }
        }
    }

    fn fn_decl(&mut self, fd: &'a FnDecl) {
        for p in &fd.params {
            if let Some(d) = &p.default {
                self.expr(d);
            }
        }
        self.block(&fd.body);
    }

    fn block(&mut self, b: &'a Block) {
        for s in &b.stmts {
            self.stmt(s);
        }
        if let Some(t) = &b.tail {
            self.expr(t);
        }
    }

    fn if_chain(&mut self, i: &'a IfStmt) {
        self.expr(&i.cond);
        self.block(&i.then_block);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => self.block(b),
            Some(ElseBranch::ElseIf(n)) => self.if_chain(n),
            None => {}
        }
    }

    fn match_stmt(&mut self, m: &'a MatchStmt) {
        self.expr(&m.scrutinee);
        for arm in &m.arms {
            if let Some(g) = &arm.guard {
                self.expr(g);
            }
            match &arm.body {
                MatchArmBody::Expr(e) => self.expr(e),
                MatchArmBody::Block(b) => self.block(b),
            }
        }
    }

    fn disposition(&mut self, d: &'a OrDisposition) {
        match d {
            OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => self.expr(e),
            OrDisposition::Raise(_) | OrDisposition::Discard(_) | OrDisposition::Wait(_) => {}
        }
    }

    fn stmt(&mut self, s: &'a Stmt) {
        match s {
            Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => self.expr(value),
            Stmt::Assign { target, value, .. } => {
                self.expr(value);
                for seg in &target.tail {
                    if let LValueSeg::Index(ix) = seg {
                        self.expr(ix);
                    }
                }
            }
            Stmt::If(i) => self.if_chain(i),
            Stmt::Match(m) => self.match_stmt(m),
            Stmt::For { iter, body, .. } => {
                self.expr(iter);
                self.block(body);
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond);
                self.block(body);
            }
            Stmt::Return(e, _) => {
                if let Some(e) = e {
                    self.expr(e);
                }
            }
            Stmt::Fail { value, .. } => self.expr(value),
            Stmt::Expr(e @ Expr::Struct { .. }) => self.literal(e, true),
            Stmt::Expr(e) => self.expr(e),
            Stmt::Block(b) => self.block(b),
            Stmt::Recovery { args, modifier, .. } => {
                for a in args {
                    self.expr(a);
                }
                if let Some(RecoveryModifier::For(e) | RecoveryModifier::Until(e)) = modifier {
                    self.expr(e);
                }
            }
            Stmt::Violate { payload, .. } => {
                if let Some(p) = payload {
                    self.expr(p);
                }
            }
            Stmt::Send { subject, value, or_disposition, .. } => {
                self.expr(subject);
                self.expr(value);
                if let Some(d) = or_disposition {
                    self.disposition(d);
                }
            }
            Stmt::ShmWrite { max, body, .. } => {
                self.expr(max);
                self.block(body);
            }
            Stmt::Reperspective { .. }
            | Stmt::Yield(_)
            | Stmt::Break(_)
            | Stmt::Continue(_)
            | Stmt::Terminate(_) => {}
        }
    }

    fn expr(&mut self, e: &'a Expr) {
        if !matches!(e, Expr::Struct { .. }) {
            (self.f)(e, false);
        }
        match e {
            Expr::Struct { .. } => self.literal(e, false),
            Expr::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Unary { operand, .. } => self.expr(operand),
            Expr::Call { callee, args, .. } => {
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
            }
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => self.expr(receiver),
            Expr::Index { receiver, index, .. } => {
                self.expr(receiver);
                self.expr(index);
            }
            Expr::Tuple(parts, _) | Expr::Array(parts, _) => {
                for p in parts {
                    self.expr(p);
                }
            }
            Expr::Block(b) => self.block(b),
            Expr::If(i) => self.if_chain(i),
            Expr::Match(m) => self.match_stmt(m),
            Expr::Sum(inner, _) | Expr::Prod(inner, _) => self.expr(inner),
            Expr::Approx { left, right, tolerance, .. } => {
                self.expr(left);
                self.expr(right);
                self.expr(tolerance);
            }
            Expr::Range { lo, hi, .. } => {
                self.expr(lo);
                self.expr(hi);
            }
            Expr::ArrayRepeat { val, .. } => self.expr(val),
            Expr::Or { inner, disposition, .. } => {
                self.expr(inner);
                self.disposition(disposition);
            }
            Expr::Literal(..) | Expr::Ident(_) | Expr::Path(_) | Expr::KwSelf(_) => {}
        }
    }
}

/// Rule 17 (GH #826): a root literal whose template pins a field is not
/// written inside a loop.
///
/// A `pinned` entry gives its field an OS thread whose join record (the
/// deferred-dissolve slot and the `pthread_t` it joins) is one alloca
/// per instantiation site, so a site reached a second time overwrites
/// the record of the first: only the last instance is joined and
/// arena-destroyed, and every earlier thread is orphaned with its arena
/// live. Placement names static resources, one thread per entry for the
/// program's life, so the shape is refused rather than joined per
/// iteration.
///
/// Read off the placement table: each of the root's constructions
/// (every literal of the root declaration, resolved, a module-qualified
/// one included) whose bound says it is written inside a loop body, and
/// whose template holds a row a `pinned` entry decides. A factory called
/// in a loop is not one (its literal is not in a loop, and each call
/// joins its own thread at the factory's exit), nor is an adapter, which
/// the bindings prelude builds once however often the root is built.
///
/// A root literal written in a params default (the root's `expanded`)
/// is built wherever a literal taking that default is, and inherits its
/// loop: it is judged at each of the outermost literals that build it
/// (its `built_by`) that is built in a loop, as if that literal were the
/// root's construction, and it pins what the declaration's entries pin
/// (lowering pins every literal of the root). A chain that passes
/// through a position lowering emits at every use (a const's value, a
/// type's field default, a closure's assertion: its `per_use`) has no
/// construction the table records, so no bound shows the root built
/// once: it is refused outright at that position's literal, loop or no
/// loop, naming the hoist.
fn pinned_root_in_a_loop(bundle: &Bundle<'_>, inputs: &LoweringLawInputs<'_>, diags: &mut Vec<Diag>) {
    let placement = inputs.placement;
    let Some(root) = &placement.root else { return };
    let span_of = |site: SiteRef| match site.universe {
        SiteUniverse::User => bundle.snapshot.site(site.id).map(|s| s.span),
        SiteUniverse::StdlibAnalysis => None,
    };
    // The first entry in source order that pins a field of the templates
    // `origin` admits.
    let first_pinned = |origin: &dyn Fn(&Origin) -> bool| {
        placement
            .instances
            .iter()
            .filter(|(key, row)| {
                origin(&key.origin) && matches!(placement.domain(row.domain).kind, DomainKind::Pinned { .. })
            })
            .filter_map(|(key, row)| match &row.decided_by {
                Decision::Entry { entry, .. } => Some((span_of(*entry)?, key.path.first()?.field.as_str())),
                _ => None,
            })
            .min_by_key(|(span, _)| span.start.0)
    };
    let mut sites: Vec<(SiteRef, (Span, &str))> = Vec::new();
    for construction in root.constructions.iter().filter(|c| c.bound.built_in_a_loop()) {
        let origin = Origin::Construction(construction.literal);
        if let Some(pinned) = first_pinned(&|o| *o == origin) {
            sites.push((construction.literal, pinned));
        }
    }
    let in_a_loop: BTreeSet<SiteRef> = root
        .expanded
        .iter()
        .flat_map(|e| e.built_by.iter().filter(|c| c.bound.built_in_a_loop()).map(|c| c.literal))
        .collect();
    if !in_a_loop.is_empty() {
        if let Some(pinned) = first_pinned(&|_| true) {
            for literal in in_a_loop {
                if !sites.iter().any(|(s, _)| *s == literal) {
                    sites.push((literal, pinned));
                }
            }
        }
    }
    let locus = root.realizes.lowered.as_str();
    // A chain through a position lowering emits at every use has no
    // bound to judge: refused outright at that position's literal.
    let per_use: BTreeMap<SiteRef, PerUsePosition> =
        root.expanded.iter().flat_map(|e| e.per_use.iter().map(|p| (p.literal, p.position))).collect();
    if let Some((entry_span, field)) = first_pinned(&|_| true).filter(|_| !per_use.is_empty()) {
        for (literal, position) in per_use {
            if sites.iter().any(|(s, _)| *s == literal) {
                continue;
            }
            let Some(span) = span_of(literal) else { continue };
            diags.push(
                Diag::ty(
                    span,
                    format!(
                        "locus `{}` is built by this literal, written in {}, but its `placement {{ }}` \
                         block pins field `{}` to its own OS thread. The position does not let the \
                         compiler show that `{}` is built once, and every build past the first spawns a \
                         fresh pinned thread while only the last one is joined. Build `{}` in a locus's \
                         `params` or a fn body instead, where the compiler sees how often it is built.",
                        locus,
                        position.describe(),
                        field,
                        locus,
                        locus
                    ),
                )
                .with_related(entry_span, format!("field `{}` is placed `pinned` here", field)),
            );
        }
    }
    for (literal, (entry_span, field)) in sites {
        let Some(span) = span_of(literal) else { continue };
        diags.push(
            Diag::ty(
                span,
                format!(
                    "locus `{}` is instantiated inside a loop, but its `placement {{ }}` block pins \
                     field `{}` to its own OS thread. Every iteration spawns a fresh pinned thread \
                     while only the last one is joined, so the earlier threads are orphaned and their \
                     arenas leak. Placement names static resources (a core, a NUMA node, `replicas = \
                     K`) — one thread per entry for the program's life — so instantiate `{}` once, \
                     outside the loop. (A loop that calls a fn holding the literal is fine: each call \
                     joins its own thread.)",
                    locus, field, locus
                ),
            )
            .with_related(entry_span, format!("field `{}` is placed `pinned` here", field)),
        );
    }
}

/// Rule 6 (F.31): an instance that runs on a thread of its own, a
/// `pinned` placement entry's or an adapter inline in `bindings { }`,
/// declares no `accept` and no closure that fires inside the lifecycle
/// cascade (epoch `birth` or `dissolve`, dissolve being the default).
/// Its children's cascade would cross threads, and the owner's thread
/// runs a birth or dissolve closure that it cannot route to the pinned
/// one.
///
/// Read off the placement table's rows, so the declaration judged is
/// the one the instance realizes (an override literal's, a stdlib
/// locus's, a generic's), never the field's written type, and every
/// pinned anchor is judged: an entry's field and each of its replicas,
/// and a binding entry's adapter. The judgment until C7 walked the
/// placement entries by the field's written type and missed three
/// shapes lowering then refused without a span: a field whose type the
/// checker resolves to `Unknown` (a stdlib locus), an `accept()` written
/// with no parameter (the checker read `accept_param`, lowering the
/// member), and the adapter, which no placement entry names.
fn pinned_features(bundle: &Bundle<'_>, inputs: &LoweringLawInputs<'_>, diags: &mut Vec<Diag>) {
    let placement = inputs.placement;
    let decls = declarations(bundle);
    let mut reported: BTreeSet<(SiteRef, &'static str)> = BTreeSet::new();
    for (key, row) in &placement.instances {
        let (entry, binding) = match &row.decided_by {
            Decision::Entry { entry, .. }
                if matches!(placement.domain(row.domain).kind, DomainKind::Pinned { .. }) =>
            {
                (*entry, false)
            }
            Decision::Binding { entry } => (*entry, true),
            _ => continue,
        };
        let Some(realizes) = &row.realizes else { continue };
        let Some(decl) = decls.get(&realizes.site) else { continue };
        let Some(why) = pinned_conflict(decl) else { continue };
        if !reported.insert((entry, why)) {
            continue;
        }
        let Some(mut span) = bundle.snapshot.site(entry.id).map(|s| s.span) else { continue };
        let locus = decl.name.name.as_str();
        let message = if binding {
            let binding_row = inputs.bindings.for_site(entry.id);
            let topic = binding_row.map(|r| r.topic.as_str()).unwrap_or("?");
            if let Some(hale_syntax::ast::BindingEntry {
                transport: hale_syntax::ast::TransportSpec::Adapter { locus, .. }, ..
            }) = binding_row.and_then(|r| r.entry(bundle)) {
                span = locus.span;
            }
            format!(
                "adapter binding for topic `{}`: `{}` runs on its own pinned thread but {}; \
                 drop the feature from the adapter locus (rule 6)",
                topic, locus, why
            )
        } else {
            let field = key.path.last().map(|s| s.field.as_str()).unwrap_or("?");
            format!(
                "placement entry `{}`: `{}` is placed `pinned` but {}; place it `cooperative`, or \
                 drop the feature (rule 6)",
                field, locus, why
            )
        };
        diags.push(Diag::ty(span, message));
    }
}

/// What a declaration does that a pinned instance cannot: lowering's
/// own two conditions, read off the declaration (an `accept` of any
/// arity, and a closure with an assertion whose epoch is `birth` or
/// `dissolve`; an assertion-less closure is inline and fires through
/// `violate`).
fn pinned_conflict(decl: &LocusDecl) -> Option<&'static str> {
    let accepts = decl
        .members
        .iter()
        .any(|m| matches!(m, LocusMember::Lifecycle(lc) if matches!(lc.kind, LifecycleKind::Accept)));
    if accepts {
        return Some(
            "declares `accept()`: a pinned locus owns its own thread and cannot accept children",
        );
    }
    let cascade_closure = decl.members.iter().any(|m| match m {
        LocusMember::Closure(c) => {
            c.assertion.is_some() && matches!(c.epoch(), EpochSpec::Birth | EpochSpec::Dissolve)
        }
        _ => false,
    });
    cascade_closure.then_some(
        "declares a closure whose epoch is `birth` or `dissolve` (dissolve is the default): the \
         lifecycle cascade cannot route it across a pinned locus's thread",
    )
}

/// Every locus declaration of both universes, by the site the placement
/// table names it with.
fn declarations<'a>(bundle: &'a Bundle<'a>) -> BTreeMap<SiteRef, &'a LocusDecl> {
    fn collect<'a>(
        items: &'a [TopDecl],
        ids: &Snapshot,
        site: fn(hale_graph::ids::SiteId) -> SiteRef,
        out: &mut BTreeMap<SiteRef, &'a LocusDecl>,
    ) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    if let Some(id) = ids.site_id(l.id) {
                        out.insert(site(id), l);
                    }
                }
                TopDecl::Module(m) => collect(&m.items, ids, site, out),
                _ => {}
            }
        }
    }
    let mut out = BTreeMap::new();
    for program in bundle.programs.values() {
        collect(&program.items, &bundle.snapshot, SiteRef::user, &mut out);
    }
    let stdlib: Option<(&'static Program, &'static Snapshot)> =
        crate::stdlib_bodies::program().zip(crate::stdlib_bodies::identities());
    if let Some((program, ids)) = stdlib {
        collect(&program.items, ids, SiteRef::stdlib, &mut out);
    }
    out
}

/// One node of the param-default containment graph (GH #813): a
/// locus name plus the field names a literal SUPPLIES. The defaults a
/// literal expands are exactly the ones it does not supply, so the
/// pair — not the locus alone — is what the construction re-enters.
type ContainmentState = (String, Vec<String>);

/// One edge out of a param default (GH #870): the state the default
/// constructs, and the factory fn it went through — `None` when the
/// default spells the literal itself. The graph is the same either
/// way; the name is what the diagnostic shows the author, who is
/// looking at a call, not at a literal.
type ContainmentEdge = (ContainmentState, Option<String>);

/// GH #813: a locus whose construction requires constructing one of
/// its own kind.
///
/// `locus Node { params { next: Node = Node { n: 1 }; } }` is not a
/// linked list — it is a locus that cannot exist. The `Node` the
/// default builds leaves ITS `next` to the same default, which builds
/// another, and the nesting has no floor. It cannot be broken from a
/// call site either: writing `Node { next: … }` needs a `Node` to
/// hand over, and building one asks the same question again. So the
/// declaration is the error, independently of whether anything
/// instantiates it.
///
/// Before this the program passed `hale check` and
/// `lower_locus_instantiation` recursed through the default until the
/// compiler's own stack ran out ("thread 'main' has overflowed its
/// stack"). Lowering then kept a re-entry guard of its own, spanless,
/// because a harness build never ran the checker; since F.40 phase 3,
/// C7 the harness's lowering view demands this law too, and the guard
/// is gone: every literal lowering expands from a default is one this
/// walk visits (below), keyed as the guard keyed it.
///
/// The graph is over BY-VALUE containment: an edge `L → M` where a
/// param default of `L` *constructs* an `M` — an `M { … }` locus
/// literal anywhere in the default's expression, or (GH #870) a call
/// to a fn that freshly constructs one.
///
/// The second half is the residue #813 left behind. `next: Node =
/// make()` with `fn make() -> Node { return Node { }; }` compiled —
/// lowering a call emits a call rather than inlining the callee, so
/// nothing recursed at COMPILE time and there was no crash to
/// prevent — and then overflowed the program's own stack at RUN time,
/// because every `Node` `make` builds leaves ITS `next` to the same
/// default, which calls `make` again. The ring is the same ring; only
/// the spelling of one edge changed.
///
/// Telling that apart from an accessor handing back a `Node` somebody
/// else already owns is a whole-program question, and the ownership
/// family's fresh-factory rows are the answer
/// ([`crate::ownership::fresh_factories`], the one producer the
/// ownership pre-pass reads too, F.40 phase 1.2c): the rule reads each
/// row's `products`, the (locus, supplied fields) a call constructs.
/// The rows are computed over the bundle's files together, with the
/// bundle's import renames, as lowering computes them, and widened by
/// the carrier fold lowering reads too (F.40 phase 3, C5:
/// [`crate::ownership::extended_factory_rows`]), so a fn whose every
/// returned arm is fresh through an `if`, a `match` or a block is a
/// factory here as it is to lowering. The products do not depend on the escape walk: a factory
/// whose returned binding escapes into a call still constructs it, and
/// still takes its edge. A call it cannot see as constructing — an
/// accessor, a method, a `std::` path — takes no edge and stays
/// accepted, exactly as before; a program like that recurses at RUN
/// time only if the callee really does build one, which is what
/// `@no_recursion` is the contract for.
///
/// A node is (locus, supplied field names) rather than the locus
/// alone: `A { n: 1, m: 2 }` written inside `A`'s own default for `m`
/// expands no default and terminates, and keying on the type would
/// report it as a cycle.
///
/// Nothing is reported for a locus this bundle cannot see. A single
/// file of a multi-file seed holds no `TopDecl::Locus` for its
/// sibling's types, so a cycle that crosses files simply has no edge
/// here and stays permissive until the whole seed is checked
/// together — the gating every other cross-file rule uses, arrived at
/// by having nothing to say rather than by a flag; a build checks the
/// whole seed. Keying a locus by its bare name is sound because loci
/// share one namespace across a seed's modules (two of one name are
/// refused as a duplicate top-level name), and a literal through a
/// module or import path names a locus the seed's own check refuses or
/// one whose own seed's check judges it.
fn self_containing_locus(bundle: &Bundle<'_>, diags: &mut Vec<Diag>) {
    fn collect<'a>(
        items: &'a [TopDecl],
        out: &mut BTreeMap<&'a str, &'a LocusDecl>,
    ) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    out.entry(l.name.name.as_str()).or_insert(l);
                }
                TopDecl::Module(m) => collect(&m.items, out),
                _ => {}
            }
        }
    }
    let mut loci: BTreeMap<&str, &LocusDecl> = BTreeMap::new();
    for program in bundle.programs.values() {
        collect(&program.items, &mut loci);
    }
    if loci.is_empty() {
        return;
    }
    // GH #870: which fns hand back a locus they freshly built, and
    // what each call constructs: the fresh-factory rows over the
    // bundle's files together (a factory in one file may hand back
    // what a sibling file's factory built), with the bundle's import
    // renames, for the loci this bundle declares.
    let programs: Vec<&Program> = bundle.programs.values().copied().collect();
    let factories: BTreeMap<String, Vec<ContainmentState>> =
        crate::ownership::extended_factory_rows(&programs, &bundle.snapshot, &bundle.import_renames)
            .into_iter()
            .filter(|(_, row)| loci.contains_key(row.locus.as_str()))
            .map(|(name, row)| (name, row.products))
            .collect();
    // Classic gray/black DFS. `finished` is the black set: every
    // cycle reachable from a state was found while that state was
    // being explored, so re-entering it later has nothing to add —
    // which is also what keeps one cycle from being reported once per
    // locus on it.
    let mut finished: BTreeSet<ContainmentState> = BTreeSet::new();
    let mut reported: BTreeSet<(u32, String)> = BTreeSet::new();
    for name in loci.keys().copied() {
        let mut path: Vec<ContainmentState> = Vec::new();
        walk_param_default_containment(
            name,
            &[],
            &loci,
            &factories,
            &mut path,
            &mut finished,
            &mut reported,
            diags,
        );
    }
}

/// One DFS step of the GH #813 containment walk. `supplied` is the
/// set of field names the literal that got us here wrote out; every
/// OTHER param of `locus` expands its default, and each locus literal
/// inside that default — plus (GH #870) each fresh-factory call — is
/// an edge.
fn walk_param_default_containment(
    locus: &str,
    supplied: &[String],
    loci: &BTreeMap<&str, &LocusDecl>,
    factories: &BTreeMap<String, Vec<ContainmentState>>,
    path: &mut Vec<ContainmentState>,
    finished: &mut BTreeSet<ContainmentState>,
    reported: &mut BTreeSet<(u32, String)>,
    diags: &mut Vec<Diag>,
) {
    let state: ContainmentState = (locus.to_string(), supplied.to_vec());
    if finished.contains(&state) {
        return;
    }
    let Some(decl) = loci.get(locus) else {
        // A sibling file's locus, or a stdlib one reached by a
        // multi-segment path: no body here, no edge, no report.
        return;
    };
    path.push(state.clone());
    for member in &decl.members {
        let LocusMember::Params(pb) = member else {
            continue;
        };
        for pd in &pb.params {
            if supplied.iter().any(|s| s == &pd.name.name) {
                continue;
            }
            let ParamInit::Value(e) = &pd.init else {
                continue;
            };
            let mut built: Vec<ContainmentEdge> = Vec::new();
            collect_constructed_loci(e, loci, factories, &mut built);
            for (child, via) in built {
                let Some(at) = path.iter().position(|s| *s == child) else {
                    walk_param_default_containment(
                        &child.0, &child.1, loci, factories, path,
                        finished, reported, diags,
                    );
                    continue;
                };
                // The cycle, as a ring of type names, rotated so the
                // locus the author is reading about comes first.
                let mut ring: Vec<&str> =
                    path[at..].iter().map(|s| s.0.as_str()).collect();
                if let Some(k) = ring.iter().position(|n| *n == locus) {
                    ring.rotate_left(k);
                }
                let chain = if ring.len() > 1 {
                    format!(" (`{}` → `{}`)", ring.join("` → `"), ring[0])
                } else {
                    String::new()
                };
                // The two spellings of one edge: a literal in the
                // default, or a call to a fn that builds one (GH
                // #870). Same rule, same ring — what differs is what
                // the author is looking at on that line.
                let (how, why) = match &via {
                    None => (
                        format!("defaults to a `{}`", child.0),
                        "every one the default builds needs another, \
                         and no locus literal can end the chain"
                            .to_string(),
                    ),
                    Some(f) => (
                        format!(
                            "defaults to `{}()`, which builds a fresh \
                             `{}`",
                            f, child.0,
                        ),
                        "every one the factory builds asks the same \
                         default again, so the program compiles and \
                         then recurses until its stack overflows"
                            .to_string(),
                    ),
                };
                let message = format!(
                    "param `{}` of `{}` {}; a locus cannot contain \
                     itself by value{} — {}. Drop the default and \
                     take the child from the caller (`{}: {};`), or \
                     hold a value rather than a locus.",
                    pd.name.name,
                    locus,
                    how,
                    chain,
                    why,
                    pd.name.name,
                    child.0,
                );
                if reported.insert((pd.span.start.0, message.clone())) {
                    diags.push(Diag::ty(pd.span, message));
                }
            }
        }
    }
    path.pop();
    finished.insert(state);
}

/// Every locus `M` constructed while `e` is evaluated, as (locus
/// name, the field names it supplies) plus the factory fn the default
/// reached it through, if any.
///
/// Two spellings construct one:
///
///   * a locus literal `M { … }`. Nested literals count too — a
///     literal inside a literal's field is constructed just as surely
///     as the outer one. Only single-segment paths that name a locus
///     in this bundle are edges; a `type` literal, a stdlib path and
///     a sibling file's name are all skipped;
///   * GH #870: a call to a fn the fresh-factory rows classify as
///     freshly building one. The states it contributes are the row's
///     products, the literals that fn hands back, so `fn make() ->
///     Node { return Node { n: 5 }; }` contributes `(Node, [n])` — the
///     same node the literal `Node { n: 5 }` would.
///
/// Every sub-expression of the default is visited, through blocks, `if`
/// and `match` arms and every statement a block holds: lowering lowers
/// each of them, every branch of a conditional included, so a literal
/// anywhere in the default is one lowering expands (F.40 phase 3, C7;
/// the walk used to stop at a block, `if` or `match`).
fn collect_constructed_loci(
    e: &Expr,
    loci: &BTreeMap<&str, &LocusDecl>,
    factories: &BTreeMap<String, Vec<ContainmentState>>,
    out: &mut Vec<ContainmentEdge>,
) {
    let mut walk = literals(|e, _bare| match e {
        Expr::Struct { path, inits, .. } if path.segments.len() == 1 => {
            let name = path.segments[0].name.as_str();
            if loci.contains_key(name) {
                let mut supplied: Vec<String> = inits.iter().map(|i| i.name.name.clone()).collect();
                supplied.sort();
                supplied.dedup();
                out.push(((name.to_string(), supplied), None));
            }
        }
        Expr::Call { callee, .. } => {
            if let Some(f) = plain_callee_name(callee) {
                if let Some(states) = factories.get(f) {
                    for s in states {
                        out.push((s.clone(), Some(f.to_string())));
                    }
                }
            }
        }
        _ => {}
    });
    walk.expr(e);
}

/// The single-segment name a callee spells, or `None` for a method,
/// a path, or anything computed. This containment walk currently follows
/// only bare calls; qualified factory calls remain outside its surface.
fn plain_callee_name(callee: &Expr) -> Option<&str> {
    match callee {
        Expr::Ident(i) => Some(i.name.as_str()),
        Expr::Path(q) if q.segments.len() == 1 => {
            Some(q.segments[0].name.as_str())
        }
        _ => None,
    }
}
