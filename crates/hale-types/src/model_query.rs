//! Shared typed queries over the canonical model.
//!
//! Five review rounds on the `causes:` engine (GH #476 Change 5f)
//! produced the same defect four times in different clothes: a rule
//! settled in one place and a sibling query left on the old key.
//! Delivery joined on the wire subject while the route query still
//! joined on the syntactic `declared_topic`; uncertainty was scoped
//! to an endpoint in one check and global in another. Each of those
//! is one question the model can answer once.
//!
//! So the questions live here, not in the engines:
//!
//!   * **may this publish reach this subscription?** —
//!     [`may_deliver`]
//!   * **is this endpoint's downstream (or upstream) fully
//!     modeled?** — [`endpoint_incomplete`]
//!   * **what does this function definitely do, and is that all?** —
//!     [`effects_of`]
//!   * **which execution domains contain these locus declarations'
//!     instances, and what placement is unknown?** —
//!     [`instance_domains`]
//!   * **what does this effect-class name mean, and how is a set of
//!     them rendered?** — [`effect_class_atoms`],
//!     [`render_effect_classes`]
//!
//! Every one of them joins on the model's typed identity.
//! `declared_topic` is a syntactic link to a declaration: a literal
//! `"t" <- …` send carries `None` there even when its text is a
//! declared topic's wire subject, and after lowering the runtime
//! cannot tell the two spellings apart. Nothing in this module
//! decides anything from it.

use std::collections::{BTreeMap, BTreeSet};

use hale_model::{
    ApplicationModel, Entities, EntityRef, FunctionId, RelationSet,
    SubjectId,
};

/// Declaration-grain flow classifications, with their original model
/// witnesses. These sets say nothing about which declarations are
/// instantiated, whether their runs terminate, or how many children
/// remain live. In particular a generic template is not a resident.
#[derive(Debug)]
pub struct FlowClasses<'a> {
    /// Concrete types selecting run-completion reclamation, with their
    /// classifying release clauses. The witnesses need not name the
    /// owner of a particular birth: flow membership is type-wide.
    pub flows: Vec<(hale_model::LocusDeclId, &'a [hale_model::FlowRelease])>,
    /// Concrete types absent from the completed flow inventory.
    pub residents: Vec<hale_model::LocusDeclId>,
    /// Generic declarations with no single asserted classification.
    pub templates: Vec<hale_model::LocusDeclId>,
    /// Unknown classification at a selected declaration.
    pub holes: Vec<&'a hale_model::Hole>,
    /// A withdrawn capability with no typed account of its gaps.
    pub unvouched: bool,
}

/// Project the selected declarations' canonical flow property. The
/// input IDs belong to this validated model. No source walk, lifecycle
/// plan, instance count, or inferred termination premise participates.
pub fn flow_classes<'a>(
    model: &'a ApplicationModel,
    declarations: &BTreeSet<hale_model::LocusDeclId>,
) -> FlowClasses<'a> {
    let mut out = FlowClasses {
        flows: Vec::new(),
        residents: Vec::new(),
        templates: Vec::new(),
        holes: model.holes.iter().filter(|h| {
            h.hides.intersects(RelationSet::FLOW_CLASSIFICATION)
                && matches!(h.at, EntityRef::LocusDecl(id) if declarations.contains(&id))
        }).collect(),
        unvouched: !model.capabilities.exact_flow_classification
            && !model.holes.iter().any(|h| h.hides.intersects(RelationSet::FLOW_CLASSIFICATION)),
    };
    for &decl in declarations {
        match &model.entities.loci[decl.index()].flow {
            hale_model::FlowClass::Resident => out.residents.push(decl),
            hale_model::FlowClass::Flow { releases } => out.flows.push((decl, releases)),
            hale_model::FlowClass::GenericTemplate => out.templates.push(decl),
        }
    }
    out
}

/// One arranged instance and its execution domain, with both model
/// records retained for a judgment's witness. Domain identity is the
/// typed `placement.domain`, never a name or a CPU-affinity set.
#[derive(Debug)]
pub struct InstanceDomain<'a> {
    /// The instance and the declaration it realizes, including the
    /// relation's provenance. The instance entity carries birth
    /// provenance.
    pub realization: &'a hale_model::Realizes,
    /// The instance's domain and the relation's provenance.
    pub placement: &'a hale_model::PlacedIn,
}

/// The known placement of selected declarations, alongside its limits.
/// Known rows remain available beside unknowns: a judgment may find a
/// concrete collision even when it cannot prove complete separation.
#[derive(Debug)]
pub struct InstanceDomains<'a> {
    /// Every represented, placed instance of a selected declaration,
    /// in canonical instance order. Replicas and separate constructions
    /// of one declaration remain separate rows.
    pub placed: Vec<InstanceDomain<'a>>,
    /// Represented instances for which the model supplies no domain.
    /// Missing placement is unknown, never evidence of separation.
    pub missing_placement: Vec<&'a hale_model::Realizes>,
    /// Selected declarations with no represented instance. This is
    /// distinct from an instance whose placement is missing; a hole
    /// may additionally say that a declaration is born dynamically.
    pub unrepresented: Vec<hale_model::LocusDeclId>,
    /// Selected declarations' holes hiding population or placement.
    /// Unrelated declarations' holes do not affect this projection.
    pub holes: Vec<&'a hale_model::Hole>,
}

/// Project instance domains from the model's existing `realizes` and
/// `placed_in` rows. The input IDs belong to this validated model.
///
/// Selection is exact: naming a declaration includes every instance of
/// it, not the instances of its descendants or a same-spelled import.
/// Births outside the arrangement are accounted for by the producer's
/// typed holes; this query never re-walks source or reconstructs the
/// placement table. It makes no decision about empty quantification or
/// whether a counterexample outranks an unknown.
pub fn instance_domains<'a>(
    model: &'a ApplicationModel,
    declarations: &BTreeSet<hale_model::LocusDeclId>,
) -> InstanceDomains<'a> {
    let mut out = InstanceDomains {
        placed: Vec::new(),
        missing_placement: Vec::new(),
        unrepresented: Vec::new(),
        holes: Vec::new(),
    };
    let mut represented = BTreeSet::new();
    for realization in &model.relations.realizes {
        if !declarations.contains(&realization.decl) {
            continue;
        }
        represented.insert(realization.decl);
        match model.relations.placed_in.binary_search_by_key(
            &realization.instance,
            |placement| placement.instance,
        ) {
            Ok(index) => out.placed.push(InstanceDomain {
                realization,
                placement: &model.relations.placed_in[index],
            }),
            Err(_) => out.missing_placement.push(realization),
        }
    }
    out.unrepresented = declarations.difference(&represented).copied().collect();
    let needed = RelationSet::OWNS
        .union(RelationSet::PLACED)
        .union(RelationSet::CARDINALITY);
    out.holes = model
        .holes
        .iter()
        .filter(|hole| {
            hole.hides.intersects(needed)
                && matches!(hole.at, EntityRef::LocusDecl(decl) if declarations.contains(&decl))
        })
        .collect();
    out
}

/// One subscription registration's declared capacity in one instance.
/// Distinct sites and replicas are distinct reservations, even when
/// they share a subject, handler, or execution domain.
#[derive(Debug)]
pub struct QueueReservation<'a> {
    /// The exact instance and its declaration.
    pub realization: &'a hale_model::Realizes,
    /// The subscription site and its authored capacity.
    pub subscription: &'a hale_model::Subscribe,
}

/// Instance-qualified queue reservations and the limits of that account.
#[derive(Debug)]
pub struct QueueReservations<'a> {
    /// One row per selected instance and subscription site.
    pub registrations: Vec<QueueReservation<'a>>,
    /// Selected declarations with no represented instance.
    pub unrepresented: Vec<hale_model::LocusDeclId>,
    /// Relevant unknown population or subscription knowledge.
    pub holes: Vec<&'a hale_model::Hole>,
    /// An unvouched relation without a typed account of its gaps
    /// cannot support a closed projection merely because no hole was
    /// recorded. Canonical producers supply that account as holes.
    pub unvouched: Vec<&'static str>,
}

/// Join subscription capacities to every selected arranged instance.
/// This accounts for declared slots, independent of key filtering and
/// placement. It does not estimate payload bytes or queue occupancy.
pub fn queue_reservations<'a>(
    model: &'a ApplicationModel,
    declarations: &BTreeSet<hale_model::LocusDeclId>,
) -> QueueReservations<'a> {
    let owners: BTreeMap<_, _> = model.relations.member_of.iter()
        .filter(|row| declarations.contains(&row.locus))
        .map(|row| (row.function, row.locus)).collect();
    let subscriptions: Vec<_> = model.relations.subscribes.iter()
        .filter(|row| owners.contains_key(&row.handler)).collect();
    let mut by_declaration: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for subscription in &subscriptions {
        by_declaration.entry(owners[&subscription.handler]).or_default().push(*subscription);
    }
    let instances: Vec<_> = model.relations.realizes.iter()
        .filter(|row| declarations.contains(&row.decl)).collect();
    let represented: BTreeSet<_> = instances.iter().map(|row| row.decl).collect();
    let instance_ids: BTreeSet<_> = instances.iter().map(|row| row.instance).collect();
    let seeds: BTreeSet<_> = model.relations.declared_in.iter()
        .filter(|row| matches!(row.entity, EntityRef::LocusDecl(id) if declarations.contains(&id)))
        .map(|row| row.seed).collect();
    let needed = RelationSet::OWNS.union(RelationSet::CARDINALITY).union(RelationSet::SUBSCRIBES);
    let holes = model.holes.iter().filter(|hole| hole.hides.intersects(needed))
        .filter(|hole| match hole.at {
            EntityRef::LocusDecl(id) => declarations.contains(&id),
            EntityRef::LocusInstance(id) => instance_ids.contains(&id),
            EntityRef::Function(id) => owners.contains_key(&id),
            // Missing registrations have no known owning locus. A
            // route with no known selected subscriber may hide one.
            EntityRef::Subject(_) | EntityRef::Topic(_) => true,
            EntityRef::Seed(id) => seeds.contains(&id),
            _ => false,
        }).collect();
    let mut registrations = Vec::new();
    for realization in instances {
        if let Some(subscriptions) = by_declaration.get(&realization.decl) {
            for subscription in subscriptions {
                registrations.push(QueueReservation { realization, subscription });
            }
        }
    }
    QueueReservations {
        registrations,
        unrepresented: declarations.difference(&represented).copied().collect(),
        holes,
        unvouched: model.capabilities.vouched_families().into_iter()
            .filter(|(_, exact, family)| !exact && family.intersects(needed))
            .filter(|(_, _, family)| !model.holes.iter().any(|hole| hole.hides.intersects(*family)))
            .map(|(name, _, _)| name).collect(),
    }
}

/// Which side of an endpoint a judgment is walking.
///
/// `causes:` asks what a publish can reach (downstream); `depends:`
/// asks what can reach a subscription (upstream). The completeness
/// question is the same shape on both sides, and so are the ways it
/// can be incomplete — including a binding, whose ROLE decides which
/// direction leaves the application.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    /// From a publish toward the handlers it may reach.
    Downstream,
    /// From a subscription back toward what may publish into it.
    Upstream,
}

/// Whether a publish to one endpoint can be DELIVERED to one
/// subscription.
///
/// The join is the model's typed wire identity — `SubjectId`.
/// Wildcards widen it. KEYS decide the rest: a keyed publish whose
/// domain is statically exact and a subscription whose predicate is
/// a literal outside it cannot meet — treating keyed delivery as
/// broadcast is the failure mode the keyed schema names. Everything
/// unknown widens, because over-approximating reach is the sound
/// direction for a law asking what a message can do.
pub fn may_deliver(
    e: &Entities,
    publish: &hale_model::Publish,
    sub: &hale_model::Subscribe,
) -> bool {
    may_deliver_keys(e, publish.subject, &publish.key_domain, sub)
}

/// [`may_deliver`] for a caller that holds a wire subject and a key
/// domain rather than a publish row — the routing-coverage claim
/// asks about a topic's permitted keys, not about one send.
pub fn may_deliver_keys(
    e: &Entities,
    subject: SubjectId,
    key_domain: &Option<hale_model::keys::KeyDomain>,
    sub: &hale_model::Subscribe,
) -> bool {
    if !subscription_covers(e, sub, subject) {
        return false;
    }
    use hale_model::keys::{KeyDomain, KeyPredicate, KeyValue};
    match (key_domain, &sub.key_predicate) {
        (Some(KeyDomain::Exact(vals)), KeyPredicate::EqLiteral(k)) => {
            vals.contains(k)
        }
        (
            Some(KeyDomain::IntRange { min, max }),
            KeyPredicate::EqLiteral(KeyValue::Int(k)),
        ) => k >= min && k <= max,
        _ => true,
    }
}

/// Does a subscription's pattern cover this wire subject? (The
/// address half of [`may_deliver`], for callers holding no publish
/// row — a backward walk starts from a subscription.)
pub fn subscription_covers(
    e: &Entities,
    sub: &hale_model::Subscribe,
    subject: SubjectId,
) -> bool {
    if sub.subject == subject {
        return true;
    }
    let pat = e.subjects[sub.subject.index()].pattern.as_str();
    let wire = e.subjects[subject.index()].pattern.as_str();
    pat.contains("**") && crate::wildcard_match(pat, wire)
}

/// Is what lies beyond this endpoint, in this direction, fully
/// modeled?
///
/// Two independent accounts, and a judgment needs both:
///
///   * **holes** — the model saying it does not know the endpoint's
///     structure. Downstream that is `SUBSCRIBES` (is the handler
///     set complete) and `KEY_FILTERS` (an unknown filter widens who
///     receives); upstream it is `PUBLISHES`. `DELIVERY` counts
///     either way: an `ExternalOpaque` boundary hides it.
///   * **typed routes** — a binding the model understands perfectly
///     well, that nonetheless takes the walk out of the application.
///     A `connect` route sends to a peer (downstream); a `listen`
///     route accepts from one (upstream). No hole is emitted for
///     these, so a judgment reading only `holes` fails open on them.
///
/// Scoped to THIS endpoint. A hole on an unrelated topic says
/// nothing about this walk — the model's hole law is
/// reachability-scoped, and a global scan lets an unrelated adapter
/// binding poison a purely local claim.
pub fn endpoint_incomplete(
    model: &ApplicationModel,
    subject: SubjectId,
    dir: Direction,
) -> bool {
    let e = &model.entities;
    let r = &model.relations;
    let wire = e.subjects[subject.index()].pattern.as_str();
    let structure = match dir {
        Direction::Downstream => RelationSet::SUBSCRIBES
            .union(RelationSet::KEY_FILTERS)
            .union(RelationSet::DELIVERY),
        Direction::Upstream => {
            RelationSet::PUBLISHES.union(RelationSet::DELIVERY)
        }
    };
    let holed = model.holes.iter().any(|h| {
        if !h.hides.intersects(structure) {
            return false;
        }
        match h.at {
            // Subject-grained residue covers by PATTERN: a hole on
            // `orders.**` is relevant to `orders.created`.
            EntityRef::Subject(sid) => {
                let pat = e.subjects[sid.index()].pattern.as_str();
                sid == subject
                    || (pat.contains("**")
                        && crate::wildcard_match(pat, wire))
            }
            // A topic-anchored hole is relevant when that topic
            // ADDRESSES this wire, however a send spelled it.
            EntityRef::Topic(t) => {
                e.topics.get(t.index()).is_some_and(|tp| tp.subject == subject)
            }
            _ => false,
        }
    });
    if holed {
        return true;
    }
    let crossing_role = match dir {
        Direction::Downstream => hale_model::BindingRole::Connect,
        Direction::Upstream => hale_model::BindingRole::Listen,
    };
    r.binds.iter().any(|b| {
        let binding = &e.bindings[b.binding.index()];
        binding.subject == subject && binding.role == crossing_role
    })
}

/// A publisher the model could not resolve, found by
/// [`unresolved_publishers`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnresolvedPublisher {
    /// The user function holding the send — or, for stdlib residue,
    /// the call that enters the interior holding it.
    pub function: FunctionId,
    pub kind: UnresolvedPublish,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnresolvedPublish {
    /// A send to a subject computed at runtime (`subj <- v`), bounded
    /// by the publish patterns its locus declares. Empty when nothing
    /// bounds it (no locus, or a locus declaring nothing).
    Computed { patterns: Vec<String> },
    /// Any other function-grain hole hiding PUBLISHES (an unanalyzed
    /// body): no pattern bounds what it sends.
    Unanalyzed { reason: String },
    /// A stdlib interior the function calls into: its sends carry no
    /// recorded subject bound or refusal policy the model can join.
    Absorbed,
}

/// Which publishers the model could NOT resolve may address
/// `subject` (a wire, or a pattern — the test is set overlap)?
///
/// The model records a computed send as a `ComputedSubject` hole at
/// its FUNCTION, hiding PUBLISHES, so a question asked at subject or
/// topic grain ([`endpoint_incomplete`], the judgment's bus-hole
/// index) never meets it. This is the function-grain half. A
/// computed send is admitted only under its locus's declared
/// `publish` patterns, and the publish site enforces them, so the
/// patterns bound which wires it can reach; residue with no pattern
/// addresses every subject.
pub fn unresolved_publishers(
    model: &ApplicationModel,
    subject: &str,
) -> Vec<UnresolvedPublisher> {
    let e = &model.entities;
    let r = &model.relations;
    let mut out: Vec<UnresolvedPublisher> = Vec::new();
    for h in &model.holes {
        let EntityRef::Function(f) = h.at else { continue };
        if !h.hides.intersects(RelationSet::PUBLISHES) {
            continue;
        }
        let kind = match h.kind {
            hale_model::HoleKind::ComputedSubject => {
                let mut patterns: Vec<String> = r
                    .member_of
                    .iter()
                    .filter(|m| m.function == f)
                    .flat_map(|m| {
                        r.declares_publish
                            .iter()
                            .filter(move |d| d.locus == m.locus)
                    })
                    .map(|d| e.subjects[d.subject.index()].pattern.clone())
                    .collect();
                patterns.sort();
                patterns.dedup();
                if !patterns.is_empty()
                    && !patterns.iter().any(|p| {
                        hale_model::subjects_can_overlap(p, subject)
                    })
                {
                    continue;
                }
                UnresolvedPublish::Computed { patterns }
            }
            _ => UnresolvedPublish::Unanalyzed {
                reason: h.reason.clone(),
            },
        };
        out.push(UnresolvedPublisher { function: f, kind });
    }
    for a in &model.analyses.stdlib_absorption {
        // The model's own residue test, plus an interior send that
        // speaks a declared topic: the absorption spells that one by
        // the topic's NAME, which is not its wire.
        let speaks_topic = a.nodes.iter().flat_map(|n| &n.events).any(|ev| {
            matches!(
                ev,
                hale_model::AbsorbedEvent::Publish {
                    declared_topic: Some(t),
                    ..
                } if e.topics.get(t.index()).is_some_and(|tp| {
                    hale_model::subjects_can_overlap(
                        &e.subjects[tp.subject.index()].pattern,
                        subject,
                    )
                })
            )
        });
        if speaks_topic || model.absorbed_publish_can_address(a, subject) {
            out.push(UnresolvedPublisher {
                function: a.from,
                kind: UnresolvedPublish::Absorbed,
            });
        }
    }
    out.sort_by_key(|u| u.function);
    out.dedup();
    out
}

/// What a function is KNOWN to do, and whether that is all of it.
///
/// Never read `Function::effects` for a judgment: it is the
/// RENDERED set, and it collapses to the single token
/// `unclassified` whenever the walk reached something unnameable,
/// discarding every class it had already proven. A law that a known
/// effect already violates would then read as merely uncertain.
pub fn effects_of(
    e: &Entities,
    f: FunctionId,
) -> (BTreeSet<String>, bool) {
    let row = &e.functions[f.index()];
    let known = row
        .effect_lower_bound
        .iter()
        .flat_map(|c| effect_class_atoms(e, c))
        .collect();
    (known, row.effects_unknown)
}

/// A class name's ATOMS, with the language's built-in folding
/// applied: `ffi` is the `syscall` bit, `spawn` and `recursion` own
/// no effect bit at all, a composed class means its expansion, and a
/// cyclic one means nothing.
pub fn effect_class_atoms(e: &Entities, name: &str) -> BTreeSet<String> {
    fn builtin_atom(n: &str) -> Option<&'static str> {
        Some(match n {
            "syscall" | "ffi" => "syscall",
            "block" => "block",
            "publish" => "publish",
            "time" => "time",
            "entropy" => "entropy",
            "env" => "env",
            "alloc" => "alloc",
            "secret_use" => "secret_use",
            "stdio" => "stdio",
            _ => return None,
        })
    }
    match e.effect_classes.iter().find(|c| c.name == name) {
        Some(c) => match &c.definition {
            hale_model::EffectClassDefinition::Composed { atoms } => {
                atoms.iter().flat_map(|a| effect_class_atoms(e, a)).collect()
            }
            hale_model::EffectClassDefinition::Atomic => {
                match builtin_atom(name) {
                    Some(b) => std::iter::once(b.to_string()).collect(),
                    None => std::iter::once(name.to_string()).collect(),
                }
            }
            hale_model::EffectClassDefinition::InvalidCycle => {
                BTreeSet::new()
            }
        },
        None => match builtin_atom(name) {
            Some(b) => std::iter::once(b.to_string()).collect(),
            None if name == "spawn" || name == "recursion" => {
                BTreeSet::new()
            }
            None => std::iter::once(name.to_string()).collect(),
        },
    }
}

/// Render a set of effect classes in the language's canonical order:
/// built-ins in their fixed order, then user classes in DECLARATION
/// order. Diagnostics are byte-compared, so the order is contract.
pub fn render_effect_classes(
    e: &Entities,
    classes: &BTreeSet<String>,
) -> Vec<String> {
    const BUILTINS: &[&str] = &[
        "syscall",
        "block",
        "publish",
        "time",
        "entropy",
        "env",
        "alloc",
        "secret_use",
        "stdio",
    ];
    let mut out: Vec<String> = Vec::new();
    for b in BUILTINS {
        if classes.contains(*b) {
            out.push((*b).to_string());
        }
    }
    let mut user: Vec<(u32, &String)> = classes
        .iter()
        .filter(|c| !BUILTINS.contains(&c.as_str()))
        .map(|c| {
            let idx = e
                .effect_classes
                .iter()
                .find(|ec| ec.name == *c)
                .map(|ec| ec.declaration_index)
                .unwrap_or(u32::MAX);
            (idx, c)
        })
        .collect();
    user.sort();
    out.extend(user.into_iter().map(|(_, c)| c.clone()));
    out
}


/// Selected declarations' storage configuration, including missing rows.
/// This projects canonical summary facts stored on the model. It neither
/// treats an unshipped mode as enforced nor turns cap arithmetic into a
/// retained-child, byte, population or restart-overlap proof.
pub fn storage_policies<'a>(
    model: &'a ApplicationModel,
    declarations: &BTreeSet<hale_model::LocusDeclId>,
) -> Vec<(hale_model::LocusDeclId, Option<&'a hale_model::LocusStorage>)> {
    declarations.iter().map(|&id| (id, model.entities.loci[id.index()].storage.as_ref())).collect()
}


/// Typed recovery observations for a body, together with observations whose
/// body has no model anchor. Unanchored observations are never silently
/// discarded by a scoped caller. This query does not resolve instance
/// targets, call arguments, field state or conditional execution.
pub struct RecoveryObservations<'a> {
    pub actions: Vec<&'a hale_model::recovery::RecoveryAction>,
    pub unanchored: Vec<&'a hale_model::recovery::RecoveryAction>,
    pub user_inventory_available: bool,
    pub stdlib_inventory_available: bool,
}

pub fn recovery_observations(model: &ApplicationModel, body: EntityRef) -> RecoveryObservations<'_> {
    let inventory = &model.analyses.recovery;
    let mut actions = Vec::new();
    let mut unanchored = Vec::new();
    for row in inventory.user.iter().chain(inventory.stdlib.iter()).flatten() {
        match row.body {
            Some(anchor) if anchor == body => actions.push(row),
            None => unanchored.push(row),
            _ => {}
        }
    }
    RecoveryObservations {
        actions, unanchored,
        user_inventory_available: inventory.user.is_some(),
        stdlib_inventory_available: inventory.stdlib.is_some(),
    }
}

/// A reachable failure/recovery execution gap, with a path through modeled
/// function calls (and delivery when the cost dimension includes it).
#[derive(Debug)]
pub struct RecoveryCostGap<'a> {
    /// The original typed gap and its failure/recovery source witness.
    pub hole: &'a hale_model::Hole,
    /// The source function through the gap's function, inclusive.
    pub path: Vec<FunctionId>,
}

/// Coverage only: never re-count a budget or infer recovery targets.
/// Fan-out includes possible bus delivery; local per-call dimensions
/// follow explicit and contracted calls. Unanchored recovery facts are
/// already represented by the model's conservative function-grain gaps.
pub fn recovery_cost_gap(model: &ApplicationModel, root: FunctionId, include_delivery: bool) -> Option<RecoveryCostGap<'_>> {
    use crate::model_graph::{self, HolePolicy, Search, Visit};
    let gaps: BTreeMap<_, _> = model.holes.iter().filter_map(|h| match h.at {
        EntityRef::Function(f) if h.kind.implicit_execution_families().is_some() => Some((f, h)),
        _ => None,
    }).collect();
    if gaps.is_empty() { return None; }
    let mut outgoing: BTreeMap<FunctionId, BTreeSet<FunctionId>> = BTreeMap::new();
    for (from, to) in executions(model).map(|edge| (edge.from, edge.to)) {
        outgoing.entry(from).or_default().insert(to);
    }
    if include_delivery {
        for publish in &model.relations.publishes {
            for sub in &model.relations.subscribes {
                if may_deliver(&model.entities, publish, sub) {
                    outgoing.entry(publish.function).or_default().insert(sub.handler);
                }
            }
        }
    }
    // The represented function graph is finite. No search ceiling may
    // turn an unexplored suffix into a missing gap.
    let found = model_graph::search([root], |f| {
        Visit::<_, (), ()>::edges(outgoing.get(f).into_iter().flatten().map(|to| (*to, ())).collect())
    }, |f| gaps.contains_key(f), |_| false, None, HolePolicy::Halt);
    match found {
        Search::Found { hit, parent } => {
            let mut path = vec![hit];
            let mut at = hit;
            while let Some((from, ())) = parent.get(&at) { path.push(*from); at = *from; }
            path.reverse();
            Some(RecoveryCostGap { hole: gaps[&hit], path })
        }
        _ => None,
    }
}

/// A supplied, type-correct invocation premise over static arranged instances.
/// This does not establish admission, failure dispatch, dynamic identities or
/// incarnations. Parameter type correctness belongs to the premise's producer.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RecoveryInvocation {
    pub function: FunctionId,
    pub receiver: Option<hale_model::LocusInstanceId>,
    pub arguments: Vec<Option<hale_model::LocusInstanceId>>,
}

/// Why an operand cannot be bound to an actual instance. A field's current
/// value is deliberately distinct from an ownership/arrangement path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryBindingGap {
    Unavailable,
    InvalidInvocation,
    UnknownOperand,
    FieldState,
    MissingReceiver,
    ParameterArity,
    OmittedArgument,
    ReceiverType,
}

/// Resolve only immutable roots under the supplied invocation. The premise
/// must later come from failure/call dispatch; this is parameter substitution,
/// not a containment verdict or evidence that a branch executes.
pub fn bind_recovery_operand(
    model: &ApplicationModel,
    invocation: &RecoveryInvocation,
    operand: &hale_model::recovery::RecoveryOperand,
) -> Result<hale_model::LocusInstanceId, RecoveryBindingGap> {
    use hale_model::recovery::RecoveryRoot;
    if invocation.function.index() >= model.entities.functions.len()
        || invocation.receiver.iter().chain(invocation.arguments.iter().flatten())
            .any(|id| id.index() >= model.entities.locus_instances.len())
    { return Err(RecoveryBindingGap::InvalidInvocation); }
    if invocation.receiver.is_some_and(|id| model.entities.functions[invocation.function.index()].owner
        != Some(model.entities.locus_instances[id.index()].decl)) {
        return Err(RecoveryBindingGap::InvalidInvocation);
    }
    if !operand.fields.is_empty() { return Err(RecoveryBindingGap::FieldState); }
    match operand.root {
        RecoveryRoot::CurrentLocus { declaration } => {
            let receiver = invocation.receiver.ok_or(RecoveryBindingGap::MissingReceiver)?;
            if model.entities.functions[invocation.function.index()].owner != Some(declaration)
                || model.entities.locus_instances[receiver.index()].decl != declaration {
                return Err(RecoveryBindingGap::ReceiverType);
            }
            Ok(receiver)
        }
        RecoveryRoot::Parameter { body, ordinal, arity } => {
            if body != EntityRef::Function(invocation.function) || arity as usize != invocation.arguments.len() {
                return Err(RecoveryBindingGap::ParameterArity);
            }
            invocation.arguments.get(ordinal as usize).copied().flatten().ok_or(RecoveryBindingGap::UnknownOperand)
        }
        RecoveryRoot::Unresolved => Err(RecoveryBindingGap::UnknownOperand),
    }
}

/// The operand whose binding is unavailable. Uncertainty on one argument
/// need not erase a known, unrelated recovery operand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryBindingInput { Receiver, Argument(u32) }

/// A call-site operand gap, scoped to its actual parameter position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryCallGap {
    pub input: RecoveryBindingInput,
    pub reason: RecoveryBindingGap,
}

/// One possible helper invocation and the unknowns that prevented fully
/// binding it. Alternatives retain the source typing context index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundRecoveryCall {
    pub invocation: RecoveryInvocation,
    pub context: usize,
    pub gaps: Vec<RecoveryCallGap>,
}

/// Substitute a canonical call's actual operands once. Targets are taken
/// from the existing call relation. An unavailable binding is an error,
/// never an empty successor set. No AST, type-name join or graph traversal.
pub fn bind_recovery_call(
    model: &ApplicationModel,
    call_index: usize,
    caller: &RecoveryInvocation,
) -> Result<Vec<BoundRecoveryCall>, RecoveryBindingGap> {
    let call = model.relations.calls.get(call_index).ok_or(RecoveryBindingGap::InvalidInvocation)?;
    if call.from != caller.function || call.to.index() >= model.entities.functions.len() {
        return Err(RecoveryBindingGap::InvalidInvocation);
    }
    let binding = model.analyses.recovery.calls.as_ref().and_then(|rows| rows.get(call_index))
        .ok_or(RecoveryBindingGap::Unavailable)?;
    if binding.from != call.from || binding.to != call.to || binding.site != call.site
        || binding.provenance != call.provenance || binding.contexts.is_empty() {
        return Err(RecoveryBindingGap::Unavailable);
    }
    if binding.contexts.iter().any(|row| row.arguments.len() > row.parameter_arity as usize) {
        return Err(RecoveryBindingGap::Unavailable);
    }
    Ok(binding.contexts.iter().enumerate().map(|(context, row)| {
        let mut gaps = Vec::new();
        let read = |input, operand: &hale_model::recovery::RecoveryOperand, gaps: &mut Vec<RecoveryCallGap>| {
            match bind_recovery_operand(model, caller, operand) {
                Ok(instance) => Some(instance),
                Err(reason) => { gaps.push(RecoveryCallGap { input, reason }); None }
            }
        };
        let owner = model.entities.functions[call.to.index()].owner;
        let mut receiver = if owner.is_some() {
            match &row.receiver {
                Some(operand) => read(RecoveryBindingInput::Receiver, operand, &mut gaps),
                None => { gaps.push(RecoveryCallGap { input: RecoveryBindingInput::Receiver, reason: RecoveryBindingGap::MissingReceiver }); None }
            }
        } else { None };
        if let (Some(instance), Some(owner)) = (receiver, owner) {
            if model.entities.locus_instances[instance.index()].decl != owner {
                gaps.push(RecoveryCallGap { input: RecoveryBindingInput::Receiver, reason: RecoveryBindingGap::ReceiverType });
                receiver = None;
            }
        }
        let mut arguments: Vec<_> = row.arguments.iter().enumerate().map(|(index, operand)| {
            read(RecoveryBindingInput::Argument(index as u32), operand, &mut gaps)
        }).collect();
        for index in arguments.len()..row.parameter_arity as usize {
            gaps.push(RecoveryCallGap { input: RecoveryBindingInput::Argument(index as u32), reason: RecoveryBindingGap::OmittedArgument });
        }
        arguments.resize(row.parameter_arity as usize, None);
        BoundRecoveryCall { invocation: RecoveryInvocation { function: call.to, receiver, arguments }, context, gaps }
    }).collect())
}

/// Why a construction context cannot supply a handler invocation. A known
/// unhandled failure is distinct from unavailable construction/routing facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryEntryGap {
    Unavailable,
    InvalidContext,
    Unhandled,
    Unknown(hale_model::recovery::RecoveryFailureGap),
}

/// A handler frame conditional on the represented construction occurrence
/// raising a ClosureViolation. This is not evidence that the occurrence is
/// live, its branch executes, or params settlement/admission has completed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConditionalRecoveryEntry {
    pub invocation: RecoveryInvocation,
    pub context: usize,
    pub guarded: bool,
    pub construction_path: Vec<Option<hale_model::ProvenanceId>>,
    pub provenance: hale_model::ProvenanceId,
}

/// Substitute the failing child and actual constructed-field supervisor into
/// the canonical handler identity. Roots, held values and unbound specializations
/// stay explicit; no field path, ownership edge or type name invents a frame.
/// A behavioral consumer must account for EVERY construction context, not select
/// a favorable one, and discharge runtime admission separately.
pub fn recovery_handler_invocation(
    model: &ApplicationModel,
    failed: hale_model::LocusInstanceId,
    context: usize,
) -> Result<ConditionalRecoveryEntry, RecoveryEntryGap> {
    use hale_model::recovery::RecoveryFailureDispatch;
    let entries = model.analyses.recovery.failure_entries.as_ref().ok_or(RecoveryEntryGap::Unavailable)?;
    let row = entries.get(failed.index()).filter(|r| r.failed == failed).ok_or(RecoveryEntryGap::InvalidContext)?;
    let source = row.contexts.get(context).ok_or(RecoveryEntryGap::InvalidContext)?;
    let (supervisor, handler) = match source.dispatch {
        RecoveryFailureDispatch::Handled { supervisor, handler, .. } => (supervisor, handler),
        RecoveryFailureDispatch::Unhandled { .. } => return Err(RecoveryEntryGap::Unhandled),
        RecoveryFailureDispatch::Unknown(gap) => return Err(RecoveryEntryGap::Unknown(gap)),
    };
    let parent = model.entities.locus_instances.get(supervisor.index()).ok_or(RecoveryEntryGap::InvalidContext)?;
    let function = model.entities.functions.get(handler.index()).ok_or(RecoveryEntryGap::InvalidContext)?;
    if function.kind != hale_model::FunctionKind::FailureHandler || function.owner != Some(parent.decl)
        || failed.index() >= model.entities.locus_instances.len() || source.construction.is_none() {
        return Err(RecoveryEntryGap::InvalidContext);
    }
    Ok(ConditionalRecoveryEntry {
        invocation: RecoveryInvocation { function: handler, receiver: Some(supervisor), arguments: vec![Some(failed), None] },
        context, guarded: source.guarded, construction_path: source.construction_path.clone(), provenance: source.provenance,
    })
}


/// Invocation order spaces remain distinct: an initialization is not an
/// authored call and cannot borrow its ordinal or argument bindings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExecutionSite {
    Call(u32),
    ParentProtocol { context: u32, stage: hale_model::ProtocolStage },
    Initialization { constructor: u32, parameter: u32 },
    ArgumentDefault { call: u32, argument: u32 },
    ClosureCheck { check:u32, closure:u32 },
    Lifecycle { constructor:u32, stage:hale_model::LifecycleStage },
}

/// An absorbed entry's call or constructor identity. Deliveries belong to
/// the separate publish namespace and are projected by the bus queries.
pub fn absorption_site(entry: &hale_model::StdlibAbsorption) -> Option<ExecutionSite> {
    if entry.entry_bus.is_some() { return None; }
    if let Some(stage)=entry.entry_lifecycle { return Some(ExecutionSite::Lifecycle { constructor:entry.site,stage }); }
    Some(match &entry.entry_initialization {
        Some(parameter) if parameter.kind == hale_model::InitializerKind::Argument => ExecutionSite::ArgumentDefault { call: entry.site, argument: parameter.ordinal },
        Some(parameter) => ExecutionSite::Initialization { constructor: entry.site, parameter: parameter.ordinal },
        None => ExecutionSite::Call(entry.site),
    })
}

/// A read-only view of the canonical invocation relations.
pub struct ExecutionRef<'a> {
    pub from: FunctionId,
    pub to: FunctionId,
    pub site: ExecutionSite,
    pub in_loop: bool,
    pub unbounded: bool,
    pub provenance: hale_model::ProvenanceId,
    pub dispatch: Option<&'a hale_model::DispatchKind>,
}
impl ExecutionRef<'_> {
    pub fn contracted(&self) -> bool { self.dispatch == Some(&hale_model::DispatchKind::ViaStdlib) }
}

/// The single endpoint projection shared by execution graph consumers.
/// Constructor evaluations retain their typed origin and site namespace.
pub fn executions(model: &ApplicationModel) -> impl Iterator<Item = ExecutionRef<'_>> {
    model.relations.protocol_evaluations.iter().map(|edge| {
        let c = &model.relations.protocol_contexts[edge.context as usize];
        ExecutionRef { from: c.from, to: edge.to, site: ExecutionSite::ParentProtocol { context: edge.context, stage: edge.stage },
            in_loop: c.in_loop, unbounded: c.unbounded, provenance: c.provenance, dispatch: None }
    }).chain(model.relations.lifecycle_evaluations.iter().map(|edge| ExecutionRef {
        from:edge.from,to:edge.to,site:ExecutionSite::Lifecycle { constructor:edge.site,stage:edge.stage },
        in_loop:edge.in_loop,unbounded:edge.unbounded,provenance:edge.provenance,dispatch:None,
    })).chain(model.relations.calls.iter().map(|call| ExecutionRef {
        from: call.from, to: call.to, site: ExecutionSite::Call(call.site),
        in_loop: call.in_loop, unbounded: call.unbounded, provenance: call.provenance, dispatch: Some(&call.dispatch),
    })).chain(model.relations.initializes.iter().map(|edge| ExecutionRef {
        from: edge.from, to: edge.to,
        site: ExecutionSite::Initialization { constructor: edge.site, parameter: edge.member },
        in_loop: edge.in_loop, unbounded: edge.unbounded, provenance: edge.provenance, dispatch: None,
    })).chain(model.relations.argument_evaluations.iter().map(|edge| ExecutionRef {
        from: edge.from, to: edge.to, site: ExecutionSite::ArgumentDefault { call:edge.site,argument:edge.argument },
        in_loop:edge.in_loop,unbounded:edge.unbounded,provenance:edge.provenance,dispatch:None,
    })).chain(model.relations.closure_evaluations.iter().map(|edge| ExecutionRef {
        from:edge.from,to:edge.to,site:ExecutionSite::ClosureCheck { check:edge.site,closure:edge.closure },
        in_loop:edge.in_loop,unbounded:edge.unbounded,provenance:edge.provenance,dispatch:None,
    }))
}

/// Source-template candidates for a guarded lifecycle entry. These rows do
/// not prove admission, scheduling, an occurrence identity or runtime ancestry.
pub struct LifecycleContextCandidates<'a> {
    pub inventory_available: bool,
    pub source_available: bool,
    pub guard: hale_model::LifecycleGuard,
    pub contexts: Vec<(usize, &'a hale_model::construction::LifecycleContext)>,
}

/// Join original constructor identities, never names, spans or source ordinals.
/// Foreign entries retain their user-side constructor identity while their
/// body and declaration identities stay in the foreign analysis namespace.
pub fn lifecycle_entry_contexts(
    model: &ApplicationModel,
    from: FunctionId,
    constructor: u32,
    stage: hale_model::LifecycleStage,
) -> LifecycleContextCandidates<'_> {
    let own = model.relations.lifecycle_evaluations.iter().find(|v|
        v.from==from && v.site==constructor && v.stage==stage);
    let foreign = model.analyses.stdlib_absorption.iter().find(|v|
        v.from==from && v.site==constructor && v.entry_lifecycle==Some(stage));
    let source = own.and_then(|v| v.source).or_else(|| foreign.and_then(|v| v.entry_constructor_source));
    let owner = own.and_then(|v| model.entities.functions[v.to.index()].owner);
    let inventory=model.analyses.lifecycle_contexts.as_ref();
    let contexts=inventory.into_iter().flat_map(|v| v.contexts.iter().enumerate()).filter(|(_,c)|
        source.is_some_and(|source| c.construction_site==Some(hale_model::construction::SiteRef::user(source)))
        && match (owner,&c.declaration.binding) {
            (Some(owner),hale_model::construction::DeclarationBinding::Declared(actual))=>owner==*actual,
            (None,hale_model::construction::DeclarationBinding::Foreign)=>foreign.is_some(),
            _=>false,
        }).collect();
    LifecycleContextCandidates { inventory_available:inventory.is_some(),source_available:source.is_some(),guard:stage.entry_guard(),contexts }
}
