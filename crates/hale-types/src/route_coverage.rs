//! Routing coverage and exclusivity — `cover keys(topic T): delivered_to(exactly_one G)`.
//!
//! For every permitted key `k` of a keyed topic, the registrations of
//! group `G` that receive `k` number exactly one: `|R_G(k)| = 1`.
//! Recipients outside `G` (an audit subscriber) are not counted, which
//! is what makes the group the unit of the obligation; they still
//! count toward what a `fallback` subscription receives, because a
//! fallback hears only what no keyed filter, in ANY group, matched.
//!
//! The recipient rule is not here. [`crate::key_routing`] owns it and
//! the fan-out budget calls the same functions; this module only
//! decides which keys are permitted, asks the shared query for their
//! recipients, and counts the ones in the group.
//!
//! Keys are represented symbolically: the permitted domain is cut by
//! the registrations that exist into the keys some filter names and
//! the intervals (or "every other value") nobody names, so a topic
//! keyed by an `Int` is judged in a handful of cases, not 2^64.
//!
//! Fail closed: an answer the model cannot count is `Invalid` with
//! the reason, never `Holds`.

use std::collections::BTreeSet;

use hale_model::keys::{KeyDomain, KeyOnUnmatched, KeyPredicate, KeyValue};
use hale_model::ApplicationModel;

use crate::key_routing::{self, Scenario, Unknowable};

/// What the claim found.
pub enum Outcome {
    Holds,
    /// Keys with no recipient in the group, and keys with several —
    /// each already rendered for the diagnostic.
    Violated {
        gaps: Vec<String>,
        overlaps: Vec<String>,
    },
    /// The claim cannot be decided from the model; the reason says
    /// what to resolve.
    Invalid(String),
}

fn key_text(v: &KeyValue) -> String {
    match v {
        KeyValue::Int(n) => n.to_string(),
        KeyValue::Bool(b) => b.to_string(),
        KeyValue::Str(s) => format!("\"{}\"", s),
        KeyValue::EnumTag(t) => t.clone(),
        KeyValue::Time(n) => format!("{}ns (Time)", n),
        KeyValue::Duration(n) => format!("{}ns", n),
        KeyValue::Decimal { lo, hi } => format!("decimal({}, {})", lo, hi),
    }
}

fn intervals_text(gaps: &[(i64, i64)]) -> String {
    let parts: Vec<String> = gaps
        .iter()
        .map(|(lo, hi)| {
            if lo == hi {
                lo.to_string()
            } else {
                format!("{}..={}", lo, hi)
            }
        })
        .collect();
    format!(
        "{} {}",
        if gaps.len() == 1 && gaps[0].0 == gaps[0].1 {
            "key"
        } else {
            "keys"
        },
        parts.join(", ")
    )
}

fn predicate_text(p: &KeyPredicate) -> String {
    match p {
        KeyPredicate::Any => "unkeyed".to_string(),
        KeyPredicate::EqLiteral(v) => format!("where key == {}", key_text(v)),
        KeyPredicate::EqReplica => "where key == replica".to_string(),
        KeyPredicate::Fallback => "where key == _".to_string(),
        KeyPredicate::Unknown => "where key == <unknown>".to_string(),
    }
}

fn why(model: &ApplicationModel, u: &Unknowable) -> String {
    let e = &model.entities;
    match u {
        Unknowable::UnknownPredicate { handler } => format!(
            "the key filter of `{}` is not statically known (it may or \
             may not receive any given key), so the recipients of a key \
             cannot be counted — write the filter as a literal or \
             `replica`",
            e.functions[handler.index()].display
        ),
        Unknowable::IncompletePopulation { decl } => format!(
            "the instance population of `{}` is incomplete — it can also \
             be born outside the arrangement, so its listed instances are \
             a lower bound, not a count",
            e.loci[decl.index()].display
        ),
        Unknowable::NoOwner { handler } => format!(
            "the subscription handler `{}` belongs to no locus, so its \
             registrations cannot be counted",
            e.functions[handler.index()].display
        ),
    }
}

/// Judge one claim row. `incomplete` is the caller's answer to "is
/// the subscriber set of this topic fully modeled?" (holes on
/// subscriptions, instance counts or key filters).
pub fn judge(
    model: &ApplicationModel,
    topic: usize,
    range: Option<(i64, i64)>,
    group_loci: &BTreeSet<u32>,
    group_name: &str,
    incomplete: bool,
) -> Outcome {
    let e = &model.entities;
    let r = &model.relations;
    let tp = &e.topics[topic];
    let Some(key) = &tp.key else {
        return Outcome::Invalid(format!(
            "topic `{}` is not keyed — routing coverage is a claim about \
             `keyed_by` topics (an unkeyed topic delivers to every \
             subscriber; use `count subscribers` for that)",
            tp.display
        ));
    };
    if incomplete {
        return Outcome::Invalid(format!(
            "the subscriber set of `{}` is not fully modeled — a hole \
             hides subscriptions, instance counts or key filters on it, \
             so exact coverage cannot be decided from the known rows",
            tp.display
        ));
    }
    if group_loci.is_empty() {
        return Outcome::Invalid(format!(
            "group `{}` has no locus members — only loci register \
             subscriptions, so nothing in it can receive a key",
            group_name
        ));
    }
    // The permitted keys: the stated interval, or what the topic's
    // publish sites can produce.
    let site_domains: Vec<&KeyDomain> = r
        .publishes
        .iter()
        .filter(|p| p.subject == tp.subject)
        .filter_map(|p| p.key_domain.as_ref())
        .collect();
    let mut domains: Vec<KeyDomain> = Vec::new();
    match range {
        Some((lo, hi)) => {
            for d in &site_domains {
                let int_ok = match d {
                    KeyDomain::AnyOfType(t) => t == "Int",
                    KeyDomain::IntRange { .. } => true,
                    KeyDomain::Exact(vs) => {
                        vs.iter().all(|v| matches!(v, KeyValue::Int(_)))
                    }
                    KeyDomain::Unknown => true,
                };
                if !int_ok {
                    return Outcome::Invalid(format!(
                        "the interval `{}..={}` states integer keys, but \
                         topic `{}` is keyed by a field of another type",
                        lo, hi, tp.display
                    ));
                }
            }
            if r.subscribes.iter().any(|s| {
                key_routing_addressed(model, s, tp.subject)
                    && matches!(
                        &s.key_predicate,
                        KeyPredicate::EqLiteral(v)
                            if !matches!(v, KeyValue::Int(_))
                    )
            }) {
                return Outcome::Invalid(format!(
                    "the interval `{}..={}` states integer keys, but a \
                     subscription to `{}` filters on a key of another type",
                    lo, hi, tp.display
                ));
            }
            domains.push(KeyDomain::IntRange { min: lo, max: hi });
        }
        None => {
            for d in &site_domains {
                let d = match d {
                    KeyDomain::Unknown => {
                        return Outcome::Invalid(format!(
                            "a publish site of `{}` produces keys the \
                             model knows nothing about — state the \
                             permitted keys with `in LO..=HI`",
                            tp.display
                        ));
                    }
                    // The whole of Int is an interval: gaps are named
                    // as intervals instead of "everything else".
                    KeyDomain::AnyOfType(t) if t == "Int" => {
                        KeyDomain::IntRange { min: i64::MIN, max: i64::MAX }
                    }
                    other => (*other).clone(),
                };
                if !domains.contains(&d) {
                    domains.push(d);
                }
            }
            if domains.is_empty() {
                return Outcome::Invalid(format!(
                    "no publish site of `{}` is known, so the permitted \
                     keys are unknown and a universal over them would \
                     hold vacuously — state them with `in LO..=HI`",
                    tp.display
                ));
            }
        }
    }

    let mut gaps: BTreeSet<String> = BTreeSet::new();
    let mut overlaps: BTreeSet<String> = BTreeSet::new();
    for domain in &domains {
        let dom = Some(domain.clone());
        let matching: Vec<&hale_model::Subscribe> = r
            .subscribes
            .iter()
            .filter(|s| {
                crate::model_query::may_deliver_keys(e, tp.subject, &dom, s)
            })
            .collect();
        let routing = match key_routing::classify(model, matching) {
            Ok(x) => x,
            Err(u) => return Outcome::Invalid(why(model, &u)),
        };
        let scenarios =
            match key_routing::scenarios(model, &routing, dom.as_ref()) {
                Ok(x) => x,
                Err(u) => return Outcome::Invalid(why(model, &u)),
            };
        for sc in &scenarios {
            let (k, label) = match sc {
                Scenario::Key(v) => {
                    (Some(v), format!("key {}", key_text(v)))
                }
                Scenario::Unnamed(Some(iv)) => (None, intervals_text(iv)),
                Scenario::Unnamed(None) => (
                    None,
                    if routing.keyed.is_empty() {
                        "every key (no subscription looks at the key)"
                            .to_string()
                    } else {
                        "every key value no filter names".to_string()
                    },
                ),
            };
            let recs = match key_routing::recipients(model, &routing, k) {
                Ok(x) => x,
                Err(u) => return Outcome::Invalid(why(model, &u)),
            };
            let mut inside: Vec<String> = Vec::new();
            let mut outside: Vec<String> = Vec::new();
            for rec in &recs {
                let handler =
                    e.functions[rec.sub.handler.index()].display.clone();
                if group_loci.contains(&rec.owner.0) {
                    for ix in &rec.instances {
                        inside.push(format!(
                            "`{}` ({}) at `{}`",
                            handler,
                            predicate_text(&rec.sub.key_predicate),
                            e.locus_instances[*ix].path
                        ));
                    }
                } else {
                    outside.push(format!("`{}`", handler));
                }
            }
            match inside.len() {
                1 => {}
                0 => {
                    let policy = match key.on_unmatched {
                        KeyOnUnmatched::Swallow => {
                            "the publish is silently swallowed"
                        }
                        KeyOnUnmatched::Fail => "the publish fails",
                        KeyOnUnmatched::Fallback => {
                            "only the fallback subscriber hears it"
                        }
                    };
                    let mut s = format!(
                        "no registration of `{}` receives {} ({}",
                        group_name, label, policy
                    );
                    if !outside.is_empty() {
                        s.push_str(&format!(
                            "; its recipients lie outside the group: {}",
                            outside.join(", ")
                        ));
                    }
                    s.push(')');
                    gaps.insert(s);
                }
                n => {
                    overlaps.insert(format!(
                        "{} registrations of `{}` receive {}: {}",
                        n,
                        group_name,
                        label,
                        inside.join("; ")
                    ));
                }
            }
        }
    }
    if gaps.is_empty() && overlaps.is_empty() {
        Outcome::Holds
    } else {
        Outcome::Violated {
            gaps: gaps.into_iter().collect(),
            overlaps: overlaps.into_iter().collect(),
        }
    }
}

fn key_routing_addressed(
    model: &ApplicationModel,
    sub: &hale_model::Subscribe,
    subject: hale_model::SubjectId,
) -> bool {
    crate::model_query::subscription_covers(&model.entities, sub, subject)
}
