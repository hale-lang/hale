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

use hale_syntax::ast::{EpochSpec, LifecycleKind, LocusDecl, LocusMember, Program, TopDecl};
use hale_syntax::Diag;

use crate::binding_rows::BindingRows;
use crate::placement::{Decision, DomainKind, PlacementTable, SiteRef};
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
    diags
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
