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
    // The V1 display map: raw post-merge symbol → author spelling,
    // over EVERY declaration table. The legacy encoder ran name()
    // (full-string demangle) unconditionally over subjects,
    // supervision child/error types, and more — including strings
    // that merely COLLIDE with an imported declaration's raw symbol
    // (a literal subject spelled like a mangled name demangles
    // under V1). The projection must apply the same rule everywhere
    // name() ran (review round 11).
    let mut v1_display: BTreeMap<&str, &str> = BTreeMap::new();
    {
        let mut add = |name: &'a str, display: &'a str| {
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
        // declarations — a method identity (`__lib_x_Store::bump`)
        // is never a renames key, so a literal subject spelled like
        // one must NOT demangle (review round 12). Every other
        // table above holds top-level declarations whose raw names
        // are exactly the renames keys.
        for f in &e.functions {
            if f.kind == FunctionKind::Free {
                add(&f.name, &f.display);
            }
        }
    }
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
    let sealed: BTreeSet<String> = e
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
        match declared {
            Some(t) => topic_display(t),
            None => v1_name(&e.subjects[subject.index()].pattern),
        }
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
        retry: Option<u32>,
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
