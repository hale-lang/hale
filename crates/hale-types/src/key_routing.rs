//! Who receives a keyed message — the one rule.
//!
//! Two judgments ask this question and must not answer it twice: the
//! fan-out budget ("how many cells does one publish cause?",
//! `evidence::model_fanout`) and the routing-coverage claim ("does
//! every permitted key reach exactly one recipient of a group?",
//! `cover keys(topic T): delivered_to(exactly_one G)`). The rule has
//! three parts, and each is a function here:
//!
//!   * [`classify`] — split the subscriptions that address the subject
//!     into unkeyed and keyed ones, refusing a filter nobody can
//!     evaluate;
//!   * [`scenarios`] — partition the publish's key domain by the
//!     registrations that exist: every key some filter names, plus the
//!     keys nobody names, SYMBOLICALLY (an interval of integers, or
//!     "every other value of the type") rather than one value at a
//!     time;
//!   * [`recipients`] — for one scenario, the registrations that
//!     receive it: unkeyed subscriptions, the keyed ones whose filter
//!     matches, and the `fallback` subscription only when nothing
//!     keyed matched.
//!
//! Every function answers `Err(Unknowable)` where the model cannot
//! count: a filter whose value is not statically known on a locus
//! that has instances, a locus whose population can also be born
//! outside the arrangement, a handler no locus owns. A caller that
//! needs an exact answer treats that as "no answer" — fan-out
//! withdraws its bound, the claim refuses to hold. Unresolved
//! knowledge is never absence.

use hale_model::keys::{KeyDomain, KeyPredicate, KeyValue};
use hale_model::{
    ApplicationModel, FunctionId, LocusDeclId, LocusInstance, Subscribe,
};

/// Why the recipients of a key cannot be counted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unknowable {
    /// A subscription whose `where key == …` value is not statically
    /// known, on a locus that has instances: it may or may not match.
    UnknownPredicate { handler: FunctionId },
    /// A locus that can also be born outside the arrangement (a
    /// runtime birth hole): the listed instances are a lower bound.
    IncompletePopulation { decl: LocusDeclId },
    /// A subscription whose handler belongs to no locus.
    NoOwner { handler: FunctionId },
}

/// The subscriptions that address one subject, split by whether they
/// look at the key.
pub struct Routing<'a> {
    /// Receive regardless of the key.
    pub unkeyed: Vec<&'a Subscribe>,
    /// Literal, replica and `fallback` filters.
    pub keyed: Vec<&'a Subscribe>,
}

/// One class of keys the publish can carry, with the same recipients.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scenario {
    /// One value some registration names (or the domain lists).
    Key(KeyValue),
    /// Values no registration names. `Some` carries the integer
    /// intervals (inclusive, ascending) when the domain is an integer
    /// interval; `None` is "every other value of the key's type", or
    /// "any key" when no subscription looks at the key at all.
    Unnamed(Option<Vec<(i64, i64)>>),
}

/// One registration group that receives a scenario: a subscription,
/// and the instances of its locus that answer it.
pub struct Recipient<'a> {
    pub sub: &'a Subscribe,
    pub owner: LocusDeclId,
    /// Indices into `entities.locus_instances`.
    pub instances: Vec<usize>,
}

impl Recipient<'_> {
    /// How many handler executions this recipient causes per message.
    pub fn runs(&self) -> u64 {
        self.instances.len() as u64
    }
}

/// The locus declaration whose handler this subscription invokes.
pub fn owner_of(
    model: &ApplicationModel,
    sub: &Subscribe,
) -> Option<LocusDeclId> {
    model
        .relations
        .member_of
        .iter()
        .find(|m| m.function == sub.handler)
        .map(|m| m.locus)
}

/// Population of one locus decl, and whether it is EXACT.
///
/// Scoped: a hole matters when it is anchored to THIS locus, never
/// because some unrelated locus is born dynamically. An exact zero is
/// an answer (a declared, never-instantiated subscriber receives
/// nothing); only a relevant hole means unknown.
pub fn population_of(
    model: &ApplicationModel,
    decl: LocusDeclId,
) -> Option<u64> {
    let e = &model.entities;
    let holed = model.holes.iter().any(|h| {
        h.hides.intersects(
            hale_model::RelationSet::OWNS
                .union(hale_model::RelationSet::CARDINALITY),
        ) && matches!(
            h.at,
            hale_model::EntityRef::LocusDecl(l) if l == decl
        )
    });
    if holed {
        return None;
    }
    Some(
        e.locus_instances.iter().filter(|i| i.decl == decl).count()
            as u64,
    )
}

/// The replica key an instance REGISTERS under.
///
/// The model reserves `Some(i)` for an actual `replicas = K` fan-out
/// and leaves an ordinary instance `None`. At runtime an ordinary
/// instance still registers under key 0, so a `where key == replica`
/// subscription on a non-replicated locus receives key-0 messages.
pub fn effective_replica(i: &LocusInstance) -> i64 {
    i.replica.unwrap_or(0) as i64
}

fn owner_or_err(
    model: &ApplicationModel,
    sub: &Subscribe,
) -> Result<LocusDeclId, Unknowable> {
    owner_of(model, sub).ok_or(Unknowable::NoOwner { handler: sub.handler })
}

fn population_or_err(
    model: &ApplicationModel,
    decl: LocusDeclId,
) -> Result<u64, Unknowable> {
    population_of(model, decl)
        .ok_or(Unknowable::IncompletePopulation { decl })
}

/// Split the subscriptions that address the subject. An unknown filter
/// belongs to a registration only if the registration EXISTS: on a
/// locus with no instance it never happens and is dropped.
pub fn classify<'a>(
    model: &ApplicationModel,
    matching: Vec<&'a Subscribe>,
) -> Result<Routing<'a>, Unknowable> {
    let mut unkeyed = Vec::new();
    let mut keyed = Vec::new();
    for sub in matching {
        match &sub.key_predicate {
            KeyPredicate::Any => unkeyed.push(sub),
            KeyPredicate::Unknown => {
                match population_of(model, owner_or_err(model, sub)?) {
                    Some(0) => {}
                    _ => {
                        return Err(Unknowable::UnknownPredicate {
                            handler: sub.handler,
                        })
                    }
                }
            }
            _ => keyed.push(sub),
        }
    }
    Ok(Routing { unkeyed, keyed })
}

/// Partition the key domain into the classes that matter.
///
/// The specific keys that can match are the ones whose registration
/// EXISTS — a filter on a locus with no instances routes nothing —
/// and they are a SET: two declarations naming key 0 cover one value.
/// A type-wide `Bool` domain is enumerated, so no impossible
/// "unmatched" class appears; an integer interval is cut by the named
/// keys into the intervals nobody names; any other domain can produce
/// a value no filter names, so that class is real.
pub fn scenarios(
    model: &ApplicationModel,
    routing: &Routing<'_>,
    domain: Option<&KeyDomain>,
) -> Result<Vec<Scenario>, Unknowable> {
    let e = &model.entities;
    let mut active: Vec<KeyValue> = Vec::new();
    for sub in &routing.keyed {
        match &sub.key_predicate {
            KeyPredicate::EqLiteral(v) => {
                match population_or_err(model, owner_or_err(model, sub)?)? {
                    0 => {}
                    _ => active.push(v.clone()),
                }
            }
            KeyPredicate::EqReplica => {
                let owner = owner_or_err(model, sub)?;
                // A concrete-row count is only a LOWER bound when
                // the population is incomplete.
                population_or_err(model, owner)?;
                for i in e.locus_instances.iter().filter(|i| i.decl == owner)
                {
                    active.push(KeyValue::Int(effective_replica(i)));
                }
            }
            _ => {}
        }
    }
    active.sort();
    active.dedup();
    let mut out: Vec<Scenario> = Vec::new();
    if routing.keyed.is_empty() {
        out.push(Scenario::Unnamed(None));
        return Ok(out);
    }
    match domain {
        // The site's own values ARE the scenarios; a value no filter
        // names simply finds nothing specific and falls back.
        Some(KeyDomain::Exact(vals)) => {
            out.extend(vals.iter().cloned().map(Scenario::Key));
        }
        Some(KeyDomain::IntRange { min, max }) => {
            let (lo, hi) = (*min as i128, *max as i128);
            let mut named: Vec<i64> = Vec::new();
            for v in &active {
                if let KeyValue::Int(k) = v {
                    if k >= min && k <= max {
                        out.push(Scenario::Key(v.clone()));
                        named.push(*k);
                    }
                }
            }
            // `active` is sorted and deduplicated, so `named` is too.
            let mut gaps: Vec<(i64, i64)> = Vec::new();
            let mut next = lo;
            for k in &named {
                let k = *k as i128;
                if k > next {
                    gaps.push((next as i64, (k - 1) as i64));
                }
                next = k + 1;
            }
            if next <= hi {
                gaps.push((next as i64, hi as i64));
            }
            if !gaps.is_empty() {
                out.push(Scenario::Unnamed(Some(gaps)));
            }
        }
        Some(KeyDomain::AnyOfType(t)) if t == "Bool" => {
            out.push(Scenario::Key(KeyValue::Bool(false)));
            out.push(Scenario::Key(KeyValue::Bool(true)));
        }
        _ => {
            out.extend(active.iter().cloned().map(Scenario::Key));
            out.push(Scenario::Unnamed(None));
        }
    }
    if out.is_empty() {
        out.push(Scenario::Unnamed(None));
    }
    Ok(out)
}

/// The registrations that receive a message carrying `key` (`None`:
/// a value no filter names).
pub fn recipients<'a>(
    model: &ApplicationModel,
    routing: &Routing<'a>,
    key: Option<&KeyValue>,
) -> Result<Vec<Recipient<'a>>, Unknowable> {
    let e = &model.entities;
    let all_of = |owner: LocusDeclId| -> Vec<usize> {
        e.locus_instances
            .iter()
            .enumerate()
            .filter(|(_, i)| i.decl == owner)
            .map(|(ix, _)| ix)
            .collect()
    };
    let mut out: Vec<Recipient<'a>> = Vec::new();
    for sub in &routing.unkeyed {
        let owner = owner_or_err(model, sub)?;
        // An EXACT zero annihilates: a declaration with no instance
        // has no runtime registration.
        if population_or_err(model, owner)? > 0 {
            out.push(Recipient { sub, owner, instances: all_of(owner) });
        }
    }
    let mut matched_any = false;
    for sub in &routing.keyed {
        let owner = owner_or_err(model, sub)?;
        let instances = match (&sub.key_predicate, key) {
            (KeyPredicate::EqLiteral(v), Some(kv)) if v == kv => {
                population_or_err(model, owner)?;
                all_of(owner)
            }
            (KeyPredicate::EqReplica, Some(KeyValue::Int(kv))) => {
                population_or_err(model, owner)?;
                // Replica indices are unique within a REPLICATED
                // field, but an ordinary instance registers under
                // the effective key 0, and several ordinary
                // instances of one declaration all do.
                e.locus_instances
                    .iter()
                    .enumerate()
                    .filter(|(_, i)| i.decl == owner)
                    .filter(|(_, i)| effective_replica(i) == *kv)
                    .map(|(ix, _)| ix)
                    .collect()
            }
            // Settled after the others: a fallback receives only
            // what nothing else matched.
            _ => Vec::new(),
        };
        if !instances.is_empty() {
            matched_any = true;
            out.push(Recipient { sub, owner, instances });
        }
    }
    if !matched_any {
        for sub in &routing.keyed {
            if sub.key_predicate == KeyPredicate::Fallback {
                let owner = owner_or_err(model, sub)?;
                if population_or_err(model, owner)? > 0 {
                    out.push(Recipient {
                        sub,
                        owner,
                        instances: all_of(owner),
                    });
                }
            }
        }
    }
    Ok(out)
}
