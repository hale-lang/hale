//! GH #476 Change 5e — the certificate-evidence sidecar.
//!
//! `derive_certificate_evidence` runs the certificate engines (the
//! one analysis authority — the grouped report is the same pass
//! `hale check` consumes) and keys each certificate's outcome +
//! diagnostics BY THE ClaimIr ORDINAL it answers. The sidecar
//! lives OUTSIDE the model (a model must not carry a cached prior
//! judgment of itself), and carries the model's `TopologyShapeV1`
//! so a judgment structurally refuses stale evidence.

use std::collections::BTreeMap;

use hale_model::{
    ApplicationModel, CertificateEvidence, ClaimIr, ClaimIrTable,
    EvidenceRow, EvidenceTable, Provenance, ProvenanceId, VerdictIr,
};

use crate::symbol::Bundle;

/// The EVIDENCE-ENGINE SEMANTICS VERSION (review round 4). The
/// package version does not change per commit, so it cannot
/// identify the analysis: two builds can share `CARGO_PKG_VERSION`
/// while differing in effect/witness traversal, allocation-summary
/// behavior, stdlib classification, renaming, or certificate
/// grouping and diagnostic rules.
///
/// CONTRACT: bump this constant in the SAME change as any
/// result-affecting modification to the certificate engines or
/// their inputs — `effects.rs` (grouping, strata, wording),
/// `alloc_summary.rs` / `callgraph.rs` (traversal), `claims.rs`
/// (clause enumeration the lowering shares), or the producer /
/// judgment in this module. The static registries that ARE
/// data (stdlib surface classification, path renames, stdlib
/// source) are hashed in directly, so drifting them does not rely
/// on anyone remembering this constant.
/// v2 (review round 6): cyclic-class certificates now judge
/// Invalid instead of replaying a vacuous Holds, and an undeclared
/// user-class `@budget` dimension judges Invalid instead of
/// Uncertified — evidence produced under v1 semantics must not be
/// replayed by a v2 judgment (or vice versa).
// 3 (GH #476 Change 6 round 8): the producer synthesizes `Holds`
// certificates for implicit lifecycle phases, absent certificate
// reports judge `uncertified` instead of `invalid`, and the
// completeness rule for certificate streams changed — evidence
// from the pre-round-8 engine must not share an inputs_digest
// with this one.
// 4 (GH #476 Change 5h, review rounds 4-7): `@budget` joined the
// evidence pipeline, and its quantitative results MOVED. Fan-out
// went from a reachability set to a weighted execution traversal,
// then from `max(immediate) + union(downstream)` to
// `max over keys of (immediate + downstream)`; interface and
// stdlib alternatives are costed whole and chosen, not merged;
// key domains constrain which scenarios exist; an exact zero
// population annihilates; and the engines resolve cross-seed
// calls. A sidecar produced under v3 can share every other digest
// with this compiler — same source, same model shape, same law and
// coverage digests — while carrying a fan-out verdict this one
// disagrees with, and `validate` treats an equal `inputs_digest`
// as proof of current semantics. It does not hash the
// implementation, so this constant is the identity.
// 5 (review round 8): the fan-out semiring completed. `loop x 0 =
// 0` now reaches the model supplier — a repeated publish or call
// that delivers nothing delivers nothing, where the supplier used
// to refuse before the contribution was known; an unknown filter
// on a locus with no instances routes nothing; and key scenarios
// are built from the DISTINCT ACTIVE routing partition rather than
// from declarations, which both invented impossible `fallback`
// executions (a `Bool` domain is exhausted by its two filters) and
// omitted real ones (two declarations naming one value do not
// cover a two-value interval). Results moved in both directions,
// including a false PASS, so v4 evidence must not be replayed.
// 6 (review round 9): the fan-out supplier equated an UNKNOWN
// subscriber population with no live registration, so a keyed
// subscriber with one arranged instance plus a runtime-birth hole
// dropped out of the routing partition entirely and certified
// fan-out zero over a delivery that definitely happens. Unresolved
// knowledge is never absence. `EqReplica` gained the same
// completeness gate, and a subject-grained CARDINALITY hole now
// withdraws the bound. This corrects a false `Holds`, so v5
// evidence must not be replayed.
pub const ANALYSIS_SEMANTICS_VERSION: u32 = 6;

/// Digest of the certificate engines' inputs OUTSIDE the model:
/// the analysis-semantics version above, the Hale-source stdlib
/// the walks absorb, the stdlib-surface classification registry,
/// the path-rename table, and the compiler version.
/// `TopologyShapeV1` cannot cover these (review rounds 3–4); a
/// judgment recomputes this and refuses evidence produced by a
/// different analysis.
pub fn analysis_inputs_digest() -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x100_0000_01b3);
        }
    };
    eat(&ANALYSIS_SEMANTICS_VERSION.to_le_bytes());
    eat(hale_stdlib::AP_SOURCE.as_bytes());
    eat(env!("CARGO_PKG_VERSION").as_bytes());
    for (segs, mangled) in hale_stdlib::PATH_RENAMES {
        for s in *segs {
            eat(s.as_bytes());
            eat(b"\x1f");
        }
        eat(mangled.as_bytes());
        eat(b"\x1e");
    }
    for surface in crate::stdlib_surface::SURFACES {
        for s in surface.ns {
            eat(s.as_bytes());
            eat(b"\x1f");
        }
        for f in surface.fns {
            eat(f.name.as_bytes());
            eat(&f.effects.0.to_le_bytes());
        }
        for p in surface.open_prefixes {
            eat(p.as_bytes());
            eat(b"\x1f");
        }
        eat(b"\x1e");
    }
    h
}

/// Derive the sidecar for one bundle's lowered law table.
/// The model's answer to "how many subscriber deliveries can
/// this publish site cause?" — shared by the evidence producer
/// and by the differentials, so fan-out supply is never the
/// thing they differ on.
pub fn model_fanout<'a>(
    model: &'a ApplicationModel,
) -> impl Fn(&crate::alloc_summary::FnKey, u32, &str) -> Option<u64> + 'a
{
    // Fan-out is TRANSITIVE AMPLIFICATION: how many subscriber
    // deliveries one invocation can cause, following the bus as far
    // as it goes. `A -> Relay::on_a -> B -> three Sinks` is four
    // deliveries, not one — the ordinary call graph the engine walks
    // never enters a handler through the bus, so this closure has to
    // carry the whole delivery closure itself.
    //
    // Round 2 counted covering `subscribes` rows: a declaration
    // count, so three arranged replicas read as one. Round 3 fixes
    // the two remaining errors — it stopped at the first hop, and it
    // gated on a GLOBAL population account, so one unrelated
    // dynamically-born locus turned every fan-out in the program
    // unbounded.
    let e = &model.entities;
    let r = &model.relations;
    // Population of one locus decl, and whether it is EXACT.
    // Scoped: a hole matters when it is anchored to a locus on THIS
    // closure, never because some unrelated locus is born
    // dynamically.
    fn population_of(
        model: &ApplicationModel,
        decl: hale_model::LocusDeclId,
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
        // Round 5: an EXACT zero is an answer. `LocusInstance` rows
        // are the statically exact population, so a subscriber
        // locus that is declared and never instantiated receives
        // nothing — collapsing that to `None` made an unused
        // declaration turn an exact `@budget(fanout = 0)` into an
        // unbounded failure. Only a RELEVANT hole means unknown,
        // and that is the check above.
        Some(
            e.locus_instances
                .iter()
                .filter(|i| i.decl == decl)
                .count() as u64,
        )
    }
    // Every publish site reachable from a function — its own, plus
    // everything it calls, plus what a stdlib interior publishes on
    // its behalf. A handler's onward amplification is not limited to
    // rows whose `function` IS the handler: a handler that calls a
    // helper which publishes causes those deliveries too.
    //
    // `None` means the answer is not knowable from here (an
    // unfollowable call, a computed publish, an unexplored
    // interior); a loop-nested contributor is `in_loop` and
    // saturates upstream.
    /// The fan-out of ONE message from `publish`, and the fan-out of
    /// one invocation of a function, as a mutually recursive pair.
    ///
    /// Round 6: the previous shape aggregated before it costed —
    /// twice. It took the max over candidate KEYS of the immediate
    /// recipients and then followed the union of every possible
    /// downstream branch; and it took a POINTWISE max over
    /// interface alternatives, keeping one entry per publish site
    /// across alternatives, which is a union rather than a choice.
    /// Neither is the runtime quantity:
    ///
    /// ```text
    ///   max over keys of (immediate(key) + downstream(key))
    ///   max over alternatives of (sum over that alternative)
    /// ```
    ///
    /// not `max(immediate) + union(downstream)` and not
    /// `sum over sites of max over alternatives`. So a whole
    /// SCENARIO — one key, one conformer — is costed end to end
    /// before any max is taken.
    ///
    /// `None` is unbounded-or-unknowable. `path` carries the
    /// publish sites and functions already on this scenario, so a
    /// productive bus cycle or a recursive helper saturates.
    fn message_fanout(
        model: &ApplicationModel,
        publish: &hale_model::Publish,
        path: &mut Vec<(u8, u32, u32)>,
    ) -> Option<u64> {
        use hale_model::keys::{KeyDomain, KeyPredicate, KeyValue};
        let e = &model.entities;
        let r = &model.relations;
        let tag = (0u8, publish.function.0, publish.site);
        if path.contains(&tag) {
            // A productive bus cycle amplifies without bound.
            return None;
        }
        // NOTE: `publish.in_loop` is deliberately NOT checked here.
        // Repeating a publish that delivers to nobody is still
        // nobody — `loop x 0 = 0` is the law the quantitative
        // semiring already states, and refusing before the
        // contribution is known made an exact zero read as
        // unbounded (round 8). The decision is at the bottom.
        // An outbound route delivers to peers this application does
        // not model — scoped to THIS subject, so an unrelated
        // adapter cannot poison the count.
        if crate::model_query::endpoint_incomplete(
            model,
            publish.subject,
            crate::model_query::Direction::Downstream,
        ) {
            return None;
        }
        // …and CARDINALITY, which the shared completeness query
        // deliberately does not consult: `causes:` asks WHICH
        // classes a publish reaches, and another instance of the
        // same locus runs the same handler. Fan-out asks HOW MANY
        // cells receive, so a subject whose subscriber count is
        // unknown has no bound. Scoped to this subject, so an
        // unrelated dynamic endpoint stays irrelevant.
        {
            let wire =
                e.subjects[publish.subject.index()].pattern.as_str();
            let cardinality_unknown = model.holes.iter().any(|h| {
                if !h.hides.intersects(
                    hale_model::RelationSet::CARDINALITY,
                ) {
                    return false;
                }
                match h.at {
                    hale_model::EntityRef::Subject(sid) => {
                        let pat =
                            e.subjects[sid.index()].pattern.as_str();
                        sid == publish.subject
                            || (pat.contains("**")
                                && crate::wildcard_match(pat, wire))
                    }
                    hale_model::EntityRef::Topic(t) => e
                        .topics
                        .get(t.index())
                        .is_some_and(|tp| tp.subject == publish.subject),
                    _ => false,
                }
            });
            if cardinality_unknown {
                return None;
            }
        }
        let matching: Vec<&hale_model::Subscribe> = r
            .subscribes
            .iter()
            .filter(|sub| crate::model_query::may_deliver(e, publish, sub))
            .collect();
        // Unkeyed subscriptions receive regardless of the key.
        let mut unkeyed: Vec<&hale_model::Subscribe> = Vec::new();
        let mut keyed: Vec<&hale_model::Subscribe> = Vec::new();
        for sub in &matching {
            match &sub.key_predicate {
                KeyPredicate::Any => unkeyed.push(sub),
                // A filter whose value is unknown may or may not
                // match — but only if the registration EXISTS.
                // An unknown expression on a locus with no instance
                // belongs to a registration that never happens
                // (round 8).
                KeyPredicate::Unknown => {
                    match population_of(model, owner_of(model, sub)?) {
                        Some(0) => {}
                        _ => return None,
                    }
                }
                _ => keyed.push(sub),
            }
        }
        // Candidate key values this site can produce. `None` stands
        // for "some value no filter names", which only a `fallback`
        // receives.
        //
        // Round 8: scenarios come from the ACTIVE routing
        // partition, not from declarations.
        //
        // The specific keys that can actually match are the ones
        // whose registration EXISTS — a filter on a locus with no
        // instances routes nothing — and they are a SET: two
        // declarations naming key 0 cover one value, not two. Two
        // failures followed from getting this wrong. A type-wide
        // `Bool` domain always got a synthetic "no filter names
        // this" scenario even though `false` and `true` exhaust it,
        // so a `fallback` fired where it never can. And an
        // `IntRange` counted duplicate declarations as distinct
        // coverage, concluding a two-value interval was covered by
        // two same-valued filters and never costing the real
        // unmatched scenario — a false PASS.
        //
        // Round 9: `population_of` has THREE outcomes, and the
        // first version of this collapsed two of them.
        // `Some(0)` is "no registration"; `None` is "the population
        // is not known" — a locus that can also be born outside the
        // arrangement. Treating `None` like `Some(0)` dropped the
        // key from the partition entirely, so a `key == 1`
        // subscriber with one ARRANGED instance plus a
        // runtime-birth hole produced no candidate at all and
        // certified fan-out zero over a delivery that definitely
        // happens. Unresolved knowledge is never absence.
        let mut active: Vec<KeyValue> = Vec::new();
        for sub in &keyed {
            match &sub.key_predicate {
                KeyPredicate::EqLiteral(v) => {
                    match population_of(model, owner_of(model, sub)?) {
                        Some(0) => {}
                        Some(_) => active.push(v.clone()),
                        // This subscription may receive, and how
                        // many cells it has is not knowable.
                        None => return None,
                    }
                }
                KeyPredicate::EqReplica => {
                    let owner = owner_of(model, sub)?;
                    // A concrete-row count is only a LOWER bound
                    // when the population is incomplete: a
                    // dynamically born ordinary instance registers
                    // under effective key 0 and is not listed.
                    if population_of(model, owner).is_none() {
                        return None;
                    }
                    for i in
                        e.locus_instances.iter().filter(|i| i.decl == owner)
                    {
                        active.push(KeyValue::Int(effective_replica(i)));
                    }
                }
                _ => {}
            }
        }
        active.sort();
        active.dedup();
        let mut candidates: Vec<Option<KeyValue>> = Vec::new();
        if keyed.is_empty() {
            candidates.push(None);
        } else {
            match &publish.key_domain {
                // The site's own values ARE the scenarios; a value
                // no filter names simply finds nothing specific and
                // falls back, which the recipient walk handles.
                Some(KeyDomain::Exact(vals)) => {
                    candidates.extend(vals.iter().cloned().map(Some));
                }
                Some(KeyDomain::IntRange { min, max }) => {
                    for v in &active {
                        if let KeyValue::Int(k) = v {
                            if k >= min && k <= max {
                                candidates.push(Some(v.clone()));
                            }
                        }
                    }
                    // One representative of "inside the interval,
                    // named by nobody" — only when such a value
                    // exists.
                    let span =
                        max.saturating_sub(*min).saturating_add(1);
                    let covered = active
                        .iter()
                        .filter(|v| match v {
                            KeyValue::Int(k) => k >= min && k <= max,
                            _ => false,
                        })
                        .count() as i64;
                    if span > covered {
                        candidates.push(None);
                    }
                }
                // A FINITE type-wide domain is enumerable, and
                // enumerating it is what stops an impossible
                // unmatched scenario.
                Some(KeyDomain::AnyOfType(t)) if t == "Bool" => {
                    candidates.push(Some(KeyValue::Bool(false)));
                    candidates.push(Some(KeyValue::Bool(true)));
                }
                // Anything else can produce a value no filter
                // names, so the unmatched scenario is real.
                _ => {
                    candidates.extend(active.iter().cloned().map(Some));
                    candidates.push(None);
                }
            }
            if candidates.is_empty() {
                candidates.push(None);
            }
        }
        path.push(tag);
        let mut best: u64 = 0;
        for k in &candidates {
            // The recipients of THIS key, as (handler, executions).
            let mut recipients: Vec<(hale_model::FunctionId, u64)> =
                Vec::new();
            for sub in &unkeyed {
                let n = population_of(model, owner_of(model, sub)?);
                match n {
                    // Round 7: an EXACT zero annihilates. A
                    // declaration with no instance has no runtime
                    // registration, so its body — however recursive
                    // or unfollowable — causes nothing and must not
                    // withdraw the bound.
                    Some(0) => {}
                    Some(n) => recipients.push((sub.handler, n)),
                    None => {
                        path.pop();
                        return None;
                    }
                }
            }
            let mut matched_any = false;
            for sub in &keyed {
                let n = match (&sub.key_predicate, k) {
                    (KeyPredicate::EqLiteral(v), Some(kv)) if v == kv => {
                        match population_of(model, owner_of(model, sub)?) {
                            Some(n) => n,
                            None => {
                                path.pop();
                                return None;
                            }
                        }
                    }
                    (KeyPredicate::EqReplica, Some(KeyValue::Int(kv))) => {
                        let Some(owner) = owner_of(model, sub) else {
                            path.pop();
                            return None;
                        };
                        // An incomplete population cannot be
                        // counted from the listed rows.
                        if population_of(model, owner).is_none() {
                            path.pop();
                            return None;
                        }
                        // Replica indices are unique within a
                        // REPLICATED field, so at most one instance
                        // answers a key there — but an ordinary
                        // instance registers under the effective
                        // key 0, and several ordinary instances of
                        // one declaration all do.
                        e.locus_instances
                            .iter()
                            .filter(|i| i.decl == owner)
                            .filter(|i| effective_replica(i) == *kv)
                            .count() as u64
                    }
                    // Settled after the others: a fallback receives
                    // only what nothing else matched.
                    _ => 0,
                };
                if n > 0 {
                    matched_any = true;
                    recipients.push((sub.handler, n));
                }
            }
            if !matched_any {
                for sub in &keyed {
                    if sub.key_predicate == KeyPredicate::Fallback {
                        match population_of(model, owner_of(model, sub)?) {
                            Some(n) if n > 0 => {
                                recipients.push((sub.handler, n))
                            }
                            Some(_) => {}
                            None => {
                                path.pop();
                                return None;
                            }
                        }
                    }
                }
            }
            // Cost this whole scenario: the deliveries themselves,
            // plus what exactly those handler executions cause.
            let mut here: u64 = 0;
            for (handler, runs) in recipients {
                if runs == 0 {
                    continue;
                }
                here = match here.checked_add(runs) {
                    Some(v) => v,
                    None => {
                        path.pop();
                        return None;
                    }
                };
                let onward = match fn_fanout(model, handler, path) {
                    Some(v) => v,
                    None => {
                        path.pop();
                        return None;
                    }
                };
                let add = match runs.checked_mul(onward) {
                    Some(v) => v,
                    None => {
                        path.pop();
                        return None;
                    }
                };
                here = match here.checked_add(add) {
                    Some(v) => v,
                    None => {
                        path.pop();
                        return None;
                    }
                };
            }
            best = best.max(here);
        }
        path.pop();
        // Repetition multiplies the contribution: zero stays zero,
        // anything else has no per-call bound.
        if publish.in_loop && best != 0 {
            return None;
        }
        Some(best)
    }

    /// The deliveries ONE invocation of `from` causes — its own
    /// publish sites plus its call tree.
    ///
    /// Ordinary sites SUM. Alternatives of one dispatch (rows
    /// sharing an authored site ordinal, or absorption entries
    /// sharing an `entry_group`) take the MAX of their WHOLE
    /// contribution, because one dispatch runs one conformer.
    fn fn_fanout(
        model: &ApplicationModel,
        from: hale_model::FunctionId,
        path: &mut Vec<(u8, u32, u32)>,
    ) -> Option<u64> {
        let tag = (1u8, from.0, 0);
        if path.contains(&tag) {
            // A recursive publishing helper has no per-call bound.
            return None;
        }
        let r = &model.relations;
        // A call or publish account this function does not know
        // makes its amplification unknowable.
        if model.holes.iter().any(|h| {
            matches!(h.at, hale_model::EntityRef::Function(f) if f == from)
                && h.hides.intersects(
                    hale_model::RelationSet::CALLS
                        .union(hale_model::RelationSet::PUBLISHES),
                )
        }) {
            return None;
        }
        path.push(tag);
        let mut total: u64 = 0;
        macro_rules! bail {
            ($e:expr) => {
                match $e {
                    Some(v) => v,
                    None => {
                        path.pop();
                        return None;
                    }
                }
            };
        }
        for p in r.publishes.iter().filter(|p| p.function == from) {
            let n = bail!(message_fanout(model, p, path));
            total = bail!(total.checked_add(n));
        }
        // ---- one dispatch-site class across BOTH accounts ----
        //
        // Round 7: ordinary rows were summed and absorption entries
        // added afterwards, so an authored site with a user
        // conformer and a stdlib conformer counted BOTH. The model
        // validator defines one authored dispatch-site class across
        // `relations.calls` and `StdlibAbsorption`; one site is one
        // choice, whichever account its alternatives come from.
        //
        // Contribution of one alternative, keyed by authored site.
        let mut by_site: BTreeMap<u32, Vec<u64>> = BTreeMap::new();
        for c in r.calls.iter().filter(|c| c.from == from) {
            // Through-stdlib rows are the CONTRACTED endpoint pair;
            // their multiplicity and grouping live in the per-entry
            // absorption account, and counting both would double.
            if c.dispatch == hale_model::DispatchKind::ViaStdlib {
                continue;
            }
            let n = bail!(fn_fanout(model, c.to, path));
            // `loop x 0 = 0`: a looped call to a callee that
            // delivers nothing still delivers nothing.
            if (c.in_loop || c.unbounded) && n != 0 {
                path.pop();
                return None;
            }
            by_site.entry(c.site).or_default().push(n);
        }
        for a in model
            .analyses
            .stdlib_absorption
            .iter()
            .filter(|a| a.from == from)
        {
            let n = bail!(absorption_fanout(model, a, path));
            if a.entry_in_loop && n != 0 {
                path.pop();
                return None;
            }
            by_site.entry(a.site).or_default().push(n);
        }
        for (_, alts) in by_site {
            // One dispatch runs ONE alternative.
            let best = alts.into_iter().max().unwrap_or(0);
            total = bail!(total.checked_add(best));
        }
        path.pop();
        Some(total)
    }

    /// The deliveries one execution of a stdlib ENTRY causes.
    ///
    /// Round 7: the interior was flattened into a bag — every node
    /// scanned once, `Interior` edges ignored, per-event dispatch
    /// groups ignored, every user re-emergence summed. But
    /// `StdlibAbsorption` deliberately preserves an interior GRAPH,
    /// and its call events carry their own group: the validator
    /// enforces that same-group interior alternatives are one
    /// interface dispatch. `std::http::Router.dispatch` fanning to
    /// two conformers is a CHOICE, and summing both reported three
    /// deliveries where two happen.
    ///
    /// Same semiring as the user call graph: ordinary events sum,
    /// same-group events take the max of whole contributions,
    /// interior cycles saturate.
    fn absorption_fanout(
        model: &ApplicationModel,
        a: &hale_model::StdlibAbsorption,
        path: &mut Vec<(u8, u32, u32)>,
    ) -> Option<u64> {
        fn node_cost(
            model: &ApplicationModel,
            a: &hale_model::StdlibAbsorption,
            idx: u32,
            seen: &mut Vec<u32>,
            path: &mut Vec<(u8, u32, u32)>,
        ) -> Option<u64> {
            // An interior cycle amplifies without bound.
            if seen.contains(&idx) {
                return None;
            }
            let node = a.nodes.get(idx as usize)?;
            seen.push(idx);
            let mut ungrouped: u64 = 0;
            let mut groups: BTreeMap<u32, u64> = BTreeMap::new();
            for ev in &node.events {
                let (cost, group) = match ev {
                    // An interior that publishes, or that the walk
                    // cannot finish, is not countable from here.
                    hale_model::AbsorbedEvent::Publish { .. }
                    | hale_model::AbsorbedEvent::PublishHole
                    | hale_model::AbsorbedEvent::Truncated
                    | hale_model::AbsorbedEvent::CallHole(_) => {
                        seen.pop();
                        return None;
                    }
                    hale_model::AbsorbedEvent::Call {
                        target,
                        in_loop,
                        group,
                        ..
                    } => {
                        let c = match target {
                            hale_model::AbsorbedTarget::Interior(n) => {
                                match node_cost(model, a, *n, seen, path)
                                {
                                    Some(v) => v,
                                    None => {
                                        seen.pop();
                                        return None;
                                    }
                                }
                            }
                            hale_model::AbsorbedTarget::User(u) => {
                                match fn_fanout(model, *u, path) {
                                    Some(v) => v,
                                    None => {
                                        seen.pop();
                                        return None;
                                    }
                                }
                            }
                        };
                        if *in_loop && c != 0 {
                            seen.pop();
                            return None;
                        }
                        (c, *group)
                    }
                };
                match group {
                    Some(g) => {
                        let slot = groups.entry(g).or_insert(0);
                        *slot = (*slot).max(cost);
                    }
                    None => {
                        ungrouped = match ungrouped.checked_add(cost) {
                            Some(v) => v,
                            None => {
                                seen.pop();
                                return None;
                            }
                        }
                    }
                }
            }
            seen.pop();
            let mut total = ungrouped;
            for (_, v) in groups {
                total = total.checked_add(v)?;
            }
            Some(total)
        }
        if a.nodes.is_empty() {
            return Some(0);
        }
        let mut seen: Vec<u32> = Vec::new();
        node_cost(model, a, 0, &mut seen, path)
    }

    /// The replica key an instance REGISTERS under.
    ///
    /// The model reserves `Some(i)` for an actual `replicas = K`
    /// fan-out and leaves an ordinary instance `None`. At runtime
    /// an ordinary instance still registers under key 0, so a
    /// `where key == replica` subscription on a non-replicated
    /// locus receives key-0 messages — and reading only `Some(ix)`
    /// made that recipient invisible to fan-out (round 7).
    fn effective_replica(i: &hale_model::LocusInstance) -> i64 {
        i.replica.unwrap_or(0) as i64
    }

    fn owner_of(
        model: &ApplicationModel,
        sub: &hale_model::Subscribe,
    ) -> Option<hale_model::LocusDeclId> {
        model
            .relations
            .member_of
            .iter()
            .find(|m| m.function == sub.handler)
            .map(|m| m.locus)
    }

    move |key: &crate::alloc_summary::FnKey,
          site: u32,
          subject: &str|
          -> Option<u64> {
        // Join on the RAW canonical name: `FnKey::display` builds
        // the raw `Locus::fn` spelling, while `Function::display` is
        // the DEMANGLED author spelling — an imported publisher
        // would miss (round 4).
        let raw = key.display();
        let fid = e
            .functions
            .iter()
            .position(|f| f.name == raw)
            .map(|i| hale_model::FunctionId(i as u32))?;
        let root = r
            .publishes
            .iter()
            .find(|p| p.function == fid && p.site == site)?;
        // Belt and braces: the row the engine is asking about must
        // address the wire it named.
        let names_it = e.subjects[root.subject.index()].pattern
            == subject
            || e.topics.iter().any(|t| {
                t.subject == root.subject
                    && (t.display == subject || t.name == subject)
            });
        if !names_it {
            return None;
        }
        let mut path: Vec<(u8, u32, u32)> = Vec::new();
        message_fanout(model, root, &mut path)
    }
}

pub fn derive_certificate_evidence(
    bundle: &Bundle<'_>,
    table: &ClaimIrTable,
    model: &ApplicationModel,
) -> EvidenceTable {
    let programs: Vec<&hale_syntax::ast::Program> =
        bundle.programs.values().copied().collect();
    let (_flat, mut groups) = crate::effects::effect_report_grouped(
        &programs,
        &bundle.import_renames,
    );
    // Change 5h: `@budget` joins the same evidence pipeline. The
    // counting engines stay the authority on WHAT they measured —
    // that is an analysis, not a law — and hand over their
    // certificate and their own diagnostics. The VERDICT becomes
    // the judgment's, which is the duplicate authority #476
    // removes: `hale check` and the artifact previously read the
    // engines' answer directly, each in its own way.
    // The `false` is the stdlib-origin flag: the budget engines
    // report at the annotated fn and its own offender sites, all
    // user spans by construction.
    groups.extend(
        crate::budget_check::certificate_groups(
            &programs,
            &bundle.import_renames,
        )
        .into_iter()
        .map(|(row, ds)| {
            (row, ds.into_iter().map(|d| (d, false)).collect())
        }),
    );
    let fanout_of = model_fanout(model);
    groups.extend(
        crate::quantitative::certificate_groups(
            &programs,
            &bundle.import_renames,
            &fanout_of,
        )
        .into_iter()
        .map(|(row, ds)| {
            (row, ds.into_iter().map(|d| (d, false)).collect())
        }),
    );
    let mut out = EvidenceTable {
        model_shape: crate::topology_projection::project_shape_hash(
            model,
        ),
        law_digest: table.semantic_digest(),
        inputs_digest: analysis_inputs_digest(),
        coverage_digest: model.analysis_coverage_digest(),
        ..EvidenceTable::default()
    };
    for sf in &bundle.sources {
        out.provenance.sources.push(
            hale_model::provenance::SourceUnit {
                path: sf.path.clone(),
                digest: sf.digest.clone(),
            },
        );
    }
    let sources = bundle.sources.clone();
    let loc = move |pos: u32| -> Option<(u32, u32)> {
        sources
            .iter()
            .filter(|f| {
                pos >= f.base && pos < f.base.saturating_add(f.len + 1)
            })
            .max_by_key(|f| f.base)
            .map(|f| (f.id, pos - f.base))
    };
    // Evidence multimap (subject display, form) → group indices,
    // consumed in generation order — the ONE place string matching
    // happens; rows key by ordinal from here on.
    let mut by_key: BTreeMap<(String, String), Vec<usize>> =
        BTreeMap::new();
    let demangled: Vec<(String, String)> = groups
        .iter()
        .map(|(row, _)| {
            (
                crate::stdlib_bodies::demangle_str(
                    &row.subject,
                    &bundle.import_renames,
                ),
                crate::stdlib_bodies::demangle_str(
                    &row.form,
                    &bundle.import_renames,
                ),
            )
        })
        .collect();
    for (i, key) in demangled.iter().enumerate() {
        by_key.entry(key.clone()).or_default().push(i);
    }
    let mut cursor: BTreeMap<(String, String), usize> =
        BTreeMap::new();
    // Round 8/9: the certificate engines walk only ANALYZABLE
    // loci, and they emit no certificate for an implicit lifecycle
    // phase with no hook body (`@phase_effects(birth: {})` on a
    // locus that declares no `birth`). No hook body performs no
    // effects, so the truthful certificate for such a phase is a
    // synthetic `Holds` — unanalyzed loci stay report-less and
    // judge `uncertified`. The analyzability fact is the MODEL's
    // (`LocusDecl::analyzable`, set once by the model builder) —
    // this layer never re-walks source.
    let locus_analyzable = |raw: &str| -> bool {
        model
            .entities
            .loci
            .iter()
            .find(|l| l.name == raw)
            .is_some_and(|l| l.analyzable)
    };
    for row in &table.rows {
        let forms = row.certificate_forms();
        if forms.is_empty() {
            continue;
        }
        let subject = match &row.law {
            ClaimIr::EffectForbid { at, .. }
            | ClaimIr::EffectOnly { at, .. }
            | ClaimIr::EffectPublishSet { at, .. }
            | ClaimIr::NoPanic { at }
            // Change 5h: a budget contract's subject is its
            // annotated fn, and the judgment refuses evidence whose
            // subject disagrees — so this list must track the one
            // there exactly.
            | ClaimIr::AllocBudget { at, .. }
            | ClaimIr::QuantBudget { at, .. } => at.0,
            _ => None,
        };
        let mut certs: Vec<CertificateEvidence> = Vec::new();
        for key in forms {
            let idx = by_key.get(&key).and_then(|list| {
                let c = cursor.entry(key.clone()).or_insert(0);
                let i = list.get(*c).copied();
                *c += 1;
                i
            });
            let Some(i) = idx else {
                if let ClaimIr::PhaseEffects { locus, .. } =
                    &row.law
                {
                    if locus_analyzable(locus.1.raw.as_str()) {
                        certs.push(CertificateEvidence {
                            form: key.1.clone(),
                            result: VerdictIr::Holds,
                            diags: Vec::new(),
                        });
                    }
                }
                continue;
            };
            let (cert, ds) = &groups[i];
            let mut diags_out: Vec<(String, ProvenanceId)> =
                Vec::new();
            // The origin flag is authoritative: the emitters tag
            // each diagnostic from the witness step's owning fn
            // (stdlib parses at base 0, so a stdlib span cannot be
            // told from a user span numerically).
            let (mut only_diags, flags): (Vec<_>, Vec<bool>) =
                ds.iter().cloned().unzip();
            crate::stdlib_bodies::demangle_imports(
                &mut only_diags,
                &bundle.import_renames,
            );
            for (d, foreign) in
                only_diags.into_iter().zip(flags)
            {
                let s0 = d.span.start.as_usize() as u32;
                let e0 = d.span.end.as_usize() as u32;
                let pid = ProvenanceId(
                    out.provenance.records.len() as u32,
                );
                let user = if foreign { None } else { loc(s0) };
                match user {
                    Some((src, local)) => {
                        out.provenance.records.push(
                            Provenance::Source {
                                source: hale_model::SourceId(src),
                                span: (
                                    local,
                                    local + e0.saturating_sub(s0),
                                ),
                            },
                        );
                    }
                    None => {
                        // Stdlib parse space (or a sourceless test
                        // bundle) — preserve the span verbatim,
                        // normalized non-inverted.
                        out.provenance.records.push(
                            Provenance::ForeignSpan {
                                span: (s0, e0.max(s0)),
                            },
                        );
                    }
                }
                diags_out.push((d.message, pid));
            }
            certs.push(CertificateEvidence {
                form: demangled[i].1.clone(),
                result: match cert.result {
                    crate::verdict::Verdict::Holds => {
                        VerdictIr::Holds
                    }
                    crate::verdict::Verdict::Violated => {
                        VerdictIr::Violated
                    }
                    crate::verdict::Verdict::Uncertified => {
                        VerdictIr::Uncertified
                    }
                    crate::verdict::Verdict::Invalid => {
                        VerdictIr::Invalid
                    }
                },
                diags: diags_out,
            });
        }
        out.rows.push(EvidenceRow {
            ordinal: row.ordinal,
            subject,
            certs,
        });
    }
    out
}
