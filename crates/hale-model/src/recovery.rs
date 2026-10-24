//! Typed recovery observations carried by the canonical model.
//!
//! These are symbolic operands, not resolved instance targets. In particular
//! an argument needs a call binding, `self` needs an execution context, and a
//! field path needs the state at the recovery statement. No containment law
//! may treat this inventory as a closed recovery relation. Unresolved values
//! stay explicit until that relation, its capabilities and holes are defined.

use crate::{ApplicationModel, EntityRef, LocusDeclId, ModelError, ProvenanceId};

/// Absence means unavailable, not an empty closed world. User and stdlib
/// walks have different site/provenance universes and are never merged by id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecoveryInventory {
    /// All observations from the user's typed-body table, when available.
    pub user: Option<Vec<RecoveryAction>>,
    /// Observations in the separate stdlib store, when available.
    pub stdlib: Option<Vec<RecoveryAction>>,
    /// Operand bindings for each represented call. None is unavailable;
    /// an empty contexts vector on a row means that call is unbound.
    pub calls: Option<Vec<RecoveryCallBinding>>,
    /// Calls evaluated in defaults, separate from authored calls in a body.
    /// Absence is unavailable; these are possible invocations, not counts.
    pub evaluated_calls: Option<Vec<RecoveryEvaluatedCall>>,
    /// Conditional failure entry contexts for each arranged logical instance.
    /// This is construction evidence, not runtime admission or completeness.
    pub failure_entries: Option<Vec<RecoveryFailureEntry>>,
}

/// All construction alternatives of one arranged logical instance. Alternatives
/// agree on its declaration/domain but need not agree on its birth context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryFailureEntry {
    /// The logical instance whose possible failure is the premise.
    pub failed: crate::LocusInstanceId,
    /// Every represented construction alternative, including unknown routes.
    pub contexts: Vec<RecoveryFailureContext>,
}

/// The positive source of a field construction's supervising context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryConstructionSource { FieldHolder, ParamsInitializer }

/// An explicit boundary of the currently represented construction/entry facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryFailureGap {
    RootContext,
    MissingConstruction,
    HeldValue,
    UnmappedSupervisor,
    AmbiguousInstance,
    AmbiguousSupervisor,
    GenericSpecialization,
    UnmappedHandler,
}

/// Static handler selection for an original field-construction context,
/// conditional on runtime delivery of a ClosureViolation. Unhandled is a
/// known route, distinct from unknown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryFailureDispatch {
    Handled { supervisor: crate::LocusInstanceId, handler: crate::FunctionId, supervision: u32 },
    Unhandled { supervisor: crate::LocusInstanceId },
    Unknown(RecoveryFailureGap),
}

/// A conditional construction occurrence. `guarded` means the occurrence exists
/// only under an alternative; false does not prove execution or runtime liveness.
/// Params settlement, domains, field state and incarnation admission remain
/// obligations of a future behavioral relation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryFailureContext {
    /// Positive evidence about the original field construction. A held
    /// projection may retain it; a holder alone supplies none.
    pub construction: Option<RecoveryConstructionSource>,
    /// An alternative must be taken for this construction to occur.
    pub guarded: bool,
    /// Root-to-field construction witnesses. Missing sites stay explicit and
    /// two root literals expanding one shared default remain distinct contexts.
    pub construction_path: Vec<Option<ProvenanceId>>,
    /// Conditional handler selection, known unhandled route, or typed gap.
    pub dispatch: RecoveryFailureDispatch,
    /// The construction witness, or the instance's fallback witness.
    pub provenance: ProvenanceId,
}

/// An evaluated default call whose target is the checker's declaration.
/// It is not an ordinary call row: repetition, guards and operand state
/// have not been incorporated into the execution/counting relation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryEvaluatedCall {
    /// The function that evaluates the outer default, when source-bound.
    pub caller: Option<crate::FunctionId>,
    /// The user callee resolved by the checker; otherwise unavailable.
    pub callee: Option<crate::FunctionId>,
    /// Outermost evaluation witness; unknown inner namespaces stay absent.
    pub evaluation: Vec<Option<ProvenanceId>>,
    /// The outer invocation witness, or an explicit synthetic unknown.
    pub provenance: ProvenanceId,
}

/// One represented call site's parameter-binding account. Targets come
/// from the canonical calls relation, never another name-based resolver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryCallBinding {
    /// The caller in the canonical calls relation.
    pub from: crate::FunctionId,
    /// The callee in the same relation.
    pub to: crate::FunctionId,
    /// The caller-local authored site ordinal.
    pub site: u32,
    /// Alternative typing contexts; empty means unavailable binding.
    pub contexts: Vec<RecoveryCallContext>,
    /// The canonical call source witness.
    pub provenance: ProvenanceId,
}

/// One checker context of a direct authored call. Fields still denote
/// values read in that context; they are not instance ownership paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryCallContext {
    /// The caller typing context, not an execution or feasibility proof.
    pub specialization: RecoverySpecialization,
    /// The written receiver for a member call, when present.
    pub receiver: Option<RecoveryOperand>,
    /// Supplied arguments, in parameter order; omitted defaults stay absent.
    pub arguments: Vec<RecoveryOperand>,
    /// The resolved declaration's number of authored parameters.
    pub parameter_arity: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryOperation {
    Restart,
    RestartInPlace,
    Quarantine,
    Bubble,
    Reorganize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryReceiverType {
    /// The declaration is the template; name preserves a concrete symbol.
    Locus { declaration: LocusDeclId, name: String },
    /// A known locus in the separate stdlib analysis store.
    ExternalLocus { name: String },
    /// A named type with no established locus declaration. It may be an
    /// interface, so this does not prove that the value is not a locus.
    /// `ClosureViolation` in `bubble(err)` is one example.
    OtherNamed { name: String },
    Unresolved,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryRoot {
    CurrentLocus { declaration: LocusDeclId },
    /// An immutable parameter of the named body, not a spelling-based join.
    Parameter { body: EntityRef, ordinal: u32, arity: u32 },
    Unresolved,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryOperand {
    /// The lexical binding; unresolved aliases are not live field paths.
    pub root: RecoveryRoot,
    /// The field values read at this statement, not an alias's saved value.
    pub fields: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoverySpecialization {
    Written,
    Concrete { symbol: String },
    Unresolved,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryBound {
    Absent,
    Literal(i64),
    Expression { provenance: ProvenanceId },
    Unresolved,
}

/// One typed statement on one typing context. Vector order is provider walk
/// order, not an execution order or a claim that a conditional branch runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryAction {
    /// Unrepresented bodies remain observations with an unresolved anchor.
    pub body: Option<EntityRef>,
    /// The function causally evaluating the statement. This can be known
    /// for a default whose lexical operand/body binding remains unresolved.
    pub execution_caller: Option<crate::FunctionId>,
    /// Whether typed under a failure-handler context (including defaults).
    pub in_handler: bool,
    /// The typing context, without formatting type arguments as identity.
    pub specialization: RecoverySpecialization,
    /// Outermost-first default evaluation witnesses. A missing site remains
    /// `None`; evaluated defaults have unresolved body/root bindings here.
    pub evaluation: Vec<Option<ProvenanceId>>,
    /// The typed recovery operation.
    pub operation: RecoveryOperation,
    /// The receiver type, distinct from its runtime instance.
    pub receiver_type: RecoveryReceiverType,
    /// The symbolic argument expression.
    pub operand: RecoveryOperand,
    /// The stated retry bound, with dynamic or unavailable cases explicit.
    pub bound: RecoveryBound,
    /// The statement source location.
    pub provenance: ProvenanceId,
    /// The first argument source location, absent for a missing argument.
    pub receiver_provenance: Option<ProvenanceId>,
}

impl RecoveryInventory {
    /// Validate references and closed shape invariants without upgrading
    /// symbolic operands to instance identities or asserting completeness.
    pub fn validate(&self, model: &ApplicationModel) -> Result<(), ModelError> {
        let e = &model.entities;
        let body_ok = |body: EntityRef| match body {
            EntityRef::Function(id) => id.index() < e.functions.len(),
            EntityRef::LocusDecl(id) => id.index() < e.loci.len(),
            EntityRef::Type(id) => id.index() < e.types.len(),
            EntityRef::Declaration(id) => id.index() < e.declarations.len(),
            _ => false,
        };
        let p_ok = |p: ProvenanceId| p.index() < model.provenance.records.len();
        if let Some(entries) = &self.failure_entries {
            if entries.len() != e.locus_instances.len() { return Err(ModelError::InvalidRecoveryFact { index: 0 }); }
            for (index, entry) in entries.iter().enumerate() {
                let bad = || ModelError::InvalidRecoveryFact { index };
                if entry.failed.index() != index || entry.contexts.is_empty() { return Err(bad()); }
                for context in &entry.contexts {
                    if !p_ok(context.provenance) || context.construction_path.iter().flatten().any(|p| !p_ok(*p))
                        || (context.construction.is_some() && context.construction_path.last() != Some(&Some(context.provenance))) {
                        return Err(bad());
                    }
                    let supervisor = match context.dispatch {
                        RecoveryFailureDispatch::Handled { supervisor, handler, supervision } => {
                            let Some(f) = e.functions.get(handler.index()) else { return Err(bad()) };
                            let Some(parent) = e.locus_instances.get(supervisor.index()) else { return Err(bad()) };
                            let Some(row) = model.relations.supervises.get(supervision as usize) else { return Err(bad()) };
                            if f.kind != crate::FunctionKind::FailureHandler || f.owner != Some(parent.decl)
                                || row.parent != parent.decl || row.child != crate::SupervisedRef::Locus(e.locus_instances[index].decl)
                                || row.error_type != "ClosureViolation"
                                || model.provenance.records.get(f.provenance.index()) != model.provenance.records.get(row.provenance.index()) {
                                return Err(bad());
                            }
                            Some(supervisor)
                        }
                        RecoveryFailureDispatch::Unhandled { supervisor } => {
                            let Some(parent) = e.locus_instances.get(supervisor.index()) else { return Err(bad()) };
                            if model.relations.supervises.iter().any(|r| r.parent == parent.decl
                                && r.child == crate::SupervisedRef::Locus(e.locus_instances[index].decl)) { return Err(bad()); }
                            Some(supervisor)
                        }
                        RecoveryFailureDispatch::Unknown(_) => None,
                    };
                    if let Some(supervisor) = supervisor {
                        if supervisor.index() >= e.locus_instances.len() || context.construction.is_none()
                            || supervisor == entry.failed || context.construction_path.len() < 2 {
                            return Err(bad());
                        }
                    }
                }
            }
        }
        for (index, call) in self.evaluated_calls.iter().flatten().enumerate() {
            if call.caller.is_some_and(|f| f.index() >= e.functions.len())
                || call.callee.is_some_and(|f| f.index() >= e.functions.len())
                || call.evaluation.is_empty() || !p_ok(call.provenance)
                || call.evaluation.iter().flatten().any(|p| !p_ok(*p))
                || (call.caller.is_some() && call.evaluation[0].is_none())
                || call.evaluation[0].is_some_and(|p| p != call.provenance) {
                return Err(ModelError::InvalidRecoveryFact { index });
            }
        }
        if let Some(calls) = &self.calls {
            if calls.len() != model.relations.calls.len() { return Err(ModelError::InvalidRecoveryFact { index: 0 }); }
            for (index, (binding, call)) in calls.iter().zip(&model.relations.calls).enumerate() {
                let bad = || ModelError::InvalidRecoveryFact { index };
                if binding.from != call.from || binding.to != call.to || binding.site != call.site
                    || binding.provenance != call.provenance || !p_ok(binding.provenance)
                    || binding.from.index() >= e.functions.len() || binding.to.index() >= e.functions.len()
                { return Err(bad()); }
                for context in &binding.contexts {
                    if context.arguments.len() > context.parameter_arity as usize { return Err(bad()); }
                    if let RecoverySpecialization::Concrete { symbol } = &context.specialization {
                        if symbol.is_empty() { return Err(bad()); }
                    }
                    for operand in context.receiver.iter().chain(&context.arguments) {
                        if operand.fields.iter().any(String::is_empty) { return Err(bad()); }
                        match operand.root {
                            RecoveryRoot::CurrentLocus { declaration } => {
                                if declaration.index() >= e.loci.len() || e.functions[binding.from.index()].owner != Some(declaration) {
                                    return Err(bad());
                                }
                            }
                            RecoveryRoot::Parameter { body, ordinal, arity } => {
                                if body != EntityRef::Function(binding.from) || ordinal >= arity { return Err(bad()); }
                            }
                            RecoveryRoot::Unresolved => {}
                        }
                    }
                }
            }
        }
        let rows = self.user.iter().flatten().map(|row| (false, row))
            .chain(self.stdlib.iter().flatten().map(|row| (true, row)));
        for (index, (stdlib, row)) in rows.enumerate() {
            let bad = || ModelError::InvalidRecoveryFact { index };
            if row.execution_caller.is_some_and(|f| f.index() >= e.functions.len())
                || row.body.is_some_and(|body| !body_ok(body)) || !p_ok(row.provenance)
                || row.receiver_provenance.is_some_and(|p| !p_ok(p))
                || row.evaluation.iter().flatten().any(|p| !p_ok(*p))
                || row.operand.fields.iter().any(|field| field.is_empty())
            {
                return Err(bad());
            }
            if let Some(EntityRef::Function(body)) = row.body {
                if row.execution_caller != Some(body) { return Err(bad()); }
            }
            let foreign = |p: ProvenanceId| matches!(model.provenance.records[p.index()], crate::Provenance::ForeignSpan { .. });
            if !row.evaluation.is_empty() && row.execution_caller.is_some()
                && row.evaluation.first().is_none_or(Option::is_none) { return Err(bad()); }
            if !row.evaluation.is_empty() && (row.body.is_some() || !matches!(row.operand.root, RecoveryRoot::Unresolved)
                || !foreign(row.provenance) || row.receiver_provenance.is_some_and(|p| !foreign(p))) {
                return Err(bad());
            }
            if stdlib {
                if row.body.is_some() || row.execution_caller.is_some() || !matches!(row.operand.root, RecoveryRoot::Unresolved)
                    || matches!(row.receiver_type, RecoveryReceiverType::Locus { .. })
                    || !foreign(row.provenance) || row.receiver_provenance.is_some_and(|p| !foreign(p))
                    || row.evaluation.iter().flatten().any(|p| !foreign(*p))
                {
                    return Err(bad());
                }
                if let RecoveryBound::Expression { provenance } = row.bound {
                    if !p_ok(provenance) || !foreign(provenance) { return Err(bad()); }
                }
            }
            match &row.receiver_type {
                RecoveryReceiverType::Locus { declaration, name } => {
                    if declaration.index() >= e.loci.len() || name.is_empty() { return Err(bad()); }
                }
                RecoveryReceiverType::ExternalLocus { name } | RecoveryReceiverType::OtherNamed { name } => {
                    if name.is_empty() { return Err(bad()); }
                }
                RecoveryReceiverType::Unresolved => {}
            }
            match row.operand.root {
                RecoveryRoot::CurrentLocus { declaration } => {
                    if declaration.index() >= e.loci.len() { return Err(bad()); }
                }
                RecoveryRoot::Parameter { body, ordinal, arity } => {
                    if !body_ok(body) || ordinal >= arity { return Err(bad()); }
                }
                RecoveryRoot::Unresolved => {}
            }
            if let RecoverySpecialization::Concrete { symbol } = &row.specialization {
                if symbol.is_empty() { return Err(bad()); }
            }
            if let RecoveryBound::Expression { provenance } = row.bound {
                if !p_ok(provenance) { return Err(bad()); }
            }
            if matches!(row.bound, RecoveryBound::Literal(_) | RecoveryBound::Expression { .. })
                && !matches!(row.operation, RecoveryOperation::Restart | RecoveryOperation::RestartInPlace)
            {
                return Err(bad());
            }
        }
        Ok(())
    }
}
