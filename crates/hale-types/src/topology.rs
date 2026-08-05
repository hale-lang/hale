//! GH #382 phase 2 — the topology artifact: the serialized model.
//!
//! The claims DSL's real interface is the SCHEMA of the derived
//! model — sorts, relations, labels — not its syntax. The artifact
//! is that model serialized, plus every named claim's result. The
//! degree of independent re-evaluation this buys is scoped
//! honestly below (see "v1 SCOPE") — the trust root is the
//! derivation (source → model), which is where it should be; that
//! half is defended by the classified frontier and the conformance
//! loops.
//!
//! Identity: `shape_hash` is FNV-1a/64 over the canonical
//! serialization of the MODEL half (sorts + relations + groups +
//! labels + unknowns, stable BTree order; claim RESULTS excluded —
//! two builds with one topology and different law share a shape). v1 note: this is the compiler-side
//! shape identity; reconciling it with the observer protocol's
//! runtime per-topic `shape_hash` (lotus_obs.c, PROTOCOL.md) is
//! tracked on #382 — the two live in different namespaces until
//! then ("topology.shape_hash" vs per-subject payload hashes).
//!
//! Names render in AUTHOR spelling (cross-seed symbols demangled)
//! — an artifact naming `__lib_lib_delta_d_Triage` points at
//! something that appears nowhere in anyone's source. ONE
//! exception (#399): the `topics` section's `subject` field is the
//! byte-exact runtime-manifest join key and stays RAW — a
//! subject-less imported topic really does register under its
//! mangled local name, and the artifact exposing that
//! non-portable identity is deliberate (declare `subject:` on
//! shared topics to fuse across binaries). Each row carries the
//! author-spelled `name` beside it.
//!
//! Consumed by `hale check <t> --dump-topology` and diffed by
//! `--check-topology <path>` — the `.hale.effects` manifest
//! precedent: emit for review, commit, and an unreviewed topology
//! change fails CI the way an API break does.
//!
//! v2 SCOPE (schema 1.1, #392 thread 1 — the normalized model
//! export): the hashed model half carries the sorts, the
//! call/publish/subscribe relations WITH WEIGHTS (loop nesting,
//! unbounded-loop membership, interface-dispatch tags), the
//! through-stdlib CONTRACTED user→user call edges
//! (`calls_via_stdlib` — the paths the evaluator walks through
//! stdlib bodies, collapsed to their user endpoints with a
//! conservative loop flag), the declared groups, the effect labels
//! (declared carriers), the PHASE RELATION (`phases` — lifecycle
//! hooks and modes vs. ordinary methods, what `during` evaluates
//! against), the SEED SORT (`seeds` — alias → member decls, what
//! `cover` evaluates against), the compiler-DERIVED per-fn effect
//! sets (`effects` — the full-walk inference, what effect-class
//! claim endpoints evaluate against), and the UNKNOWNS (fns with
//! indirect calls, untyped-receiver method calls, dead
//! uninhabited-interface dispatch, or computed publish subjects —
//! each recorded so an outside evaluator applies the same rule).
//!
//! What that supports independently replaying: every claim verb
//! over the exported relations — `forbid`/`only edges` incl.
//! through-stdlib reachability, `require`/`count` bus-end
//! cardinality, `cover` via the seed sort, `during` via the phase
//! relation, and `bound` over USER classes via labels + weights
//! (dispatch alternatives group by (from fn, interface, method)
//! and fold with max). Remaining compiler-certified: `bound` over
//! BUILT-IN classes (site counting through the stdlib interior,
//! which the artifact deliberately does not serialize) and any
//! walk past the step ceiling.
//!
//! PROVENANCE (unhashed): a `provenance` section carries per-edge
//! and per-decl source spans as bundle-global byte offsets
//! (`[start, end]`). It is excluded from `shape_hash` on purpose —
//! moving code must not change the shape identity — and sits with
//! the claim results in the unhashed half.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::*;

use crate::alloc_summary::{self, Callee, EffectSiteKind, FnKey};
use crate::symbol::Bundle;

/// The artifact's schema version. Additions are minor versions;
/// changes are breaking. 1.1 (#392): weights on call edges,
/// `calls_via_stdlib`, `phases`, `seeds`, `effects` in the hashed
/// half (existing `shape_hash` values change); unhashed
/// `provenance` section. 1.2 (#399): unhashed `topics` section —
/// the per-topic OBSERVATION identity (wire subject, canonical
/// payload shape, `payload_hash`), the join key a recording/WAL
/// segment carries; model `shape_hash` values unchanged.
pub const TOPOLOGY_SCHEMA: &str = "1.2";

/// Serialize the bundle's model + claim results as the topology
/// artifact (JSON).
pub fn dump_topology(bundle: &Bundle<'_>) -> String {
    let programs: Vec<&Program> =
        bundle.programs.values().copied().collect();
    let (top, _resolve_diags) = crate::resolve::build_top_scope(bundle);
    let graph = crate::bus_graph::build_bus_graph(bundle, &top);
    // User code only — an app's artifact describes the app, the
    // same ruling as the effects manifest (a library's own artifact
    // comes from checking that library).
    let summary = alloc_summary::summarize_programs_with_renames(
        &programs,
        &bundle.import_renames,
    );

    // Author-spelling map: mangled -> alias::Name.
    let demangle: BTreeMap<&str, String> = bundle
        .import_renames
        .iter()
        .map(|(segs, mangled)| (mangled.as_str(), segs.join("::")))
        .collect();
    let name = |n: &str| -> String {
        demangle.get(n).cloned().unwrap_or_else(|| n.to_string())
    };
    let fn_name = |k: &FnKey| -> String {
        match &k.locus {
            Some(l) => format!("{}::{}", name(l), k.fn_name),
            None => name(&k.fn_name),
        }
    };

    // ---- sorts ----
    let mut loci: BTreeSet<String> = BTreeSet::new();
    let mut fns: BTreeSet<String> = BTreeSet::new();
    let mut topics: BTreeSet<String> = BTreeSet::new();
    fn walk(
        items: &[TopDecl],
        loci: &mut BTreeSet<String>,
        topics: &mut BTreeSet<String>,
        free_fns: &mut BTreeSet<String>,
    ) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    loci.insert(l.name.name.clone());
                }
                TopDecl::Topic(t) => {
                    topics.insert(t.name.name.clone());
                }
                TopDecl::Fn(f) => {
                    free_fns.insert(f.name.name.clone());
                }
                TopDecl::Module(m) => {
                    walk(&m.items, loci, topics, free_fns)
                }
                _ => {}
            }
        }
    }
    let mut raw_loci = BTreeSet::new();
    let mut raw_topics = BTreeSet::new();
    let mut raw_free_fns = BTreeSet::new();
    for p in &programs {
        walk(&p.items, &mut raw_loci, &mut raw_topics, &mut raw_free_fns);
    }
    for l in &raw_loci {
        loci.insert(name(l));
    }
    for t in &raw_topics {
        topics.insert(name(t));
    }
    // The fn sort is the summary's user keys.
    let user_key = |k: &FnKey| -> bool {
        match &k.locus {
            Some(l) => raw_loci.contains(l),
            None => raw_free_fns.contains(&k.fn_name),
        }
    };
    for k in summary.fns.keys() {
        if user_key(k) {
            fns.insert(fn_name(k));
        }
    }

    // ---- relations, with weights (#392) ----
    // A call row's weights: merged over parallel edges between one
    // (from, to) pair, in the conservative direction (any loop-
    // nested edge marks the row looped).
    #[derive(Default)]
    struct EdgeMeta {
        looped: bool,
        unbounded: bool,
        via_interface: Option<String>,
    }
    let mut calls: BTreeMap<(String, String), EdgeMeta> =
        BTreeMap::new();
    let mut publishes: BTreeSet<(String, String)> = BTreeSet::new();
    // Provenance (unhashed): bundle-global byte-offset spans.
    let mut call_spans: BTreeSet<(String, String, u32, u32)> =
        BTreeSet::new();
    let mut publish_spans: BTreeSet<(String, String, u32, u32)> =
        BTreeSet::new();
    for (k, fs) in &summary.fns {
        if !user_key(k) {
            continue;
        }
        for edge in &fs.calls {
            if let Callee::Resolved(next) = &edge.callee {
                if user_key(next) {
                    let m = calls
                        .entry((fn_name(k), fn_name(next)))
                        .or_default();
                    m.looped |= edge.loop_depth > 0;
                    m.unbounded |= edge.in_unbounded_loop;
                    if let Some(i) = &edge.via_interface {
                        m.via_interface = Some(name(i));
                    }
                    call_spans.insert((
                        fn_name(k),
                        fn_name(next),
                        edge.span.start.as_usize() as u32,
                        edge.span.end.as_usize() as u32,
                    ));
                }
            }
        }
        for site in &fs.effect_sites {
            if let EffectSiteKind::Publish(Some(s)) = &site.kind {
                publishes.insert((fn_name(k), name(s)));
                publish_spans.insert((
                    fn_name(k),
                    name(s),
                    site.span.start.as_usize() as u32,
                    site.span.end.as_usize() as u32,
                ));
            }
        }
    }

    // ---- through-stdlib contraction (#392) ----
    // The evaluator walks the stdlib-merged summary; the artifact
    // deliberately serializes only user rows. Collapse every path
    // that ENTERS non-user bodies and re-emerges at a user fn into
    // one contracted edge, so reachability over the artifact matches
    // reachability as evaluated. `looped` is conservative: true if
    // ANY contraction path crosses a loop-nested or unbounded edge.
    let merged = crate::stdlib_bodies::summarize_with_stdlib_and_renames(
        &programs,
        &bundle.import_renames,
    );
    let mut via_stdlib: BTreeMap<(String, String), bool> =
        BTreeMap::new();
    for (k, fs) in &merged.fns {
        if !user_key(k) {
            continue;
        }
        let mut stack: Vec<(FnKey, bool)> = Vec::new();
        let mut seen: BTreeSet<FnKey> = BTreeSet::new();
        for edge in &fs.calls {
            if let Callee::Resolved(next) = &edge.callee {
                if !user_key(next) && seen.insert(next.clone()) {
                    stack.push((
                        next.clone(),
                        edge.loop_depth > 0 || edge.in_unbounded_loop,
                    ));
                }
            }
        }
        let mut steps = 0u32;
        while let Some((n, lp)) = stack.pop() {
            steps += 1;
            if steps > crate::callgraph::MAX_STEPS {
                break;
            }
            let Some(nfs) = merged.fns.get(&n) else { continue };
            for edge in &nfs.calls {
                let Callee::Resolved(next) = &edge.callee else {
                    continue;
                };
                let l2 = lp
                    || edge.loop_depth > 0
                    || edge.in_unbounded_loop;
                if user_key(next) {
                    let e = via_stdlib
                        .entry((fn_name(k), fn_name(next)))
                        .or_insert(false);
                    *e |= l2;
                } else if seen.insert(next.clone()) {
                    stack.push((next.clone(), l2));
                }
            }
        }
    }
    let mut subscribes: BTreeSet<(String, String, String)> =
        BTreeSet::new();
    let mut subscribe_spans: BTreeSet<(String, String, String, u32, u32)> =
        BTreeSet::new();
    for (subject, info) in &graph.subjects {
        for s in &info.subscribers {
            subscribes.insert((
                name(subject),
                name(&s.locus),
                s.handler.clone(),
            ));
            subscribe_spans.insert((
                name(subject),
                name(&s.locus),
                s.handler.clone(),
                s.span.start.as_usize() as u32,
                s.span.end.as_usize() as u32,
            ));
        }
    }

    // ---- the normalized model (#392): phases, seeds, decl spans ----
    let vmodel =
        crate::model::Model::derive(&programs, &bundle.import_renames);
    // Phase relation, user loci only (the sort the artifact serializes).
    let mut phase_rows: BTreeMap<String, (String, bool)> =
        BTreeMap::new();
    for (k, p) in &vmodel.phases {
        if user_key(k) {
            phase_rows
                .insert(fn_name(k), (p.phase.clone(), p.hook));
        }
    }
    // Seed sort: alias -> author-spelled member decls.
    let mut seed_rows: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (alias, members) in &vmodel.seeds {
        seed_rows.insert(
            alias.clone(),
            members.iter().map(|m| name(m)).collect(),
        );
    }
    // Compiler-derived per-fn effect sets over the stdlib-merged
    // walk — what an effect-class claim endpoint evaluates against.
    // PURE fns are omitted; an unclassifiable walk renders as
    // ["unclassified"], honestly.
    let effect_names = crate::effects::effect_names_of(&programs);
    let ffi = crate::effects::ffi_names(&programs);
    let mut derived_effects: BTreeMap<String, Vec<String>> =
        BTreeMap::new();
    for k in merged.fns.keys() {
        if !user_key(k) {
            continue;
        }
        let e = crate::frontier::infer_effects(&merged, k, &ffi);
        let classes =
            crate::frontier::render_effects_named(e, &effect_names);
        if !classes.is_empty() {
            derived_effects.insert(fn_name(k), classes);
        }
    }
    // Decl spans (unhashed provenance).
    let mut decl_spans: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    for (decl, info) in &vmodel.decls {
        decl_spans.insert(
            name(decl),
            (
                info.span.start.as_usize() as u32,
                info.span.end.as_usize() as u32,
            ),
        );
    }

    // ---- groups (the claim vocabulary, as declared) ----
    let mut group_rows: BTreeMap<String, Vec<String>> = BTreeMap::new();
    fn walk_groups<'a>(items: &'a [TopDecl], out: &mut Vec<&'a GroupDecl>) {
        for item in items {
            match item {
                TopDecl::Group(g) => out.push(g),
                TopDecl::Module(m) => walk_groups(&m.items, out),
                _ => {}
            }
        }
    }
    let mut group_decls = Vec::new();
    for p in &programs {
        walk_groups(&p.items, &mut group_decls);
    }
    for g in group_decls {
        group_rows.insert(
            name(&g.name.name),
            g.members.iter().map(|m| name(&m.display())).collect(),
        );
    }

    // ---- labels: declared effect carriers (`is:` tags) ----
    let mut labels: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (k, set) in &summary.carries {
        if !user_key(k) {
            continue;
        }
        let classes =
            crate::frontier::render_effects_named(*set, &effect_names);
        if !classes.is_empty() {
            labels.insert(fn_name(k), classes);
        }
    }

    // ---- unknowns: where the evaluator fails closed ----
    let mut unknowns: BTreeMap<String, BTreeSet<String>> =
        BTreeMap::new();
    for (k, fs) in &summary.fns {
        if !user_key(k) {
            continue;
        }
        for edge in &fs.calls {
            match &edge.callee {
                Callee::Resolved(_) => {}
                Callee::Unresolved(n) => {
                    if edge.indirect
                        || fs.fn_params.iter().any(|p| p == n)
                    {
                        unknowns
                            .entry(fn_name(k))
                            .or_default()
                            .insert("indirect_call".to_string());
                    } else if let Some(iface) = &edge.via_interface {
                        // #392: a call through an interface no locus
                        // in the build conforms to. Not an unknown
                        // in the fail-closed sense — an uninhabited
                        // interface has no values in a closed world,
                        // so every walker treats the site as DEAD.
                        // Recorded (inside the hashed model half) so
                        // an outside evaluator applies the same rule
                        // and a conformer appearing later changes
                        // `shape_hash`. (A dispatch WITH conformers
                        // is already fanned out to ordinary resolved
                        // call edges above and never lands here.)
                        unknowns.entry(fn_name(k)).or_default().insert(
                            format!(
                                "uninhabited_interface_call:{}.{}",
                                name(iface),
                                n
                            ),
                        );
                    } else if edge.receiver_present
                        && edge.recv_ty.is_none()
                    {
                        // The wrapper-shaped hole: a method call on
                        // a receiver the summarizer cannot type.
                        // Recorded WITH the callee name so an
                        // outside evaluator can apply the same
                        // fail-closed rule — and so introducing one
                        // changes shape_hash.
                        unknowns.entry(fn_name(k)).or_default().insert(
                            format!("untyped_receiver_call:{}", n),
                        );
                    }
                }
            }
        }
        for site in &fs.effect_sites {
            if matches!(site.kind, EffectSiteKind::Publish(None)) {
                unknowns
                    .entry(fn_name(k))
                    .or_default()
                    .insert("computed_publish".to_string());
            }
        }
    }

    // ---- claims ----
    let (_diags, outcomes) = crate::claims::claims_report(
        &programs,
        &graph,
        &bundle.import_renames,
    );
    // Rendered forms carry post-mangle topic refs; rewrite them to
    // author spelling (longest-mangled-first, the demangle_imports
    // rule, so a prefix symbol cannot partially rewrite another).
    let mut demangle_pairs: Vec<(&str, String)> = demangle
        .iter()
        .map(|(m, p)| (*m, p.clone()))
        .collect();
    demangle_pairs.sort_by_key(|(m, _)| std::cmp::Reverse(m.len()));
    let demangle_str = |s: &str| -> String {
        let mut s = s.to_string();
        for (mangled, public) in &demangle_pairs {
            if s.contains(mangled) {
                s = s.replace(mangled, public);
            }
        }
        s
    };

    // ---- serialize (canonical: BTree order throughout) ----
    let mut model = String::new();
    model.push_str("  \"sorts\": {\n");
    model.push_str(&format!(
        "    \"loci\": [{}],\n",
        join_str(loci.iter())
    ));
    model.push_str(&format!(
        "    \"fns\": [{}],\n",
        join_str(fns.iter())
    ));
    model.push_str(&format!(
        "    \"topics\": [{}]\n",
        join_str(topics.iter())
    ));
    model.push_str("  },\n");
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
        if let Some(i) = &meta.via_interface {
            row.push_str(&format!(", \"via_interface\": {}", quote(i)));
        }
        row.push_str("},\n");
        model.push_str(&row);
    }
    trim_trailing_comma(&mut model);
    // Contracted through-stdlib user→user edges (#392): what the
    // evaluator's stdlib-merged walk reaches, collapsed to user
    // endpoints. Reachability replay composes `calls` ∪ this.
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
    // Groups, labels, and unknowns are VERIFICATION-relevant, so
    // they live inside the hashed model half: a carrier added, a
    // group widened, or a new fail-closed site all change the
    // shape identity (the review's "two identities" concern,
    // resolved by making the one hash cover what evaluation reads).
    model.push_str("  \"groups\": {\n");
    for (g, members) in &group_rows {
        model.push_str(&format!(
            "    {}: [{}],\n",
            quote(g),
            join_str(members.iter())
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
    // #392: the phase relation, the seed sort, and the compiler-
    // derived per-fn effect sets — the rows `during`, `cover`, and
    // effect-class endpoints evaluate against. Hashed: each is
    // verification-relevant, so changing one changes the identity.
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
    model.push_str("  },\n  \"unknowns\": [\n");
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

    let shape_hash = fnv1a64(model.as_bytes());

    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!(
        "  \"schema\": {},\n",
        quote(TOPOLOGY_SCHEMA)
    ));
    out.push_str(&format!(
        "  \"shape_hash\": \"{:016x}\",\n",
        shape_hash
    ));
    out.push_str(&model);
    // Provenance (#392): source spans as bundle-global byte offsets
    // [start, end]. UNHASHED on purpose — moving code must not
    // change the shape identity — so it sits in the results half
    // beside the claim rows.
    out.push_str(",\n  \"provenance\": {\n    \"calls\": [\n");
    for (from, to, s, e) in &call_spans {
        out.push_str(&format!(
            "      {{\"from\": {}, \"to\": {}, \"span\": [{}, {}]}},\n",
            quote(from),
            quote(to),
            s,
            e
        ));
    }
    trim_trailing_comma(&mut out);
    out.push_str("    ],\n    \"publishes\": [\n");
    for (f, subj, s, e) in &publish_spans {
        out.push_str(&format!(
            "      {{\"fn\": {}, \"subject\": {}, \"span\": [{}, {}]}},\n",
            quote(f),
            quote(subj),
            s,
            e
        ));
    }
    trim_trailing_comma(&mut out);
    out.push_str("    ],\n    \"subscribes\": [\n");
    for (subj, locus, handler, s, e) in &subscribe_spans {
        out.push_str(&format!(
            "      {{\"subject\": {}, \"locus\": {}, \"handler\": {}, \
             \"span\": [{}, {}]}},\n",
            quote(subj),
            quote(locus),
            quote(handler),
            s,
            e
        ));
    }
    trim_trailing_comma(&mut out);
    out.push_str("    ],\n    \"decls\": {\n");
    for (decl, (s, e)) in &decl_spans {
        out.push_str(&format!(
            "      {}: [{}, {}],\n",
            quote(decl),
            s,
            e
        ));
    }
    trim_trailing_comma(&mut out);
    out.push_str("    }\n  }");
    // #399: the per-topic OBSERVATION identity — the join between
    // this artifact and a recording. The runtime manifest fuses
    // topics on (name, shape_hash) where shape_hash =
    // FNV-1a/64(wire_subject ++ ':' ++ canonical_shape); a WAL
    // segment carrying that pair matches a row here, which names
    // the exact checked topology it ran under. Computed by the
    // SAME `topic_identity` functions codegen registers shapes
    // with, so the artifact and the emitted binary cannot drift.
    // UNHASHED by ruling: payload field shape does not affect
    // claim evaluation, so it is not part of the model identity —
    // the artifact document is the reference, not the fusion.
    out.push_str(",\n  \"topics\": [\n");
    {
        let mut rows: BTreeSet<(String, String, String, u64)> =
            BTreeSet::new();
        for p in &programs {
            let wire =
                crate::topic_identity::topic_wire_subjects(&p.items);
            fn topics_of<'a>(
                items: &'a [TopDecl],
                out: &mut Vec<&'a TopicDecl>,
            ) {
                for item in items {
                    match item {
                        TopDecl::Topic(t) => out.push(t),
                        TopDecl::Module(m) => {
                            topics_of(&m.items, out)
                        }
                        _ => {}
                    }
                }
            }
            let mut ts = Vec::new();
            topics_of(&p.items, &mut ts);
            for t in ts {
                let subj = wire
                    .get(&t.name.name)
                    .cloned()
                    .unwrap_or_else(|| t.name.name.clone());
                let shape =
                    crate::topic_identity::canonical_topic_shape(
                        &p.items, t,
                    );
                let h = crate::topic_identity::topic_shape_hash(
                    &subj, &shape,
                );
                rows.insert((
                    name(&t.name.name),
                    subj,
                    shape,
                    h,
                ));
            }
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
    out.push_str(",\n  \"claims\": [\n");
    for o in &outcomes {
        out.push_str(&format!(
            "    {{\"name\": {}, \"form\": {}, \"result\": {}}},\n",
            quote(&o.name),
            quote(&demangle_str(&o.form)),
            quote(o.result)
        ));
    }
    trim_trailing_comma(&mut out);
    // #392 §8: every fn-grained certificate — `@effects` asserts,
    // `@phase_effects` contracts, `@budget` in both families —
    // lowered to the claim IR's vocabulary with its verdict, from
    // the same evaluations that gate the build. One schema of
    // record: the artifact carries ALL law, bundle-quantified and
    // fn-grained, in one place. Unhashed like the claim results —
    // rows are law + verdicts, not topology.
    let mut lowered = crate::effects::certificate_rows(
        &programs,
        &bundle.import_renames,
    );
    lowered.extend(crate::budget_check::certificate_rows(
        &programs,
        &bundle.import_renames,
    ));
    let fanout = |subj: &str| -> u64 {
        graph
            .subjects
            .get(subj)
            .map(|si| si.subscribers.len().max(1) as u64)
            .unwrap_or(1)
    };
    lowered.extend(crate::quantitative::certificate_rows(
        &programs, &fanout,
    ));
    out.push_str(",\n  \"lowered\": [\n");
    for r in &lowered {
        out.push_str(&format!(
            "    {{\"subject\": {}, \"form\": {}, \"result\": {}}},\n",
            quote(&demangle_str(&r.subject)),
            quote(&demangle_str(&r.form)),
            quote(if r.violated { "violated" } else { "holds" })
        ));
    }
    trim_trailing_comma(&mut out);
    out.push_str("  ]\n}\n");
    out
}

fn join_str<'a>(items: impl Iterator<Item = &'a String>) -> String {
    items
        .map(|s| quote(s))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Minimal JSON string escaping — names are identifiers and wire
/// subjects, but fail-closed on the full set anyway.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Drop the trailing ",\n" of the last array element (valid JSON).
fn trim_trailing_comma(s: &mut String) {
    if s.ends_with(",\n") {
        s.truncate(s.len() - 2);
        s.push('\n');
    }
}

/// FNV-1a, 64-bit — the runtime's hash family (lotus_obs.c uses
/// FNV for the per-topic payload shape); deterministic, dependency-
/// free, stable across platforms.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}
