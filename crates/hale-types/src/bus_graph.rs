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
//! checker's bus rules read the graph (F.40 phase 3, C4,
//! spec/semantics.md rules 7, 9 and 10): the walk's bound, cross-seed
//! and wildcard facts are columns of the graph's canonical subjects,
//! [`BusGraph::wires`], and a subject the graph cannot resolve is a
//! hole, [`BusGraph::holes`], that no rule fires on.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::*;
use hale_syntax::Span;

use crate::binding_rows::BindingRows;
use crate::resolve::TopScope;
use crate::symbol::{Bundle, TopSymbol};
use crate::topic_identity::TopicRows;

// === The walk =====================================================

/// A single publish site, captured during the walk with enough
/// context (owning locus type, payload-resolution key) to build a
/// `PublisherSite` later.
pub(crate) struct RawPub {
    pub(crate) locus: String,
    pub(crate) decl: usize,
    pub(crate) topic: Option<String>,
    pub(crate) key: String,
    pub(crate) subject: Subject,
    pub(crate) span: Span,
    pub(crate) id: NodeId,
}

/// A single subscribe site. `qualified` flags the cross-seed
/// `BusSubject::QualifiedTopic` shape; `keyed` flags a Phase-3
/// `where key == …` routing filter. Both are statically
/// unresolvable-to-a-single-call here, so they force ineligibility.
pub(crate) struct RawSub {
    pub(crate) locus: String,
    pub(crate) decl: usize,
    pub(crate) topic: Option<String>,
    pub(crate) handler: String,
    pub(crate) handler_span: Span,
    pub(crate) key: String,
    pub(crate) subject: Subject,
    pub(crate) span: Span,
    pub(crate) id: NodeId,
    pub(crate) qualified: bool,
    pub(crate) keyed: bool,
}

/// The product of one walk over the bundle's bus topology: the
/// eligibility gate's inputs, keyed by `BusSubject::canonical()`, and
/// each site's canonical subject, from which the graph's wire rows are
/// built.
pub(crate) struct BusWalk {
    pub(crate) bound: BTreeSet<String>,
    /// The wire subjects of the bound topics (the rows' `bound`).
    pub(crate) bound_wires: BTreeSet<String>,
    pub(crate) cross_seed: BTreeSet<String>,
    pub(crate) pub_sites: Vec<RawPub>,
    pub(crate) sub_sites: Vec<RawSub>,
    /// Every locus declaration, in walk order.
    pub(crate) decls: Vec<LocusDeclRow>,
    /// Every handler edge, in walk order.
    pub(crate) edges: Vec<BusEdge>,
}

/// Walk every locus's `bus { }` + `bindings { }` blocks once,
/// collecting the publisher/subscriber ends AND the per-site detail,
/// each site's subject resolved through the topic rows, over each
/// program's items. The one walk [`build_bus_graph`] and
/// [`stdlib_bus_rows`] read — do not duplicate it.
pub(crate) fn collect_bus_walk<'p>(
    programs: impl IntoIterator<Item = (&'p str, &'p [TopDecl])>,
    topics: &TopicRows,
    bindings: &BindingRows,
) -> BusWalk {
    // The bound set is the binding rows' projection, at both grains the
    // graph is keyed at (the entry's topic name and its wire subject);
    // and the canonical subject each entry binds, for the wire rows: the
    // topic's row, when one answers.
    let bound_wires = bindings
        .rows
        .iter()
        .filter_map(|r| match Subject::of_topic(&r.topic, topics) {
            Subject::Wire(wire) => Some(wire),
            Subject::Unresolved(_) => None,
        })
        .collect();
    let mut w = BusWalk {
        bound: bindings.bound_subjects(),
        bound_wires,
        cross_seed: BTreeSet::new(),
        pub_sites: Vec::new(),
        sub_sites: Vec::new(),
        decls: Vec::new(),
        edges: Vec::new(),
    };

    fn walk(items: &[TopDecl], w: &mut BusWalk, topics: &TopicRows, at: &mut LocusDeclRow) {
        for (i, item) in items.iter().enumerate() {
            at.path.push(i);
            match item {
                TopDecl::Locus(l) => {
                    let locus = l.name.name.clone();
                    let decl = w.decls.len();
                    w.decls.push(LocusDeclRow { name: locus.clone(), imported: l.imported, ..at.clone() });
                    // The topic a site names by a plain reference, as written.
                    let topic = |subject: &BusSubject| match subject {
                        BusSubject::Topic(id) => Some(id.name.clone()),
                        _ => None,
                    };
                    w.edges.extend(handler_edges(l, w.decls.len() - 1, topics));
                    for m in &l.members {
                        match m {
                            LocusMember::Bus(bb) => {
                                for bm in &bb.members {
                                    match bm {
                                        BusMember::Publish { subject, span, id, .. } => {
                                            let written = topic(subject);
                                            let key = subject.canonical().to_string();
                                            if matches!(subject, BusSubject::QualifiedTopic(_)) {
                                                w.cross_seed.insert(key.clone());
                                            }
                                            let subject = Subject::of(subject, topics);
                                            if let Some(d) = w.decls.last_mut() {
                                                d.publishes.push(subject.clone());
                                            }
                                            w.pub_sites.push(RawPub {
                                                locus: locus.clone(),
                                                decl,
                                                topic: written,
                                                key,
                                                subject,
                                                span: *span,
                                                id: *id,
                                            });
                                        }
                                        BusMember::Subscribe {
                                            subject,
                                            handler,
                                            key_filter,
                                            span,
                                            id,
                                            ..
                                        } => {
                                            let written = topic(subject);
                                            let key = subject.canonical().to_string();
                                            let qualified = matches!(
                                                subject,
                                                BusSubject::QualifiedTopic(_)
                                            );
                                            if qualified {
                                                w.cross_seed.insert(key.clone());
                                            }
                                            let subject = Subject::of(subject, topics);
                                            if let Some(d) = w.decls.last_mut() {
                                                d.subscribes.push((subject.clone(), handler.name.clone()));
                                            }
                                            // A diagnostic names the handler at the
                                            // declaration's own first `fn` of its
                                            // name, else at the declaration's name.
                                            let handler_span = l
                                                .members
                                                .iter()
                                                .find_map(|m| match m {
                                                    LocusMember::Fn(f) if f.name.name == handler.name => {
                                                        Some(f.name.span)
                                                    }
                                                    _ => None,
                                                })
                                                .unwrap_or(l.name.span);
                                            w.sub_sites.push(RawSub {
                                                locus: locus.clone(),
                                                decl,
                                                topic: written,
                                                handler: handler.name.clone(),
                                                handler_span,
                                                key,
                                                subject,
                                                span: *span,
                                                id: *id,
                                                qualified,
                                                keyed: key_filter.is_some(),
                                            });
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                TopDecl::Module(md) => {
                    at.modules.push(md.name.name.clone());
                    walk(&md.items, w, topics, at);
                    at.modules.pop();
                }
                _ => {}
            }
            at.path.pop();
        }
    }
    for (name, items) in programs {
        let mut at = LocusDeclRow { program: name.to_string(), ..LocusDeclRow::default() };
        walk(items, &mut w, topics, &mut at);
    }
    w
}

/// A locus declaration of the graph (F.40 phase 3, C4): the
/// declaration that wrote a site or a handler body, by its position,
/// so two loci of one name are two declarations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocusDeclRow {
    pub name: String,
    /// The `module { }` path it is declared under, outermost first;
    /// empty at the top level.
    pub modules: Vec<String>,
    /// The bundle program that holds it, and its item index at each
    /// depth (a module's contents under the module's index).
    pub program: String,
    pub path: Vec<usize>,
    /// An imported seed's declaration (`LocusDecl::imported`).
    pub imported: bool,
    /// Its `publish` subjects, in member order.
    pub publishes: Vec<Subject>,
    /// Its `subscribe` subjects with their handlers, in member order.
    pub subscribes: Vec<(Subject, String)>,
}

impl LocusDeclRow {
    /// The handlers of the subscriptions whose subject the declaration
    /// does not also publish, in member order: its genuine cross-context
    /// receives (spec/semantics.md rule 7). A self-publish→subscribe is
    /// devirtualized to a direct call, not a bus receive. Subjects are
    /// compared under the canonical key, so a topic published by its
    /// name and subscribed by its literal subject is a self-publish; an
    /// unresolved subject is compared as written.
    pub fn external_handlers(&self) -> Vec<&str> {
        self.subscribes
            .iter()
            .filter(|(subject, _)| !self.publishes.contains(subject))
            .map(|(_, handler)| handler.as_str())
            .collect()
    }

    /// The declaration this row names, in the bundle the graph was
    /// built over.
    pub fn decl<'b>(&self, bundle: &Bundle<'b>) -> Option<&'b LocusDecl> {
        let mut items: &'b [TopDecl] = &bundle.programs.get(&self.program)?.items;
        let (last, modules) = self.path.split_last()?;
        for i in modules {
            let TopDecl::Module(m) = items.get(*i)? else { return None };
            items = &m.items;
        }
        match items.get(*last)? {
            TopDecl::Locus(l) => Some(l),
            _ => None,
        }
    }
}

/// An edge of the graph (spec/semantics.md rule 10): declaration `decl`
/// subscribes `from` with a handler whose body sends to `to`, so a cell
/// on `from` can cause a cell on `to`. Both ends are wire subjects; a
/// subscription or a send the graph cannot resolve forms no edge. The
/// handler body is the declaration's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusEdge {
    pub from: String,
    /// The subscription's subject as written (`BusSubject::canonical`):
    /// how a diagnostic spells the node.
    pub from_written: String,
    pub to: String,
    /// The declaration ([`BusGraph::decls`]) that wrote the handler.
    pub decl: usize,
    pub handler: String,
    /// The send's span.
    pub span: Span,
    /// The send's identity: what the intra-locus rewrite's relation
    /// names (`IntraLocusRewrite::send`) when lowering turns the send
    /// into a direct call.
    pub send: NodeId,
    /// The send fires on every run of the handler: no `if`, `match` or
    /// loop encloses it.
    pub unconditional: bool,
}

/// The edges of one locus declaration: for each subscription, the
/// sends of its handler's body, the declaration's own `fn` of that name.
fn handler_edges(l: &LocusDecl, decl: usize, topics: &TopicRows) -> Vec<BusEdge> {
    let mut bodies: BTreeMap<&str, &Block> = BTreeMap::new();
    for m in &l.members {
        if let LocusMember::Fn(f) = m {
            bodies.insert(f.name.name.as_str(), &f.body);
        }
    }
    let mut edges = Vec::new();
    for m in &l.members {
        let LocusMember::Bus(bb) = m else { continue };
        for bm in &bb.members {
            let BusMember::Subscribe { subject, handler, .. } = bm else { continue };
            let Subject::Wire(from) = Subject::of(subject, topics) else { continue };
            let Some(body) = bodies.get(handler.name.as_str()) else { continue };
            let mut sends = Vec::new();
            sends_in_block(body, false, &mut sends);
            for (to, span, send, conditional) in sends {
                let Some(to) = send_subject(to, topics) else { continue };
                edges.push(BusEdge {
                    from: from.clone(),
                    from_written: subject.canonical().to_string(),
                    to,
                    decl,
                    handler: handler.name.clone(),
                    span,
                    send,
                    unconditional: !conditional,
                });
            }
        }
    }
    edges
}

/// The wire subject a `Topic <- v` send addresses: a string literal, or
/// a topic name its row answers. None for anything else: a computed
/// subject, a qualified path, a name no row answers.
fn send_subject(e: &Expr, topics: &TopicRows) -> Option<String> {
    match e {
        Expr::Literal(Literal::String(s), _) => Some(s.clone()),
        Expr::Ident(id) => Subject::of_topic(&id.name, topics).wire().map(str::to_string),
        _ => None,
    }
}

/// A send: its subject, span and identity, and whether it is guarded.
type SendSite<'a> = (&'a Expr, Span, NodeId, bool);

/// The sends of a block, each with whether an `if`, `match` or loop
/// encloses it. A plain `{ ... }` block always executes, so its sends
/// keep the enclosing answer; a `match` arm counts only when it is a
/// block.
fn sends_in_block<'a>(b: &'a Block, conditional: bool, out: &mut Vec<SendSite<'a>>) {
    for s in &b.stmts {
        match s {
            Stmt::Send { subject, span, id, .. } => out.push((subject, *span, *id, conditional)),
            Stmt::If(i) => sends_in_if(i, out),
            Stmt::Match(m) => {
                for arm in &m.arms {
                    if let MatchArmBody::Block(b) = &arm.body {
                        sends_in_block(b, true, out);
                    }
                }
            }
            Stmt::For { body, .. } | Stmt::While { body, .. } => sends_in_block(body, true, out),
            Stmt::Block(b) => sends_in_block(b, conditional, out),
            _ => {}
        }
    }
}

fn sends_in_if<'a>(i: &'a IfStmt, out: &mut Vec<SendSite<'a>>) {
    sends_in_block(&i.then_block, true, out);
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => sends_in_block(b, true, out),
        Some(ElseBranch::ElseIf(n)) => sends_in_if(n, out),
        None => {}
    }
}

/// A site's subject under the graph's canonical key (spec/semantics.md
/// rule 9): the wire subject (spec/model.md rule 8), or a hole.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Subject {
    /// A wire subject: a literal subject as written (a `**` pattern
    /// included), or the wire of the topic row a topic reference names.
    /// A topic published by its declared name and subscribed by its
    /// literal subject is one `Wire`.
    Wire(String),
    /// A subject the graph cannot resolve, as written (a path joined
    /// with `::`): a topic name no unbroken row answers, or a qualified
    /// path (`alias::Topic`) no import rename resolved.
    Unresolved(String),
}

impl Subject {
    /// The canonical subject a `bus { }` member names.
    pub(crate) fn of(subject: &BusSubject, topics: &TopicRows) -> Subject {
        match subject {
            BusSubject::Literal { subject, .. } => Subject::Wire(subject.clone()),
            BusSubject::Topic(id) => Subject::of_topic(&id.name, topics),
            BusSubject::QualifiedTopic(qn) => Subject::Unresolved(
                qn.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::"),
            ),
        }
    }

    /// The canonical subject a topic name names: its row's wire, or a
    /// hole when no unbroken row answers it.
    pub(crate) fn of_topic(name: &str, topics: &TopicRows) -> Subject {
        match topics.named(name) {
            Some(t) if !t.broken => Subject::Wire(t.wire.clone()),
            _ => Subject::Unresolved(name.to_string()),
        }
    }

    /// The wire subject, when the graph resolved one.
    pub fn wire(&self) -> Option<&str> {
        match self {
            Subject::Wire(w) => Some(w),
            Subject::Unresolved(_) => None,
        }
    }
}

/// One canonical subject of the graph: a wire subject some site names
/// or some topic carries, with the facts rule 9 reads (F.40 phase 3,
/// C4). A `**` pattern is not a row: its coverage is a column of the
/// rows it covers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WireRow {
    /// The first publish site on the subject, in walk order.
    pub published: Option<Span>,
    /// The first subscribe site on the subject, in walk order.
    pub subscribed: Option<Span>,
    /// A `bindings { }` entry binds a topic carrying the subject to a
    /// transport: an external peer is (or may be) its other end.
    pub bound: bool,
    /// A cross-seed reference (`alias::Topic`) may name the subject: a
    /// qualified path whose last segment is the name of a topic
    /// carrying it, or the subject itself. The other seed owns the
    /// other half.
    pub cross_seed: bool,
    /// A `**` publish pattern covers the subject.
    pub published_by_pattern: bool,
    /// A `**` subscription covers the subject.
    pub subscribed_by_pattern: bool,
}

/// A site whose subject the graph cannot resolve: recorded, and judged
/// by no rule (an unresolved subject is not a proven orphan).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hole {
    /// The subject as written.
    pub written: String,
    /// The locus whose `bus { }` block holds the site.
    pub locus: String,
    /// `true` for a publish, `false` for a subscription.
    pub publish: bool,
    pub span: Span,
}

/// The graph's canonical subjects and its holes, from the walk's sites.
fn wire_rows(walk: &BusWalk, topics: &TopicRows) -> (BTreeMap<String, WireRow>, Vec<Hole>) {
    let mut rows: BTreeMap<String, WireRow> = BTreeMap::new();
    let mut holes = Vec::new();
    for t in topics.iter().filter(|t| !t.broken) {
        rows.entry(t.wire.clone()).or_default();
    }
    let ends = walk
        .pub_sites
        .iter()
        .map(|p| (&p.subject, &p.locus, true, p.span))
        .chain(walk.sub_sites.iter().map(|s| (&s.subject, &s.locus, false, s.span)));
    let mut patterns: Vec<(&str, bool)> = Vec::new();
    for (subject, locus, publish, span) in ends {
        match subject {
            Subject::Wire(w) if w.contains("**") => patterns.push((w, publish)),
            Subject::Wire(w) => {
                let row = rows.entry(w.clone()).or_default();
                let first = if publish { &mut row.published } else { &mut row.subscribed };
                first.get_or_insert(span);
            }
            Subject::Unresolved(written) => holes.push(Hole {
                written: written.clone(),
                locus: locus.clone(),
                publish,
                span,
            }),
        }
    }
    for wire in &walk.bound_wires {
        if let Some(row) = rows.get_mut(wire) {
            row.bound = true;
        }
    }
    // The walk's cross-seed fact is the last segment of each qualified
    // path: it may name a topic of that name, or the subject it spells.
    for seg in &walk.cross_seed {
        let named = match Subject::of_topic(seg, topics) {
            Subject::Wire(w) => Some(w),
            Subject::Unresolved(_) => None,
        };
        for wire in named.iter().chain(std::iter::once(seg)) {
            if let Some(row) = rows.get_mut(wire) {
                row.cross_seed = true;
            }
        }
    }
    for (wire, row) in rows.iter_mut() {
        for (pattern, publish) in &patterns {
            if crate::wildcard_match(pattern, wire) {
                if *publish {
                    row.published_by_pattern = true;
                } else {
                    row.subscribed_by_pattern = true;
                }
            }
        }
    }
    (rows, holes)
}

// === Public graph =================================================

/// Where a locus type's handlers run relative to the main thread, read
/// from the placement table: the label of the set of domains its
/// instances run in ([`crate::placement::PlacementTable::domains_by_type`]).
///
/// `CrossPool`/`Pinned`/`Unknown` mean some instance's handler may run
/// on a *different* OS thread, so any later devirtualization must still
/// route through the mailbox/queue rather than a same-thread direct
/// call; `SameThread` is the placement where an intra-thread direct call
/// is the lowering #1b would pick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// Every instance runs on main: a root field with no entry or
    /// `cooperative(pool = main)`, anything nested under one, a literal
    /// main runs; or no instance is built at all.
    SameThread,
    /// An instance runs on a named cooperative pool other than `main`
    /// (and none on a pinned thread): placed there, or nested under a
    /// field placed there.
    CrossPool(String),
    /// An instance runs on a pinned thread: a field placed `pinned`,
    /// anything nested under one, or an adapter in `bindings { }`.
    Pinned,
    /// An instance runs where the table cannot say (a dynamic site of
    /// unknown domain), and none on a known thread off main. Never main.
    Unknown,
}

impl Placement {
    /// The label of a type whose instances run in `domains`: `Pinned` if
    /// any runs pinned, else `CrossPool` of the first pool one runs on,
    /// else `Unknown` if any runs where the table cannot say, else
    /// `SameThread`.
    pub fn of(domains: &crate::placement::TypeDomains, table: &crate::placement::PlacementTable) -> Placement {
        use crate::placement::DomainKind;
        let kinds: Vec<&DomainKind> = domains.known.iter().map(|d| &table.domain(*d).kind).collect();
        if kinds.iter().any(|k| matches!(k, DomainKind::Pinned { .. })) {
            return Placement::Pinned;
        }
        if let Some(name) = kinds.iter().find_map(|k| match k {
            DomainKind::Pool { name, .. } => Some(name.clone()),
            _ => None,
        }) {
            return Placement::CrossPool(name);
        }
        if domains.unknown {
            return Placement::Unknown;
        }
        Placement::SameThread
    }
}

/// Every type's [`Placement`], by the name lowering keys on: the bus
/// graph's labels and `check_bounded_bus`'s. A type the table has no
/// instance of runs nowhere and is absent (`SameThread` to a reader).
pub fn type_placements(table: &crate::placement::PlacementTable) -> BTreeMap<String, Placement> {
    table.domains_by_type().iter().map(|(name, d)| (name.clone(), Placement::of(d, table))).collect()
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
    ///       `Placement::SameThread`, and so is every publisher (a
    ///       CrossPool / Pinned / Unknown one runs, or may run, on
    ///       another OS thread and CANNOT be direct-called — it must
    ///       enqueue), and
    ///   (b) every subscriber handler is provably **QUIET** by the
    ///       syntactic effect-walk in [`handler_is_quiet`] — it
    ///       mutates ONLY its own `self` fields with pure expressions
    ///       and has no other effect.
    /// Defaults to `false` (default-bail). The FLAT-payload condition
    /// is the third leg of the gate, [`SubjectInfo::payload_flat`]: the
    /// plan's flavor ANDs the two (`DispatchFlavor::of`). A false
    /// positive is an observable-ordering bug, so this stays
    /// conservative.
    pub direct_call_eligible: bool,
    /// The direct-call gate's third leg: the subject's payload is flat
    /// ([`payload_is_flat`]) at every site that names its type (the
    /// publishers', else the subscribers'), and at least one does. A
    /// direct call hands the publisher's live storage to the handler,
    /// which only pointer-free POD survives.
    pub payload_flat: bool,
    /// The sends on this subject the intra-locus rewrite turned into
    /// direct calls, as (publishing locus, subscriber handler) pairs
    /// (F.40 boundary 7). Empty from [`build_bus_graph`]: a graph over
    /// an authored program still sees those sends, and the resolved
    /// program fills this from the rewrite's relation, so a graph over
    /// the rewritten program still knows every publish the rewrite
    /// removed from the program's text.
    pub direct_sends: Vec<(String, String)>,
    /// The topic references the topic rewrite turned into this wire
    /// subject, as (site, topic as written) pairs (F.40 phase 2.1b).
    /// Empty from [`build_bus_graph`], as `direct_sends` is: the
    /// resolved program fills it from the rewrite's relation, so a
    /// graph keyed by wire subject still knows which declaration each
    /// subscribe, publish and send named.
    pub written_topics: Vec<(hale_syntax::ast::NodeId, String)>,
}

/// A `publish` site with what the gate reads of it, in the site's own
/// program: its subject as the graph keys it, its payload and whether
/// the payload is flat.
#[derive(Debug, Clone)]
pub struct PublishRow {
    /// The `publish` member's identity.
    pub id: NodeId,
    pub locus: String,
    /// The declaration that holds the site: its index in the walk's
    /// [`BusGraph::decls`] (a stdlib row's, in the stdlib's walk).
    pub decl: usize,
    /// The topic the site names by a plain reference, as written; `None`
    /// for a literal subject or a qualified path.
    pub topic: Option<String>,
    /// The subject as written (`BusSubject::canonical`): a topic's
    /// name, a literal subject, a qualified path joined with `::`.
    pub key: String,
    /// The payload's type name, `"?"` when it does not resolve.
    pub payload: String,
    /// The payload resolves and is flat ([`payload_is_flat`]).
    pub flat: bool,
    pub span: Span,
}

/// A `subscribe` site with what the gate reads of it, in the site's own
/// program.
#[derive(Debug, Clone)]
pub struct SubscribeRow {
    /// The `subscribe` member's identity.
    pub id: NodeId,
    pub locus: String,
    /// The declaration that holds the site, as [`PublishRow::decl`].
    pub decl: usize,
    /// The topic the site names by a plain reference, as
    /// [`PublishRow::topic`].
    pub topic: Option<String>,
    pub handler: String,
    /// Where a diagnostic names the handler: the name of the
    /// declaration's own first `fn` of the handler's name, else the
    /// declaration's name.
    pub handler_span: Span,
    /// The subject as written, as [`PublishRow::key`].
    pub key: String,
    pub payload: String,
    pub flat: bool,
    /// The handler is the locus's own `fn` and provably quiet
    /// ([`handler_is_quiet`]): the direct-call gate's second leg.
    pub quiet: bool,
    /// A cross-seed qualified subject (`alias::Topic`).
    pub qualified: bool,
    /// A Phase-3 `where key == …` routing filter.
    pub keyed: bool,
    pub span: Span,
}

/// The sites of a graph, each with the facts its program answers for it,
/// and the program-wide facts the gate reads: everything the gate
/// decides a subject from except the placement table, which labels a
/// locus when the subjects are assembled ([`BusRows::subjects`]). Rows
/// over two programs concatenate into rows over both (F.40 phase 3,
/// C5): lowering's graph is the snapshot's rows, rekeyed by the topic
/// rewrite, followed by the stdlib's ([`lowering_bus_graph`]).
#[derive(Debug, Clone, Default)]
pub struct BusRows {
    /// The program has an entry point (a top-level `main locus` or
    /// `fn main`): its bus graph is complete.
    pub closed_world: bool,
    /// The subjects a `bindings { }` entry binds, at both grains (the
    /// topic name and its wire subject).
    pub bound: BTreeSet<String>,
    /// The subjects a cross-seed qualified path names.
    pub cross_seed: BTreeSet<String>,
    /// Every `publish` site, in walk order.
    pub publishes: Vec<PublishRow>,
    /// Every `subscribe` site, in walk order.
    pub subscribes: Vec<SubscribeRow>,
}

/// The whole-bundle bus graph. `subjects` is keyed by
/// `BusSubject::canonical()` (the gates, the model, hale/busGraph);
/// `wires` by the canonical subject, the wire, which the checker's bus
/// rules read (F.40 phase 3, C4).
#[derive(Debug, Clone, Default)]
pub struct BusGraph {
    pub subjects: BTreeMap<String, SubjectInfo>,
    /// The sites `subjects` is assembled from ([`BusRows::subjects`]).
    pub rows: BusRows,
    /// Every wire subject a site names or an unbroken topic carries.
    pub wires: BTreeMap<String, WireRow>,
    /// The sites whose subject the graph cannot resolve, in walk order.
    pub holes: Vec<Hole>,
    /// Every locus declaration, in walk order: what an edge's `decl`
    /// indexes.
    pub decls: Vec<LocusDeclRow>,
    /// Every handler edge, in walk order (rule 10's graph).
    pub edges: Vec<BusEdge>,
}

impl BusGraph {
    /// The row of a locus declaration of `bundle`, the bundle the graph
    /// was built over: the declaration itself, not its name.
    pub fn decl_row(&self, bundle: &Bundle<'_>, decl: &LocusDecl) -> Option<&LocusDeclRow> {
        self.decls.iter().find(|row| row.decl(bundle).is_some_and(|d| std::ptr::eq(d, decl)))
    }

    /// Rule 10's query (spec/semantics.md rule 10): the first cycle
    /// reachable from subject `root` over the edges `keep` admits,
    /// depth-first with each subject's edges in graph order, as the
    /// edges that close it, in order; `None` when there is none.
    pub fn cycle_from(&self, root: &str, keep: &dyn Fn(&BusEdge) -> bool) -> Option<Vec<&BusEdge>> {
        fn dfs<'g>(
            g: &'g BusGraph,
            node: &'g str,
            keep: &dyn Fn(&BusEdge) -> bool,
            gray: &mut BTreeSet<&'g str>,
            black: &mut BTreeSet<&'g str>,
            nodes: &mut Vec<&'g str>,
            path: &mut Vec<&'g BusEdge>,
        ) -> Option<Vec<&'g BusEdge>> {
            gray.insert(node);
            nodes.push(node);
            for e in g.edges.iter().filter(|e| e.from == node && keep(e)) {
                if gray.contains(e.to.as_str()) {
                    let start = nodes.iter().position(|n| *n == e.to).unwrap_or(0);
                    let mut cycle = path[start..].to_vec();
                    cycle.push(e);
                    return Some(cycle);
                }
                if !black.contains(e.to.as_str()) {
                    path.push(e);
                    if let Some(c) = dfs(g, &e.to, keep, gray, black, nodes, path) {
                        return Some(c);
                    }
                    path.pop();
                }
            }
            nodes.pop();
            gray.remove(node);
            black.insert(node);
            None
        }
        let root = self.edges.iter().find(|e| e.from == root)?.from.as_str();
        dfs(self, root, keep, &mut BTreeSet::new(), &mut BTreeSet::new(), &mut Vec::new(), &mut Vec::new())
    }

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

    /// The graph's per-subject gate facts in the plan's shape (GH #476
    /// Change 8): what `DispatchPlan::from_gates` decides a flavor
    /// from. Publisher loci are sorted and deduplicated; subscribers
    /// keep the graph's site order.
    pub fn dispatch_gates(&self) -> Vec<hale_model::DispatchGate> {
        self.subjects
            .iter()
            .map(|(subject, info)| hale_model::DispatchGate {
                subject: subject.clone(),
                static_eligible: info.eligible,
                direct_eligible: info.direct_call_eligible,
                payload_flat: info.payload_flat,
                ineligible_reason: info
                    .ineligible_reason
                    .as_ref()
                    .map(|r| r.tag().to_string()),
                publisher_loci: {
                    let mut p: Vec<String> = info
                        .publishers
                        .iter()
                        .map(|s| s.locus.clone())
                        .collect();
                    p.sort();
                    p.dedup();
                    p
                },
                subscribers: info
                    .subscribers
                    .iter()
                    .map(|s| (s.locus.clone(), s.handler.clone()))
                    .collect(),
            })
            .collect()
    }
}

/// Build the authoritative [`BusGraph`] for a bundle: the snapshot's
/// `bus_graph` family (`Snapshot::demand_bus_graph`), and the one
/// producer a bundle no snapshot holds builds through. Run this AFTER
/// typecheck so `top` carries resolved payload types.
///
/// Reads the one walk, [`collect_bus_walk`]: each site's row, with what
/// its program answers for it ([`BusRows`]), the subjects assembled from
/// the rows with the placement table's labels ([`BusRows::subjects`]),
/// and the canonical subjects the checker's bus rules read. The
/// bound-topic set is the binding rows' projection; the closed world
/// reads the entry row.
pub fn build_bus_graph(
    bundle: &Bundle<'_>,
    top: &TopScope,
    bindings: &BindingRows,
    placement: &crate::placement::PlacementTable,
    entry: &crate::entry::EntryRow,
) -> BusGraph {
    let programs = bundle.programs.iter().map(|(name, p)| (name.as_str(), p.items.as_slice()));
    let walk = collect_bus_walk(programs, &top.topics, bindings);
    let (wires, holes) = wire_rows(&walk, &top.topics);
    // Closed-world gate input (DEVIRT-ONLY notion): a complete,
    // closed-world program is one with an ENTRY POINT — a bare
    // top-level `fn main` free function OR an entry (the entry row's:
    // the seed's own top-level `main locus`, F.40 phase 3, E0). Either
    // produces an executable whose every subscriber is statically
    // declared in-bundle (an executable cannot gain subscribers at
    // runtime — there is no dynamic `subscribe`), so the bus graph
    // is complete.
    //
    // This is deliberately BROADER than `check::check_bus_graph`'s
    // diagnostics gate, which stays entry-only to keep its
    // orphan/dead-receiver warnings over-fire-conscious. The two
    // notions are separate by design — do not unify them. The
    // canonical `fn main` entry shape mirrors codegen's
    // `TopDecl::Fn(f) if f.name.name == "main"` lookup.
    let has_entry_point = entry.entry().is_some()
        || bundle
            .programs
            .values()
            .any(|p| p.items.iter().any(|i| matches!(i, TopDecl::Fn(f) if f.name.name == "main")));
    let rows = bus_rows(bundle, top, &walk, has_entry_point);
    let subjects = rows.subjects(placement);
    BusGraph { subjects, rows, wires, holes, decls: walk.decls, edges: walk.edges }
}

/// The rows of a walk: each site with its payload, its flatness and
/// (a subscription's) whether its handler is quiet, as the site's own
/// program answers them. `closed_world` is the program's entry point
/// ([`build_bus_graph`]).
fn bus_rows(bundle: &Bundle<'_>, top: &TopScope, walk: &BusWalk, closed_world: bool) -> BusRows {
    // The payload's flatness, over the resolved type the site names.
    let flat = |locus: &str, key: &str| {
        resolve_payload_ty(top, locus, key).is_some_and(|t| payload_is_flat(bundle, top, t))
    };
    BusRows {
        closed_world,
        bound: walk.bound.clone(),
        cross_seed: walk.cross_seed.clone(),
        publishes: walk
            .pub_sites
            .iter()
            .map(|p| PublishRow {
                id: p.id,
                locus: p.locus.clone(),
                decl: p.decl,
                topic: p.topic.clone(),
                key: p.key.clone(),
                payload: resolve_payload(top, &p.locus, &p.key),
                flat: flat(&p.locus, &p.key),
                span: p.span,
            })
            .collect(),
        subscribes: walk
            .sub_sites
            .iter()
            .map(|s| SubscribeRow {
                id: s.id,
                locus: s.locus.clone(),
                decl: s.decl,
                topic: s.topic.clone(),
                handler: s.handler.clone(),
                handler_span: s.handler_span,
                key: s.key.clone(),
                payload: resolve_payload(top, &s.locus, &s.key),
                flat: flat(&s.locus, &s.key),
                quiet: find_handler_fn(bundle, &s.locus, &s.handler).map(handler_is_quiet).unwrap_or(false),
                qualified: s.qualified,
                keyed: s.keyed,
                span: s.span,
            })
            .collect(),
    }
}

impl BusRows {
    /// The subjects the rows assemble into, each with its gate, the
    /// placement table labelling each site's locus: one per subject a
    /// site names, keyed as the site writes it.
    pub fn subjects(&self, placement: &crate::placement::PlacementTable) -> BTreeMap<String, SubjectInfo> {
        let placements = type_placements(placement);
        let label = |locus: &str| placements.get(locus).cloned().unwrap_or(Placement::SameThread);

        // Gather every subject that appears on either end.
        let keys: BTreeSet<&str> = self
            .publishes
            .iter()
            .map(|p| p.key.as_str())
            .chain(self.subscribes.iter().map(|s| s.key.as_str()))
            .collect();

        let mut subjects: BTreeMap<String, SubjectInfo> = BTreeMap::new();
        for key in keys {
            let pubs: Vec<&PublishRow> = self.publishes.iter().filter(|p| p.key == key).collect();
            let subs: Vec<&SubscribeRow> = self.subscribes.iter().filter(|s| s.key == key).collect();
            let publishers: Vec<PublisherSite> = pubs
                .iter()
                .map(|p| PublisherSite { locus: p.locus.clone(), payload: p.payload.clone(), span: p.span })
                .collect();
            let subscribers: Vec<SubscriberSite> = subs
                .iter()
                .map(|s| SubscriberSite {
                    locus: s.locus.clone(),
                    handler: s.handler.clone(),
                    placement: label(&s.locus),
                    payload: s.payload.clone(),
                    span: s.span,
                })
                .collect();

            let reason = self.classify(key);
            let eligible = reason.is_none();
            // Direct-call gate (slice-2). STRONGER than `eligible`:
            // additionally every subscriber must be same-thread AND its
            // handler provably quiet (the flat-payload leg is ANDed in at
            // the codegen publish site). Default-bail: a missing handler
            // body or any unmodeled placement/effect ⟹ not direct.
            //
            // GH #253 follow-up (caught by the devirt differential on
            // corpus fixture 72): every PUBLISHER must be same-thread
            // too. A direct call executes the handler on the PUBLISHING
            // thread — a pinned (or pooled) publisher would run a
            // same-thread subscriber's handler off-main, and two such
            // publishers run it CONCURRENTLY: a `self.seen + 1`
            // read-modify-write loses updates (observed as a rare
            // saw-1-of-2 under CI load). Off-thread publishers must
            // stay on the enqueue path, which serializes dispatch on
            // the draining thread.
            let direct_call_eligible = eligible
                && !subscribers.is_empty()
                && pubs.iter().all(|p| label(&p.locus) == Placement::SameThread)
                && subscribers.iter().all(|s| s.placement == Placement::SameThread)
                && subs.iter().all(|s| s.quiet);
            // The third leg: the payload's flatness at each site that
            // names its type (the publishers', else the subscribers').
            let site_flat: Vec<bool> = if pubs.is_empty() {
                subs.iter().map(|s| s.flat).collect()
            } else {
                pubs.iter().map(|p| p.flat).collect()
            };
            let payload_flat = !site_flat.is_empty() && site_flat.iter().all(|f| *f);
            subjects.insert(
                key.to_string(),
                SubjectInfo {
                    publishers,
                    subscribers,
                    eligible,
                    ineligible_reason: reason,
                    direct_call_eligible,
                    payload_flat,
                    direct_sends: Vec::new(),
                    written_topics: Vec::new(),
                },
            );
        }
        subjects
    }

    /// The soundness-critical gate. Returns `None` when the subject is
    /// statically devirtualizable, else the first failing reason in the
    /// canonical check order. DEFAULTS TO INELIGIBLE: every condition
    /// must be positively cleared.
    fn classify(&self, key: &str) -> Option<StaticIneligible> {
        // 1) Closed-world: the program has an entry point (`fn main` or
        //    a `main locus`), so its bus graph is complete.
        if !self.closed_world {
            return Some(StaticIneligible::OpenWorld);
        }
        // 2) No transport adapter binding.
        if self.bound.contains(key) {
            return Some(StaticIneligible::TransportBound);
        }
        // 3) No wildcard — neither the subject itself nor any pattern
        //    covering it on either end.
        let covers = |pattern: &str| pattern.contains("**") && crate::wildcard_match(pattern, key);
        if key.contains("**")
            || self.publishes.iter().any(|p| covers(&p.key))
            || self.subscribes.iter().any(|s| covers(&s.key))
        {
            return Some(StaticIneligible::Wildcard);
        }
        // 4) Not referenced cross-seed.
        if self.cross_seed.contains(key) {
            return Some(StaticIneligible::CrossSeed);
        }
        // 5) Every subscriber resolves to a concrete local handler:
        //    a plain `Topic`/literal subject with no routing key. A
        //    qualified subject or a Phase-3 `where key` filter is not
        //    a single-call dispatch — ineligible.
        for s in self.subscribes.iter().filter(|s| s.key == key) {
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
}

/// The rows of the stdlib's sites in a merged program: `stdlib` is the
/// stdlib's items, the tail of the program `bundle` holds, and `top` the
/// scope over it. The snapshot's families are derived over the checked
/// programs, which hold no stdlib, so the stdlib's rows are the one part
/// of lowering's graph the merged program answers itself
/// ([`lowering_bus_graph`]); each site answers over the whole merged program,
/// as it did when the graph was built over it. The stdlib has no entry
/// point and binds nothing.
pub fn stdlib_bus_rows(bundle: &Bundle<'_>, top: &TopScope, stdlib: &[TopDecl]) -> BusRows {
    let walk = collect_bus_walk([(crate::snapshot::STDLIB_SEED, stdlib)], &top.topics, &BindingRows::default());
    bus_rows(bundle, top, &walk, false)
}

/// Lowering's bus graph (F.40 phase 3, C5): the snapshot's rows, read
/// for the program lowering walks through the view's correspondence,
/// followed by the stdlib's ([`stdlib_bus_rows`]), assembled into subjects
/// with the placement table's labels.
///
/// A user site keeps its identity in the merged program
/// ([`crate::correspondence::Image::Checked`]), and the topic rewrite
/// turned the topic it named into the topic's wire literal, so its row
/// is keyed by the relation's wire (`TopicRewrite::wire`), the subject
/// the merged site spells. Every other fact of the row is the checked
/// site's: a site's payload, flatness and handler are its own program's
/// facts, which neither the rewrites nor the merge change. A stdlib
/// row's site is a stdlib site of the correspondence
/// ([`crate::correspondence::Image::Stdlib`]).
///
/// The graph is its subjects and its rows: the checker's rows (the wire
/// rows, the holes, the declarations and the edges) are the snapshot's,
/// over the checked programs, and lowering reads none of them.
pub fn lowering_bus_graph(
    snapshot: &BusGraph,
    stdlib: BusRows,
    topic_rewrites: &[hale_syntax::desugar::TopicRewrite],
    correspondence: &crate::correspondence::Correspondence,
    placement: &crate::placement::PlacementTable,
) -> Result<BusGraph, String> {
    use crate::correspondence::Image;
    let wire_of: BTreeMap<u32, &str> =
        topic_rewrites.iter().filter(|r| !r.site.is_none()).map(|r| (r.site.0, r.wire.as_str())).collect();
    let rekey = |id: NodeId, key: &mut String| -> Result<(), String> {
        let Some(Image::Checked(_)) = correspondence.image(id) else {
            return Err(format!("the bus site {} has no checked image in the merged program", id.0));
        };
        if let Some(wire) = wire_of.get(&id.0) {
            *key = wire.to_string();
        }
        Ok(())
    };
    let stdlib_site = |id: NodeId| -> Result<(), String> {
        match correspondence.image(id) {
            Some(Image::Stdlib(_)) => Ok(()),
            _ => Err(format!("the stdlib's bus site {} is no stdlib site of the merged program", id.0)),
        }
    };
    let mut rows = snapshot.rows.clone();
    for p in &mut rows.publishes {
        rekey(p.id, &mut p.key)?;
    }
    for s in &mut rows.subscribes {
        rekey(s.id, &mut s.key)?;
    }
    for p in &stdlib.publishes {
        stdlib_site(p.id)?;
    }
    for s in &stdlib.subscribes {
        stdlib_site(s.id)?;
    }
    rows.closed_world |= stdlib.closed_world;
    rows.bound.extend(stdlib.bound);
    rows.cross_seed.extend(stdlib.cross_seed);
    rows.publishes.extend(stdlib.publishes);
    rows.subscribes.extend(stdlib.subscribes);
    let subjects = rows.subjects(placement);
    Ok(BusGraph { subjects, rows, ..BusGraph::default() })
}

/// The dispatch gates (F.40 phase 4, S9): the one gate set the dispatch
/// plan is derived from, the snapshot's `dispatch` family
/// (`Snapshot::demand_dispatch_gates`). They are the gates of lowering's
/// graph ([`lowering_bus_graph`]) without the merge it is read through:
/// the snapshot's rows, each keyed by the wire subject the program
/// lowering walks spells it with (a plain topic reference by the scope's
/// topic row's wire, which the topic rewrite writes there; any other
/// subject as written), in the snapshot's walk order, followed by the
/// stdlib's rows ([`crate::stdlib_bodies::bus_rows`], once per process),
/// assembled into subjects with the placement table's labels. A
/// subject's subscribers are in registration order, the order the direct
/// lowering bakes and the plan's digest frames.
pub fn derive_dispatch_gates(
    snapshot: &BusGraph,
    top: &TopScope,
    placement: &crate::placement::PlacementTable,
) -> Vec<hale_model::DispatchGate> {
    let mut rows = snapshot.rows.clone();
    let rekey = |topic: &Option<String>, key: &mut String| {
        if let Some(topic) = topic {
            *key = top.topics.named(topic).map_or_else(|| topic.clone(), |row| row.wire.clone());
        }
    };
    for p in &mut rows.publishes {
        rekey(&p.topic, &mut p.key);
    }
    for s in &mut rows.subscribes {
        rekey(&s.topic, &mut s.key);
    }
    if let Some(stdlib) = crate::stdlib_bodies::bus_rows() {
        rows.closed_world |= stdlib.closed_world;
        rows.bound.extend(stdlib.bound.iter().cloned());
        rows.cross_seed.extend(stdlib.cross_seed.iter().cloned());
        rows.publishes.extend(stdlib.publishes.iter().cloned());
        rows.subscribes.extend(stdlib.subscribes.iter().cloned());
    }
    let subjects = rows.subjects(placement);
    BusGraph { subjects, rows, ..BusGraph::default() }.dispatch_gates()
}

/// The resolved payload type of a site on `key`: the declared topic's,
/// else the locus's publish or subscribe declaration on that subject.
fn resolve_payload_ty<'t>(top: &'t TopScope, locus: &str, key: &str) -> Option<&'t crate::ty::Ty> {
    for sym in top.symbols.values() {
        if let TopSymbol::Topic(t) = sym {
            if t.name == key || t.wire_subject == key {
                return Some(&t.payload);
            }
        }
    }
    if let Some(TopSymbol::Locus(l)) = top.lookup(locus) {
        if let Some(p) = l.bus_publishes.iter().find(|p| p.subject == key) {
            return Some(&p.payload);
        }
        if let Some(s) = l.bus_subscribes.iter().find(|s| s.subject == key) {
            return Some(&s.payload);
        }
    }
    None
}

/// Whether a bus payload of type `ty` is flat: a struct whose every
/// field is an inline-by-value scalar (`Int`, `Float`, `Bool`,
/// `Decimal`, `Duration`, or an enum with no payload variant), and
/// nothing else. A payload-carrying enum, a pointer-bearing field
/// (`String`, `Bytes`, `Time`, a view, a nested struct, an array, a
/// tuple, a locus) or a type the scope cannot resolve is not flat: the
/// default is false. The direct-call gate's third leg (the gate's
/// `payload_flat` column): codegen's `bus_payload_is_flat` rule, moved
/// verbatim onto resolved types (F.40 phase 3, P3 3 of 3), so the plan
/// decides the flavor; the codec keeps its own copy over lowered types,
/// and lowering refuses a plan whose column disagrees with it.
pub fn payload_is_flat(bundle: &Bundle<'_>, top: &TopScope, ty: &crate::ty::Ty) -> bool {
    use crate::symbol::TypeKind;
    use crate::ty::Ty;
    // A generic instantiation resolves to its monomorph's name
    // (`Box<Int>` is `Box_Int`, `crate::resolve`), which no scope
    // declares: its fields are the generic declaration's, each type
    // parameter replaced by the argument the name's tokens spell, as
    // codegen's monomorph lays them out.
    fn generic_instance_fields(bundle: &Bundle<'_>, name: &str) -> Option<Vec<Ty>> {
        let decl = bundle.programs.values().find_map(|p| {
            flat_decls(&p.items).find_map(|it| match it {
                TopDecl::Type(t)
                    if !t.generics.is_empty()
                        && name.strip_prefix(t.name.name.as_str()).is_some_and(|r| r.starts_with('_')) =>
                {
                    Some(t)
                }
                _ => None,
            })
        })?;
        let tokens: Vec<&str> = name[decl.name.name.len() + 1..].split('_').collect();
        if tokens.len() != decl.generics.len() {
            return None;
        }
        let arg = |tok: &str| -> Ty {
            let prim = [
                PrimType::Int,
                PrimType::Float,
                PrimType::Bool,
                PrimType::String,
                PrimType::Duration,
                PrimType::Decimal,
                PrimType::Time,
                PrimType::Bytes,
                PrimType::BytesView,
                PrimType::BytesMut,
                PrimType::StringView,
            ]
            .into_iter()
            .find(|p| crate::ty::generic_arg_mangle_token(*p) == Some(tok));
            prim.map(Ty::Prim).unwrap_or_else(|| Ty::Named(tok.to_string()))
        };
        let TypeDeclBody::Struct(fields) = &decl.body else { return None };
        Some(
            fields
                .iter()
                .map(|f| match &f.ty {
                    TypeExpr::Primitive(p, _) => Ty::Prim(*p),
                    TypeExpr::Named { path, generic_args, .. } if path.segments.len() == 1 && generic_args.is_empty() => {
                        let n = &path.segments[0].name;
                        match decl.generics.iter().position(|g| g.name.name == *n) {
                            Some(i) => arg(tokens[i]),
                            None => Ty::Named(n.clone()),
                        }
                    }
                    _ => Ty::Unknown,
                })
                .collect(),
        )
    }
    fn field_is_flat_scalar(top: &TopScope, ty: &Ty, depth: usize) -> bool {
        match ty {
            Ty::Prim(p) => matches!(p, PrimType::Int | PrimType::Float | PrimType::Bool | PrimType::Decimal | PrimType::Duration),
            Ty::Named(n) if depth < 16 => match top.lookup(n) {
                Some(TopSymbol::Type(t)) => match &t.kind {
                    TypeKind::Enum(variants) => variants.iter().all(|v| v.fields.is_empty()),
                    TypeKind::Alias(a) => field_is_flat_scalar(top, a, depth + 1),
                    TypeKind::Struct(_) => false,
                },
                _ => false,
            },
            _ => false,
        }
    }
    fn flat(bundle: &Bundle<'_>, top: &TopScope, ty: &Ty, depth: usize) -> bool {
        match ty {
            Ty::Named(n) if depth < 16 => match top.lookup(n) {
                Some(TopSymbol::Type(t)) => match &t.kind {
                    TypeKind::Struct(fields) => fields.iter().all(|f| field_is_flat_scalar(top, &f.ty, 0)),
                    TypeKind::Alias(a) => flat(bundle, top, a, depth + 1),
                    TypeKind::Enum(_) => false,
                },
                None => generic_instance_fields(bundle, n)
                    .is_some_and(|fields| fields.iter().all(|f| field_is_flat_scalar(top, f, 0))),
                _ => false,
            },
            _ => false,
        }
    }
    flat(bundle, top, ty, 0)
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
