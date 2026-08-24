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
    let addressed = sub.subject == publish.subject || {
        let pat = e.subjects[sub.subject.index()].pattern.as_str();
        let wire = e.subjects[publish.subject.index()].pattern.as_str();
        pat.contains("**") && crate::wildcard_match(pat, wire)
    };
    if !addressed {
        return false;
    }
    use hale_model::keys::{KeyDomain, KeyPredicate, KeyValue};
    match (&publish.key_domain, &sub.key_predicate) {
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
