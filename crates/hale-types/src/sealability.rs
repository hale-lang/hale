//! GH #436: which loci could be `@sealed` today, and what it would cost.
//!
//! `@sealed` is opt-in, so adopting it across an existing codebase is a
//! question nobody can answer by reading: which loci already hold their
//! state privately, and which ones have callers reaching into them?
//! That is mechanically computable, and leaving it to inspection was
//! the part of "measure before building more" I wrongly called
//! not-code.
//!
//! **The survey reads the rows the sealed rule reads** (F.40 phase 4,
//! W4). The checker records every access to a locus's `params` through
//! a receiver typed as that locus, sealed or not, as a row of the typed
//! bodies ([`crate::typed_bodies::ParamAccess`]); the rule
//! ([`crate::sealed_access`]) reports the rows whose receiver is sealed
//! and whose reader is not the receiver. The survey asks the same
//! question of every locus: the rows reaching into it from outside its
//! own members are what sealing it would break. It re-checks nothing,
//! so it agrees with the rule by construction rather than by
//! re-running it, and it names a locus by its declaration, never by a
//! message's text.

use std::collections::BTreeMap;

use hale_syntax::ast::{Program, TopDecl};

use crate::placement::SiteUniverse;
use crate::typed_bodies::TypedBodies;

/// One locus's verdict.
pub struct Sealable {
    pub locus: String,
    /// Sites outside the locus that read OR write its `params`, as
    /// `Locus.param`. Empty means sealing it today is a no-op.
    pub blockers: Vec<String>,
}

/// Survey every locus `programs` declare (at any depth of modules), over
/// `rows`, the typed-body table of their check.
///
/// Already-sealed loci are included — they are sealed, so a program
/// that checks has no access to report for them, and omitting them
/// would make the report read as though they were unexamined.
pub fn survey(programs: &[&Program], rows: &TypedBodies) -> Vec<Sealable> {
    let mut by_decl: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for (_, access) in rows.param_accesses() {
        if access.receiver.universe != SiteUniverse::User || access.from_inside() {
            continue;
        }
        by_decl
            .entry(access.receiver.decl.0)
            .or_default()
            .push(format!("{}.{}", crate::sealed_access::shown_name(&access.locus), access.param));
    }
    let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in programs {
        for item in hale_syntax::ast::flat_decls(&p.items) {
            if let TopDecl::Locus(l) = item {
                let blockers = by_name.entry(l.name.name.clone()).or_default();
                blockers.extend(by_decl.get(&l.id.0).into_iter().flatten().cloned());
            }
        }
    }
    by_name
        .into_iter()
        .map(|(locus, mut blockers)| {
            blockers.sort();
            blockers.dedup();
            Sealable { locus, blockers }
        })
        .collect()
}

/// Render the survey for the CLI.
pub fn render(rows: &[Sealable]) -> String {
    let free: Vec<&Sealable> =
        rows.iter().filter(|r| r.blockers.is_empty()).collect();
    let blocked: Vec<&Sealable> =
        rows.iter().filter(|r| !r.blockers.is_empty()).collect();

    let mut out = String::new();
    out.push_str(&format!(
        "sealability: {} of {} loci can be `@sealed` today\n",
        free.len(),
        rows.len()
    ));
    if !free.is_empty() {
        out.push_str(
            "\n  free to seal (nothing outside touches their params):\n",
        );
        for r in &free {
            out.push_str(&format!("    {}\n", r.locus));
        }
    }
    if !blocked.is_empty() {
        out.push_str("\n  would break callers:\n");
        for r in &blocked {
            out.push_str(&format!(
                "    {} — {} external access(es): {}\n",
                r.locus,
                r.blockers.len(),
                r.blockers.join(", ")
            ));
        }
    }
    out
}
