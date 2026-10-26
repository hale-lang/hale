//! Aggregate resource claims over canonical instance and capacity rows.
//!
//! Queue slots are declared reservation limits. Keys do not erase a
//! registration's capacity. A finite known subtotal exceeding the limit
//! or a known unbounded registration is a violation despite other holes.

use std::collections::BTreeSet;

use hale_model::keys::Capacity;
use hale_model::{ApplicationModel, ClaimIr, ClaimIrTable, EntityRef};
use hale_syntax::Diag;

use crate::judgment::{domain_is_judgable, span_of, Judged};
use crate::model_query::queue_reservations;
use crate::verdict::Verdict;

pub(crate) fn judge(
    table: &ClaimIrTable,
    model: &ApplicationModel,
    source_bases: &[u32],
) -> Vec<Judged> {
    let mut out = Vec::new();
    for row in &table.rows {
        let ClaimIr::QueueSlots { group, limit } = &row.law else {
            continue;
        };
        let span = span_of(&table.provenance, source_bases, row.provenance);
        let evidence_span = |id| span_of(&model.provenance, source_bases, id);
        let mut diags = Vec::new();
        let declarations: BTreeSet<_> = model
            .relations
            .group_members
            .iter()
            .filter(|member| Some(member.group) == group.group)
            .filter_map(|member| match member.member {
                EntityRef::LocusDecl(id) => Some(id),
                _ => None,
            })
            .collect();
        let verdict = if group.group.is_none()
            || !domain_is_judgable(table, group)
            || declarations.is_empty()
        {
            diags.push(Diag::ty(span, format!(
                "claim `{}` is invalid: queue_slots requires group `{}` to select a nonempty set of locus declarations",
                row.name, group.name.display,
            )));
            Verdict::Invalid
        } else {
            let projection = queue_reservations(model, &declarations);
            let unused: Vec<_> = projection
                .unrepresented
                .iter()
                .filter(|decl| {
                    !projection
                        .unvouched
                        .iter()
                        .any(|name| matches!(*name, "exact_ownership" | "exact_cardinality"))
                        && !projection.holes.iter().any(|hole| {
                            hole.hides.intersects(
                                hale_model::RelationSet::OWNS
                                    .union(hale_model::RelationSet::CARDINALITY),
                            ) && match hole.at {
                                EntityRef::LocusDecl(id) => id == **decl,
                                _ => true,
                            }
                        })
                })
                .collect();
            if !unused.is_empty() {
                for decl in unused {
                    let locus = &model.entities.loci[decl.index()];
                    diags.push(
                        Diag::ty(
                            span,
                            format!(
                        "claim `{}` is invalid: selected locus `{}` has no arranged instance",
                        row.name, locus.display,
                    ),
                        )
                        .with_related(evidence_span(locus.provenance), "the undeployed locus"),
                    );
                }
                Verdict::Invalid
            } else {
                let mut total: u128 = 0;
                let mut unbounded = false;
                for registration in &projection.registrations {
                    match registration.subscription.capacity {
                        Capacity::Bounded(n) => total = total.saturating_add(n as u128),
                        Capacity::Unbounded => unbounded = true,
                    }
                }
                if unbounded || total > *limit as u128 {
                    let amount = if unbounded {
                        "unbounded".to_string()
                    } else {
                        total.to_string()
                    };
                    diags.push(Diag::ty(span, format!(
                        "claim `{}` violated: group `{}` reserves {} subscription queue slots, exceeding {}",
                        row.name, group.name.display, amount, limit,
                    )));
                    for registration in &projection.registrations {
                        let instance = &model.entities.locus_instances
                            [registration.realization.instance.index()];
                        let subscription = registration.subscription;
                        let amount = match subscription.capacity {
                            Capacity::Bounded(n) => n.to_string(),
                            Capacity::Unbounded => "unbounded".to_string(),
                        };
                        diags.push(
                            Diag::ty(
                                evidence_span(subscription.provenance),
                                format!(
                                    "claim `{}` witness: `{}` subscription #{} reserves {} slots",
                                    row.name, instance.path, subscription.site, amount,
                                ),
                            )
                            .with_related(
                                evidence_span(instance.provenance),
                                "the arranged instance",
                            ),
                        );
                    }
                    Verdict::Violated
                } else if !projection.holes.is_empty()
                    || !projection.unrepresented.is_empty()
                    || !projection.unvouched.is_empty()
                {
                    diags.push(Diag::ty(span, format!(
                        "claim `{}` cannot be certified: queue reservations for group `{}` have an unknown population or subscription set (known subtotal: {} slots)",
                        row.name, group.name.display, total,
                    )));
                    for hole in projection.holes {
                        diags.push(Diag::ty(
                            evidence_span(hole.provenance),
                            format!("claim `{}`: {}", row.name, hole.reason,),
                        ));
                    }
                    for family in projection.unvouched {
                        diags.push(Diag::ty(
                            span,
                            format!(
                                "claim `{}`: model does not vouch for {} completeness",
                                row.name, family,
                            ),
                        ));
                    }
                    Verdict::Uncertified
                } else {
                    Verdict::Holds
                }
            }
        };
        out.push(Judged {
            ordinal: row.ordinal,
            verdict,
            diags,
            foreign: Vec::new(),
        });
    }
    out
}
