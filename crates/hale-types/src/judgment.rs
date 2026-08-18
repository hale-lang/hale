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
    // subject id → subscriber rows.
    let mut subs_of: BTreeMap<u32, Vec<(FunctionId, ProvenanceId)>> =
        BTreeMap::new();
    for su in &r.subscribes {
        subs_of
            .entry(su.subject.0)
            .or_default()
            .push((su.handler, su.provenance));
    }
    // written publish text → subject id (topic name or pattern).
    let mut subject_by_text: BTreeMap<&str, u32> = BTreeMap::new();
    for (i, su) in e.subjects.iter().enumerate() {
        subject_by_text.insert(su.pattern.as_str(), i as u32);
    }
    for t in &e.topics {
        subject_by_text.insert(t.name.as_str(), t.subject.0);
    }
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
                                for (handler, spid) in subs_of
                                    .get(sid)
                                    .into_iter()
                                    .flatten()
                                {
                                    edges.push((
                                        V::User(*handler),
                                        StepIr::Bus {
                                            subject: written.clone(),
                                            publish_provenance: Some(
                                                *ppid,
                                            ),
                                            subscribe_provenance:
                                                *spid,
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
                                            subject_by_text
                                                .get(subject.as_str())
                                                .and_then(|sid| {
                                                    subs_of.get(sid)
                                                })
                                                .into_iter()
                                                .flatten()
                                        {
                                            edges.push((
                                                V::User(*handler),
                                                StepIr::Bus {
                                                    subject: subject
                                                        .clone(),
                                                    publish_provenance:
                                                        None,
                                                    subscribe_provenance:
                                                        *spid,
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
