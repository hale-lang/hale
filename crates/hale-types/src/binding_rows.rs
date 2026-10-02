//! The `bindings` family (F.40 phase 3, P2): one row per `bindings { }`
//! entry, the decisions about it made once.
//!
//! Before this row the decisions were made at five sites that did not
//! always agree: the checker's role inference walked the bundle for
//! publishers and subscribers by wire subject, the model builder asked
//! the desugar's `binding_role_for` by topic name, the set of bound
//! topics was collected three times (the `or wait` legality check, the
//! gate walk, the bus graph), and codegen chose transport, adapter,
//! codec and producer-versus-attach again at emission. The row holds
//! each decision once, and those sites read it.
//!
//! The rows cover every entry of every locus of the bundle, an imported
//! `main`'s included ([`BindingRow::imported`]: its entries bind nothing
//! and count toward nothing a diagnostic counts, but the bound-topic set
//! has always held them). A consumer filters on the flags it needs; none
//! walks the declarations itself.

use std::collections::BTreeSet;

use hale_graph::ids::SiteId;
use hale_syntax::ast::{
    BindingEntry, BusMember, BusSubject, LocusDecl, LocusMember, TopDecl, TransportRole,
    TransportSpec,
};
use hale_syntax::Span;

use crate::capability::Transport;
use crate::resolve::TopScope;
use crate::Bundle;

/// The locus the substrate unix transport instantiates, by role: the
/// stdlib locus a `unix(...)` entry is sugar for (GH #233). Its failure
/// is the transport loss the declaring locus's `on_failure` handles.
pub const UNIX_LISTEN_LOCUS: &str = "__StdBusUnixListenTransport";
pub const UNIX_CONNECT_LOCUS: &str = "__StdBusUnixConnectTransport";

/// One `bindings { }` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingRow {
    /// The entry's site, as the snapshot minted it. `None` only on a
    /// bundle nothing minted (the checker's test entries).
    pub site: Option<SiteId>,
    pub span: Span,
    /// The declaring locus, with the facts the consumers' filters read.
    pub locus: String,
    pub is_main: bool,
    /// Merged from an imported seed: the entry binds nothing here.
    pub imported: bool,
    /// Declared inside a `module { }`, at any depth.
    pub module_nested: bool,
    /// The topic as the entry names it, and where.
    pub topic: String,
    pub topic_span: Span,
    /// The topic's wire subject, when a topic row declares it.
    pub wire: Option<String>,
    pub transport: Transport,
    /// The role: the entry's own, else the one the topic's ends decide
    /// (publish-only is `Connect`, subscribe-only `Listen`). `None` for a
    /// transport without a role (adapter, shm_ring), and for a `unix`
    /// entry no role is inferable for (both ends, or neither): the
    /// checker's diagnostic, and codegen refuses it.
    pub role: Option<TransportRole>,
    /// Whether some locus of the bundle publishes the topic / subscribes
    /// to it by a topic reference (the ends the role rule reads).
    pub publishes: bool,
    pub subscribes: bool,
    /// The adapter locus a `Adapter` entry names.
    pub adapter: Option<String>,
    /// The codec locus a `codec(L { ... })` clause names.
    pub codec: Option<String>,
    /// Whether the bundle publishes the topic's wire subject, by topic
    /// reference or by literal subject: a `layout:`-bound `shm_ring`
    /// entry creates the foreign ring (the producer) when it does, and
    /// attaches it read-only at a subscriber's birth when it does not.
    pub producer: bool,
    /// The stdlib locus the entry instantiates as its transport, whose
    /// failure is the transport loss the declaring locus's `on_failure`
    /// handles: a `unix` entry's, by role. `None` for the transports with
    /// no locus of their own to name.
    pub loss_locus: Option<&'static str>,
    /// Where the entry is in the bundle: the program, the path of item
    /// indices to the locus, the member and the entry.
    at: (String, Vec<usize>, usize, usize),
}

impl BindingRow {
    /// The topic's canonical key: its wire subject, the topic's own name
    /// when no row declares it.
    pub fn key(&self) -> &str {
        self.wire.as_deref().unwrap_or(&self.topic)
    }

    /// The entry this row describes, in the bundle it was read from.
    pub fn entry<'b>(&self, bundle: &Bundle<'b>) -> Option<&'b BindingEntry> {
        let (program, path, member, entry) = &self.at;
        match locus_at(bundle, program, path)?.members.get(*member)? {
            LocusMember::Bindings(bb) => bb.entries.get(*entry),
            _ => None,
        }
    }
}

fn locus_at<'b>(bundle: &Bundle<'b>, program: &str, path: &[usize]) -> Option<&'b LocusDecl> {
    let mut items: &'b [TopDecl] = &bundle.programs.get(program)?.items;
    let (last, modules) = path.split_last()?;
    for i in modules {
        let TopDecl::Module(m) = items.get(*i)? else { return None };
        items = &m.items;
    }
    match items.get(*last)? {
        TopDecl::Locus(l) => Some(l),
        _ => None,
    }
}

/// The `bindings` rows of one bundle, in the bundle's program order and
/// each program's declaration order (a module's contents after the
/// module), an entry after the entries before it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BindingRows {
    pub rows: Vec<BindingRow>,
}

impl BindingRows {
    /// The row of the entry minted as `site`.
    pub fn for_site(&self, site: SiteId) -> Option<&BindingRow> {
        self.rows.iter().find(|r| r.site == Some(site))
    }

    /// The bound-topic set, by the names entries write: what the `or
    /// wait` legality check and the gate walk ask.
    pub fn bound_names(&self) -> BTreeSet<String> {
        self.rows.iter().map(|r| r.topic.clone()).collect()
    }

    /// The bound-topic set at both grains the bus graph is keyed at: the
    /// name an entry writes and the wire subject the topic rewrite turns
    /// every reference into (a transport-bound subject must not slip past
    /// the devirtualization gate under either spelling).
    pub fn bound_subjects(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for r in &self.rows {
            out.insert(r.topic.clone());
            if let Some(w) = &r.wire {
                out.insert(w.clone());
            }
        }
        out
    }
}

/// The `bindings` family's producer: one walk over the bundle's
/// declarations for the ends and the entries, over the scope's topic
/// rows for each entry's wire subject.
pub fn derive_binding_rows(bundle: &Bundle<'_>, top: &TopScope) -> BindingRows {
    let key = |name: &str| {
        top.topics.named(name).map_or_else(|| name.to_string(), |t| t.wire.clone())
    };
    // The ends. By topic reference, by wire subject: what the role rule
    // reads ("only topic references count: a role the desugar cannot
    // infer is not inferable"); and by literal subject too for the
    // producer test, which asks what the bundle publishes on the wire.
    let mut pubs: BTreeSet<String> = BTreeSet::new();
    let mut subs: BTreeSet<String> = BTreeSet::new();
    let mut literal_pubs: BTreeSet<String> = BTreeSet::new();
    for program in bundle.programs.values() {
        for item in hale_syntax::ast::flat_decls(&program.items) {
            let TopDecl::Locus(l) = item else { continue };
            for member in &l.members {
                let LocusMember::Bus(bb) = member else { continue };
                for bm in &bb.members {
                    match bm {
                        BusMember::Publish { subject: BusSubject::Topic(id), .. } => {
                            pubs.insert(key(&id.name));
                        }
                        BusMember::Publish { subject: BusSubject::Literal { subject, .. }, .. } => {
                            literal_pubs.insert(subject.clone());
                        }
                        BusMember::Subscribe { subject: BusSubject::Topic(id), .. } => {
                            subs.insert(key(&id.name));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn walk(
        items: &[TopDecl],
        program: &str,
        path: &mut Vec<usize>,
        f: &mut impl FnMut(&LocusDecl, &str, &[usize]),
    ) {
        for (i, item) in items.iter().enumerate() {
            path.push(i);
            match item {
                TopDecl::Locus(l) => f(l, program, path),
                TopDecl::Module(m) => walk(&m.items, program, path, f),
                _ => {}
            }
            path.pop();
        }
    }

    let mut rows = Vec::new();
    for (name, program) in &bundle.programs {
        walk(&program.items, name, &mut Vec::new(), &mut |l, program, path| {
            for (mi, member) in l.members.iter().enumerate() {
                let LocusMember::Bindings(bb) = member else { continue };
                for (ei, e) in bb.entries.iter().enumerate() {
                    let wire = top.topics.named(&e.topic.name).map(|t| t.wire.clone());
                    let k = wire.as_deref().unwrap_or(&e.topic.name);
                    let (publishes, subscribes) = (pubs.contains(k), subs.contains(k));
                    let (transport, explicit, adapter) = match &e.transport {
                        TransportSpec::Unix { role, .. } => (Transport::Unix, *role, None),
                        TransportSpec::Adapter { locus, .. } => {
                            (Transport::Adapter, None, Some(locus.name.clone()))
                        }
                        TransportSpec::ShmRing { .. } => (Transport::ShmRing, None, None),
                    };
                    // The one role rule: the entry's own role, else the
                    // ends decide. Only a substrate unix entry has one.
                    let role = match transport {
                        Transport::Unix => explicit
                            .or_else(|| hale_syntax::desugar::role_from_ends(publishes, subscribes)),
                        _ => None,
                    };
                    let loss_locus = match (transport, role) {
                        (Transport::Unix, Some(TransportRole::Listen)) => Some(UNIX_LISTEN_LOCUS),
                        (Transport::Unix, Some(TransportRole::Connect)) => Some(UNIX_CONNECT_LOCUS),
                        _ => None,
                    };
                    rows.push(BindingRow {
                        site: bundle.snapshot.site_id(e.id),
                        span: e.span,
                        locus: l.name.name.clone(),
                        is_main: l.is_main,
                        imported: l.imported,
                        module_nested: path.len() > 1,
                        topic: e.topic.name.clone(),
                        topic_span: e.topic.span,
                        wire: wire.clone(),
                        transport,
                        role,
                        publishes,
                        subscribes,
                        adapter,
                        codec: e.codec.as_ref().map(|c| c.locus.name.clone()),
                        producer: publishes || literal_pubs.contains(k),
                        loss_locus,
                        at: (program.to_string(), path.to_vec(), mi, ei),
                    });
                }
            }
        });
    }
    BindingRows { rows }
}
