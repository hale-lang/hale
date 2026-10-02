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
    MatchArmBody, MatchStmt, OrDisposition, ParamInit, Program, RecoveryModifier, Stmt, StructInit, TopDecl,
    TypeExpr,
};
use hale_syntax::{Diag, Span};

use crate::binding_rows::BindingRows;
use crate::placement::{Decision, DomainKind, Origin, PlacementTable, SiteRef, SiteUniverse};
use crate::snapshot::Snapshot;
use crate::Bundle;

/// The rows the laws read.
pub struct LoweringLawInputs<'a> {
    /// The placement table: which instance runs pinned, what it
    /// realizes, and what decided it.
    pub placement: &'a PlacementTable,
    /// The binding rows: the topic an adapter's binding entry names.
    pub bindings: &'a BindingRows,
}

/// Every law that replaced a lowering backstop, over `bundle`.
pub fn lowering_laws(bundle: &Bundle<'_>, inputs: &LoweringLawInputs<'_>) -> Vec<Diag> {
    let mut diags = Vec::new();
    pinned_features(bundle, inputs, &mut diags);
    pinned_root_in_a_loop(bundle, inputs, &mut diags);
    placement_entry_consumed(bundle, inputs, &mut diags);
    diags
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
/// literal builds the root, and the entry builds it from its defaults);
/// both spellings are judged, and a default every literal overrides is
/// dead text, not a dropped placement. Read off the placement table: the
/// root is the lowering root, and its literals are the table's
/// constructions of it, every literal of the root declaration as
/// resolved (an imported seed's `main` is not the root, and a literal of
/// another locus that shares its name is not one of them).
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
    // Each construction's field inits.
    let wanted: BTreeSet<SiteRef> = root.constructions.iter().map(|c| c.literal).collect();
    let mut found = RootLiterals { ids: &bundle.snapshot, wanted: &wanted, sites: Vec::new() };
    for program in bundle.programs.values() {
        found.items(&program.items);
    }
    let sites = found.sites;

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
        // The default is live when some site omits the field, and when
        // no literal builds the root at all (the entry's implicit
        // template, or a library seed checked on its own: the default is
        // the only initialiser there is).
        if !(any_site_takes_default || sites.is_empty()) {
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

/// The field inits of the literals the placement table lists as the
/// root's constructions, found where the table's scopes find them: every
/// fn body (its parameters' defaults included) and every locus member
/// body, at any nesting.
struct RootLiterals<'a, 'b> {
    ids: &'b Snapshot,
    wanted: &'b BTreeSet<SiteRef>,
    sites: Vec<&'a [StructInit]>,
}

impl<'a> RootLiterals<'a, '_> {
    fn items(&mut self, items: &'a [TopDecl]) {
        for item in items {
            match item {
                TopDecl::Fn(fd) => self.fn_decl(fd),
                TopDecl::Locus(l) => {
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
                TopDecl::Module(m) => self.items(&m.items),
                _ => {}
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
        match e {
            Expr::Struct { inits, id, .. } => {
                if self.ids.site_id(*id).is_some_and(|s| self.wanted.contains(&SiteRef::user(s))) {
                    self.sites.push(inits.as_slice());
                }
                for i in inits {
                    self.expr(&i.value);
                }
            }
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
fn pinned_root_in_a_loop(bundle: &Bundle<'_>, inputs: &LoweringLawInputs<'_>, diags: &mut Vec<Diag>) {
    let placement = inputs.placement;
    let Some(root) = &placement.root else { return };
    let span_of = |site: SiteRef| match site.universe {
        SiteUniverse::User => bundle.snapshot.site(site.id).map(|s| s.span),
        SiteUniverse::StdlibAnalysis => None,
    };
    for construction in root.constructions.iter().filter(|c| c.bound.in_a_loop()) {
        let origin = Origin::Construction(construction.literal);
        // The first entry in source order that pins a field of this
        // template.
        let pinned = placement
            .instances
            .iter()
            .filter(|(key, row)| {
                key.origin == origin && matches!(placement.domain(row.domain).kind, DomainKind::Pinned { .. })
            })
            .filter_map(|(key, row)| match &row.decided_by {
                Decision::Entry { entry, .. } => Some((span_of(*entry)?, key.path.first()?.field.as_str())),
                _ => None,
            })
            .min_by_key(|(span, _)| span.start.0);
        let Some((entry_span, field)) = pinned else { continue };
        let Some(span) = span_of(construction.literal) else { continue };
        let locus = root.realizes.lowered.as_str();
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
        let Some(span) = bundle.snapshot.site(entry.id).map(|s| s.span) else { continue };
        let locus = decl.name.name.as_str();
        let message = if binding {
            let topic = inputs
                .bindings
                .for_site(entry.id)
                .map(|r| r.topic.as_str())
                .unwrap_or("?");
            format!(
                "binding entry `{}`: adapter `{}` runs pinned (an adapter inline in `bindings {{ }}` \
                 has a thread of its own) but {}; drop the feature, or bind the topic another way \
                 (rule 6)",
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
