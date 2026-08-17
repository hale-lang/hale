//! GH #476 Changes 5a–5e — the judgment engine over
//! `ClaimIr` × `ApplicationModel`.
//!
//! Family-by-family migration of the claim evaluators onto the
//! canonical model. Each family lands with DIAGNOSTICS PARITY —
//! the same public spelling, spans, and related notes the
//! authoritative evaluator emits, held byte-equal by a permanent
//! corpus differential — plus negative controls proving the engine
//! reads the model relations it claims to (dropping a family's
//! rows must change its verdicts). The old evaluators in
//! `claims.rs` stay live and authoritative until Change 9 removes
//! the duplicate authorities.
//!
//! 5a: reachability (`forbid reaches`) + holes. The walk reuses
//! `model_graph::search` with a two-kind vertex: user functions
//! (model rows — `calls` at site grain, publish × subscribe
//! composition, `member_of` ∩ the summary universe for groups,
//! `phase_of` for `during`, typed holes for the fail-closed edges)
//! and INTERIOR stdlib vertices from the legacy absorption sidecar
//! (`LegacyProjection::stdlib_absorption`), which preserves the
//! evaluator's BFS layering through stdlib bodies — hole-vs-hit
//! timing and interior witness spellings included.

use std::collections::{BTreeMap, BTreeSet};

use hale_model::{
    AbsorbedEvent, AbsorbedHoleKind, AbsorbedTarget, ApplicationModel,
    ClaimIr, ClaimIrTable, DispatchKind, EntityRef, FunctionId,
    GroupId, HoleKind, Provenance, ProvenanceId, SetIr,
};
use hale_syntax::{Diag, Span};

use crate::model_graph::{self, HolePolicy, Visit};
use crate::verdict::Verdict;

/// One judged law row: the ClaimIr ordinal, the verdict, and the
/// row's diagnostics (validation + evaluation, in the evaluator's
/// per-claim order). Table-level pre-pass diagnostics (duplicate
/// claim names) are returned separately, before every row's.
#[derive(Debug)]
pub struct Judged {
    pub ordinal: u32,
    pub verdict: Verdict,
    pub diags: Vec<Diag>,
}

/// A walk vertex: a user function, or an interior stdlib vertex of
/// one absorption entry.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum V {
    User(FunctionId),
    /// (absorption entry index, node index within it)
    Interior(u32, u32),
}

/// One walk step — enough to render the witness and the related
/// spans identically to the evaluator.
#[derive(Clone, Debug)]
enum StepIr {
    Call {
        /// (interface display, method) when the edge is a dispatch
        /// alternative.
        dispatch: Option<(String, String)>,
        /// The call row's provenance — absent for interior edges,
        /// whose spans the evaluator suppresses anyway
        /// (`is_bundle_fn` gate).
        provenance: Option<ProvenanceId>,
        /// The edge leaves a stdlib body.
        from_stdlib: bool,
    },
    Bus {
        subject: String,
        publish_provenance: Option<ProvenanceId>,
        subscribe_provenance: ProvenanceId,
        from_stdlib: bool,
    },
}

/// Bus-composition hole index (GH #476 rounds 4–6): ONE shared
/// lookup for every judgment that composes publishes with
/// subscribers, over the model's TYPED identities. Topic-grain
/// holes match only a publish's `declared_topic: Option<TopicId>`;
/// subject-grain holes cover only the publish's WIRE pattern (the
/// same exact-or-wildcard predicate known delivery applies: a hole
/// at `audit.**` covers a wire subject `audit.event`). Strings
/// never cross the divide — a literal wire address whose text
/// collides with a topic name is not that topic, and a topic's
/// NAME is not a wire spelling (round 6).
struct BusHoles {
    subject_pats: Vec<(String, hale_model::RelationSet)>,
    topic_holes: BTreeMap<u32, hale_model::RelationSet>,
    /// First set-level spelling whose holes hide PUBLISHES
    /// (round 8): an unknown PUBLISHER can create a bus edge no
    /// walk can see, so every judgment whose projection requires
    /// publish completeness poisons on it — a known
    /// counterexample still wins (the monotonic rule).
    publishers_incomplete: Option<String>,
}

impl BusHoles {
    fn build(model: &ApplicationModel) -> Self {
        let e = &model.entities;
        let mut subject_pats: Vec<(
            String,
            hale_model::RelationSet,
        )> = Vec::new();
        let mut topic_holes: BTreeMap<
            u32,
            hale_model::RelationSet,
        > = BTreeMap::new();
        let mut publishers_incomplete: Option<String> = None;
        for h in &model.holes {
            match h.at {
                EntityRef::Subject(sid) => {
                    let pat =
                        e.subjects[sid.index()].pattern.clone();
                    if h.hides.intersects(
                        hale_model::RelationSet::PUBLISHES,
                    ) && publishers_incomplete.is_none()
                    {
                        publishers_incomplete = Some(pat.clone());
                    }
                    subject_pats.push((pat, h.hides));
                }
                EntityRef::Topic(t) => {
                    if h.hides.intersects(
                        hale_model::RelationSet::PUBLISHES,
                    ) && publishers_incomplete.is_none()
                    {
                        publishers_incomplete = Some(
                            e.topics[t.index()].name.clone(),
                        );
                    }
                    let e2 = topic_holes
                        .entry(t.0)
                        .or_insert(hale_model::RelationSet(0));
                    *e2 = e2.union(h.hides);
                }
                _ => {}
            }
        }
        BusHoles {
            subject_pats,
            topic_holes,
            publishers_incomplete,
        }
    }

    fn blocks(
        &self,
        mask: hale_model::RelationSet,
        declared_topic: Option<u32>,
        wire: Option<&str>,
    ) -> bool {
        declared_topic.is_some_and(|t| {
            self.topic_holes
                .get(&t)
                .is_some_and(|m| m.intersects(mask))
        }) || wire.is_some_and(|w| {
            self.subject_pats.iter().any(|(pat, m)| {
                m.intersects(mask)
                    && (pat == w
                        || (pat.contains("**")
                            && crate::wildcard_match(pat, w)))
            })
        })
    }
}

fn span_of(
    prov: &hale_model::ProvenanceTable,
    bases: &[u32],
    pid: ProvenanceId,
) -> Span {
    match prov.records.get(pid.index()) {
        Some(Provenance::Source { source, span }) => {
            let base = bases.get(source.index()).copied().unwrap_or(0);
            Span::new(
                (base + span.0) as usize,
                (base + span.1) as usize,
            )
        }
        Some(Provenance::ForeignSpan { span }) => {
            Span::new(span.0 as usize, span.1 as usize)
        }
        _ => Span::new(0, 0),
    }
}

/// Judge every `forbid reaches` row of one lowered law table
/// against its model (GH #476 Change 5a). Returns the table-level
/// pre-pass diagnostics (duplicate claim names — the evaluator
/// emits them before any validation or evaluation) and the judged
/// rows.
pub fn judge_forbid_reaches(
    table: &ClaimIrTable,
    model: &ApplicationModel,
    source_bases: &[u32],
) -> (Vec<Diag>, Vec<Judged>) {
    let e = &model.entities;
    let r = &model.relations;
    let claim_span = |pid: ProvenanceId| -> Span {
        span_of(&table.provenance, source_bases, pid)
    };
    let model_span = |pid: ProvenanceId| -> Span {
        span_of(&model.provenance, source_bases, pid)
    };
    let absorption = &model.legacy.stdlib_absorption;
    let display = |v: V| -> String {
        match v {
            V::User(f) => e.functions[f.index()].display.clone(),
            V::Interior(a, n) => absorption[a as usize].nodes
                [n as usize]
                .display
                .clone(),
        }
    };

    // The summary universe: only these functions are walk vertices.
    let v1: BTreeSet<FunctionId> =
        model.legacy.topology_v1_fns.iter().copied().collect();

    // Group → fn-grain projection (the evaluator's `fn_set`):
    // member loci project through the SUMMARY universe (v1), but a
    // member FREE fn enters unconditionally — an empty free fn with
    // no summary row still exists as a source/sink decl (the
    // evaluator inserts every named free fn after the summary
    // loop; review: declaration-only free functions).
    let mut group_fns: BTreeMap<GroupId, BTreeSet<FunctionId>> =
        BTreeMap::new();
    {
        let mut by_locus: BTreeMap<u32, Vec<FunctionId>> =
            BTreeMap::new();
        for mo in &r.member_of {
            by_locus.entry(mo.locus.0).or_default().push(mo.function);
        }
        for gm in &r.group_members {
            let set = group_fns.entry(gm.group).or_default();
            match gm.member {
                EntityRef::LocusDecl(l) => {
                    for f in by_locus.get(&l.0).into_iter().flatten() {
                        if v1.contains(f) {
                            set.insert(*f);
                        }
                    }
                }
                EntityRef::Function(f) => {
                    set.insert(f);
                }
                _ => {}
            }
        }
        for (gi, _) in e.groups.iter().enumerate() {
            group_fns.entry(GroupId(gi as u32)).or_default();
        }
    }
    // The evaluator iterates fn sets in FnKey order — free fns
    // (locus: None) BEFORE methods — and search halts at the first
    // hole, so seeding order is verdict-relevant.
    let fnkey_sorted = |set: &BTreeSet<FunctionId>| -> Vec<FunctionId> {
        let mut v: Vec<FunctionId> = set.iter().copied().collect();
        v.sort_by_key(|f| {
            let raw = &e.functions[f.index()].name;
            match raw.rsplit_once("::") {
                Some((l, m)) => (1u8, l.to_string(), m.to_string()),
                None => (0u8, String::new(), raw.clone()),
            }
        });
        v
    };

    // Per-fn direct call rows by authored site (ViaStdlib rows are
    // the legacy contraction — the absorption replaces them here).
    let mut calls_of: BTreeMap<
        FunctionId,
        Vec<(u32, FunctionId, StepIr)>,
    > = BTreeMap::new();
    for c in &r.calls {
        let step = match &c.dispatch {
            DispatchKind::Direct => StepIr::Call {
                dispatch: None,
                provenance: Some(c.provenance),
                from_stdlib: false,
            },
            DispatchKind::Interface { interface } => {
                let iface_disp = e
                    .interfaces
                    .iter()
                    .find(|i| i.name == *interface)
                    .map(|i| i.display.clone())
                    .unwrap_or_else(|| interface.clone());
                let method = e.functions[c.to.index()]
                    .name
                    .rsplit("::")
                    .next()
                    .unwrap_or_default()
                    .to_string();
                StepIr::Call {
                    dispatch: Some((iface_disp, method)),
                    provenance: Some(c.provenance),
                    from_stdlib: false,
                }
            }
            DispatchKind::ViaStdlib => continue,
        };
        calls_of.entry(c.from).or_default().push((c.site, c.to, step));
    }
    for v in calls_of.values_mut() {
        v.sort_by_key(|(site, to, _)| (*site, *to));
    }
    // (from fn) → absorption entries (site, index).
    let mut absorb_of: BTreeMap<FunctionId, Vec<(u32, u32)>> =
        BTreeMap::new();
    for (ai, a) in absorption.iter().enumerate() {
        absorb_of
            .entry(a.from)
            .or_default()
            .push((a.site, ai as u32));
    }
    // Subscription rows grouped by their subject PATTERN, sorted —
    // the evaluator's subscribers_of iterates the graph's subject
    // map in key order and matches exact-or-wildcard.
    // Keyed by the subscription's CANONICAL spelling — the topic
    // NAME for declared subscriptions, the authored pattern for
    // literal/wildcard ones — exactly the graph keying the
    // evaluator's subscribers_of iterates.
    let mut subs_by_pattern: BTreeMap<
        &str,
        Vec<(FunctionId, ProvenanceId)>,
    > = BTreeMap::new();
    for su in &r.subscribes {
        let key = match su.declared_topic {
            Some(t) => e.topics[t.index()].name.as_str(),
            None => e.subjects[su.subject.index()].pattern.as_str(),
        };
        subs_by_pattern
            .entry(key)
            .or_default()
            .push((su.handler, su.provenance));
    }
    let subs_for = |written: &str| -> Vec<(FunctionId, ProvenanceId)> {
        let mut out = Vec::new();
        for (pattern, subs) in &subs_by_pattern {
            let covers = *pattern == written
                || (pattern.contains("**")
                    && crate::wildcard_match(pattern, written));
            if covers {
                out.extend(subs.iter().cloned());
            }
        }
        out
    };
    // fn → publish rows (site, subject id, written text, prov).
    let mut pubs_of: BTreeMap<
        FunctionId,
        Vec<(u32, u32, Option<u32>, String, ProvenanceId)>,
    > = BTreeMap::new();
    for p in &r.publishes {
        let written = match p.declared_topic {
            Some(t) => e.topics[t.index()].name.clone(),
            None => e.subjects[p.subject.index()].pattern.clone(),
        };
        pubs_of.entry(p.function).or_default().push((
            p.site,
            p.subject.0,
            p.declared_topic.map(|t| t.0),
            written,
            p.provenance,
        ));
    }
    for v in pubs_of.values_mut() {
        v.sort_by_key(|(site, ..)| *site);
    }
    // fn → fail-closed holes. Each retains its ORIGINAL `hides`
    // mask: relevance is decided per consultation site against the
    // relation family that site walks (via_calls → CALLS, via_bus →
    // PUBLISHES, an effects(C) destination → EFFECTS); the KIND
    // only selects the diagnostic wording — a new hole species
    // cannot be silently ignored, and a family the walk does not
    // need cannot poison it (review round 2).
    #[derive(Clone)]
    enum FnHole {
        Indirect,
        Untyped { callee: String },
        Computed,
        /// Any other species (e.g. an unanalyzed body) — fails
        /// closed with its recorded reason.
        Other { reason: String },
    }
    let mut holes_of: BTreeMap<
        FunctionId,
        Vec<(hale_model::RelationSet, Option<u32>, FnHole)>,
    > = BTreeMap::new();
    // Holes anchored at NON-function entities participate too —
    // subject grain (covering patterns, round 4) AND topic grain
    // (round 5) — through the ONE shared BusHoles index.
    let bus_holes = BusHoles::build(model);
    for h in &model.holes {
        match h.at {
            EntityRef::Function(f) => {
                let walk_families = hale_model::RelationSet::CALLS
                    .union(hale_model::RelationSet::PUBLISHES)
                    .union(hale_model::RelationSet::EFFECTS);
                if !h.hides.intersects(walk_families) {
                    continue;
                }
                let hole = match &h.kind {
                    HoleKind::IndirectCall => FnHole::Indirect,
                    HoleKind::UntypedReceiver { callee } => {
                        FnHole::Untyped {
                            callee: callee.clone(),
                        }
                    }
                    HoleKind::ComputedSubject => FnHole::Computed,
                    _ => FnHole::Other {
                        reason: h.reason.clone(),
                    },
                };
                holes_of.entry(f).or_default().push((
                    h.hides,
                    h.authored_site,
                    hole,
                ));
            }
            _ => {}
        }
    }
    // The evaluator refuses at the FIRST unresolved site in source
    // order — hole selection within a space is by authored site,
    // never by the model's canonical (kind, reason) sort (review
    // round 3).
    let earliest_hole = |f: &FunctionId,
                         mask: hale_model::RelationSet|
     -> Option<&FnHole> {
        holes_of
            .get(f)
            .into_iter()
            .flatten()
            .filter(|(hides, _, _)| hides.intersects(mask))
            .min_by_key(|(_, site, _)| site.unwrap_or(u32::MAX))
            .map(|(_, _, h)| h)
    };
    // fn → phase name (during filter).
    let mut phase_of: BTreeMap<FunctionId, &str> = BTreeMap::new();
    for po in &r.phase_of {
        phase_of.insert(
            po.function,
            e.phases[po.phase.index()].name.as_str(),
        );
    }
    let direct_of = |f: FunctionId| -> &[String] {
        &e.functions[f.index()].direct_effects
    };
    let class_atoms = |name: &str| -> BTreeSet<String> {
        match e.effect_classes.iter().find(|c| c.name == name) {
            Some(c) => match &c.definition {
                hale_model::EffectClassDefinition::Composed {
                    atoms,
                } => atoms.iter().cloned().collect(),
                hale_model::EffectClassDefinition::Atomic => {
                    [name.to_string()].into_iter().collect()
                }
                hale_model::EffectClassDefinition::InvalidCycle => {
                    BTreeSet::new()
                }
            },
            None => [name.to_string()].into_iter().collect(),
        }
    };

    // ---- claim names are the contract-of-record (pre-pass over
    // every claims-block row, ALL families — the evaluator checks
    // before any validation or evaluation). Dup rows still
    // evaluate. ----
    let mut pre: Vec<Diag> = Vec::new();
    {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for row in &table.rows {
            if !matches!(
                row.origin,
                hale_model::ClaimOrigin::Main
                    | hale_model::ClaimOrigin::Constitution { .. }
                    | hale_model::ClaimOrigin::Library { .. }
            ) {
                continue;
            }
            if !seen.insert(row.name.as_str()) {
                pre.push(Diag::ty(
                    claim_span(row.provenance),
                    format!(
                        "claim `{}` is declared more than once — the name \
                         is the contract-of-record and must be unique",
                        row.name
                    ),
                ));
            }
        }
    }

    let mut out = Vec::new();
    for row in &table.rows {
        let ClaimIr::ForbidReaches {
            src,
            dst,
            via_calls,
            via_bus,
            during,
            avoiding,
        } = &row.law
        else {
            continue;
        };
        let mut diags: Vec<Diag> = Vec::new();
        // ---- the evaluator's validation pass ----
        let mut ok = true;
        let group_decl_names: Vec<&str> =
            e.groups.iter().map(|g| g.display.as_str()).collect();
        let check_group = |gref: &hale_model::GroupRef,
                               diags: &mut Vec<Diag>|
         -> bool {
            if gref.group.is_some() {
                return true;
            }
            let mut near: Vec<&&str> = group_decl_names
                .iter()
                .filter(|g| {
                    crate::effects::close(g, &gref.name.display)
                })
                .collect();
            near.sort();
            let hint = match near.first() {
                Some(n) => format!(" Did you mean `{}`?", n),
                None => String::new(),
            };
            diags.push(Diag::ty(
                claim_span(gref.provenance),
                format!(
                    "claim `{}` names group `{}`, which is never declared. \
                     Add `group {} = {{ … }};` at the top level.{}",
                    row.name,
                    gref.name.display,
                    gref.name.display,
                    hint
                ),
            ));
            false
        };
        let SetIr::Group(src_ref) = src else {
            if let SetIr::EffectCarriers(c) = src {
                diags.push(Diag::ty(
                    claim_span(c.provenance),
                    format!(
                        "claim `{}`: `effects(...)` is only valid \
                         in target position — sources must be \
                         declared groups",
                        row.name
                    ),
                ));
            }
            out.push(Judged {
                ordinal: row.ordinal,
                verdict: Verdict::Invalid,
                diags,
            });
            continue;
        };
        ok &= check_group(src_ref, &mut diags);
        match dst {
            SetIr::Group(g) => {
                ok &= check_group(g, &mut diags);
            }
            SetIr::EffectCarriers(c) => {
                // A cyclic definition resolves to no effect — a
                // prohibition over it would hold vacuously. Invalid
                // BEFORE evaluation (review: invalid cycles).
                if let Some(id) = c.class {
                    if matches!(
                        e.effect_classes[id.index()].definition,
                        hale_model::EffectClassDefinition::InvalidCycle
                    ) {
                        diags.push(Diag::ty(
                            claim_span(c.provenance),
                            format!(
                                "claim `{}`: effect class `{}` is defined in \
                                 terms of itself. A cyclic definition resolves \
                                 to no effect at all, so every contract naming \
                                 it would hold vacuously.",
                                row.name, c.name
                            ),
                        ));
                        ok = false;
                    }
                }
                if !c.builtin
                    && c.class.map_or(true, |id| {
                        !e.effect_classes[id.index()].declared
                    })
                {
                    let mut near: Vec<&String> = e
                        .effect_classes
                        .iter()
                        .filter(|ec| ec.declared)
                        .map(|ec| &ec.name)
                        .filter(|n| crate::effects::close(n, &c.name))
                        .collect();
                    near.sort();
                    let hint = match near.first() {
                        Some(n) => format!(" Did you mean `{}`?", n),
                        None => String::new(),
                    };
                    diags.push(Diag::ty(
                        claim_span(c.provenance),
                        format!(
                            "claim `{}` names effect class `{}`, which is never \
                             declared. Add `effect {};` at the top level.{}",
                            row.name, c.name, c.name, hint
                        ),
                    ));
                    ok = false;
                }
            }
        }
        if let Some(a) = avoiding {
            ok &= check_group(a, &mut diags);
            if let Some(av_gid) = a.group {
                let members = |g: GroupId| -> BTreeSet<&EntityRef> {
                    r.group_members
                        .iter()
                        .filter(|gm| gm.group == g)
                        .filter(|gm| {
                            matches!(
                                gm.member,
                                EntityRef::LocusDecl(_)
                                    | EntityRef::Function(_)
                            )
                        })
                        .map(|gm| &gm.member)
                        .collect()
                };
                let av_members = members(av_gid);
                for set in [src, dst] {
                    let SetIr::Group(n) = set else { continue };
                    let Some(gid) = n.group else { continue };
                    if !av_members.is_disjoint(&members(gid)) {
                        diags.push(Diag::ty(
                            claim_span(a.provenance),
                            format!(
                                "claim `{}`: `avoiding {}` overlaps \
                                 `{}` — masking an endpoint makes \
                                 the claim weaker than it reads (a \
                                 masked target holds vacuously; a \
                                 masked source drops roots). Make \
                                 the gate disjoint from the \
                                 endpoints",
                                row.name,
                                a.name.display,
                                n.name.display
                            ),
                        ));
                        ok = false;
                    }
                }
            }
        }
        if !ok {
            out.push(Judged {
                ordinal: row.ordinal,
                verdict: Verdict::Invalid,
                diags,
            });
            continue;
        }
        let Some(src_gid) = src_ref.group else {
            unreachable!("refused by the validation pass above")
        };
        // ---- projection vacuity ----
        let decl_count = |g: GroupId| {
            r.group_members.iter().filter(|gm| gm.group == g).count()
        };
        let vacuous = |gid: GroupId,
                           gref: &hale_model::GroupRef,
                           which: &str,
                           diags: &mut Vec<Diag>|
         -> bool {
            if decl_count(gid) == 0 || !group_fns[&gid].is_empty() {
                return false;
            }
            diags.push(Diag::ty(
                claim_span(gref.provenance),
                format!(
                    "claim `{}`: group `{}` projects to no executable {} \
                     vertices — its declarations have no fns, so the claim \
                     proves nothing about them. The fn-grained walk cannot \
                     see pure-data access; name the loci that HOLD the \
                     behavior, or drop the claim",
                    row.name, gref.name.display, which
                ),
            ));
            true
        };
        if vacuous(src_gid, src_ref, "source", &mut diags) {
            out.push(Judged {
                ordinal: row.ordinal,
                verdict: Verdict::Invalid,
                diags,
            });
            continue;
        }
        if let SetIr::Group(dst_ref) = dst {
            if let Some(dst_gid) = dst_ref.group {
                if vacuous(dst_gid, dst_ref, "target", &mut diags) {
                    out.push(Judged {
                        ordinal: row.ordinal,
                        verdict: Verdict::Invalid,
                        diags,
                    });
                    continue;
                }
            }
        }
        // ---- roots, during, mask, dst ----
        let mut roots: BTreeSet<FunctionId> =
            group_fns[&src_gid].clone();
        if let Some(p) = during {
            roots.retain(|f| {
                phase_of.get(f).is_some_and(|ph| *ph == p.name)
            });
            if roots.is_empty() && !group_fns[&src_gid].is_empty() {
                diags.push(Diag::ty(
                    claim_span(p.provenance),
                    format!(
                        "claim `{}`: phase `{}` names nothing in group \
                         `{}` — no member locus declares it. A claim over \
                         an empty phase holds vacuously",
                        row.name, p.name, src_ref.name.display
                    ),
                ));
                out.push(Judged {
                    ordinal: row.ordinal,
                    verdict: Verdict::Invalid,
                    diags,
                });
                continue;
            }
        }
        let mask: Option<&BTreeSet<FunctionId>> = avoiding
            .as_ref()
            .and_then(|a| a.group.map(|g| &group_fns[&g]));
        enum DstIr<'x> {
            Group(&'x BTreeSet<FunctionId>),
            Effects(BTreeSet<String>),
        }
        let dst_test = match dst {
            SetIr::Group(g) => DstIr::Group(
                &group_fns[&g.group.expect("validated above")],
            ),
            SetIr::EffectCarriers(c) => {
                DstIr::Effects(class_atoms(&c.name))
            }
        };
        if let DstIr::Group(g) = &dst_test {
            if g.is_empty() {
                out.push(Judged {
                    ordinal: row.ordinal,
                    verdict: Verdict::Holds,
                    diags,
                });
                continue;
            }
        }
        let row_span = claim_span(row.provenance);
        // ---- the walk ----
        // Absorption truncation is SATURATION, not an ordinary
        // hole: the evaluator maps step-ceiling exhaustion to
        // Violated, so the distinct signal must survive the search
        // (review round 2: byte-identical message, wrong verdict).
        let mut truncated = false;
        // A SET-LEVEL subscriber hole has no authored position
        // relative to the known subscriber rows, so it cannot
        // preempt them (round 6): known edges still compose — a
        // known counterexample stays Violated — and the flag only
        // downgrades a would-be Holds to Uncertified after the
        // search.
        let mut bus_unknown: Option<(String, String)> = None;
        let ordered_roots = fnkey_sorted(&roots);
        let search = model_graph::search(
            ordered_roots.iter().map(|f| V::User(*f)),
            |v: &V| {
                let mut edges: Vec<(V, StepIr)> = Vec::new();
                match v {
                    V::User(f) => {
                        if *via_calls {
                            if let Some(h) = earliest_hole(
                                f,
                                hale_model::RelationSet::CALLS,
                            ) {
                                match h {
                                    FnHole::Indirect => {
                                        diags.push(Diag::ty(
                                            row_span,
                                            format!(
                                                "claim `{}` cannot be certified: \
                                                 `{}` (reachable from `{}`) calls \
                                                 through a function-typed \
                                                 parameter, whose target is not \
                                                 knowable statically. An \
                                                 unresolvable edge fails closed",
                                                row.name,
                                                display(*v),
                                                src_ref.name.display
                                            ),
                                        ));
                                        return Visit::hole(());
                                    }
                                    FnHole::Untyped { callee } => {
                                        diags.push(Diag::ty(
                                            row_span,
                                            format!(
                                                "claim `{}` cannot be certified: \
                                                 `{}` (reachable from `{}`) calls \
                                                 `{}` on a receiver the compiler \
                                                 cannot type, so the walk cannot \
                                                 follow the edge. An unresolvable \
                                                 edge fails closed — bind the \
                                                 receiver to a typed field or \
                                                 local so the call resolves",
                                                row.name,
                                                display(*v),
                                                src_ref.name.display,
                                                callee
                                            ),
                                        ));
                                        return Visit::hole(());
                                    }
                                    FnHole::Computed => {
                                        diags.push(Diag::ty(
                                            row_span,
                                            format!(
                                                "claim `{}` cannot be certified: `{}` \
                                                 (reachable from `{}`) publishes to a \
                                                 computed subject, which could route to \
                                                 any subscriber. An unresolvable edge \
                                                 fails closed",
                                                row.name,
                                                display(*v),
                                                src_ref.name.display
                                            ),
                                        ));
                                        return Visit::hole(());
                                    }
                                    FnHole::Other { reason } => {
                                        diags.push(Diag::ty(
                                            row_span,
                                            format!(
                                                "claim `{}` cannot be certified: \
                                                 `{}` (reachable from `{}`) — {}. \
                                                 An unresolvable edge fails closed",
                                                row.name,
                                                display(*v),
                                                src_ref.name.display,
                                                reason
                                            ),
                                        ));
                                        return Visit::hole(());
                                    }
                                }
                            }
                            let direct = calls_of
                                .get(f)
                                .map(|x| x.as_slice())
                                .unwrap_or(&[]);
                            let absorbed = absorb_of
                                .get(f)
                                .map(|x| x.as_slice())
                                .unwrap_or(&[]);
                            let mut items: Vec<(u32, u8, usize)> =
                                Vec::new();
                            for (i, (site, _, _)) in
                                direct.iter().enumerate()
                            {
                                items.push((*site, 0, i));
                            }
                            for (i, (site, _)) in
                                absorbed.iter().enumerate()
                            {
                                items.push((*site, 1, i));
                            }
                            items.sort();
                            for (_, kind, i) in items {
                                if kind == 0 {
                                    let (_, to, step) = &direct[i];
                                    edges.push((
                                        V::User(*to),
                                        step.clone(),
                                    ));
                                } else {
                                    let (_, ai) = absorbed[i];
                                    let a = &absorption[ai as usize];
                                    edges.push((
                                        V::Interior(ai, 0),
                                        StepIr::Call {
                                            dispatch: a
                                                .entry_dispatch
                                                .clone(),
                                            // The AUTHORED entry
                                            // call — a hit at an
                                            // interior node crosses
                                            // here, and the
                                            // evaluator points at
                                            // it.
                                            provenance: Some(
                                                a.entry_provenance,
                                            ),
                                            from_stdlib: false,
                                        },
                                    ));
                                }
                            }
                        }
                        if *via_bus {
                            if let Some(h) = earliest_hole(
                                f,
                                hale_model::RelationSet::PUBLISHES,
                            ) {
                                let msg = match h {
                                    FnHole::Computed => format!(
                                        "claim `{}` cannot be certified: `{}` \
                                         (reachable from `{}`) publishes to a \
                                         computed subject, which could route to \
                                         any subscriber. An unresolvable edge \
                                         fails closed",
                                        row.name,
                                        display(*v),
                                        src_ref.name.display
                                    ),
                                    FnHole::Untyped { callee } => format!(
                                        "claim `{}` cannot be certified: \
                                         `{}` (reachable from `{}`) calls \
                                         `{}` on a receiver the compiler \
                                         cannot type, so the walk cannot \
                                         follow the edge. An unresolvable \
                                         edge fails closed — bind the \
                                         receiver to a typed field or \
                                         local so the call resolves",
                                        row.name,
                                        display(*v),
                                        src_ref.name.display,
                                        callee
                                    ),
                                    FnHole::Indirect => format!(
                                        "claim `{}` cannot be certified: \
                                         `{}` (reachable from `{}`) calls \
                                         through a function-typed \
                                         parameter, whose target is not \
                                         knowable statically. An \
                                         unresolvable edge fails closed",
                                        row.name,
                                        display(*v),
                                        src_ref.name.display
                                    ),
                                    FnHole::Other { reason } => format!(
                                        "claim `{}` cannot be certified: \
                                         `{}` (reachable from `{}`) — {}. \
                                         An unresolvable edge fails closed",
                                        row.name,
                                        display(*v),
                                        src_ref.name.display,
                                        reason
                                    ),
                                };
                                diags.push(Diag::ty(row_span, msg));
                                return Visit::hole(());
                            }
                            for (_, sid, topic, written, ppid) in
                                pubs_of.get(f).into_iter().flatten()
                            {
                                // A hole covering this publish's
                                // TYPED identity — its declared
                                // topic or its wire subject — means
                                // the known subscriber rows are a
                                // lower bound. Set-level holes have
                                // no authored position, so they
                                // defer rather than halt
                                // (rounds 3–6).
                                if bus_holes.blocks(
                                    hale_model::RelationSet::SUBSCRIBES,
                                    *topic,
                                    Some(
                                        e.subjects[*sid as usize]
                                            .pattern
                                            .as_str(),
                                    ),
                                ) && bus_unknown.is_none()
                                {
                                    bus_unknown = Some((
                                        display(*v),
                                        written.clone(),
                                    ));
                                }
                                for (handler, spid) in
                                    subs_for(written)
                                {
                                    edges.push((
                                        V::User(handler),
                                        StepIr::Bus {
                                            subject: written.clone(),
                                            publish_provenance: Some(
                                                *ppid,
                                            ),
                                            subscribe_provenance:
                                                spid,
                                            from_stdlib: false,
                                        },
                                    ));
                                }
                            }
                        }
                        // An effects(C) destination NEEDS this
                        // vertex's EFFECTS rows — a hole hiding
                        // them means the known rows are not the
                        // whole story, so exhausting them must not
                        // conclude Holds (review round 2).
                        if matches!(dst_test, DstIr::Effects(_)) {
                            if let Some(h) = earliest_hole(
                                f,
                                hale_model::RelationSet::EFFECTS,
                            ) {
                                let why = match h {
                                    FnHole::Other { reason } => {
                                        reason.clone()
                                    }
                                    FnHole::Untyped { callee } => {
                                        format!(
                                            "calls `{}` on a receiver \
                                             the compiler cannot type",
                                            callee
                                        )
                                    }
                                    FnHole::Indirect => {
                                        "calls through a \
                                         function-typed parameter"
                                            .to_string()
                                    }
                                    FnHole::Computed => {
                                        "publishes to a computed \
                                         subject"
                                            .to_string()
                                    }
                                };
                                diags.push(Diag::ty(
                                    row_span,
                                    format!(
                                        "claim `{}` cannot be certified: \
                                         the effects of `{}` (reachable \
                                         from `{}`) are not fully \
                                         analyzable — {}. Unknown effects \
                                         fail closed",
                                        row.name,
                                        display(*v),
                                        src_ref.name.display,
                                        why
                                    ),
                                ));
                                return Visit::hole(());
                            }
                        }
                        Visit::edges(edges)
                    }
                    V::Interior(ai, ni) => {
                        let node = &absorption[*ai as usize].nodes
                            [*ni as usize];
                        for ev in &node.events {
                            match ev {
                                AbsorbedEvent::Call {
                                    target,
                                    dispatch,
                                    ..
                                } => {
                                    if !*via_calls {
                                        continue;
                                    }
                                    let tv = match target {
                                        AbsorbedTarget::Interior(
                                            n2,
                                        ) => V::Interior(*ai, *n2),
                                        AbsorbedTarget::User(f2) => {
                                            V::User(*f2)
                                        }
                                    };
                                    edges.push((
                                        tv,
                                        StepIr::Call {
                                            dispatch: dispatch
                                                .clone(),
                                            provenance: None,
                                            from_stdlib: true,
                                        },
                                    ));
                                }
                                AbsorbedEvent::CallHole(k) => {
                                    if !*via_calls {
                                        continue;
                                    }
                                    match k {
                                        AbsorbedHoleKind::IndirectCall => {
                                            diags.push(Diag::ty(
                                                row_span,
                                                format!(
                                                    "claim `{}` cannot be certified: \
                                                     `{}` (reachable from `{}`) calls \
                                                     through a function-typed \
                                                     parameter, whose target is not \
                                                     knowable statically. An \
                                                     unresolvable edge fails closed",
                                                    row.name,
                                                    display(*v),
                                                    src_ref
                                                        .name
                                                        .display
                                                ),
                                            ));
                                        }
                                        AbsorbedHoleKind::OpaqueCall { callee } => {
                                            diags.push(Diag::ty(
                                                row_span,
                                                format!(
                                                    "claim `{}` cannot be certified: \
                                                     `{}` (reachable from `{}`) calls \
                                                     `{}` on a receiver the compiler \
                                                     cannot type, so the walk cannot \
                                                     follow the edge. An unresolvable \
                                                     edge fails closed — bind the \
                                                     receiver to a typed field or \
                                                     local so the call resolves",
                                                    row.name,
                                                    display(*v),
                                                    src_ref
                                                        .name
                                                        .display,
                                                    callee
                                                ),
                                            ));
                                        }
                                    }
                                    return Visit::hole(());
                                }
                                AbsorbedEvent::Publish {
                                    subject,
                                    declared_topic,
                                    ..
                                } => {
                                    if !*via_bus {
                                        continue;
                                    }
                                    {
                                        // Typed identity: a
                                        // declared topic's wire is
                                        // its subject's pattern; a
                                        // literal's wire is its own
                                        // text (round 6).
                                        let wire = match declared_topic {
                                            Some(t) => e.subjects
                                                [e.topics
                                                    [t.index()]
                                                .subject
                                                .index()]
                                            .pattern
                                            .as_str(),
                                            None => subject.as_str(),
                                        };
                                        if bus_holes.blocks(
                                            hale_model::RelationSet::SUBSCRIBES,
                                            declared_topic
                                                .map(|t| t.0),
                                            Some(wire),
                                        ) && bus_unknown.is_none()
                                        {
                                            bus_unknown = Some((
                                                display(*v),
                                                subject.clone(),
                                            ));
                                        }
                                        for (handler, spid) in
                                            subs_for(subject)
                                        {
                                            edges.push((
                                                V::User(handler),
                                                StepIr::Bus {
                                                    subject: subject
                                                        .clone(),
                                                    publish_provenance:
                                                        None,
                                                    subscribe_provenance:
                                                        spid,
                                                    from_stdlib: true,
                                                },
                                            ));
                                        }
                                    }
                                }
                                AbsorbedEvent::PublishHole => {
                                    if !*via_bus {
                                        continue;
                                    }
                                    diags.push(Diag::ty(
                                        row_span,
                                        format!(
                                            "claim `{}` cannot be certified: `{}` \
                                             (reachable from `{}`) publishes to a \
                                             computed subject, which could route to \
                                             any subscriber. An unresolvable edge \
                                             fails closed",
                                            row.name,
                                            display(*v),
                                            src_ref.name.display
                                        ),
                                    ));
                                    return Visit::hole(());
                                }
                                AbsorbedEvent::Truncated => {
                                    // Saturation at the unexpanded
                                    // frontier — do NOT halt: the
                                    // known prefix may still hold a
                                    // concrete witness, and the
                                    // evaluator's BFS finds it
                                    // before the ceiling verdict
                                    // (review round 3). Surfaced
                                    // after the search when nothing
                                    // known was found.
                                    truncated = true;
                                }
                            }
                        }
                        Visit::edges(edges)
                    }
                }
            },
            |v: &V| match v {
                V::User(f) => match &dst_test {
                    DstIr::Group(g) => g.contains(f),
                    DstIr::Effects(atoms) => direct_of(*f)
                        .iter()
                        .any(|c| atoms.contains(c)),
                },
                // The evaluator applies direct_effects to EVERY
                // visited FnKey, stdlib included — an effects(C)
                // destination can be satisfied inside a stdlib body
                // (review: stdlib effect sinks).
                V::Interior(a, n) => match &dst_test {
                    DstIr::Group(_) => false,
                    DstIr::Effects(atoms) => absorption[*a as usize]
                        .nodes[*n as usize]
                        .direct_effects
                        .iter()
                        .any(|c| atoms.contains(c)),
                },
            },
            |v: &V| match v {
                V::User(f) => mask.is_some_and(|m| m.contains(f)),
                V::Interior(..) => false,
            },
            Some(crate::callgraph::MAX_STEPS),
            HolePolicy::Halt,
        );
        let verdict = match search {
            model_graph::Search::Found { hit, parent } => {
                render_violation_ir(
                    row,
                    &src_ref.name.display,
                    &dst_display(dst),
                    hit,
                    &parent,
                    &display,
                    row_span,
                    &mut diags,
                    &model_span,
                    e,
                );
                Verdict::Violated
            }
            model_graph::Search::Uncertified { .. } => {
                Verdict::Uncertified
            }
            model_graph::Search::Saturated { .. } => {
                diags.push(Diag::ty(
                    row_span,
                    format!(
                        "claim `{}`: reachability walk exceeded {} steps \
                         — cannot certify",
                        row.name,
                        crate::callgraph::MAX_STEPS
                    ),
                ));
                Verdict::Violated
            }
            model_graph::Search::NotFound => {
                if truncated {
                    // The known prefix held no witness and the
                    // interior was cut at the ceiling — the
                    // evaluator's saturation verdict.
                    diags.push(Diag::ty(
                        row_span,
                        format!(
                            "claim `{}`: reachability walk exceeded {} steps \
                             — cannot certify",
                            row.name,
                            crate::callgraph::MAX_STEPS
                        ),
                    ));
                    Verdict::Violated
                } else if let Some((vd, subj)) = &bus_unknown {
                    // No known counterexample, but a subscriber set
                    // in the composition is incomplete.
                    diags.push(Diag::ty(
                        row_span,
                        format!(
                            "claim `{}` cannot be certified: `{}` \
                             (reachable from `{}`) publishes to \
                             \"{}\", whose subscribers are not \
                             fully modeled. An unresolvable edge \
                             fails closed",
                            row.name,
                            vd,
                            src_ref.name.display,
                            subj
                        ),
                    ));
                    Verdict::Uncertified
                } else if *via_bus
                    && bus_holes.publishers_incomplete.is_some()
                {
                    // The PUBLISHER set of some subject is
                    // incomplete (round 8): an unknown publisher —
                    // possibly a fn this walk visited — may create
                    // a bus edge the composition cannot see.
                    let subj = bus_holes
                        .publishers_incomplete
                        .as_deref()
                        .unwrap_or_default();
                    diags.push(Diag::ty(
                        row_span,
                        format!(
                            "claim `{}` cannot be certified: the \
                             publisher set of \"{}\" is not fully \
                             modeled — an unknown publisher may \
                             create a bus edge the walk cannot \
                             see. An unresolvable edge fails \
                             closed",
                            row.name, subj
                        ),
                    ));
                    Verdict::Uncertified
                } else {
                    Verdict::Holds
                }
            }
        };
        out.push(Judged {
            ordinal: row.ordinal,
            verdict,
            diags,
        });
    }
    (pre, out)
}

fn dst_display(dst: &SetIr) -> String {
    match dst {
        SetIr::Group(g) => g.name.display.clone(),
        SetIr::EffectCarriers(c) => format!("effects({})", c.name),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_violation_ir(
    row: &hale_model::ClaimRow,
    src_disp: &str,
    dst_disp: &str,
    hit: V,
    parent: &BTreeMap<V, (V, StepIr)>,
    display: &dyn Fn(V) -> String,
    row_span: Span,
    diags: &mut Vec<Diag>,
    model_span: &dyn Fn(ProvenanceId) -> Span,
    e: &hale_model::Entities,
) {
    let mut rev: Vec<(V, Option<&StepIr>)> = Vec::new();
    let mut cur = hit;
    loop {
        match parent.get(&cur) {
            Some((prev, step)) => {
                rev.push((cur, Some(step)));
                cur = *prev;
            }
            None => {
                rev.push((cur, None));
                break;
            }
        }
    }
    rev.reverse();
    let mut path = String::new();
    for (node, incoming) in &rev {
        match incoming {
            None => path.push_str(&format!("`{}`", display(*node))),
            Some(StepIr::Call {
                dispatch: Some((iface, method)),
                ..
            }) => {
                path.push_str(&format!(
                    " -(dispatches {}.{})-> `{}`",
                    iface,
                    method,
                    display(*node)
                ));
            }
            Some(StepIr::Call { .. }) => {
                path.push_str(&format!(" -> `{}`", display(*node)));
            }
            Some(StepIr::Bus { subject, .. }) => {
                path.push_str(&format!(
                    " -(publishes \"{}\")-> `{}`",
                    subject,
                    display(*node)
                ));
            }
        }
    }
    diags.push(Diag::ty(
        row_span,
        format!(
            "claim `{}` violated: `{}` reaches `{}` — witness: {}",
            row.name, src_disp, dst_disp, path,
        ),
    ));
    // Related spans: the crossing edge, gated exactly like the
    // evaluator's `is_bundle_fn` (a crossing out of a stdlib body
    // has no bundle span).
    if let Some((_, Some(step))) = rev.last() {
        match step {
            StepIr::Call {
                provenance,
                dispatch,
                from_stdlib,
            } => {
                if !from_stdlib {
                    if let Some(pid) = provenance {
                        let msg = match dispatch {
                            Some((iface, _)) => format!(
                                "claim `{}`: the boundary into `{}` is \
                                 crossed by this dispatch through `{}`. \
                                 A call on an interface reaches EVERY \
                                 conforming locus, whatever this \
                                 expression happens to construct — so the \
                                 witness names one the claim forbids. \
                                 Narrow the receiver's type, or exclude \
                                 the conformer from the group",
                                row.name, dst_disp, iface
                            ),
                            None => format!(
                                "claim `{}`: the boundary into `{}` is \
                                 crossed by this call",
                                row.name, dst_disp
                            ),
                        };
                        diags.push(Diag::ty(model_span(*pid), msg));
                    }
                }
            }
            StepIr::Bus {
                publish_provenance,
                subscribe_provenance,
                from_stdlib,
                ..
            } => {
                if !from_stdlib {
                    if let Some(pid) = publish_provenance {
                        diags.push(Diag::ty(
                            model_span(*pid),
                            format!(
                                "claim `{}`: the crossing publish \
                                 happens here",
                                row.name
                            ),
                        ));
                    }
                }
                diags.push(Diag::ty(
                    model_span(*subscribe_provenance),
                    format!(
                        "claim `{}`: delivered at this \
                         subscription",
                        row.name
                    ),
                ));
            }
        }
    }
    // The destination's declaration.
    let V::User(hit_fn_id) = hit else { return };
    let hit_fn = &e.functions[hit_fn_id.index()];
    let (decl_disp, decl_prov) = match hit_fn.name.rsplit_once("::") {
        Some((locus_raw, _)) => {
            match e.loci.iter().find(|l| l.name == locus_raw) {
                Some(l) => (l.display.clone(), Some(l.provenance)),
                None => (hit_fn.display.clone(), None),
            }
        }
        None => (hit_fn.display.clone(), Some(hit_fn.provenance)),
    };
    if let Some(p) = decl_prov {
        let sp = model_span(p);
        if sp != Span::new(0, 0) {
            diags.push(Diag::ty(
                sp,
                format!(
                    "claim `{}`: the forbidden destination `{}` is \
                     declared here",
                    row.name, decl_disp
                ),
            ));
        }
    }
}

/// Judge every `only edges` row of one lowered law table against
/// its model (GH #476 Change 5b). Boundary grants are DIRECT-edge
/// law — no walk: every direct call or bus edge from the source
/// group into the destination group must match a granted line
/// (call edges are never grantable).
pub fn judge_only_edges(
    table: &ClaimIrTable,
    model: &ApplicationModel,
    source_bases: &[u32],
) -> Vec<Judged> {
    let e = &model.entities;
    let r = &model.relations;
    let claim_span = |pid: ProvenanceId| -> Span {
        span_of(&table.provenance, source_bases, pid)
    };
    let model_span = |pid: ProvenanceId| -> Span {
        span_of(&model.provenance, source_bases, pid)
    };
    let fn_disp = |f: FunctionId| e.functions[f.index()].display.clone();
    let v1: BTreeSet<FunctionId> =
        model.legacy.topology_v1_fns.iter().copied().collect();

    // Group projections (fn grain + locus decl grain), evaluator
    // iteration order: FnKey Ord = (locus: None < Some, name) —
    // free fns first, then methods grouped by locus.
    let fnkey_order = |raw: &str| -> (u8, String, String) {
        match raw.rsplit_once("::") {
            Some((l, m)) => (1, l.to_string(), m.to_string()),
            None => (0, String::new(), raw.to_string()),
        }
    };
    let mut by_locus: BTreeMap<u32, Vec<FunctionId>> = BTreeMap::new();
    for mo in &r.member_of {
        by_locus.entry(mo.locus.0).or_default().push(mo.function);
    }
    let group_fns = |g: GroupId| -> Vec<FunctionId> {
        let mut set: BTreeSet<FunctionId> = BTreeSet::new();
        for gm in r.group_members.iter().filter(|gm| gm.group == g) {
            match gm.member {
                EntityRef::LocusDecl(l) => {
                    for f in by_locus.get(&l.0).into_iter().flatten() {
                        if v1.contains(f) {
                            set.insert(*f);
                        }
                    }
                }
                EntityRef::Function(f) => {
                    // Declaration-only free fns are still decls.
                    set.insert(f);
                }
                _ => {}
            }
        }
        let mut v: Vec<FunctionId> = set.into_iter().collect();
        v.sort_by_key(|f| fnkey_order(&e.functions[f.index()].name));
        v
    };
    let group_loci = |g: GroupId| -> BTreeSet<u32> {
        r.group_members
            .iter()
            .filter(|gm| gm.group == g)
            .filter_map(|gm| match gm.member {
                EntityRef::LocusDecl(l) => Some(l.0),
                _ => None,
            })
            .collect()
    };
    // fn → its locus id.
    let locus_of: BTreeMap<FunctionId, u32> = r
        .member_of
        .iter()
        .map(|mo| (mo.function, mo.locus.0))
        .collect();
    // Per-fn call rows / publish rows by site; holes.
    let mut calls_of: BTreeMap<
        FunctionId,
        Vec<(u32, FunctionId, ProvenanceId)>,
    > = BTreeMap::new();
    for c in &r.calls {
        if matches!(c.dispatch, DispatchKind::ViaStdlib) {
            continue;
        }
        calls_of
            .entry(c.from)
            .or_default()
            .push((c.site, c.to, c.provenance));
    }
    for v in calls_of.values_mut() {
        v.sort_by_key(|(site, to, _)| (*site, *to));
    }
    let mut pubs_of: BTreeMap<
        FunctionId,
        Vec<(u32, u32, Option<u32>, String, ProvenanceId)>,
    > = BTreeMap::new();
    for p in &r.publishes {
        let written = match p.declared_topic {
            Some(t) => e.topics[t.index()].name.clone(),
            None => e.subjects[p.subject.index()].pattern.clone(),
        };
        pubs_of.entry(p.function).or_default().push((
            p.site,
            p.subject.0,
            p.declared_topic.map(|t| t.0),
            written,
            p.provenance,
        ));
    }
    for v in pubs_of.values_mut() {
        v.sort_by_key(|(site, ..)| *site);
    }
    // Keyed by the subscription's CANONICAL spelling — the topic
    // NAME for declared subscriptions, the authored pattern for
    // literal/wildcard ones — exactly the graph keying the
    // evaluator's subscribers_of iterates.
    let mut subs_by_pattern: BTreeMap<
        &str,
        Vec<(FunctionId, ProvenanceId)>,
    > = BTreeMap::new();
    for su in &r.subscribes {
        let key = match su.declared_topic {
            Some(t) => e.topics[t.index()].name.as_str(),
            None => e.subjects[su.subject.index()].pattern.as_str(),
        };
        subs_by_pattern
            .entry(key)
            .or_default()
            .push((su.handler, su.provenance));
    }
    let subs_for = |written: &str| -> Vec<(FunctionId, ProvenanceId)> {
        let mut out = Vec::new();
        for (pattern, subs) in &subs_by_pattern {
            let covers = *pattern == written
                || (pattern.contains("**")
                    && crate::wildcard_match(pattern, written));
            if covers {
                out.extend(subs.iter().cloned());
            }
        }
        out
    };
    // Holes retain their hides-mask; each event space consults the
    // family IT walks (call space → CALLS, publish space →
    // PUBLISHES) and the kind picks wording only. `authored_site`
    // (first occurrence) lets the boundary check interleave holes
    // with known edges as authored — the evaluator walks bodies in
    // source order and refuses at the hole's authored position,
    // BEFORE any later crossing is reported.
    #[derive(Clone)]
    enum FnHole {
        Indirect,
        Untyped { callee: String },
        Computed,
        Other { reason: String },
    }
    let mut holes_of: BTreeMap<
        FunctionId,
        Vec<(hale_model::RelationSet, Option<u32>, FnHole)>,
    > = BTreeMap::new();
    // Non-function holes participate through the shared two-grain
    // BusHoles index (subject patterns + topic names, rounds 4–5).
    let bus_holes = BusHoles::build(model);
    for h in &model.holes {
        match h.at {
            EntityRef::Function(f) => {
                let families = hale_model::RelationSet::CALLS
                    .union(hale_model::RelationSet::PUBLISHES);
                if !h.hides.intersects(families) {
                    continue;
                }
                let hole = match &h.kind {
                    HoleKind::IndirectCall => FnHole::Indirect,
                    HoleKind::UntypedReceiver { callee } => {
                        FnHole::Untyped {
                            callee: callee.clone(),
                        }
                    }
                    HoleKind::ComputedSubject => FnHole::Computed,
                    _ => FnHole::Other {
                        reason: h.reason.clone(),
                    },
                };
                holes_of
                    .entry(f)
                    .or_default()
                    .push((h.hides, h.authored_site, hole));
            }
            _ => {}
        }
    }

    let mut out = Vec::new();
    for row in &table.rows {
        let ClaimIr::OnlyEdges { src, dst, grants } = &row.law else {
            continue;
        };
        let mut diags: Vec<Diag> = Vec::new();
        let row_span = claim_span(row.provenance);
        // Validation (unknown groups) — Invalid without evaluation.
        let group_decl_names: Vec<&str> =
            e.groups.iter().map(|g| g.display.as_str()).collect();
        let mut ok = true;
        for gref in [src, dst] {
            if gref.group.is_none() {
                let mut near: Vec<&&str> = group_decl_names
                    .iter()
                    .filter(|g| {
                        crate::effects::close(g, &gref.name.display)
                    })
                    .collect();
                near.sort();
                let hint = match near.first() {
                    Some(n) => format!(" Did you mean `{}`?", n),
                    None => String::new(),
                };
                diags.push(Diag::ty(
                    claim_span(gref.provenance),
                    format!(
                        "claim `{}` names group `{}`, which is never declared. \
                         Add `group {} = {{ … }};` at the top level.{}",
                        row.name,
                        gref.name.display,
                        gref.name.display,
                        hint
                    ),
                ));
                ok = false;
            }
        }
        if !ok {
            out.push(Judged {
                ordinal: row.ordinal,
                verdict: Verdict::Invalid,
                diags,
            });
            continue;
        }
        let (src_gid, dst_gid) =
            (src.group.unwrap(), dst.group.unwrap());
        // Projection vacuity, source then target.
        let decl_count = |g: GroupId| {
            r.group_members.iter().filter(|gm| gm.group == g).count()
        };
        let src_fns = group_fns(src_gid);
        let dst_fns_v = group_fns(dst_gid);
        let mut vacuous = |gref: &hale_model::GroupRef,
                           fns: &[FunctionId],
                           which: &str,
                           diags: &mut Vec<Diag>|
         -> bool {
            let gid = gref.group.unwrap();
            if decl_count(gid) == 0 || !fns.is_empty() {
                return false;
            }
            diags.push(Diag::ty(
                claim_span(gref.provenance),
                format!(
                    "claim `{}`: group `{}` projects to no executable {} \
                     vertices — its declarations have no fns, so the claim \
                     proves nothing about them. The fn-grained walk cannot \
                     see pure-data access; name the loci that HOLD the \
                     behavior, or drop the claim",
                    row.name, gref.name.display, which
                ),
            ));
            true
        };
        if vacuous(src, &src_fns, "source", &mut diags)
            || vacuous(dst, &dst_fns_v, "target", &mut diags)
        {
            out.push(Judged {
                ordinal: row.ordinal,
                verdict: Verdict::Invalid,
                diags,
            });
            continue;
        }
        let dst_fn_set: BTreeSet<FunctionId> =
            dst_fns_v.iter().copied().collect();
        let dst_loci = group_loci(dst_gid);
        // The grant set: first-segment topic spellings as written.
        let granted: BTreeSet<&str> = grants
            .iter()
            .map(|g| g.topic.name.raw.as_str())
            .collect();
        let granted_disp = if granted.is_empty() {
            "none".to_string()
        } else {
            granted
                .iter()
                .map(|s| format!("`{}`", s))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut verdict = Verdict::Holds;
        let mut bus_unknown: Option<(String, String)> = None;
        let mut reported: BTreeSet<String> = BTreeSet::new();
        'fns: for f in &src_fns {
            // The evaluator walks fs.calls in SOURCE order and
            // refuses at an unfollowable edge's authored position —
            // a crossing AFTER the hole is never reported.
            // Interleave known call rows and call-space holes by
            // authored site (review: ordered event streams).
            enum CallEv<'x> {
                Edge(FunctionId, ProvenanceId),
                Hole(&'x FnHole),
            }
            let mut evs: Vec<(u32, u8, CallEv)> = Vec::new();
            for (site, to, cprov) in
                calls_of.get(f).into_iter().flatten()
            {
                evs.push((*site, 0, CallEv::Edge(*to, *cprov)));
            }
            for (hides, site, h) in
                holes_of.get(f).into_iter().flatten()
            {
                if !hides.intersects(hale_model::RelationSet::CALLS) {
                    continue;
                }
                evs.push((site.unwrap_or(u32::MAX), 1, CallEv::Hole(h)));
            }
            evs.sort_by_key(|(site, tie, _)| (*site, *tie));
            for (_, _, ev) in evs {
                let (to, cprov) = match ev {
                    CallEv::Hole(FnHole::Indirect) => {
                        diags.push(Diag::ty(
                            row_span,
                            format!(
                                "claim `{}` cannot be certified: `{}` \
                                 calls through a function-typed \
                                 parameter, whose target is not \
                                 knowable statically. An unresolvable \
                                 edge fails closed",
                                row.name,
                                fn_disp(*f)
                            ),
                        ));
                        verdict = Verdict::Uncertified;
                        break 'fns;
                    }
                    CallEv::Hole(FnHole::Untyped { callee }) => {
                        diags.push(Diag::ty(
                            row_span,
                            format!(
                                "claim `{}` cannot be certified: `{}` \
                                 calls `{}` on a receiver the \
                                 compiler cannot type, so the walk \
                                 cannot follow the edge. An \
                                 unresolvable edge fails closed — \
                                 bind the receiver to a typed field \
                                 or local so the call resolves",
                                row.name,
                                fn_disp(*f),
                                callee
                            ),
                        ));
                        verdict = Verdict::Uncertified;
                        break 'fns;
                    }
                    CallEv::Hole(FnHole::Other { reason }) => {
                        diags.push(Diag::ty(
                            row_span,
                            format!(
                                "claim `{}` cannot be certified: `{}` \
                                 — {}. An unresolvable edge fails \
                                 closed",
                                row.name,
                                fn_disp(*f),
                                reason
                            ),
                        ));
                        verdict = Verdict::Uncertified;
                        break 'fns;
                    }
                    CallEv::Hole(FnHole::Computed) => {
                        diags.push(Diag::ty(
                            row_span,
                            format!(
                                "claim `{}` cannot be certified: `{}` \
                                 publishes to a computed subject, which could \
                                 route to any subscriber. An unresolvable \
                                 edge fails closed",
                                row.name,
                                fn_disp(*f)
                            ),
                        ));
                        verdict = Verdict::Uncertified;
                        break 'fns;
                    }
                    CallEv::Edge(to, cprov) => (to, cprov),
                };
                {
                if dst_fn_set.contains(&to) {
                    let key =
                        format!("{}->{}", fn_disp(*f), fn_disp(to));
                    if reported.insert(key) {
                        diags.push(Diag::ty(
                            row_span,
                            format!(
                                "claim `{}` violated: un-granted \
                                 edge `{}` -> `{}` — call edges \
                                 are not grantable; the boundary \
                                 between `{}` and `{}` must be a \
                                 bus edge named in the grant list",
                                row.name,
                                fn_disp(*f),
                                fn_disp(to),
                                src.name.display,
                                dst.name.display
                            ),
                        ));
                        diags.push(Diag::ty(
                            model_span(cprov),
                            format!(
                                "claim `{}`: this call \
                                 crosses the boundary. A \
                                 call cannot be granted — \
                                 route it through a topic \
                                 named in the grant list, or \
                                 move the callee out of `{}`",
                                row.name, dst.name.display
                            ),
                        ));
                        verdict = Verdict::Violated;
                    }
                }
                }
            }
            enum PubEv<'x> {
                Row(u32, Option<u32>, &'x str, ProvenanceId),
                Hole(&'x FnHole),
            }
            let mut pevs: Vec<(u32, u8, PubEv)> = Vec::new();
            for (site, sid, topic, written, pprov) in
                pubs_of.get(f).into_iter().flatten()
            {
                pevs.push((
                    *site,
                    0,
                    PubEv::Row(
                        *sid,
                        *topic,
                        written.as_str(),
                        *pprov,
                    ),
                ));
            }
            for (hides, site, h) in
                holes_of.get(f).into_iter().flatten()
            {
                if !hides
                    .intersects(hale_model::RelationSet::PUBLISHES)
                {
                    continue;
                }
                pevs.push((
                    site.unwrap_or(u32::MAX),
                    1,
                    PubEv::Hole(h),
                ));
            }
            pevs.sort_by_key(|(site, tie, _)| (*site, *tie));
            for (_, _, pev) in pevs {
                let (written, pprov) = match pev {
                    PubEv::Hole(h) => {
                        let msg = match h {
                            FnHole::Computed => format!(
                                "claim `{}` cannot be certified: `{}` \
                                 publishes to a computed subject, which could \
                                 route to any subscriber. An unresolvable \
                                 edge fails closed",
                                row.name,
                                fn_disp(*f)
                            ),
                            FnHole::Indirect => format!(
                                "claim `{}` cannot be certified: `{}` \
                                 calls through a function-typed \
                                 parameter, whose target is not \
                                 knowable statically. An unresolvable \
                                 edge fails closed",
                                row.name,
                                fn_disp(*f)
                            ),
                            FnHole::Untyped { callee } => format!(
                                "claim `{}` cannot be certified: `{}` \
                                 calls `{}` on a receiver the \
                                 compiler cannot type, so the walk \
                                 cannot follow the edge. An \
                                 unresolvable edge fails closed — \
                                 bind the receiver to a typed field \
                                 or local so the call resolves",
                                row.name,
                                fn_disp(*f),
                                callee
                            ),
                            FnHole::Other { reason } => format!(
                                "claim `{}` cannot be certified: `{}` \
                                 — {}. An unresolvable edge fails \
                                 closed",
                                row.name,
                                fn_disp(*f),
                                reason
                            ),
                        };
                        diags.push(Diag::ty(row_span, msg));
                        verdict = Verdict::Uncertified;
                        break 'fns;
                    }
                    PubEv::Row(sid, topic, w, p) => {
                        // A set-level subscriber hole has no
                        // authored position relative to the known
                        // rows, so it DEFERS: the known rows below
                        // are still checked — a known ungranted
                        // crossing stays Violated — and the flag
                        // downgrades only a would-be Holds
                        // (round 6). TYPED identities: the topic
                        // id and the wire pattern, never text
                        // collisions.
                        if bus_holes.blocks(
                            hale_model::RelationSet::SUBSCRIBES,
                            topic,
                            Some(
                                e.subjects[sid as usize]
                                    .pattern
                                    .as_str(),
                            ),
                        ) && bus_unknown.is_none()
                        {
                            bus_unknown = Some((
                                fn_disp(*f),
                                w.to_string(),
                            ));
                        }
                        (w, p)
                    }
                };
                let pprov = &pprov;
                let written = &written.to_string();
                {
                for (handler, sprov) in subs_for(written) {
                    let Some(hl) = locus_of.get(&handler) else {
                        continue;
                    };
                    if !dst_loci.contains(hl) {
                        continue;
                    }
                    if granted.contains(written.as_str()) {
                        continue;
                    }
                    let handler_fn = &e.functions[handler.index()];
                    let sprov = &sprov;
                    let (sub_locus_disp, sub_handler) =
                        match handler_fn.display.rsplit_once("::") {
                            Some((l, m)) => {
                                (l.to_string(), m.to_string())
                            }
                            None => (
                                String::new(),
                                handler_fn.display.clone(),
                            ),
                        };
                    let key = format!(
                        "{}-({})->{}::{}",
                        fn_disp(*f),
                        written,
                        sub_locus_disp,
                        sub_handler
                    );
                    if reported.insert(key) {
                        diags.push(Diag::ty(
                            row_span,
                            format!(
                                "claim `{}` violated: un-granted edge \
                                 `{}` -(publishes \"{}\")-> `{}::{}`. \
                                 Granted: {}. If this edge is intended, \
                                 name it in the grant list — a grant is \
                                 a reviewable line",
                                row.name,
                                fn_disp(*f),
                                written,
                                sub_locus_disp,
                                sub_handler,
                                granted_disp
                            ),
                        ));
                        diags.push(Diag::ty(
                            model_span(*pprov),
                            format!(
                                "claim `{}`: the un-granted publish \
                                 happens here",
                                row.name
                            ),
                        ));
                        diags.push(Diag::ty(
                            model_span(*sprov),
                            format!(
                                "claim `{}`: received here. Grant this \
                                 edge with `publish {};` if it is \
                                 intended",
                                row.name, written
                            ),
                        ));
                        verdict = Verdict::Violated;
                    }
                }
                }
            }
        }
        if verdict == Verdict::Holds {
            if let Some((fd, w)) = &bus_unknown {
                diags.push(Diag::ty(
                    row_span,
                    format!(
                        "claim `{}` cannot be certified: `{}` \
                         publishes to \"{}\", whose subscribers \
                         are not fully modeled. An unresolvable \
                         edge fails closed",
                        row.name, fd, w
                    ),
                ));
                verdict = Verdict::Uncertified;
            } else if let Some(subj) =
                &bus_holes.publishers_incomplete
            {
                // Round 8: an unknown PUBLISHER — possibly a src
                // fn — may create an ungranted crossing the known
                // rows cannot show. A known counterexample above
                // still wins.
                diags.push(Diag::ty(
                    row_span,
                    format!(
                        "claim `{}` cannot be certified: the \
                         publisher set of \"{}\" is not fully \
                         modeled — an unknown publisher may create \
                         an ungranted edge. An unresolvable edge \
                         fails closed",
                        row.name, subj
                    ),
                ));
                verdict = Verdict::Uncertified;
            }
        }
        out.push(Judged {
            ordinal: row.ordinal,
            verdict,
            diags,
        });
    }
    out
}

/// Judge the endpoint/coverage/count family (GH #476 Change 5c):
/// `require publishes/subscribes`, `require sealed`,
/// `require attributed`, `cover`, and `count` rows.
pub fn judge_endpoints(
    table: &ClaimIrTable,
    model: &ApplicationModel,
    source_bases: &[u32],
) -> Vec<Judged> {
    let e = &model.entities;
    let r = &model.relations;
    let claim_span = |pid: ProvenanceId| -> Span {
        span_of(&table.provenance, source_bases, pid)
    };
    // Group decl-grain projections.
    let group_loci = |g: GroupId| -> BTreeSet<u32> {
        r.group_members
            .iter()
            .filter(|gm| gm.group == g)
            .filter_map(|gm| match gm.member {
                EntityRef::LocusDecl(l) => Some(l.0),
                _ => None,
            })
            .collect()
    };
    let locus_of: BTreeMap<FunctionId, u32> = r
        .member_of
        .iter()
        .map(|mo| (mo.function, mo.locus.0))
        .collect();
    // Canonical-spelling endpoint maps: publisher loci come from
    // DECLARED ends (`bus { publish T; }` — the graph's publisher
    // rows), subscriber loci from subscription rows.
    let mut publishers_at: BTreeMap<&str, BTreeSet<u32>> =
        BTreeMap::new();
    for dp in &r.declares_publish {
        let key = match dp.declared_topic {
            Some(t) => e.topics[t.index()].name.as_str(),
            None => e.subjects[dp.subject.index()].pattern.as_str(),
        };
        publishers_at.entry(key).or_default().insert(dp.locus.0);
    }
    let mut subscribers_at: BTreeMap<&str, BTreeSet<u32>> =
        BTreeMap::new();
    for su in &r.subscribes {
        let key = match su.declared_topic {
            Some(t) => e.topics[t.index()].name.as_str(),
            None => e.subjects[su.subject.index()].pattern.as_str(),
        };
        if let Some(l) = locus_of.get(&su.handler) {
            subscribers_at.entry(key).or_default().insert(*l);
        }
    }
    // The declared-topic name universe (validation) + seed topics.
    let topic_names: BTreeSet<&str> =
        e.topics.iter().map(|t| t.name.as_str()).collect();
    let mut seed_topics: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
    for di in &r.declared_in {
        if let EntityRef::Topic(t) = di.entity {
            seed_topics
                .entry(e.seeds[di.seed.index()].name.as_str())
                .or_default()
                .push(t.0);
        }
    }
    // AUTHORED user-class carriage — the labels table holds the
    // EXPANDED class set (a composed `effect io = {syscall, block}`
    // labels its atoms), but `require attributed` asks whether the
    // author wrote a user class, composed or atomic
    // (Function.carries_user_class, computed by the evaluator's own
    // fns_carrying_a_user_class).
    let carries_user: BTreeSet<FunctionId> = e
        .functions
        .iter()
        .enumerate()
        .filter(|(_, f)| f.carries_user_class)
        .map(|(i, _)| FunctionId(i as u32))
        .collect();
    let fn_raw = |f: FunctionId| e.functions[f.index()].name.clone();
    let fn_disp =
        |f: FunctionId| e.functions[f.index()].display.clone();
    let fnkey_order = |raw: &str| -> (u8, String, String) {
        match raw.rsplit_once("::") {
            Some((l, m)) => (1, l.to_string(), m.to_string()),
            None => (0, String::new(), raw.to_string()),
        }
    };
    // Relation-aware endpoint holes (review round 4): PUBLISHES,
    // SUBSCRIBES, and CARDINALITY are independently hideable at
    // subject and topic grain, and a relevant hole means the known
    // endpoint rows are a LOWER BOUND, never a proved absence.
    // Monotone cases stay decidable: a known witness still proves
    // an existential, and enough known rows still prove `>=`.
    let bus_holes = BusHoles::build(model);
    let topic_by_name: BTreeMap<&str, u32> = e
        .topics
        .iter()
        .enumerate()
        .map(|(i, t)| (t.name.as_str(), i as u32))
        .collect();
    let endpoint_unknown = |topic_raw: &str, publishers: bool| {
        let fam = if publishers {
            hale_model::RelationSet::PUBLISHES
        } else {
            hale_model::RelationSet::SUBSCRIBES
        };
        let mask = fam.union(hale_model::RelationSet::CARDINALITY);
        // TYPED projection (round 6): the claim names a declared
        // topic — its TopicId matches topic-grain holes, and its
        // WIRE pattern (never its name) is what subject-grain
        // holes cover. A subject hole whose pattern merely equals
        // the topic's NAME is a different wire identity.
        let topic = topic_by_name.get(topic_raw).copied();
        let wire = topic.map(|t| {
            e.subjects[e.topics[t as usize].subject.index()]
                .pattern
                .as_str()
        });
        bus_holes.blocks(mask, topic, wire)
    };

    let mut out = Vec::new();
    for row in &table.rows {
        let mut diags: Vec<Diag> = Vec::new();
        let row_span = claim_span(row.provenance);
        // Shared validation helpers over ClaimIr refs.
        let group_decl_names: Vec<&str> =
            e.groups.iter().map(|g| g.display.as_str()).collect();
        let check_group = |gref: &hale_model::GroupRef,
                           diags: &mut Vec<Diag>|
         -> bool {
            if gref.group.is_some() {
                return true;
            }
            let mut near: Vec<&&str> = group_decl_names
                .iter()
                .filter(|g| {
                    crate::effects::close(g, &gref.name.display)
                })
                .collect();
            near.sort();
            let hint = match near.first() {
                Some(n) => format!(" Did you mean `{}`?", n),
                None => String::new(),
            };
            diags.push(Diag::ty(
                claim_span(gref.provenance),
                format!(
                    "claim `{}` names group `{}`, which is never declared. \
                     Add `group {} = {{ … }};` at the top level.{}",
                    row.name,
                    gref.name.display,
                    gref.name.display,
                    hint
                ),
            ));
            false
        };
        let check_topic = |t: &hale_model::TopicIrRef,
                           diags: &mut Vec<Diag>|
         -> bool {
            if !t.name.raw.contains("::")
                && topic_names.contains(t.name.raw.as_str())
            {
                return true;
            }
            if t.name.raw.contains("::") {
                diags.push(Diag::ty(
                    claim_span(t.provenance),
                    format!(
                        "claim `{}`: topic reference `{}` does not \
                         resolve — no imported topic matches this \
                         path. Unknown names are errors, never empty \
                         sets",
                        row.name, t.name.display
                    ),
                ));
                return false;
            }
            let mut near: Vec<&&str> = topic_names
                .iter()
                .filter(|n| crate::effects::close(n, &t.name.raw))
                .collect();
            near.sort();
            let hint = match near.first() {
                Some(n) => format!(" Did you mean `{}`?", n),
                None => String::new(),
            };
            diags.push(Diag::ty(
                claim_span(t.provenance),
                format!(
                    "claim `{}` names topic `{}`, which is never \
                     declared.{}",
                    row.name, t.name.raw, hint
                ),
            ));
            false
        };
        match &row.law {
            ClaimIr::RequireEndpoint {
                publishers,
                group,
                topic,
            } => {
                let mut ok = check_group(group, &mut diags);
                ok &= check_topic(topic, &mut diags);
                if !ok {
                    out.push(Judged {
                        ordinal: row.ordinal,
                        verdict: Verdict::Invalid,
                        diags,
                    });
                    continue;
                }
                let g_loci = group_loci(group.group.unwrap());
                let ends = if *publishers {
                    publishers_at.get(topic.name.raw.as_str())
                } else {
                    subscribers_at.get(topic.name.raw.as_str())
                };
                let hit = ends.is_some_and(|loci| {
                    loci.iter().any(|l| g_loci.contains(l))
                });
                if hit {
                    // A known witness proves the existential even
                    // when the endpoint set is incomplete.
                    out.push(Judged {
                        ordinal: row.ordinal,
                        verdict: Verdict::Holds,
                        diags,
                    });
                    continue;
                }
                if endpoint_unknown(&topic.name.raw, *publishers) {
                    diags.push(Diag::ty(
                        row_span,
                        format!(
                            "claim `{}` cannot be certified: the {} \
                             set of `{}` is not fully modeled — a \
                             member of `{}` may {} it",
                            row.name,
                            if *publishers {
                                "publisher"
                            } else {
                                "subscriber"
                            },
                            topic.name.display,
                            group.name.display,
                            if *publishers {
                                "publish"
                            } else {
                                "subscribe to"
                            }
                        ),
                    ));
                    out.push(Judged {
                        ordinal: row.ordinal,
                        verdict: Verdict::Uncertified,
                        diags,
                    });
                    continue;
                }
                diags.push(Diag::ty(
                    row_span,
                    format!(
                        "claim `{}` violated: no member of `{}` {} `{}`",
                        row.name,
                        group.name.display,
                        if *publishers {
                            "publishes"
                        } else {
                            "subscribes"
                        },
                        topic.name.display
                    ),
                ));
                out.push(Judged {
                    ordinal: row.ordinal,
                    verdict: Verdict::Violated,
                    diags,
                });
            }
            ClaimIr::RequireSealed { group } => {
                if !check_group(group, &mut diags) {
                    out.push(Judged {
                        ordinal: row.ordinal,
                        verdict: Verdict::Invalid,
                        diags,
                    });
                    continue;
                }
                let g_loci = group_loci(group.group.unwrap());
                if g_loci.is_empty() {
                    diags.push(Diag::ty(
                        row_span,
                        format!(
                            "claim `{}`: group `{}` contains no loci, so \
                             `require sealed` would quantify over an empty set and \
                             hold while confining nothing. Sealing is a property of \
                             loci — name a group of them.",
                            row.name, group.name.display
                        ),
                    ));
                    out.push(Judged {
                        ordinal: row.ordinal,
                        verdict: Verdict::Invalid,
                        diags,
                    });
                    continue;
                }
                // Raw-sorted, display-rendered — the evaluator lists
                // raw names and demangles the whole message after.
                let mut unsealed: Vec<(String, String)> = g_loci
                    .iter()
                    .map(|l| &e.loci[*l as usize])
                    .filter(|l| !l.sealed)
                    .map(|l| (l.name.clone(), l.display.clone()))
                    .collect();
                unsealed.sort();
                if unsealed.is_empty() {
                    out.push(Judged {
                        ordinal: row.ordinal,
                        verdict: Verdict::Holds,
                        diags,
                    });
                    continue;
                }
                let one = unsealed.len() == 1;
                diags.push(Diag::ty(
                    row_span,
                    format!(
                        "claim `{}` violated: {} in `{}` {} not `@sealed`, so {} \
                         state is readable by anything holding {} — {}",
                        row.name,
                        if one { "a locus" } else { "loci" },
                        group.name.display,
                        if one { "is" } else { "are" },
                        if one { "its" } else { "their" },
                        if one { "it" } else { "them" },
                        unsealed
                            .iter()
                            .map(|(_, d)| d.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
                out.push(Judged {
                    ordinal: row.ordinal,
                    verdict: Verdict::Violated,
                    diags,
                });
            }
            ClaimIr::RequireAttributed { class } => {
                if crate::claims::attributed_mask(&class.name)
                    .is_none()
                {
                    diags.push(Diag::ty(
                        claim_span(class.provenance),
                        format!(
                            "claim `{}`: `require attributed` takes a \
                             built-in class with countable DIRECT sites — \
                             `syscall`, `block`, `publish`, `time`, \
                             `entropy`, `env`, `alloc`, `secret_use`. `{}` \
                             is not one of those: a user class would be \
                             trivially true, and `ffi` / `spawn` / \
                             `recursion` are structural properties with no \
                             site to attribute.",
                            row.name, class.name
                        ),
                    ));
                    out.push(Judged {
                        ordinal: row.ordinal,
                        verdict: Verdict::Invalid,
                        diags,
                    });
                    continue;
                }
                let mut unattributed: Vec<String> = e
                    .functions
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| {
                        f.attribution
                            .iter()
                            .any(|c| *c == class.name)
                    })
                    .filter(|(i, _)| {
                        !carries_user
                            .contains(&FunctionId(*i as u32))
                    })
                    .map(|(_, f)| f.display.clone())
                    .collect();
                if unattributed.is_empty() {
                    // A fn whose EFFECTS the model declares hidden
                    // (an unanalyzed body) may perform the class
                    // without an authored purpose — same unknown as
                    // an opaque call, same fallback (review round
                    // 2: `require attributed` must consult holes,
                    // not only Function.opaque_call).
                    let effects_holed: BTreeSet<FunctionId> = model
                        .holes
                        .iter()
                        .filter(|h| {
                            h.hides.intersects(
                                hale_model::RelationSet::EFFECTS,
                            )
                        })
                        .filter_map(|h| match h.at {
                            EntityRef::Function(f) => Some(f),
                            _ => None,
                        })
                        .collect();
                    let mut opaque: Vec<String> = e
                        .functions
                        .iter()
                        .enumerate()
                        .filter(|(i, f)| {
                            (f.opaque_call
                                || effects_holed.contains(
                                    &FunctionId(*i as u32),
                                ))
                                && !carries_user.contains(
                                    &FunctionId(*i as u32),
                                )
                        })
                        .map(|(_, f)| f.display.clone())
                        .collect();
                    if !opaque.is_empty() {
                        opaque.sort();
                        diags.push(Diag::ty(
                            row_span,
                            format!(
                                "claim `{}` uncertified: {} an indirect or opaque \
                                 call whose target this check cannot resolve, and \
                                 names no purpose of its own — so whether a `{}` \
                                 boundary is crossed there is unknown. Classify \
                                 the caller (`@effects(is: {{...}})`) or bind the \
                                 callee so it resolves: {}",
                                row.name,
                                if opaque.len() == 1 {
                                    "a fn makes"
                                } else {
                                    "fns make"
                                },
                                class.name,
                                opaque.join(", ")
                            ),
                        ));
                        out.push(Judged {
                            ordinal: row.ordinal,
                            verdict: Verdict::Uncertified,
                            diags,
                        });
                        continue;
                    }
                    out.push(Judged {
                        ordinal: row.ordinal,
                        verdict: Verdict::Holds,
                        diags,
                    });
                    continue;
                }
                unattributed.sort();
                diags.push(Diag::ty(
                    row_span,
                    format!(
                        "claim `{}` violated: {} `{}` with no declared purpose — \
                         classify {} (`@effects(is: {{...}})`) with a user effect \
                         class so the operation is attributable: {}",
                        row.name,
                        if unattributed.len() == 1 {
                            "a fn performs"
                        } else {
                            "fns perform"
                        },
                        class.name,
                        if unattributed.len() == 1 {
                            "it"
                        } else {
                            "them"
                        },
                        unattributed.join(", ")
                    ),
                ));
                out.push(Judged {
                    ordinal: row.ordinal,
                    verdict: Verdict::Violated,
                    diags,
                });
            }
            ClaimIr::Cover { seed, group } => {
                let mut ok = check_group(group, &mut diags);
                if !seed_topics.contains_key(seed.name.as_str()) {
                    diags.push(Diag::ty(
                        claim_span(seed.provenance),
                        format!(
                            "claim `{}`: `seed({})` names no import alias \
                             with declared topics — the coverage domain \
                             would be empty, and a universal over an \
                             empty domain holds vacuously",
                            row.name, seed.name
                        ),
                    ));
                    ok = false;
                }
                if !ok {
                    out.push(Judged {
                        ordinal: row.ordinal,
                        verdict: Verdict::Invalid,
                        diags,
                    });
                    continue;
                }
                let g_loci = group_loci(group.group.unwrap());
                let mut uncovered: Vec<(String, String)> =
                    Vec::new();
                let mut unknown: Vec<String> = Vec::new();
                let mut tids: Vec<u32> =
                    seed_topics[seed.name.as_str()].clone();
                tids.sort_by_key(|t| {
                    e.topics[*t as usize].name.clone()
                });
                for t in tids {
                    let topic = &e.topics[t as usize];
                    let covered = subscribers_at
                        .get(topic.name.as_str())
                        .is_some_and(|loci| {
                            loci.iter().any(|l| g_loci.contains(l))
                        });
                    if !covered {
                        // An apparently-uncovered topic whose
                        // subscriber set is incomplete cannot prove
                        // the violation (round 4) — but neither can
                        // it prove coverage.
                        if endpoint_unknown(&topic.name, false) {
                            unknown.push(topic.display.clone());
                        } else {
                            uncovered.push((
                                topic.name.clone(),
                                topic.display.clone(),
                            ));
                        }
                    }
                }
                if uncovered.is_empty() && !unknown.is_empty() {
                    diags.push(Diag::ty(
                        row_span,
                        format!(
                            "claim `{}` cannot be certified: the \
                             subscriber set of {} is not fully \
                             modeled, so coverage cannot be decided \
                             from the known rows",
                            row.name,
                            unknown
                                .iter()
                                .map(|d| format!("`{}`", d))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    ));
                    out.push(Judged {
                        ordinal: row.ordinal,
                        verdict: Verdict::Uncertified,
                        diags,
                    });
                    continue;
                }
                if uncovered.is_empty() {
                    out.push(Judged {
                        ordinal: row.ordinal,
                        verdict: Verdict::Holds,
                        diags,
                    });
                    continue;
                }
                let list = uncovered
                    .iter()
                    .map(|(_, d)| format!("`{}`", d))
                    .collect::<Vec<_>>()
                    .join(", ");
                diags.push(Diag::ty(
                    row_span,
                    format!(
                        "claim `{}` violated: {} topic(s) declared in seed `{}` \
                         have no subscriber in `{}`: {}",
                        row.name,
                        uncovered.len(),
                        seed.name,
                        group.name.display,
                        list
                    ),
                ));
                out.push(Judged {
                    ordinal: row.ordinal,
                    verdict: Verdict::Violated,
                    diags,
                });
            }
            ClaimIr::Count {
                publishers,
                topic,
                cmp,
                n,
            } => {
                if !check_topic(topic, &mut diags) {
                    out.push(Judged {
                        ordinal: row.ordinal,
                        verdict: Verdict::Invalid,
                        diags,
                    });
                    continue;
                }
                let ends = if *publishers {
                    publishers_at.get(topic.name.raw.as_str())
                } else {
                    subscribers_at.get(topic.name.raw.as_str())
                };
                let loci: Vec<(String, String)> = ends
                    .map(|set| {
                        let mut v: Vec<(String, String)> = set
                            .iter()
                            .map(|l| {
                                let ld = &e.loci[*l as usize];
                                (ld.name.clone(), ld.display.clone())
                            })
                            .collect();
                        v.sort();
                        v
                    })
                    .unwrap_or_default();
                let actual = loci.len() as u64;
                let unknown =
                    endpoint_unknown(&topic.name.raw, *publishers);
                let holds = match cmp {
                    hale_model::CountCmpIr::Eq => actual == *n,
                    hale_model::CountCmpIr::Le => actual <= *n,
                    hale_model::CountCmpIr::Ge => actual >= *n,
                };
                if unknown {
                    // The known rows are a LOWER BOUND (round 4):
                    // `>=` can still hold from enough known rows,
                    // and a lower bound already over an upper
                    // bound still violates — everything else is
                    // undecidable from an incomplete set.
                    let lower_bound_decides = match cmp {
                        hale_model::CountCmpIr::Ge => actual >= *n,
                        hale_model::CountCmpIr::Le
                        | hale_model::CountCmpIr::Eq => {
                            actual > *n
                        }
                    };
                    if !lower_bound_decides {
                        diags.push(Diag::ty(
                            row_span,
                            format!(
                                "claim `{}` cannot be certified: \
                                 the {} set of `{}` is not fully \
                                 modeled — {} known {} a lower \
                                 bound, not a count",
                                row.name,
                                if *publishers {
                                    "publisher"
                                } else {
                                    "subscriber"
                                },
                                topic.name.display,
                                actual,
                                if actual == 1 { "is" } else { "are" }
                            ),
                        ));
                        out.push(Judged {
                            ordinal: row.ordinal,
                            verdict: Verdict::Uncertified,
                            diags,
                        });
                        continue;
                    }
                }
                if holds {
                    out.push(Judged {
                        ordinal: row.ordinal,
                        verdict: Verdict::Holds,
                        diags,
                    });
                    continue;
                }
                let who = if loci.is_empty() {
                    String::new()
                } else {
                    format!(
                        " ({})",
                        loci.iter()
                            .map(|(_, d)| format!("`{}`", d))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                let cmp_str = match cmp {
                    hale_model::CountCmpIr::Eq => "==",
                    hale_model::CountCmpIr::Le => "<=",
                    hale_model::CountCmpIr::Ge => ">=",
                };
                diags.push(Diag::ty(
                    row_span,
                    format!(
                        "claim `{}` violated: counted {} {}{} of `{}`, claim \
                         requires {} {}",
                        row.name,
                        actual,
                        if *publishers {
                            "publisher(s)"
                        } else {
                            "subscriber(s)"
                        },
                        who,
                        topic.name.display,
                        cmp_str,
                        n
                    ),
                ));
                out.push(Judged {
                    ordinal: row.ordinal,
                    verdict: Verdict::Violated,
                    diags,
                });
            }
            _ => continue,
        }
        let _ = (&fn_raw, &fn_disp, &fnkey_order);
    }
    out
}

/// Judge the quantitative-bound family (GH #476 Change 5d):
/// `bound C <= N on paths from G` — call-tree SUM of carrier
/// sites, MAX over dispatch alternatives, unbounded on recursion,
/// loop-nested carriers, unfollowable calls, and computed
/// subjects, exactly the evaluator's `site_count`.
pub fn judge_bound(
    table: &ClaimIrTable,
    model: &ApplicationModel,
    source_bases: &[u32],
) -> Vec<Judged> {
    let e = &model.entities;
    let r = &model.relations;
    let claim_span = |pid: ProvenanceId| -> Span {
        span_of(&table.provenance, source_bases, pid)
    };
    let model_span = |pid: ProvenanceId| -> Span {
        span_of(&model.provenance, source_bases, pid)
    };
    let absorption = &model.legacy.stdlib_absorption;
    let display = |v: V| -> String {
        match v {
            V::User(f) => e.functions[f.index()].display.clone(),
            V::Interior(a, n) => absorption[a as usize].nodes
                [n as usize]
                .display
                .clone(),
        }
    };
    let v1: BTreeSet<FunctionId> =
        model.legacy.topology_v1_fns.iter().copied().collect();
    let mut by_locus: BTreeMap<u32, Vec<FunctionId>> = BTreeMap::new();
    for mo in &r.member_of {
        by_locus.entry(mo.locus.0).or_default().push(mo.function);
    }
    // The 5a projection: member loci project through the summary
    // universe (v1), but a member FREE fn enters unconditionally —
    // the evaluator's fn_set inserts every named free fn, including
    // declaration-only ones with no summary row (review round 2:
    // bound did not inherit this and rejected such groups as
    // projection-vacuous where the evaluator counts them as zero).
    let group_fns = |g: GroupId| -> BTreeSet<FunctionId> {
        let mut set: BTreeSet<FunctionId> = BTreeSet::new();
        for gm in r.group_members.iter().filter(|gm| gm.group == g) {
            match gm.member {
                EntityRef::LocusDecl(l) => {
                    for f in by_locus.get(&l.0).into_iter().flatten() {
                        if v1.contains(f) {
                            set.insert(*f);
                        }
                    }
                }
                EntityRef::Function(f) => {
                    set.insert(f);
                }
                _ => {}
            }
        }
        set
    };
    // The evaluator iterates fn sets in FnKey order — free fns
    // (locus: None) BEFORE methods — and stops at the first
    // unbounded root, so seeding order selects the reported reason.
    let fnkey_sorted = |set: &BTreeSet<FunctionId>| -> Vec<FunctionId> {
        let mut vv: Vec<FunctionId> = set.iter().copied().collect();
        vv.sort_by_key(|f| {
            let raw = &e.functions[f.index()].name;
            match raw.rsplit_once("::") {
                Some((l, m)) => (1u8, l.to_string(), m.to_string()),
                None => (0u8, String::new(), raw.clone()),
            }
        });
        vv
    };
    // Adjacency (calls with loop/group, publishes, subscriptions by
    // canonical spelling), and per-fn labels for the carrier test.
    struct CallRow {
        site: u32,
        to: FunctionId,
        in_loop: bool,
        provenance: ProvenanceId,
    }
    let mut calls_of: BTreeMap<FunctionId, Vec<CallRow>> =
        BTreeMap::new();
    for c in &r.calls {
        if matches!(c.dispatch, DispatchKind::ViaStdlib) {
            continue;
        }
        calls_of.entry(c.from).or_default().push(CallRow {
            site: c.site,
            to: c.to,
            in_loop: c.in_loop,
            provenance: c.provenance,
        });
    }
    for v in calls_of.values_mut() {
        v.sort_by_key(|cr| (cr.site, cr.to));
    }
    let mut absorb_of: BTreeMap<FunctionId, Vec<(u32, u32)>> =
        BTreeMap::new();
    for (ai, a) in absorption.iter().enumerate() {
        absorb_of
            .entry(a.from)
            .or_default()
            .push((a.site, ai as u32));
    }
    let mut pubs_of: BTreeMap<
        FunctionId,
        Vec<(u32, String, bool, ProvenanceId, Option<u32>, u32)>,
    > = BTreeMap::new();
    for p in &r.publishes {
        let written = match p.declared_topic {
            Some(t) => e.topics[t.index()].name.clone(),
            None => e.subjects[p.subject.index()].pattern.clone(),
        };
        pubs_of.entry(p.function).or_default().push((
            p.site,
            written,
            p.in_loop,
            p.provenance,
            p.declared_topic.map(|t| t.0),
            p.subject.0,
        ));
    }
    for v in pubs_of.values_mut() {
        v.sort_by_key(|(site, ..)| *site);
    }
    let mut subs_by_pattern: BTreeMap<&str, Vec<FunctionId>> =
        BTreeMap::new();
    for su in &r.subscribes {
        let key = match su.declared_topic {
            Some(t) => e.topics[t.index()].name.as_str(),
            None => e.subjects[su.subject.index()].pattern.as_str(),
        };
        subs_by_pattern.entry(key).or_default().push(su.handler);
    }
    let subs_for = |written: &str| -> Vec<FunctionId> {
        let mut out = Vec::new();
        for (pattern, subs) in &subs_by_pattern {
            let covers = *pattern == written
                || (pattern.contains("**")
                    && crate::wildcard_match(pattern, written));
            if covers {
                out.extend(subs.iter().copied());
            }
        }
        out
    };
    let mut labels_of: BTreeMap<FunctionId, Vec<&str>> =
        BTreeMap::new();
    for l in &model.labels {
        if let EntityRef::Function(f) = l.at {
            labels_of.entry(f).or_default().push(l.label.as_str());
        }
    }
    // Holes trigger by relation family and carry their authored
    // site, so the count walk consumes them at the evaluator's
    // source position (an unfollowable edge AFTER a recursive call
    // classifies as the cycle, not the hole).
    #[derive(Clone)]
    enum FnHole {
        Unfollowable(Option<u32>, ProvenanceId),
        Computed(Option<u32>, ProvenanceId),
    }
    let mut holes_of: BTreeMap<FunctionId, Vec<FnHole>> =
        BTreeMap::new();
    // Carrier counting needs EFFECTS beside the traversal families
    // (review round 3): a fn whose EFFECTS rows are hidden has an
    // UNKNOWN own-count — counting zero and certifying the bound
    // would fail open. Bus traversal likewise needs subject-grain
    // SUBSCRIBES: a hole at Subject(S) means fan-out through S is
    // not fully modeled.
    let mut effects_holed: BTreeSet<FunctionId> = BTreeSet::new();
    let bus_holes = BusHoles::build(model);
    for h in &model.holes {
        match h.at {
            EntityRef::Function(f) => {
                if h.hides
                    .intersects(hale_model::RelationSet::EFFECTS)
                {
                    effects_holed.insert(f);
                }
                let families = hale_model::RelationSet::CALLS
                    .union(hale_model::RelationSet::PUBLISHES);
                if !h.hides.intersects(families) {
                    continue;
                }
                let hole = match &h.kind {
                    HoleKind::ComputedSubject => FnHole::Computed(
                        h.authored_site,
                        h.provenance,
                    ),
                    _ => FnHole::Unfollowable(
                        h.authored_site,
                        h.provenance,
                    ),
                };
                holes_of.entry(f).or_default().push(hole);
            }
            _ => {}
        }
    }
    let class_atoms = |name: &str| -> BTreeSet<String> {
        match e.effect_classes.iter().find(|c| c.name == name) {
            Some(c) => match &c.definition {
                hale_model::EffectClassDefinition::Composed {
                    atoms,
                } => atoms.iter().cloned().collect(),
                hale_model::EffectClassDefinition::Atomic => {
                    [name.to_string()].into_iter().collect()
                }
                hale_model::EffectClassDefinition::InvalidCycle => {
                    BTreeSet::new()
                }
            },
            None => [name.to_string()].into_iter().collect(),
        }
    };

    #[derive(Clone)]
    enum UnboundedIr {
        Cycle(V),
        LoopCarrier { at: V, prov: Option<ProvenanceId> },
        Unfollowable { at: V, prov: Option<ProvenanceId> },
        ComputedSubject { at: V, prov: Option<ProvenanceId> },
        StepCeiling,
    }

    /// Why a count is a LOWER BOUND rather than exact (review
    /// round 5): unknowns no longer abort the walk — the known
    /// contribution is still counted (a known count already over
    /// the limit proves the violation regardless of what is
    /// hidden), and the flag downgrades a would-be Holds to
    /// Uncertified.
    #[derive(Clone)]
    enum UnknownCause {
        Effects { at: V },
        Subscribers { at: V, subject: String },
        Publishers { subject: String },
    }

    let mut out = Vec::new();
    for row in &table.rows {
        let ClaimIr::Bound { class, limit, from } = &row.law else {
            continue;
        };
        let mut diags: Vec<Diag> = Vec::new();
        let row_span = claim_span(row.provenance);
        // ---- validation: group + class rules ----
        let group_decl_names: Vec<&str> =
            e.groups.iter().map(|g| g.display.as_str()).collect();
        let mut ok = true;
        if from.group.is_none() {
            let mut near: Vec<&&str> = group_decl_names
                .iter()
                .filter(|g| {
                    crate::effects::close(g, &from.name.display)
                })
                .collect();
            near.sort();
            let hint = match near.first() {
                Some(n) => format!(" Did you mean `{}`?", n),
                None => String::new(),
            };
            diags.push(Diag::ty(
                claim_span(from.provenance),
                format!(
                    "claim `{}` names group `{}`, which is never declared. \
                     Add `group {} = {{ … }};` at the top level.{}",
                    row.name,
                    from.name.display,
                    from.name.display,
                    hint
                ),
            ));
            ok = false;
        }
        if class.builtin && class.name != "secret_use" {
            diags.push(Diag::ty(
                claim_span(class.provenance),
                format!(
                    "claim `{}`: `bound` takes a user-declared \
                     effect class (or `secret_use`) — the counted \
                     built-ins keep their `@budget` spellings \
                     (`publish`, `block_points`, `alloc_per_call`), \
                     and a second way to write one contract is \
                     what this rejects",
                    row.name
                ),
            ));
            ok = false;
        } else if class.class.is_some_and(|id| {
            matches!(
                e.effect_classes[id.index()].definition,
                hale_model::EffectClassDefinition::InvalidCycle
            )
        }) {
            // A cyclic definition resolves to no effect — a bound
            // over it would hold by counting zero. Invalid BEFORE
            // evaluation, same rule as 5a (review round 2).
            diags.push(Diag::ty(
                claim_span(class.provenance),
                format!(
                    "claim `{}`: effect class `{}` is defined in \
                     terms of itself. A cyclic definition resolves \
                     to no effect at all, so every contract naming \
                     it would hold vacuously.",
                    row.name, class.name
                ),
            ));
            ok = false;
        } else if !class.builtin
            && class.class.map_or(true, |id| {
                !e.effect_classes[id.index()].declared
            })
        {
            let mut near: Vec<&String> = e
                .effect_classes
                .iter()
                .filter(|ec| ec.declared)
                .map(|ec| &ec.name)
                .filter(|n| crate::effects::close(n, &class.name))
                .collect();
            near.sort();
            let hint = match near.first() {
                Some(n) => format!(" Did you mean `{}`?", n),
                None => String::new(),
            };
            diags.push(Diag::ty(
                claim_span(class.provenance),
                format!(
                    "claim `{}` names effect class `{}`, which is never \
                     declared. Add `effect {};` at the top level.{}",
                    row.name, class.name, class.name, hint
                ),
            ));
            ok = false;
        }
        if !ok {
            out.push(Judged {
                ordinal: row.ordinal,
                verdict: Verdict::Invalid,
                diags,
            });
            continue;
        }
        let gid = from.group.unwrap();
        let fns = group_fns(gid);
        let decl_count = r
            .group_members
            .iter()
            .filter(|gm| gm.group == gid)
            .count();
        if decl_count > 0 && fns.is_empty() {
            diags.push(Diag::ty(
                claim_span(from.provenance),
                format!(
                    "claim `{}`: group `{}` projects to no executable {} \
                     vertices — its declarations have no fns, so the claim \
                     proves nothing about them. The fn-grained walk cannot \
                     see pure-data access; name the loci that HOLD the \
                     behavior, or drop the claim",
                    row.name, from.name.display, "source"
                ),
            ));
            out.push(Judged {
                ordinal: row.ordinal,
                verdict: Verdict::Invalid,
                diags,
            });
            continue;
        }
        let atoms = class_atoms(&class.name);
        let own_count = |v: V| -> u64 {
            let carried = match v {
                V::User(f) => labels_of
                    .get(&f)
                    .is_some_and(|ls| {
                        ls.iter().any(|l| atoms.contains(*l))
                    }),
                V::Interior(a, n) => absorption[a as usize].nodes
                    [n as usize]
                    .carries
                    .iter()
                    .any(|l| atoms.contains(l)),
            };
            if carried {
                1
            } else {
                0
            }
        };
        // ---- the DFS (the evaluator's site_count, over V) ----
        struct Ctx<'x> {
            calls_of: &'x BTreeMap<FunctionId, Vec<CallRow>>,
            absorb_of: &'x BTreeMap<FunctionId, Vec<(u32, u32)>>,
            pubs_of: &'x BTreeMap<
                FunctionId,
                Vec<(
                    u32,
                    String,
                    bool,
                    ProvenanceId,
                    Option<u32>,
                    u32,
                )>,
            >,
            holes_of: &'x BTreeMap<FunctionId, Vec<FnHole>>,
            absorption: &'x [hale_model::StdlibAbsorption],
            effects_holed: &'x BTreeSet<FunctionId>,
            bus_holes: &'x BusHoles,
            subjects: &'x [hale_model::Subject],
            topics: &'x [hale_model::Topic],
        }
        impl<'x> Ctx<'x> {
            fn wire_of(&self, sid: u32) -> String {
                self.subjects[sid as usize].pattern.clone()
            }
            fn wire_of_topic_or(
                &self,
                t: Option<hale_model::TopicId>,
                text: &str,
            ) -> String {
                match t {
                    Some(t) => self.subjects[self.topics
                        [t.index()]
                    .subject
                    .index()]
                    .pattern
                    .clone(),
                    None => text.to_string(),
                }
            }
        }
        fn site_count_ir(
            v: V,
            cx: &Ctx,
            own: &dyn Fn(V) -> u64,
            subs_for: &dyn Fn(&str) -> Vec<FunctionId>,
            stack: &mut Vec<V>,
            memo: &mut BTreeMap<V, (u64, Vec<V>)>,
            steps: &mut u32,
            unknown: &mut Option<UnknownCause>,
        ) -> Result<(u64, Vec<V>), UnboundedIr> {
            if let Some(hit) = memo.get(&v) {
                return Ok(hit.clone());
            }
            if stack.contains(&v) {
                return Err(UnboundedIr::Cycle(v));
            }
            *steps += 1;
            if *steps > crate::callgraph::MAX_STEPS {
                return Err(UnboundedIr::StepCeiling);
            }
            if let V::User(f) = v {
                if cx.effects_holed.contains(&f) {
                    // Hidden carrier facts: the KNOWN labels still
                    // count (a lower bound already over the limit
                    // proves the violation), and the flag keeps a
                    // would-be Holds honest (review round 5).
                    if unknown.is_none() {
                        *unknown =
                            Some(UnknownCause::Effects { at: v });
                    }
                }
            }
            let own_n = own(v);
            stack.push(v);
            let mut total: u64 = 0;
            let mut best_child: (u64, Vec<V>) = (0, Vec::new());
            let mut group_best: BTreeMap<u32, (u64, Vec<V>)> =
                BTreeMap::new();
            let mut unbounded: Option<UnboundedIr> = None;
            // Successor events for either vertex kind, in the
            // evaluator's order: calls (with holes as unfollowable),
            // then publishes.
            enum Ev {
                Call {
                    to: V,
                    in_loop: bool,
                    group: Option<u32>,
                    prov: Option<ProvenanceId>,
                },
                Unfollowable(Option<ProvenanceId>),
                Publish {
                    subject: String,
                    in_loop: bool,
                    prov: Option<ProvenanceId>,
                    /// TYPED identity for hole coverage (round 6):
                    /// the declared topic id and the wire pattern.
                    topic: Option<u32>,
                    wire: String,
                },
                Computed(Option<ProvenanceId>),
                Truncated,
            }
            let mut evs: Vec<Ev> = Vec::new();
            match v {
                V::User(f) => {
                    let direct = cx
                        .calls_of
                        .get(&f)
                        .map(|x| x.as_slice())
                        .unwrap_or(&[]);
                    let absorbed = cx
                        .absorb_of
                        .get(&f)
                        .map(|x| x.as_slice())
                        .unwrap_or(&[]);
                    let holes = cx
                        .holes_of
                        .get(&f)
                        .map(|x| x.as_slice())
                        .unwrap_or(&[]);
                    let mut items: Vec<(u32, u8, usize)> = Vec::new();
                    for (i, cr) in direct.iter().enumerate() {
                        items.push((cr.site, 0, i));
                    }
                    for (i, (site, _)) in
                        absorbed.iter().enumerate()
                    {
                        items.push((*site, 1, i));
                    }
                    // Call-space holes interleave at their authored
                    // sites (kind 2).
                    for (i, h) in holes.iter().enumerate() {
                        if let FnHole::Unfollowable(site, _) = h {
                            items.push((
                                site.unwrap_or(u32::MAX),
                                2,
                                i,
                            ));
                        }
                    }
                    items.sort();
                    for (site, kind, i) in items {
                        if kind == 2 {
                            if let FnHole::Unfollowable(_, p) =
                                &holes[i]
                            {
                                evs.push(Ev::Unfollowable(Some(
                                    *p,
                                )));
                            }
                        } else if kind == 0 {
                            let cr = &direct[i];
                            evs.push(Ev::Call {
                                to: V::User(cr.to),
                                in_loop: cr.in_loop,
                                group: Some(site),
                                prov: Some(cr.provenance),
                            });
                        } else {
                            let (_, ai) = absorbed[i];
                            let a = &cx.absorption[ai as usize];
                            // The REAL entry edge: its loop nesting
                            // and span come from the authored call
                            // (review: looped stdlib entries).
                            // The alternatives of ONE authored
                            // dispatch share the LOCAL site as
                            // their group, whether the target is a
                            // user fn or an absorption entry —
                            // entry_group is summary-global and
                            // would split the dispatch in two,
                            // summing what must max (review
                            // round 2).
                            evs.push(Ev::Call {
                                to: V::Interior(ai, 0),
                                in_loop: a.entry_in_loop,
                                group: Some(site),
                                prov: Some(a.entry_provenance),
                            });
                        }
                    }
                    let pubs = cx
                        .pubs_of
                        .get(&f)
                        .map(|x| x.as_slice())
                        .unwrap_or(&[]);
                    let mut pitems: Vec<(u32, u8, usize)> =
                        Vec::new();
                    for (i, (site, ..)) in pubs.iter().enumerate() {
                        pitems.push((*site, 0, i));
                    }
                    for (i, h) in holes.iter().enumerate() {
                        if let FnHole::Computed(site, _) = h {
                            pitems.push((
                                site.unwrap_or(u32::MAX),
                                1,
                                i,
                            ));
                        }
                    }
                    pitems.sort();
                    for (_, kind, i) in pitems {
                        if kind == 1 {
                            if let FnHole::Computed(_, p) = &holes[i]
                            {
                                evs.push(Ev::Computed(Some(*p)));
                            }
                        } else {
                            let (
                                _,
                                written,
                                in_loop,
                                pprov,
                                topic,
                                sid,
                            ) = &pubs[i];
                            evs.push(Ev::Publish {
                                subject: written.clone(),
                                in_loop: *in_loop,
                                prov: Some(*pprov),
                                topic: *topic,
                                wire: cx.wire_of(*sid),
                            });
                        }
                    }
                }
                V::Interior(ai, ni) => {
                    let node = &cx.absorption[ai as usize].nodes
                        [ni as usize];
                    for ev in &node.events {
                        match ev {
                            AbsorbedEvent::Call {
                                target,
                                in_loop,
                                group,
                                ..
                            } => {
                                let to = match target {
                                    AbsorbedTarget::Interior(n2) => {
                                        V::Interior(ai, *n2)
                                    }
                                    AbsorbedTarget::User(f2) => {
                                        V::User(*f2)
                                    }
                                };
                                evs.push(Ev::Call {
                                    to,
                                    in_loop: *in_loop,
                                    group: *group,
                                    prov: None,
                                });
                            }
                            AbsorbedEvent::CallHole(_) => {
                                evs.push(Ev::Unfollowable(None))
                            }
                            AbsorbedEvent::Publish {
                                subject,
                                in_loop,
                                declared_topic,
                            } => evs.push(Ev::Publish {
                                subject: subject.clone(),
                                in_loop: *in_loop,
                                prov: None,
                                topic: declared_topic.map(|t| t.0),
                                wire: cx.wire_of_topic_or(
                                    *declared_topic,
                                    subject,
                                ),
                            }),
                            AbsorbedEvent::PublishHole => {
                                evs.push(Ev::Computed(None))
                            }
                            AbsorbedEvent::Truncated => {
                                evs.push(Ev::Truncated)
                            }
                        }
                    }
                }
            }
            for ev in evs {
                match ev {
                    Ev::Unfollowable(p) => {
                        unbounded = Some(UnboundedIr::Unfollowable {
                            at: v,
                            prov: p,
                        });
                        break;
                    }
                    Ev::Computed(p) => {
                        unbounded =
                            Some(UnboundedIr::ComputedSubject {
                                at: v,
                                prov: p,
                            });
                        break;
                    }
                    Ev::Truncated => {
                        unbounded = Some(UnboundedIr::StepCeiling);
                        break;
                    }
                    Ev::Call {
                        to,
                        in_loop,
                        group,
                        prov,
                    } => {
                        match site_count_ir(
                            to, cx, own, subs_for, stack, memo,
                            steps, unknown,
                        ) {
                            Err(u) => {
                                unbounded = Some(u);
                                break;
                            }
                            Ok((w, p)) => {
                                if in_loop && w > 0 {
                                    unbounded = Some(
                                        UnboundedIr::LoopCarrier {
                                            at: v,
                                            prov,
                                        },
                                    );
                                    break;
                                }
                                match group {
                                    Some(g) => {
                                        let eb = group_best
                                            .entry(g)
                                            .or_insert((
                                                0,
                                                Vec::new(),
                                            ));
                                        if w > eb.0 {
                                            *eb = (w, p);
                                        }
                                    }
                                    None => {
                                        total = total
                                            .saturating_add(w);
                                        if w > best_child.0 {
                                            best_child = (w, p);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ev::Publish {
                        subject,
                        in_loop,
                        prov,
                        topic,
                        wire,
                    } => {
                        if cx.bus_holes.blocks(
                            hale_model::RelationSet::SUBSCRIBES,
                            topic,
                            Some(wire.as_str()),
                        ) && unknown.is_none()
                        {
                            // Known handlers still count below —
                            // the incomplete set only forbids
                            // certifying a Holds (round 5).
                            *unknown =
                                Some(UnknownCause::Subscribers {
                                    at: v,
                                    subject: subject.clone(),
                                });
                        }
                        let _ = &prov;
                        for handler in subs_for(&subject) {
                            match site_count_ir(
                                V::User(handler),
                                cx,
                                own,
                                subs_for,
                                stack,
                                memo,
                                steps,
                                unknown,
                            ) {
                                Err(u) => {
                                    unbounded = Some(u);
                                    break;
                                }
                                Ok((w, p)) => {
                                    if in_loop && w > 0 {
                                        unbounded = Some(
                                            UnboundedIr::LoopCarrier {
                                                at: v,
                                                prov,
                                            },
                                        );
                                        break;
                                    }
                                    total =
                                        total.saturating_add(w);
                                    if w > best_child.0 {
                                        best_child = (w, p);
                                    }
                                }
                            }
                        }
                        if unbounded.is_some() {
                            break;
                        }
                    }
                }
            }
            if unbounded.is_none() {
                for (w, p) in group_best.into_values() {
                    total = total.saturating_add(w);
                    if w > best_child.0 {
                        best_child = (w, p);
                    }
                }
            }
            stack.pop();
            if let Some(u) = unbounded {
                return Err(u);
            }
            let mut path = vec![v];
            path.extend(best_child.1);
            let res = (own_n + total, path);
            memo.insert(v, res.clone());
            Ok(res)
        }
        let cx = Ctx {
            calls_of: &calls_of,
            absorb_of: &absorb_of,
            pubs_of: &pubs_of,
            holes_of: &holes_of,
            absorption,
            effects_holed: &effects_holed,
            bus_holes: &bus_holes,
            subjects: &e.subjects,
            topics: &e.topics,
        };
        let mut worst: (u64, Vec<V>) = (0, Vec::new());
        let mut why: Option<UnboundedIr> = None;
        // Round 8: an incomplete PUBLISHER set anywhere on the bus
        // means the count's fan-out may be missing edges — the
        // known lower bound still decides a violation; a would-be
        // Holds downgrades.
        let mut unknown_cause: Option<UnknownCause> = bus_holes
            .publishers_incomplete
            .as_ref()
            .map(|s| UnknownCause::Publishers {
                subject: s.clone(),
            });
        for root in &fnkey_sorted(&fns) {
            let mut stack = Vec::new();
            let mut memo: BTreeMap<V, (u64, Vec<V>)> = BTreeMap::new();
            let mut steps = 0u32;
            match site_count_ir(
                V::User(*root),
                &cx,
                &own_count,
                &subs_for,
                &mut stack,
                &mut memo,
                &mut steps,
                &mut unknown_cause,
            ) {
                Err(u) => {
                    why = Some(u);
                    break;
                }
                Ok((w, p)) => {
                    if w > worst.0 {
                        worst = (w, p);
                    }
                }
            }
        }
        if let Some(u) = why {
            let why_str = match &u {
                UnboundedIr::Cycle(k) => format!(
                    "`{}` is reachable from itself, so the count repeats \
                     per recursion",
                    display(*k)
                ),
                UnboundedIr::LoopCarrier { at, .. } => format!(
                    "a carrier is reached from inside a loop in `{}`, so \
                     it repeats per iteration",
                    display(*at)
                ),
                UnboundedIr::Unfollowable { at, .. } => format!(
                    "`{}` makes a call the walk cannot follow, so the \
                     contribution beyond it is unknown",
                    display(*at)
                ),
                UnboundedIr::ComputedSubject { at, .. } => format!(
                    "`{}` publishes to a computed subject, which could \
                     route to any subscriber",
                    display(*at)
                ),
                UnboundedIr::StepCeiling => {
                    "the walk hit its step ceiling before settling"
                        .into()
                }
            };
            diags.push(Diag::ty(
                row_span,
                format!(
                    "claim `{}` violated: paths from `{}` carry an \
                     unbounded number of `{}` sites (limit {}) — {}",
                    row.name,
                    from.name.display,
                    class.name,
                    limit,
                    why_str
                ),
            ));
            let site = match &u {
                UnboundedIr::LoopCarrier { at, prov } => prov
                    .map(|p| {
                        (*at, p, "this is the loop-nested carrier")
                    }),
                UnboundedIr::Unfollowable { at, prov } => {
                    prov.map(|p| {
                        (
                            *at,
                            p,
                            "this is the call the walk cannot follow",
                        )
                    })
                }
                UnboundedIr::ComputedSubject { at, prov } => prov
                    .map(|p| {
                        (
                            *at,
                            p,
                            "this is the computed publish subject",
                        )
                    }),
                _ => None,
            };
            if let Some((at, p, label)) = site {
                if matches!(at, V::User(_)) {
                    diags.push(Diag::ty(
                        model_span(p),
                        format!("claim `{}`: {}", row.name, label),
                    ));
                }
            }
            out.push(Judged {
                ordinal: row.ordinal,
                verdict: Verdict::Violated,
                diags,
            });
            continue;
        }
        let (w, path) = worst;
        if w <= *limit {
            // The KNOWN lower bound is within the limit — but an
            // incomplete model cannot certify Holds (round 5: the
            // monotonic rule endpoint counts already follow).
            if let Some(cause) = &unknown_cause {
                let msg = match cause {
                    UnknownCause::Effects { at } => format!(
                        "claim `{}` cannot be certified: the `{}` \
                         carrier facts of `{}` are not fully \
                         analyzable, so its contribution to the \
                         count is unknown",
                        row.name,
                        class.name,
                        display(*at)
                    ),
                    UnknownCause::Subscribers { at, subject } => {
                        format!(
                            "claim `{}` cannot be certified: `{}` \
                             publishes to \"{}\", whose \
                             subscribers are not fully modeled — \
                             the fan-out's contribution to the \
                             count is unknown",
                            row.name,
                            display(*at),
                            subject
                        )
                    }
                    UnknownCause::Publishers { subject } => {
                        format!(
                            "claim `{}` cannot be certified: the \
                             publisher set of \"{}\" is not \
                             fully modeled — an unknown publisher \
                             may add fan-out the count cannot see",
                            row.name, subject
                        )
                    }
                };
                diags.push(Diag::ty(row_span, msg));
                out.push(Judged {
                    ordinal: row.ordinal,
                    verdict: Verdict::Uncertified,
                    diags,
                });
                continue;
            }
            out.push(Judged {
                ordinal: row.ordinal,
                verdict: Verdict::Holds,
                diags,
            });
            continue;
        }
        let chain = path
            .iter()
            .map(|k| format!("`{}`", display(*k)))
            .collect::<Vec<_>>()
            .join(" -> ");
        diags.push(Diag::ty(
            row_span,
            format!(
                "claim `{}` violated: heaviest path from `{}` carries {} \
                 `{}` sites, limit {} — path: {}",
                row.name, from.name.display, w, class.name, limit, chain
            ),
        ));
        out.push(Judged {
            ordinal: row.ordinal,
            verdict: Verdict::Violated,
            diags,
        });
    }
    out
}

/// Judge the pointwise-certificate family (GH #476 Change 5e):
/// `@effects(none/only/publish)` and `@no_panic` on fns, and
/// `@phase_effects` on loci. The certificate ENGINES stay the one
/// analysis authority — the builder runs them and stores each
/// certificate's outcome and diagnostics as
/// [`hale_model::CertificateEvidence`] (the artifact's `lowered`
/// rows, with their diags); this judgment renders verdicts and
/// diagnostics from that model data, plus the undeclared-class
/// validation over the typed effect-class table.
pub fn judge_certificates(
    table: &ClaimIrTable,
    model: &ApplicationModel,
    source_bases: &[u32],
) -> Vec<Judged> {
    let e = &model.entities;
    let claim_span = |pid: ProvenanceId| -> Span {
        span_of(&table.provenance, source_bases, pid)
    };
    let model_span = |pid: ProvenanceId| -> Span {
        span_of(&model.provenance, source_bases, pid)
    };
    // Evidence multimap keyed (subject display, form), consumed in
    // order — duplicate identical certificates on one fn pair up by
    // generation order.
    let mut evidence: BTreeMap<(String, String), Vec<usize>> =
        BTreeMap::new();
    for (i, ev) in model.evidence.iter().enumerate() {
        evidence
            .entry((ev.subject_display.clone(), ev.form.clone()))
            .or_default()
            .push(i);
    }
    let mut cursor: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut take = |key: (String, String)| -> Option<usize> {
        let list = evidence.get(&key)?;
        let c = cursor.entry(key).or_insert(0);
        let i = list.get(*c).copied();
        *c += 1;
        i
    };
    let verdict_of = |v: hale_model::VerdictIr| match v {
        hale_model::VerdictIr::Holds => Verdict::Holds,
        hale_model::VerdictIr::Violated => Verdict::Violated,
        hale_model::VerdictIr::Uncertified => Verdict::Uncertified,
        hale_model::VerdictIr::Invalid => Verdict::Invalid,
    };
    let undeclared = |c: &EffectClassRefLite| -> bool {
        !c.builtin
            && c.class.map_or(true, |id| {
                !e.effect_classes[id.index()].declared
            })
    };
    struct EffectClassRefLite {
        builtin: bool,
        class: Option<hale_model::EffectClassId>,
    }
    let lite = |c: &hale_model::EffectClassRef| EffectClassRefLite {
        builtin: c.builtin,
        class: c.class,
    };
    let mut out = Vec::new();
    for row in &table.rows {
        let subject_disp = |at: &(
            Option<FunctionId>,
            hale_model::NameRef,
        )| at.1.display.clone();
        let mut diags: Vec<Diag> = Vec::new();
        // Per-root undeclared-class validation (the evaluator's
        // pass 1 covers Forbid/Causes lists; dedup is per root and
        // handled here per row, matching the per-assert lowering).
        let mut p1 =
            |classes: &[hale_model::EffectClassRef],
             subj: &str,
             diags: &mut Vec<Diag>| {
                let mut seen: BTreeSet<&str> = BTreeSet::new();
                for c in classes {
                    if undeclared(&lite(c))
                        && seen.insert(c.name.as_str())
                    {
                        let mut near: Vec<&String> = e
                            .effect_classes
                            .iter()
                            .filter(|ec| ec.declared)
                            .map(|ec| &ec.name)
                            .filter(|n| {
                                crate::effects::close(n, &c.name)
                            })
                            .collect();
                        near.sort();
                        let hint = match near.first() {
                            Some(n) => {
                                format!(" Did you mean `{}`?", n)
                            }
                            None => String::new(),
                        };
                        diags.push(Diag::ty(
                            claim_span(c.provenance),
                            format!(
                                "`{}` asserts about effect class `{}`, \
                                 which is never declared. Add `effect {};` \
                                 at the top level.{}",
                                subj, c.name, c.name, hint
                            ),
                        ));
                    }
                }
            };
        let (keys, verdict_src): (
            Vec<(String, String)>,
            Option<&(Option<FunctionId>, hale_model::NameRef)>,
        ) = match &row.law {
            ClaimIr::EffectForbid { at, classes } => {
                p1(classes, &at.1.display, &mut diags);
                (
                    classes
                        .iter()
                        .map(|c| {
                            (
                                subject_disp(at),
                                format!(
                                    "forbid reaches({{{}}}, effects({}))",
                                    at.1.display, c.name
                                ),
                            )
                        })
                        .collect(),
                    Some(at),
                )
            }
            ClaimIr::EffectOnly { at, classes } => {
                let set = classes
                    .iter()
                    .map(|c| c.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                (
                    vec![(
                        subject_disp(at),
                        format!(
                            "only effects {{{}}} on {{{}}}",
                            set, at.1.display
                        ),
                    )],
                    Some(at),
                )
            }
            ClaimIr::EffectPublishSet { at, entries } => {
                let allowed = entries
                    .iter()
                    .map(|s| s.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                (
                    vec![(
                        subject_disp(at),
                        format!(
                            "only publishes {{{}}} from {{{}}}",
                            allowed, at.1.display
                        ),
                    )],
                    Some(at),
                )
            }
            ClaimIr::NoPanic { at } => (
                vec![(
                    subject_disp(at),
                    format!(
                        "forbid reaches({{{}}}, panic)",
                        at.1.display
                    ),
                )],
                Some(at),
            ),
            ClaimIr::PhaseEffects { locus, phases } => {
                let keys = phases
                    .iter()
                    .map(|(phase, allowed)| {
                        let set = allowed
                            .iter()
                            .map(|c| c.name.clone())
                            .collect::<Vec<_>>()
                            .join(", ");
                        (
                            locus.1.display.clone(),
                            format!(
                                "only effects {{{}}} on {{{}}} during {}",
                                set, locus.1.display, phase
                            ),
                        )
                    })
                    .collect();
                (keys, None)
            }
            _ => continue,
        };
        let _ = verdict_src;
        let mut verdict = Verdict::Holds;
        for key in keys {
            let Some(i) = take(key) else {
                // No evidence row — the engines never saw this
                // certificate (an unresolved subject); Invalid.
                verdict = Verdict::Invalid;
                continue;
            };
            let ev = &model.evidence[i];
            let v = verdict_of(ev.result);
            if severity(v) > severity(verdict) {
                verdict = v;
            }
            for (msg, pid) in &ev.diags {
                diags.push(Diag::ty(model_span(*pid), msg.clone()));
            }
        }
        out.push(Judged {
            ordinal: row.ordinal,
            verdict,
            diags,
        });
    }
    out
}

fn severity(v: Verdict) -> u8 {
    match v {
        Verdict::Holds => 0,
        Verdict::Uncertified => 1,
        Verdict::Violated => 2,
        Verdict::Invalid => 3,
    }
}
