//! Execution-domain separation (GH #1327), over the canonical model.
//!
//! This checks the complete instance projection of the selected locus
//! declarations. It does not infer placement from source, treat a
//! declaration as one instance, or confuse CPU affinity with a domain.

use std::collections::BTreeSet;

use hale_model::{ApplicationModel, ClaimIr, ClaimIrTable, EntityRef, GroupRef, LocusDeclId};
use hale_syntax::Diag;

use crate::judgment::{domain_is_judgable, span_of, Judged};
use crate::model_query::{instance_domains, InstanceDomains};
use crate::verdict::Verdict;

pub(crate) fn judge(
    table: &ClaimIrTable,
    model: &ApplicationModel,
    source_bases: &[u32],
) -> Vec<Judged> {
    let mut out = Vec::new();
    let claim_span = |pid| span_of(&table.provenance, source_bases, pid);
    let model_span = |pid| span_of(&model.provenance, source_bases, pid);
    for row in &table.rows {
        let ClaimIr::RequireSeparateDomains { left, right } = &row.law else {
            continue;
        };
        let mut diags = Vec::new();
        let select = |group: &GroupRef, diags: &mut Vec<Diag>| -> Option<BTreeSet<LocusDeclId>> {
            let Some(id) = group.group else {
                diags.push(Diag::ty(
                    claim_span(group.provenance),
                    format!(
                        "claim `{}` is invalid: group `{}` is never declared",
                        row.name, group.name.display,
                    ),
                ));
                return None;
            };
            if !domain_is_judgable(table, group) {
                diags.push(Diag::ty(
                    claim_span(group.provenance),
                    format!(
                        "claim `{}` is invalid: group `{}` has a refused selection; \
                     resolve its group diagnostics before checking execution-domain separation",
                        row.name, group.name.display,
                    ),
                ));
                return None;
            }
            let loci: BTreeSet<_> = model
                .relations
                .group_members
                .iter()
                .filter(|member| member.group == id)
                .filter_map(|member| match member.member {
                    EntityRef::LocusDecl(decl) => Some(decl),
                    _ => None,
                })
                .collect();
            if loci.is_empty() {
                diags.push(Diag::ty(
                    claim_span(group.provenance),
                    format!(
                        "claim `{}` is invalid: group `{}` selects no locus declarations; \
                     `separate_domains` requires a nonempty instance population on both sides",
                        row.name, group.name.display,
                    ),
                ));
                return None;
            }
            Some(loci)
        };
        let a = select(left, &mut diags);
        let b = select(right, &mut diags);
        let verdict = if let (Some(a), Some(b)) = (a, b) {
            let a = instance_domains(model, &a);
            let b = instance_domains(model, &b);
            // An unused declaration is a bad operand, not an absent
            // deployment the quantifier can silently omit. A hole at
            // that declaration instead records an unknown population.
            let mut invalid = false;
            for (group, projection) in [(left, &a), (right, &b)] {
                for decl in &projection.unrepresented {
                    if projection
                        .holes
                        .iter()
                        .any(|hole| hole.at == EntityRef::LocusDecl(*decl))
                    {
                        continue;
                    }
                    let locus = &model.entities.loci[decl.index()];
                    diags.push(Diag::ty(claim_span(group.provenance), format!(
                        "claim `{}` is invalid: group `{}` selects locus `{}`, which has no \
                         instance in the deployment; `separate_domains` cannot certify an unused declaration",
                        row.name, group.name.display, locus.display,
                    )).with_related(model_span(locus.provenance), "the undeployed locus declaration"));
                    invalid = true;
                }
            }
            if invalid {
                Verdict::Invalid
            } else if let Some((a, b)) = a.placed.iter().find_map(|a| {
                b.placed
                    .iter()
                    .find(|b| a.placement.domain == b.placement.domain)
                    .map(|b| (a, b))
            }) {
                // A concrete collision disproves separation even if
                // additional selected births or domains are unknown.
                // The same instance selected twice is a collision too.
                let left_instance = &model.entities.locus_instances[a.realization.instance.index()];
                let right_instance =
                    &model.entities.locus_instances[b.realization.instance.index()];
                let domain = &model.entities.thread_domains[a.placement.domain.index()];
                diags.push(Diag::ty(
                    claim_span(row.provenance),
                    format!(
                        "claim `{}` violated: `{}` in group `{}` and `{}` in group `{}` \
                     share execution domain `{}`",
                        row.name,
                        left_instance.path,
                        left.name.display,
                        right_instance.path,
                        right.name.display,
                        domain.name,
                    ),
                ));
                // Evidence uses ordinary diagnostic rows, which both
                // the check path and artifact preserve with locations.
                for (group, instance) in [(left, left_instance), (right, right_instance)] {
                    diags.push(Diag::ty(
                        model_span(instance.provenance),
                        format!(
                            "claim `{}` witness: `{}` selected by `{}` runs in `{}`",
                            row.name, instance.path, group.name.display, domain.name,
                        ),
                    ));
                }
                Verdict::Violated
            } else if incomplete(&a) || incomplete(&b) {
                diags.push(Diag::ty(
                    claim_span(row.provenance),
                    format!(
                    "claim `{}` cannot be certified: the execution domains of groups `{}` and `{}` \
                     are not completely known",
                    row.name, left.name.display, right.name.display,
                ),
                ));
                for (group, projection) in [(left, &a), (right, &b)] {
                    for hole in &projection.holes {
                        diags.push(Diag::ty(
                            model_span(hole.provenance),
                            format!(
                                "claim `{}`: group `{}` has unknown placement or population: {}",
                                row.name, group.name.display, hole.reason,
                            ),
                        ));
                    }
                    for realization in &projection.missing_placement {
                        let instance =
                            &model.entities.locus_instances[realization.instance.index()];
                        diags.push(Diag::ty(model_span(instance.provenance), format!(
                            "claim `{}`: instance `{}` selected by `{}` has no modeled execution domain",
                            row.name, instance.path, group.name.display,
                        )));
                    }
                }
                Verdict::Uncertified
            } else {
                Verdict::Holds
            }
        } else {
            Verdict::Invalid
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

fn incomplete(projection: &InstanceDomains<'_>) -> bool {
    !projection.holes.is_empty()
        || !projection.missing_placement.is_empty()
        || !projection.unrepresented.is_empty()
}
