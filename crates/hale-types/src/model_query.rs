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

use std::collections::BTreeSet;

use hale_model::{
    ApplicationModel, Entities, EntityRef, FunctionId, RelationSet,
    SubjectId,
};

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
