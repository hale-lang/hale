//! GH #476 Change 8 — `DispatchPlan`: the typed lowering plan.
//!
//! Which lowering flavor a subject's dispatch gets (direct call,
//! static bucket, dynamic queue) is a CONCLUSION derived from the
//! model, never a model row (`relation.rs`'s rule). This module owns
//! that conclusion: `DispatchPlan::derive(&ApplicationModel)` turns
//! the model's dispatch-gate facts (the trusted BusGraph analysis,
//! bridged through [`crate::application::DispatchGate`] like every
//! other legacy engine) and the Change-8 arrangement (instances,
//! placements, thread domains) into one typed plan per subject —
//! and #464's stage-0 survey question ("how much queued traffic is
//! same-domain?") becomes a field on each row instead of a bespoke
//! topology walk.
//!
//! The plan participates in EXECUTION IDENTITY: `digest()` is folded
//! into the exec digest, so two builds whose dispatch decisions
//! differ can never share a recording identity (#464's
//! boot-resolved-flag rule, applied from day one).

use crate::application::ApplicationModel;

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
    pub fn of(static_eligible: bool, direct_eligible: bool) -> Self {
        if direct_eligible {
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
#[derive(Clone, Debug)]
pub struct SubjectPlan {
    /// The subject key the gates were computed over (the BusGraph's
    /// site spelling).
    pub subject: String,
    pub flavor: DispatchFlavor,
    /// The gate's reason when the flavor is `Dynamic`.
    pub ineligible_reason: Option<String>,
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
#[derive(Clone, Debug, Default)]
pub struct DispatchPlan {
    pub subjects: Vec<SubjectPlan>,
}

impl DispatchPlan {
    /// Derive the plan from the model: gate facts × arrangement.
    pub fn derive(m: &ApplicationModel) -> DispatchPlan {
        // locus display → the domains of its arranged instances.
        let domain_name = |id: crate::ids::ThreadDomainId| {
            m.entities
                .thread_domains
                .get(id.index())
                .map(|d| d.name.clone())
                .unwrap_or_default()
        };
        let mut domains_of: std::collections::BTreeMap<
            &str,
            Vec<String>,
        > = std::collections::BTreeMap::new();
        for re in &m.relations.realizes {
            let Some(decl) = m.entities.loci.get(re.decl.index())
            else {
                continue;
            };
            let domain = m
                .relations
                .placed_in
                .iter()
                .find(|p| p.instance == re.instance)
                .map(|p| domain_name(p.domain));
            if let Some(d) = domain {
                domains_of
                    .entry(decl.display.as_str())
                    .or_default()
                    .push(d);
            }
        }
        // …minus every locus the model admits it does not fully
        // place. A locus can have an arranged instance AND a
        // placement hole at the same time: one `Sub` under `App` on
        // main, another born dynamically inside a pinned locus. The
        // arranged instance would answer "main" for the whole
        // population and manufacture a same-domain claim about a
        // process that has a `Sub` on another thread. A hole hiding
        // OWNS or PLACED at a locus decl therefore DELETES that
        // locus's domain answer — incomplete, not partially known.
        for h in &m.holes {
            if !h.hides.intersects(
                crate::hole::RelationSet::OWNS
                    .union(crate::hole::RelationSet::PLACED),
            ) {
                continue;
            }
            if let crate::ids::EntityRef::LocusDecl(id) = h.at {
                if let Some(decl) = m.entities.loci.get(id.index()) {
                    domains_of.remove(decl.display.as_str());
                }
            }
        }
        DispatchPlan::from_gates(&m.legacy.dispatch_gates, &domains_of)
    }

    /// The plan over raw gate facts plus a locus-display → thread
    /// domains map. `derive` supplies the model's arrangement for
    /// the map; codegen, which holds the merged (user + stdlib,
    /// desugared) bus graph its own lowering must agree with,
    /// supplies its gates and an empty map — flavors depend only on
    /// the gates, so an absent arrangement costs the `same_domain`
    /// survey field and nothing else.
    /// `domains_of` is a COMPLETE account per key: a locus present
    /// in the map has every one of its instances represented, and a
    /// locus the model cannot fully place must be ABSENT (that is
    /// what `derive` does with placement holes). A partial entry
    /// would silently become a same-domain claim.
    pub fn from_gates(
        gates: &[crate::application::DispatchGate],
        domains_of: &std::collections::BTreeMap<&str, Vec<String>>,
    ) -> DispatchPlan {
        let mut subjects: Vec<SubjectPlan> = Vec::new();
        for g in gates {
            let flavor =
                DispatchFlavor::of(g.static_eligible, g.direct_eligible);
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
                subscribers: g.subscribers.clone(),
                publisher_domains,
                subscriber_domains,
                same_domain,
            });
        }
        subjects.sort_by(|a, b| a.subject.cmp(&b.subject));
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
    /// dispatch decisions are part of what a recording pins.
    pub fn digest(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut eat = |bytes: &[u8]| {
            for b in bytes {
                h ^= u64::from(*b);
                h = h.wrapping_mul(0x100000001b3);
            }
        };
        for s in &self.subjects {
            eat(s.subject.as_bytes());
            eat(&[0, s.flavor as u8, u8::from(s.same_domain)]);
            for (l, f) in &s.subscribers {
                eat(l.as_bytes());
                eat(&[1]);
                eat(f.as_bytes());
            }
        }
        h
    }
}
