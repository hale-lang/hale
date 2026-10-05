//! The role rows (F.40 phase 4, A4): what the program says about who
//! may reach its served surface — every `role` declaration, every
//! `@gated(role:)` site and the api entry's role source — over the
//! programs after the desugar sequence, with or without an `api:`
//! entry. Rows of the `api_surface` family: the snapshot demands them
//! once (`Snapshot::demand_role_rows`) and hands them to the check
//! (`CheckInputs::roles`), and the roles an environment maps are their
//! projection ([`RoleRows::declared_roles`]).
//!
//! What another family owns is not copied here: which handler
//! subscribes to which topic and which sites publish it are the bus
//! graph's, the bound topics the binding rows', a topic's wire key the
//! topic rows'.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{
    BusMember, ContractDirection, ContractKind, ContractName, Expr, Ident, LocusMember, NodeId, PerspectiveMember,
    PrimType, QualifiedName, TopDecl, TypeExpr,
};
use hale_syntax::{Diag, Span};

use crate::binding_rows::BindingRows;
use crate::bus_graph::BusGraph;
use crate::entry::EntryRow;
use crate::law::{RuleId, Violation};
use crate::topic_identity::TopicRows;
use crate::Bundle;

/// A role is declared once.
const DECLARED_ONCE: RuleId = RuleId::registered("verification/structural", "role-declared-once");
/// Every role a site names is declared, or is `owner`.
const DECLARED: RuleId = RuleId::registered("verification/structural", "role-declared");
/// `includes` is acyclic.
const ACYCLIC: RuleId = RuleId::registered("verification/structural", "role-includes-acyclic");
/// No gate on a free fn.
const FREE_FN: RuleId = RuleId::registered("verification/structural", "gate-on-a-free-fn");
/// A gated locus fn is a subscribed handler.
const PLAIN_METHOD: RuleId = RuleId::registered("verification/structural", "gate-on-a-plain-method");
/// A gated handler's topic is not bound to a transport.
const BOUND_TOPIC: RuleId = RuleId::registered("verification/structural", "gate-on-a-bound-topic");
/// Every subscriber (or publisher) of one topic states the same gate.
const GATES_AGREE: RuleId = RuleId::registered("verification/structural", "gates-agree-per-topic");
/// The role source names a locus of the bundle.
const SOURCE_LOCUS: RuleId = RuleId::registered("verification/structural", "role-source-is-a-locus");
/// The role source's locus has a `fn holds`.
const SOURCE_HOLDS: RuleId = RuleId::registered("verification/structural", "role-source-has-holds");
/// The role source's `holds` is `std::api::RoleSource`'s.
const SOURCE_SIGNATURE: RuleId = RuleId::registered("verification/structural", "role-source-signature");

/// One `role` declaration, as written. Two declarations of one name are
/// two rows: the vocabulary's own rule reads them.
#[derive(Debug, Clone, PartialEq)]
pub struct RoleDeclRow {
    /// The role's name, with its span.
    pub name: Ident,
    /// The roles it `includes`, each with its span, in the order written.
    pub includes: Vec<Ident>,
    /// The declaration's span.
    pub span: Span,
    /// Its site: the bundle program that holds it, and its item index at
    /// each depth (a module's contents under the module's index).
    pub program: String,
    pub path: Vec<usize>,
}

/// Where a `@gated(role:)` sits: every place the syntax takes one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GateKind {
    /// A top-level `fn`.
    FreeFn,
    /// A locus's `fn`.
    Method,
    /// A perspective's `fn`.
    PerspectiveMethod,
    /// A `contract` member, with its direction (the parser takes a gate
    /// on an `expose` only).
    Contract(ContractDirection),
    /// A `publish` member of a `bus { }` block, by its identity: the bus
    /// graph's `PublishRow::id`.
    Publish(NodeId),
}

/// The locus declaration a gate sits in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateLocus {
    pub name: String,
    /// Its ordinal among the bundle's locus declarations in walk order:
    /// the index of its row in the bus graph's `decls`.
    pub decl: usize,
    /// An imported seed's locus.
    pub imported: bool,
}

/// One `@gated(role:)` site.
#[derive(Debug, Clone, PartialEq)]
pub struct GateRow {
    pub kind: GateKind,
    /// The declaration it sits on: the free fn, or the locus or
    /// perspective that holds the member.
    pub decl: String,
    /// The locus, for a gate on a locus's member.
    pub locus: Option<GateLocus>,
    /// The member's name: the fn's, the contract member's (empty for an
    /// inferred one), or the subject a `publish` names
    /// (`BusSubject::canonical`).
    pub member: String,
    /// The role it names, with its span.
    pub role: Ident,
    /// The gated declaration's span: the fn's, the contract member's or
    /// the `publish` member's.
    pub span: Span,
}

/// A `fn holds` as written.
#[derive(Debug, Clone, PartialEq)]
pub struct HoldsFn {
    pub name: Ident,
    /// Its parameters' types as written.
    pub params: Vec<TypeExpr>,
    pub ret: Option<TypeExpr>,
    pub fallible: bool,
}

/// The locus a role source names, as the bundle declares it.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceLocus {
    /// Its first `fn holds`, if it declares one.
    pub holds: Option<HoldsFn>,
}

/// The api entry's `roles:` clause: the membership source a gate asks.
#[derive(Debug, Clone, PartialEq)]
pub struct RoleSource {
    /// The `api:` entry it is named in: the bundle's last.
    pub entry: Span,
    /// The clause's span.
    pub span: Span,
    /// The locus it names, as written: a locus literal's path, or the
    /// declared type of the `self.<param>` it reads (a param of a locus
    /// that carries an `api:` entry). `None` for any other expression,
    /// which is typed against `std::api::RoleSource` at the generated
    /// init.
    pub names: Option<String>,
    /// The bundle's locus of that name — the last declared, a generated
    /// `__Api` locus aside — when there is one.
    pub locus: Option<SourceLocus>,
}

/// The role rows of a bundle.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RoleRows {
    /// Every `role` declaration, in walk order.
    pub roles: Vec<RoleDeclRow>,
    /// Every `@gated(role:)` site, in walk order: a declaration's members
    /// in the order written, a `bus { }` block's `publish` members among
    /// them.
    pub gates: Vec<GateRow>,
    /// The last `roles:` clause of an `api:` entry, when one names a
    /// source.
    pub source: Option<RoleSource>,
    /// The entry row's root carries an `api:` entry: the binding is
    /// generated, and `owner` gates its description.
    pub served: bool,
}

impl RoleRows {
    /// The roles an environment maps: the declared names, sorted and
    /// de-duplicated, with `owner` when the program is served or
    /// declares any role, because that is when something is gated on it.
    pub fn declared_roles(&self) -> Vec<String> {
        self.vocabulary().into_iter().map(|(name, _)| name).collect()
    }

    /// The vocabulary the binding is generated with: each declared role
    /// once, by name, with the `includes` of its first declaration, and
    /// `owner` as [`RoleRows::declared_roles`] adds it, with no
    /// `includes` unless a declaration gives it some.
    pub fn vocabulary(&self) -> Vec<(String, Vec<String>)> {
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for r in &self.roles {
            out.entry(r.name.name.clone())
                .or_insert_with(|| r.includes.iter().map(|i| i.name.clone()).collect());
        }
        if self.served || !out.is_empty() {
            out.entry("owner".to_string()).or_default();
        }
        out.into_iter().collect()
    }
}

/// The role rows' producer: one walk over the bundle's declarations, in
/// the bundle's program order. `entry` is the bundle's entry row, whose
/// root is the `main locus` the api binding is generated into.
pub fn role_rows(bundle: &Bundle<'_>, entry: &EntryRow) -> RoleRows {
    struct Walk<'a> {
        rows: RoleRows,
        program: String,
        path: Vec<usize>,
        /// Locus declarations seen so far: the next one's ordinal.
        loci: usize,
        /// The source's inputs: the last `api:` entry's span, the last
        /// `roles:` clause, the params of every locus that carries an
        /// entry, and the loci a source may name.
        api_entry: Option<Span>,
        clause: Option<&'a hale_syntax::ast::ApiRoles>,
        params: Vec<(&'a str, &'a TypeExpr)>,
        by_name: BTreeMap<&'a str, &'a hale_syntax::ast::LocusDecl>,
    }

    fn gate(kind: GateKind, decl: &str, locus: Option<GateLocus>, member: String, role: &Ident, span: Span) -> GateRow {
        GateRow { kind, decl: decl.to_string(), locus, member, role: role.clone(), span }
    }

    fn walk<'a>(items: &'a [TopDecl], w: &mut Walk<'a>) {
        for (i, item) in items.iter().enumerate() {
            w.path.push(i);
            match item {
                TopDecl::Role(r) => w.rows.roles.push(RoleDeclRow {
                    name: r.name.clone(),
                    includes: r.includes.clone(),
                    span: r.span,
                    program: w.program.clone(),
                    path: w.path.clone(),
                }),
                TopDecl::Fn(f) => {
                    if let Some(g) = &f.gated {
                        let row = gate(GateKind::FreeFn, &f.name.name, None, f.name.name.clone(), g, f.span);
                        w.rows.gates.push(row);
                    }
                }
                TopDecl::Perspective(p) => {
                    for m in &p.members {
                        if let PerspectiveMember::Fn(f) = m {
                            if let Some(g) = &f.gated {
                                let kind = GateKind::PerspectiveMethod;
                                w.rows.gates.push(gate(kind, &p.name.name, None, f.name.name.clone(), g, f.span));
                            }
                        }
                    }
                }
                TopDecl::Locus(l) => {
                    let at = GateLocus { name: l.name.name.clone(), decl: w.loci, imported: l.imported };
                    w.loci += 1;
                    if !l.name.name.starts_with("__Api") {
                        w.by_name.insert(&l.name.name, l);
                    }
                    for m in &l.members {
                        match m {
                            LocusMember::Fn(f) => {
                                if let Some(g) = &f.gated {
                                    let (kind, member) = (GateKind::Method, f.name.name.clone());
                                    w.rows.gates.push(gate(kind, &l.name.name, Some(at.clone()), member, g, f.span));
                                }
                            }
                            LocusMember::Contract(cb) => {
                                let ContractKind::Members(members) = &cb.kind else { continue };
                                for cm in members {
                                    let Some(g) = &cm.gated else { continue };
                                    let member = match &cm.name {
                                        ContractName::Named(n) => n.name.clone(),
                                        ContractName::Inferred => String::new(),
                                    };
                                    let kind = GateKind::Contract(cm.direction);
                                    w.rows.gates.push(gate(kind, &l.name.name, Some(at.clone()), member, g, cm.span));
                                }
                            }
                            LocusMember::Bus(bb) => {
                                for bm in &bb.members {
                                    let BusMember::Publish { subject, gated: Some(g), span, id, .. } = bm else {
                                        continue;
                                    };
                                    let (kind, member) = (GateKind::Publish(*id), subject.canonical().to_string());
                                    w.rows.gates.push(gate(kind, &l.name.name, Some(at.clone()), member, g, *span));
                                }
                            }
                            LocusMember::Bindings(bb) => {
                                let Some(api) = &bb.api else { continue };
                                w.api_entry = Some(api.span);
                                if let Some(r) = &api.roles {
                                    w.clause = Some(r);
                                }
                                for pm in &l.members {
                                    if let LocusMember::Params(pb) = pm {
                                        for prm in &pb.params {
                                            if let Some(t) = &prm.ty {
                                                w.params.push((&prm.name.name, t));
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                TopDecl::Module(m) => walk(&m.items, w),
                _ => {}
            }
            w.path.pop();
        }
    }

    let mut w = Walk {
        rows: RoleRows::default(),
        program: String::new(),
        path: Vec::new(),
        loci: 0,
        api_entry: None,
        clause: None,
        params: Vec::new(),
        by_name: BTreeMap::new(),
    };
    for (name, p) in &bundle.programs {
        w.program = name.clone();
        walk(&p.items, &mut w);
    }

    if let Some(clause) = w.clause {
        let named = |path: &QualifiedName| -> String {
            path.segments.iter().map(|s| s.name.clone()).collect::<Vec<_>>().join("::")
        };
        let names = match &clause.expr {
            Expr::Struct { path, .. } => Some(named(path)),
            Expr::Field { receiver, name, .. } if matches!(**receiver, Expr::KwSelf(_)) => {
                w.params.iter().find(|(n, _)| *n == name.name).and_then(|(_, t)| match t {
                    TypeExpr::Named { path, .. } => Some(named(path)),
                    _ => None,
                })
            }
            _ => None,
        };
        let locus = names.as_deref().and_then(|n| w.by_name.get(n)).map(|l| SourceLocus {
            holds: l.members.iter().find_map(|m| match m {
                LocusMember::Fn(f) if f.name.name == "holds" => Some(HoldsFn {
                    name: f.name.clone(),
                    params: f.params.iter().map(|p| p.ty.clone()).collect(),
                    ret: f.ret.clone(),
                    fallible: f.fallible.is_some(),
                }),
                _ => None,
            }),
        });
        w.rows.source = Some(RoleSource { entry: w.api_entry.unwrap_or(clause.span), span: clause.span, names, locus });
    }
    w.rows.served = entry.root().and_then(|m| m.decl(bundle)).is_some_and(|l| {
        l.members.iter().any(|m| matches!(m, LocusMember::Bindings(bb) if bb.api.is_some()))
    });
    w.rows
}

// ---- the law ---------------------------------------------------------

/// GH #1109: the role rules, a law over the role rows and the rows it
/// joins them to: the bus graph (which handler of which declaration
/// subscribes to which topic, and which sites publish it), the binding
/// rows (which topics are bound to a transport) and the topic rows (a
/// topic's wire key, which a reference joins on, spec/model.md rule 8).
///
/// Roles are declared vocabulary like `group` and `effect`: a name
/// nothing declares is an error, bundle-wide, with `owner` the one role
/// that needs no declaration (a program declares it only to give it
/// `includes`). `includes` is grant-only and union-only, so a cycle says
/// nothing and is refused. A gate goes on a subscribed handler, an
/// `expose` member or a `publish`, and every subscriber (or publisher)
/// of one topic states the same gate, because the binding refuses the
/// message, not the handler. A gated handler's topic cannot also be
/// bound to a transport in `bindings { }`: that transport has no gate,
/// so the annotation would promise a check that does not run. A gate in
/// a generated `__Api` locus or an imported one is not judged: the
/// binding does not reach it as the entrypoint's.
///
/// The ten rules are the ten functions below, one message each, each a
/// registered rule whose finding is a [`Violation`] (F.40 phase 4, W5);
/// this walks the rows in the order the diagnostics are reported: the
/// vocabulary, the free fns, each locus declaration's gates, the topics'
/// agreement, then the role source. One walk judges the ten, so the
/// findings keep the order the walk reaches them in.
pub fn role_laws(rows: &RoleRows, bus: &BusGraph, topics: &TopicRows, bindings: &BindingRows) -> Vec<Diag> {
    let mut found = Vec::new();
    role_walk(rows, bus, topics, bindings, &mut found);
    crate::law::diags(found)
}

/// The walk [`role_laws`] reports, in its order.
fn role_walk(rows: &RoleRows, bus: &BusGraph, topics: &TopicRows, bindings: &BindingRows, found: &mut Vec<Violation>) {

    // The vocabulary: the first declaration of a name is the role.
    let mut decls: BTreeMap<&str, &RoleDeclRow> = BTreeMap::new();
    for r in &rows.roles {
        match decls.get(r.name.name.as_str()) {
            Some(first) => found.push(declared_twice(r, first)),
            None => {
                decls.insert(&r.name.name, r);
            }
        }
    }
    let declared = |n: &str| n == "owner" || decls.contains_key(n);
    for (name, r) in &decls {
        for inc in &r.includes {
            if !declared(&inc.name) {
                found.push(undeclared(inc, &format!("`role {} includes …`", name)));
            }
        }
        if includes_itself(name, &decls) {
            found.push(role_cycle(name, r.name.span));
        }
    }

    // Review F3: a gate on a free fn is an error — nothing there is
    // reached from the binding — and its role is checked all the same.
    for g in rows.gates.iter().filter(|g| g.kind == GateKind::FreeFn) {
        found.push(gate_on_a_free_fn(g));
        if !declared(&g.role.name) {
            found.push(undeclared(&g.role, &format!("`@gated` on `{}`", g.member)));
        }
    }

    // A topic reference joins on its row's wire subject (spec/model.md
    // rule 8), and is named as written.
    let topic_key = |name: &str| -> String { topics.named(name).map_or_else(|| name.to_string(), |t| t.wire.clone()) };
    let bound: BTreeSet<String> = bindings.bound_names().iter().map(|n| topic_key(n)).collect();
    // The sites, and the gate each topic's subscribers and publishers
    // state: wire subject → (the topic as written, [(site, role, span)]).
    let mut sub_gates: Gates = BTreeMap::new();
    let mut pub_gates: Gates = BTreeMap::new();
    for (d, decl) in bus.decls.iter().enumerate() {
        // An imported locus is not reached from the binding, so its
        // gates (or their absence) say nothing about the entrypoint's.
        if decl.name.starts_with("__Api") || decl.imported {
            continue;
        }
        let locus = decl.name.as_str();
        let gates_here = || rows.gates.iter().filter(move |g| g.locus.as_ref().is_some_and(|l| l.decl == d));
        // Every subscription, by handler: one handler may subscribe
        // several topics (review F5), and each is a site.
        // (handler, (wire key, topic as written), where the handler is named).
        let subscribed: Vec<(&str, Option<(String, &str)>, Span)> = bus
            .rows
            .subscribes
            .iter()
            .filter(|s| s.decl == d)
            .map(|s| (s.handler.as_str(), s.topic.as_deref().map(|t| (topic_key(t), t)), s.handler_span))
            .collect();
        for p in bus.rows.publishes.iter().filter(|p| p.decl == d) {
            let gate = gates_here().find(|g| g.kind == GateKind::Publish(p.id) && g.span == p.span);
            if let Some(g) = gate {
                if !declared(&g.role.name) {
                    found.push(undeclared(&g.role, &format!("`@gated` on `{}`'s publish", locus)));
                }
            }
            if let Some(t) = &p.topic {
                let entry = pub_gates.entry(topic_key(t)).or_insert_with(|| (t.clone(), Vec::new()));
                entry.1.push((format!("{} publishes it", locus), gate.map(|g| g.role.name.clone()), p.span));
            }
        }
        for g in gates_here() {
            match g.kind {
                GateKind::Method => {
                    if !declared(&g.role.name) {
                        found.push(undeclared(&g.role, &format!("`@gated` on `{}.{}`", locus, g.member)));
                    }
                    let mine: Vec<&Option<(String, &str)>> =
                        subscribed.iter().filter(|(h, _, _)| *h == g.member).map(|(_, t, _)| t).collect();
                    if mine.is_empty() {
                        found.push(gate_on_a_plain_method(g, locus));
                    }
                    for (key, topic) in mine.into_iter().flatten() {
                        if bound.contains(key) {
                            found.push(gate_on_a_bound_topic(g, locus, topic));
                        }
                        let entry = sub_gates.entry(key.clone()).or_insert_with(|| (topic.to_string(), Vec::new()));
                        entry.1.push((format!("{}.{}", locus, g.member), Some(g.role.name.clone()), g.role.span));
                    }
                }
                GateKind::Contract(ContractDirection::Expose) => {
                    if !declared(&g.role.name) {
                        found.push(undeclared(&g.role, &format!("`@gated` on an `expose` of `{}`", locus)));
                    }
                }
                _ => {}
            }
        }
        // Ungated subscribers count too: every subscriber of a topic
        // must agree.
        for (h, topic, span) in &subscribed {
            let Some((key, t)) = topic else { continue };
            let gated = gates_here().any(|g| g.kind == GateKind::Method && g.member == *h);
            if !gated {
                let entry = sub_gates.entry(key.clone()).or_insert_with(|| (t.to_string(), Vec::new()));
                entry.1.push((format!("{}.{}", locus, h), None, *span));
            }
        }
    }
    for (kind, gates) in [("subscribes", &sub_gates), ("publishes", &pub_gates)] {
        // Reported in the order of the topics' written names.
        let mut groups: Vec<&(String, Vec<GateSite>)> = gates.values().collect();
        groups.sort_by(|a, b| a.0.cmp(&b.0));
        for (topic, sites) in groups {
            let distinct: BTreeSet<&Option<String>> = sites.iter().map(|(_, r, _)| r).collect();
            if distinct.len() >= 2 {
                found.push(gates_disagree(topic, sites, kind));
            }
        }
    }

    // A program-named source is a locus satisfying std::api::RoleSource
    // (review F6): a locus literal or `self.<param>` of the main locus
    // is checked structurally here, with the fn's span; any other
    // expression is typed against the interface at the generated init.
    let Some(src) = &rows.source else { return };
    let Some(locus_name) = src.names.as_deref() else { return };
    if locus_name.starts_with("__Std") || locus_name.starts_with("std::") {
        return;
    }
    // A qualified path (`lib::TableRoles`) is renamed to the imported
    // locus's mangled name only on the build path; here the generated
    // init is typed against the interface, which is check enough.
    if locus_name.contains("::") && src.locus.is_none() {
        return;
    }
    let Some(l) = &src.locus else {
        found.push(source_is_no_locus(src, locus_name));
        return;
    };
    let Some(f) = &l.holds else {
        found.push(source_has_no_holds(src, locus_name));
        return;
    };
    if let Some(d) = holds_is_not_a_role_source(src, locus_name, f) {
        found.push(d);
    }
}

/// A site that states (or does not state) a gate on a topic: the site as
/// a diagnostic names it, the role, and where it is.
type GateSite = (String, Option<String>, Span);
type Gates = BTreeMap<String, (String, Vec<GateSite>)>;

/// `name` is reachable from its own `includes`.
fn includes_itself(name: &str, decls: &BTreeMap<&str, &RoleDeclRow>) -> bool {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut stack: Vec<&str> = decls[name].includes.iter().map(|i| i.name.as_str()).collect();
    while let Some(cur) = stack.pop() {
        if cur == name {
            return true;
        }
        if !seen.insert(cur) {
            continue;
        }
        if let Some(more) = decls.get(cur) {
            stack.extend(more.includes.iter().map(|i| i.name.as_str()));
        }
    }
    false
}

/// Rule: a role is declared once.
fn declared_twice(r: &RoleDeclRow, first: &RoleDeclRow) -> Violation {
    Violation::error(
        DECLARED_ONCE,
        r.name.span,
        format!(
            "role `{}` is declared twice; a role is one name the \
             deployment maps, so declare it once and `includes` it \
             where a wider role should hold it",
            r.name.name
        ),
    )
    .step(first.name.span, "the first declaration")
}

/// Rule: every role a site names is declared (or is `owner`). `at` says
/// which site: an `includes`, a free fn's gate, a publish's, a
/// handler's or an `expose`'s.
fn undeclared(n: &Ident, at: &str) -> Violation {
    Violation::error(
        DECLARED,
        n.span,
        format!(
            "{} names role `{}`, which nothing declares — roles are declared \
             vocabulary: `role {};` at top level (only `owner` needs no declaration)",
            at, n.name, n.name
        ),
    )
}

/// Rule: `includes` is acyclic.
fn role_cycle(name: &str, span: Span) -> Violation {
    Violation::error(
        ACYCLIC,
        span,
        format!(
            "role `{}` includes itself through its `includes` chain; \
             composition is grant-only and union-only, so a cycle says nothing",
            name
        ),
    )
}

/// Rule: no gate on a free fn.
fn gate_on_a_free_fn(g: &GateRow) -> Violation {
    Violation::error(
        FREE_FN,
        g.role.span,
        format!(
            "`@gated(role: {})` on the free fn `{}`: a gate goes on a subscribed \
             handler, an `expose` member or a `publish` — it is checked at the api \
             binding, and a free fn is never reached from there",
            g.role.name, g.member
        ),
    )
}

/// Rule: a gated locus fn is a subscribed handler.
fn gate_on_a_plain_method(g: &GateRow, locus: &str) -> Violation {
    Violation::error(
        PLAIN_METHOD,
        g.role.span,
        format!(
            "`@gated(role: {})` on `{}.{}`, which no `subscribe` line of \
             `{}` names: a gate goes on a subscribed handler, an `expose` \
             member or a `publish` — it is checked at the binding, and a \
             plain method is never reached from there",
            g.role.name, locus, g.member, locus
        ),
    )
}

/// Rule: a gated handler's topic is not bound to a transport.
fn gate_on_a_bound_topic(g: &GateRow, locus: &str, topic: &str) -> Violation {
    Violation::error(
        BOUND_TOPIC,
        g.role.span,
        format!(
            "`@gated(role: {})` on `{}.{}`, but its topic `{}` is \
             bound to a transport in `bindings {{ }}` that has no \
             gate: a message from another process would reach the \
             handler unchecked. Reach the topic through the api \
             binding, or drop the annotation",
            g.role.name, locus, g.member, topic
        ),
    )
}

/// Rule: every subscriber (or publisher) of one topic states the same
/// gate. Named at the first gated site.
fn gates_disagree(topic: &str, sites: &[GateSite], kind: &str) -> Violation {
    let listed: Vec<String> = sites
        .iter()
        .map(|(site, r, _)| match r {
            Some(r) => format!("{} gated `{}`", site, r),
            None => format!("{} ungated", site),
        })
        .collect();
    let (_, _, span) = sites.iter().find(|(_, r, _)| r.is_some()).unwrap_or(&sites[0]);
    Violation::error(
        GATES_AGREE,
        *span,
        format!(
            "topic `{}`: {} — every locus that {} one topic states the same gate, \
             because the binding refuses the message, not the handler",
            topic,
            listed.join(", "),
            kind
        ),
    )
}

/// Rule: the role source names a locus of the bundle.
fn source_is_no_locus(src: &RoleSource, locus_name: &str) -> Violation {
    Violation::error(
        SOURCE_LOCUS,
        src.span,
        format!(
            "api binding: `roles:` names `{}`, which is no locus of this bundle; a role \
             source is a locus with `fn holds(p: std::api::Principal, r: String) -> Bool` \
             (std::api::RoleSource)",
            locus_name
        ),
    )
}

/// Rule: the role source's locus has a `fn holds`.
fn source_has_no_holds(src: &RoleSource, locus_name: &str) -> Violation {
    Violation::error(
        SOURCE_HOLDS,
        src.span,
        format!(
            "api binding: `roles:` names `{}`, which has no `fn holds`: a role source \
             answers `fn holds(p: std::api::Principal, r: String) -> Bool` \
             (std::api::RoleSource) — whether the principal holds the role directly; \
             the binding walks `includes` itself",
            locus_name
        ),
    )
}

/// Rule: the role source's `holds` is `std::api::RoleSource`'s, as
/// written: two parameters, a `std::api::Principal` and a `String`,
/// returning `Bool`, not fallible. Every way it is not is listed.
fn holds_is_not_a_role_source(src: &RoleSource, locus_name: &str, f: &HoldsFn) -> Option<Violation> {
    let is_principal = |t: &TypeExpr| match t {
        TypeExpr::Named { path, generic_args, .. } if generic_args.is_empty() => {
            let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
            segs == ["std", "api", "Principal"] || segs == ["__StdApiPrincipal"]
        }
        _ => false,
    };
    let mut why: Vec<String> = Vec::new();
    if f.params.len() != 2 {
        why.push(format!("takes {} parameter(s), not 2", f.params.len()));
    } else {
        if !is_principal(&f.params[0]) {
            why.push("its first parameter is not a `std::api::Principal`".to_string());
        }
        if !matches!(&f.params[1], TypeExpr::Primitive(PrimType::String, _)) {
            why.push("its second parameter is not a `String`".to_string());
        }
    }
    if !matches!(&f.ret, Some(TypeExpr::Primitive(PrimType::Bool, _))) {
        why.push("it does not return `Bool`".to_string());
    }
    if f.fallible {
        why.push("it is fallible".to_string());
    }
    if why.is_empty() {
        return None;
    }
    Some(
        Violation::error(
            SOURCE_SIGNATURE,
            f.name.span,
            format!(
                "`{}.holds` does not satisfy std::api::RoleSource: {} — a role source \
                 answers `fn holds(p: std::api::Principal, r: String) -> Bool`, not \
                 fallible, whether the principal holds the role directly (the binding \
                 walks `includes` itself)",
                locus_name,
                why.join("; ")
            ),
        )
        .step(src.entry, "the api entry that names it"),
    )
}
