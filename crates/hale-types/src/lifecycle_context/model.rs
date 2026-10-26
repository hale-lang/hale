//! Projection of the shared source contexts, never of a lifecycle plan.
use super::{LifecycleContexts, OwnerGap};
use crate::placement::{DeclRef, Origin, PlacementTable, SiteRef, SiteUniverse, Template};
use hale_graph::ids::SiteId;
use hale_model::construction as m;
use hale_model::{LocusDeclId, ProvenanceId};
use std::collections::BTreeMap;

/// Keep every source template and domain in the same order, so owner and
/// representative-parent indices retain their original source grain.
pub fn project_contexts(
    source: &LifecycleContexts,
    placement: &PlacementTable,
    declarations: &BTreeMap<SiteId, LocusDeclId>,
    domain_provenance: ProvenanceId,
    provenance: &mut impl FnMut(SiteRef) -> ProvenanceId,
) -> m::LifecycleInventory {
    fn declaration(
        d: &DeclRef,
        declarations: &BTreeMap<SiteId, LocusDeclId>,
        provenance: &mut impl FnMut(SiteRef) -> ProvenanceId,
    ) -> m::ContextDeclaration {
        let binding = match d.site.universe {
            SiteUniverse::StdlibAnalysis => m::DeclarationBinding::Foreign,
            SiteUniverse::User => declarations
                .get(&d.site.id)
                .copied()
                .map(m::DeclarationBinding::Declared)
                .unwrap_or(m::DeclarationBinding::Unrepresented),
        };
        m::ContextDeclaration {
            key: m::DeclarationKey {
                site: d.site,
                specialization: d.lowered.clone(),
            },
            binding,
            provenance: provenance(d.site),
        }
    }
    let contexts = source
        .instances
        .iter()
        .map(|s| {
            let construction_site = match &s.site.template {
                Template::Dynamic { literal } => Some(*literal),
                Template::Static(k) if k.path.is_empty() => match k.origin {
                    Origin::Construction(site) => Some(site),
                    Origin::Entry(_) | Origin::Binding(_) => None,
                },
                Template::Static(k) => placement.instances.get(k).and_then(|r| r.literal),
            };
            let literal = match &s.site.template {
                Template::Dynamic { literal } => *literal,
                Template::Static(k) => placement
                    .instances
                    .get(k)
                    .and_then(|r| r.literal)
                    .unwrap_or(match k.origin {
                        Origin::Construction(site)
                        | Origin::Entry(site)
                        | Origin::Binding(site) => site,
                    }),
            };
            let owner_gaps = s
                .owner_gaps
                .iter()
                .map(|gap| match gap {
                    OwnerGap::Recursive { candidate } => m::OwnerGap::Recursive {
                        candidate: *candidate,
                    },
                    OwnerGap::UnrepresentedEnclosing { declaration: d } => {
                        m::OwnerGap::UnrepresentedEnclosing {
                            declaration: declaration(d, declarations, provenance),
                        }
                    }
                    OwnerGap::MissingPlacedOwner { key } => {
                        m::OwnerGap::MissingPlacedOwner { key: key.clone() }
                    }
                })
                .collect();
            m::LifecycleContext {
                declaration: declaration(&s.site.decl, declarations, provenance),
                template: s.site.template.clone(),
                construction_site,
                how: s.how,
                contributions: s.contributions.clone(),
                owner_gaps,
                bound: s.bound.clone(),
                in_handler: s.in_handler,
                statement_of: s.statement_of,
                field: s.field.clone(),
                placed: s.placed,
                provenance: provenance(literal),
            }
        })
        .collect();
    m::LifecycleInventory {
        domains: placement
            .domains
            .iter()
            .map(|source| m::ContextDomain {
                source: source.clone(),
                provenance: domain_provenance,
            })
            .collect(),
        contexts,
    }
}
