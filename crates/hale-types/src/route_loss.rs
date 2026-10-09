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

fn covers(pattern: &str, wire: &str) -> bool {
    pattern == wire
        || (pattern.contains("**") && crate::wildcard_match(pattern, wire))
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
        // Subscriber queues.
        for su in &r.subscribes {
            if !covers(e.subjects[su.subject.index()].pattern.as_str(), wire) {
                continue;
            }
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
                format!("subscriber `{}` on `{}`", fn_disp(su.handler), route),
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
    out
}
