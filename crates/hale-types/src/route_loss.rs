//! `require no_silent_loss(…)` (GH #1327 § 2) — the configuration
//! half: no modeled boundary on a named route is configured to
//! discard silently.
//!
//! A **route** is a wire subject: a topic's, or the subject a
//! literal endpoint names. A boundary is any recorded fact on the
//! route that decides what happens to a message that cannot be
//! served, and each is read from the model, not from source text:
//!
//! | boundary | recorded as | silent when |
//! |---|---|---|
//! | unmatched key | `Topic.key.on_unmatched` | `swallow` |
//! | subscriber queue | `Subscribe.shed` | `drop_old` / `drop_new` |
//! | send | `Publish.disposition` | `or discard` |
//! | binding | `Binding.loss` | `drop` |
//!
//! A refusal the caller observes (`on_unmatched: fail`, a bounded
//! topic's `on_full: fail`, with `or raise` / `or wait` or no
//! disposition at the site) is not a silent discard. A send whose
//! refusal is routed into user code (`or fail …`, `or <handler>`) is
//! **unproven**: the model records that a handler exists, not what
//! it does, and the mere presence of a handler proves little. So is
//! a connect-side binding whose link loss is supervision policy
//! (the publishes during a lost window are dropped) unless every
//! send on the route waits.
//!
//! A wildcard endpoint is a route for every wire it covers: a group
//! subscribing `pay.**` is judged on each modeled address the
//! pattern matches as the bus does (its topics, sends and bindings —
//! an address spelled with `**`, like a topic on `pay.inner.**`,
//! included), and on the pattern itself for the remainder no modeled
//! endpoint names.
//!
//! A publisher the model could not resolve is missing route
//! evidence: a computed send (`subj <- v`) whose locus's declared
//! publish pattern overlaps the route, a body nobody walked, or a
//! stdlib interior that may publish there. Its subject and refusal
//! policy are unknown, so the route is unproven. The declared
//! pattern bounds a computed send — the publish site enforces it — so
//! a send declared under `audit.**` says nothing about `pay.event`.
//!
//! Precedence is the repo's: a concrete silent setting beats a gap
//! in the model (`violated`), a gap beats a false proof
//! (`uncertified`), and only a complete route with nothing silent
//! `holds`. This is the POLICY claim: accepted-message accounting
//! and durability across process failure are out of scope.

use std::collections::{BTreeMap, BTreeSet};

use hale_model::keys::{
    BindingLossBehavior, KeyOnUnmatched, PublishDisposition, ShedPolicy,
};
use hale_model::{ApplicationModel, BindingRole, SubjectId, TransportKind};

/// One boundary setting a claim names: where it is and what is set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// The route, as the author spells it.
    pub route: String,
    /// The boundary's place: `topic T`, `subscriber F`, `send in F`,
    /// `binding`.
    pub at: String,
    /// The setting, e.g. `on_unmatched: swallow`.
    pub setting: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is `{}`", self.at, self.setting)
    }
}

#[derive(Debug)]
pub enum Outcome {
    Holds,
    /// At least one boundary is configured to discard silently.
    Violated(Vec<Finding>),
    /// Nothing is configured silent, but something is unproven: a
    /// handler, a supervision-dependent binding, or a gap in the
    /// model of the route.
    Uncertified(Vec<String>),
}

/// Does an endpoint on `pattern` meet the route `wire`? Set overlap,
/// because a route may itself be a wildcard's remainder; for a
/// concrete wire this is the bus's own match.
fn covers(pattern: &str, wire: &str) -> bool {
    hale_model::subjects_can_overlap(pattern, wire)
}

/// Judge every route in `routes`. `incomplete(subject)` says whether
/// a hole hides the route's subscribers, publishers or delivery.
pub fn judge(
    model: &ApplicationModel,
    routes: &BTreeSet<SubjectId>,
    incomplete: &dyn Fn(SubjectId) -> bool,
) -> Outcome {
    let e = &model.entities;
    let r = &model.relations;
    let fn_disp = |f: hale_model::FunctionId| {
        e.functions[f.index()].display.clone()
    };
    let mut silent: Vec<Finding> = Vec::new();
    let mut unproven: Vec<String> = Vec::new();
    for &s in routes {
        let wire = e.subjects[s.index()].pattern.as_str();
        let topics: Vec<&hale_model::Topic> =
            e.topics.iter().filter(|t| t.subject == s).collect();
        let route = topics
            .first()
            .map(|t| t.display.clone())
            .unwrap_or_else(|| wire.to_string());
        let mut note = |at: String, setting: String| {
            silent.push(Finding {
                route: route.clone(),
                at,
                setting,
            });
        };
        // Topic-level: the unmatched-key policy and the bound's
        // refusal contract. A refusal is only as observable as the
        // sends that meet it, which the send rows below judge.
        for t in &topics {
            if let Some(k) = &t.key {
                if k.on_unmatched == KeyOnUnmatched::Swallow {
                    note(
                        format!("topic `{}` keyed by `{}`", t.display, k.field),
                        "on_unmatched: swallow".to_string(),
                    );
                }
            }
        }
        // Subscriber queues. A wildcard subscription sits on every
        // route under it, so it is named by its OWN spelling and the
        // finding is listed once, however many routes reach it.
        for su in &r.subscribes {
            let pat = e.subjects[su.subject.index()].pattern.as_str();
            if !covers(pat, wire) {
                continue;
            }
            let spelled = e
                .topics
                .iter()
                .find(|t| t.subject == su.subject)
                .map(|t| t.display.clone())
                .unwrap_or_else(|| pat.to_string());
            let shed = match su.shed {
                ShedPolicy::None => continue,
                ShedPolicy::DropOld => "drop_old",
                ShedPolicy::DropNew => "drop_new",
            };
            let bound = match su.capacity {
                hale_model::Capacity::Bounded(n) => format!("bounded({n}), "),
                hale_model::Capacity::Unbounded => String::new(),
            };
            note(
                format!("subscriber `{}` on `{}`", fn_disp(su.handler), spelled),
                format!("{bound}on_full: {shed}"),
            );
        }
        // Sends.
        let mut every_send_waits = true;
        let mut handlers: BTreeSet<String> = BTreeSet::new();
        for p in &r.publishes {
            if e.subjects[p.subject.index()].pattern.as_str() != wire {
                continue;
            }
            match p.disposition {
                PublishDisposition::Discard => note(
                    format!("send in `{}` on `{}`", fn_disp(p.function), route),
                    "or discard".to_string(),
                ),
                PublishDisposition::Handler => {
                    handlers.insert(fn_disp(p.function));
                }
                PublishDisposition::Default | PublishDisposition::Raise => {}
                PublishDisposition::Wait => {}
            }
            if p.disposition != PublishDisposition::Wait {
                every_send_waits = false;
            }
        }
        // Publishers the model could not resolve: a computed send
        // whose declared pattern covers this route, a body nobody
        // walked, a stdlib interior. Their target and refusal policy
        // are unknown, so the route's send account is incomplete.
        for u in crate::model_query::unresolved_publishers(model, wire) {
            use crate::model_query::UnresolvedPublish as U;
            let f = fn_disp(u.function);
            unproven.push(match u.kind {
                U::Computed { patterns } if !patterns.is_empty() => format!(
                    "`{}` publishes to a subject computed at runtime \
                     under its declared `{}`, which covers `{}`; the \
                     model knows neither that send's subject nor its \
                     refusal policy",
                    f,
                    patterns.join("`, `"),
                    route
                ),
                U::Computed { .. } => format!(
                    "`{}` publishes to a subject computed at runtime \
                     that no declared pattern bounds, so it may send on \
                     `{}` under a refusal policy the model does not know",
                    f, route
                ),
                U::Unanalyzed { reason } => format!(
                    "`{}` holds a publish the model did not resolve \
                     ({}), so it may send on `{}` under a refusal policy \
                     the model does not know",
                    f, reason, route
                ),
                U::Absorbed => format!(
                    "`{}` calls into a stdlib interior that may publish \
                     on `{}`, and the model does not record that send's \
                     refusal policy",
                    f, route
                ),
            });
        }
        for f in handlers {
            unproven.push(format!(
                "the send in `{}` on `{}` routes its refusal into a \
                 custom handler, and the model records that a handler \
                 exists, not what it does with the refusal",
                f, route
            ));
        }
        // Bindings.
        for tb in &r.binds {
            let b = &e.bindings[tb.binding.index()];
            if b.subject != s {
                continue;
            }
            let transport = match &b.transport {
                TransportKind::Unix => "unix".to_string(),
                TransportKind::Udp => "udp".to_string(),
                TransportKind::ShmRing => "shm_ring".to_string(),
                TransportKind::Adapter(n) => format!("adapter {n}"),
            };
            let side = match b.role {
                BindingRole::Listen => "listen",
                BindingRole::Connect => "connect",
            };
            match b.loss {
                BindingLossBehavior::Drop => note(
                    format!("{transport} {side} binding of `{route}`"),
                    "loss: drop".to_string(),
                ),
                BindingLossBehavior::WaitCapable if !every_send_waits => {
                    unproven.push(format!(
                        "the {transport} {side} binding of `{route}` marks \
                         its link lost on a send failure and drops the \
                         publishes of that window; whether the loss is \
                         handled is supervision policy the model does not \
                         record, and not every send on the route is \
                         `or wait`"
                    ));
                }
                BindingLossBehavior::WaitCapable
                | BindingLossBehavior::Fail => {}
            }
        }
        if incomplete(s) {
            unproven.push(format!(
                "the boundaries of `{}` are not fully modeled — a hole \
                 hides its subscribers, publishers or delivery",
                route
            ));
        }
    }
    if !silent.is_empty() {
        let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
        silent.retain(|f| seen.insert((f.at.clone(), f.setting.clone())));
        return Outcome::Violated(silent);
    }
    unproven.sort();
    unproven.dedup();
    if !unproven.is_empty() {
        return Outcome::Uncertified(unproven);
    }
    Outcome::Holds
}

/// The routes a group's members publish or subscribe: sends in a
/// member's methods, declared `publish` ends of member loci, and
/// subscription handlers that are members' methods. Member free
/// functions count for their sends.
///
/// A wildcard endpoint (`subscribe "pay.**"`, a declared `publish
/// "pay.**"`) is a route for EVERY wire it covers: each modeled
/// subject the pattern matches joins the set — a concrete one, or a
/// topic's, send's or binding's address even when it is spelled with
/// `**` — so the topics, sends and bindings on those wires are
/// judged. The pattern itself stays in the set for the remainder no
/// modeled endpoint names — what can arrive there is only unresolved
/// publishers, which `judge` asks about by overlap.
pub fn group_routes(
    model: &ApplicationModel,
    loci: &BTreeSet<u32>,
    fns: &BTreeSet<hale_model::FunctionId>,
) -> BTreeSet<SubjectId> {
    let r = &model.relations;
    let locus_of: BTreeMap<hale_model::FunctionId, u32> =
        r.member_of.iter().map(|m| (m.function, m.locus.0)).collect();
    let mine = |f: hale_model::FunctionId| {
        fns.contains(&f)
            || locus_of.get(&f).is_some_and(|l| loci.contains(l))
    };
    let mut out = BTreeSet::new();
    for p in &r.publishes {
        if mine(p.function) {
            out.insert(p.subject);
        }
    }
    for d in &r.declares_publish {
        if loci.contains(&d.locus.0) {
            out.insert(d.subject);
        }
    }
    for su in &r.subscribes {
        if mine(su.handler) {
            out.insert(su.subject);
        }
    }
    // The ADDRESSES a message is sent to: a topic's subject, a
    // send's, a binding's. One may be spelled with `**` (a topic on
    // `pay.inner.**`) and is still a wire the bus delivers on — a
    // `pay.**` subscription receives it — so it is judged as a route,
    // not mistaken for a pattern's remainder.
    let e = &model.entities;
    let addresses: BTreeSet<SubjectId> = e
        .topics
        .iter()
        .map(|t| t.subject)
        .chain(r.publishes.iter().map(|p| p.subject))
        .chain(e.bindings.iter().map(|b| b.subject))
        .collect();
    let patterns: Vec<&str> = out
        .iter()
        .map(|s| e.subjects[s.index()].pattern.as_str())
        .filter(|p| p.contains("**"))
        .collect();
    let covered: Vec<SubjectId> = e
        .subjects
        .iter()
        .enumerate()
        .map(|(i, row)| (SubjectId(i as u32), row.pattern.as_str()))
        .filter(|(id, spelled)| {
            (!spelled.contains("**") || addresses.contains(id))
                // The bus's match: the address is the literal text a
                // send carries, tested against the endpoint's pattern.
                && patterns.iter().any(|p| crate::wildcard_match(p, spelled))
        })
        .map(|(id, _)| id)
        .collect();
    out.extend(covered);
    out
}
