//! Projection of the checker's recovery observations into the canonical
//! model. No source walk, target inference, call substitution or behavioral
//! judgment lives here. Symbolic operands retain their unresolved boundary.

use std::collections::BTreeMap;

use hale_model::recovery::{
    RecoveryAction, RecoveryBound, RecoveryInventory, RecoveryOperand, RecoveryOperation,
    RecoveryReceiverType, RecoveryRoot, RecoverySpecialization,
};
use hale_model::{EntityRef, LocusDeclId, ProvenanceId};
use hale_syntax::ast::{NodeId, RecoveryOp};
use hale_syntax::Span;

use crate::handler_routing::{HandlerRouting, RetryBound};
use crate::placement::SiteUniverse;
use crate::ty::Ty;
use crate::typed_bodies::{RecoveryRoot as TypedRoot, RecoveryRow, TypedBodies};

/// The model's projection seam. The stdlib's rows remain in their separate
/// universe even when a numeric body/declaration id overlaps a user's id.
pub fn project_recovery_facts(
    typed: &TypedBodies,
    stdlib: Option<&[RecoveryRow]>,
    handlers: &HandlerRouting,
    bodies: &BTreeMap<u32, EntityRef>,
    loci: &BTreeMap<u32, LocusDeclId>,
    mut provenance: impl FnMut(Span, SiteUniverse) -> ProvenanceId,
    mut evaluation_span: impl FnMut(u32) -> Option<Span>,
) -> RecoveryInventory {
    let mut project = |body: Option<NodeId>, row: &RecoveryRow, universe: SiteUniverse| {
        let user = universe == SiteUniverse::User;
        let direct = user && row.evaluation.is_empty();
        let body_entity = if direct { body.and_then(|id| bodies.get(&id.0).copied()) } else { None };
        let receiver_type = match (&row.receiver, row.child) {
            (Ty::Named(name), Some(child)) if child.universe == SiteUniverse::User && user => {
                loci.get(&child.decl.0).map(|declaration| RecoveryReceiverType::Locus {
                    declaration: *declaration, name: name.clone(),
                }).unwrap_or(RecoveryReceiverType::Unresolved)
            }
            (Ty::Named(name), Some(child)) if child.universe == SiteUniverse::StdlibAnalysis => {
                RecoveryReceiverType::ExternalLocus { name: name.clone() }
            }
            (Ty::Named(name), None) => RecoveryReceiverType::OtherNamed { name: name.clone() },
            _ => RecoveryReceiverType::Unresolved,
        };
        let root = match row.operand.root {
            TypedRoot::CurrentLocus(locus) if direct && locus.universe == SiteUniverse::User => {
                loci.get(&locus.decl.0).map(|declaration| RecoveryRoot::CurrentLocus {
                    declaration: *declaration,
                }).unwrap_or(RecoveryRoot::Unresolved)
            }
            TypedRoot::Parameter { body, ordinal, arity } if direct => {
                bodies.get(&body).map(|body| RecoveryRoot::Parameter { body: *body, ordinal, arity })
                    .unwrap_or(RecoveryRoot::Unresolved)
            }
            _ => RecoveryRoot::Unresolved,
        };
        let specialization = if row.specialization.is_empty() {
            RecoverySpecialization::Written
        } else if let Some(body) = body.filter(|_| user) {
            let template = typed.body(body).and_then(|b| b.enclosing_locus).unwrap_or(body);
            typed.monomorphs().of(template, &row.specialization).map(|m| {
                RecoverySpecialization::Concrete { symbol: m.name.clone() }
            }).unwrap_or(RecoverySpecialization::Unresolved)
        } else {
            RecoverySpecialization::Unresolved
        };
        let bound = if !row.bounded {
            RecoveryBound::Absent
        } else if direct {
            match handlers.retry_bound_at(row.statement) {
                Some(RetryBound::Const(value)) => RecoveryBound::Literal(value),
                Some(RetryBound::Expr(span)) => RecoveryBound::Expression { provenance: provenance(span, universe) },
                None => RecoveryBound::Unresolved,
            }
        } else {
            RecoveryBound::Unresolved
        };
        let evaluation: Vec<_> = row.evaluation.iter().enumerate().map(|(depth, site)| {
            // The outer invocation belongs to the user's walk. Nested
            // defaults can enter the stdlib's separately minted store;
            // their bare numeric ids do not qualify that namespace.
            // Never let an overlapping user id manufacture a witness.
            if user && depth == 0 { evaluation_span(*site).map(|span| provenance(span, universe)) } else { None }
        }).collect();
        // A default's expression may come from another source/store. Until
        // its declaration is bound, these offsets cannot index the caller's
        // text. Its known evaluation sites remain separate user witnesses.
        let source_universe = if row.evaluation.is_empty() { universe } else { SiteUniverse::StdlibAnalysis };
        let execution_caller = if user && (direct || evaluation.first().is_some_and(Option::is_some)) {
            body.and_then(|id| bodies.get(&id.0)).and_then(|b| match b { EntityRef::Function(f) => Some(*f), _ => None })
        } else { None };
        RecoveryAction {
            body: body_entity,
            execution_caller,
            in_handler: row.in_handler,
            specialization,
            evaluation,
            operation: match row.op {
                RecoveryOp::Restart => RecoveryOperation::Restart,
                RecoveryOp::RestartInPlace => RecoveryOperation::RestartInPlace,
                RecoveryOp::Quarantine => RecoveryOperation::Quarantine,
                RecoveryOp::Bubble => RecoveryOperation::Bubble,
                RecoveryOp::Reorganize => RecoveryOperation::Reorganize,
            },
            receiver_type,
            operand: RecoveryOperand { root, fields: row.operand.fields.clone() },
            bound,
            provenance: provenance(row.statement, source_universe),
            receiver_provenance: row.receiver_span.map(|span| provenance(span, source_universe)),
        }
    };
    let user = Some(typed.recoveries().map(|(body, row)| project(Some(body), row, SiteUniverse::User)).collect());
    let stdlib = stdlib.map(|rows| rows.iter().map(|row| project(None, row, SiteUniverse::StdlibAnalysis)).collect());
    RecoveryInventory { user, stdlib, calls: None, evaluated_calls: None, failure_entries: None }
}

/// Human-readable observations for `hale model dump`; never a machine
/// identity or an input to another compiler pass.
pub fn render_recovery_facts(model: &hale_model::ApplicationModel) -> String {
    let inventory = &model.analyses.recovery;
    if inventory.user.as_ref().is_some_and(Vec::is_empty)
        && inventory.stdlib.as_ref().is_some_and(Vec::is_empty)
    {
        return String::new();
    }
    let body_name = |body: EntityRef| match body {
        EntityRef::Function(id) => model.entities.functions[id.index()].display.clone(),
        EntityRef::LocusDecl(id) => model.entities.loci[id.index()].display.clone(),
        EntityRef::Type(id) => model.entities.types[id.index()].display.clone(),
        EntityRef::Declaration(id) => model.entities.declarations[id.index()].display.clone(),
        _ => "unresolved body".into(),
    };
    let mut text = String::from("recovery observations (symbolic; instance targets unresolved):\n");
    for (label, rows) in [("user", &inventory.user), ("stdlib", &inventory.stdlib)] {
        let Some(rows) = rows else {
            text.push_str(&format!("  {label}: unavailable\n"));
            continue;
        };
        text.push_str(&format!("  {label}: {} typed statement contexts\n", rows.len()));
        for row in rows {
            let operation = match row.operation {
                RecoveryOperation::Restart => "restart",
                RecoveryOperation::RestartInPlace => "restart_in_place",
                RecoveryOperation::Quarantine => "quarantine",
                RecoveryOperation::Bubble => "bubble",
                RecoveryOperation::Reorganize => "reorganize",
            };
            let receiver = match &row.receiver_type {
                RecoveryReceiverType::Locus { name, .. } | RecoveryReceiverType::ExternalLocus { name }
                | RecoveryReceiverType::OtherNamed { name } => name.as_str(),
                RecoveryReceiverType::Unresolved => "unresolved type",
            };
            let mut operand = match row.operand.root {
                RecoveryRoot::CurrentLocus { declaration } => format!("self({})", model.entities.loci[declaration.index()].display),
                RecoveryRoot::Parameter { body, ordinal, .. } => format!("parameter {} of {}", ordinal, body_name(body)),
                RecoveryRoot::Unresolved => "unresolved value".into(),
            };
            for field in &row.operand.fields { operand.push('.'); operand.push_str(field); }
            let mut context = match &row.specialization {
                RecoverySpecialization::Written => "written".to_string(),
                RecoverySpecialization::Concrete { symbol } => symbol.clone(),
                RecoverySpecialization::Unresolved => "unresolved specialization".into(),
            };
            if !row.evaluation.is_empty() {
                context.push_str(&format!("; evaluated default depth {}", row.evaluation.len()));
            }
            let bound = match row.bound {
                RecoveryBound::Absent => String::new(),
                RecoveryBound::Literal(n) => format!(" for {n}"),
                RecoveryBound::Expression { .. } => " for runtime expression".into(),
                RecoveryBound::Unresolved => " for unresolved bound".into(),
            };
            text.push_str(&format!("    {} [{context}]: {operation} {receiver} via {operand}{bound}\n",
                row.body.map(body_name).unwrap_or_else(|| "unresolved body".into())));
        }
    }
    text
}

/// Join typed invocation operands to the existing call relation by caller
/// declaration, authored site identity and callee declaration. No text/span
/// join, second call resolver or default-context substitution is permitted.
pub fn project_recovery_call_bindings(
    typed: &TypedBodies,
    bodies: &BTreeMap<u32, EntityRef>,
    loci: &BTreeMap<u32, LocusDeclId>,
    calls: &[hale_model::Call],
    sites: &BTreeMap<(hale_model::FunctionId, u32), std::collections::BTreeSet<Option<hale_graph::ids::SiteId>>>,
) -> Option<Vec<hale_model::recovery::RecoveryCallBinding>> {
    use hale_model::recovery::{RecoveryCallBinding, RecoveryCallContext};
    let rows = typed.recovery_calls()?;
    let mut by_site: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for (body, row) in rows {
        if !row.evaluation.is_empty() { continue; }
        if let Some(EntityRef::Function(from)) = bodies.get(&body.0) {
            by_site.entry((*from, row.call.0)).or_default().push((body, row));
        }
    }
    let operand = |value: &crate::typed_bodies::RecoveryOperand| RecoveryOperand {
        root: match value.root {
            TypedRoot::CurrentLocus(locus) if locus.universe == SiteUniverse::User => {
                loci.get(&locus.decl.0).map(|declaration| RecoveryRoot::CurrentLocus { declaration: *declaration })
                    .unwrap_or(RecoveryRoot::Unresolved)
            }
            TypedRoot::Parameter { body, ordinal, arity } => bodies.get(&body)
                .map(|body| RecoveryRoot::Parameter { body: *body, ordinal, arity })
                .unwrap_or(RecoveryRoot::Unresolved),
            _ => RecoveryRoot::Unresolved,
        },
        fields: value.fields.clone(),
    };
    Some(calls.iter().map(|call| {
        let mut contexts = Vec::new();
        let authored = sites.get(&(call.from, call.site)).filter(|s| s.len() == 1)
            .and_then(|s| s.first()).copied().flatten();
        if let Some(site) = authored {
            for (body, row) in by_site.get(&(call.from, site.index)).into_iter().flatten() {
                if row.callee.and_then(|id| bodies.get(&id.0)) != Some(&EntityRef::Function(call.to)) { continue; }
                let Some(parameter_arity) = row.callee_arity else { continue };
                // A variadic ABI is not a positional recovery-helper contract.
                if row.arguments.len() > parameter_arity as usize { continue; }
                let specialization = if row.specialization.is_empty() {
                    RecoverySpecialization::Written
                } else {
                    let template = typed.body(*body).and_then(|b| b.enclosing_locus).unwrap_or(*body);
                    typed.monomorphs().of(template, &row.specialization).map(|m| RecoverySpecialization::Concrete { symbol: m.name.clone() })
                        .unwrap_or(RecoverySpecialization::Unresolved)
                };
                let context = RecoveryCallContext {
                    specialization,
                    receiver: row.receiver.as_ref().map(&operand),
                    arguments: row.arguments.iter().map(&operand).collect(),
                    parameter_arity,
                };
                if !contexts.contains(&context) { contexts.push(context); }
            }
        }
        RecoveryCallBinding { from: call.from, to: call.to, site: call.site, contexts, provenance: call.provenance }
    }).collect())
}

/// Keep evaluated default calls in their qualified contexts. The outer
/// invocation is a user witness; nested expression ids do not qualify the
/// source store and are deliberately not looked up in the user's mint.
pub fn project_evaluated_recovery_calls(
    typed: &TypedBodies,
    bodies: &BTreeMap<u32, EntityRef>,
    mut outer_span: impl FnMut(u32) -> Option<Span>,
    mut provenance: impl FnMut(Option<Span>) -> ProvenanceId,
) -> Option<Vec<hale_model::recovery::RecoveryEvaluatedCall>> {
    let rows = typed.recovery_calls()?;
    Some(rows.filter(|(_, row)| !row.evaluation.is_empty()).map(|(body, row)| {
        let outer = outer_span(row.evaluation[0]);
        let witness = provenance(outer);
        let mut evaluation = vec![None; row.evaluation.len()];
        if outer.is_some() { evaluation[0] = Some(witness); }
        let function = |id| match bodies.get(&id) { Some(EntityRef::Function(f)) => Some(*f), _ => None };
        hale_model::recovery::RecoveryEvaluatedCall {
            caller: outer.and_then(|_| function(body.0)),
            callee: row.callee.and_then(|callee| function(callee.0)),
            evaluation, provenance: witness,
        }
    }).collect())
}

/// One finite reverse closure for coverage, not a counting graph. Missing
/// callees and relevant call holes seed uncertainty; known default calls
/// participate so nested defaults cannot hide a later recovery helper.
/// Each search state retains the seed's coverage mask, so invocation-only
/// failure uncertainty cannot invent an unresolved direct publish site.
pub fn evaluated_calls_reaching_recovery(
    calls: &[hale_model::Call],
    evaluated: &[hale_model::recovery::RecoveryEvaluatedCall],
    roots: impl IntoIterator<Item = (hale_model::FunctionId, hale_model::RelationSet)>,
) -> BTreeMap<hale_model::FunctionId, hale_model::RelationSet> {
    use crate::model_graph::{self, HolePolicy, Visit};
    let mut reverse: BTreeMap<_, std::collections::BTreeSet<_>> = BTreeMap::new();
    for call in calls { reverse.entry(call.to).or_default().insert(call.from); }
    for call in evaluated {
        if let (Some(from), Some(to)) = (call.caller, call.callee) { reverse.entry(to).or_default().insert(from); }
    }
    let opaque = hale_model::HoleKind::UnmodeledRecovery.implicit_execution_families().expect("recovery gap");
    let roots = roots.into_iter().chain(evaluated.iter().filter(|c| c.callee.is_none())
        .filter_map(|c| c.caller.map(|f| (f, opaque)))).map(|(f, mask)| (f, mask.0));
    let mut reached = BTreeMap::new();
    let _ = model_graph::search(roots, |(f, mask)| {
        reached.entry(*f).and_modify(|known: &mut hale_model::RelationSet| *known = known.union(hale_model::RelationSet(*mask)))
            .or_insert(hale_model::RelationSet(*mask));
        Visit::<_, (), ()>::edges(reverse.get(f).into_iter().flatten().map(|from| ((*from, *mask), ())).collect())
    }, |_| false, |_| false, None, HolePolicy::Halt);
    reached
}

/// Project conditional field-construction routes from placement source rows.
/// The arrangement correspondence retains every construction alternative.
/// An ownership edge by itself supplies no construction or supervisor fact.
#[allow(clippy::too_many_arguments)]
pub fn project_recovery_failure_entries(
    table: &crate::placement::PlacementTable,
    constructions: &BTreeMap<crate::placement::InstanceKey, hale_model::LocusInstanceId>,
    entities: &hale_model::Entities,
    supervision: &[hale_model::Supervises],
    handlers: &HandlerRouting,
    functions: &BTreeMap<hale_graph::ids::SiteId, hale_model::FunctionId>,
    mut provenance: impl FnMut(crate::placement::SiteRef) -> Option<ProvenanceId>,
) -> Vec<hale_model::recovery::RecoveryFailureEntry> {
    use hale_model::recovery::{RecoveryConstructionSource as Source, RecoveryFailureContext as Context,
        RecoveryFailureDispatch as Dispatch, RecoveryFailureEntry as Entry, RecoveryFailureGap as Gap};
    use crate::ownership::SupervisionOverride;
    let built = table.root.as_ref().is_some_and(|root| root.built_by_a_literal());
    let mut witnesses = BTreeMap::new();
    let mut witness_at = |site| *witnesses.entry(site).or_insert_with(|| provenance(site));
    // Source correspondence is independent of the current holder. Preserve
    // every alias rather than selecting the first logical instance of a source.
    let mut source_instances: BTreeMap<_, std::collections::BTreeSet<_>> = BTreeMap::new();
    for (key, instance) in constructions {
        if let Some(source) = table.construction_source(key) {
            source_instances.entry(source).or_default().insert(*instance);
        }
    }
    let mut out: Vec<_> = entities.locus_instances.iter().enumerate().map(|(index, _)| Entry {
        failed: hale_model::LocusInstanceId(index as u32), contexts: Vec::new(),
    }).collect();
    for (key, instance) in constructions {
        let Some(entry) = out.get_mut(instance.index()) else { continue };
        let fallback = entities.locus_instances[instance.index()].provenance;
        let source = table.construction_source(key);
        let row = source.and_then(|key| table.instances.get(key));
        let witness = row.and_then(|r| r.literal).and_then(&mut witness_at).unwrap_or(fallback);
        let mut construction_path = Vec::new();
        let mut cursor = source;
        // Owner keys strictly shorten the field path. A malformed producer
        // cannot make witness extraction loop forever.
        for _ in 0..=source.map_or(0, |key| key.path.len()) {
            let Some(at) = cursor.and_then(|key| table.instances.get(key)) else { break };
            construction_path.push(at.literal.and_then(&mut witness_at));
            cursor = at.owner.as_ref();
        }
        construction_path.reverse();
        let mut context = Context { construction: None,
            guarded: row.is_some_and(|r| r.guarded) || table.instances.get(key).is_some_and(|r| r.guarded), construction_path,
            dispatch: Dispatch::Unknown(Gap::MissingConstruction), provenance: witness };
        context.dispatch = (|| {
            if !built { return Dispatch::Unknown(Gap::MissingConstruction); }
            let Some(row) = row else { return Dispatch::Unknown(Gap::MissingConstruction) };
            if source.and_then(|key| source_instances.get(key)).is_some_and(|instances| instances.len() != 1) {
                return Dispatch::Unknown(Gap::AmbiguousInstance);
            }
            if row.literal.is_none() && row.owner.is_some() { return Dispatch::Unknown(Gap::HeldValue); }
            let Some(owner) = row.owner.as_ref() else {
                return Dispatch::Unknown(if source != Some(key) { Gap::HeldValue } else { Gap::RootContext });
            };
            context.construction = match row.construction_supervisor {
                Some(SupervisionOverride::FieldHolder) => Some(Source::FieldHolder),
                Some(SupervisionOverride::ParamsInitializer) => Some(Source::ParamsInitializer),
                _ => return Dispatch::Unknown(Gap::MissingConstruction),
            };
            let Some(supervisors) = table.construction_source(owner).and_then(|key| source_instances.get(key))
                else { return Dispatch::Unknown(Gap::UnmappedSupervisor) };
            if supervisors.len() != 1 { return Dispatch::Unknown(Gap::AmbiguousSupervisor); }
            let supervisor = *supervisors.first().expect("one supervisor");
            let (Some(child), Some(parent)) = (row.realizes.as_ref(), table.instances.get(owner).and_then(|r| r.realizes.as_ref()))
                else { return Dispatch::Unknown(Gap::UnmappedSupervisor) };
            if !parent.args.is_empty() || !child.args.is_empty() { return Dispatch::Unknown(Gap::GenericSpecialization); }
            let Some(handler) = handlers.route_instance(NodeId(parent.site.id.index), &parent.lowered, &child.lowered)
                else { return Dispatch::Unhandled { supervisor } };
            if handler.parent_id != Some(parent.site.id) || handler.child_decl != Some(child.site)
                || handler.error_type != "ClosureViolation" { return Dispatch::Unknown(Gap::UnmappedHandler); }
            let Some(function) = handler.id.and_then(|id| functions.get(&id)).copied()
                else { return Dispatch::Unknown(Gap::UnmappedHandler) };
            let ordinal = handlers.rows().iter().position(|r| r.id == handler.id);
            let Some(relation) = ordinal.and_then(|ordinal| supervision.iter().position(|r| r.authored_ordinal == ordinal as u32))
                else { return Dispatch::Unknown(Gap::UnmappedHandler) };
            Dispatch::Handled { supervisor, handler: function, supervision: relation as u32 }
        })();
        entry.contexts.push(context);
    }
    for entry in &mut out {
        if entry.contexts.is_empty() {
            entry.contexts.push(Context { construction: None, guarded: false, construction_path: Vec::new(),
                dispatch: Dispatch::Unknown(Gap::MissingConstruction),
                provenance: entities.locus_instances[entry.failed.index()].provenance });
        }
    }
    out
}

/// Render conditional construction routes for inspection. This is neither a
/// machine identity nor a certificate that a failure is admitted or contained.
pub fn render_recovery_failure_entries(model: &hale_model::ApplicationModel) -> String {
    use hale_model::recovery::{RecoveryConstructionSource as Source, RecoveryFailureDispatch as Dispatch, RecoveryFailureGap as Gap};
    let Some(entries) = &model.analyses.recovery.failure_entries else { return String::new() };
    if entries.is_empty() { return String::new(); }
    let witness = |id: ProvenanceId| match &model.provenance.records[id.index()] {
        hale_model::Provenance::Source { source, span } => format!("{}:{}..{}", model.provenance.sources[source.index()].path, span.0, span.1),
        hale_model::Provenance::ForeignSpan { span } => format!("unheld:{}..{}", span.0, span.1),
        hale_model::Provenance::Synthetic { origin } => origin.clone(),
    };
    let mut text = String::from("conditional failure entries (construction facts; not runtime admission):\n");
    for entry in entries {
        for (index, context) in entry.contexts.iter().enumerate() {
            let source = match context.construction {
                Some(Source::FieldHolder) => "field holder",
                Some(Source::ParamsInitializer) => "params initializer",
                None => "unavailable construction",
            };
            let route = match context.dispatch {
                Dispatch::Handled { supervisor, handler, .. } => format!("{} on {}", model.entities.functions[handler.index()].display, model.entities.locus_instances[supervisor.index()].path),
                Dispatch::Unhandled { supervisor } => format!("unhandled by {}", model.entities.locus_instances[supervisor.index()].path),
                Dispatch::Unknown(reason) => format!("unknown: {}", match reason {
                    Gap::RootContext => "root context",
                    Gap::MissingConstruction => "missing construction",
                    Gap::HeldValue => "held value's original supervisor",
                    Gap::UnmappedSupervisor => "unmapped supervisor",
                    Gap::AmbiguousInstance => "multiple logical instances of one construction",
                    Gap::AmbiguousSupervisor => "multiple logical supervisors of one construction",
                    Gap::GenericSpecialization => "unbound generic specialization",
                    Gap::UnmappedHandler => "unmapped handler",
                }),
            };
            let path = context.construction_path.iter().map(|p| p.map(&witness).unwrap_or_else(|| "unavailable site".into())).collect::<Vec<_>>().join(" -> ");
            text.push_str(&format!("  {} [context {index}; {source}{}]: {route}\n    construction: {path}\n",
                model.entities.locus_instances[entry.failed.index()].path, if context.guarded { "; guarded occurrence" } else { "" }));
        }
    }
    text
}
