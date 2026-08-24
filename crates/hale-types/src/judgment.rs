//! GH #476 Changes 5a–5e — the judgment engine over
//! `ClaimIr` × `ApplicationModel`.
//!
//! Family-by-family migration of the claim evaluators onto the
//! canonical model. Each family landed with DIAGNOSTICS PARITY —
//! the same public spelling, spans, and related notes the old
//! evaluator emitted, held byte-equal by a permanent corpus
//! differential — plus negative controls proving the engine reads
//! the model relations it claims to (dropping a family's rows must
//! change its verdicts).
//!
//! Change 9 finished it: these engines ARE the authority now, for
//! `hale check` (via [`claim_law_diags`]) as well as for the
//! artifact. The evaluator in `claims.rs` keeps law SELECTION —
//! which laws exist at all — and is otherwise a test oracle with no
//! production callers, enforced by
//! `tests/legacy_oracle_is_test_only.rs`.
//!
//! 5a: reachability (`forbid reaches`) + holes. The walk reuses
//! `model_graph::search` with a two-kind vertex: user functions
//! (model rows — `calls` at site grain, publish × subscribe
//! composition, `member_of` ∩ the summary universe for groups,
//! `phase_of` for `during`, typed holes for the fail-closed edges)
//! and INTERIOR stdlib vertices from the legacy absorption sidecar
//! (`Analyses::stdlib_absorption`), which preserves the
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
    /// Per-diag SOURCE-SPACE discriminator (round 5): `true` when
    /// the diag's span lives in the FOREIGN (stdlib) offset space,
    /// whose numbers can overlap a bundle file — a consumer must
    /// never re-resolve such a span against bundle sources. Empty
    /// = every diag is bundle-space (the 5a–5d judgments only
    /// speak claim/model spans); only `judge_certificates` fills
    /// it, from the evidence records' own variants.
    pub foreign: Vec<bool>,
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
/// May a law quantifying over this group be judged at all?
///
/// The answer is CARRIED from law selection
/// ([`hale_model::GroupSelection`]), never re-derived here. The
/// first version of this guard asked the model "does the group have
/// members, and did it declare `may_be_empty`?", and that is a
/// different question with three wrong answers: a group whose only
/// member is misspelled resolves to nothing, so `{ Missing }
/// may_be_empty` read as intentionally empty; `{ Worker, Missing }`
/// read as resolved because one member survived, and the law was
/// judged over a subset of the domain the author wrote; and a name
/// declared twice read as fine, while the model keeps the LAST
/// declaration and selection keeps the first — so the law was
/// judged against a definition selection had discarded.
///
/// No diagnostic is emitted at the refusal: selection owns the
/// message. What this decides is only whether a VERDICT may be
/// recorded, and over a refused domain the honest answer is
/// `Invalid` — there is no witness and no program it describes.
fn domain_is_judgable(
    table: &ClaimIrTable,
    gref: &hale_model::GroupRef,
) -> bool {
    match table.group_selection.get(&gref.name.raw) {
        Some(status) => status.is_judgable(),
        // A group selection never saw at all: the reference is
        // unresolved, which the per-family validation already
        // reports and refuses.
        None => true,
    }
}

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
    let absorption = &model.analyses.stdlib_absorption;
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
        model.summarized_fns().collect();

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
                foreign: Vec::new(),
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
                foreign: Vec::new(),
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
        // EVERY group operand, not just the endpoints. `avoiding`
        // is a domain too: its members become the mask that removes
        // paths from the walk, so a partially-resolved gate masks
        // with the members that happened to survive and the claim
        // can be proved by a subset of the gate the author wrote.
        // Checked here — after validation, so the shape diagnostics
        // still fire — and before roots, mask, or any verdict is
        // derived from a refused group (review round 3).
        if !domain_is_judgable(table, src_ref)
            || matches!(dst, SetIr::Group(g)
                if !domain_is_judgable(table, g))
            || avoiding
                .as_ref()
                .is_some_and(|a| !domain_is_judgable(table, a))
        {
            out.push(Judged {
                ordinal: row.ordinal,
                verdict: Verdict::Invalid,
                diags,
                foreign: Vec::new(),
            });
            continue;
        }
        if vacuous(src_gid, src_ref, "source", &mut diags) {
            out.push(Judged {
                ordinal: row.ordinal,
                verdict: Verdict::Invalid,
                diags,
                foreign: Vec::new(),
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
                        foreign: Vec::new(),
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
                    foreign: Vec::new(),
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
                    foreign: Vec::new(),
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
            foreign: Vec::new(),
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
        model.summarized_fns().collect();

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
        // …and unknown GRANT topics. A grant names the reviewable
        // declaration that admits an edge, so a grant naming
        // nothing is an invalid law, not a law with one fewer
        // grant — silently dropping it would evaluate a WEAKER
        // claim than the one written and report its violations as
        // if the author had chosen them. (GH #476 Change 9: the
        // evaluator validated this and the engine did not; the
        // corpus differential could not see it, because the
        // fixture that covers it is a `format!` template the
        // corpus provider skips.)
        let topic_names: Vec<&str> =
            e.topics.iter().map(|t| t.display.as_str()).collect();
        for g in grants {
            if g.topic.topic.is_some() {
                continue;
            }
            let mut near: Vec<&&str> = topic_names
                .iter()
                .filter(|n| {
                    crate::effects::close(n, &g.topic.name.raw)
                })
                .collect();
            near.sort();
            let hint = match near.first() {
                Some(n) => format!(" Did you mean `{}`?", n),
                None => String::new(),
            };
            diags.push(Diag::ty(
                claim_span(g.topic.provenance),
                format!(
                    "claim `{}` names topic `{}`, which is never \
                     declared.{}",
                    row.name, g.topic.name.raw, hint
                ),
            ));
            ok = false;
        }
        if !ok {
            out.push(Judged {
                ordinal: row.ordinal,
                verdict: Verdict::Invalid,
                diags,
                foreign: Vec::new(),
            });
            continue;
        }
        let (src_gid, dst_gid) =
            (src.group.unwrap(), dst.group.unwrap());
        if !domain_is_judgable(table, src)
            || !domain_is_judgable(table, dst)
        {
            out.push(Judged {
                ordinal: row.ordinal,
                verdict: Verdict::Invalid,
                diags,
                foreign: Vec::new(),
            });
            continue;
        }
        // Projection vacuity, source then target.
        let decl_count = |g: GroupId| {
            r.group_members.iter().filter(|gm| gm.group == g).count()
        };
        let src_fns = group_fns(src_gid);
        let dst_fns_v = group_fns(dst_gid);
        let vacuous = |gref: &hale_model::GroupRef,
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
                foreign: Vec::new(),
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
            foreign: Vec::new(),
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
    let mut refused_ordinals: BTreeSet<u32> = BTreeSet::new();
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
        // A domain law selection already refused holds nothing —
        // see `empty_refused_domain`.
        let refused_domain = match &row.law {
            ClaimIr::RequireEndpoint { group, .. }
            | ClaimIr::RequireSealed { group }
            | ClaimIr::Cover { group, .. } => {
                !domain_is_judgable(table, group)
            }
            _ => false,
        };
        // NB: the refusal does not short-circuit the arm. Its
        // validation diagnostics are the evaluator's and must still
        // be emitted — what a refused domain forbids is a VERDICT,
        // so the arm runs and every verdict it produced is forced
        // after the loop (arms `continue` from several points, so
        // the rewrite cannot live at the bottom of the body).
        if refused_domain {
            refused_ordinals.insert(row.ordinal);
        }
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
                        foreign: Vec::new(),
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
                        foreign: Vec::new(),
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
                        foreign: Vec::new(),
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
                    foreign: Vec::new(),
                });
            }
            ClaimIr::RequireSealed { group } => {
                if !check_group(group, &mut diags) {
                    out.push(Judged {
                        ordinal: row.ordinal,
                        verdict: Verdict::Invalid,
                        diags,
                        foreign: Vec::new(),
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
                        foreign: Vec::new(),
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
                        foreign: Vec::new(),
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
                    foreign: Vec::new(),
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
                        foreign: Vec::new(),
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
                            foreign: Vec::new(),
                        });
                        continue;
                    }
                    out.push(Judged {
                        ordinal: row.ordinal,
                        verdict: Verdict::Holds,
                        diags,
                        foreign: Vec::new(),
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
                    foreign: Vec::new(),
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
                        foreign: Vec::new(),
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
                        foreign: Vec::new(),
                    });
                    continue;
                }
                if uncovered.is_empty() {
                    out.push(Judged {
                        ordinal: row.ordinal,
                        verdict: Verdict::Holds,
                        diags,
                        foreign: Vec::new(),
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
                    foreign: Vec::new(),
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
                        foreign: Vec::new(),
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
                            foreign: Vec::new(),
                        });
                        continue;
                    }
                }
                if holds {
                    out.push(Judged {
                        ordinal: row.ordinal,
                        verdict: Verdict::Holds,
                        diags,
                        foreign: Vec::new(),
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
                    foreign: Vec::new(),
                });
            }
            _ => continue,
        }
        let _ = (&fn_raw, &fn_disp, &fnkey_order);
    }
    for j in out.iter_mut() {
        if refused_ordinals.contains(&j.ordinal) {
            j.verdict = Verdict::Invalid;
        }
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
    let mut refused_ordinals: BTreeSet<u32> = BTreeSet::new();
    let e = &model.entities;
    let r = &model.relations;
    let claim_span = |pid: ProvenanceId| -> Span {
        span_of(&table.provenance, source_bases, pid)
    };
    let model_span = |pid: ProvenanceId| -> Span {
        span_of(&model.provenance, source_bases, pid)
    };
    let absorption = &model.analyses.stdlib_absorption;
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
        model.summarized_fns().collect();
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
        // As in the endpoint family: the arm still runs and still
        // reports, and only the verdict is refused.
        if !domain_is_judgable(table, from) {
            refused_ordinals.insert(row.ordinal);
        }
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
                foreign: Vec::new(),
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
                foreign: Vec::new(),
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
                foreign: Vec::new(),
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
                    foreign: Vec::new(),
                });
                continue;
            }
            out.push(Judged {
                ordinal: row.ordinal,
                verdict: Verdict::Holds,
                diags,
                foreign: Vec::new(),
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
            foreign: Vec::new(),
        });
    }
    for j in out.iter_mut() {
        if refused_ordinals.contains(&j.ordinal) {
            j.verdict = Verdict::Invalid;
        }
    }
    out
}

/// Judge the pointwise-certificate family (GH #476 Change 5e):
/// `@effects(...)`, `@no_panic`, `@budget(...)` on fns, and
/// `@phase_effects` / `@effects(depends:)` on loci. The certificate
/// ENGINES stay the one analysis authority — the producer
/// ([`crate::evidence::derive_certificate_evidence`]) runs them and
/// keys each certificate's outcome + diagnostics by the ClaimIr
/// ordinal in an [`hale_model::EvidenceTable`] SIDECAR (outside the
/// model: a model must not carry a cached prior judgment of
/// itself). This judgment:
///
/// - refuses stale evidence structurally (`model_shape` must equal
///   the judged model's `TopologyShapeV1`) — every certificate row
///   goes `Invalid` rather than replaying another model's outcomes;
/// - consumes evidence rows by ORDINAL and typed subject — no
///   string matching here; a missing / short / subject-disagreeing
///   row is `Invalid`;
/// - emits exactly ONE `Judged` row per certificate-family ClaimIr
///   row, including the families whose engines run elsewhere
///   (`causes:` in the bus-graph pass, `depends:` / `@budget` in
///   their own passes) — those judge to at-minimum `Uncertified`
///   until their engines are migrated, so no lowered law can
///   silently drop out of the verdict stream;
/// - forces `Invalid` (never a vacuous `Holds`) when a row asserts
///   about an effect class that is never declared. The DIAGNOSTIC
///   for that (with the evaluator's per-annotated-root dedup across
///   `none:`/`causes:`/`is:` lists) is the LOWERING's — it lands in
///   `ClaimIrTable::issues`, because `is:` produces no row at all
///   and one authority must own the dedup.
pub fn judge_certificates(
    table: &ClaimIrTable,
    model: &ApplicationModel,
    evidence: &hale_model::EvidenceTable,
    source_bases: &[u32],
) -> Vec<Judged> {
    let e = &model.entities;
    let claim_span = |pid: ProvenanceId| -> Span {
        span_of(&table.provenance, source_bases, pid)
    };
    let ev_span = |pid: ProvenanceId| -> Span {
        span_of(&evidence.provenance, source_bases, pid)
    };
    // MANDATORY validation, performed by the judgment itself
    // (review round 2): the sidecar must tie to BOTH the model
    // (TopologyShapeV1) and the LAW TABLE it answers (semantic
    // digest — topology does not hash annotation laws), and must be
    // structurally well-formed (unique in-range ordinals, resolvable
    // diagnostic provenance). A malformed or mis-tied sidecar
    // invalidates every certificate row rather than being
    // replayed, partially consumed, or silently collapsed.
    let stale = evidence
        .validate(
            model,
            crate::topology_projection::project_shape_hash(model),
            table,
            crate::evidence::analysis_inputs_digest(),
        )
        .is_err();
    let ev_rows: BTreeMap<u32, &hale_model::EvidenceRow> =
        evidence.rows.iter().map(|r| (r.ordinal, r)).collect();
    let verdict_of = |v: hale_model::VerdictIr| match v {
        hale_model::VerdictIr::Holds => Verdict::Holds,
        hale_model::VerdictIr::Violated => Verdict::Violated,
        hale_model::VerdictIr::Uncertified => Verdict::Uncertified,
        hale_model::VerdictIr::Invalid => Verdict::Invalid,
    };
    let undeclared = |c: &hale_model::EffectClassRef| -> bool {
        !c.builtin
            && c.class.map_or(true, |id| {
                !e.effect_classes[id.index()].declared
            })
    };
    // A row asserting about a class that is never declared is not
    // a checkable law. The diagnostic (per-root dedup across the
    // root's class lists) is emitted by the LOWERING as a table
    // issue; here the consequence is the verdict.
    let any_undeclared = |classes: &[hale_model::EffectClassRef]| {
        classes.iter().any(undeclared)
    };
    // A cyclically-defined class resolves to no effect at all — a
    // certificate naming it would hold vacuously. The shared
    // class-validity rule reachability and bound already apply
    // (review round 5): Invalid, never Holds. The legacy pass
    // reports the cycle in its global pre-stratum; the machine
    // verdict must agree.
    let cyclic = |c: &hale_model::EffectClassRef| -> bool {
        c.class.is_some_and(|id| {
            matches!(
                e.effect_classes[id.index()].definition,
                hale_model::EffectClassDefinition::InvalidCycle
            )
        })
    };
    let mut out = Vec::new();
    for row in &table.rows {
        // Structural shape of the row: how many certificates the
        // engines produce for it, and the typed subject the
        // evidence row must agree with. `None` count = a family
        // whose engine has not migrated — judged Uncertified.
        let (expected, subject, classes): (
            Option<usize>,
            Option<FunctionId>,
            &[hale_model::EffectClassRef],
        ) = match &row.law {
            ClaimIr::EffectForbid { at, classes } => {
                (Some(classes.len()), at.0, classes.as_slice())
            }
            ClaimIr::EffectOnly { at, .. } => (Some(1), at.0, &[]),
            ClaimIr::EffectPublishSet { at, .. } => {
                (Some(1), at.0, &[])
            }
            ClaimIr::NoPanic { at } => (Some(1), at.0, &[]),
            ClaimIr::PhaseEffects { phases, .. } => {
                (Some(phases.len()), None, &[])
            }
            ClaimIr::EffectCauses { at, classes } => {
                (None, at.0, classes.as_slice())
            }
            ClaimIr::DependsSet { .. } => (None, None, &[]),
            ClaimIr::AllocBudget { at, .. } => (None, at.0, &[]),
            ClaimIr::QuantBudget { at, dim, .. } => (
                None,
                at.0,
                // A user-class budget dimension is a class
                // reference like any other (review round 6): an
                // undeclared class makes the row Invalid, never a
                // fall-through Uncertified — the quantitative
                // evaluator refuses it the same way.
                match dim {
                    hale_model::QuantDimIr::UserClass(c) => {
                        std::slice::from_ref(c)
                    }
                    _ => &[],
                },
            ),
            _ => continue,
        };
        let mut diags: Vec<Diag> = Vec::new();
        // Invalid, never a vacuous Holds (review) — the diagnostic
        // itself is a lowering issue, not a judgment diag.
        // The cycle rule covers EVERY class-bearing form (Only and
        // phase allow-lists included — a cyclic class is not a
        // valid denotation anywhere), while the undeclared rule
        // keeps the evaluator's pass-1 scope.
        let any_cyclic = match &row.law {
            ClaimIr::EffectForbid { classes, .. }
            | ClaimIr::EffectOnly { classes, .. }
            | ClaimIr::EffectCauses { classes, .. } => {
                classes.iter().any(&cyclic)
            }
            ClaimIr::PhaseEffects { phases, .. } => phases
                .iter()
                .flat_map(|(_, allowed)| allowed.iter())
                .any(&cyclic),
            ClaimIr::QuantBudget {
                dim: hale_model::QuantDimIr::UserClass(c),
                ..
            } => cyclic(c),
            _ => false,
        };
        let invalid_class = any_undeclared(classes) || any_cyclic;
        let Some(expected) = expected else {
            // Engine not migrated (causes / depends / budgets):
            // still exactly one Judged row, at minimum Uncertified.
            out.push(Judged {
                ordinal: row.ordinal,
                verdict: if invalid_class {
                    Verdict::Invalid
                } else {
                    Verdict::Uncertified
                },
                diags,
                foreign: Vec::new(),
            });
            continue;
        };
        if stale {
            diags.push(Diag::ty(
                claim_span(row.provenance),
                format!(
                    "claim `{}`: certificate evidence does not \
                     tie to this model and law table (stale or \
                     malformed sidecar) — re-derive evidence",
                    row.name
                ),
            ));
            out.push(Judged {
                ordinal: row.ordinal,
                verdict: Verdict::Invalid,
                diags,
                foreign: Vec::new(),
            });
            continue;
        }
        let ev = ev_rows.get(&row.ordinal).copied();
        let usable = ev.filter(|r| {
            r.subject == subject && r.certs.len() == expected
        });
        let Some(ev) = usable else {
            // Round 8: an entirely REPORT-LESS row (no evidence
            // row, or one with zero certificates) is a subject the
            // engines never analyzed — a module-scoped body. That
            // is residue, not invalidity: `uncertified`, with the
            // reason on the row. A PARTIAL disagreement (subject
            // mismatch, or a count that differs with certificates
            // present) is still Invalid — evidence answering some
            // other law. A statically invalid class dominates
            // both.
            let report_missing =
                ev.is_none_or(|r| r.certs.is_empty());
            let verdict = if invalid_class {
                Verdict::Invalid
            } else if report_missing {
                diags.push(Diag::ty(
                    claim_span(row.provenance),
                    format!(
                        "claim `{}`: the certificate engines did not \
                         analyze this subject (module-scoped bodies are \
                         outside the legacy analyzable universe) — \
                         uncertified",
                        row.name
                    ),
                ));
                Verdict::Uncertified
            } else {
                Verdict::Invalid
            };
            out.push(Judged {
                ordinal: row.ordinal,
                verdict,
                diags,
                foreign: Vec::new(),
            });
            continue;
        };
        let mut verdict = Verdict::Holds;
        // The source-space discriminator (round 5): every diag
        // accumulated BEFORE this loop is claim-space (bundle);
        // evidence diags carry their record's own variant — a
        // ForeignSpan lives in the stdlib offset space, and its
        // numbers must never be re-resolved against bundle files.
        let mut foreign: Vec<bool> = vec![false; diags.len()];
        for cert in ev.certs.iter() {
            let v = verdict_of(cert.result);
            if severity(v) > severity(verdict) {
                verdict = v;
            }
            for (msg, pid) in &cert.diags {
                let is_foreign = matches!(
                    evidence.provenance.records.get(pid.index()),
                    Some(hale_model::Provenance::ForeignSpan {
                        ..
                    })
                );
                diags.push(Diag::ty(ev_span(*pid), msg.clone()));
                foreign.push(is_foreign);
            }
        }
        if invalid_class {
            verdict = Verdict::Invalid;
        }
        out.push(Judged {
            ordinal: row.ordinal,
            verdict,
            diags,
            foreign,
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

/// GH #476 Change 9 — the CHECK path's claim diagnostics, from the
/// same judgment the artifact projects.
///
/// `hale check` used to call a second evaluator (`claims.rs`) that
/// re-derived these four families from source, in parallel with the
/// engines here deriving them from the model. Two authorities for
/// one question is the defect this epic exists to remove: the
/// corpus differentials could only ever hold them equal, never make
/// them the same answer. This is the same answer.
///
/// Scope is the MIGRATED, self-judged families — reachability,
/// boundary, endpoint, bound. Certificates are deliberately absent:
/// `judge_certificates` judges rows against evidence the effects
/// engine produced, and that engine emits its own diagnostics in
/// check; re-emitting them here would duplicate, which is the thing
/// being deleted. `Unmigrated` rows keep their existing single
/// authority (`frontier`, `quantitative`, `budget_check`).
pub fn claim_law_diags(bundle: &crate::symbol::Bundle<'_>) -> Vec<Diag> {
    // The epic's demand rule: a program that swears to nothing has
    // nothing to judge, and must not pay for a model derivation.
    // The scan is structural and AST-cheap — no resolution, no
    // summary — so the no-claims path (the LSP's) stays what it was.
    if !has_claim_surface(bundle) {
        return Vec::new();
    }
    let model = crate::model_builder::derive_application_model(bundle);
    let table = crate::claim_lowering::lower_claims(bundle, &model);
    // Law-SELECTION invalidity (unknown/cyclic constitution, illegal
    // adoption, collisions) produced no row to judge, so it must be
    // reported from the table itself or it disappears between
    // checking and the artifact.
    let source_bases: Vec<u32> =
        bundle.sources.iter().map(|f| f.base).collect();
    let mut out: Vec<Diag> = Vec::new();
    // Law-SELECTION issues are NOT emitted here: `claims::
    // selection_diags` is their one authority (they are questions
    // about which laws exist, not about what a law says), and the
    // check path calls it alongside this. The table still carries
    // them for the artifact, whose law account must show every
    // issue in one document.
    let evidence = crate::evidence::derive_certificate_evidence(
        bundle, &table, &model,
    );
    let (pre, judged) = crate::topology_projection::judge_all(
        &table,
        &model,
        &evidence,
        &source_bases,
    );
    // Table-level pre-pass first (duplicate claim names), then each
    // row in AUTHORED order — the evaluator's order, which the
    // diagnostics differential holds byte-equal.
    out.extend(pre);
    for row in &table.rows {
        if !matches!(
            row.family(),
            hale_model::JudgmentFamily::Reachability
                | hale_model::JudgmentFamily::Boundary
                | hale_model::JudgmentFamily::Endpoint
                | hale_model::JudgmentFamily::Bound
                | hale_model::JudgmentFamily::Causes
                | hale_model::JudgmentFamily::Depends
        ) {
            continue;
        }
        if let Some(j) = judged.get(&row.ordinal) {
            out.extend(j.diags.iter().cloned());
        }
    }
    crate::stdlib_bodies::demangle_imports(&mut out, &bundle.import_renames);
    // Law diagnostics are `Claim`-kinded at the source, exactly as
    // the evaluator emitted them. `check` re-kinds its whole law
    // block defensively; a consumer that reads this function
    // directly (the LSP, a test) gets the right kind without it.
    for d in &mut out {
        if d.kind == hale_syntax::error::DiagKind::Type {
            d.kind = hale_syntax::error::DiagKind::Claim;
        }
    }
    out
}

/// Does this bundle declare any claim the judgment engines would
/// judge — a `claims { }` block (world tier or library tier) or a
/// `constitution` to adopt one from?
///
/// …plus `@effects(causes: …)` on a function and
/// `@effects(depends: …)` on a locus, which are annotation-carried
/// but judged here since Changes 5f and 5g. Other annotations are
/// deliberately NOT a claim surface: their rows are the certificate
/// family, whose diagnostics belong to the effects engine.
fn has_claim_surface(bundle: &crate::symbol::Bundle<'_>) -> bool {
    use hale_syntax::ast::{EffectAssert, FnDecl, LocusMember, TopDecl};
    fn causes(fd: &FnDecl) -> bool {
        fd.effects
            .iter()
            .any(|a| matches!(a, EffectAssert::Causes(_)))
    }
    fn walk(items: &[TopDecl]) -> bool {
        items.iter().any(|item| match item {
            TopDecl::Claims(_) | TopDecl::Constitution(_) => true,
            TopDecl::Fn(f) => causes(f),
            TopDecl::Locus(l) => {
                l.depends.is_some()
                    || l.members.iter().any(|m| match m {
                        LocusMember::Claims(_) => true,
                        LocusMember::Fn(f) => causes(f),
                        _ => false,
                    })
            }
            TopDecl::Module(m) => walk(&m.items),
            _ => false,
        })
    }
    bundle.programs.values().any(|p| walk(&p.items))
}


pub use crate::model_query::{
    effect_class_atoms, may_deliver, render_effect_classes,
};

/// GH #476 Change 5f — `@effects(causes: {…})` over the model.
///
/// "What can this fn cause ANYWHERE in the system": the effects of
/// every handler a publish it can reach may be delivered to,
/// following further hops when those handlers publish in turn.
///
/// Three rules the first draft of this engine got wrong, each of
/// which certified an absence it could not see:
///
///  * **The causal set is accumulated, never subtracted.** It holds
///    only what DOWNSTREAM handlers do. Seeding it with the root's
///    own effects and subtracting them back out cannot express
///    provenance: one `syscall` member subtracted once erases the
///    downstream occurrence along with the local one, and the
///    undeclared causal effect disappears. (The evaluator has this
///    bug; the model is the canonical authority and does not
///    inherit it.)
///  * **Incompleteness survives as `Uncertified`.** An unclassified
///    handler, a computed publish subject, an unfollowable call, a
///    truncated stdlib interior — each means the causal set is a
///    lower bound, not an answer. A known excess still wins as
///    `Violated`; otherwise relevant uncertainty refuses to certify.
///  * **Delivery is the typed wire identity**, via
///    [`possibly_delivers`], including wildcard coverage and
///    publishes that happen inside stdlib bodies (the absorption
///    interiors). Comparing rendered subject text missed ordinary
///    deliveries between a topic-name publish and a literal
///    subscribe on the same wire.
/// One claim row's own span, in bundle-global offsets — the anchor
/// both the evaluator and the judgment put a row's diagnostic on,
/// and therefore the key a differential joins one law to one law by.
pub fn claim_row_span(
    table: &ClaimIrTable,
    source_bases: &[u32],
    row: &hale_model::ClaimRow,
) -> Span {
    span_of(&table.provenance, source_bases, row.provenance)
}

/// What one `causes:` row's traversal actually reached.
///
/// The differential needs this to authorize a divergence against
/// THIS row rather than against facts anywhere in the program (a
/// hole on an unrelated topic must not excuse a wrong answer here).
/// Recomputing the closure inside the test would re-implement the
/// engine in its own test, which is how a test starts agreeing with
/// a bug — so the engine reports what it saw.
#[derive(Clone, Debug, Default)]
pub struct CausesWitness {
    /// Handlers a publish on this row's closure may deliver to.
    pub reached_handlers: Vec<FunctionId>,
    /// Handlers among those whose effect set is not fully known.
    pub unknown_handlers: Vec<FunctionId>,
    /// Endpoints whose downstream is incomplete (subscriber set,
    /// key filter, or an opaque external boundary).
    pub incomplete_endpoints: Vec<hale_model::SubjectId>,
    /// Functions on the closure whose outgoing calls or publish
    /// sites are not fully known — an unfollowable call or a
    /// computed subject means the publish set is a lower bound.
    pub incomplete_discovery: Vec<FunctionId>,
    /// The closure left user code through a stdlib interior.
    pub crossed_stdlib_interior: bool,
    /// A publish on the closure is routed off-process by a typed
    /// outbound binding — the peer's behaviour is outside this
    /// model even though the transport is understood.
    pub crosses_external_route: bool,
    /// The closure took more than one bus hop.
    pub multi_hop: bool,
}

pub fn judge_causes(
    table: &ClaimIrTable,
    model: &ApplicationModel,
    source_bases: &[u32],
) -> Vec<Judged> {
    judge_causes_witnessed(table, model, source_bases)
        .into_iter()
        .map(|(j, _)| j)
        .collect()
}

/// [`judge_causes`], with each row's traversal witness.
pub fn judge_causes_witnessed(
    table: &ClaimIrTable,
    model: &ApplicationModel,
    source_bases: &[u32],
) -> Vec<(Judged, CausesWitness)> {
    let e = &model.entities;
    let r = &model.relations;
    let claim_span = |pid: ProvenanceId| -> Span {
        span_of(&table.provenance, source_bases, pid)
    };
    // A function's DERIVED classes, and whether they are known at
    // all. `unclassified` is the analysis saying "this body reaches
    // something I cannot name" — it is not the empty set.
    let effects_of =
        |f: FunctionId| crate::model_query::effects_of(e, f);
    // Which relation families a hole at a function hides.
    let hole_hides = |f: FunctionId, mask: hale_model::RelationSet| {
        model.holes.iter().any(|h| {
            h.at == EntityRef::Function(f) && h.hides.intersects(mask)
        })
    };
    // Interior stdlib publishes, keyed by the user fn they are
    // reached from, plus whether that interior is fully explored.
    let mut interior_pubs: BTreeMap<u32, Vec<hale_model::SubjectId>> =
        BTreeMap::new();
    let mut interior_unknown: BTreeSet<u32> = BTreeSet::new();
    for a in &model.analyses.stdlib_absorption {
        for node in &a.nodes {
            for ev in &node.events {
                match ev {
                    hale_model::AbsorbedEvent::Publish {
                        subject,
                        declared_topic,
                        ..
                    } => {
                        // Interior publishes name a subject by wire
                        // text (or a declared topic); resolve to the
                        // typed id the delivery query joins on.
                        let sid = declared_topic
                            .map(|t| e.topics[t.index()].subject)
                            .or_else(|| {
                                e.subjects
                                    .iter()
                                    .position(|s| s.pattern == *subject)
                                    .map(|i| {
                                        hale_model::SubjectId(i as u32)
                                    })
                            });
                        match sid {
                            Some(sid) => interior_pubs
                                .entry(a.from.0)
                                .or_default()
                                .push(sid),
                            // A subject the model has no row for is
                            // an address it cannot reason about.
                            None => {
                                interior_unknown.insert(a.from.0);
                            }
                        }
                    }
                    hale_model::AbsorbedEvent::PublishHole
                    | hale_model::AbsorbedEvent::Truncated
                    | hale_model::AbsorbedEvent::CallHole(_) => {
                        interior_unknown.insert(a.from.0);
                    }
                    hale_model::AbsorbedEvent::Call { .. } => {}
                }
            }
        }
    }
    let mut callees: BTreeMap<u32, Vec<FunctionId>> = BTreeMap::new();
    for c in &r.calls {
        callees.entry(c.from.0).or_default().push(c.to);
    }

    let mut out: Vec<(Judged, CausesWitness)> = Vec::new();
    for row in &table.rows {
        let ClaimIr::EffectCauses { at, classes } = &row.law else {
            continue;
        };
        let mut diags: Vec<Diag> = Vec::new();
        // An UNRESOLVED subject (round 3): a module-scoped body is
        // outside the analyzable universe, so this walk has nothing
        // to start from. Skipping the row left `judge_certificates`
        // to answer with a bare `uncertified` and no explanation —
        // silent on the check path, and evidence-less in the
        // artifact, which admission then refused.
        let Some(fid) = at.0 else {
            diags.push(Diag::ty(
                claim_span(row.provenance),
                format!(
                    "declared causal set cannot be certified: `{}` \
                     is outside the analyzable universe (a \
                     module-scoped body), so this walk has no \
                     starting point. Move the subject to the top \
                     level to have its causal closure checked.",
                    at.1.display
                ),
            ));
            out.push((
                Judged {
                    ordinal: row.ordinal,
                    verdict: Verdict::Uncertified,
                    diags,
                    foreign: Vec::new(),
                },
                CausesWitness::default(),
            ));
            continue;
        };
        // A class that resolves to nothing makes the contract
        // vacuous: invalid before evaluation, as in the certificate
        // family. The declaration owns the diagnostic.
        if classes.iter().any(|c| {
            c.class.is_some_and(|id| {
                matches!(
                    e.effect_classes[id.index()].definition,
                    hale_model::EffectClassDefinition::InvalidCycle
                )
            })
        }) {
            out.push((
                Judged {
                    ordinal: row.ordinal,
                    verdict: Verdict::Invalid,
                    diags,
                    foreign: Vec::new(),
                },
                CausesWitness::default(),
            ));
            continue;
        }

        // ---- the causal closure ----
        // Publishes reachable from the root (through calls), the
        // handlers they may deliver to, and — because a handler may
        // publish in turn — onward until it settles.
        let mut uncertain = false;
        let mut witness = CausesWitness::default();
        let mut caused: BTreeSet<String> = BTreeSet::new();
        let mut via: Vec<String> = Vec::new();
        let mut fn_frontier: Vec<FunctionId> = vec![fid];
        let mut fn_seen: BTreeSet<u32> = BTreeSet::new();
        let mut handled: BTreeSet<u32> = BTreeSet::new();
        while let Some(f) = fn_frontier.pop() {
            if !fn_seen.insert(f.0) {
                continue;
            }
            // Discovery incompleteness: an unfollowable call or a
            // computed subject means the publish set is a lower
            // bound.
            if hole_hides(
                f,
                hale_model::RelationSet::CALLS
                    .union(hale_model::RelationSet::PUBLISHES),
            ) || interior_unknown.contains(&f.0)
            {
                uncertain = true;
                witness.incomplete_discovery.push(f);
            }
            if interior_pubs.contains_key(&f.0)
                || interior_unknown.contains(&f.0)
            {
                witness.crossed_stdlib_interior = true;
            }
            if f != fid {
                witness.multi_hop = true;
            }
            // (subject id, AUTHORED spelling). The join below is on
            // the id; the witness renders what the author wrote —
            // `Orders`, not `orders` — because a path a developer
            // reads must name the declaration they can go look at.
            let mut published: Vec<(
                hale_model::SubjectId,
                String,
                hale_model::Publish,
            )> = r
                .publishes
                .iter()
                .filter(|p| p.function == f)
                .map(|p| {
                    let written = match p.declared_topic {
                        Some(t) => e.topics[t.index()].display.clone(),
                        None => e.subjects[p.subject.index()]
                            .pattern
                            .clone(),
                    };
                    (p.subject, written, p.clone())
                })
                .collect();
            // An interior stdlib publish has no key domain the model
            // records, so it widens like any unknown one.
            published.extend(
                interior_pubs.get(&f.0).into_iter().flatten().map(|s| {
                    (
                        *s,
                        e.subjects[s.index()].pattern.clone(),
                        hale_model::Publish {
                            function: f,
                            subject: *s,
                            declared_topic: None,
                            payload: hale_model::PayloadContractId(0),
                            site: 0,
                            in_loop: false,
                            key_domain: None,
                            disposition:
                                hale_model::keys::PublishDisposition::Default,
                            provenance: hale_model::ProvenanceId(0),
                        },
                    )
                }),
            );
            // NOT deduplicated. Publishes are site-grained on
            // purpose: one function may publish one subject at
            // several sites with different key domains, and
            // collapsing them before the delivery query throws away
            // the site that actually delivers. Handler facts are
            // deduplicated AFTER every site has been evaluated
            // (review round 3).
            published.sort_by(|a, b| {
                (a.0, &a.1, a.2.site).cmp(&(b.0, &b.1, b.2.site))
            });
            for (subject, written, pubrow) in published {
                // Delivery incompleteness, SCOPED TO THIS ENDPOINT.
                // A hole somewhere else in the application says
                // nothing about this causal closure — the model's
                // hole law is reachability-scoped, and a global
                // check turns an unrelated adapter binding into
                // uncertainty about a purely local publish.
                //
                // The relation that matters is SUBSCRIBES (is the
                // set of possible handlers complete?) rather than
                // DELIVERY (the must-deliver guarantee), plus
                // KEY_FILTERS, since an unknown filter widens who
                // may receive.
                // ONE question, asked in one place: is what lies
                // beyond this endpoint fully modeled? Holes about
                // the handler set or its filters, an opaque
                // boundary, and a typed outbound route that leaves
                // the application are all the same answer, and
                // scoping them to this endpoint is what keeps an
                // unrelated binding from poisoning a local law.
                if crate::model_query::endpoint_incomplete(
                    model,
                    subject,
                    crate::model_query::Direction::Downstream,
                ) {
                    uncertain = true;
                    witness.incomplete_endpoints.push(subject);
                }
                for su in r
                    .subscribes
                    .iter()
                    .filter(|su| {
                        crate::model_query::may_deliver(e, &pubrow, su)
                    })
                {
                    let (eff, unknown) = effects_of(su.handler);
                    witness.reached_handlers.push(su.handler);
                    if unknown {
                        witness.unknown_handlers.push(su.handler);
                        // The handler reaches something unnameable:
                        // whatever it causes is not measurable here.
                        uncertain = true;
                    }
                    if handled.insert(su.handler.0) && !eff.is_empty() {
                        via.push(format!(
                            "`{}` -> subject `{}` -> `{}`",
                            e.functions[fid.index()].display,
                            written,
                            e.functions[su.handler.index()].display
                        ));
                    }
                    caused.extend(eff);
                    // Onward hops: what this handler publishes is
                    // also caused by the root.
                    fn_frontier.push(su.handler);
                }
            }
            for next in callees.get(&f.0).into_iter().flatten() {
                fn_frontier.push(*next);
            }
        }

        let mut allowed: BTreeSet<String> = BTreeSet::new();
        for c in classes {
            allowed.extend(effect_class_atoms(e, &c.name));
        }
        let excess: BTreeSet<String> = caused
            .into_iter()
            .filter(|c| !allowed.contains(c))
            .collect();
        let verdict = if !excess.is_empty() {
            // A KNOWN excess is an answer, whatever else is unknown.
            diags.push(Diag::ty(
                claim_span(row.provenance),
                format!(
                    "declared causal set violated: `{}` can transitively \
                     cause {} through the bus, which its \
                     `@effects(causes: …)` does not declare.{} Add the \
                     class to the declaration, or route the publish to a \
                     subject whose subscribers don't perform it.",
                    e.functions[fid.index()].display,
                    render_effect_classes(e, &excess).join(", "),
                    if via.is_empty() {
                        String::new()
                    } else {
                        format!(" Path: {}.", via.join("; "))
                    }
                ),
            ));
            Verdict::Violated
        } else if uncertain {
            // Round 3: an uncertified row without an explanation was
            // SILENT on the check path — `claim_law_diags` appends
            // diagnostics, never verdicts — so a law that could not
            // be certified compiled clean while the artifact marked
            // the document `law_failed`. That is exactly the
            // check/artifact disagreement this epic removes. It also
            // left the row with no evidence, which admission (which
            // requires a non-holds migrated row to retain its
            // judgment's evidence) then refused.
            //
            // The witness already knows WHY; say it.
            let mut why: Vec<String> = Vec::new();
            if !witness.unknown_handlers.is_empty() {
                let names: Vec<&str> = witness
                    .unknown_handlers
                    .iter()
                    .map(|h| e.functions[h.index()].display.as_str())
                    .collect();
                why.push(format!(
                    "the effects of {} are not fully classified",
                    names
                        .iter()
                        .map(|n| format!("`{}`", n))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if !witness.incomplete_endpoints.is_empty() {
                let subs: Vec<String> = witness
                    .incomplete_endpoints
                    .iter()
                    .map(|s| {
                        format!(
                            "`{}`",
                            e.subjects[s.index()].pattern
                        )
                    })
                    .collect();
                why.push(format!(
                    "the set of subscribers to {} is not fully \
                     modelled (an unknown filter, an opaque \
                     boundary, or a route that leaves this \
                     application)",
                    subs.join(", ")
                ));
            }
            if !witness.incomplete_discovery.is_empty() {
                why.push(
                    "a call this walk could not follow may publish \
                     further"
                        .to_string(),
                );
            }
            if why.is_empty() {
                why.push(
                    "part of the causal closure is not modelled"
                        .to_string(),
                );
            }
            diags.push(Diag::ty(
                claim_span(row.provenance),
                format!(
                    "declared causal set cannot be certified: `{}` \
                     publishes into a closure this model does not \
                     fully know — {}. The law is neither kept nor \
                     broken here; classify the effects, close the \
                     endpoint, or narrow the publish.",
                    e.functions[fid.index()].display,
                    why.join("; ")
                ),
            ));
            Verdict::Uncertified
        } else {
            Verdict::Holds
        };
        witness.reached_handlers.sort();
        witness.reached_handlers.dedup();
        witness.unknown_handlers.sort();
        witness.unknown_handlers.dedup();
        witness.incomplete_endpoints.sort();
        witness.incomplete_endpoints.dedup();
        witness.incomplete_discovery.sort();
        witness.incomplete_discovery.dedup();
        out.push((
            Judged {
                ordinal: row.ordinal,
                verdict,
                diags,
                foreign: Vec::new(),
            },
            witness,
        ));
    }
    out
}

/// The traversal witness a `depends:` row leaves behind — what the
/// backward walk reached, and everywhere it could not see.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct DependsWitness {
    /// Every subject that can reach this locus, in the order the
    /// walk settled them.
    pub reached_subjects: Vec<hale_model::SubjectId>,
    /// Reached subjects whose upstream is not fully modeled.
    pub incomplete_endpoints: Vec<hale_model::SubjectId>,
    /// Params typed as a `sync`-discipline form: input channels
    /// outside the message graph entirely.
    pub sync_form_params: Vec<String>,
}

/// GH #476 Change 5g — `@effects(depends: {A, B})` on a locus,
/// judged over the canonical model.
///
/// The backward dual of [`judge_causes`], and deliberately built
/// from the same shared queries: `causes:` asks what a publish can
/// REACH, `depends:` asks what can reach a subscription. Running
/// them on different joins is exactly the defect
/// [`crate::model_query`] exists to prevent — a dependence routed
/// through one republishing intermediary is invisible in the
/// depending locus's own `bus {}` block, so the walk has to be
/// transitive and it has to agree with delivery.
///
/// It is a COMPLETE declaration: every subject that can transitively
/// reach any handler this locus owns must be named. Reachability,
/// not dataflow — a locus subscribing to a laundered republish of S
/// depends on S whether or not it reads the field.
pub fn judge_depends(
    table: &ClaimIrTable,
    model: &ApplicationModel,
    source_bases: &[u32],
) -> Vec<Judged> {
    judge_depends_witnessed(table, model, source_bases)
        .into_iter()
        .map(|(j, _)| j)
        .collect()
}

/// [`judge_depends`], with each row's traversal witness.
pub fn judge_depends_witnessed(
    table: &ClaimIrTable,
    model: &ApplicationModel,
    source_bases: &[u32],
) -> Vec<(Judged, DependsWitness)> {
    let e = &model.entities;
    let r = &model.relations;
    let claim_span = |pid: ProvenanceId| -> Span {
        span_of(&table.provenance, source_bases, pid)
    };
    // locus -> the functions it owns. `depends:` is a claim about a
    // LOCUS, but subscription is a fact about a FUNCTION, and
    // `member_of` is the only bridge.
    let mut fns_of: BTreeMap<u32, Vec<FunctionId>> = BTreeMap::new();
    for m in &r.member_of {
        fns_of.entry(m.locus.0).or_default().push(m.function);
    }
    // Reverse CALLS. A publish inside a free helper belongs to
    // every locus that can reach that helper, so the backward walk
    // has to climb the call graph before it can ask "whose inputs
    // are these?".
    let mut callers_of: BTreeMap<u32, Vec<FunctionId>> =
        BTreeMap::new();
    for c in &r.calls {
        callers_of.entry(c.to.0).or_default().push(c.from);
    }
    // …and through the stdlib: a user fn that reaches a user fn
    // through a stdlib interior is still a caller.
    for a in &model.analyses.stdlib_absorption {
        for node in &a.nodes {
            for ev in &node.events {
                if let hale_model::AbsorbedEvent::Call {
                    target: hale_model::AbsorbedTarget::User(u),
                    ..
                } = ev
                {
                    callers_of.entry(u.0).or_default().push(a.from);
                }
            }
        }
    }
    let owner_of: BTreeMap<u32, hale_model::LocusDeclId> = r
        .member_of
        .iter()
        .map(|m| (m.function.0, m.locus))
        .collect();
    // Every locus that owns a function which can transitively reach
    // `f` — including `f`'s own owner.
    // Functions whose publish set the model could not resolve.
    let computed_publishers: BTreeSet<u32> = model
        .holes
        .iter()
        .filter(|h| h.hides.intersects(hale_model::RelationSet::PUBLISHES))
        .filter_map(|h| match h.at {
            EntityRef::Function(f) => Some(f.0),
            _ => None,
        })
        .collect();
    let owning_loci = |f: FunctionId| -> Vec<hale_model::LocusDeclId> {
        let mut seen_fn: BTreeSet<u32> = BTreeSet::new();
        let mut out: BTreeSet<u32> = BTreeSet::new();
        let mut frontier = vec![f];
        while let Some(cur) = frontier.pop() {
            if !seen_fn.insert(cur.0) {
                continue;
            }
            if let Some(l) = owner_of.get(&cur.0) {
                out.insert(l.0);
                // A method's own locus is an answer; its callers
                // still matter (another locus may call into it).
            }
            for up in callers_of.get(&cur.0).into_iter().flatten() {
                frontier.push(*up);
            }
        }
        out.into_iter().map(hale_model::LocusDeclId).collect()
    };

    let mut out = Vec::new();
    for row in table.rows.iter() {
        let ClaimIr::DependsSet { locus, entries } = &row.law else {
            continue;
        };
        let mut witness = DependsWitness::default();
        let mut diags: Vec<Diag> = Vec::new();

        // Static invalidity DOMINATES: an operand that names
        // nothing cannot be certified against, and a replayed
        // engine result is never an alternative.
        let Some(lid) = locus.0 else {
            out.push((
                Judged {
                    ordinal: row.ordinal,
                    verdict: Verdict::Invalid,
                    diags,
                    foreign: Vec::new(),
                },
                witness,
            ));
            continue;
        };
        let decl = &e.loci[lid.index()];

        // Obligation 1 (#340): a param typed as a form carrying a
        // `sync` discipline. Another pool writes it, this locus
        // reads it, and NO bus edge records the transfer — so the
        // message graph, which is all this walk can see, cannot
        // support a completeness claim at all.
        for prm in &decl.params {
            let holds_sync = prm
                .decl
                .is_some_and(|d| e.loci[d.index()].sync_form);
            if !holds_sync {
                continue;
            }
            witness.sync_form_params.push(prm.name.clone());
            diags.push(Diag::ty(
                claim_span(row.provenance),
                format!(
                    "declared dependency set is incomplete: `{}` holds \
                     `{}` as `{}`, a form carrying a `sync` discipline \
                     — shared state another pool can write. That is an \
                     input channel outside the bus graph, and \
                     `depends:` closes over the message graph only.",
                    decl.display, prm.type_name, prm.name
                ),
            ));
        }

        // Author spelling for a wire subject: the topic that
        // declares it, when exactly one does. Identity stays the
        // `SubjectId` — this is only how it is SHOWN, the same
        // raw/display duality the topics section carries.
        let shown = |sid: u32| -> String {
            let subject = &e.subjects[sid as usize];
            let mut named = e
                .topics
                .iter()
                .filter(|t| t.subject.0 == sid)
                .map(|t| t.display.clone());
            match (named.next(), named.next()) {
                (Some(one), None) => one,
                _ => subject.pattern.clone(),
            }
        };
        // Obligation 2: the backward closure. BFS over LOCI,
        // remembering how each subject was reached so the
        // diagnostic can name the path and not only the verdict.
        let mut uncertain = false;
        // subject -> (via locus display, into subject display)
        let mut seen: BTreeMap<u32, Option<(String, String)>> =
            BTreeMap::new();
        let mut seen_locus: BTreeSet<u32> = BTreeSet::new();
        seen_locus.insert(lid.0);
        // (subject reaching the frontier locus, how it got there)
        let mut queue: Vec<(hale_model::SubjectId, Option<(String, String)>)> =
            Vec::new();
        let subs_of = |l: u32| -> Vec<hale_model::Subscribe> {
            let owned = fns_of.get(&l).cloned().unwrap_or_default();
            r.subscribes
                .iter()
                .filter(|su| owned.contains(&su.handler))
                .cloned()
                .collect()
        };
        for su in subs_of(lid.0) {
            queue.push((su.subject, None));
        }
        while let Some((subject, via)) = queue.pop() {
            if seen.contains_key(&subject.0) {
                continue;
            }
            seen.insert(subject.0, via);
            witness.reached_subjects.push(subject);
            // Is this endpoint's UPSTREAM fully modeled? Holes over
            // who publishes it, an opaque delivery boundary, and a
            // `listen` binding that accepts from a peer are all the
            // same answer — and scoping the question to THIS
            // subject is what keeps an unrelated adapter from
            // poisoning a purely local claim.
            if crate::model_query::endpoint_incomplete(
                model,
                subject,
                crate::model_query::Direction::Upstream,
            ) {
                uncertain = true;
                witness.incomplete_endpoints.push(subject);
            }
            // Function-grained publisher residue (round 2). A
            // COMPUTED publish is recorded as a PUBLISHES hole at
            // its function, not as a `Publish` row and not anchored
            // to any subject — so neither the endpoint query (which
            // reads subject- and topic-anchored holes) nor the row
            // walk below can see it. A publisher whose subject the
            // model could not name might name THIS wire, and a
            // backward completeness claim cannot be certified over
            // it.
            //
            // Deliberately not scoped to this subject: the hole
            // exists precisely because the subject is unknown, so
            // there is nothing narrower to scope it to. It
            // downgrades to `uncertified`, never to a violation.
            if !computed_publishers.is_empty() {
                uncertain = true;
            }
            // Who can publish INTO a subscription on this subject?
            // The join is delivery, not the syntactic topic link: a
            // literal `"t" <- …` send reaches a `t` subscriber, and
            // a keyed publish that cannot meet this predicate is
            // not an upstream at all.
            for su in r.subscribes.iter() {
                if !crate::model_query::subscription_covers(
                    e, su, subject,
                ) {
                    continue;
                }
                for p in r.publishes.iter() {
                    if !crate::model_query::may_deliver(e, p, su) {
                        continue;
                    }
                    // Every LOCUS this publish can be reached FROM,
                    // not just the one that owns the publishing
                    // function.
                    //
                    // Round 2: the walk used to stop at a free
                    // function, recording its subject and giving up
                    // because there was no owner locus to inspect.
                    // But a handler that calls a free helper which
                    // publishes is a real path — `Secret ->
                    // Relay::on_secret -> emit_clean() -> Clean ->
                    // Target::on_clean` — and stopping there let a
                    // locus certify `depends: {Clean}` while
                    // `Secret` reached it. The publish site's
                    // CALLERS are part of the backward walk.
                    for owner in owning_loci(p.function) {
                        let plocus = &e.loci[owner.index()];
                        if !seen_locus.insert(owner.0) {
                            continue;
                        }
                        for up in subs_of(owner.0) {
                            queue.push((
                                up.subject,
                                Some((
                                    plocus.display.clone(),
                                    shown(subject.0),
                                )),
                            ));
                        }
                    }
                    // A publish no locus can be reached from still
                    // delivers here — record the subject even when
                    // the walk cannot continue past it.
                    if !seen.contains_key(&p.subject.0) {
                        queue.push((
                            p.subject,
                            Some((
                                e.functions[p.function.index()]
                                    .display
                                    .clone(),
                                shown(subject.0),
                            )),
                        ));
                    }
                }
            }
        }

        // Which reached subjects the declaration does NOT name. A
        // selector resolves to typed ids at lowering, so this is an
        // id comparison — no name matching, and no second answer to
        // "what does this selector mean".
        let declared: BTreeSet<u32> = entries
            .iter()
            .flat_map(|sel| {
                sel.subjects
                    .iter()
                    .map(|s| s.0)
                    .chain(sel.topics.iter().map(|t| {
                        e.topics[t.index()].subject.0
                    }))
                    .collect::<Vec<_>>()
            })
            .collect();
        // An entry that resolved to nothing names no subject in
        // this application. Static invalidity DOMINATES: the old
        // engine matched declared entries by NAME, so an unresolved
        // one silently covered nothing and every reached subject
        // came back as an omission — a violation report about the
        // subjects, when the actual defect is the typo. A law whose
        // operands do not resolve cannot be certified against, and
        // it cannot be violated either.
        let unresolved: Vec<&hale_model::BusSelector> = entries
            .iter()
            .filter(|sel| sel.subjects.is_empty() && sel.topics.is_empty())
            .collect();
        if !unresolved.is_empty() {
            let names: Vec<String> =
                unresolved.iter().map(|s| s.name.clone()).collect();
            diags.push(Diag::ty(
                claim_span(row.provenance),
                format!(
                    "declared dependency set is invalid: `{}` names `{}`, which is not a topic or subject in this application — so the set constrains nothing and cannot be judged. Name a declared topic, or remove the entry.",
                    decl.display,
                    names.join("`, `")
                ),
            ));
            out.push((
                Judged {
                    ordinal: row.ordinal,
                    verdict: Verdict::Invalid,
                    diags,
                    foreign: Vec::new(),
                },
                witness,
            ));
            continue;
        }

        let mut undeclared: Vec<(u32, Option<(String, String)>)> = seen
            .iter()
            .filter(|(sid, _)| !declared.contains(sid))
            .map(|(sid, via)| (*sid, via.clone()))
            .collect();
        undeclared.sort_by_key(|(sid, _)| *sid);
        for (sid, via) in &undeclared {
            let path = match via {
                Some((locus, into)) => format!(
                    " Path: subject `{}` -> `{}` -> subject `{}` -> `{}`.",
                    shown(*sid),
                    locus,
                    into,
                    decl.display
                ),
                None => format!(
                    " It is subscribed directly by `{}`.",
                    decl.display
                ),
            };
            diags.push(Diag::ty(
                claim_span(row.provenance),
                format!(
                    "declared dependency set violated: `{}` can \
                     transitively depend on `{}` through the bus, which \
                     its `@effects(depends: …)` does not declare.{} Add \
                     the subject to the set, or route the input through \
                     a subject this locus doesn't reach.",
                    decl.display,
                    shown(*sid),
                    path
                ),
            ));
        }

        // A KNOWN excess is an answer, whatever else is unknown —
        // the same precedence `causes:` settled on in round 2.
        let verdict = if !diags.is_empty() {
            Verdict::Violated
        } else if uncertain {
            Verdict::Uncertified
        } else {
            Verdict::Holds
        };
        witness.incomplete_endpoints.sort();
        witness.incomplete_endpoints.dedup();
        out.push((
            Judged {
                ordinal: row.ordinal,
                verdict,
                diags,
                foreign: Vec::new(),
            },
            witness,
        ));
    }
    out
}
