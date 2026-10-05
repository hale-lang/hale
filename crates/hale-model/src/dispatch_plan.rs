//! GH #476 Change 8 — `DispatchPlan`: the typed lowering plan.
//!
//! Which lowering flavor a subject's dispatch gets (direct call,
//! static bucket, dynamic queue) is a CONCLUSION, never a model row
//! (`relation.rs`'s rule). This module owns that conclusion:
//! [`DispatchPlan::from_gates`] turns the program's dispatch-gate facts
//! (the trusted BusGraph analysis, bridged through
//! [`crate::application::DispatchGate`] like every other legacy engine)
//! and the Change-8 arrangement's thread domains into one typed plan
//! per subject — and #464's stage-0 survey question ("how much queued
//! traffic is same-domain?") becomes a field on each row instead of a
//! bespoke topology walk. A program has one plan: lowering lowers it,
//! and the model holds it projected onto the subjects and loci the
//! model names ([`DispatchPlan::projected`]).
//!
//! The plan participates in EXECUTION IDENTITY: `digest()` is folded
//! into the exec digest, so two builds whose dispatch decisions
//! differ can never share a recording identity (#464's
//! boot-resolved-flag rule, applied from day one).

/// The lowering flavor a subject's dispatch receives.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DispatchFlavor {
    /// Runtime hash-lookup dispatch — the default, and the only
    /// sound choice for anything the gates cannot positively clear.
    Dynamic,
    /// Compile-time subject id + static bucket walk (dispatch still
    /// queued; only the lookup devirtualized).
    StaticBucket,
    /// Synchronous direct calls to every subscriber handler (the
    /// quiet/flat/same-thread tier).
    StaticDirect,
}

impl DispatchFlavor {
    /// THE decision ladder — the single place in the toolchain
    /// where gate booleans become a lowering flavor. Codegen calls
    /// this rather than open-coding `if eligible { .. if direct
    /// { .. } }`, so the plan a recording pins and the plan the
    /// backend emits cannot drift by editing one of two ladders.
    /// The direct tier takes all three legs: same-thread and quiet
    /// (`direct_eligible`) and a flat payload (`payload_flat`); a
    /// direct-eligible subject with a managed payload is a static
    /// bucket, as lowering has always emitted it (F.40 phase 3, P3 3
    /// of 3: the plan used to say `static_direct` for it).
    pub fn of(static_eligible: bool, direct_eligible: bool, payload_flat: bool) -> Self {
        if direct_eligible && payload_flat {
            // `direct_call_eligible` is computed as a REFINEMENT of
            // `eligible` upstream; assert the containment here so a
            // future gate edit that breaks it fails loudly instead
            // of silently promoting an ineligible subject.
            debug_assert!(
                static_eligible,
                "direct-call eligibility must refine static eligibility"
            );
            DispatchFlavor::StaticDirect
        } else if static_eligible {
            DispatchFlavor::StaticBucket
        } else {
            DispatchFlavor::Dynamic
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            DispatchFlavor::Dynamic => "dynamic",
            DispatchFlavor::StaticBucket => "static_bucket",
            DispatchFlavor::StaticDirect => "static_direct",
        }
    }
}

/// One subject's plan row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubjectPlan {
    /// The subject key the gates were computed over (the BusGraph's
    /// site spelling).
    pub subject: String,
    /// The lowering this subject gets. A CONCLUSION derived from the
    /// model, never an authored fact.
    pub flavor: DispatchFlavor,
    /// The gate's reason when the flavor is `Dynamic`.
    pub ineligible_reason: Option<String>,
    /// The gate's `payload_flat` column, the direct tier's third leg.
    /// Not in [`DispatchPlan::digest`]: the flavor it decides is.
    pub payload_flat: bool,
    /// Subscriber (locus, handler) pairs — what the direct lowering
    /// bakes.
    pub subscribers: Vec<(String, String)>,
    /// The thread domains hosting the subject's PUBLISHER loci
    /// (every arranged instance of each publishing locus), sorted +
    /// deduped. Empty when a publisher locus has no arranged
    /// instance (a dynamic birth) — which also forfeits
    /// `same_domain`.
    pub publisher_domains: Vec<String>,
    /// Likewise for subscriber loci.
    pub subscriber_domains: Vec<String>,
    /// #464 stage 0: every publish site and every subscriber of
    /// this subject sits in ONE thread domain — the precondition
    /// for the future same-domain flavors (local queue / widened
    /// direct). Computed conservatively: any unknown domain
    /// (dynamic birth, unarranged locus) forfeits it.
    pub same_domain: bool,
}

/// The whole-program dispatch plan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DispatchPlan {
    /// One plan per subject, in subject order.
    pub subjects: Vec<SubjectPlan>,
}

impl DispatchPlan {
    /// The plan over raw gate facts plus a locus → thread domains map,
    /// keyed by the gates' spelling of a locus (the raw post-merge
    /// symbol a gate's `publisher_loci` and `subscribers` carry).
    /// The one plan of a program is derived here once (F.40 phase 4,
    /// S9): over its one gate set (`hale_types::bus_graph::
    /// derive_dispatch_gates`, the bus graph's rows keyed by wire and
    /// the stdlib's after them) with the [`domain_map`] of the
    /// arrangement projection the model's rows are made from. Lowering
    /// lowers it, and the model holds it [`DispatchPlan::projected`].
    /// The flavor depends only on the gates; the domains fill the
    /// `same_domain` survey column. A locus the map does not hold (a
    /// stdlib locus, which the arrangement places no instance of)
    /// forfeits it.
    /// `domains_of` is a COMPLETE account per key: a locus present
    /// in the map has every one of its instances represented, and a
    /// locus the arrangement cannot fully place must be ABSENT (that
    /// is what [`domain_map`] does with the unplaced). A partial
    /// entry would silently become a same-domain claim.
    pub fn from_gates(
        gates: &[crate::application::DispatchGate],
        domains_of: &std::collections::BTreeMap<&str, Vec<String>>,
    ) -> DispatchPlan {
        let mut subjects: Vec<SubjectPlan> = Vec::new();
        for g in gates {
            let flavor =
                DispatchFlavor::of(g.static_eligible, g.direct_eligible, g.payload_flat);
            let collect = |loci: &[String]| -> (Vec<String>, bool) {
                let mut out: Vec<String> = Vec::new();
                let mut complete = !loci.is_empty();
                for l in loci {
                    match domains_of.get(l.as_str()) {
                        Some(ds) if !ds.is_empty() => {
                            out.extend(ds.iter().cloned())
                        }
                        _ => complete = false,
                    }
                }
                out.sort();
                out.dedup();
                (out, complete)
            };
            let sub_loci: Vec<String> = g
                .subscribers
                .iter()
                .map(|(l, _)| l.clone())
                .collect();
            let (publisher_domains, pubs_complete) =
                collect(&g.publisher_loci);
            let (subscriber_domains, subs_complete) =
                collect(&sub_loci);
            let same_domain = pubs_complete
                && subs_complete
                && publisher_domains.len() == 1
                && publisher_domains == subscriber_domains;
            subjects.push(SubjectPlan {
                subject: g.subject.clone(),
                flavor,
                ineligible_reason: g.ineligible_reason.clone(),
                payload_flat: g.payload_flat,
                subscribers: g.subscribers.clone(),
                publisher_domains,
                subscriber_domains,
                same_domain,
            });
        }
        subjects.sort_by(|a, b| a.subject.cmp(&b.subject));
        DispatchPlan { subjects }
    }

    /// The plan as a reader that names only `subjects` and declares
    /// only `loci` holds it: the model's (F.40 phase 4, S9), which
    /// names the subjects its own bus sites name and declares no stdlib
    /// locus. The rows of those subjects, each with its subscriber
    /// column restricted to those loci, sorted and deduplicated; every
    /// other column is the plan's own, so a row's flavor, reason and
    /// domains are the ones lowering lowers by. The stdlib's `log.**`
    /// row is held only where the program names `log.**` itself, and
    /// then without the stdlib's sinks among its subscribers.
    pub fn projected(
        &self,
        subjects: &std::collections::BTreeSet<&str>,
        loci: &std::collections::BTreeSet<&str>,
    ) -> DispatchPlan {
        let subjects = self
            .subjects
            .iter()
            .filter(|s| subjects.contains(s.subject.as_str()))
            .map(|s| {
                let mut row = s.clone();
                row.subscribers.retain(|(locus, _)| loci.contains(locus.as_str()));
                row.subscribers.sort();
                row.subscribers.dedup();
                row
            })
            .collect();
        DispatchPlan { subjects }
    }

    /// The subjects lowered to the static bucket (or stronger), in
    /// deterministic id order — the codegen id assignment.
    pub fn static_subjects(&self) -> Vec<&SubjectPlan> {
        self.subjects
            .iter()
            .filter(|s| {
                !matches!(s.flavor, DispatchFlavor::Dynamic)
            })
            .collect()
    }

    /// #464 stage 0: (same-domain queued subjects, total subjects).
    /// "Queued" = static-bucket or dynamic — the traffic a future
    /// same-domain local queue would accelerate.
    pub fn same_domain_queued(&self) -> (usize, usize) {
        let same = self
            .subjects
            .iter()
            .filter(|s| {
                s.same_domain
                    && !matches!(
                        s.flavor,
                        DispatchFlavor::StaticDirect
                    )
            })
            .count();
        (same, self.subjects.len())
    }

    /// The plan's identity — folded into the execution digest, so
    /// dispatch decisions are part of what a recording pins. It
    /// covers what lowering reads: each subject, its flavor and its
    /// subscribers.
    pub fn digest(&self) -> u64 {
        let mut h = hale_graph::identity::Fnv64::new();
        let mut eat = |bytes: &[u8]| h.write(bytes);
        for s in &self.subjects {
            eat(s.subject.as_bytes());
            // The third byte is where `same_domain` sat, reserved and
            // always 0: no lowering reads the column, so it is no part
            // of what a build is. GH #464 (the same-domain flavors,
            // which lower by it) is the change that makes it the
            // column again. Lowering's plan carried no domains until
            // F.40 phase 3 (C5), so 0 is the byte every digest already
            // framed.
            eat(&[0, s.flavor as u8, 0]);
            for (l, f) in &s.subscribers {
                eat(l.as_bytes());
                eat(&[1]);
                eat(f.as_bytes());
            }
        }
        h.finish()
    }
}

/// THE domain map the dispatch plan reads (F.40 phase 3, C5): each
/// locus → the thread domains of its arranged instances, from the
/// arrangement's `(locus, domain)` pairs, minus every locus the
/// arrangement does not fully place. A locus can have an arranged
/// instance AND an instance the arrangement cannot see (one `Sub` under
/// `App` on main, another born dynamically inside a pinned locus): the
/// arranged one would answer "main" for the whole population and
/// manufacture a same-domain claim about a process that has a `Sub` on
/// another thread, so an unplaced locus has no answer at all —
/// incomplete, not partially known.
///
/// Keys are the gates' spelling of a locus, the raw post-merge symbol,
/// never a display name. It is fed the arrangement projection the
/// model's arrangement rows are made from (`hale_types::arrangement`).
pub fn domain_map<'a>(
    placed: impl IntoIterator<Item = (&'a str, String)>,
    unplaced: impl IntoIterator<Item = &'a str>,
) -> std::collections::BTreeMap<&'a str, Vec<String>> {
    let mut domains_of: std::collections::BTreeMap<&'a str, Vec<String>> =
        std::collections::BTreeMap::new();
    for (locus, domain) in placed {
        domains_of.entry(locus).or_default().push(domain);
    }
    for locus in unplaced {
        domains_of.remove(locus);
    }
    domains_of
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(same_domain: bool) -> DispatchPlan {
        DispatchPlan {
            subjects: vec![SubjectPlan {
                subject: "evt".to_string(),
                flavor: DispatchFlavor::StaticBucket,
                ineligible_reason: None,
                payload_flat: false,
                subscribers: vec![("Sub".to_string(), "on_evt".to_string())],
                publisher_domains: vec!["main".to_string()],
                subscriber_domains: vec!["main".to_string()],
                same_domain,
            }],
        }
    }

    /// The digest covers what lowering reads; `same_domain` is the
    /// reserved byte until GH #464 lowers by it.
    #[test]
    fn same_domain_is_no_part_of_the_digest() {
        assert_eq!(plan(true).digest(), plan(false).digest());
        let mut other = plan(true);
        other.subjects[0].flavor = DispatchFlavor::Dynamic;
        assert_ne!(other.digest(), plan(true).digest(), "the flavor is");
    }

    /// The projection keeps the named subjects' rows, each subscriber
    /// column over the named loci and sorted, and every other column as
    /// the plan has it.
    #[test]
    fn the_projection_restricts_rows_and_subscribers_only() {
        let mut p = plan(true);
        p.subjects[0].subscribers =
            vec![("Sub".to_string(), "on_evt".to_string()), ("__StdSink".to_string(), "on".to_string()), ("A".to_string(), "h".to_string())];
        let mut other = p.subjects[0].clone();
        other.subject = "log.**".to_string();
        p.subjects.push(other);
        let projected = p.projected(&["evt"].into(), &["A", "Sub"].into());
        assert_eq!(projected.subjects.len(), 1, "the subjects named only");
        let row = &projected.subjects[0];
        assert_eq!(row.subscribers, [("A".to_string(), "h".to_string()), ("Sub".to_string(), "on_evt".to_string())]);
        let mut rest = row.clone();
        rest.subscribers = p.subjects[0].subscribers.clone();
        assert_eq!(rest, p.subjects[0], "every other column is the plan's");
    }
}
