//! GH #476 Change 5e — the certificate-evidence sidecar.
//!
//! `derive_certificate_evidence` runs the certificate engines (the
//! one analysis authority — the grouped report is the same pass
//! `hale check` consumes) and keys each certificate's outcome +
//! diagnostics BY THE ClaimIr ORDINAL it answers. The sidecar
//! lives OUTSIDE the model (a model must not carry a cached prior
//! judgment of itself), and carries the model's `TopologyShapeV1`
//! so a judgment structurally refuses stale evidence.

use std::collections::{BTreeMap, BTreeSet};

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
pub const ANALYSIS_SEMANTICS_VERSION: u32 = 3;

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
    // How many instances of `sub`'s owner receive `publish`.
    fn reached_by(
        model: &ApplicationModel,
        publish: &hale_model::Publish,
        sub: &hale_model::Subscribe,
    ) -> Option<u64> {
            let e = &model.entities;
            let r = &model.relations;
            let owner = r
                .member_of
                .iter()
                .find(|m| m.function == sub.handler)
                .map(|m| m.locus)?;
            let total = population_of(model, owner)?;
            if sub.key_predicate
                != hale_model::keys::KeyPredicate::EqReplica
            {
                return Some(total);
            }
            // `where key == replica` narrows to the replica the key
            // selects, when the publish key is statically exact.
            match &publish.key_domain {
                Some(hale_model::keys::KeyDomain::Exact(vals)) => Some(
                    e.locus_instances
                        .iter()
                        .filter(|i| i.decl == owner)
                        .filter(|i| {
                            i.replica.is_some_and(|ix| {
                                vals.iter().any(|v| {
                                    matches!(
                                        v,
                                        hale_model::keys::KeyValue::Int(k)
                                            if *k == ix as i64
                                    )
                                })
                            })
                        })
                        .count() as u64,
                ),
                // An unknown key may select any replica.
                _ => Some(total),
            }
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
    /// How many times each publish site EXECUTES per invocation of
    /// `from` — its own sites plus everything it calls.
    ///
    /// Round 5: this was a reachability set, so a handler calling
    /// one publishing helper TWICE counted the helper's site once.
    /// Model call rows are site-grained precisely because two calls
    /// are two executions. This is the call semiring the budget
    /// engines already use: ordinary sites SUM, alternatives of one
    /// interface dispatch (rows sharing a site ordinal) take the
    /// MAX, and recursion or a loop saturates.
    ///
    /// `None` is unbounded-or-unknowable.
    fn publish_executions(
        model: &ApplicationModel,
        from: hale_model::FunctionId,
        path: &mut Vec<u32>,
    ) -> Option<BTreeMap<(u32, u32), u64>> {
        // A recursive publishing helper has no per-call bound.
        if path.contains(&from.0) {
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
        for a in model
            .analyses
            .stdlib_absorption
            .iter()
            .filter(|a| a.from == from)
        {
            if a.entry_in_loop {
                return None;
            }
            for node in &a.nodes {
                for ev in &node.events {
                    match ev {
                        // An interior that publishes, or that the
                        // walk cannot finish, is not countable from
                        // here.
                        hale_model::AbsorbedEvent::Publish { .. }
                        | hale_model::AbsorbedEvent::PublishHole
                        | hale_model::AbsorbedEvent::Truncated
                        | hale_model::AbsorbedEvent::CallHole(_) => {
                            return None
                        }
                        hale_model::AbsorbedEvent::Call {
                            in_loop, ..
                        } if *in_loop => return None,
                        _ => {}
                    }
                }
            }
        }
        let mut out: BTreeMap<(u32, u32), u64> = BTreeMap::new();
        for p in r.publishes.iter().filter(|p| p.function == from) {
            if p.in_loop {
                return None;
            }
            *out.entry((from.0, p.site)).or_insert(0) += 1;
        }
        path.push(from.0);
        // Group call rows by SITE: alternatives of one interface
        // dispatch share an ordinal and one of them runs, while two
        // calls to the same callee are two ordinals and both run.
        let mut by_site: BTreeMap<u32, Vec<&hale_model::Call>> =
            BTreeMap::new();
        for c in r.calls.iter().filter(|c| c.from == from) {
            by_site.entry(c.site).or_default().push(c);
        }
        for (_, alts) in by_site {
            let mut best: BTreeMap<(u32, u32), u64> = BTreeMap::new();
            for c in alts {
                if c.in_loop || c.unbounded {
                    path.pop();
                    return None;
                }
                let sub = match publish_executions(model, c.to, path) {
                    Some(m) => m,
                    None => {
                        path.pop();
                        return None;
                    }
                };
                for (k, v) in sub {
                    let slot = best.entry(k).or_insert(0);
                    *slot = (*slot).max(v);
                }
            }
            for (k, v) in best {
                *out.entry(k).or_insert(0) += v;
            }
        }
        path.pop();
        Some(out)
    }

    /// The maximum number of subscriber deliveries ONE message from
    /// `publish` can cause.
    ///
    /// Round 5: summing every subscription `may_deliver` admits
    /// counts the UNION of possible recipients, not the recipients
    /// of one message. A message carries ONE key: two disjoint
    /// literal filters cannot both receive it, `where key ==
    /// replica` selects at most the one instance whose index equals
    /// that key, and a `fallback` receives only what no other
    /// filter matched. `may_deliver` is a may-REACH relation — the
    /// right question for `causes:`, and not by itself an answer
    /// about simultaneous cardinality.
    ///
    /// So: unkeyed subscriptions always receive, and over the
    /// candidate key values this site can produce, take the MAX.
    fn deliveries_of(
        model: &ApplicationModel,
        publish: &hale_model::Publish,
    ) -> Option<u64> {
        use hale_model::keys::{KeyDomain, KeyPredicate, KeyValue};
        let e = &model.entities;
        let r = &model.relations;
        let matching: Vec<&hale_model::Subscribe> = r
            .subscribes
            .iter()
            .filter(|sub| crate::model_query::may_deliver(e, publish, sub))
            .collect();
        // Everything that receives regardless of key.
        let mut base: u64 = 0;
        let mut keyed: Vec<&hale_model::Subscribe> = Vec::new();
        for sub in &matching {
            match &sub.key_predicate {
                KeyPredicate::Any => {
                    base = base
                        .checked_add(population_of(model, owner_of(model, sub)?)?)?;
                }
                // A filter whose value is unknown may or may not
                // match this message; it can never support a bound.
                KeyPredicate::Unknown => return None,
                _ => keyed.push(sub),
            }
        }
        if keyed.is_empty() {
            return Some(base);
        }
        // Candidate key values: what this site can produce, else
        // every literal the filters name (plus one value none of
        // them names, which only a `fallback` receives).
        let mut candidates: Vec<Option<KeyValue>> = Vec::new();
        match &publish.key_domain {
            Some(KeyDomain::Exact(vals)) => {
                candidates.extend(vals.iter().cloned().map(Some));
            }
            _ => {
                for sub in &keyed {
                    if let KeyPredicate::EqLiteral(v) = &sub.key_predicate {
                        candidates.push(Some(v.clone()));
                    }
                }
                // `where key == replica` is satisfied by an
                // instance's own index, so each index is a
                // candidate value.
                for sub in &keyed {
                    if sub.key_predicate == KeyPredicate::EqReplica {
                        let owner = owner_of(model, sub)?;
                        for i in e
                            .locus_instances
                            .iter()
                            .filter(|i| i.decl == owner)
                        {
                            if let Some(ix) = i.replica {
                                candidates
                                    .push(Some(KeyValue::Int(ix as i64)));
                            }
                        }
                    }
                }
                // …and "some value no filter names".
                candidates.push(None);
            }
        }
        let mut best: u64 = 0;
        for k in &candidates {
            let mut here: u64 = 0;
            let mut matched_any = false;
            for sub in &keyed {
                let n = match (&sub.key_predicate, k) {
                    (KeyPredicate::EqLiteral(v), Some(k)) if v == k => {
                        population_of(model, owner_of(model, sub)?)?
                    }
                    (KeyPredicate::EqReplica, Some(KeyValue::Int(k))) => {
                        let owner = owner_of(model, sub)?;
                        // Replica indices are unique within a field,
                        // so at most ONE instance answers a key.
                        e.locus_instances
                            .iter()
                            .filter(|i| i.decl == owner)
                            .filter(|i| {
                                i.replica.is_some_and(|ix| ix as i64 == *k)
                            })
                            .count() as u64
                    }
                    // Evaluated after the others: a fallback
                    // receives only what nothing else matched.
                    (KeyPredicate::Fallback, _) => 0,
                    _ => 0,
                };
                if n > 0 {
                    matched_any = true;
                }
                here = here.checked_add(n)?;
            }
            if !matched_any {
                for sub in &keyed {
                    if sub.key_predicate == KeyPredicate::Fallback {
                        here = here.checked_add(population_of(
                            model,
                            owner_of(model, sub)?,
                        )?)?;
                    }
                }
            }
            best = best.max(here);
        }
        base.checked_add(best)
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
        // The site ordinal is the model's own publish-row key.
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
        // A WEIGHTED execution traversal (rounds 4-5). Each work
        // item carries how many handler invocations reached it;
        // downstream contributions multiply by that count, and the
        // handler's own call TREE contributes execution counts
        // rather than a reachable set.
        let mut total: u64 = 0;
        let mut frontier: Vec<(hale_model::Publish, u64)> =
            vec![(root.clone(), 1)];
        // A productive bus cycle is unbounded, not settled: bound
        // the work and saturate rather than terminate quietly.
        let mut steps = 0u32;
        while let Some((p, mult)) = frontier.pop() {
            steps += 1;
            if steps > 4096 {
                return None;
            }
            // A publish inside a loop repeats per invocation.
            if p.in_loop {
                return None;
            }
            // An outbound route delivers to peers this application
            // does not model — scoped to THIS subject, so an
            // unrelated adapter cannot poison the count.
            if crate::model_query::endpoint_incomplete(
                model,
                p.subject,
                crate::model_query::Direction::Downstream,
            ) {
                return None;
            }
            // How many cells ONE message from this site reaches.
            let per_message = deliveries_of(model, &p)?;
            total = total.checked_add(mult.checked_mul(per_message)?)?;
            // …and what those handler EXECUTIONS publish onward.
            // Each reached handler runs once per delivery to it, so
            // its own executions multiply by that count.
            for sub in r.subscribes.iter() {
                if !crate::model_query::may_deliver(e, &p, sub) {
                    continue;
                }
                let reaching = reached_by(model, &p, sub)?;
                let runs = mult.checked_mul(reaching)?;
                if runs == 0 {
                    continue;
                }
                let mut path: Vec<u32> = Vec::new();
                let sites =
                    publish_executions(model, sub.handler, &mut path)?;
                for ((fnid, site), times) in sites {
                    let onward = r
                        .publishes
                        .iter()
                        .find(|q| {
                            q.function.0 == fnid && q.site == site
                        })?
                        .clone();
                    frontier
                        .push((onward, runs.checked_mul(times)?));
                }
            }
        }
        Some(total)
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
