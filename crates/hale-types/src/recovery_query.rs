//! Conditional recovery-target queries over canonical identities.
//!
//! A construction-selected handler frame is the premise, not a runtime
//! admission proof. The shared graph engine follows checker-supplied call
//! bindings without collapsing two instances of the same declaration.
//! Missing operands and execution coverage remain gaps. No result of this
//! module is a containment verdict or discharges lifecycle/incarnation work.

use std::collections::{BTreeMap, BTreeSet};
use hale_model::{ApplicationModel, EntityRef, FunctionId, LocusInstanceId, ModelError, RelationSet};
use hale_model::recovery::{RecoveryReceiverType, RecoverySpecialization};
use crate::model_graph::{self, HolePolicy, Search, Visit};
use crate::model_query::{self, ConditionalRecoveryEntry, RecoveryBindingGap, RecoveryCallGap, RecoveryEntryGap, RecoveryInvocation};

/// One canonical call and its typing alternative in a recovery witness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryProbeCall {
    /// Index into this model's canonical calls relation.
    pub relation: usize,
    /// Index into that call's retained typing alternatives.
    pub context: usize,
}

/// A source path, conditional on the selected construction and handler entry.
/// Every frame retains receiver/argument identities, not just function names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryProbeTrace {
    /// The selected arranged logical instance, not an incarnation.
    pub failed: LocusInstanceId,
    /// Which construction alternative supplies this conditional entry.
    pub context: usize,
    /// The construction witness; absent if selecting an entry is itself unknown.
    pub entry: Option<ConditionalRecoveryEntry>,
    /// Handler and helper frames, each with its own receiver and arguments.
    pub frames: Vec<RecoveryInvocation>,
    /// The canonical call/context path between those frames.
    pub calls: Vec<RecoveryProbeCall>,
}

/// The exact source account that prevents exhaustive target inspection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryProbeGap {
    Entry(RecoveryEntryGap),
    ActionsUnavailable,
    CallBinding { relation: usize, reason: RecoveryBindingGap },
    CallOperand { relation: usize, context: usize, gap: RecoveryCallGap },
    CallSpecialization { relation: usize, context: usize },
    ActionBinding { action: usize, reason: RecoveryBindingGap },
    ActionSpecialization { action: usize },
    UnboundAction { action: usize },
    ModelHole { hole: usize },
    ImplicitExecution { target: FunctionId, site: model_query::ExecutionSite, provenance: hale_model::ProvenanceId },
    ContractedCall { relation: usize },
    ForeignExecution { absorption: usize },
    DeliveryExecution { publish: usize },
    ImplicitRecovery { action: usize },
    UnvouchedCalls,
}

/// Invalid selections are errors, never empty target sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryProbeError { InvalidFailedInstance, InvalidAllowedInstance }

/// Inspection of the represented source paths, not a language verdict.
/// `NoRepresentedEscape` does not establish runtime containment: construction
/// guards, admission, incarnations and native recovery semantics remain outside
/// this premise. A possible escape is a may-path, not a feasible temporal trace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConditionalRecoveryProbe {
    /// A represented action receiver outside the selected set, under the source premise.
    PossibleEscape { action: usize, target: LocusInstanceId, trace: RecoveryProbeTrace },
    /// No represented outside target was found, but the reachable source account is incomplete.
    Incomplete { gap: RecoveryProbeGap, trace: RecoveryProbeTrace },
    /// Exhausted the inspected source graph. Runtime containment remains unproved.
    NoRepresentedEscape,
    /// The caller's explicit inspection ceiling prevented exhaustion.
    Saturated,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Vertex {
    Entry(usize),
    Frame { context: usize, frame: RecoveryInvocation },
    Action { context: usize, frame: RecoveryInvocation, action: usize, target: LocusInstanceId },
}
impl Vertex {
    fn context(&self) -> usize {
        match self { Self::Entry(c) => *c, Self::Frame { context, .. } | Self::Action { context, .. } => *context }
    }
}
#[derive(Clone, Debug)]
enum Step { Entry, Call(RecoveryProbeCall), Action }

/// A validated-model index. Construction, call and typed recovery tables stay
/// the authorities; this index performs no source walk or name resolution.
pub struct RecoveryQueries<'a> {
    model: &'a ApplicationModel,
    calls: BTreeMap<FunctionId, Vec<usize>>,
    actions: BTreeMap<FunctionId, Vec<usize>>,
    unbound: BTreeMap<FunctionId, Vec<usize>>,
    holes: BTreeMap<FunctionId, Vec<usize>>,
    unscoped_holes: Vec<usize>,
    unvouched_calls: bool,
    implicit: BTreeMap<FunctionId, Vec<(FunctionId, model_query::ExecutionSite, hale_model::ProvenanceId)>>,
    foreign: BTreeMap<FunctionId, Vec<RecoveryProbeGap>>,
    publishes: BTreeMap<FunctionId, Vec<usize>>,
}

impl<'a> RecoveryQueries<'a> {
    /// Validate once, then reuse the canonical relation indices across probes.
    pub fn new(model: &'a ApplicationModel) -> Result<Self, ModelError> {
        model.validate()?;
        let mut out = Self { model, calls: BTreeMap::new(), actions: BTreeMap::new(), unbound: BTreeMap::new(),
            holes: BTreeMap::new(), unscoped_holes: Vec::new(),
            unvouched_calls: !model.capabilities.exact_calls && !model.holes.iter().any(|h| h.hides.intersects(RelationSet::CALLS)),
            implicit: BTreeMap::new(), foreign: BTreeMap::new(), publishes: BTreeMap::new() };
        for (index, call) in model.relations.calls.iter().enumerate() {
            if call.dispatch == hale_model::DispatchKind::ViaStdlib { out.foreign.entry(call.from).or_default().push(RecoveryProbeGap::ContractedCall { relation: index }); }
            else { out.calls.entry(call.from).or_default().push(index); }
        }
        for (index, action) in model.analyses.recovery.user.iter().flatten().enumerate() {
            if let Some(EntityRef::Function(body)) = action.body { out.actions.entry(body).or_default().push(index); }
            else if let Some(caller) = action.execution_caller { out.unbound.entry(caller).or_default().push(index); }
        }
        for (index, hole) in model.holes.iter().enumerate() {
            if hole.hides.intersects(RelationSet::CALLS.union(RelationSet::EFFECTS)) {
                match hole.at {
                    EntityRef::Function(body) => { out.holes.entry(body).or_default().push(index); }
                    // Current validation permits call/effect gaps only on
                    // functions. Preserve an unscoped gap if that vocabulary
                    // later expands, until this query learns its scope.
                    _ => out.unscoped_holes.push(index),
                }
            }
        }
        for edge in model_query::executions(model) {
            if !matches!(edge.site, model_query::ExecutionSite::Call(_)) { out.implicit.entry(edge.from).or_default().push((edge.to, edge.site, edge.provenance)); }
        }
        for (absorption, call) in model.analyses.stdlib_absorption.iter().enumerate() {
            out.foreign.entry(call.from).or_default().push(RecoveryProbeGap::ForeignExecution { absorption });
        }
        for (index, publish) in model.relations.publishes.iter().enumerate() { out.publishes.entry(publish.function).or_default().push(index); }
        Ok(out)
    }

    /// Search EVERY construction alternative with the shared graph engine.
    /// A known source target outside `allowed` wins over a missing account;
    /// otherwise the first scoped gap is returned. `max_steps` is an explicit
    /// inspection limit, and exhaustion of that limit never means absence.
    pub fn probe_boundary(&self, failed: LocusInstanceId, allowed: &BTreeSet<LocusInstanceId>, max_steps: Option<u32>)
        -> Result<ConditionalRecoveryProbe, RecoveryProbeError>
    {
        let m = self.model;
        if failed.index() >= m.entities.locus_instances.len() { return Err(RecoveryProbeError::InvalidFailedInstance); }
        if allowed.iter().any(|id| id.index() >= m.entities.locus_instances.len()) { return Err(RecoveryProbeError::InvalidAllowedInstance); }
        let count = m.analyses.recovery.failure_entries.as_ref().and_then(|rows| rows.get(failed.index())).map_or(1, |row| row.contexts.len());
        let entries: Vec<_> = (0..count).map(|context| model_query::recovery_handler_invocation(m, failed, context)).collect();
        let found = model_graph::search((0..count).map(Vertex::Entry), |vertex| {
            match vertex {
                Vertex::Entry(context) => match &entries[*context] {
                    Ok(entry) => Visit::edges(vec![(Vertex::Frame { context: *context, frame: entry.invocation.clone() }, Step::Entry)]),
                    Err(reason) => Visit::hole(RecoveryProbeGap::Entry(reason.clone())),
                },
                Vertex::Action { action, .. } => Visit::hole(RecoveryProbeGap::ImplicitRecovery { action: *action }),
                Vertex::Frame { context, frame } => {
                    let mut edges = Vec::new();
                    let mut gap = m.analyses.recovery.user.is_none().then_some(RecoveryProbeGap::ActionsUnavailable);
                    for &action in self.actions.get(&frame.function).into_iter().flatten() {
                        let row = &m.analyses.recovery.user.as_ref().expect("indexed inventory")[action];
                        if row.specialization != RecoverySpecialization::Written {
                            gap.get_or_insert(RecoveryProbeGap::ActionSpecialization { action }); continue;
                        }
                        let target = model_query::bind_recovery_operand(m, frame, &row.operand).and_then(|target| {
                            match row.receiver_type {
                                RecoveryReceiverType::Locus { declaration, .. } if m.entities.locus_instances[target.index()].decl == declaration => Ok(target),
                                _ => Err(RecoveryBindingGap::ReceiverType),
                            }
                        });
                        match target {
                            Ok(target) => edges.push((Vertex::Action { context: *context, frame: frame.clone(), action, target }, Step::Action)),
                            Err(reason) => { gap.get_or_insert(RecoveryProbeGap::ActionBinding { action, reason }); }
                        }
                    }
                    for &relation in self.calls.get(&frame.function).into_iter().flatten() {
                        match model_query::bind_recovery_call(m, relation, frame) {
                            Ok(calls) => for call in calls {
                                if m.analyses.recovery.calls.as_ref().expect("bound call inventory")[relation].contexts[call.context].specialization
                                    != RecoverySpecialization::Written {
                                    gap.get_or_insert(RecoveryProbeGap::CallSpecialization { relation, context: call.context }); continue;
                                }
                                if let Some(reason) = call.gaps.first() {
                                    gap.get_or_insert(RecoveryProbeGap::CallOperand { relation, context: call.context, gap: reason.clone() });
                                }
                                edges.push((Vertex::Frame { context: *context, frame: call.invocation }, Step::Call(RecoveryProbeCall { relation, context: call.context })));
                            },
                            Err(reason) => { gap.get_or_insert(RecoveryProbeGap::CallBinding { relation, reason }); }
                        }
                    }
                    if let Some(&action) = self.unbound.get(&frame.function).and_then(|rows| rows.first()) { gap.get_or_insert(RecoveryProbeGap::UnboundAction { action }); }
                    if let Some(&(target, site, provenance)) = self.implicit.get(&frame.function).and_then(|rows| rows.first()) { gap.get_or_insert(RecoveryProbeGap::ImplicitExecution { target, site, provenance }); }
                    if let Some(foreign) = self.foreign.get(&frame.function).and_then(|rows| rows.first()) { gap.get_or_insert_with(|| foreign.clone()); }
                    if let Some(&publish) = self.publishes.get(&frame.function).and_then(|rows| rows.first()) { gap.get_or_insert(RecoveryProbeGap::DeliveryExecution { publish }); }
                    if let Some(&hole) = self.holes.get(&frame.function).and_then(|rows| rows.first()) { gap.get_or_insert(RecoveryProbeGap::ModelHole { hole }); }
                    if let Some(&hole) = self.unscoped_holes.first() { gap.get_or_insert(RecoveryProbeGap::ModelHole { hole }); }
                    if self.unvouched_calls { gap.get_or_insert(RecoveryProbeGap::UnvouchedCalls); }
                    Visit { edges, hole: gap }
                }
            }
        }, |vertex| matches!(vertex, Vertex::Action { target, .. } if !allowed.contains(target)), |_| false, max_steps, HolePolicy::PathWins);
        Ok(match found {
            Search::Found { hit, parent } => match &hit {
                Vertex::Action { action, target, .. } => ConditionalRecoveryProbe::PossibleEscape { action: *action, target: *target,
                    trace: trace(failed, &entries, &hit, &parent) },
                _ => unreachable!("only a bound action is a target"),
            },
            Search::Uncertified { at, hole, parent } => ConditionalRecoveryProbe::Incomplete { gap: hole, trace: trace(failed, &entries, &at, &parent) },
            Search::NotFound => ConditionalRecoveryProbe::NoRepresentedEscape,
            Search::Saturated { .. } => ConditionalRecoveryProbe::Saturated,
        })
    }
}

fn trace(failed: LocusInstanceId, entries: &[Result<ConditionalRecoveryEntry, RecoveryEntryGap>], at: &Vertex, parent: &BTreeMap<Vertex, (Vertex, Step)>) -> RecoveryProbeTrace {
    let context = at.context();
    let mut vertices = vec![at]; let mut steps = Vec::new(); let mut cursor = at;
    while let Some((from, step)) = parent.get(cursor) { vertices.push(from); steps.push(step); cursor = from; }
    vertices.reverse(); steps.reverse();
    RecoveryProbeTrace { failed, context, entry: entries[context].as_ref().ok().cloned(),
        frames: vertices.into_iter().filter_map(|v| match v { Vertex::Frame { frame, .. } => Some(frame.clone()), _ => None }).collect(),
        calls: steps.into_iter().filter_map(|s| match s { Step::Call(call) => Some(call.clone()), _ => None }).collect() }
}

/// Human-readable first target witnesses for model inspection. The empty
/// allowed set asks for any represented receiver, not a policy violation.
/// The explicit search limit is reported; no machine identity uses this text.
pub fn render_first_targets(model: &ApplicationModel) -> String {
    if model.analyses.recovery.user.as_ref().is_some_and(Vec::is_empty)
        && model.analyses.recovery.stdlib.as_ref().is_some_and(Vec::is_empty) { return String::new(); }
    let Some(entries) = &model.analyses.recovery.failure_entries else {
        return "first conditional recovery targets: unavailable construction-entry inventory\n".into();
    };
    let relevant: Vec<_> = entries.iter().filter(|row| row.contexts.iter().any(|c|
        matches!(c.dispatch, hale_model::recovery::RecoveryFailureDispatch::Handled { .. }))).collect();
    if relevant.is_empty() { return String::new(); }
    let mut text = String::from("first conditional recovery targets (source may-paths; not runtime containment):\n");
    let Ok(query) = RecoveryQueries::new(model) else { text.push_str("  unavailable: invalid model\n"); return text };
    for entry in relevant {
        let failed = &model.entities.locus_instances[entry.failed.index()].path;
        match query.probe_boundary(entry.failed, &BTreeSet::new(), Some(512)).expect("validated instance selection") {
            ConditionalRecoveryProbe::PossibleEscape { action, target, trace } => {
                let row = &model.analyses.recovery.user.as_ref().expect("available inventory")[action];
                let path = trace.frames.iter().map(|frame| model.entities.functions[frame.function.index()].display.as_str()).collect::<Vec<_>>().join(" -> ");
                text.push_str(&format!("  {failed} [context {}]: {:?} {}\n    source: {path}\n", trace.context, row.operation, model.entities.locus_instances[target.index()].path));
            }
            ConditionalRecoveryProbe::Incomplete { gap, trace } => text.push_str(&format!("  {failed} [context {}]: incomplete {gap:?}\n", trace.context)),
            ConditionalRecoveryProbe::NoRepresentedEscape => text.push_str(&format!("  {failed}: no represented target; runtime coverage unproved\n")),
            ConditionalRecoveryProbe::Saturated => text.push_str(&format!("  {failed}: inspection limit reached (512 states)\n")),
        }
    }
    text
}
