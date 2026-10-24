//! Concrete flow membership after specialization, shared by lifecycle
//! and emission. The declaration survey stays independent of typing.
use crate::flows::FlowRows;
use crate::generic_instances::{self, Instantiation};
use crate::typed_bodies::Monomorphs;
use hale_syntax::ast::{flat_decls, Program, TopDecl};
use hale_syntax::Span;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
pub struct FlowSource {
    pub child: String,
    pub owner: String,
    pub release: Span,
    pub universe: crate::placement::SiteUniverse,
    pub specialization: Option<String>,
}

#[derive(Clone, Debug)]
pub struct FlowInstances {
    pub instantiations: Vec<Instantiation>,
    pub sources: Vec<FlowSource>,
    members: BTreeSet<String>,
}

impl FlowInstances {
    pub fn is_flow(&self, name: &str) -> bool {
        self.members.contains(name)
    }

    /// The survey's clauses, the closed specialization inventory and
    /// the stdlib's own clauses; no lifecycle or LLVM plan is read.
    pub fn derive(
        bundle: &crate::Bundle<'_>,
        survey: &FlowRows,
        typed: &Monomorphs,
        top: &crate::resolve::TopScope,
    ) -> Result<Self, generic_instances::Error> {
        let programs: Vec<&Program> = bundle.programs.values().copied().collect();
        let stdlib = Some(crate::stdlib_bodies::program().ok_or_else(|| {
            (
                "the stdlib analysis copy is unavailable; concrete flow membership is incomplete"
                    .to_string(),
                Span::new(0, 0),
            )
        })?);
        let instantiations =
            generic_instances::derive(&programs, stdlib, &bundle.import_renames, typed)?;
        let user_rows = survey.len();
        let mut rows = survey.clone();
        if let Some(stdlib) = stdlib {
            rows = rows.with_stdlib(&stdlib.items);
        }
        // A typed request may carry an already named monomorph. Resolve
        // it as the concrete locus the inventory says it is.
        rows.declared_loci(
            instantiations
                .iter()
                .filter(|i| i.locus)
                .map(|i| i.name.clone()),
        );
        rows = rows.admit_interfaces(top);
        let mut sources = Vec::new();
        for (index, flow) in rows.iter().enumerate() {
            for clause in &flow.clauses {
                for child in clause.locus.iter().chain(&clause.admits) {
                    sources.push(FlowSource {
                        child: child.clone(),
                        owner: clause.owner.clone(),
                        release: clause.span,
                        universe: if index < user_rows {
                            crate::placement::SiteUniverse::User
                        } else {
                            crate::placement::SiteUniverse::StdlibAnalysis
                        },
                        specialization: None,
                    });
                }
            }
        }
        // The same declaration precedence as emission's template maps.
        let mut templates = BTreeMap::new();
        for (universe, program) in programs
            .iter()
            .map(|p| (crate::placement::SiteUniverse::User, *p))
            .chain(stdlib.map(|p| (crate::placement::SiteUniverse::StdlibAnalysis, p)))
        {
            for item in flat_decls(&program.items) {
                if let TopDecl::Locus(l) = item {
                    if !l.generics.is_empty() {
                        templates.insert(l.name.name.clone(), (universe, l));
                    }
                }
            }
        }
        for instance in instantiations.iter().filter(|i| i.locus) {
            let Some((universe, template)) = templates.get(&instance.template) else {
                continue;
            };
            let subst = template
                .generics
                .iter()
                .zip(&instance.args)
                .map(|(g, a)| (g.name.name.clone(), a.clone()))
                .collect();
            for (release, child) in rows.specialization_clauses(template, |ty| {
                generic_instances::substitute_type_expr(ty, &subst)
            }) {
                sources.push(FlowSource {
                    child,
                    owner: template.name.name.clone(),
                    release,
                    universe: *universe,
                    specialization: Some(instance.name.clone()),
                });
            }
        }
        let members = sources.iter().map(|s| s.child.clone()).collect();
        Ok(Self {
            instantiations,
            sources,
            members,
        })
    }
}
