//! Declaration-grain reclamation policy, over the model's shared flow
//! classification. This judgment asserts neither termination nor a
//! retained population or byte bound.
use std::collections::BTreeSet;

use hale_model::{ApplicationModel, ClaimIr, ClaimIrTable, EntityRef};
use hale_syntax::Diag;

use crate::judgment::{domain_is_judgable, span_of, Judged};
use crate::model_query::flow_classes;
use crate::verdict::Verdict;

pub(crate) fn judge(
    table: &ClaimIrTable,
    model: &ApplicationModel,
    source_bases: &[u32],
) -> Vec<Judged> {
    let mut out = Vec::new();
    for row in &table.rows {
        let ClaimIr::RequireFlow { group } = &row.law else {
            continue;
        };
        let span = span_of(&table.provenance, source_bases, row.provenance);
        let mut diags = Vec::new();
        let declarations: BTreeSet<_> = model
            .relations
            .group_members
            .iter()
            .filter(|m| Some(m.group) == group.group)
            .filter_map(|m| match m.member {
                EntityRef::LocusDecl(id) => Some(id),
                _ => None,
            })
            .collect();
        let verdict = if group.group.is_none()
            || !domain_is_judgable(table, group)
            || declarations.is_empty()
        {
            diags.push(Diag::ty(span, format!(
                "claim `{}` is invalid: require flow needs group `{}` to select a nonempty set of locus declarations",
                row.name, group.name.display,
            )));
            Verdict::Invalid
        } else {
            let projection = flow_classes(model, &declarations);
            if projection.unvouched {
                // A resident is absence from the completed inventory.
                // Without a closed account of its gaps that absence is
                // not a counterexample, even if the stored tag says so.
                diags.push(Diag::ty(span, format!(
                    "claim `{}` is uncertified: flow classification is not vouched for and has no typed account of its gaps",
                    row.name,
                )));
                Verdict::Uncertified
            } else if !projection.residents.is_empty() {
                for id in projection.residents {
                    let locus = &model.entities.loci[id.index()];
                    diags.push(Diag::ty(span, format!(
                        "claim `{}` violated: `{}` in group `{}` is resident; no release clause classifies this concrete type as a flow",
                        row.name, locus.display, group.name.display,
                    )).with_related(span_of(&model.provenance, source_bases, locus.provenance), "the resident child type"));
                }
                Verdict::Violated
            } else if !projection.templates.is_empty() || !projection.holes.is_empty() {
                for id in projection.templates {
                    let locus = &model.entities.loci[id.index()];
                    diags.push(Diag::ty(span, format!(
                        "claim `{}` is uncertified: `{}` is a generic template; its concrete specializations need individual flow classification",
                        row.name, locus.display,
                    )).with_related(span_of(&model.provenance, source_bases, locus.provenance), "the generic declaration"));
                }
                // Canonical template rows account for these holes. Keep
                // a reason even for a model carrying a broader gap.
                if diags.is_empty() {
                    diags.push(Diag::ty(span, format!("claim `{}` is uncertified: selected flow classification has unresolved facts", row.name)));
                }
                Verdict::Uncertified
            } else {
                Verdict::Holds
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
