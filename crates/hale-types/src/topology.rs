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

/// 1.10 (downstream handoff P25, 2026-08-12): `supervision` in the
/// HASHED half — one row per `on_failure` handler (supervising
/// locus, supervised child + error types, the recovery ops the body
/// invokes, a literal retry bound when written), plus spanned
/// `provenance.supervision` rows. Existing `shape_hash` values
/// change. The observer's live RESTART/SUPERV_TRANS/DISSOLVE stream
/// finally has declared policy to anchor to.
///
/// 1.9 (GH #436 review): `labels.sealed` in the HASHED half — the
/// loci whose state is confined. Sealing is a structural
/// confidentiality property, and without it in the model a locus
/// could gain or lose `@sealed` with no `shape_hash` diff at all,
/// which is exactly the invisible security change the artifact
/// exists to surface. Existing `shape_hash` values change.
///
/// The artifact's schema version. Additions are minor versions;
/// changes are breaking. 1.1 (#392): weights on call edges,
/// `calls_via_stdlib`, `phases`, `seeds`, `effects` in the hashed
/// half (existing `shape_hash` values change); unhashed
/// `provenance` section. 1.2 (#399): unhashed `topics` section —
/// the per-topic OBSERVATION identity (wire subject, canonical
/// payload shape, `payload_hash`), the join key a recording/WAL
/// segment carries; model `shape_hash` values unchanged. 1.3:
/// unhashed-by-`shape_hash` but now COVERED `artifact_digest` — a
/// whole-body integrity hash as the final key, so a consumer that
/// trusts an artifact it did not produce can verify the sections
/// `shape_hash` omits (`topics`, `provenance`, claim results). 1.4:
/// `verdict` (`clean` / `law_failed`), the document's own outcome;
/// and one result vocabulary across `claims` and `lowered` — see
/// [`crate::verdict::Verdict`], which adds `uncertified` as a state
/// distinct from `violated`. 1.5 (#409): a claim row gains an
/// optional `source` — the constitution an adopted clause came from.
/// 1.6 (#409 review): an `evaluation` section naming the adopted
/// constitutions and the digest of each one's normalized closure, so
/// two entrypoints can be shown to have resolved the SAME claimset
/// rather than merely the same name. 1.7 (#415 review 2): that
/// section splits into `roots` (named directly) and `closure`
/// (everything they reach), and gains the `environment` label —
/// identities now come from the adoption traversal, so a constitution
/// contributing no clause of its own is no longer invisible.
/// 1.11 (GH #476 Change 6): three unhashed, digest-covered typed
/// sections — `law` (every lowered ClaimIr row: ordinal, name,
/// origin, judgment family, machine verdict, provenance; plus
/// `law_digest` and `inputs_digest`, the sidecar ties a consumer
/// checks before trusting external evidence against this
/// artifact), `capabilities` (the model's positive completeness
/// account, typed), and `adequacy` (per migrated judgment family:
/// `exact` when capabilities vouch every relation family that
/// judgment consumes, else `degraded`). The legacy `claims` /
/// `lowered` string rows remain, now PROJECTED from the same
/// canonical path.
pub const TOPOLOGY_SCHEMA: &str = "1.11";

/// GH #408 Phase 0: what the rows MEAN, as distinct from their shape.
///
/// `schema` says a row has these fields. It cannot say that "an
/// interface dispatch fans out to every conformer" or "unknown
/// implies violation" were the rules in force when the rows were
/// produced. Two compilers agreeing on the schema and disagreeing on
/// the semantics would compose artifacts into a model neither of them
/// would certify — and nothing in the document would reveal it.
///
/// Bump whenever the interpretation of any row changes, even when its
/// shape does not. A consumer that does not recognise the value must
/// refuse rather than assume equivalence.
/// 2 (GH #476 Change 6): law verdicts come from the canonical
/// judgments, whose interpretation is stricter in two documented
/// places — a certificate naming a cyclically-defined or
/// undeclared effect class reports `invalid` (previously a vacuous
/// `holds`), and `require attributed` over a body the analysis
/// could not walk reports `uncertified` (previously a fail-open
/// `holds`).
pub const MODEL_SEMANTICS: u32 = 2;

/// The model identity alone (downstream handoff P26, 2026-08-12):
/// the same `shape_hash` `dump_topology` stamps, for embedding in
/// the built binary's observation segment. Extracted from the full
/// serialization rather than recomputed, so the two can never
/// drift — the cost (one artifact render at build time) is the
/// same analysis stack `hale check` runs in ~10 ms on the largest
/// apps.
pub fn model_shape_hash(bundle: &Bundle<'_>) -> u64 {
    let art = dump_topology(bundle);
    art.lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("\"shape_hash\": \"")?
                .strip_suffix("\",")
        })
        .and_then(|h| u64::from_str_radix(h, 16).ok())
        .unwrap_or(0)
}

/// Serialize the bundle's model + claim results as the topology
/// artifact (JSON).
pub fn dump_topology(bundle: &Bundle<'_>) -> String {
    dump_topology_parts(bundle).0
}

/// The artifact PLUS the legacy model-half string (GH #476
/// Change 6, review round 1): production emits the PROJECTION of
/// `ApplicationModel` (`project_model_half` — one semantic
/// authority); the legacy gathering remains ONLY as the
/// differential's comparison arm until Change 9 retires it. It
/// never supplies emitted rows.
#[doc(hidden)]
pub fn dump_topology_parts(
    bundle: &Bundle<'_>,
) -> (String, String) {
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
                publishes.insert((fn_name(k), name(&s.text)));
                publish_spans.insert((
                    fn_name(k),
                    name(&s.text),
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
    // The walk itself lives in `callgraph::legacy_via_stdlib_contraction`,
    // shared with the model builder's `LegacyProjection` so the
    // projected `TopologyShapeV1` hash and this serialization cannot
    // drift (GH #476 Change 2, review round 9).
    let mut via_stdlib: BTreeMap<(String, String), bool> =
        BTreeMap::new();
    for ((k, next), looped) in
        crate::callgraph::legacy_via_stdlib_contraction(&merged, &user_key)
    {
        let e = via_stdlib
            .entry((fn_name(&k), fn_name(&next)))
            .or_insert(false);
        *e |= looped;
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
    // Downstream handoff P24 (2026-08-12): topic DECLARATIONS are
    // spanned decls too. `provenance.publishes` / `.subscribes`
    // carry the sites, but the `topic Orders { … }` line — the one
    // a developer looks at when asking "is this topic live?" — had
    // no entry, so an editor lens could anchor on every use and
    // not on the declaration. Every name in `sorts.topics` now has
    // a `provenance.decls` row.
    {
        fn walk_topics<'a>(
            items: &'a [TopDecl],
            out: &mut Vec<&'a TopicDecl>,
        ) {
            for item in items {
                match item {
                    TopDecl::Topic(t) => out.push(t),
                    TopDecl::Module(m) => walk_topics(&m.items, out),
                    _ => {}
                }
            }
        }
        let mut topic_decls = Vec::new();
        for p in &programs {
            walk_topics(&p.items, &mut topic_decls);
        }
        for t in topic_decls {
            decl_spans.entry(name(&t.name.name)).or_insert((
                t.name.span.start.as_usize() as u32,
                t.name.span.end.as_usize() as u32,
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

    // ---- supervision (downstream handoff P25, 2026-08-12) ----
    //
    // `on_failure` had NO representation: not a decl, not a
    // relation, no section — while RESTART / SUPERV_TRANS /
    // LOCUS_DISSOLVE are the richest live signal an observer has,
    // with nothing in the model to anchor to ("declared retry cap
    // 3, observed 3 in 40s" was structurally impossible). One row
    // per on_failure handler: the supervising locus, the
    // supervised child + error types, the recovery ops its body
    // invokes, and a literal retry bound when one is written
    // (`restart(c) for N`). HASHED — a policy change is a
    // topology change. Spans ride in provenance.supervision.
    struct SupRow {
        locus: String,
        child: String,
        err: String,
        ops: Vec<String>,
        retry: Option<i64>,
        span: (u32, u32),
    }
    let mut sup_rows: Vec<SupRow> = Vec::new();
    {
        fn te_name(t: &TypeExpr) -> String {
            match t {
                TypeExpr::Named { path, .. } => path
                    .segments
                    .iter()
                    .map(|s| s.name.clone())
                    .collect::<Vec<_>>()
                    .join("::"),
                _ => "?".to_string(),
            }
        }
        fn walk_ops(
            b: &Block,
            ops: &mut Vec<String>,
            retry: &mut Option<i64>,
        ) {
            for st in &b.stmts {
                match st {
                    Stmt::Recovery { op, modifier, .. } => {
                        let n = match op {
                            RecoveryOp::Restart => "restart",
                            RecoveryOp::RestartInPlace => {
                                "restart_in_place"
                            }
                            RecoveryOp::Quarantine => "quarantine",
                            RecoveryOp::Reorganize => "reorganize",
                            RecoveryOp::Bubble => "bubble",
                        };
                        if !ops.iter().any(|o| o == n) {
                            ops.push(n.to_string());
                        }
                        if let Some(RecoveryModifier::For(
                            Expr::Literal(Literal::Int(k), _),
                        )) = modifier
                        {
                            *retry = Some(*k);
                        }
                    }
                    Stmt::If(i) => {
                        walk_ops(&i.then_block, ops, retry);
                        let mut cur = i.else_block.as_deref();
                        while let Some(eb) = cur {
                            match eb {
                                ElseBranch::Else(bb) => {
                                    walk_ops(bb, ops, retry);
                                    cur = None;
                                }
                                ElseBranch::ElseIf(ei) => {
                                    walk_ops(&ei.then_block, ops, retry);
                                    cur = ei.else_block.as_deref();
                                }
                            }
                        }
                    }
                    Stmt::While { body, .. }
                    | Stmt::For { body, .. } => walk_ops(body, ops, retry),
                    Stmt::Block(bb) => walk_ops(bb, ops, retry),
                    _ => {}
                }
            }
        }
        fn walk_loci<'a>(
            items: &'a [TopDecl],
            out: &mut Vec<&'a LocusDecl>,
        ) {
            for item in items {
                match item {
                    TopDecl::Locus(l) => out.push(l),
                    TopDecl::Module(m) => walk_loci(&m.items, out),
                    _ => {}
                }
            }
        }
        let mut loci = Vec::new();
        for p in &programs {
            walk_loci(&p.items, &mut loci);
        }
        for l in loci {
            for member in &l.members {
                if let LocusMember::Failure(fd) = member {
                    let mut ops = Vec::new();
                    let mut retry = None;
                    walk_ops(&fd.body, &mut ops, &mut retry);
                    sup_rows.push(SupRow {
                        locus: name(&l.name.name),
                        child: fd
                            .params
                            .first()
                            .map(|p| name(&te_name(&p.ty)))
                            .unwrap_or_else(|| "?".to_string()),
                        err: fd
                            .params
                            .get(1)
                            .map(|p| name(&te_name(&p.ty)))
                            .unwrap_or_else(|| "?".to_string()),
                        ops,
                        retry,
                        span: (
                            fd.span.start.as_usize() as u32,
                            fd.span.end.as_usize() as u32,
                        ),
                    });
                }
            }
        }
        sup_rows.sort_by(|a, b| {
            (&a.locus, &a.child).cmp(&(&b.locus, &b.child))
        });
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
    // GH #476 Change 6: the artifact's law rows are PROJECTED from
    // the canonical path (ClaimIr renders the forms, the Change-5
    // judgments produce the verdicts, the evidence sidecar carries
    // the certificate results). The evaluator report is still run
    // for the constitution identities — and it remains the CHECK
    // authority until Change 9; the artifact_law_projection corpus
    // differential holds the two row sets equal (modulo the
    // Change-5 documented divergences, which the SEMANTICS bump
    // records).
    let (_diags, _old_outcomes, identities) =
        crate::claims::claims_report_with_identities(
        &programs,
        &graph,
        &bundle.import_renames,
    );
    let vmodel = crate::model_builder::derive_application_model(bundle);
    let law_table = crate::claim_lowering::lower_claims(bundle, &vmodel);
    let law_evidence = crate::evidence::derive_certificate_evidence(
        bundle, &law_table, &vmodel,
    );
    let source_bases: Vec<u32> =
        bundle.sources.iter().map(|f| f.base).collect();
    let legacy_unmigrated =
        crate::topology_projection::legacy_unmigrated_verdicts(
            bundle, &graph, &law_table,
        );
    let (outcomes, projected_lowered, law_rows) =
        crate::topology_projection::project_law_rows(
            bundle,
            &vmodel,
            &law_table,
            &law_evidence,
            &source_bases,
            &legacy_unmigrated,
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
    // GH #436 review: sealing is a MODEL fact, not merely a claim
    // input. A locus gaining or losing `@sealed` changes what the
    // program structurally confines; if `shape_hash` did not move,
    // the one diff a reviewer most needs to see would be invisible.
    let sealed: BTreeSet<String> = {
        let mut out = BTreeSet::new();
        fn walk(items: &[TopDecl], out: &mut BTreeSet<String>) {
            for item in items {
                match item {
                    TopDecl::Locus(l) if l.sealed => {
                        out.insert(l.name.name.clone());
                    }
                    TopDecl::Module(m) => walk(&m.items, out),
                    _ => {}
                }
            }
        }
        for p in &programs {
            walk(&p.items, &mut out);
        }
        out
    };
    // Its own key rather than a row inside `labels`: that map is
    // fn -> effect classes, and a locus-level structural property is
    // a different shape. (An earlier draft emitted a second `labels`
    // object, producing a duplicate JSON key that every parser
    // silently resolved to the LAST one — the sealed set vanished
    // while `shape_hash` still moved, which is the worst of both.)
    model.push_str(&format!(
        "  \"sealed\": [{}],\n",
        join_str(sealed.iter().map(|s| name(s)).collect::<Vec<_>>().iter())
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

    // GH #476 Change 6 (review round 1): the emitted model half is
    // the PROJECTION — artifact generation projects the model
    // (acceptance criterion 6). The legacy string just built above
    // is returned for the corpus differential only.
    let legacy_model = model;
    let model =
        crate::topology_projection::project_model_half(&vmodel);
    let shape_hash = fnv1a64(model.as_bytes());
    debug_assert_eq!(
        shape_hash,
        crate::topology_projection::project_shape_hash(&vmodel)
    );

    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!(
        "  \"schema\": {},\n  \"semantics\": {},\n",
        quote(TOPOLOGY_SCHEMA),
        MODEL_SEMANTICS
    ));
    out.push_str(&format!(
        "  \"shape_hash\": \"{:016x}\",\n",
        shape_hash
    ));
    out.push_str(&model);
    // GH #408 Phase 0: the source map, so a span means something
    // outside the process that produced it.
    //
    // Bundle-global offsets are a concatenation artifact. A consumer
    // composing artifacts from separately compiled applications
    // cannot turn `[1204, 1231]` into a location, so no cross-artifact
    // witness could say where to look — which is most of what a
    // witness is for. Paths are relative to the checked target and
    // carry a content digest, so an artifact stays comparable across
    // machines and a consumer can tell a stale pairing from a fresh
    // one.
    out.push_str(",\n  \"sources\": [\n");
    for (i, sf) in bundle.sources.iter().enumerate() {
        out.push_str(&format!(
            "    {{\"id\": {}, \"path\": {}, \"digest\": {}}}{}\n",
            sf.id,
            quote(&sf.path),
            quote(&sf.digest),
            if i + 1 == bundle.sources.len() { "" } else { "," }
        ));
    }
    out.push_str("  ]");

    // Provenance (#392): source spans, now resolved to
    // `(source, [local_start, local_end])`. UNHASHED by `shape_hash`
    // on purpose — moving code must not change the shape identity —
    // so it sits in the results half beside the claim rows.
    let loc = |pos: u32| -> (i64, u32) {
        match bundle
            .sources
            .iter()
            .filter(|f| pos >= f.base && pos < f.base.saturating_add(f.len + 1))
            .max_by_key(|f| f.base)
        {
            Some(f) => (f.id as i64, pos - f.base),
            // -1 rather than a guessed file: a span the map cannot
            // place is better reported as unplaceable than attributed
            // to the wrong source.
            None => (-1, pos),
        }
    };
    out.push_str(",\n  \"provenance\": {\n    \"calls\": [\n");
    for (from, to, s, e) in &call_spans {
        out.push_str(&format!(
            "      {{\"from\": {}, \"to\": {}, \"source\": {}, \"span\": [{}, {}]}},\n",
            quote(from),
            quote(to),
            loc(*s).0,
            loc(*s).1,
            loc(*e).1
        ));
    }
    trim_trailing_comma(&mut out);
    out.push_str("    ],\n    \"publishes\": [\n");
    for (f, subj, s, e) in &publish_spans {
        out.push_str(&format!(
            "      {{\"fn\": {}, \"subject\": {}, \"source\": {}, \"span\": [{}, {}]}},\n",
            quote(f),
            quote(subj),
            loc(*s).0,
            loc(*s).1,
            // BOTH endpoints localize. This end was the raw
            // bundle-global offset while every other provenance
            // section localized both, so a publish in any source
            // whose virtual base is nonzero produced a row naming a
            // file and a span reaching past the end of it — a
            // file-local start with a bundle-global end, which is not
            // a coordinate in any single system. A consumer resolving
            // it lands outside the file it was told to open.
            loc(*e).1
        ));
    }
    trim_trailing_comma(&mut out);
    out.push_str("    ],\n    \"subscribes\": [\n");
    for (subj, locus, handler, s, e) in &subscribe_spans {
        out.push_str(&format!(
            "      {{\"subject\": {}, \"locus\": {}, \"handler\": {}, \
             \"source\": {}, \"span\": [{}, {}]}},\n",
            quote(subj),
            quote(locus),
            quote(handler),
            loc(*s).0,
            loc(*s).1,
            loc(*e).1
        ));
    }
    trim_trailing_comma(&mut out);
    out.push_str("    ],\n    \"decls\": {\n");
    for (decl, (s, e)) in &decl_spans {
        out.push_str(&format!(
            "      {}: {{\"source\": {}, \"span\": [{}, {}]}},\n",
            quote(decl),
            loc(*s).0,
            loc(*s).1,
            loc(*e).1
        ));
    }
    trim_trailing_comma(&mut out);
    out.push_str("    },\n    \"supervision\": [\n");
    for r in &sup_rows {
        out.push_str(&format!(
            "      {{\"locus\": {}, \"child\": {}, \"source\": {}, \"span\": [{}, {}]}},\n",
            quote(&r.locus),
            quote(&r.child),
            loc(r.span.0).0,
            loc(r.span.0).1,
            loc(r.span.1).1
        ));
    }
    trim_trailing_comma(&mut out);
    out.push_str("    ]\n  }");
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
        // GH #409: `source` names the constitution an adopted clause
        // came from, absent for one written in this main. It is what
        // makes "product law or environment rail?" answerable by
        // looking, and it is what a workspace check reads to ask
        // whether every entrypoint adopted the shared claimset.
        let src = match &o.source {
            Some(c) => format!(", \"source\": {}", quote(c)),
            None => String::new(),
        };
        out.push_str(&format!(
            "    {{\"name\": {}, \"form\": {}, \"result\": {}{}}},\n",
            quote(&o.name),
            quote(&demangle_str(&o.form)),
            quote(o.result.as_str()),
            src
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
    // Effects-family certificates come from the evidence sidecar
    // (Change 6); `@budget` rows keep their old producers until the
    // quantitative engines migrate (JudgmentFamily::Unmigrated).
    let mut lowered: Vec<crate::effects::LoweredCertificate> =
        projected_lowered
            .iter()
            .map(|r| crate::effects::LoweredCertificate {
                subject: r.subject.clone(),
                form: r.form.clone(),
                result: r.result,
            })
            .collect();
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
    // Close the `claims` array before opening `lowered` — omitting
    // this emitted a document no standards-compliant JSON parser
    // accepts, for every shape (no claims, one, many, with or
    // without lowered rows). It survived because the artifact tests
    // asserted on substrings and never parsed the whole document;
    // `topology_artifact_is_valid_json` now does.
    out.push_str("  ],\n  \"lowered\": [\n");
    for r in &lowered {
        out.push_str(&format!(
            "    {{\"subject\": {}, \"form\": {}, \"result\": {}}},\n",
            quote(&demangle_str(&r.subject)),
            quote(&demangle_str(&r.form)),
            quote(r.result.as_str())
        ));
    }
    trim_trailing_comma(&mut out);
    out.push_str("  ]");

    // GH #476 Change 6: the TYPED law section — every lowered
    // ClaimIr row with its judgment family and machine verdict,
    // addressable by ordinal, plus the two digests a consumer
    // checks before trusting external evidence against this
    // artifact. Unhashed by `shape_hash` (law rows are results,
    // not topology), covered by `artifact_digest`.
    out.push_str(",\n  \"law\": {\n");
    out.push_str(&format!(
        "    \"law_digest\": \"{:016x}\",\n",
        law_table.semantic_digest()
    ));
    out.push_str(&format!(
        "    \"inputs_digest\": \"{:016x}\",\n",
        law_evidence.inputs_digest
    ));
    out.push_str("    \"rows\": [\n");
    for r in &law_rows {
        let prov = match &r.provenance {
            Some((file, a, b)) => format!(
                ", \"file\": {}, \"span\": [{}, {}]",
                quote(file),
                a,
                b
            ),
            None => String::new(),
        };
        let certs = if r.certs.is_empty() {
            String::new()
        } else {
            let cs: Vec<String> = r
                .certs
                .iter()
                .map(|(i, form, res)| {
                    format!(
                        "{{\"ordinal\": {}, \"form\": {}, \
                         \"result\": {}}}",
                        i,
                        quote(&demangle_str(form)),
                        quote(res.as_str())
                    )
                })
                .collect();
            format!(", \"certs\": [{}]", cs.join(", "))
        };
        out.push_str(&format!(
            "      {{\"ordinal\": {}, \"name\": {}, \"origin\": {}, \
             \"family\": {}, \"verdict\": {}, \"law\": {}{}{}}},\n",
            r.ordinal,
            quote(&demangle_str(&r.name)),
            quote(&r.origin),
            quote(r.family.as_str()),
            quote(r.verdict.as_str()),
            // Verbatim: the payload carries RAW (canonical) and
            // DISPLAY spellings side by side — demangling the
            // whole object would collapse the raw identity.
            r.law,
            certs,
            prov
        ));
    }
    trim_trailing_comma(&mut out);
    out.push_str("    ]\n  }");

    // The model's positive completeness account, typed — what the
    // artifact can promise is exact, without reverse-engineering
    // the `unknowns` strings.
    out.push_str(",\n  \"capabilities\": {\n");
    {
        let caps = vmodel.capabilities.vouched_families();
        for (i, (cname, claimed, _)) in caps.iter().enumerate() {
            out.push_str(&format!(
                "    \"{}\": {}{}\n",
                cname,
                claimed,
                if i + 1 == caps.len() { "" } else { "," }
            ));
        }
    }
    out.push_str("  }");

    // Per migrated judgment family: can this model support the
    // family's judgment EXACTLY (`exact`), or do holes degrade it
    // (`degraded` — judgments still run; reachable holes force
    // `uncertified`)?
    out.push_str(",\n  \"adequacy\": {\n");
    {
        let adequacy =
            crate::topology_projection::family_adequacy(&vmodel);
        for (i, (fam, exact)) in adequacy.iter().enumerate() {
            out.push_str(&format!(
                "    \"{}\": {}{}\n",
                fam.as_str(),
                quote(if *exact { "exact" } else { "degraded" }),
                if i + 1 == adequacy.len() { "" } else { "," }
            ));
        }
    }
    out.push_str("  }");

    // GH #409 (review finding 5): WHICH evaluation this artifact
    // certifies. Per-claim `source` answers "where did this clause
    // come from"; it cannot answer "which deployment was this run
    // for". Two environments extending one base can produce
    // identical claim rows, so without this the artifacts of a dev
    // check and a prod check are indistinguishable while certifying
    // different things.
    //
    // Inside the digest-covered body: an evaluation context that
    // could be edited after the fact would certify nothing.
    out.push_str(",\n  \"evaluation\": {\n");
    // The environment this run was for, when one was named. Two
    // environment labels selecting identical law produce equivalent
    // certificates on the `closure` alone — but the prose promised
    // this section says WHICH deployment was certified, and only the
    // label can say that.
    if let Some(env) = crate::claims::current_environment() {
        out.push_str(&format!(
            "    \"environment\": {},\n",
            quote(&env)
        ));
    }
    // `roots` — what was asked for. `closure` — what applied.
    //
    // Both come from the adoption traversal. Deriving them from the
    // `source` of emitted claim rows dropped every constitution that
    // contributes no clause of its own, so a directly-selected
    // `constitution Dev extends Left { }` never appeared at all.
    let mut section = |label: &str, ids: &[crate::claims::ConstitutionIdentity], last: bool| {
        out.push_str(&format!("    \"{}\": [\n", label));
        for (i, id) in ids.iter().enumerate() {
            out.push_str(&format!(
                "      {{\"name\": {}, \"digest\": {}}}{}\n",
                quote(&id.name),
                quote(&id.digest),
                if i + 1 == ids.len() { "" } else { "," }
            ));
        }
        out.push_str(if last { "    ]\n" } else { "    ],\n" });
    };
    section("roots", &identities.roots, false);
    section("closure", &identities.closure, true);
    out.push_str("  }");

    // The document's own verdict (schema 1.4). Every law in this
    // artifact — bundle claims and fn-grained certificates alike —
    // reduces to one field a consumer can read without scanning
    // rows.
    //
    // Composing artifacts across binaries requires "did this
    // component pass?" as a precondition, and reconstructing that by
    // walking two arrays and knowing which verdict strings count as
    // passing is exactly the kind of thing a consumer gets subtly
    // wrong. `Verdict::passed()` is the single definition, and only
    // `holds` passes — `uncertified` does not, because a law that
    // could not be checked has not been satisfied.
    //
    // Note this says nothing about whether the program TYPECHECKS.
    // It does not have to: an artifact is only emitted for a program
    // that does, so its existence already carries that.
    // Change 6: the MACHINE verdicts join the pass condition. For
    // the migrated families the law rows are the judgment's word —
    // stricter than the engine replay in the two documented places
    // (cyclic/undeclared classes ⇒ invalid; attributed-over-hole ⇒
    // uncertified). Unmigrated rows carry the OLD engines'
    // authoritative results (`legacy_unmigrated_verdicts`), so
    // EVERY application-tier row participates: no non-passing law
    // row can coexist with a `clean` document verdict (round 1).
    let law_pass = law_rows.iter().all(|r| {
        matches!(r.family, hale_model::JudgmentFamily::Fleet)
            || r.verdict.passed()
    });
    let all_pass = outcomes.iter().all(|o| o.result.passed())
        && lowered.iter().all(|r| r.result.passed())
        && law_pass;
    out.push_str(&format!(
        ",\n  \"verdict\": {}",
        quote(if all_pass { "clean" } else { "law_failed" })
    ));

    // Integrity (schema 1.3). `shape_hash` is an IDENTITY, not an
    // integrity check: it deliberately covers the model half only,
    // so `topics`, `provenance` and the claim results all sit
    // outside it. That is right for what those sections were for —
    // moving a comment must not churn the model identity — but it
    // leaves two holes the moment anything TRUSTS an artifact it
    // did not produce:
    //
    //   * cross-binary composition joins endpoints on the `topics`
    //     rows (wire subject + payload hash). Verifying `shape_hash`
    //     and then joining on unhashed rows means the join key is
    //     outside the thing that was verified.
    //   * a baseline gate that greps `shape_hash` out of a file can
    //     be defeated by editing that one line — the rest of the
    //     document need not agree with it.
    //
    // So the digest covers the ENTIRE body, results and provenance
    // included, and is the last key: everything preceding it is
    // exactly what was hashed, which makes verification a prefix
    // hash with no need to re-serialize or canonicalize.
    let digest = fnv1a64(out.as_bytes());
    out.push_str(&format!(
        "{}{:016x}\"\n}}\n",
        ARTIFACT_DIGEST_KEY, digest
    ));
    (out, legacy_model)
}

/// The exact byte sequence introducing the integrity digest. It is
/// the artifact's final key, so everything before this marker is the
/// hashed body. Written once and shared by the emitter and the
/// verifier so the two cannot drift.
pub const ARTIFACT_DIGEST_KEY: &str = ",\n  \"artifact_digest\": \"";

/// Verify an artifact's integrity digest.
///
/// `None` means the document carries no digest — every artifact
/// emitted before schema 1.3. That is reported distinctly from
/// `Some(false)` on purpose: a consumer may choose to accept an
/// older artifact, but it must never mistake "nothing to check"
/// for "checked and intact".
pub fn verify_artifact_digest(artifact: &str) -> Option<bool> {
    // rfind: the digest is the final key, and searching from the end
    // means a user-authored string that happens to contain the
    // marker cannot shadow the real one.
    let at = artifact.rfind(ARTIFACT_DIGEST_KEY)?;
    let body = &artifact[..at];
    let rest = &artifact[at + ARTIFACT_DIGEST_KEY.len()..];
    let claimed = rest.split('"').next()?;
    Some(claimed == format!("{:016x}", fnv1a64(body.as_bytes())))
}

pub(crate) fn join_str<'a>(items: impl Iterator<Item = &'a String>) -> String {
    items
        .map(|s| quote(s))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Minimal JSON string escaping — names are identifiers and wire
/// subjects, but fail-closed on the full set anyway.
pub(crate) fn quote(s: &str) -> String {
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
pub(crate) fn trim_trailing_comma(s: &mut String) {
    if s.ends_with(",\n") {
        s.truncate(s.len() - 2);
        s.push('\n');
    }
}

/// FNV-1a, 64-bit — the runtime's hash family (lotus_obs.c uses
/// FNV for the per-topic payload shape); deterministic, dependency-
/// free, stable across platforms.
pub(crate) fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}
