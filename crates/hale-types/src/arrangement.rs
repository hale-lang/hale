//! The arrangement: the placement table's rows projected onto the
//! user's locus declarations (F.40 phase 3, P1;
//! `notes/f40-placement-correspondence.md` § 2.4).
//!
//! It is read twice, and is one projection, the snapshot's
//! (`Snapshot::demand_arrangement`, F.40 phase 4, Q1): the model builder
//! makes the model's instances, owners, placements and placement holes
//! from it, and lowering reads the thread domains each locus runs on
//! from it ([`Arrangement::domains`], the dispatch plan's domain map:
//! F.40 phase 3, C5). Lowering cannot read the model's rows (a program
//! that swears to nothing lowers without a model, and the harness lowers
//! one the check refused), so it reads what they are made of.
//!
//! The instances are those of the root lowering deploys (one template
//! per literal of it, or the entry's implicit construction of a root no
//! literal builds), each where it runs. A held instance is arranged
//! under its holder, as the source's actual rows the table projects
//! there; the source template's own rows (`PlacementTable::handed_off`)
//! answer where it was built, and are not arranged.
//!
//! A path is the fields from the root, the replica index after the
//! replicated field (`App.f[2].k`). It has no construction component, so
//! one path stands for the rows of every template and alternative that
//! reach it: it is arranged when they agree on what they realize and
//! where they run, and otherwise it is left out with everything under it,
//! each declaration realized there unplaced (contract 3).
//!
//! The projection is user-only (U-4): a row realizing a stdlib
//! declaration (a field typed `std::io::tcp::Listener`, a stdlib locus
//! nested under a user one) is left out with its subtree, since its
//! declaration is no entity of the model and adding one would be shape
//! (contract 1). The coverage is partial by design: the table, not the
//! arrangement, answers every placement question. An adapter of the
//! root's `bindings { }` is no instance of the arrangement, and a literal
//! `fn main` builds besides the root stays outside it, a birth
//! [`Arrangement::unarranged`] accounts for.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{LocusDecl, LocusMember, Program, TopDecl};

use crate::ownership_graph::OwnershipGraph;
use crate::placement::{DomainId, DomainKind, InstanceKey, InstanceRow, Origin, PlacementTable, SiteRef};
use crate::snapshot::Snapshot;

/// One arranged instance.
pub struct Arranged {
    /// The fields from the root, the replica index after the replicated
    /// field.
    pub path: String,
    /// The user declaration it realizes, by name: the model's entity is
    /// one per name, and the dispatch plans key a locus by it.
    pub decl: String,
    /// The instance's OWN replica index — what codegen bakes into
    /// replica `i` and what a keyed subscriber on this field registers
    /// under: the replica row's, never an ancestor's copied down.
    pub replica: Option<u32>,
    /// The thread domain it runs on, by name (`main`, `pool:<name>`,
    /// `pinned:<path>`).
    pub domain: String,
    /// Its owner's path; `None` for the root.
    pub parent: Option<String>,
    /// The field's name in its owner's params, as written; the root's
    /// own name for the root.
    pub span: hale_syntax::Span,
}

/// The placement table's rows projected onto the user's declarations,
/// each declaration by its name. It borrows nothing, so a snapshot holds
/// it: one projection per snapshot, the model's and lowering's (F.40
/// phase 4, Q1).
pub struct Arrangement {
    /// The root's declaration, by name, when the table has a root the
    /// user declared.
    pub root: Option<String>,
    /// The arranged instances, one per path, in path order.
    pub instances: Vec<Arranged>,
    /// Every row at or under a path whose templates disagree, with the
    /// path they disagree at: in path order, each path's rows as the
    /// table yields them. No instance is arranged there, and each
    /// declaration realized there is unplaced.
    pub disagreeing: Vec<(Arranged, String)>,
    /// The births outside the arrangement (owner and placement resolve
    /// at runtime), each with its declaration's name and span, in the
    /// ownership graph's order. Each declaration is unplaced.
    pub unarranged: Vec<(String, hale_syntax::Span)>,
}

/// The path of an instance key: the root's name, then each field, the
/// replica index after the first.
fn path_of(root_name: Option<&str>, k: &InstanceKey) -> String {
    let mut p = root_name.unwrap_or_default().to_string();
    for (i, step) in k.path.iter().enumerate() {
        p.push('.');
        p.push_str(&step.field);
        if i == 0 {
            if let Some(r) = k.replica {
                p.push_str(&format!("[{r}]"));
            }
        }
    }
    p
}

impl Arrangement {
    /// A table domain's name: `main`, `pool:<name>`, or `pinned:<the
    /// anchor's path>`.
    pub fn domain_name(&self, table: &PlacementTable, d: DomainId) -> String {
        match &table.domain(d).kind {
            DomainKind::Main => "main".to_string(),
            DomainKind::Pool { name, .. } => format!("pool:{name}"),
            DomainKind::Pinned { anchor, .. } => {
                format!("pinned:{}", path_of(self.root.as_deref(), anchor))
            }
        }
    }

    /// Every declaration the arrangement does not fully place, by name:
    /// one realized under a path whose templates disagree, or born
    /// outside the arrangement.
    pub fn unplaced(&self) -> impl Iterator<Item = &str> + '_ {
        self.disagreeing.iter().map(|(a, _)| a.decl.as_str()).chain(self.unarranged.iter().map(|(l, _)| l.as_str()))
    }

    /// The dispatch plans' domain map
    /// ([`hale_model::dispatch_plan::domain_map`]) over this
    /// arrangement, keyed by each declaration's name: the raw post-merge
    /// symbol, the gates' spelling of a locus.
    pub fn domains(&self) -> BTreeMap<&str, Vec<String>> {
        hale_model::dispatch_plan::domain_map(
            self.instances.iter().map(|a| (a.decl.as_str(), a.domain.clone())),
            self.unplaced(),
        )
    }
}

/// Project the placement table onto the user's locus declarations of
/// `programs` (minted by `snapshot`), with the births the ownership graph
/// finds outside the arrangement. The snapshot projects it once
/// (`Snapshot::demand_arrangement`), and the model builder and lowering
/// read that one.
pub fn project_arrangement<'a>(
    programs: &[&'a Program],
    snapshot: &Snapshot,
    table: &PlacementTable,
    ownership: &OwnershipGraph,
) -> Arrangement {
    // The user's locus declarations by their minted site: what a row's
    // `realizes` names. A stdlib site is never here.
    let mut decl_at: BTreeMap<SiteRef, &'a LocusDecl> = BTreeMap::new();
    // And by node and by name: what a birth's declaration joins.
    let mut by_node: BTreeMap<u32, &'a LocusDecl> = BTreeMap::new();
    let mut by_name: BTreeMap<&'a str, &'a LocusDecl> = BTreeMap::new();
    for pr in programs {
        for item in hale_syntax::ast::flat_decls(&pr.items) {
            if let TopDecl::Locus(l) = item {
                if let Some(id) = snapshot.site_id(l.id) {
                    decl_at.insert(SiteRef::user(id), l);
                }
                if !l.id.is_none() {
                    by_node.insert(l.id.0, l);
                }
                by_name.insert(l.name.name.as_str(), l);
            }
        }
    }
    let root = table.root.as_ref().and_then(|r| decl_at.get(&r.realizes.site).copied());
    let root_name: Option<&str> = root.map(|l| l.name.name.as_str());
    let root_template = |o: Origin| match o {
        Origin::Entry(_) => true,
        Origin::Construction(c) => table.root.as_ref().is_some_and(|r| r.constructions.iter().any(|x| x.literal == c)),
        Origin::Binding(_) => false,
    };
    let domain_name = |d: DomainId| -> String {
        match &table.domain(d).kind {
            DomainKind::Main => "main".to_string(),
            DomainKind::Pool { name, .. } => format!("pool:{name}"),
            DomainKind::Pinned { anchor, .. } => format!("pinned:{}", path_of(root_name, anchor)),
        }
    };
    // The declaration a row realizes: a user locus.
    let user_decl = |r: &InstanceRow| -> Option<&'a LocusDecl> { decl_at.get(&r.realizes.as_ref()?.site).copied() };
    let prefix = |k: &InstanceKey, i: usize| InstanceKey {
        origin: k.origin,
        path: k.path[..i].to_vec(),
        replica: if i == 0 { None } else { k.replica },
    };
    let handed_off = table.handed_off();
    // Keep coverage before projecting user declarations. A template or
    // alternative whose subtree is unenumerable must not borrow a known
    // descendant from another template or alternative.
    let mut coverage: BTreeMap<String, BTreeSet<&InstanceKey>> = BTreeMap::new();
    let mut parents: BTreeMap<String, BTreeSet<&InstanceKey>> = BTreeMap::new();
    // Per path, every user row that reaches it.
    let mut at_path: BTreeMap<String, Vec<Arranged>> = BTreeMap::new();
    for (k, r) in &table.instances {
        if !root_template(k.origin) || handed_off.contains(k) {
            continue;
        }
        let path = path_of(root_name, k);
        coverage.entry(path.clone()).or_default().insert(k);
        // The row and every row above it realize a user locus.
        if !(0..k.path.len()).all(|i| table.instances.get(&prefix(k, i)).and_then(user_decl).is_some()) {
            continue;
        }
        let Some(l) = user_decl(r) else { continue };
        // The field's name in its owner's params, as written; the root's
        // own name for the root.
        let span = match (k.path.last(), r.owner.as_ref().and_then(|o| table.instances.get(o)).and_then(user_decl)) {
            (Some(step), Some(owner)) => owner
                .members
                .iter()
                .filter_map(|m| match m {
                    LocusMember::Params(pb) => pb.params.iter().find(|p| p.name.name == step.field),
                    _ => None,
                })
                .map(|p| p.name.span)
                .next()
                .unwrap_or(l.name.span),
            _ => l.name.span,
        };
        if let Some(owner) = &r.owner {
            parents.entry(path.clone()).or_default().insert(owner);
        }
        at_path.entry(path.clone()).or_default().push(Arranged {
            path,
            decl: l.name.name.clone(),
            // `validate` requires `None` on every path whose last
            // component is not a replica.
            replica: if k.path.len() == 1 { k.replica } else { None },
            domain: domain_name(r.domain),
            parent: r.owner.as_ref().map(|o| path_of(root_name, o)),
            span,
        });
    }
    // Two rows realize one declaration when they name it alike: the
    // model has one entity per name.
    let disagree: Vec<String> = at_path
        .iter()
        .filter(|(path, rows)| {
            rows.iter().any(|a| a.decl != rows[0].decl || a.domain != rows[0].domain)
                || rows.len() != coverage[*path].len()
                // Full instance keys retain construction and alternative
                // identity, which the model path omits.
                || rows[0].parent.as_ref().is_some_and(|parent| parents.get(*path) != coverage.get(parent))
        })
        .map(|(p, _)| p.clone())
        .collect();
    let mut instances: Vec<Arranged> = Vec::new();
    let mut disagreeing: Vec<(Arranged, String)> = Vec::new();
    for (path, rows) in at_path {
        let under = disagree
            .iter()
            .find(|d| path == **d || path.starts_with(&format!("{d}.")) || path.starts_with(&format!("{d}[")));
        match under {
            None => instances.extend(rows.into_iter().take(1)),
            Some(d) => disagreeing.extend(rows.into_iter().map(|a| (a, d.clone()))),
        }
    }
    // Instances in canonical (path-sorted) order.
    instances.sort_by(|a, b| a.path.cmp(&b.path));
    // C3: the ownership graph evaluates defaults per construction,
    // including explicit overrides and additional dynamic holders. Tell
    // it which source literals this arrangement represents; a held row
    // represents the literal of its construction source. Disagreeing
    // template paths are already unplaced above.
    let represented: BTreeSet<_> = table
        .instances
        .iter()
        .filter(|(key, _)| root_template(key.origin))
        .filter_map(|(_, row)| {
            row.literal
                .or_else(|| row.built_by.as_ref().and_then(|key| table.instances.get(key)).and_then(|source| source.literal))
        })
        .filter(|site| site.universe == crate::placement::SiteUniverse::User)
        .map(|site| site.id)
        .collect();
    let mut unarranged = Vec::new();
    for birth in ownership.unarranged_births(table, &represented) {
        let Some(decl) = birth.child_decl.map(|i| &ownership.declarations[i]) else { continue };
        // Minted declarations join by site; the legacy unminted bundle
        // keeps its name fallback.
        let l = match decl.id {
            Some(id) => by_node.get(&id.index),
            None => by_name.get(decl.name.as_str()),
        };
        let Some(l) = l else { continue };
        unarranged.push((l.name.name.clone(), birth.span));
    }
    Arrangement { root: root.map(|l| l.name.name.clone()), instances, disagreeing, unarranged }
}
