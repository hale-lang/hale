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
//! `model_graph::search` — the same engine both existing tiers
//! share — with `FunctionId` vertices over model rows: `calls`
//! (Direct/Interface site rows plus the ViaStdlib contraction),
//! the publish × subscribe composition per subject, `member_of` ∩
//! the summary universe for group projection, `phase_of` for
//! `during`, and typed holes (`IndirectCall` /
//! `UntypedReceiver { callee }` / `ComputedSubject`) for the
//! fail-closed edges.

use std::collections::{BTreeMap, BTreeSet};

use hale_model::{
    ApplicationModel, ClaimIr, ClaimIrTable, DispatchKind, EntityRef,
    FunctionId, GroupId, HoleKind, Provenance, ProvenanceId, SetIr,
};
use hale_syntax::{Diag, Span};

use crate::model_graph::{self, HolePolicy, Visit};
use crate::verdict::Verdict;

/// One judged law row: the ClaimIr ordinal, the verdict, and the
/// diagnostics — byte-compatible with the authoritative
/// evaluator's for the migrated family.
#[derive(Debug)]
pub struct Judged {
    pub ordinal: u32,
    pub verdict: Verdict,
    pub diags: Vec<Diag>,
}

/// One walk step, mirroring the evaluator's `Step` — enough to
/// render the witness identically.
#[derive(Clone, Debug)]
enum StepIr {
    Call {
        via_interface: Option<String>,
        provenance: ProvenanceId,
        /// A contracted through-stdlib edge: the crossing happens
        /// inside a non-bundle body, so the evaluator's
        /// `is_bundle_fn` gate suppresses the crossing-edge diag.
        via_stdlib: bool,
        /// Interior stdlib path (display, dispatch) between the
        /// caller and the target, for witness rendering — absorbed
        /// edges only.
        interior: Vec<(String, Option<(String, String)>)>,
        /// Dispatch rendering of the final step INTO the target.
        into_to: Option<(String, String)>,
    },
    Bus {
        subject: String,
        publish_provenance: ProvenanceId,
        subscribe_provenance: ProvenanceId,
    },
}

/// Resolve a model provenance record back to the evaluator's
/// bundle-global span. `source_bases[id]` is the base offset of
/// provenance source `id` (derived from `Bundle::sources` by the
/// caller). Synthetic records resolve to a zero span — parity for
/// them is settled by the differential.
fn span_of(
    model_prov: &hale_model::ProvenanceTable,
    bases: &[u32],
    pid: ProvenanceId,
) -> Span {
    match model_prov.records.get(pid.index()) {
        Some(Provenance::Source { source, span }) => {
            let base =
                bases.get(source.index()).copied().unwrap_or(0);
            Span::new(
                (base + span.0) as usize,
                (base + span.1) as usize,
            )
        }
        _ => Span::new(0, 0),
    }
}

/// Judge every `forbid reaches` row of one lowered law table
/// against its model (GH #476 Change 5a).
pub fn judge_forbid_reaches(
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
    let fn_display =
        |f: FunctionId| e.functions[f.index()].display.clone();

    // The summary universe: only these functions are walk vertices
    // (the evaluator walks summary keys; module-scoped declarations
    // hole out separately).
    let v1: BTreeSet<FunctionId> =
        model.legacy.topology_v1_fns.iter().copied().collect();

    // Group → fn-grain projection (the evaluator's `fn_set`).
    let mut group_fns: BTreeMap<GroupId, BTreeSet<FunctionId>> =
        BTreeMap::new();
    {
        let mut by_locus: BTreeMap<u32, Vec<FunctionId>> =
            BTreeMap::new();
        for mo in &r.member_of {
            by_locus
                .entry(mo.locus.0)
                .or_default()
                .push(mo.function);
        }
        for gm in &r.group_members {
            let set = group_fns.entry(gm.group).or_default();
            match gm.member {
                EntityRef::LocusDecl(l) => {
                    for f in
                        by_locus.get(&l.0).into_iter().flatten()
                    {
                        if v1.contains(f) {
                            set.insert(*f);
                        }
                    }
                }
                EntityRef::Function(f) => {
                    if v1.contains(&f) {
                        set.insert(f);
                    }
                }
                _ => {}
            }
        }
        // A group with declared members and no summary projection
        // still needs an entry (vacuity fires elsewhere).
        for (gi, _) in e.groups.iter().enumerate() {
            group_fns
                .entry(GroupId(gi as u32))
                .or_default();
        }
    }

    // Per-fn adjacency, in the evaluator's edge order: direct
    // calls by authored site, then the ViaStdlib contraction, then
    // publish sites by authored site fanned out to subscribers.
    let mut calls_of: BTreeMap<
        FunctionId,
        Vec<(u32, FunctionId, StepIr)>,
    > = BTreeMap::new();
    for c in &r.calls {
        let step = match &c.dispatch {
            DispatchKind::Direct => StepIr::Call {
                via_interface: None,
                provenance: c.provenance,
                via_stdlib: false,
                interior: Vec::new(),
                into_to: None,
            },
            DispatchKind::Interface { interface } => StepIr::Call {
                via_interface: Some(
                    e.interfaces
                        .iter()
                        .find(|i| i.name == *interface)
                        .map(|i| i.display.clone())
                        .unwrap_or_else(|| interface.clone()),
                ),
                provenance: c.provenance,
                via_stdlib: false,
                interior: Vec::new(),
                into_to: None,
            },
            // ViaStdlib rows are the LEGACY contraction — the
            // absorption sidecar replaces them for judgment (it
            // carries interior witnesses and holes); skip to avoid
            // double edges.
            DispatchKind::ViaStdlib => continue,
        };
        calls_of
            .entry(c.from)
            .or_default()
            .push((c.site, c.to, step));
    }
    for v in calls_of.values_mut() {
        v.sort_by_key(|(site, to, _)| (*site, *to));
    }
    // subject → subscriber handler rows (handler fn + provenance),
    // canonical order.
    let mut subs_of: BTreeMap<u32, Vec<(FunctionId, ProvenanceId)>> =
        BTreeMap::new();
    for su in &r.subscribes {
        subs_of
            .entry(su.subject.0)
            .or_default()
            .push((su.handler, su.provenance));
    }
    // fn → publish rows (site, subject id, display subject, prov).
    let mut pubs_of: BTreeMap<
        FunctionId,
        Vec<(u32, u32, String, ProvenanceId)>,
    > = BTreeMap::new();
    for p in &r.publishes {
        // The evaluator's Step::Bus subject is the site's WRITTEN
        // text: the topic name for declared sends, the literal for
        // literal sends — the display projection of the row.
        let written = match p.declared_topic {
            Some(t) => e.topics[t.index()].name.clone(),
            None => e.subjects[p.subject.index()].pattern.clone(),
        };
        pubs_of.entry(p.function).or_default().push((
            p.site,
            p.subject.0,
            written,
            p.provenance,
        ));
    }
    for v in pubs_of.values_mut() {
        v.sort_by_key(|(site, ..)| *site);
    }
    // (from, site) → absorption entries.
    let mut absorb_of: BTreeMap<
        FunctionId,
        Vec<&hale_model::StdlibAbsorption>,
    > = BTreeMap::new();
    for a in &model.legacy.stdlib_absorption {
        absorb_of.entry(a.from).or_default().push(a);
    }
    // fn → fail-closed holes, for the visitor.
    #[derive(Clone)]
    enum FnHole {
        Indirect,
        Untyped { callee: String },
        Computed,
    }
    let mut holes_of: BTreeMap<FunctionId, Vec<FnHole>> =
        BTreeMap::new();
    for h in &model.holes {
        let EntityRef::Function(f) = h.at else { continue };
        let hole = match &h.kind {
            HoleKind::IndirectCall => FnHole::Indirect,
            HoleKind::UntypedReceiver { callee } => {
                FnHole::Untyped {
                    callee: callee.clone(),
                }
            }
            HoleKind::ComputedSubject => FnHole::Computed,
            _ => continue,
        };
        holes_of.entry(f).or_default().push(hole);
    }
    // fn → phase name (during filter).
    let mut phase_of: BTreeMap<FunctionId, &str> = BTreeMap::new();
    for po in &r.phase_of {
        phase_of
            .insert(po.function, e.phases[po.phase.index()].name.as_str());
    }
    // fn → direct effect classes (effects(C) destination test).
    let direct_of = |f: FunctionId| -> &[String] {
        &e.functions[f.index()].direct_effects
    };
    // Composed-class expansion for the destination mask.
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
        // ---- the evaluator's validation pass (validate_claim's
        // ForbidReaches arm): unknown names, effects-in-source,
        // undeclared classes, and the avoiding-overlap guard. Any
        // failure = Invalid, evaluation skipped. ----
        let mut ok = true;
        let group_decl_names: Vec<&str> =
            e.groups.iter().map(|g| g.display.as_str()).collect();
        let mut check_group =
            |gref: &hale_model::GroupRef, diags: &mut Vec<Diag>| -> bool {
                if gref.group.is_some() {
                    return true;
                }
                let mut near: Vec<&&str> = group_decl_names
                    .iter()
                    .filter(|g| crate::effects::close(g, &gref.name.display))
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
            // `effects(...)` in source position: rejected by the
            // evaluator's validation with its exact spelling.
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
                // check_class: a USER class must be declared.
                if !c.builtin && c.class.map_or(true, |id| {
                    !e.effect_classes[id.index()].declared
                }) {
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
            // The overlap guard: DECL-grain member intersection
            // between the mask and each endpoint.
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
                                row.name, a.name.display, n.name.display
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
        // Projection vacuity (the evaluator's guard): a group whose
        // declarations project to no executable vertices proves
        // nothing while reading as law. Fail closed.
        let decl_count = |g: GroupId| {
            r.group_members.iter().filter(|gm| gm.group == g).count()
        };
        let mut vacuous =
            |gid: GroupId,
             gref: &hale_model::GroupRef,
             which: &str,
             diags: &mut Vec<Diag>|
             -> bool {
                if decl_count(gid) == 0
                    || !group_fns[&gid].is_empty()
                {
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
        let mask: Option<&BTreeSet<FunctionId>> = match avoiding {
            Some(a) => match a.group {
                Some(g) => Some(&group_fns[&g]),
                None => None,
            },
            None => None,
        };
        enum DstIr<'x> {
            Group(&'x BTreeSet<FunctionId>),
            Effects(BTreeSet<String>),
        }
        let dst_test = match dst {
            SetIr::Group(g) => match g.group {
                Some(gid) => DstIr::Group(&group_fns[&gid]),
                None => {
                    out.push(Judged {
                        ordinal: row.ordinal,
                        verdict: Verdict::Invalid,
                        diags,
                    });
                    continue;
                }
            },
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
        let search = model_graph::search(
            roots.iter().cloned(),
            |f: &FunctionId| {
                let mut edges: Vec<(FunctionId, StepIr)> = Vec::new();
                if *via_calls {
                    for h in
                        holes_of.get(f).into_iter().flatten()
                    {
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
                                        fn_display(*f),
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
                                        fn_display(*f),
                                        src_ref.name.display,
                                        callee
                                    ),
                                ));
                                return Visit::hole(());
                            }
                            FnHole::Computed => {}
                        }
                    }
                    // Interleave model call rows and absorbed
                    // stdlib consequences by authored site — the
                    // evaluator's BFS position.
                    let mut items: Vec<(u32, u8, usize)> = Vec::new();
                    let direct =
                        calls_of.get(f).map(|v| v.as_slice()).unwrap_or(&[]);
                    for (i, (site, _, _)) in
                        direct.iter().enumerate()
                    {
                        items.push((*site, 0, i));
                    }
                    let absorbed = absorb_of
                        .get(f)
                        .map(|v| v.as_slice())
                        .unwrap_or(&[]);
                    for (i, a) in absorbed.iter().enumerate() {
                        items.push((a.site, 1, i));
                    }
                    items.sort();
                    for (_, kind, i) in items {
                        if kind == 0 {
                            let (_, to, step) = &direct[i];
                            edges.push((*to, step.clone()));
                        } else {
                            let a = absorbed[i];
                            for h in &a.holes {
                                match &h.kind {
                                    hale_model::AbsorbedHoleKind::IndirectCall => {
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
                                                h.at_display,
                                                src_ref.name.display
                                            ),
                                        ));
                                        return Visit::hole(());
                                    }
                                    hale_model::AbsorbedHoleKind::OpaqueCall { callee } => {
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
                                                h.at_display,
                                                src_ref.name.display,
                                                callee
                                            ),
                                        ));
                                        return Visit::hole(());
                                    }
                                    hale_model::AbsorbedHoleKind::ComputedPublish => {
                                        if *via_bus {
                                            diags.push(Diag::ty(
                                                row_span,
                                                format!(
                                                    "claim `{}` cannot be certified: `{}` \
                                                     (reachable from `{}`) publishes to a \
                                                     computed subject, which could route to \
                                                     any subscriber. An unresolvable edge \
                                                     fails closed",
                                                    row.name,
                                                    h.at_display,
                                                    src_ref.name.display
                                                ),
                                            ));
                                            return Visit::hole(());
                                        }
                                    }
                                }
                            }
                            for ae in &a.edges {
                                edges.push((
                                    ae.to,
                                    StepIr::Call {
                                        via_interface: None,
                                        provenance: ProvenanceId(0),
                                        via_stdlib: true,
                                        interior: ae.interior.clone(),
                                        into_to: ae.into_to.clone(),
                                    },
                                ));
                            }
                        }
                    }
                }
                if *via_bus {
                    if holes_of
                        .get(f)
                        .into_iter()
                        .flatten()
                        .any(|h| matches!(h, FnHole::Computed))
                    {
                        diags.push(Diag::ty(
                            row_span,
                            format!(
                                "claim `{}` cannot be certified: `{}` \
                                 (reachable from `{}`) publishes to a \
                                 computed subject, which could route to \
                                 any subscriber. An unresolvable edge \
                                 fails closed",
                                row.name,
                                fn_display(*f),
                                src_ref.name.display
                            ),
                        ));
                        return Visit::hole(());
                    }
                    for (_, sid, written, ppid) in
                        pubs_of.get(f).into_iter().flatten()
                    {
                        for (handler, spid) in
                            subs_of.get(sid).into_iter().flatten()
                        {
                            edges.push((
                                *handler,
                                StepIr::Bus {
                                    subject: written.clone(),
                                    publish_provenance: *ppid,
                                    subscribe_provenance: *spid,
                                },
                            ));
                        }
                    }
                }
                Visit::edges(edges)
            },
            |f: &FunctionId| match &dst_test {
                DstIr::Group(g) => g.contains(f),
                DstIr::Effects(atoms) => direct_of(*f)
                    .iter()
                    .any(|c| atoms.contains(c)),
            },
            |f: &FunctionId| {
                mask.is_some_and(|m| m.contains(f))
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
                    e,
                    &fn_display,
                    row_span,
                    &mut diags,
                    &model_span,
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
            model_graph::Search::NotFound => Verdict::Holds,
        };
        out.push(Judged {
            ordinal: row.ordinal,
            verdict,
            diags,
        });
    }
    out
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
    hit: FunctionId,
    parent: &BTreeMap<FunctionId, (FunctionId, StepIr)>,
    e: &hale_model::Entities,
    fn_display: &dyn Fn(FunctionId) -> String,
    row_span: Span,
    diags: &mut Vec<Diag>,
    model_span: &dyn Fn(ProvenanceId) -> Span,
) {
    let mut rev: Vec<(FunctionId, Option<&StepIr>)> = Vec::new();
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
            None => {
                path.push_str(&format!("`{}`", fn_display(*node)))
            }
            Some(StepIr::Call {
                via_interface: Some(iface),
                ..
            }) => {
                let method = e.functions[node.index()]
                    .name
                    .rsplit("::")
                    .next()
                    .unwrap_or_default()
                    .to_string();
                path.push_str(&format!(
                    " -(dispatches {}.{})-> `{}`",
                    iface,
                    method,
                    fn_display(*node)
                ));
            }
            Some(StepIr::Call {
                interior, into_to, ..
            }) => {
                for (d, dsp) in interior {
                    match dsp {
                        Some((iface, method)) => {
                            path.push_str(&format!(
                                " -(dispatches {}.{})-> `{}`",
                                iface, method, d
                            ));
                        }
                        None => path
                            .push_str(&format!(" -> `{}`", d)),
                    }
                }
                match into_to {
                    Some((iface, method)) => {
                        path.push_str(&format!(
                            " -(dispatches {}.{})-> `{}`",
                            iface,
                            method,
                            fn_display(*node)
                        ));
                    }
                    None => path.push_str(&format!(
                        " -> `{}`",
                        fn_display(*node)
                    )),
                }
            }
            Some(StepIr::Bus { subject, .. }) => {
                path.push_str(&format!(
                    " -(publishes \"{}\")-> `{}`",
                    subject,
                    fn_display(*node)
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
    // #392 provenance: WHERE to edit — the crossing edge and the
    // destination's declaration, mirroring the evaluator's
    // `is_bundle_fn` gating (a ViaStdlib crossing lives in a
    // non-bundle body, so its span is suppressed).
    if let Some((_, Some(step))) = rev.last() {
        match step {
            StepIr::Call {
                provenance,
                via_interface,
                via_stdlib,
                ..
            } => {
                if !via_stdlib {
                    let msg = match via_interface {
                        Some(iface) => format!(
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
                    diags.push(Diag::ty(model_span(*provenance), msg));
                }
            }
            StepIr::Bus {
                publish_provenance,
                subscribe_provenance,
                ..
            } => {
                diags.push(Diag::ty(
                    model_span(*publish_provenance),
                    format!(
                        "claim `{}`: the crossing publish \
                         happens here",
                        row.name
                    ),
                ));
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
    // The destination's declaration: the hit's locus (or the free
    // fn itself), by display spelling — the evaluator renders the
    // raw decl and demangles afterwards, which lands on the same
    // string.
    let hit_fn = &e.functions[hit.index()];
    let (decl_disp, decl_prov) = match hit_fn.name.rsplit_once("::") {
        Some((locus_raw, _)) => {
            match e.loci.iter().find(|l| l.name == locus_raw) {
                Some(l) => {
                    (l.display.clone(), Some(l.provenance))
                }
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
