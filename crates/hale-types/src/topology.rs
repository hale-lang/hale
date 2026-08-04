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
//! something that appears nowhere in anyone's source.
//!
//! Consumed by `hale check <t> --dump-topology` and diffed by
//! `--check-topology <path>` — the `.hale.effects` manifest
//! precedent: emit for review, commit, and an unreviewed topology
//! change fails CI the way an API break does.
//!
//! v1 SCOPE (honest contract): the artifact carries the sorts, the
//! call/publish/subscribe relations, the declared groups, the
//! effect labels (declared carriers), and the UNKNOWNS (fns with
//! indirect calls, untyped-receiver method calls — recorded with
//! the callee name so an outside evaluator can apply the same
//! fail-closed rule — or computed publish subjects: every place
//! the evaluator failed closed). That is enough to independently
//! re-evaluate the reachability-class claims (`forbid reaches`,
//! `only edges`, `require`/`cover`/`count`) and to audit where
//! certification stopped. It is NOT yet the complete normalized
//! verification model (no per-edge spans, weights, phase relation,
//! or seed-membership sort); exporting that model is the
//! architectural milestone tracked on #382, and until then this is
//! a topology + claims REPORT whose passing rows are certified by
//! the derivation, not re-derivable from the file alone.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::*;

use crate::alloc_summary::{self, Callee, EffectSiteKind, FnKey};
use crate::symbol::Bundle;

/// The artifact's schema version. Additions are minor versions;
/// changes are breaking.
pub const TOPOLOGY_SCHEMA: &str = "1.0";

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

    // ---- relations (with provenance implicit in the names) ----
    let mut calls: BTreeSet<(String, String)> = BTreeSet::new();
    let mut publishes: BTreeSet<(String, String)> = BTreeSet::new();
    for (k, fs) in &summary.fns {
        if !user_key(k) {
            continue;
        }
        for edge in &fs.calls {
            if let Callee::Resolved(next) = &edge.callee {
                if user_key(next) {
                    calls.insert((fn_name(k), fn_name(next)));
                }
            }
        }
        for site in &fs.effect_sites {
            if let EffectSiteKind::Publish(Some(s)) = &site.kind {
                publishes.insert((fn_name(k), name(s)));
            }
        }
    }
    let mut subscribes: BTreeSet<(String, String, String)> =
        BTreeSet::new();
    for (subject, info) in &graph.subjects {
        for s in &info.subscribers {
            subscribes.insert((
                name(subject),
                name(&s.locus),
                s.handler.clone(),
            ));
        }
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
    let effect_names = crate::effects::effect_names_of(&programs);
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
    for (from, to) in &calls {
        model.push_str(&format!(
            "      {{\"from\": {}, \"to\": {}}},\n",
            quote(from),
            quote(to)
        ));
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
