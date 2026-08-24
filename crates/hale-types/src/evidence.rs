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
        if publish.in_loop {
            return None;
        }
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
                // match; it can never support a bound.
                KeyPredicate::Unknown => return None,
                _ => keyed.push(sub),
            }
        }
        // Candidate key values this site can produce. `None` stands
        // for "some value no filter names", which only a `fallback`
        // receives.
        let mut candidates: Vec<Option<KeyValue>> = Vec::new();
        if keyed.is_empty() {
            candidates.push(None);
        } else {
            match &publish.key_domain {
                Some(KeyDomain::Exact(vals)) => {
                    candidates.extend(vals.iter().cloned().map(Some));
                }
                _ => {
                    for sub in &keyed {
                        if let KeyPredicate::EqLiteral(v) =
                            &sub.key_predicate
                        {
                            candidates.push(Some(v.clone()));
                        }
                        if sub.key_predicate == KeyPredicate::EqReplica {
                            let owner = owner_of(model, sub)?;
                            for i in e
                                .locus_instances
                                .iter()
                                .filter(|i| i.decl == owner)
                            {
                                if let Some(ix) = i.replica {
                                    candidates.push(Some(
                                        KeyValue::Int(ix as i64),
                                    ));
                                }
                            }
                        }
                    }
                    candidates.push(None);
                }
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
                        // Replica indices are unique within a field,
                        // so at most ONE instance answers a key.
                        e.locus_instances
                            .iter()
                            .filter(|i| i.decl == owner)
                            .filter(|i| {
                                i.replica.is_some_and(|ix| ix as i64 == *kv)
                            })
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
        // Direct and interface call sites, grouped by authored
        // ordinal: alternatives of one dispatch share it.
        let mut by_site: BTreeMap<u32, Vec<&hale_model::Call>> =
            BTreeMap::new();
        for c in r.calls.iter().filter(|c| c.from == from) {
            // Through-stdlib rows are the CONTRACTED endpoint pair;
            // their execution multiplicity and dispatch grouping
            // live in the per-entry absorption account below, and
            // counting both would double.
            if c.dispatch == hale_model::DispatchKind::ViaStdlib {
                continue;
            }
            by_site.entry(c.site).or_default().push(c);
        }
        for (_, alts) in by_site {
            let mut best: u64 = 0;
            for c in alts {
                if c.in_loop || c.unbounded {
                    path.pop();
                    return None;
                }
                best = best.max(bail!(fn_fanout(model, c.to, path)));
            }
            total = bail!(total.checked_add(best));
        }
        // Through-stdlib entries, at their AUTHORED site grain: one
        // `StdlibAbsorption` row per entry site, alternatives of one
        // dispatch sharing `entry_group`.
        let mut by_group: BTreeMap<(u32, Option<u32>), Vec<u64>> =
            BTreeMap::new();
        for a in model
            .analyses
            .stdlib_absorption
            .iter()
            .filter(|a| a.from == from)
        {
            if a.entry_in_loop {
                path.pop();
                return None;
            }
            let mut here: u64 = 0;
            for node in &a.nodes {
                for ev in &node.events {
                    match ev {
                        // An interior that publishes, or that the
                        // walk cannot finish, is not countable.
                        hale_model::AbsorbedEvent::Publish { .. }
                        | hale_model::AbsorbedEvent::PublishHole
                        | hale_model::AbsorbedEvent::Truncated
                        | hale_model::AbsorbedEvent::CallHole(_) => {
                            path.pop();
                            return None;
                        }
                        hale_model::AbsorbedEvent::Call {
                            target: hale_model::AbsorbedTarget::User(u),
                            in_loop,
                            ..
                        } => {
                            if *in_loop {
                                path.pop();
                                return None;
                            }
                            let n = bail!(fn_fanout(model, *u, path));
                            here = bail!(here.checked_add(n));
                        }
                        _ => {}
                    }
                }
            }
            // Group by (site, entry_group): a grouped alternative is
            // a choice, a distinct site is another execution.
            by_group
                .entry((a.site, a.entry_group))
                .or_default()
                .push(here);
        }
        // Alternatives of one entry_group take the max; distinct
        // groups and distinct sites sum.
        let mut by_entry_group: BTreeMap<u32, u64> = BTreeMap::new();
        let mut ungrouped: u64 = 0;
        for ((_, group), costs) in by_group {
            let here = costs.into_iter().max().unwrap_or(0);
            match group {
                Some(g) => {
                    let slot = by_entry_group.entry(g).or_insert(0);
                    *slot = (*slot).max(here);
                }
                None => {
                    ungrouped = bail!(ungrouped.checked_add(here))
                }
            }
        }
        total = bail!(total.checked_add(ungrouped));
        for (_, v) in by_entry_group {
            total = bail!(total.checked_add(v));
        }
        path.pop();
        Some(total)
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
