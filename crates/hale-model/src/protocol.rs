//! Parent callback opportunities. These source facts are not runtime events,
//! admitted populations, caller frames or complete lifetime traces.

use crate::{FunctionId, LocusDeclId, ProvenanceId};
use hale_graph::ids::SiteId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProtocolStage {
    Accept,
    Release,
}
impl ProtocolStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Release => "release",
        }
    }
    pub fn guard(self) -> &'static str {
        match self {
            Self::Accept => "admission_opportunity",
            Self::Release => "release_opportunity",
        }
    }
}

/// A portable authored-call ordinal and a constructor ordinal never share a
/// namespace. Source-summary ordinals must be projected before entering here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProtocolRoot {
    Construction(u32),
    Call(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolDefinition {
    pub function: FunctionId,
    pub owner: LocusDeclId,
    pub stage: ProtocolStage,
    /// Resolved nominal symbol; None retains an unresolved signature.
    pub child: Option<String>,
    pub interface: bool,
    pub empty: bool,
}

/// A source constructor under a candidate accepting parent. A nested default
/// keeps its real body and constructor separately from the triggering root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolContext {
    /// Resolved source specialization, not the constructor template name.
    pub child: String,
    pub from: FunctionId,
    pub root: ProtocolRoot,
    pub root_source: SiteId,
    pub constructor_function: FunctionId,
    pub constructor: u32,
    pub constructor_source: SiteId,
    pub owner: LocusDeclId,
    pub in_loop: bool,
    pub unbounded: bool,
    pub provenance: ProvenanceId,
    pub constructor_provenance: ProvenanceId,
}
impl ProtocolContext {
    pub fn key(
        &self,
    ) -> (
        FunctionId,
        ProtocolRoot,
        FunctionId,
        u32,
        LocusDeclId,
        String,
    ) {
        (
            self.from,
            self.root,
            self.constructor_function,
            self.constructor,
            self.owner,
            self.child.clone(),
        )
    }
}

/// Entry into a nonempty parent body, conditional on its opportunity. The
/// context index names the source account, never a runtime occurrence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolEvaluation {
    pub context: u32,
    pub to: FunctionId,
    pub stage: ProtocolStage,
}

impl crate::ApplicationModel {
    /// Resolved constructor symbols from the existing source context inventory.
    /// Template identity and specialization identity remain separate. No name
    /// prefix parsing or source reconstruction supplies a generic binding.
    pub fn protocol_constructor_symbols(
        &self,
        function: FunctionId,
        site: u32,
    ) -> std::collections::BTreeSet<&str> {
        use crate::construction::{DeclarationBinding, SiteRef};
        let mut symbols = std::collections::BTreeSet::new();
        let Some(c) = self
            .relations
            .constructions
            .iter()
            .find(|c| c.function == function && c.site == site)
        else {
            return symbols;
        };
        let crate::ConstructorTarget::Locus(child) = c.owner else {
            return symbols;
        };
        let Some(declaration) = self.entities.loci.get(child.index()) else {
            return symbols;
        };
        if let Some(source) = c.source {
            for context in self
                .analyses
                .lifecycle_contexts
                .iter()
                .flat_map(|i| &i.contexts)
                .filter(|context| {
                    context.construction_site == Some(SiteRef::user(source))
                        && context.declaration.binding == DeclarationBinding::Declared(child)
                })
            {
                symbols.insert(context.declaration.key.specialization.as_str());
            }
        }
        // A represented nongeneric target already is its resolved nominal
        // identity. A generic template is never its own concrete specialization.
        if symbols.is_empty() && !matches!(declaration.flow, crate::FlowClass::GenericTemplate) {
            symbols.insert(declaration.name.as_str());
        }
        symbols
    }

    pub(crate) fn validate_protocol_account(&self) -> Result<(), crate::ModelError> {
        use std::collections::{BTreeMap, BTreeSet};
        let e = &self.entities;
        let r = &self.relations;
        let bad = |index, law| crate::ModelError::CoverageLaw { index, law };
        let mut definitions = BTreeMap::new();
        let mut positions = BTreeSet::new();
        let mut previous = None;
        for (index, d) in r.protocol_definitions.iter().enumerate() {
            let f = e
                .functions
                .get(d.function.index())
                .ok_or_else(|| bad(index, "parent definition names no function"))?;
            let owner = e
                .loci
                .get(d.owner.index())
                .ok_or_else(|| bad(index, "parent definition names no locus"))?;
            let phases: Vec<_> = r
                .phase_of
                .iter()
                .filter(|p| p.function == d.function)
                .collect();
            if previous.is_some_and(|old| old >= d.function)
                || f.kind != crate::FunctionKind::Hook
                || f.owner != Some(d.owner)
                || !f.analyzed
                || !f.summarized
                || f.decl.is_none()
                || f.name != format!("{}::{}", owner.name, d.stage.as_str())
                || f.display != format!("{}::{}", owner.display, d.stage.as_str())
                || phases.len() != 1
                || !e
                    .phases
                    .get(phases[0].phase.index())
                    .is_some_and(|p| p.name == d.stage.as_str())
                || !positions.insert((d.owner, d.stage))
                || d.child.as_ref().is_some_and(String::is_empty)
            {
                return Err(bad(
                    index,
                    "parent definition identity, stage, signature and locus ownership must agree",
                ));
            }
            if d.empty
                && (!f.direct_effects.is_empty()
                    || !f.effect_lower_bound.is_empty()
                    || f.effects_unknown
                    || r.calls.iter().any(|c| c.from == d.function)
                    || r.initializes.iter().any(|c| c.from == d.function)
                    || r.constructions.iter().any(|c| c.function == d.function)
                    || r.publishes.iter().any(|c| c.function == d.function)
                    || r.protocol_contexts.iter().any(|c| c.from == d.function))
            {
                return Err(bad(
                    index,
                    "an empty parent body cannot hide represented execution",
                ));
            }
            previous = Some(d.function);
            definitions.insert(d.function, d);
        }
        for (index, f) in e.functions.iter().enumerate() {
            let function = FunctionId(index as u32);
            let parent_phase = f.kind == crate::FunctionKind::Hook
                && r.phase_of
                    .iter()
                    .filter(|p| p.function == function)
                    .filter_map(|p| e.phases.get(p.phase.index()))
                    .any(|p| matches!(p.name.as_str(), "accept" | "release"));
            if parent_phase != definitions.contains_key(&function) {
                return Err(bad(
                    index,
                    "every authored parent callback requires exactly one definition",
                ));
            }
        }
        let constructors: BTreeMap<_, _> = r
            .constructions
            .iter()
            .map(|c| ((c.function, c.site), c))
            .collect();
        let hides = crate::HoleKind::UnmodeledInitialization
            .implicit_execution_families()
            .unwrap();
        let mut expected = BTreeSet::new();
        let mut previous = None;
        for (index, c) in r.protocol_contexts.iter().enumerate() {
            if previous.is_some_and(|old| old >= c.key()) {
                return Err(bad(index, "parent contexts must be sorted and unique"));
            }
            previous = Some(c.key());
            let from = e
                .functions
                .get(c.from.index())
                .ok_or_else(|| bad(index, "parent context names no caller"))?;
            let actual = constructors
                .get(&(c.constructor_function, c.constructor))
                .ok_or_else(|| bad(index, "parent context names no actual constructor"))?;
            if from.owner.is_none()
                || from.initializer.is_some()
                || actual.source != Some(c.constructor_source)
                || actual.provenance != c.constructor_provenance
                || c.owner.index() >= e.loci.len()
                || c.unbounded && !c.in_loop
                || c.provenance.index() >= self.provenance.records.len()
            {
                return Err(bad(
                    index,
                    "parent context must retain its caller, real constructor and source witnesses",
                ));
            }
            let (looped, unbounded, witness, mut pending) = match c.root {
                ProtocolRoot::Construction(site) => {
                    let root = constructors
                        .get(&(c.from, site))
                        .ok_or_else(|| bad(index, "parent root names no constructor"))?;
                    if root.source != Some(c.root_source) {
                        return Err(bad(
                            index,
                            "parent root source must equal its constructor source",
                        ));
                    }
                    (
                        root.in_loop,
                        root.unbounded,
                        root.provenance,
                        r.initializes
                            .iter()
                            .filter(|v| v.from == c.from && v.site == site)
                            .map(|v| v.to)
                            .collect::<Vec<_>>(),
                    )
                }
                ProtocolRoot::Call(site) => {
                    let call = r
                        .calls
                        .iter()
                        .find(|v| v.from == c.from && v.site == site)
                        .ok_or_else(|| bad(index, "parent root names no authored call"))?;
                    (
                        call.in_loop,
                        call.unbounded,
                        call.provenance,
                        r.argument_evaluations
                            .iter()
                            .filter(|v| v.from == c.from && v.site == site)
                            .map(|v| v.to)
                            .collect(),
                    )
                }
            };
            if witness != c.provenance
                || c.in_loop != (looped || actual.in_loop)
                || c.unbounded != (unbounded || actual.unbounded)
            {
                return Err(bad(
                    index,
                    "parent context must preserve root and constructor repetition and provenance",
                ));
            }
            let direct = c.root == ProtocolRoot::Construction(c.constructor)
                && c.from == c.constructor_function;
            if direct && c.root_source != c.constructor_source {
                return Err(bad(
                    index,
                    "direct parent context requires one original constructor identity",
                ));
            }
            let mut seen = BTreeSet::new();
            while let Some(function) = pending.pop() {
                if !seen.insert(function) {
                    continue;
                }
                pending.extend(
                    r.initializes
                        .iter()
                        .filter(|v| v.from == function)
                        .map(|v| v.to),
                );
                pending.extend(
                    r.argument_evaluations
                        .iter()
                        .filter(|v| v.from == function)
                        .map(|v| v.to),
                );
            }
            if !direct && !seen.contains(&c.constructor_function) {
                return Err(bad(index, "nested parent construction must follow the root's initializer evaluations, never ordinary calls"));
            }
            if !self.holes.iter().any(|h| {
                h.at == crate::EntityRef::Function(c.from)
                    && h.kind == crate::HoleKind::UnmodeledInitialization
                    && h.hides.contains(hides)
            }) {
                return Err(bad(
                    index,
                    "parent opportunity requires scoped invocation-context coverage",
                ));
            }
            let crate::ConstructorTarget::Locus(_child) = actual.owner else {
                return Err(bad(index, "a data type has no parent admission protocol"));
            };
            if definitions
                .values()
                .any(|d| d.owner == c.owner && d.interface && !d.empty)
            {
                return Err(bad(
                    index,
                    "interface parent matching remains outside the portable nominal account",
                ));
            }
            if !self
                .protocol_constructor_symbols(c.constructor_function, c.constructor)
                .contains(c.child.as_str())
            {
                return Err(bad(index, "parent child specialization must be supplied by its actual source constructor account"));
            }
            for d in definitions
                .values()
                .filter(|d| d.owner == c.owner && !d.empty)
            {
                if d.child.as_ref().is_some_and(|name| *name == c.child) {
                    expected.insert((index as u32, d.stage, d.function));
                }
            }
            if !expected.iter().any(|v| v.0 == index as u32) {
                return Err(bad(
                    index,
                    "parent context requires a matching nonempty callback",
                ));
            }
        }
        let actual: Vec<_> = r
            .protocol_evaluations
            .iter()
            .map(|v| (v.context, v.stage, v.to))
            .collect();
        if actual != expected.into_iter().collect::<Vec<_>>() {
            return Err(bad(
                0,
                "parent evaluations must equal exactly the contexts' matching nonempty callbacks",
            ));
        }
        Ok(())
    }
}
