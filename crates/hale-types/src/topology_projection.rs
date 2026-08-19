//! GH #476 Change 3 — the model-backed artifact encoder.
//!
//! Projects an [`ApplicationModel`] down to the topology artifact's
//! HASHED model half — the exact byte string `dump_topology` hashes
//! as `shape_hash` (`TopologyShapeV1`). The epic's exit criterion:
//! this projection must reproduce the legacy serialization
//! byte-for-byte over the whole corpus BEFORE any cutover, so
//! existing `.halerec` recordings keep admitting and a topology
//! diff never moves because the derivation changed hands.
//!
//! Until the Change-6 versioned transition, BOTH derivations stay
//! live and `tests/topology_projection.rs` holds them byte-equal
//! over every corpus fixture — the same conformance-loop shape as
//! the effects manifest. A model-builder change that would alter
//! the artifact identity fails that differential loudly instead of
//! silently re-keying replay admission. At Change 6 the legacy
//! derivation and [`hale_model::LegacyProjection`] are deleted
//! together and the artifact becomes a pure projection of the
//! model.
//!
//! ## Projection rules (legacy artifact = DISPLAY spelling)
//!
//! The model keys everything by RAW post-merge symbol and carries
//! author spelling in `display` fields; the legacy artifact renders
//! displays. Specifically:
//!
//!  * `sorts.fns` / call endpoints — `Function.display`, restricted
//!    to `legacy.topology_v1_fns` (the behavior-summary universe;
//!    module-scoped and empty declarations exist in the model but
//!    not in the legacy sort).
//!  * `calls_via_stdlib` — `legacy.topology_v1_calls_via_stdlib`
//!    verbatim: the legacy one-Boolean no-revisit walk's rows, NOT
//!    the model's lattice rows (whose loop bits may be stronger).
//!  * publish/subscribe subjects — a declared endpoint renders its
//!    topic's display NAME (the legacy artifact never wrote wire
//!    subjects into relations); an undeclared endpoint renders its
//!    literal/authored pattern, which is author-spelled already.
//!  * `unknowns` — re-folded from typed holes plus
//!    `dead_interface_calls` (the model separates dead dispatches
//!    from genuine residue; the legacy section conflates them).

use std::collections::{BTreeMap, BTreeSet};

use hale_model::{
    ApplicationModel, DispatchKind, EntityRef, FunctionKind, HoleKind,
    SelectorForm, SupervisedRef,
};

use crate::topology::{fnv1a64, join_str, quote, trim_trailing_comma};

/// The projected `shape_hash` — FNV-1a/64 over
/// [`project_model_half`], exactly as `dump_topology` stamps it.
pub fn project_shape_hash(m: &ApplicationModel) -> u64 {
    fnv1a64(project_model_half(m).as_bytes())
}

/// Render the hashed model half of the topology artifact from the
/// model alone. Byte-compatible with the string `dump_topology`
/// builds internally (the substring `shape_hash` covers).
/// The V1 display map: raw post-merge symbol → author spelling,
/// over EVERY declaration table. The legacy encoder ran name()
/// (full-string demangle) unconditionally over subjects,
/// supervision child/error types, and more — including strings
/// that merely COLLIDE with an imported declaration's raw symbol
/// (a literal subject spelled like a mangled name demangles
/// under V1). EVERY projected section that renders an endpoint
/// subject applies this one map (rounds 11, 3 — the hashed half
/// and the provenance tail must join on identical spellings).
fn v1_display_map<'e>(
    e: &'e hale_model::Entities,
) -> BTreeMap<&'e str, &'e str> {
    let mut v1_display: BTreeMap<&'e str, &'e str> = BTreeMap::new();
    let mut add = |name: &'e str, display: &'e str| {
        if name != display {
            v1_display.insert(name, display);
        }
    };
    for l in &e.loci {
        add(&l.name, &l.display);
    }
    for t in &e.topics {
        add(&t.name, &t.display);
    }
    for t in &e.types {
        add(&t.name, &t.display);
    }
    for i in &e.interfaces {
        add(&i.name, &i.display);
    }
    for g in &e.groups {
        add(&g.name, &g.display);
    }
    for d in &e.declarations {
        add(&d.name, &d.display);
    }
    // FREE functions only: the legacy name() is an EXACT lookup
    // in import_renames, which holds renamed top-level
    // declarations — a method identity is never a renames key
    // (review round 12).
    for f in &e.functions {
        if f.kind == FunctionKind::Free {
            add(&f.name, &f.display);
        }
    }
    v1_display
}

/// The ONE V1 endpoint-subject renderer (round 3): a declared
/// endpoint renders its topic's display name; an undeclared
/// endpoint renders its authored pattern through the V1 display
/// map — the relation and provenance sections must join on
/// identical spellings.
fn v1_endpoint_subject(
    e: &hale_model::Entities,
    v1_display: &BTreeMap<&str, &str>,
    declared: Option<hale_model::TopicId>,
    subject: hale_model::SubjectId,
) -> String {
    match declared {
        Some(t) => e.topics[t.index()].display.clone(),
        None => {
            let raw = e.subjects[subject.index()].pattern.as_str();
            v1_display
                .get(raw)
                .map(|d| d.to_string())
                .unwrap_or_else(|| raw.to_string())
        }
    }
}

pub fn project_model_half<'a>(m: &'a ApplicationModel) -> String {
    let e = &m.entities;
    let r = &m.relations;

    let fn_display =
        |id: hale_model::FunctionId| e.functions[id.index()].display.clone();
    let locus_display =
        |id: hale_model::LocusDeclId| e.loci[id.index()].display.clone();
    let topic_display =
        |id: hale_model::TopicId| e.topics[id.index()].display.clone();
    let iface_display = |raw: &str| -> String {
        e.interfaces
            .iter()
            .find(|i| i.name == raw)
            .map(|i| i.display.clone())
            .unwrap_or_else(|| raw.to_string())
    };
    let v1_display = v1_display_map(e);
    let v1_name = |raw: &str| -> String {
        v1_display
            .get(raw)
            .map(|d| d.to_string())
            .unwrap_or_else(|| raw.to_string())
    };
    // The legacy fn universe: only these functions appear in the
    // artifact's fn-keyed sections.
    let v1: BTreeSet<hale_model::FunctionId> =
        m.legacy.topology_v1_fns.iter().copied().collect();

    // ---- sorts ----
    let loci: BTreeSet<String> =
        e.loci.iter().map(|l| l.display.clone()).collect();
    let fns: BTreeSet<String> = m
        .legacy
        .topology_v1_fns
        .iter()
        .map(|id| fn_display(*id))
        .collect();
    let topics: BTreeSet<String> =
        e.topics.iter().map(|t| t.display.clone()).collect();
    // RAW-name order with display values: the legacy encoder sorts
    // sealed loci by raw canonical name and demangles only while
    // serializing, so two imports whose raw and alias orders
    // disagree render displays in raw order (review round 13).
    // `e.loci` is canonically raw-sorted — collect WITHOUT
    // re-sorting.
    let sealed: Vec<String> = e
        .loci
        .iter()
        .filter(|l| l.sealed)
        .map(|l| l.display.clone())
        .collect();

    // ---- relations ----
    // Site-grained model rows merge to the legacy endpoint grain:
    // loop bits OR, any interface-dispatch site tags the edge.
    #[derive(Default)]
    struct EdgeMeta {
        looped: bool,
        unbounded: bool,
        // (authored site ordinal, display) — the legacy encoder
        // walks call sites in SOURCE order and last-writer-wins, so
        // when one (from, to) pair is dispatched through several
        // interfaces the winner is the greatest authored site, NOT
        // the lexicographically greatest interface the model's
        // canonical row order would visit last (review round 11).
        via_interface: Option<(u32, String)>,
    }
    let mut calls: BTreeMap<(String, String), EdgeMeta> = BTreeMap::new();
    for c in &r.calls {
        if c.dispatch == DispatchKind::ViaStdlib {
            continue;
        }
        // The legacy relation covers summary-resolved user→user
        // edges only; edges the model recovers into unanalyzed
        // callees (module-scoped bodies) have no legacy row.
        if !v1.contains(&c.from) || !v1.contains(&c.to) {
            continue;
        }
        let meta = calls
            .entry((fn_display(c.from), fn_display(c.to)))
            .or_default();
        meta.looped |= c.in_loop;
        meta.unbounded |= c.unbounded;
        if let DispatchKind::Interface { interface } = &c.dispatch {
            let cand = (c.site, iface_display(interface));
            if meta
                .via_interface
                .as_ref()
                .map(|(s, _)| cand.0 >= *s)
                .unwrap_or(true)
            {
                meta.via_interface = Some(cand);
            }
        }
    }
    let mut via_stdlib: BTreeMap<(String, String), bool> = BTreeMap::new();
    for (f, t, looped) in &m.legacy.topology_v1_calls_via_stdlib {
        let entry = via_stdlib
            .entry((fn_display(*f), fn_display(*t)))
            .or_insert(false);
        *entry |= *looped;
    }
    // Subjects: a declared endpoint renders the topic display NAME;
    // an undeclared endpoint renders its authored pattern.
    let subject_of = |declared: Option<hale_model::TopicId>,
                      subject: hale_model::SubjectId|
     -> String {
        v1_endpoint_subject(e, &v1_display, declared, subject)
    };
    let mut publishes: BTreeSet<(String, String)> = BTreeSet::new();
    for p in &r.publishes {
        if !v1.contains(&p.function) {
            continue;
        }
        publishes.insert((
            fn_display(p.function),
            subject_of(p.declared_topic, p.subject),
        ));
    }
    // handler locus via member_of; handler short name from the
    // canonical `Locus::method` identity (method names are never
    // path-qualified).
    let handler_locus: BTreeMap<hale_model::FunctionId, hale_model::LocusDeclId> =
        r.member_of.iter().map(|mo| (mo.function, mo.locus)).collect();
    let mut subscribes: BTreeSet<(String, String, String)> = BTreeSet::new();
    for s in &r.subscribes {
        let subj = subject_of(s.declared_topic, s.subject);
        let handler_short = e.functions[s.handler.index()]
            .name
            .rsplit("::")
            .next()
            .unwrap_or_default()
            .to_string();
        let locus = handler_locus
            .get(&s.handler)
            .map(|l| locus_display(*l))
            .unwrap_or_default();
        subscribes.insert((subj, locus, handler_short));
    }

    // ---- groups: authored selector lists, ordered ----
    let mut group_rows: BTreeMap<String, Vec<(u32, String)>> = BTreeMap::new();
    for g in &e.groups {
        group_rows.insert(g.display.clone(), Vec::new());
    }
    for sel in &r.group_selectors {
        let g = &e.groups[sel.group.index()];
        let display = match &sel.selector {
            SelectorForm::Named { display, .. } => display.clone(),
            SelectorForm::SeedGlob { display, .. } => display.clone(),
        };
        group_rows
            .entry(g.display.clone())
            .or_default()
            .push((sel.ordinal, display));
    }
    for members in group_rows.values_mut() {
        members.sort_by_key(|(ord, _)| *ord);
    }

    // ---- labels: declared effect carriers ----
    // Row order within one entity IS the artifact's class order
    // (render_effects_named order, preserved by the builder) — do
    // not re-sort it (review round 11).
    // Restricted to the legacy fn universe (the legacy encoder
    // iterates summary-keyed carriers): a lawful model may label a
    // non-V1 function, and that row must not appear in a section
    // whose fns are absent from sorts.fns (review round 12).
    let mut labels: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in &m.labels {
        if let EntityRef::Function(f) = row.at {
            if !v1.contains(&f) {
                continue;
            }
            labels
                .entry(fn_display(f))
                .or_default()
                .push(row.label.clone());
        }
    }

    // ---- phases ----
    // NO v1 filter: the legacy phase relation comes from the AST
    // walk (vmodel), which includes empty-bodied hooks the behavior
    // summary never keys — `run() { }` has a phase row and no fn
    // sort entry.
    let mut phase_rows: BTreeMap<String, (String, bool)> = BTreeMap::new();
    for po in &r.phase_of {
        let hook = matches!(
            e.functions[po.function.index()].kind,
            FunctionKind::Hook | FunctionKind::Mode
        );
        phase_rows.insert(
            fn_display(po.function),
            (e.phases[po.phase.index()].name.clone(), hook),
        );
    }

    // ---- seeds: alias → member displays, in RAW-name order ----
    // (the legacy sort iterates a BTreeSet of mangled names and
    // display-maps afterwards, so raw order is the authored fact).
    let raw_and_display = |er: &EntityRef| -> Option<(String, String)> {
        Some(match er {
            EntityRef::Function(id) => {
                let f = &e.functions[id.index()];
                (f.name.clone(), f.display.clone())
            }
            EntityRef::LocusDecl(id) => {
                let l = &e.loci[id.index()];
                (l.name.clone(), l.display.clone())
            }
            EntityRef::Topic(id) => {
                let t = &e.topics[id.index()];
                (t.name.clone(), t.display.clone())
            }
            EntityRef::Type(id) => {
                let t = &e.types[id.index()];
                (t.name.clone(), t.display.clone())
            }
            EntityRef::Interface(id) => {
                let i = &e.interfaces[id.index()];
                (i.name.clone(), i.display.clone())
            }
            EntityRef::Group(id) => {
                let g = &e.groups[id.index()];
                (g.name.clone(), g.display.clone())
            }
            EntityRef::Declaration(id) => {
                let d = &e.declarations[id.index()];
                (d.name.clone(), d.display.clone())
            }
            _ => return None,
        })
    };
    let mut seed_rows: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for s in &e.seeds {
        seed_rows.insert(s.name.clone(), Vec::new());
    }
    {
        let mut by_seed: BTreeMap<String, BTreeMap<String, String>> =
            BTreeMap::new();
        for di in &r.declared_in {
            let Some((raw, display)) = raw_and_display(&di.entity) else {
                continue;
            };
            by_seed
                .entry(e.seeds[di.seed.index()].name.clone())
                .or_default()
                .insert(raw, display);
        }
        for (alias, members) in by_seed {
            seed_rows.insert(
                alias,
                members.into_values().collect(),
            );
        }
    }

    // ---- effects: derived per-fn classes, declaration order ----
    // Same restriction: the legacy effects section iterates the
    // stdlib-merged summary's user keys — the V1 universe — never
    // the model's broader declaration universe (review round 12).
    let mut derived_effects: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for id in &m.legacy.topology_v1_fns {
        let f = &e.functions[id.index()];
        if !f.effects.is_empty() {
            derived_effects.insert(f.display.clone(), f.effects.clone());
        }
    }

    // ---- supervision ----
    struct SupRow {
        locus: String,
        child: String,
        err: String,
        ops: Vec<String>,
        retry: Option<i64>,
        // Authored position for the tie-break below.
        origin: u32,
    }
    let mut sup_rows: Vec<SupRow> = r
        .supervises
        .iter()
        .map(|s| SupRow {
            locus: locus_display(s.parent),
            child: match &s.child {
                SupervisedRef::Locus(l) => locus_display(*l),
                SupervisedRef::External(n) => v1_name(n),
            },
            err: v1_name(&s.error_type),
            ops: s.policy.ops.clone(),
            retry: s.policy.retry_bound,
            origin: s.authored_ordinal,
        })
        .collect();
    // The legacy encoder collects handlers in SOURCE order and then
    // STABLE-sorts by (locus, child) only — handlers sharing both
    // keep authored order, not error-type order (the model's
    // canonical key). Recover authored order from provenance, then
    // reproduce the legacy stable sort (review round 11).
    sup_rows.sort_by(|a, b| a.origin.cmp(&b.origin));
    sup_rows.sort_by(|a, b| (&a.locus, &a.child).cmp(&(&b.locus, &b.child)));

    // ---- unknowns: re-fold holes + dead dispatches ----
    let mut unknowns: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for h in &m.holes {
        let EntityRef::Function(f) = h.at else { continue };
        if !v1.contains(&f) {
            continue;
        }
        let reason = match &h.kind {
            HoleKind::IndirectCall => "indirect_call".to_string(),
            HoleKind::UntypedReceiver { callee } => {
                format!("untyped_receiver_call:{}", callee)
            }
            HoleKind::ComputedSubject => "computed_publish".to_string(),
            _ => continue,
        };
        unknowns.entry(fn_display(f)).or_default().insert(reason);
    }
    for d in &r.dead_interface_calls {
        if !v1.contains(&d.from) {
            continue;
        }
        unknowns.entry(fn_display(d.from)).or_default().insert(format!(
            "uninhabited_interface_call:{}.{}",
            iface_display(&d.interface),
            d.method
        ));
    }

    // ---- serialize: byte-identical to dump_topology ----
    let mut model = String::new();
    model.push_str("  \"sorts\": {\n");
    model.push_str(&format!("    \"loci\": [{}],\n", join_str(loci.iter())));
    model.push_str(&format!("    \"fns\": [{}],\n", join_str(fns.iter())));
    model.push_str(&format!(
        "    \"topics\": [{}]\n",
        join_str(topics.iter())
    ));
    model.push_str("  },\n");
    model.push_str(&format!(
        "  \"sealed\": [{}],\n",
        join_str(sealed.iter())
    ));
    model.push_str("  \"relations\": {\n    \"calls\": [\n");
    for ((from, to), meta) in &calls {
        let mut row = format!(
            "      {{\"from\": {}, \"to\": {}",
            quote(from),
            quote(to)
        );
        if meta.looped {
            row.push_str(", \"loop\": true");
        }
        if meta.unbounded {
            row.push_str(", \"unbounded\": true");
        }
        if let Some((_, i)) = &meta.via_interface {
            row.push_str(&format!(", \"via_interface\": {}", quote(i)));
        }
        row.push_str("},\n");
        model.push_str(&row);
    }
    trim_trailing_comma(&mut model);
    model.push_str("    ],\n    \"calls_via_stdlib\": [\n");
    for ((from, to), looped) in &via_stdlib {
        let mut row = format!(
            "      {{\"from\": {}, \"to\": {}",
            quote(from),
            quote(to)
        );
        if *looped {
            row.push_str(", \"loop\": true");
        }
        row.push_str("},\n");
        model.push_str(&row);
    }
    trim_trailing_comma(&mut model);
    model.push_str("    ],\n    \"publishes\": [\n");
    for (f, s) in &publishes {
        model.push_str(&format!(
            "      {{\"fn\": {}, \"subject\": {}}},\n",
            quote(f),
            quote(s)
        ));
    }
    trim_trailing_comma(&mut model);
    model.push_str("    ],\n    \"subscribes\": [\n");
    for (subj, locus, handler) in &subscribes {
        model.push_str(&format!(
            "      {{\"subject\": {}, \"locus\": {}, \"handler\": {}}},\n",
            quote(subj),
            quote(locus),
            quote(handler)
        ));
    }
    trim_trailing_comma(&mut model);
    model.push_str("    ]\n  },\n");
    model.push_str("  \"groups\": {\n");
    for (g, members) in &group_rows {
        let names: Vec<String> =
            members.iter().map(|(_, d)| d.clone()).collect();
        model.push_str(&format!(
            "    {}: [{}],\n",
            quote(g),
            join_str(names.iter())
        ));
    }
    trim_trailing_comma(&mut model);
    model.push_str("  },\n  \"labels\": {\n");
    for (f, classes) in &labels {
        model.push_str(&format!(
            "    {}: [{}],\n",
            quote(f),
            join_str(classes.iter())
        ));
    }
    trim_trailing_comma(&mut model);
    model.push_str("  },\n  \"phases\": {\n");
    for (f, (phase, hook)) in &phase_rows {
        model.push_str(&format!(
            "    {}: {{\"phase\": {}, \"kind\": {}}},\n",
            quote(f),
            quote(phase),
            quote(if *hook { "hook" } else { "method" })
        ));
    }
    trim_trailing_comma(&mut model);
    model.push_str("  },\n  \"seeds\": {\n");
    for (alias, members) in &seed_rows {
        model.push_str(&format!(
            "    {}: [{}],\n",
            quote(alias),
            join_str(members.iter())
        ));
    }
    trim_trailing_comma(&mut model);
    model.push_str("  },\n  \"effects\": {\n");
    for (f, classes) in &derived_effects {
        model.push_str(&format!(
            "    {}: [{}],\n",
            quote(f),
            join_str(classes.iter())
        ));
    }
    trim_trailing_comma(&mut model);
    model.push_str("  },\n  \"supervision\": [\n");
    for r in &sup_rows {
        let retry = r
            .retry
            .map(|n| format!(", \"retry_bound\": {}", n))
            .unwrap_or_default();
        model.push_str(&format!(
            "    {{\"locus\": {}, \"child\": {}, \"err\": {}, \"ops\": [{}]{}}},\n",
            quote(&r.locus),
            quote(&r.child),
            quote(&r.err),
            join_str(r.ops.iter()),
            retry
        ));
    }
    trim_trailing_comma(&mut model);
    model.push_str("  ],\n  \"unknowns\": [\n");
    for (f, reasons) in &unknowns {
        let rs = reasons
            .iter()
            .map(|r| quote(r))
            .collect::<Vec<_>>()
            .join(", ");
        model.push_str(&format!(
            "    {{\"fn\": {}, \"reasons\": [{}]}},\n",
            quote(f),
            rs
        ));
    }
    trim_trailing_comma(&mut model);
    model.push_str("  ]");
    model
}

// ==================================================================
// GH #476 Change 6 — the artifact's LAW rows, projected from the
// model instead of read off the evaluators.
// ==================================================================

/// One projected `claims` row — exactly the fields the artifact
/// serializes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectedClaimRow {
    pub name: String,
    pub form: String,
    pub result: crate::verdict::Verdict,
    pub source: Option<String>,
    /// The law ordinal this row projects from (round 3): the
    /// claims-to-law join is BY ORDINAL, never by name — a
    /// duplicate-name row cannot masquerade.
    pub ordinal: u32,
}

/// One projected `lowered` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectedLoweredRow {
    pub subject: String,
    pub form: String,
    pub result: crate::verdict::Verdict,
}

/// One TYPED law row (GH #476 Change 6) — the artifact's `law`
/// section: every lowered ClaimIr row with its judgment family and
/// machine verdict, addressable by ordinal. The legacy `claims` /
/// `lowered` string rows remain for their existing consumers; this
/// is the row a typed consumer (fleet composition, Track A's claim
/// view, iris) reads instead of parsing form strings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectedLawRow {
    pub ordinal: u32,
    pub name: String,
    /// "main" | "constitution:<name>" | "library:<alias>" |
    /// "annotation" | "fleet".
    pub origin: String,
    pub family: hale_model::JudgmentFamily,
    pub verdict: crate::verdict::Verdict,
    /// The TYPED law payload — one tagged JSON object per ClaimIr
    /// variant with the law's operands
    /// ([`hale_model::ClaimRow::law_payload_json`]), serialized
    /// verbatim into the artifact.
    pub law: String,
    /// Per-certificate evidence for certificate-family rows:
    /// (certificate ordinal within the row, form, engine result,
    /// the certificate's own ordered diagnostics).
    pub certs: Vec<(
        u32,
        String,
        crate::verdict::Verdict,
        Vec<(String, Option<(String, u32, u32)>)>,
    )>,
    /// The judgment's ordered diagnostics for this row — the
    /// EVIDENCE behind the verdict (round 4: a violated
    /// reachability law carries its countermodel path, an
    /// uncertified law its refusal reason), resolved to
    /// (message, source location).
    pub evidence: Vec<(String, Option<(String, u32, u32)>)>,
    /// (source path, start, end) when the row's provenance is a
    /// source record.
    pub provenance: Option<(String, u32, u32)>,
}

/// The authoritative verdicts of the UNMIGRATED families (GH #476
/// Change 6): from the OLD engines, bridged only where the old
/// engine DEMONSTRABLY enumerated the row (round 2 — the old
/// walks are NONRECURSIVE: a module-scoped annotation is lowered
/// but never evaluated, and treating its missing diagnostic as a
/// pass would manufacture `holds`). Missing report evidence stays
/// `uncertified`.
///
/// - `@budget` rows align POSITIONALLY against the old
///   certificate rows, but only over the old-visible subsequence
///   (top-level fns and locus members, in the old engines'
///   declaration order); module-scoped rows are skipped and stay
///   uncertified.
/// - `causes:` / `depends:` attribute the old passes' diagnostics
///   by the exact anchor span — and only for old-visible subjects
///   carrying exactly ONE assert of the kind: the old engine
///   anchors every assert of a fn at the same span, so with two
///   asserts a diagnostic cannot be attributed and that subject's
///   rows stay uncertified (a lone no-diagnostic subject is a
///   real evaluation: holds).
pub fn legacy_unmigrated_verdicts(
    bundle: &crate::symbol::Bundle<'_>,
    graph: &crate::bus_graph::BusGraph,
    table: &hale_model::ClaimIrTable,
) -> std::collections::BTreeMap<u32, crate::verdict::Verdict> {
    use hale_model::ClaimIr;
    use hale_syntax::ast::{EffectAssert, LocusMember, TopDecl};
    let programs: Vec<&hale_syntax::ast::Program> =
        bundle.programs.values().copied().collect();
    let mut out: std::collections::BTreeMap<
        u32,
        crate::verdict::Verdict,
    > = std::collections::BTreeMap::new();

    // ---- the old engines' NONRECURSIVE enumeration ----
    // (top-level fns, then locus members, per program in order —
    // exactly the walk frontier/budget_check/quantitative run; no
    // TopDecl::Module recursion.)
    let mut vis_alloc: Vec<String> = Vec::new();
    let mut vis_quant: Vec<String> = Vec::new();
    let mut vis_causes: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    let mut vis_depends: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
    {
        let mut note_fn =
            |raw: String, fd: &hale_syntax::ast::FnDecl| {
                if fd.budget.is_some() {
                    vis_alloc.push(raw.clone());
                }
                for _ in &fd.quantities {
                    vis_quant.push(raw.clone());
                }
                let n = fd
                    .effects
                    .iter()
                    .filter(|a| {
                        matches!(a, EffectAssert::Causes(_))
                    })
                    .count();
                if n > 0 {
                    *vis_causes.entry(raw).or_insert(0) += n;
                }
            };
        for p in &programs {
            for item in &p.items {
                match item {
                    TopDecl::Fn(fd) => {
                        note_fn(fd.name.name.clone(), fd)
                    }
                    TopDecl::Locus(l) => {
                        if l.depends.is_some() {
                            vis_depends
                                .insert(l.name.name.clone());
                        }
                        for m in &l.members {
                            if let LocusMember::Fn(fd) = m {
                                note_fn(
                                    format!(
                                        "{}::{}",
                                        l.name.name,
                                        fd.name.name
                                    ),
                                    fd,
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // ---- budgets: positional alignment over the old-visible
    // subsequence ----
    let old_alloc = crate::budget_check::certificate_rows(
        &programs,
        &bundle.import_renames,
    );
    let visible_alloc: Vec<u32> = table
        .rows
        .iter()
        .filter(|r| {
            matches!(r.law, ClaimIr::AllocBudget { .. })
                && vis_alloc.iter().any(|v| *v == r.name)
        })
        .map(|r| r.ordinal)
        .collect();
    if visible_alloc.len() == old_alloc.len() {
        for (ord, r) in
            visible_alloc.iter().zip(old_alloc.iter())
        {
            out.insert(*ord, r.result);
        }
    }
    let fanout = |subj: &str| -> u64 {
        graph
            .subjects
            .get(subj)
            .map(|si| si.subscribers.len().max(1) as u64)
            .unwrap_or(1)
    };
    let old_quant = crate::quantitative::certificate_rows(
        &programs, &fanout,
    );
    let visible_quant: Vec<u32> = table
        .rows
        .iter()
        .filter(|r| {
            matches!(r.law, ClaimIr::QuantBudget { .. })
                && vis_quant.iter().any(|v| *v == r.name)
        })
        .map(|r| r.ordinal)
        .collect();
    if visible_quant.len() == old_quant.len() {
        for (ord, r) in
            visible_quant.iter().zip(old_quant.iter())
        {
            out.insert(*ord, r.result);
        }
    }

    // ---- causes / depends: span-attributed, single-assert
    // subjects only ----
    let span_of_row =
        |row: &hale_model::ClaimRow| -> Option<(usize, usize)> {
            match table
                .provenance
                .records
                .get(row.provenance.index())
            {
                Some(hale_model::Provenance::Source {
                    source,
                    span,
                }) => {
                    let base = bundle
                        .sources
                        .iter()
                        .find(|f| f.id == source.0)
                        .map(|f| f.base)
                        .unwrap_or(0);
                    Some((
                        (base + span.0) as usize,
                        (base + span.1) as usize,
                    ))
                }
                _ => None,
            }
        };
    let causes_diags =
        crate::frontier::causes_diags(&programs, graph);
    let depends_diags =
        crate::frontier::depends_diags(&programs, graph);
    for row in &table.rows {
        let (diags, enumerated, unambiguous) = match &row.law {
            ClaimIr::EffectCauses { .. } => (
                &causes_diags,
                vis_causes.contains_key(&row.name),
                vis_causes.get(&row.name) == Some(&1),
            ),
            ClaimIr::DependsSet { .. } => (
                &depends_diags,
                vis_depends.contains(&row.name),
                true,
            ),
            _ => continue,
        };
        if !enumerated {
            continue; // never evaluated — stays uncertified
        }
        let Some((a, b)) = span_of_row(row) else {
            continue;
        };
        let hit = diags.iter().any(|d| {
            d.span.start.as_usize() == a
                && d.span.end.as_usize() == b
        });
        if hit && !unambiguous {
            // Two asserts share one anchor — the diagnostic
            // cannot be attributed to a row.
            continue;
        }
        out.insert(
            row.ordinal,
            if hit {
                crate::verdict::Verdict::Violated
            } else {
                crate::verdict::Verdict::Holds
            },
        );
    }
    out
}

/// Project the artifact's claim/evidence rows from the canonical
/// path: `ClaimIr` rows rendered by the model-side authority
/// ([`hale_model::ClaimRow::claims_form`]), verdicts from the
/// Change-5 judgments, certificate rows from the evidence sidecar.
///
/// Scope (the migrated families): claims-block rows and the
/// effects-family certificates. `@budget` rows keep their old
/// producers until the quantitative engines migrate, and `causes:`
/// lowers but has never had an artifact row — both are
/// `JudgmentFamily::Unmigrated`.
pub fn project_law_rows(
    bundle: &crate::symbol::Bundle<'_>,
    model: &ApplicationModel,
    table: &hale_model::ClaimIrTable,
    evidence: &hale_model::EvidenceTable,
    source_bases: &[u32],
    legacy_unmigrated: &std::collections::BTreeMap<
        u32,
        crate::verdict::Verdict,
    >,
) -> (
    Vec<ProjectedClaimRow>,
    Vec<ProjectedLoweredRow>,
    Vec<ProjectedLawRow>,
) {
    use hale_model::{ClaimIr, ClaimOrigin};
    let _ = bundle;
    // Merge every judged family by ordinal.
    let mut judged: std::collections::BTreeMap<
        u32,
        crate::judgment::Judged,
    > = std::collections::BTreeMap::new();
    let (_pre, r5a) = crate::judgment::judge_forbid_reaches(
        table,
        model,
        source_bases,
    );
    for j in r5a {
        judged.insert(j.ordinal, j);
    }
    for j in crate::judgment::judge_only_edges(
        table,
        model,
        source_bases,
    ) {
        judged.insert(j.ordinal, j);
    }
    for j in crate::judgment::judge_endpoints(
        table,
        model,
        source_bases,
    ) {
        judged.insert(j.ordinal, j);
    }
    for j in
        crate::judgment::judge_bound(table, model, source_bases)
    {
        judged.insert(j.ordinal, j);
    }
    for j in crate::judgment::judge_certificates(
        table,
        model,
        evidence,
        source_bases,
    ) {
        judged.insert(j.ordinal, j);
    }
    // Bundle-global diag spans → (source path, local span), the
    // same placement rule the artifact's provenance section uses.
    let locate = |sp: hale_syntax::Span|
     -> Option<(String, u32, u32)> {
        let s0 = sp.start.as_usize() as u32;
        let e0 = sp.end.as_usize() as u32;
        bundle
            .sources
            .iter()
            .filter(|f| {
                s0 >= f.base
                    && s0 < f.base.saturating_add(f.len + 1)
            })
            .max_by_key(|f| f.base)
            .map(|f| {
                (
                    f.path.clone(),
                    s0 - f.base,
                    e0.saturating_sub(f.base),
                )
            })
    };
    let verdicts: std::collections::BTreeMap<
        u32,
        crate::verdict::Verdict,
    > = judged
        .iter()
        .map(|(o, j)| (*o, j.verdict))
        .collect();

    // ---- claims rows: claims-block origins, authored order ----
    let mut claims: Vec<ProjectedClaimRow> = Vec::new();
    for row in &table.rows {
        let source = match &row.origin {
            ClaimOrigin::Main | ClaimOrigin::Library { .. } => None,
            ClaimOrigin::Constitution { name } => {
                Some(name.clone())
            }
            _ => continue,
        };
        let Some(form) = row.claims_form() else { continue };
        let Some(v) = verdicts.get(&row.ordinal) else {
            continue;
        };
        claims.push(ProjectedClaimRow {
            name: row.name.clone(),
            form,
            result: *v,
            source,
            ordinal: row.ordinal,
        });
    }

    // ---- lowered rows: the effects-family certificates, from the
    // evidence sidecar. Row order preserves the legacy report's
    // stream: phase certificates first (they are generated by the
    // placement pass, before the annotated-roots loop), then the
    // assert certificates — both in authored order within their
    // stratum. ----
    let by_ordinal: std::collections::BTreeMap<
        u32,
        &hale_model::ClaimRow,
    > = table.rows.iter().map(|r| (r.ordinal, r)).collect();
    let mut ev_rows: Vec<&hale_model::EvidenceRow> =
        evidence.rows.iter().collect();
    ev_rows.sort_by_key(|r| {
        let is_phase = by_ordinal
            .get(&r.ordinal)
            .is_some_and(|row| {
                matches!(row.law, ClaimIr::PhaseEffects { .. })
            });
        (if is_phase { 0u8 } else { 1 }, r.ordinal)
    });
    let mut lowered: Vec<ProjectedLoweredRow> = Vec::new();
    for r in ev_rows {
        let Some(row) = by_ordinal.get(&r.ordinal) else {
            continue;
        };
        let forms = row.certificate_forms();
        for (cert, (subject, form)) in
            r.certs.iter().zip(forms.into_iter())
        {
            // The stored form is validated equal to the expected
            // one (EvidenceTable::validate); the subject spelling
            // comes from the same authority.
            debug_assert_eq!(cert.form, form);
            lowered.push(ProjectedLoweredRow {
                subject,
                form: cert.form.clone(),
                result: match cert.result {
                    hale_model::VerdictIr::Holds => {
                        crate::verdict::Verdict::Holds
                    }
                    hale_model::VerdictIr::Violated => {
                        crate::verdict::Verdict::Violated
                    }
                    hale_model::VerdictIr::Uncertified => {
                        crate::verdict::Verdict::Uncertified
                    }
                    hale_model::VerdictIr::Invalid => {
                        crate::verdict::Verdict::Invalid
                    }
                },
            });
        }
    }
    // ---- typed law rows: EVERY lowered row, ordinal order ----
    let mut law: Vec<ProjectedLawRow> = Vec::new();
    for row in &table.rows {
        let origin = match &row.origin {
            ClaimOrigin::Main => "main".to_string(),
            ClaimOrigin::Constitution { name } => {
                format!("constitution:{}", name)
            }
            ClaimOrigin::Library { alias } => match alias {
                Some(a) => format!("library:{}", a),
                None => "library".to_string(),
            },
            ClaimOrigin::Annotation => "annotation".to_string(),
            ClaimOrigin::FleetPlan => "fleet".to_string(),
        };
        let provenance = match table
            .provenance
            .records
            .get(row.provenance.index())
        {
            Some(hale_model::Provenance::Source {
                source,
                span,
            }) => table
                .provenance
                .sources
                .get(source.index())
                .map(|su| (su.path.clone(), span.0, span.1)),
            _ => None,
        };
        // Unjudged fleet rows are lowered elsewhere (Change 7);
        // every application-tier row has a verdict by the 5e
        // completeness law.
        let Some(v) = verdicts.get(&row.ordinal) else {
            continue;
        };
        // An UNMIGRATED family's judgment verdict is `uncertified`
        // ("no migrated engine") — the artifact substitutes the
        // OLD engine's authoritative result where one exists
        // (round 1: an uncertified law row under a clean document
        // verdict with no other evidence is an unwitnessed pass).
        // A stricter judgment verdict (`invalid` for an undeclared
        // budget class) is never weakened.
        let family = row.family();
        let verdict = if family
            == hale_model::JudgmentFamily::Unmigrated
            && *v == crate::verdict::Verdict::Uncertified
        {
            legacy_unmigrated
                .get(&row.ordinal)
                .copied()
                .unwrap_or(*v)
        } else {
            *v
        };
        let ev_loc = |pid: hale_model::ProvenanceId|
         -> Option<(String, u32, u32)> {
            match evidence.provenance.records.get(pid.index()) {
                Some(hale_model::Provenance::Source {
                    source,
                    span,
                }) => evidence
                    .provenance
                    .sources
                    .get(source.index())
                    .map(|su| (su.path.clone(), span.0, span.1)),
                _ => None,
            }
        };
        let certs: Vec<(
            u32,
            String,
            crate::verdict::Verdict,
            Vec<(String, Option<(String, u32, u32)>)>,
        )> = evidence
            .rows
            .iter()
            .find(|r| r.ordinal == row.ordinal)
            .map(|r| {
                r.certs
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        (
                            i as u32,
                            c.form.clone(),
                            match c.result {
                                hale_model::VerdictIr::Holds => {
                                    crate::verdict::Verdict::Holds
                                }
                                hale_model::VerdictIr::Violated => {
                                    crate::verdict::Verdict::Violated
                                }
                                hale_model::VerdictIr::Uncertified => {
                                    crate::verdict::Verdict::Uncertified
                                }
                                hale_model::VerdictIr::Invalid => {
                                    crate::verdict::Verdict::Invalid
                                }
                            },
                            c.diags
                                .iter()
                                .map(|(msg, pid)| {
                                    (msg.clone(), ev_loc(*pid))
                                })
                                .collect(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let row_evidence: Vec<(
            String,
            Option<(String, u32, u32)>,
        )> = judged
            .get(&row.ordinal)
            .map(|j| {
                j.diags
                    .iter()
                    .map(|d| {
                        (d.message.clone(), locate(d.span))
                    })
                    .collect()
            })
            .unwrap_or_default();
        law.push(ProjectedLawRow {
            ordinal: row.ordinal,
            name: row.name.clone(),
            origin,
            family,
            verdict,
            law: row.law_payload_json(
                &model.entities,
                &table.provenance,
            ),
            certs,
            evidence: row_evidence,
            provenance,
        });
    }
    (claims, lowered, law)
}

/// Per-family adequacy (GH #476 Change 6): can this model support
/// family F's judgment EXACTLY? `exact` iff the model's
/// capabilities vouch for every relation family F's projection
/// consumes; `degraded` otherwise — the judgment still runs, but
/// holes force Uncertified wherever they are reachable. The typed
/// answer a consumer reads instead of reverse-engineering the
/// `unknowns` strings.
pub fn family_adequacy(
    m: &ApplicationModel,
) -> Vec<(hale_model::JudgmentFamily, bool)> {
    use hale_model::JudgmentFamily as F;
    // The POSITIVE account decides (round 3): a family is `exact`
    // only when the capabilities VOUCH for every relation family
    // its projection consumes — `exact_publishes` and
    // `exact_subscribes` are independent flags, so publish and
    // subscribe completeness count separately. Holes are the
    // validation cross-check (`CapabilityContradiction`), never a
    // source of positive knowledge: an unvouched family with no
    // hole reads `degraded`, because absence of recorded unknowns
    // is not proof of completeness.
    let mut vouched = hale_model::RelationSet(0);
    for (_, claimed, fam) in m.capabilities.vouched_families() {
        if claimed {
            vouched = vouched.union(fam);
        }
    }
    [
        F::Reachability,
        F::Boundary,
        F::Endpoint,
        F::Bound,
        F::Certificate,
    ]
    .into_iter()
    .map(|f| (f, vouched.contains(f.required_relations())))
    .collect()
}

/// The artifact's UNHASHED tail — `sources`, `provenance`, and
/// `topics` — projected from the model (GH #476 Change 6, review
/// round 2: the legacy gathering must not supply ANY production
/// section; every modeled fact renders from `ApplicationModel`).
/// Byte-compatible with the legacy serialization; the corpus
/// differential compares the two until Change 9 retires the
/// legacy arm.
pub fn project_unhashed_tail(m: &ApplicationModel) -> String {
    let e = &m.entities;
    let r = &m.relations;
    let fn_display =
        |id: hale_model::FunctionId| e.functions[id.index()].display.clone();
    let locus_display =
        |id: hale_model::LocusDeclId| e.loci[id.index()].display.clone();
    // A provenance record as (source id, local start, local end);
    // synthetic/foreign records render source -1, matching the
    // legacy "unplaceable" rule.
    let loc = |pid: hale_model::ProvenanceId| -> (i64, u32, u32) {
        match m.provenance.records.get(pid.index()) {
            Some(hale_model::Provenance::Source { source, span }) => {
                (source.index() as i64, span.0, span.1)
            }
            Some(hale_model::Provenance::ForeignSpan { span }) => {
                (-1, span.0, span.1)
            }
            _ => (-1, 0, 0),
        }
    };
    let mut out = String::new();

    // ---- sources ----
    out.push_str(",\n  \"sources\": [\n");
    let n = m.provenance.sources.len();
    for (i, su) in m.provenance.sources.iter().enumerate() {
        out.push_str(&format!(
            "    {{\"id\": {}, \"path\": {}, \"digest\": {}}}{}\n",
            i,
            quote(&su.path),
            quote(&su.digest),
            if i + 1 == n { "" } else { "," }
        ));
    }
    out.push_str("  ]");

    // ---- provenance ----
    // Legacy sorts each subsection by its BTreeSet key with
    // BUNDLE-GLOBAL spans; (source, local) sorts identically
    // because source ids ascend with file bases.
    out.push_str(",\n  \"provenance\": {\n    \"calls\": [\n");
    {
        // Same universe filter as the hashed `calls` section: both
        // endpoints in the summary (v1) universe — a
        // declaration-universe-recovered edge (known edge + hole
        // coexisting) has no legacy span row.
        let v1: BTreeSet<u32> = m
            .legacy
            .topology_v1_fns
            .iter()
            .map(|f| f.0)
            .collect();
        let mut rows: BTreeSet<(String, String, i64, u32, u32)> =
            BTreeSet::new();
        for c in &r.calls {
            if matches!(
                c.dispatch,
                hale_model::DispatchKind::ViaStdlib
            ) {
                continue;
            }
            if !v1.contains(&c.from.0) || !v1.contains(&c.to.0) {
                continue;
            }
            let (src, a, b) = loc(c.provenance);
            rows.insert((
                fn_display(c.from),
                fn_display(c.to),
                src,
                a,
                b,
            ));
        }
        for (from, to, src, a, b) in &rows {
            out.push_str(&format!(
                "      {{\"from\": {}, \"to\": {}, \"source\": {}, \"span\": [{}, {}]}},\n",
                quote(from),
                quote(to),
                src,
                a,
                b
            ));
        }
    }
    trim_trailing_comma(&mut out);
    out.push_str("    ],\n    \"publishes\": [\n");
    {
        let mut rows: BTreeSet<(String, String, i64, u32, u32)> =
            BTreeSet::new();
        let v1_display = v1_display_map(e);
        for p in &r.publishes {
            let written = v1_endpoint_subject(
                e,
                &v1_display,
                p.declared_topic,
                p.subject,
            );
            let (src, a, b) = loc(p.provenance);
            rows.insert((
                fn_display(p.function),
                written,
                src,
                a,
                b,
            ));
        }
        for (f, subj, src, a, b) in &rows {
            out.push_str(&format!(
                "      {{\"fn\": {}, \"subject\": {}, \"source\": {}, \"span\": [{}, {}]}},\n",
                quote(f),
                quote(subj),
                src,
                a,
                b
            ));
        }
    }
    trim_trailing_comma(&mut out);
    out.push_str("    ],\n    \"subscribes\": [\n");
    {
        let mut locus_of: BTreeMap<u32, String> = BTreeMap::new();
        for mo in &r.member_of {
            locus_of.insert(
                mo.function.0,
                locus_display(mo.locus),
            );
        }
        let mut rows: BTreeSet<(String, String, String, i64, u32, u32)> =
            BTreeSet::new();
        let v1_display = v1_display_map(e);
        for su in &r.subscribes {
            let written = v1_endpoint_subject(
                e,
                &v1_display,
                su.declared_topic,
                su.subject,
            );
            let handler_full =
                e.functions[su.handler.index()].display.clone();
            let handler_short = handler_full
                .rsplit("::")
                .next()
                .unwrap_or(&handler_full)
                .to_string();
            let locus = locus_of
                .get(&su.handler.0)
                .cloned()
                .unwrap_or_default();
            let (src, a, b) = loc(su.provenance);
            rows.insert((
                written,
                locus,
                handler_short,
                src,
                a,
                b,
            ));
        }
        for (subj, locus, handler, src, a, b) in &rows {
            out.push_str(&format!(
                "      {{\"subject\": {}, \"locus\": {}, \"handler\": {}, \
                 \"source\": {}, \"span\": [{}, {}]}},\n",
                quote(subj),
                quote(locus),
                quote(handler),
                src,
                a,
                b
            ));
        }
    }
    trim_trailing_comma(&mut out);
    out.push_str("    ],\n    \"decls\": {\n");
    {
        let mut rows: BTreeMap<String, (i64, u32, u32)> =
            BTreeMap::new();
        // The decl universe is TOP declarations — free fns yes,
        // methods/hooks no (they are members, not decls).
        for f in &e.functions {
            if !matches!(f.kind, hale_model::FunctionKind::Free) {
                continue;
            }
            let (src, a, b) = loc(f.provenance);
            rows.entry(f.display.clone())
                .or_insert((src, a, b));
        }
        for g in &e.groups {
            let (src, a, b) = loc(g.provenance);
            rows.entry(g.display.clone())
                .or_insert((src, a, b));
        }
        for l in &e.loci {
            let (src, a, b) = loc(l.provenance);
            rows.entry(l.display.clone())
                .or_insert((src, a, b));
        }
        for t in &e.types {
            let (src, a, b) = loc(t.provenance);
            rows.entry(t.display.clone())
                .or_insert((src, a, b));
        }
        for i in &e.interfaces {
            let (src, a, b) = loc(i.provenance);
            rows.entry(i.display.clone())
                .or_insert((src, a, b));
        }
        for t in &e.topics {
            let (src, a, b) = loc(t.provenance);
            rows.entry(t.display.clone())
                .or_insert((src, a, b));
        }
        for (decl, (src, a, b)) in &rows {
            out.push_str(&format!(
                "      {}: {{\"source\": {}, \"span\": [{}, {}]}},\n",
                quote(decl),
                src,
                a,
                b
            ));
        }
    }
    trim_trailing_comma(&mut out);
    out.push_str("    },\n    \"supervision\": [\n");
    {
        struct Row {
            locus: String,
            child: String,
            src: i64,
            a: u32,
            b: u32,
            origin: u32,
        }
        let mut rows: Vec<Row> = r
            .supervises
            .iter()
            .map(|s| {
                let (src, a, b) = loc(s.provenance);
                Row {
                    locus: locus_display(s.parent),
                    child: match &s.child {
                        hale_model::SupervisedRef::Locus(l) => {
                            locus_display(*l)
                        }
                        hale_model::SupervisedRef::External(
                            n,
                        ) => n.clone(),
                    },
                    src,
                    a,
                    b,
                    origin: s.authored_ordinal,
                }
            })
            .collect();
        rows.sort_by(|x, y| {
            (&x.locus, &x.child, x.origin)
                .cmp(&(&y.locus, &y.child, y.origin))
        });
        for row in &rows {
            out.push_str(&format!(
                "      {{\"locus\": {}, \"child\": {}, \"source\": {}, \"span\": [{}, {}]}},\n",
                quote(&row.locus),
                quote(&row.child),
                row.src,
                row.a,
                row.b
            ));
        }
    }
    trim_trailing_comma(&mut out);
    out.push_str("    ]\n  }");

    // ---- topics: the per-topic observation identity ----
    out.push_str(",\n  \"topics\": [\n");
    {
        let mut rows: BTreeSet<(String, String, String, u64)> =
            BTreeSet::new();
        for t in &e.topics {
            let subj =
                e.subjects[t.subject.index()].pattern.clone();
            let payload = &e.payloads[t.payload.index()];
            // The OBSERVATION identity fuses subject and shape
            // (`topic_shape_hash`) — the model's payload hash is
            // shape-only, and the runtime manifest joins on the
            // fused value. A payload the per-file gathering could
            // not structurally resolve (the model's `opaque:`
            // contracts — cross-seed types) renders as the empty
            // shape, the spelling this section always carried.
            // Opaque by the STRUCTURAL flag (round 3): `opaque`
            // is not a reserved word — a struct whose first field
            // is literally named `opaque` has structural shape
            // `opaque:i`, and string inspection would erase it.
            let shape = if payload.opaque {
                String::new()
            } else {
                payload.shape.clone()
            };
            let h = crate::topic_identity::topic_shape_hash(
                &subj, &shape,
            );
            rows.insert((
                t.display.clone(),
                subj,
                shape,
                h,
            ));
        }
        for (tname, subj, shape, h) in &rows {
            out.push_str(&format!(
                "    {{\"name\": {}, \"subject\": {}, \"shape\": {}, \
                 \"payload_hash\": \"{:016x}\"}},\n",
                quote(tname),
                quote(subj),
                quote(shape),
                h
            ));
        }
    }
    trim_trailing_comma(&mut out);
    out.push_str("  ]");
    out
}
