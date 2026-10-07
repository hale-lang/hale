//! A declaration's dependents (F.40 phase 3, X2): which top-level
//! declarations of a snapshot may check differently when one changes,
//! found through the families' rows and never through the text.
//!
//! The unit is a top-level declaration of a snapshot program
//! ([`Declaration`]): a `module { }` is one, its members inside it. A
//! declaration is named by its own minted site; one the mint gives no
//! site (a `target`, a `role`, a top-level `claims` block) has none.
//!
//! The relation is the union of the families' edges, each joined to the
//! declarations it names:
//!
//! - **The callgraph** (the effect rows' `targets` column, read where
//!   the rows read it, [`AllocSummary::fns`]' resolved call edges, so the
//!   relation demands no effects fixpoint): a declaration whose body
//!   calls a fn or a method of another reads it, and the readers close
//!   transitively — the reveal rule's transparency is a fixpoint over the
//!   bodies a call reaches. A body the reveal rule reads that is no row
//!   (an `on_failure` handler, a params initializer, a constant's value,
//!   a callee that is no name…) is summarized beside the rows
//!   ([`AllocSummary::declaration_bodies`]), keyed to its declaration's
//!   site, and its calls are that declaration's. A call to a fn the
//!   summary has no row for is unresolved, and its bare name joins the
//!   fns of that name. A call the summary records no edge for at all
//!   (one in a position no summary body covers: a closure's clauses, a
//!   type's field defaults) makes the declaration holding it a dependent
//!   of every declaration.
//! - **The ownership graph**: a locus and the loci it instantiates,
//!   accepts or is instantiated by.
//! - **The bus graph**: every publisher and subscriber of one subject,
//!   with the topic that declares it.
//! - **The placement table**: a declaration an instance or a dynamic
//!   site realizes, with the declaration its literal sits in, its
//!   owner's and its enclosing scope's.
//! - **The flow rows**: a flow child and every locus whose `release`
//!   clause names it.
//!
//! These are one hop, the callgraph's readers closed: the answer for a
//! declaration is itself, every declaration whose calls reach it, and
//! its neighbours in the other four. The callgraph, the ownership and
//! bus graphs and the flow rows name a declaration by its declared name;
//! the join answers every declaration that declares the name, so a name
//! two declarations share answers both (a superset, never a miss). The
//! placement table names sites, joined exactly.
//!
//! The relation answers for an edit to a declaration's bodies: what its
//! fns, methods and hooks do, which is what the families record. An edit
//! to what a declaration declares (a signature, a field, a member) is
//! read through the scope, which no family records — a field read on a
//! parameter of the locus's type is no call and builds nothing — so such
//! an edit is the seed's, checked whole by the reuse rule
//! (`crates/hale-types/tests/declaration_dependents.rs` holds both: the
//! relation covering every body edit of a corpus mutation, and a reader
//! of a declared surface no family names).
//!
//! The families place fns and loci: a callgraph row is a fn or a method,
//! the other families' rows are loci. Every other declaration (a type, a
//! topic, a const, an interface, a module…) and one with no site is
//! [`Dependents::Whole`]: no family row says who reads it, so a change to
//! it is a change to the seed. A placement hole that leaves its literal's
//! declaration unresolved makes the declaration it sits in a dependent of
//! every declaration, since the table cannot say which one it realizes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use hale_graph::ids::SiteId;
use hale_syntax::ast::{Program, TopDecl};
use hale_syntax::sites::{for_each_site_in_item, SiteKind};
use hale_syntax::Span;
use hale_types::alloc_summary::{AllocSummary, CallEdge, Callee, FnKey};
use hale_types::bus_graph::BusGraph;
use hale_types::flows::FlowRows;
use hale_types::ownership_graph::OwnershipGraph;
use hale_types::placement::{Enclosing, HoleAt, HoleKind, PlacementTable, SiteRef, SiteUniverse};

/// One top-level declaration of a snapshot program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// The declaration's own site; `None` for one the mint gives none.
    pub site: Option<SiteId>,
    /// The snapshot program that holds it, and its position among the
    /// program's items.
    pub program: PathBuf,
    pub index: usize,
    /// What it declares (`fn`, `locus`, `type`, …) and its name as
    /// declared (empty for a top-level `claims` block).
    pub kind: &'static str,
    pub name: String,
    pub span: Span,
}

/// What a change to one declaration may change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dependents {
    /// The declarations to check again: the changed one and every
    /// declaration the families say reads it, by their sites.
    Decls(BTreeSet<SiteId>),
    /// No family places the declaration: the seed is checked whole.
    Whole(&'static str),
}

/// The declarations of `programs`, in program then item order, each with
/// the site `identities` minted for it.
pub(crate) fn declarations(
    programs: &BTreeMap<PathBuf, Program>,
    identities: &hale_types::snapshot::Snapshot,
) -> Vec<Declaration> {
    let mut out = Vec::new();
    for (path, prog) in programs {
        for (index, item) in prog.items.iter().enumerate() {
            let (kind, name) = kind_and_name(item);
            let mut site = None;
            for_each_site_in_item(item, &mut |_, _, id| {
                if site.is_none() && !id.is_none() {
                    site = Some(id.0);
                }
            });
            out.push(Declaration {
                site: site.and_then(|i| {
                    let at = identities.sites.binary_search_by_key(&i, |s| s.id.index).ok()?;
                    Some(identities.sites[at].id)
                }),
                program: path.clone(),
                index,
                kind,
                name,
                span: item.span(),
            });
        }
    }
    out
}

/// A declaration's kind and its declared name.
pub fn kind_and_name(item: &TopDecl) -> (&'static str, String) {
    match item {
        TopDecl::Locus(l) => ("locus", l.name.name.clone()),
        TopDecl::Perspective(p) => ("perspective", p.name.name.clone()),
        TopDecl::Type(t) => ("type", t.name.name.clone()),
        TopDecl::Const(c) => ("const", c.name.name.clone()),
        TopDecl::Fn(f) => ("fn", f.name.name.clone()),
        TopDecl::Module(m) => ("module", m.name.name.clone()),
        TopDecl::Interface(i) => ("interface", i.name.name.clone()),
        TopDecl::Topic(t) => ("topic", t.name.name.clone()),
        TopDecl::RingLayout(r) => ("ring_layout", r.name.name.clone()),
        TopDecl::Target(t) => ("target", t.name.name.clone()),
        TopDecl::Group(g) => ("group", g.name.name.clone()),
        TopDecl::Role(r) => ("role", r.name.name.clone()),
        TopDecl::Claims(_) => ("claims", String::new()),
        TopDecl::Constitution(c) => ("constitution", c.name.name.clone()),
        TopDecl::Unit(u) => ("unit", u.name.name.clone()),
        TopDecl::Api(a) => ("api", a.name.name.clone()),
    }
}

/// The families' rows, joined to declarations: per declaration (by its
/// index in [`declarations`]), the declarations whose calls reach it and
/// its neighbours in the structural families.
pub(crate) struct DependencyIndex {
    callers: Vec<BTreeSet<usize>>,
    neighbours: Vec<BTreeSet<usize>>,
    /// Declarations holding a placement hole that leaves the declaration
    /// its literal realizes unresolved, or a call the summary records no
    /// edge for: a dependent of every declaration.
    always: BTreeSet<usize>,
    /// Site index → the declaration it sits in.
    owner_of: BTreeMap<u32, usize>,
}

/// What the index reads: the snapshot's families.
pub(crate) struct Families<'a> {
    pub programs: &'a BTreeMap<PathBuf, Program>,
    pub decls: &'a [Declaration],
    pub summary: &'a AllocSummary,
    pub ownership: &'a OwnershipGraph,
    pub bus: &'a BusGraph,
    pub placement: &'a PlacementTable,
    pub flows: &'a FlowRows,
}

impl DependencyIndex {
    pub(crate) fn build(f: &Families<'_>) -> DependencyIndex {
        let n = f.decls.len();
        // Every call of the program's own the summary records an edge
        // for, by its span: a call it does not (one the walk does not
        // descend to) is read below.
        let recorded: BTreeSet<(u32, u32)> = f
            .summary
            .fns
            .values()
            .filter(|fs| f.summary.is_own(&fs.key))
            .chain(f.summary.declaration_bodies.iter().map(|b| &b.summary))
            .flat_map(|fs| fs.calls.iter().map(|e| (e.span.start.0, e.span.end.0)))
            .collect();
        // Each declaration's own items, modules flattened: what a name
        // in a family row joins to.
        let mut loci: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        let mut fns: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        let mut topics: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        let mut owner_of: BTreeMap<u32, usize> = BTreeMap::new();
        let mut always = BTreeSet::new();
        for (i, d) in f.decls.iter().enumerate() {
            let item = &f.programs[&d.program].items[d.index];
            for decl in hale_syntax::ast::flat_decls(std::slice::from_ref(item)) {
                let into = match decl {
                    TopDecl::Locus(_) => &mut loci,
                    TopDecl::Fn(_) => &mut fns,
                    TopDecl::Topic(_) => &mut topics,
                    _ => continue,
                };
                into.entry(kind_and_name(decl).1).or_default().insert(i);
            }
            for_each_site_in_item(item, &mut |kind, span, id| {
                if !id.is_none() {
                    owner_of.insert(id.0, i);
                }
                // A call no edge records names a callee the relation
                // cannot join: the declaration holding it is a dependent
                // of every declaration.
                if kind == SiteKind::Call && !recorded.contains(&(span.start.0, span.end.0)) {
                    always.insert(i);
                }
            });
        }
        let named = |map: &BTreeMap<String, BTreeSet<usize>>, name: &str| -> Vec<usize> {
            map.get(name).map(|s| s.iter().copied().collect()).unwrap_or_default()
        };
        // A row is its declaration's: the program's by its site, a
        // stdlib copy's none of them; a row no mint numbered joins by
        // its name.
        let of_key = |k: &FnKey| -> Vec<usize> {
            match (k.decl, &k.locus) {
                (Some(d), _) if d.universe == SiteUniverse::User => owner_of.get(&d.index).copied().into_iter().collect(),
                (Some(_), _) => Vec::new(),
                (None, Some(l)) => named(&loci, l),
                (None, None) => named(&fns, &k.fn_name),
            }
        };
        let user_site = |s: &SiteRef| -> Option<usize> {
            (s.universe == SiteUniverse::User).then(|| owner_of.get(&s.id.index).copied()).flatten()
        };

        // What a call names: a resolved fn or method, or a fn the summary
        // could not resolve it to by the bare name it was written with.
        let callees = |edge: &CallEdge| -> Vec<usize> {
            match &edge.callee {
                Callee::Resolved(target) => of_key(target),
                Callee::Unresolved(name) if !edge.receiver_present => named(&fns, name),
                Callee::Unresolved(_) => Vec::new(),
            }
        };
        let mut callers = vec![BTreeSet::new(); n];
        for (key, fs) in &f.summary.fns {
            for caller in of_key(key) {
                for edge in &fs.calls {
                    for callee in callees(edge) {
                        callers[callee].insert(caller);
                    }
                }
            }
        }
        // The bodies that are no row, each its declaration's own.
        for body in &f.summary.declaration_bodies {
            let Some(&caller) = owner_of.get(&body.declaration.index) else { continue };
            for edge in &body.summary.calls {
                for callee in callees(edge) {
                    callers[callee].insert(caller);
                }
            }
        }

        let mut neighbours = vec![BTreeSet::new(); n];
        let mut join = |a: &[usize], b: &[usize]| {
            for &x in a {
                for &y in b {
                    if x != y {
                        neighbours[x].insert(y);
                        neighbours[y].insert(x);
                    }
                }
            }
        };
        // The ownership graph: births, accepts, instantiations.
        for site in &f.ownership.sites {
            join(&named(&loci, &site.enclosing_locus), &named(&loci, &site.child_ty));
        }
        for (parent, children) in f.ownership.accepts.iter().chain(&f.ownership.instantiated_by) {
            for child in children {
                join(&named(&loci, parent), &named(&loci, child));
            }
        }
        // The bus graph: one subject's ends, and its topic.
        for (subject, info) in &f.bus.subjects {
            let mut ends: Vec<usize> = named(&topics, subject);
            for p in &info.publishers {
                ends.extend(named(&loci, &p.locus));
            }
            for s in &info.subscribers {
                ends.extend(named(&loci, &s.locus));
            }
            join(&ends, &ends);
        }
        // The placement table: what an instance realizes, where its
        // literal sits, its owner's declaration; a dynamic site's too.
        let realized = |key: &hale_types::placement::InstanceKey| -> Vec<usize> {
            f.placement
                .instances
                .get(key)
                .and_then(|row| row.realizes.as_ref())
                .and_then(|d| user_site(&d.site))
                .into_iter()
                .collect()
        };
        for row in f.placement.instances.values() {
            let mut ends: Vec<usize> = Vec::new();
            ends.extend(row.realizes.as_ref().and_then(|d| user_site(&d.site)));
            ends.extend(row.literal.as_ref().and_then(user_site));
            if let Some(owner) = &row.owner {
                ends.extend(realized(owner));
            }
            if let Some(built) = &row.built_by {
                ends.extend(realized(built));
            }
            join(&ends, &ends);
        }
        if let Some(root) = &f.placement.root {
            let mut ends: Vec<usize> = user_site(&root.realizes.site).into_iter().collect();
            ends.extend(root.constructions.iter().filter_map(|c| user_site(&c.literal)));
            join(&ends, &ends);
        }
        for c in &f.placement.entry_literals {
            let ends: Vec<usize> = user_site(&c.literal).into_iter().collect();
            join(&ends, &ends);
        }
        for d in &f.placement.dynamic {
            let mut ends: Vec<usize> = user_site(&d.literal).into_iter().collect();
            ends.extend(d.realizes.as_ref().and_then(|r| user_site(&r.site)));
            match &d.enclosing {
                Enclosing::Locus(r) => ends.extend(user_site(&r.site)),
                Enclosing::Fn(s) => ends.extend(user_site(s)),
            }
            join(&ends, &ends);
        }
        for hole in &f.placement.holes {
            if !matches!(hole.kind, HoleKind::UnresolvedDeclaration { .. } | HoleKind::UnresolvedArguments) {
                continue;
            }
            let at = match &hole.at {
                HoleAt::Dynamic(s) | HoleAt::Entry(s) => user_site(s),
                HoleAt::Instance(key) => f.placement.instances.get(key).and_then(|r| r.literal.as_ref()).and_then(user_site),
            };
            match at {
                Some(i) => {
                    always.insert(i);
                }
                // A hole the table names no literal for sits nowhere the
                // relation can name: every declaration depends on every
                // other through it.
                None => always.extend(0..n),
            }
        }
        // The flow rows: a flow child and the loci whose `release`
        // clauses name it.
        for flow in f.flows.iter() {
            for clause in &flow.clauses {
                let child = clause.locus.as_deref().unwrap_or(&flow.child);
                join(&named(&loci, &clause.owner), &named(&loci, child));
            }
        }
        DependencyIndex { callers, neighbours, always, owner_of }
    }

    /// The declaration a site sits in.
    pub(crate) fn owner(&self, site: SiteId) -> Option<usize> {
        self.owner_of.get(&site.index).copied()
    }

    /// The dependents of declaration `i` of `decls`.
    pub(crate) fn dependents(&self, decls: &[Declaration], i: usize) -> Dependents {
        let d = &decls[i];
        if d.site.is_none() {
            return Dependents::Whole("the declaration has no site");
        }
        if !matches!(d.kind, "fn" | "locus") {
            return Dependents::Whole("no family has a row for the declaration's kind");
        }
        let mut out: BTreeSet<usize> = BTreeSet::from([i]);
        let mut work = vec![i];
        while let Some(x) = work.pop() {
            for &c in &self.callers[x] {
                if out.insert(c) {
                    work.push(c);
                }
            }
        }
        out.extend(self.neighbours[i].iter().copied());
        out.extend(self.always.iter().copied());
        Dependents::Decls(out.into_iter().filter_map(|j| decls[j].site).collect())
    }
}
