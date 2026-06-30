//! Authoritative bus-dispatch graph + static-eligibility gate
//! (static-bus-dispatch devirtualization, build #1a).
//!
//! Hale's bus topology is fully *static*: every subscription is a
//! `LocusMember::Bus(BusBlock)` → `BusMember::Subscribe { subject,
//! handler, .. }` declared in source — there is no runtime
//! `subscribe()` construct (grep the tree: the only `subscribe`
//! token is the declarative bus-block keyword). So the set of
//! subscribers on every subject is statically enumerable, which is
//! the premise that makes devirtualization sound.
//!
//! This module reifies that graph as a [`BusGraph`] and classifies
//! each subject with a soundness-critical *eligibility gate*: a
//! subject is `eligible` only when its dispatch can be lowered to a
//! direct, statically-resolved call with no loss of meaning. The
//! gate **defaults to ineligible** — any subject shape, placement,
//! or condition this pass does not explicitly understand is marked
//! ineligible with a reason. A false `eligible` is a future
//! codegen-correctness bug; a false-ineligible only misses the
//! optimization.
//!
//! Build #1a is pure analysis: nothing here changes codegen. The
//! diagnostics pass (`check::check_bus_graph`) and `build_bus_graph`
//! SHARE the publishers/subscribers/bound/cross-seed/wildcard walk
//! via [`collect_bus_walk`] — there is one walk, two consumers.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::*;
use hale_syntax::Span;

use crate::resolve::TopScope;
use crate::symbol::{Bundle, TopSymbol};

// === Shared walk (consumed by both check_bus_graph + build) =======

/// One end of the bus graph: subject-key → first declaration span,
/// plus the wildcard patterns seen on that end (matched separately).
#[derive(Default)]
pub(crate) struct BusEnd {
    pub(crate) concrete: BTreeMap<String, Span>,
    pub(crate) wildcards: Vec<String>,
}

impl BusEnd {
    fn record(&mut self, key: String, span: Span) {
        if key.contains("**") {
            self.wildcards.push(key);
        } else {
            self.concrete.entry(key).or_insert(span);
        }
    }
    /// Does this end carry `subject` — exactly, or via a wildcard?
    pub(crate) fn covers(&self, subject: &str) -> bool {
        self.concrete.contains_key(subject)
            || self.wildcards.iter().any(|p| crate::wildcard_match(p, subject))
    }
}

/// A single publish site, captured during the walk with enough
/// context (owning locus type, payload-resolution key) to build a
/// `PublisherSite` later.
pub(crate) struct RawPub {
    pub(crate) locus: String,
    pub(crate) key: String,
    pub(crate) span: Span,
}

/// A single subscribe site. `qualified` flags the cross-seed
/// `BusSubject::QualifiedTopic` shape; `keyed` flags a Phase-3
/// `where key == …` routing filter. Both are statically
/// unresolvable-to-a-single-call here, so they force ineligibility.
pub(crate) struct RawSub {
    pub(crate) locus: String,
    pub(crate) handler: String,
    pub(crate) key: String,
    pub(crate) span: Span,
    pub(crate) qualified: bool,
    pub(crate) keyed: bool,
}

/// The product of one walk over the bundle's bus topology — every
/// input the orphan diagnostics OR the eligibility gate needs.
pub(crate) struct BusWalk {
    pub(crate) publishers: BusEnd,
    pub(crate) subscribers: BusEnd,
    pub(crate) bound: BTreeSet<String>,
    pub(crate) cross_seed: BTreeSet<String>,
    pub(crate) pub_sites: Vec<RawPub>,
    pub(crate) sub_sites: Vec<RawSub>,
}

/// Walk every locus's `bus { }` + `bindings { }` blocks once,
/// collecting the publisher/subscriber ends (for orphan
/// diagnostics) AND the per-site detail (for the graph). This is
/// the single source of truth `check_bus_graph` and
/// `build_bus_graph` both consume — do not duplicate the walk.
pub(crate) fn collect_bus_walk(bundle: &Bundle<'_>) -> BusWalk {
    let mut w = BusWalk {
        publishers: BusEnd::default(),
        subscribers: BusEnd::default(),
        bound: BTreeSet::new(),
        cross_seed: BTreeSet::new(),
        pub_sites: Vec::new(),
        sub_sites: Vec::new(),
    };

    fn walk(items: &[TopDecl], w: &mut BusWalk) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    let locus = l.name.name.clone();
                    for m in &l.members {
                        match m {
                            LocusMember::Bus(bb) => {
                                for bm in &bb.members {
                                    match bm {
                                        BusMember::Publish { subject, span, .. } => {
                                            let key = subject.canonical().to_string();
                                            if matches!(subject, BusSubject::QualifiedTopic(_)) {
                                                w.cross_seed.insert(key.clone());
                                            }
                                            w.publishers.record(key.clone(), *span);
                                            w.pub_sites.push(RawPub {
                                                locus: locus.clone(),
                                                key,
                                                span: *span,
                                            });
                                        }
                                        BusMember::Subscribe {
                                            subject,
                                            handler,
                                            key_filter,
                                            span,
                                            ..
                                        } => {
                                            let key = subject.canonical().to_string();
                                            let qualified = matches!(
                                                subject,
                                                BusSubject::QualifiedTopic(_)
                                            );
                                            if qualified {
                                                w.cross_seed.insert(key.clone());
                                            }
                                            w.subscribers.record(key.clone(), *span);
                                            w.sub_sites.push(RawSub {
                                                locus: locus.clone(),
                                                handler: handler.name.clone(),
                                                key,
                                                span: *span,
                                                qualified,
                                                keyed: key_filter.is_some(),
                                            });
                                        }
                                    }
                                }
                            }
                            LocusMember::Bindings(bbk) => {
                                for entry in &bbk.entries {
                                    w.bound.insert(entry.topic.name.clone());
                                }
                            }
                            _ => {}
                        }
                    }
                }
                TopDecl::Module(md) => walk(&md.items, w),
                _ => {}
            }
        }
    }
    for program in bundle.programs.values() {
        walk(&program.items, &mut w);
    }
    w
}

// === Public graph =================================================

/// Where a subscriber's handler runs relative to the publisher's
/// thread — mirrors the placement classification computed in
/// `hale-syntax::desugar` (`collect_off_owner_thread_fields`).
///
/// `CrossPool`/`Pinned` mean the handler runs on a *different* OS
/// thread, so any later devirtualization must still route through
/// the mailbox/queue rather than a same-thread direct call;
/// `SameThread` is the placement where an intra-thread direct call
/// is the lowering #1b would pick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// Cooperative on the owner's (main) thread — `cooperative`,
    /// `cooperative(pool = main)`, or no placement entry at all.
    SameThread,
    /// A named cooperative pool other than `main` — its own OS
    /// thread shared with co-placed loci.
    CrossPool(String),
    /// `pinned` — the locus owns a dedicated OS thread.
    Pinned,
}

/// A resolved publish site on a subject.
#[derive(Debug, Clone)]
pub struct PublisherSite {
    pub locus: String,
    /// Payload type name (`Ty::display()`), or `"?"` if it could
    /// not be resolved from the topic decl / locus symbol.
    pub payload: String,
    pub span: Span,
}

/// A resolved subscribe site on a subject.
#[derive(Debug, Clone)]
pub struct SubscriberSite {
    pub locus: String,
    pub handler: String,
    pub placement: Placement,
    pub payload: String,
    pub span: Span,
}

/// Why a subject is NOT statically devirtualizable. Ordered by the
/// gate's check order; `Unanalyzable` is the catch-all that keeps
/// the gate sound for shapes this pass does not explicitly model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaticIneligible {
    /// No `main` locus in the bundle — the other end of a channel
    /// may live in a downstream consumer (open world).
    OpenWorld,
    /// The subject is bound to a transport adapter (`bindings { }`)
    /// — an external peer is (or may be) the real counterparty.
    TransportBound,
    /// The subject is a `**` wildcard, or is covered by a wildcard
    /// pattern on either end — the subscriber set is not a fixed
    /// concrete list.
    Wildcard,
    /// A cross-seed (`alias::Foo`) reference — the other half is
    /// owned by another seed/bundle.
    CrossSeed,
    /// Any other shape this pass cannot statically resolve to a
    /// concrete local handler set (qualified subject, Phase-3
    /// routing key, unknown `BusSubject` variant, …). Carries a
    /// human-readable reason. Default-to-ineligible lives here.
    Unanalyzable(String),
}

impl StaticIneligible {
    /// A short tag for classification summaries / test assertions.
    pub fn tag(&self) -> &'static str {
        match self {
            StaticIneligible::OpenWorld => "OpenWorld",
            StaticIneligible::TransportBound => "TransportBound",
            StaticIneligible::Wildcard => "Wildcard",
            StaticIneligible::CrossSeed => "CrossSeed",
            StaticIneligible::Unanalyzable(_) => "Unanalyzable",
        }
    }
}

/// Per-subject view of the bus graph.
#[derive(Debug, Clone)]
pub struct SubjectInfo {
    pub publishers: Vec<PublisherSite>,
    pub subscribers: Vec<SubscriberSite>,
    /// `true` iff every soundness condition holds (see
    /// `build_bus_graph`). Defaults to `false` for anything the
    /// gate does not positively clear.
    pub eligible: bool,
    /// `Some(reason)` exactly when `!eligible`.
    pub ineligible_reason: Option<StaticIneligible>,
    /// Direct-call devirtualization (build #1b slice-2). `true` iff
    /// the publish on this subject may be lowered to a *synchronous
    /// direct call* to each subscriber handler — collapsing the
    /// cooperative-queue enqueue + deferred drain entirely — with no
    /// loss of observable meaning. Strictly STRONGER than `eligible`:
    /// it additionally requires that
    ///   (a) the subject has ≥1 subscriber, every one of which is
    ///       `Placement::SameThread` (a CrossPool / Pinned subscriber
    ///       runs on another OS thread and CANNOT be direct-called —
    ///       it must enqueue), and
    ///   (b) every subscriber handler is provably **QUIET** by the
    ///       syntactic effect-walk in [`handler_is_quiet`] — it
    ///       mutates ONLY its own `self` fields with pure expressions
    ///       and has no other effect.
    /// Defaults to `false` (default-bail). The FLAT-payload condition
    /// is the third leg of the gate but is checked at the codegen
    /// publish site (it needs the lowered payload type), so this flag
    /// is ANDed with `bus_payload_is_flat` there. A false positive is
    /// an observable-ordering bug, so this stays conservative.
    pub direct_call_eligible: bool,
}

/// The whole-bundle bus graph, keyed by `BusSubject::canonical()`.
#[derive(Debug, Clone, Default)]
pub struct BusGraph {
    pub subjects: BTreeMap<String, SubjectInfo>,
}

impl BusGraph {
    /// Count of subjects cleared as statically devirtualizable.
    pub fn eligible_count(&self) -> usize {
        self.subjects.values().filter(|s| s.eligible).count()
    }
    /// Histogram of ineligible subjects by reason tag.
    pub fn ineligible_by_reason(&self) -> BTreeMap<&'static str, usize> {
        let mut h: BTreeMap<&'static str, usize> = BTreeMap::new();
        for s in self.subjects.values() {
            if let Some(r) = &s.ineligible_reason {
                *h.entry(r.tag()).or_insert(0) += 1;
            }
        }
        h
    }
}

/// Build the authoritative [`BusGraph`] for a bundle. Run this
/// AFTER typecheck so `top` carries resolved payload types.
///
/// Shares the publishers/subscribers/bound/cross-seed/wildcard
/// computation with `check::check_bus_graph` via
/// [`collect_bus_walk`]; the only graph-specific work here is
/// joining per-site detail (locus, handler, payload, placement)
/// and applying the eligibility gate.
pub fn build_bus_graph(bundle: &Bundle<'_>, top: &TopScope) -> BusGraph {
    let walk = collect_bus_walk(bundle);

    // Closed-world gate input (DEVIRT-ONLY notion): a complete,
    // closed-world program is one with an ENTRY POINT — a bare
    // top-level `fn main` free function OR a `main locus`. Either
    // produces an executable whose every subscriber is statically
    // declared in-bundle (an executable cannot gain subscribers at
    // runtime — there is no dynamic `subscribe`), so the bus graph
    // is complete.
    //
    // This is deliberately BROADER than `check::check_bus_graph`'s
    // diagnostics gate, which stays `main locus`-only to keep its
    // orphan/dead-receiver warnings over-fire-conscious. The two
    // notions are separate by design — do not unify them. The
    // canonical `fn main` entry shape mirrors codegen's
    // `TopDecl::Fn(f) if f.name.name == "main"` lookup.
    let has_entry_point = bundle.programs.values().any(|p| {
        p.items.iter().any(|i| {
            matches!(i, TopDecl::Locus(l) if l.is_main)
                || matches!(i, TopDecl::Fn(f) if f.name.name == "main")
        })
    });

    let placements = collect_subscriber_placements(bundle);

    // Gather every subject that appears on either end.
    let mut keys: BTreeSet<String> = BTreeSet::new();
    for p in &walk.pub_sites {
        keys.insert(p.key.clone());
    }
    for s in &walk.sub_sites {
        keys.insert(s.key.clone());
    }

    let mut subjects: BTreeMap<String, SubjectInfo> = BTreeMap::new();
    for key in keys {
        let publishers: Vec<PublisherSite> = walk
            .pub_sites
            .iter()
            .filter(|p| p.key == key)
            .map(|p| PublisherSite {
                locus: p.locus.clone(),
                payload: resolve_payload(top, &p.locus, &key),
                span: p.span,
            })
            .collect();
        let subscribers: Vec<SubscriberSite> = walk
            .sub_sites
            .iter()
            .filter(|s| s.key == key)
            .map(|s| SubscriberSite {
                locus: s.locus.clone(),
                handler: s.handler.clone(),
                placement: placements
                    .get(&s.locus)
                    .cloned()
                    .unwrap_or(Placement::SameThread),
                payload: resolve_payload(top, &s.locus, &key),
                span: s.span,
            })
            .collect();

        let reason = classify(&key, &walk, has_entry_point);
        let eligible = reason.is_none();
        // Direct-call gate (slice-2). STRONGER than `eligible`:
        // additionally every subscriber must be same-thread AND its
        // handler provably quiet (the flat-payload leg is ANDed in at
        // the codegen publish site). Default-bail: a missing handler
        // body or any unmodeled placement/effect ⟹ not direct.
        let direct_call_eligible = eligible
            && !subscribers.is_empty()
            && subscribers.iter().all(|s| {
                s.placement == Placement::SameThread
                    && find_handler_fn(bundle, &s.locus, &s.handler)
                        .map(handler_is_quiet)
                        .unwrap_or(false)
            });
        subjects.insert(
            key,
            SubjectInfo {
                publishers,
                subscribers,
                eligible,
                ineligible_reason: reason,
                direct_call_eligible,
            },
        );
    }

    BusGraph { subjects }
}

/// The soundness-critical gate. Returns `None` when the subject is
/// statically devirtualizable, else the first failing reason in the
/// canonical check order. DEFAULTS TO INELIGIBLE: every condition
/// must be positively cleared.
fn classify(
    key: &str,
    walk: &BusWalk,
    has_entry_point: bool,
) -> Option<StaticIneligible> {
    // 1) Closed-world: the bundle has an entry point (`fn main` or
    //    a `main locus`), so its bus graph is complete.
    if !has_entry_point {
        return Some(StaticIneligible::OpenWorld);
    }
    // 2) No transport adapter binding.
    if walk.bound.contains(key) {
        return Some(StaticIneligible::TransportBound);
    }
    // 3) No wildcard — neither the subject itself nor any pattern
    //    covering it on either end.
    if key.contains("**")
        || walk.publishers.wildcards.iter().any(|p| crate::wildcard_match(p, key))
        || walk.subscribers.wildcards.iter().any(|p| crate::wildcard_match(p, key))
    {
        return Some(StaticIneligible::Wildcard);
    }
    // 4) Not referenced cross-seed.
    if walk.cross_seed.contains(key) {
        return Some(StaticIneligible::CrossSeed);
    }
    // 5) Every subscriber resolves to a concrete local handler:
    //    a plain `Topic`/literal subject with no routing key. A
    //    qualified subject or a Phase-3 `where key` filter is not
    //    a single-call dispatch — ineligible.
    for s in walk.sub_sites.iter().filter(|s| s.key == key) {
        if s.qualified {
            return Some(StaticIneligible::Unanalyzable(format!(
                "subscriber `{}` on `{}` uses a cross-seed qualified subject",
                s.handler, key
            )));
        }
        if s.keyed {
            return Some(StaticIneligible::Unanalyzable(format!(
                "subscriber `{}` on `{}` carries a Phase-3 routing-key filter",
                s.handler, key
            )));
        }
    }
    None
}

/// Resolve a site's payload type name. Tries the declared-topic
/// route first (subject name / wire subject → `TopicInfo.payload`),
/// then the owning locus's resolved bus entries (literal `of type
/// T` sites). `"?"` when neither resolves.
fn resolve_payload(top: &TopScope, locus: &str, key: &str) -> String {
    // Declared topic addressed by name or wire subject.
    for sym in top.symbols.values() {
        if let TopSymbol::Topic(t) = sym {
            if t.name == key || t.wire_subject == key {
                return t.payload.display();
            }
        }
    }
    // Literal subject: read the resolved payload off the locus.
    if let Some(TopSymbol::Locus(l)) = top.lookup(locus) {
        for p in &l.bus_publishes {
            if p.subject == key {
                return p.payload.display();
            }
        }
        for s in &l.bus_subscribes {
            if s.subject == key {
                return s.payload.display();
            }
        }
    }
    "?".to_string()
}

/// Map each locus *type* name to the [`Placement`] it receives
/// where instantiated as a placed field. Mirrors
/// `desugar::collect_off_owner_thread_fields`: a `placement { }`
/// entry keys on the owner's `params` field name, and that field's
/// declared type names the placed child locus.
///
/// First-placement-wins when a type is placed in multiple fields
/// (the multi-instance case); placement is informational for the
/// gate, so a conservative single label suffices.
fn collect_subscriber_placements(bundle: &Bundle<'_>) -> BTreeMap<String, Placement> {
    let mut out: BTreeMap<String, Placement> = BTreeMap::new();

    fn walk(items: &[TopDecl], out: &mut BTreeMap<String, Placement>) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    // field name → declared locus-type name.
                    let mut field_ty: BTreeMap<String, String> = BTreeMap::new();
                    for member in &l.members {
                        if let LocusMember::Params(pb) = member {
                            for p in &pb.params {
                                if let Some(ty) = &p.ty {
                                    if let Some(name) = single_named_type(ty) {
                                        field_ty.insert(p.name.name.clone(), name);
                                    }
                                }
                            }
                        }
                    }
                    for member in &l.members {
                        if let LocusMember::Placement(pb) = member {
                            for e in &pb.entries {
                                let Some(child_ty) = field_ty.get(&e.field.name) else {
                                    continue;
                                };
                                let placement = match &e.spec {
                                    PlacementSpec::Cooperative { pool } => match pool {
                                        Some(p) if p.name != "main" => {
                                            Placement::CrossPool(p.name.clone())
                                        }
                                        _ => Placement::SameThread,
                                    },
                                    PlacementSpec::Pinned { .. } => Placement::Pinned,
                                };
                                out.entry(child_ty.clone()).or_insert(placement);
                            }
                        }
                    }
                }
                TopDecl::Module(m) => walk(&m.items, out),
                _ => {}
            }
        }
    }
    for program in bundle.programs.values() {
        walk(&program.items, &mut out);
    }
    out
}

/// Whole-program off-thread-placement probe for static-devirt #3
/// (static-pinned). Returns `true` iff ANY `placement { }` entry in
/// the bundle places a locus off its owner's thread — a `pinned`
/// spec, or a `cooperative(pool = X)` with X != `main`. These are
/// exactly the placements that make the runtime flip
/// `g_bus_has_pinned` (via `lotus_bus_mark_pinned` on a pinned spawn,
/// or `lotus_coop_pool_start_all` when a non-main pool has workers).
///
/// This is ONE term of codegen's "does a thread cross the bus
/// boundary" predicate (`program_has_offthread`): codegen ORs it with
/// an inline adapter `bindings { }` recv-loop check (a transport
/// thread is pinned-equivalent but carries no placement entry, so it
/// is invisible to this probe). Only when the FULL predicate is false
/// may codegen emit the no-acquire-load static enqueue
/// (`lotus_bus_queue_enqueue_st`) — this probe alone is NOT
/// sufficient. Unlike the per-subject placement labels on
/// `SubscriberSite` (subscribers only, first-placement-wins), this
/// scans every placement entry so a pinned *publisher* (or any
/// non-subscriber off-thread locus) is still caught.
pub fn has_offthread_placement(bundle: &Bundle<'_>) -> bool {
    fn walk(items: &[TopDecl]) -> bool {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    for member in &l.members {
                        if let LocusMember::Placement(pb) = member {
                            for e in &pb.entries {
                                match &e.spec {
                                    PlacementSpec::Cooperative { pool } => {
                                        if let Some(p) = pool {
                                            if p.name != "main" {
                                                return true;
                                            }
                                        }
                                    }
                                    PlacementSpec::Pinned { .. } => return true,
                                }
                            }
                        }
                    }
                }
                TopDecl::Module(m) => {
                    if walk(&m.items) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }
    bundle.programs.values().any(|p| walk(&p.items))
}

/// The single named type a `TypeExpr` denotes (a bare `Named`
/// path), else `None`. Non-locus field types never appear in a
/// `placement { }` block (typecheck enforces), so we don't confirm
/// locus-ness here.
fn single_named_type(ty: &TypeExpr) -> Option<String> {
    match ty {
        TypeExpr::Named { path, .. } => path.segments.last().map(|s| s.name.clone()),
        _ => None,
    }
}

// === Quiet-handler classifier (direct-call devirt slice-2) =========
//
// The soundness theorem: loci are ISOLATED — a publisher cannot read a
// subscriber's `self` state. So if a same-thread subscriber's handler
// is QUIET (mutates ONLY its own `self` fields, with no other effect),
// running it SYNCHRONOUSLY at the publish point is indistinguishable
// from running it at the next deferred drain: its only effect is
// isolated state nobody can read until the subscriber's own code runs
// at a later cooperative point (by which time the handler has completed
// in either mode), and multiple quiet handlers touch only their own
// state so dispatch order is irrelevant. ⟹ identical observable
// behavior. Any deviation from "same-thread + quiet" keeps the deferred
// enqueue.
//
// `handler_is_quiet` is a CONSERVATIVE syntactic effect-walk that
// DEFAULTS TO NOT-QUIET. The handler body may contain ONLY:
//   * `self.<field…> = <pure-expr>` assignments (incl. compound `+=`),
//     where the LValue is `self`-headed and every segment is a field
//     (NO index — an array-index store can trap and the array field is
//     a heap pointer);
//   * `let <local> = <pure-expr>` bindings (the local joins the pure
//     scope);
//   * `if` / `while` / `block` control flow whose conditions are pure
//     and whose bodies are themselves quiet;
//   * bare `return [pure-expr]`, `break`, `continue`.
// EVERYTHING ELSE bails: any function/method call, any `<-` send, any
// I/O, any `accept`/`terminate`/`release`/lifecycle op, any
// `yield`/`fail`/`violate`/recovery, `match`, `for`, tuple-`let`,
// `Stmt::Expr` (a bare expression statement — the usual carrier of a
// call like `println(...)`), and any AST node not explicitly modeled.
//
// `<pure-expr>` = arithmetic (EXCLUDING `/` and `%`, which lower to
// `sdiv`/`srem` and can TRAP on divide-by-zero — a trap reorders a
// program abort relative to publisher I/O, an observable difference) /
// comparison / logical / bitwise over: literals, `self.<field…>`
// reads, a handler PARAMETER's FIELD read (`param.field` — a flat
// payload's fields are all by-value scalars, so this is a value read
// with no lifetime hazard), and LOCAL reads. A BARE parameter ident is
// NOT pure: a flat payload is passed by pointer, and capturing that
// pointer into `self` (`self.last = p`) would alias the publisher's
// live (reused) storage under the direct call while the deferred path
// delivers a copy — so we forbid the bare-param read entirely. That
// closes the only channel by which publisher/payload memory could
// enter `self`; every permitted `self` mutation is therefore either a
// scalar value or a `self`-internal pointer copy, both byte- and
// lifetime-identical between the direct and deferred lowerings.

/// Locate locus `locus`'s method `handler` FnDecl anywhere in the
/// bundle (walking nested modules). `None` ⟹ not found ⟹ caller bails.
fn find_handler_fn<'a>(
    bundle: &Bundle<'a>,
    locus: &str,
    handler: &str,
) -> Option<&'a FnDecl> {
    fn search<'a>(
        items: &'a [TopDecl],
        locus: &str,
        handler: &str,
    ) -> Option<&'a FnDecl> {
        for item in items {
            match item {
                TopDecl::Locus(l) if l.name.name == locus => {
                    for m in &l.members {
                        if let LocusMember::Fn(f) = m {
                            if f.name.name == handler {
                                return Some(f);
                            }
                        }
                    }
                }
                TopDecl::Module(md) => {
                    if let Some(f) = search(&md.items, locus, handler) {
                        return Some(f);
                    }
                }
                _ => {}
            }
        }
        None
    }
    for program in bundle.programs.values() {
        if let Some(f) = search(&program.items, locus, handler) {
            return Some(f);
        }
    }
    None
}

/// Is the handler QUIET? See the module-level note above for the
/// soundness argument and the exact allow/bail list.
fn handler_is_quiet(f: &FnDecl) -> bool {
    // A handler annotated `@ffi` has no analyzable body; bail.
    if f.ffi.is_some() {
        return false;
    }
    let params: BTreeSet<String> =
        f.params.iter().map(|p| p.name.name.clone()).collect();
    let locals: BTreeSet<String> = BTreeSet::new();
    block_is_quiet(&f.body, &params, &locals)
}

/// Quiet over a block: each statement quiet, and the (discarded) tail
/// expression — if any — pure. Locals are scoped to the block (cloned
/// in) so an inner `let` cannot leak out.
fn block_is_quiet(
    b: &Block,
    params: &BTreeSet<String>,
    locals: &BTreeSet<String>,
) -> bool {
    let mut locals = locals.clone();
    for s in &b.stmts {
        if !stmt_is_quiet(s, params, &mut locals) {
            return false;
        }
    }
    if let Some(tail) = &b.tail {
        if !expr_is_pure(tail, params, &locals) {
            return false;
        }
    }
    true
}

fn stmt_is_quiet(
    s: &Stmt,
    params: &BTreeSet<String>,
    locals: &mut BTreeSet<String>,
) -> bool {
    match s {
        Stmt::Let { name, value, .. } => {
            if !expr_is_pure(value, params, locals) {
                return false;
            }
            locals.insert(name.name.clone());
            true
        }
        Stmt::Assign {
            target, value, ..
        } => {
            // Target must be `self.<field…>` — self-headed, every tail
            // segment a Field (no Index store), and the RHS pure. The
            // assign op (`=` or compound `+=`/…) is irrelevant: a
            // compound op just reads-then-writes the same self field,
            // still a pure-valued self mutation.
            if target.head.name != "self" {
                return false;
            }
            if target.tail.is_empty() {
                return false; // bare `self = …` (rejected upstream anyway)
            }
            if !target
                .tail
                .iter()
                .all(|seg| matches!(seg, LValueSeg::Field(_)))
            {
                return false;
            }
            expr_is_pure(value, params, locals)
        }
        Stmt::If(ifstmt) => if_is_quiet(ifstmt, params, locals),
        Stmt::While { cond, body, .. } => {
            expr_is_pure(cond, params, locals)
                && block_is_quiet(body, params, locals)
        }
        Stmt::Block(b) => block_is_quiet(b, params, locals),
        Stmt::Return(opt, _) => {
            opt.as_ref()
                .map(|e| expr_is_pure(e, params, locals))
                .unwrap_or(true)
        }
        Stmt::Break(_) | Stmt::Continue(_) => true,
        // Everything else is NOT quiet: LetTuple, Match, For, Fail,
        // Yield, Terminate, Recovery, Violate, Send, ShmWrite, and any
        // bare Stmt::Expr (the carrier of a call / println / helper).
        _ => false,
    }
}

fn if_is_quiet(
    ifstmt: &IfStmt,
    params: &BTreeSet<String>,
    locals: &BTreeSet<String>,
) -> bool {
    if !expr_is_pure(&ifstmt.cond, params, locals) {
        return false;
    }
    if !block_is_quiet(&ifstmt.then_block, params, locals) {
        return false;
    }
    match ifstmt.else_block.as_deref() {
        None => true,
        Some(ElseBranch::Else(b)) => block_is_quiet(b, params, locals),
        Some(ElseBranch::ElseIf(inner)) => if_is_quiet(inner, params, locals),
    }
}

/// A pure (effect-free, non-trapping, non-capturing) expression. See
/// the module note: arithmetic excl. `/`,`%`; comparison; logical;
/// bitwise; over literals, `self.<field…>`, `param.field`, and locals.
fn expr_is_pure(
    e: &Expr,
    params: &BTreeSet<String>,
    locals: &BTreeSet<String>,
) -> bool {
    match e {
        Expr::Literal(_, _) => true,
        // Reading `self` (effect-free); only meaningful as a Field
        // receiver, but harmless on its own.
        Expr::KwSelf(_) => true,
        // A bare ident is pure ONLY if it is a LOCAL. A bare PARAMETER
        // ident is excluded (a flat payload is passed by pointer;
        // capturing it into `self` would alias publisher storage).
        Expr::Ident(i) => locals.contains(&i.name),
        Expr::Field { receiver, .. } => match receiver.as_ref() {
            // `param.field`: a flat payload's fields are by-value
            // scalars, so this is a pure value read.
            Expr::Ident(i) if params.contains(&i.name) => true,
            // `self.field`, `local.field`, `self.a.b`, …
            other => expr_is_pure(other, params, locals),
        },
        Expr::Binary { op, left, right, .. } => {
            bin_op_is_pure_safe(*op)
                && expr_is_pure(left, params, locals)
                && expr_is_pure(right, params, locals)
        }
        Expr::Unary { operand, .. } => {
            // Neg / Not / BitNot — all pure on scalars.
            expr_is_pure(operand, params, locals)
        }
        // Everything else bails: Call, Index, Path/Path2, Tuple, Array,
        // Struct, Block, If, Match, Sum, Prod, Approx, Range,
        // ArrayRepeat, Or — any of which is a call, an allocation, a
        // possibly-trapping access, or an unmodeled shape.
        _ => false,
    }
}

/// Binary ops that are guaranteed non-trapping and side-effect-free.
/// `/` and `%` are EXCLUDED: they lower to `sdiv`/`srem`, which trap on
/// divide-by-zero, and a trap is an observable program abort whose
/// timing would differ between the synchronous and deferred lowerings.
fn bin_op_is_pure_safe(op: BinOp) -> bool {
    use BinOp::*;
    match op {
        Add | Sub | Mul | Eq | NotEq | Lt | Gt | LtEq | GtEq | And | Or
        | BitAnd | BitOr | BitXor | Shl | Shr => true,
        Div | Mod => false,
    }
}
