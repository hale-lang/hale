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

use crate::alloc_summary::{self, FnKey};
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
// 1.12 (GH #476 Change 7): canonical endpoint identity joins the
// HASHED model half — an explicitly versioned shape transition.
// Shape hashes change for every bus-carrying program; recorded
// baselines and `.halerec` admissions must be re-recorded once.
// 1.13: `relations.calls_via_stdlib` is INTERPRETED by the model's
// contraction, not by the pre-model walk it used to reproduce.
//
// Both answer "which user fns does a path through stdlib bodies
// connect", and they agree on endpoints; they can disagree on the
// hashed `loop` bit. The old walk kept a set-valued `seen` per
// caller, so a stdlib body first reached on a non-looped path was
// never revisited when a looped path reached it later — the bit
// stayed false. The model's relation revisits on strengthening, so
// the bit is true whenever ANY path is loop-nested. The model's
// answer is the sound one for the question the bit is asked
// (does this carrier repeat per iteration?), and this is the
// versioned transition that adopts it: a program in the
// distinguishing class gets a new `shape_hash` and must be
// re-recorded once. No corpus program is in that class — today's
// stdlib re-emerges into user code only from inside its own loops,
// which sets the bit either way — so this bumps the schema without
// moving a single committed baseline hash.
pub const TOPOLOGY_SCHEMA: &str = "1.13";

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
    dump_topology_parts(bundle)
}

/// The artifact. One authority: every emitted section is a
/// PROJECTION of `ApplicationModel` (GH #476 Change 6 inverted the
/// direction; Change 9 deleted the legacy gathering that had stayed
/// behind as the corpus differential's comparison arm).
///
/// Retained under its Change-6 name because the claim/law pipeline
/// below it is one long function; `dump_topology` is the caller
/// everything else uses.
#[doc(hidden)]
pub fn dump_topology_parts(bundle: &Bundle<'_>) -> String {
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

    // GH #476 Change 9: the legacy relation gathering (calls,
    // publishes, subscribes, labels, unknowns, group rows) lived
    // here and served only the second serialization. Every one of
    // those sections is now projected from `ApplicationModel`,
    // which holds the same facts at finer grain — so the gathering
    // is deleted rather than left running unpublished.
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
    // GH #476 Change 9: the supervision rows the legacy
    // gathering collected here are gone with it — the artifact's
    // supervision section projects from the model's `supervises`
    // relation, which is where the fact lives.

    // ---- labels: declared effect carriers (`is:` tags) ----
    for (k, set) in &summary.carries {
        if !user_key(k) {
            continue;
        }
        let classes =
            crate::frontier::render_effects_named(*set, &effect_names);
        if !classes.is_empty() {
        }
    }

    // ---- claims ----
    // The artifact's law rows are PROJECTED from the canonical path
    // (ClaimIr renders the forms, the Change-5 judgments produce
    // the verdicts, the evidence sidecar carries the certificate
    // results) — and since Change 9 that same judgment is what
    // `hale check` reports, so the document and the checker cannot
    // disagree about a law. Law SELECTION still comes from the
    // claim surface, which is where adoption is settled; this call
    // takes the constitution identities from it and nothing else.
    let identities = crate::claims::constitution_identities(
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
    let (outcomes, projected_lowered, law_rows, law_issues) =
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

    // ---- the hashed model half ----
    //
    // GH #476 Change 9: PROJECTED, full stop. Change 6 inverted
    // the direction (production emits the projection) but kept the
    // legacy gathering here as the corpus differential's
    // comparison arm; that arm was ~190 lines of second
    // serialization of facts the model already holds, and it is
    // deleted with the rest of the duplicate authorities.
    // `tests/topology_projection.rs` now pins artifact identity
    // against a committed baseline instead of against a rival
    // implementation.

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
    // GH #476 Change 9: the UNHASHED tail — sources, provenance,
    // topics — is PROJECTED from the model, and the legacy
    // gathering that used to be rebuilt here for the corpus
    // differential is gone with the rest of the second authority.
    out.push_str(
        &crate::topology_projection::project_unhashed_tail(&vmodel),
    );
    // Round 8: the typed ENDPOINT section — every bus endpoint the
    // model carries, at WIRE-SUBJECT grain, including a declared
    // publisher with no send site (`bus {{ publish "addr" of type
    // T; }}` is a real endpoint the V1 site-grained relations
    // never show). `law.subjects` must equal exactly the subjects
    // this section and the topics section carry — the full model
    // subject universe is validated against its own typed
    // projection, never reverse-engineered from the narrower V1
    // view. Unhashed (endpoint rows are join surface, not shape).
    {
        let subj_pat = |sid: hale_model::SubjectId| -> &str {
            vmodel
                .entities
                .subjects
                .get(sid.index())
                .map(|su| su.pattern.as_str())
                .unwrap_or("")
        };
        // Round 10: every endpoint row carries its TYPED identity
        // — the wire subject AND the declared topic when one
        // covers the end. A literal address whose text collides
        // with a topic display stays a literal (`declared_topic`
        // is the model's syntactic fact, never inferred from
        // strings).
        let topic_field = |t: &Option<hale_model::TopicId>|
         -> String {
            match t {
                Some(tid) => vmodel
                    .entities
                    .topics
                    .get(tid.index())
                    .map(|tp| {
                        format!(
                            ", \"topic\": {}",
                            quote(&tp.display)
                        )
                    })
                    .unwrap_or_default(),
                None => String::new(),
            }
        };
        // Round 11: site rows are LOSSLESS — each carries its
        // owning fn/handler and authored site ordinal, so no two
        // typed rows collapse under the legacy display projection
        // (a topic-covered end and a colliding literal stay
        // distinct facts).
        let fn_disp = |fid: hale_model::FunctionId| -> &str {
            vmodel
                .entities
                .functions
                .get(fid.index())
                .map(|f| f.display.as_str())
                .unwrap_or("")
        };
        // Round 12: every site row is ANCHORED to the typed
        // provenance account — its authored span, which must
        // correspond to the span-grained provenance section rows.
        let ep_loc = |pid: hale_model::ProvenanceId| -> String {
            match vmodel.provenance.records.get(pid.index()) {
                Some(hale_model::Provenance::Source {
                    source,
                    span,
                }) => vmodel
                    .provenance
                    .sources
                    .get(source.index())
                    .map(|su| {
                        format!(
                            ", \"file\": {}, \"span\": [{}, {}]",
                            quote(&su.path),
                            span.0,
                            span.1
                        )
                    })
                    .unwrap_or_default(),
                _ => String::new(),
            }
        };
        let mut rows: Vec<String> = Vec::new();
        for r in &vmodel.relations.publishes {
            rows.push(format!(
                "{{\"verb\": \"publish\", \"subject\": {}, \"via\": \"site\", \"fn\": {}, \"site\": {}{}{}}}",
                quote(subj_pat(r.subject)),
                quote(fn_disp(r.function)),
                r.site,
                topic_field(&r.declared_topic),
                ep_loc(r.provenance)
            ));
        }
        for r in &vmodel.relations.declares_publish {
            let locus = vmodel
                .entities
                .loci
                .get(r.locus.index())
                .map(|l| l.display.as_str())
                .unwrap_or("");
            rows.push(format!(
                "{{\"verb\": \"publish\", \"subject\": {}, \"via\": \"declaration\", \"locus\": {}{}{}}}",
                quote(subj_pat(r.subject)),
                quote(locus),
                topic_field(&r.declared_topic),
                ep_loc(r.provenance)
            ));
        }
        for r in &vmodel.relations.subscribes {
            rows.push(format!(
                "{{\"verb\": \"subscribe\", \"subject\": {}, \"via\": \"declaration\", \"fn\": {}, \"site\": {}{}{}}}",
                quote(subj_pat(r.subject)),
                quote(fn_disp(r.handler)),
                r.site,
                topic_field(&r.declared_topic),
                ep_loc(r.provenance)
            ));
        }
        rows.sort();
        rows.dedup();
        out.push_str(&format!(
            ",\n  \"endpoints\": [{}]",
            rows.join(", ")
        ));
        // Round 9: the DECLARED-publisher relation as its own
        // typed projection (`declares_publish(locus, subject)`) —
        // the anchor for `via: declaration` publish endpoints,
        // which no site-grained relation shows.
        let mut decl_rows: Vec<String> = vmodel
            .relations
            .declares_publish
            .iter()
            .map(|r| {
                let locus = vmodel
                    .entities
                    .loci
                    .get(r.locus.index())
                    .map(|l| l.display.as_str())
                    .unwrap_or("");
                format!(
                    "{{\"locus\": {}, \"subject\": {}{}{}}}",
                    quote(locus),
                    quote(subj_pat(r.subject)),
                    topic_field(&r.declared_topic),
                    ep_loc(r.provenance)
                )
            })
            .collect();
        decl_rows.sort();
        decl_rows.dedup();
        out.push_str(&format!(
            ",\n  \"declares_publish\": [{}]",
            decl_rows.join(", ")
        ));
    }
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
            "    {{\"name\": {}, \"form\": {}, \"result\": {}, \
             \"ordinal\": {}{}}},\n",
            quote(&o.name),
            quote(&demangle_str(&o.form)),
            quote(o.result.as_str()),
            o.ordinal,
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
    // Round 6: every lowered row is KEYED to the typed law it
    // evidences — (law ordinal, certificate ordinal). Certificate
    // rows carry theirs from the projection; the legacy budget /
    // quantitative producers are keyed by their form re-rendered
    // from the typed operands (`ClaimRow::budget_lowered_form`),
    // consumed in table order.
    let mut lowered_keys: Vec<Option<(u32, Option<u32>)>> =
        projected_lowered
            .iter()
            .map(|r| Some((r.ordinal, r.cert)))
            .collect();
    let mut budget_ordinals: std::collections::BTreeMap<
        String,
        std::collections::VecDeque<u32>,
    > = std::collections::BTreeMap::new();
    for row in &law_table.rows {
        if let Some(form) = row.budget_lowered_form() {
            budget_ordinals
                .entry(form)
                .or_default()
                .push_back(row.ordinal);
        }
    }
    let mut push_budget_rows =
        |rows: Vec<crate::effects::LoweredCertificate>,
         lowered: &mut Vec<crate::effects::LoweredCertificate>,
         keys: &mut Vec<Option<(u32, Option<u32>)>>| {
            for r in rows {
                let form = demangle_str(&r.form);
                let key = budget_ordinals
                    .get_mut(&form)
                    .and_then(|q| q.pop_front())
                    .map(|o| (o, None));
                keys.push(key);
                lowered.push(r);
            }
        };
    push_budget_rows(
        crate::budget_check::certificate_rows(
            &programs,
            &bundle.import_renames,
        ),
        &mut lowered,
        &mut lowered_keys,
    );
    let fanout = |subj: &str| -> u64 {
        graph
            .subjects
            .get(subj)
            .map(|si| si.subscribers.len().max(1) as u64)
            .unwrap_or(1)
    };
    push_budget_rows(
        crate::quantitative::certificate_rows(&programs, &fanout),
        &mut lowered,
        &mut lowered_keys,
    );
    // Close the `claims` array before opening `lowered` — omitting
    // this emitted a document no standards-compliant JSON parser
    // accepts, for every shape (no claims, one, many, with or
    // without lowered rows). It survived because the artifact tests
    // asserted on substrings and never parsed the whole document;
    // `topology_artifact_is_valid_json` now does.
    out.push_str("  ],\n  \"lowered\": [\n");
    for (r, key) in lowered.iter().zip(lowered_keys.iter()) {
        let keyed = match key {
            Some((o, Some(c))) => {
                format!(", \"ordinal\": {}, \"cert\": {}", o, c)
            }
            Some((o, None)) => format!(", \"ordinal\": {}", o),
            None => String::new(),
        };
        out.push_str(&format!(
            "    {{\"subject\": {}, \"form\": {}, \"result\": {}{}}},\n",
            quote(&demangle_str(&r.subject)),
            quote(&demangle_str(&r.form)),
            quote(r.result.as_str()),
            keyed
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
    // Round 7: `law_digest` is a RECOMPUTABLE canonical fingerprint
    // over the serialized law rows — the rows text parsed and
    // re-serialized through serde_json (BTreeMap keys, compact
    // separators), then fnv1a64 — so a consumer recomputes it from
    // the parsed document, and editing a row while keeping the
    // stale digest refuses. (The ClaimIrTable's internal semantic
    // digest remains the derive-time evidence tie; this field is
    // the EXTERNAL contract.)
    let law_rows_text = {
        let mut rows_out = String::from("[\n");
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
        let diag_list = |ds: &[(
            String,
            Option<(String, u32, u32)>,
        )]|
         -> String {
            let items: Vec<String> = ds
                .iter()
                .map(|(msg, at)| {
                    let at = match at {
                        Some((file, a, b)) => format!(
                            ", \"file\": {}, \"span\": [{}, {}]",
                            quote(file),
                            a,
                            b
                        ),
                        None => String::new(),
                    };
                    format!(
                        "{{\"message\": {}{}}}",
                        quote(&demangle_str(msg)),
                        at
                    )
                })
                .collect();
            format!("[{}]", items.join(", "))
        };
        let certs = if r.certs.is_empty() {
            String::new()
        } else {
            let cs: Vec<String> = r
                .certs
                .iter()
                .map(|(i, form, res, diags)| {
                    let ev = if diags.is_empty() {
                        String::new()
                    } else {
                        format!(
                            ", \"evidence\": {}",
                            diag_list(diags)
                        )
                    };
                    format!(
                        "{{\"ordinal\": {}, \"form\": {}, \
                         \"result\": {}{}}}",
                        i,
                        quote(&demangle_str(form)),
                        quote(res.as_str()),
                        ev
                    )
                })
                .collect();
            format!(", \"certs\": [{}]", cs.join(", "))
        };
        let evidence = if r.evidence.is_empty() {
            String::new()
        } else {
            format!(
                ", \"evidence\": {}",
                diag_list(&r.evidence)
            )
        };
        // The row's own RENDERED FORM. Admission re-renders it from
        // the typed payload, so editing an operand orphans it — the
        // defense that already covers claims-block rows through the
        // compatibility `claims` section, extended to the
        // annotation-origin families that have no entry there
        // (GH #476 Change 5f: this is what `law.legacy` was
        // providing for rows that imported an outside verdict, and
        // it must not disappear when a family stops importing one).
        let form = match law_table
            .rows
            .iter()
            .find(|lr| lr.ordinal == r.ordinal)
            .and_then(|lr| lr.legacy_form())
        {
            Some(f) => {
                format!(", \"form\": {}", quote(&demangle_str(&f)))
            }
            None => String::new(),
        };
        rows_out.push_str(&format!(
            "      {{\"ordinal\": {}, \"name\": {}, \"origin\": {}, \
             \"family\": {}, \"verdict\": {}, \"law\": {}{}{}{}{}}},\n",
            r.ordinal,
            quote(&demangle_str(&r.name)),
            quote(&r.origin),
            quote(r.family.as_str()),
            quote(r.verdict.as_str()),
            // Verbatim: the payload carries RAW (canonical) and
            // DISPLAY spellings side by side — demangling the
            // whole object would collapse the raw identity.
            r.law,
            form,
            certs,
            evidence,
            prov
        ));
    }
        trim_trailing_comma(&mut rows_out);
        rows_out.push_str("    ]");
        rows_out
    };
    // Round 9: the law-selection ISSUES participate in the digest
    // and the document verdict — no claim error disappears between
    // checking and projection.
    let law_issues_text = {
        let items: Vec<String> = law_issues
            .iter()
            .map(|(msg, at)| {
                let loc = match at {
                    Some((f, a, b)) => format!(
                        ", \"file\": {}, \"span\": [{}, {}]",
                        quote(f),
                        a,
                        b
                    ),
                    None => String::new(),
                };
                format!(
                    "{{\"message\": {}{}}}",
                    quote(&demangle_str(msg)),
                    loc
                )
            })
            .collect();
        format!("[{}]", items.join(", "))
    };
    let law_digest = {
        let rows: serde_json::Value =
            serde_json::from_str(&law_rows_text)
                .expect("the emitter's own law rows parse");
        let issues: serde_json::Value =
            serde_json::from_str(&law_issues_text)
                .expect("the emitter's own issues parse");
        fnv1a64(
            serde_json::to_string(&serde_json::json!({
                "issues": issues,
                "rows": rows,
            }))
            .expect("canonical serialization")
            .as_bytes(),
        )
    };
    out.push_str(",\n  \"law\": {\n");
    out.push_str(&format!(
        "    \"law_digest\": \"{:016x}\",\n",
        law_digest
    ));
    out.push_str(&format!(
        "    \"inputs_digest\": \"{:016x}\",\n",
        law_evidence.inputs_digest
    ));
    // Round 5/6: the FULL law-subject catalogs — annotation
    // subjects resolve against the whole model function table
    // (module fns included), wider than the legacy `sorts.fns`
    // summary universe. Round 6: every catalog entry is a
    // CANONICAL PAIR `{name: raw, display}` — the raw half is the
    // machine join key, and a resolved reference must match one
    // exact pair (cross-row consistency alone cannot anchor a
    // singleton reference).
    {
        let pair_catalog = |label: &str,
                            mut pairs: Vec<(String, String)>|
         -> String {
            pairs.sort();
            pairs.dedup();
            format!(
                "    \"{}\": [{}],\n",
                label,
                pairs
                    .iter()
                    .map(|(n, d)| format!(
                        "{{\"name\": {}, \"display\": {}}}",
                        quote(n),
                        quote(d)
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        // Round 10: function-grain ANALYSIS COVERAGE — the model's
        // `analyzed` bit per function (false for module-scoped
        // bodies and `on_failure` handlers). The analyzed subset
        // must equal `sorts.fns` exactly (the hashed summary
        // universe), which is what anchors the coverage account.
        {
            let mut rows: Vec<String> = vmodel
                .entities
                .functions
                .iter()
                .map(|f| {
                    let kind = match f.kind {
                        hale_model::FunctionKind::Hook => "hook",
                        hale_model::FunctionKind::Method => {
                            "method"
                        }
                        hale_model::FunctionKind::Free => "free",
                        hale_model::FunctionKind::Mode => "mode",
                        hale_model::FunctionKind::FailureHandler => {
                            "failure"
                        }
                    };
                    let owner = match f.owner {
                        Some(l) => vmodel
                            .entities
                            .loci
                            .get(l.index())
                            .map(|l| {
                                format!(
                                    ", \"owner\": {}",
                                    quote(&l.display)
                                )
                            })
                            .unwrap_or_default(),
                        None => String::new(),
                    };
                    format!(
                        "{{\"name\": {}, \"display\": {}, \"analyzed\": {}, \"summarized\": {}, \"kind\": {}{}}}",
                        quote(&f.name),
                        quote(&f.display),
                        f.analyzed,
                        f.summarized,
                        quote(kind),
                        owner
                    )
                })
                .collect();
            rows.sort();
            rows.dedup();
            out.push_str(&format!(
                "    \"fn_universe\": [{}],\n",
                rows.join(", ")
            ));
        }
        // Loci carry an ANALYZABLE flag: the legacy certificate
        // engines walk only top-level loci, so a module-scoped
        // locus's phase contracts have no engine report and judge
        // `uncertified` — a consumer needs the discriminator to
        // hold the two shapes to their exact verdicts. Round 9:
        // the fact is the MODEL's (`LocusDecl::analyzable`) — one
        // authority shared with the evidence layer; this
        // projection never re-walks source.
        {
            let mut rows: Vec<String> = vmodel
                .entities
                .loci
                .iter()
                .map(|l| {
                    format!(
                        "{{\"name\": {}, \"display\": {}, \"analyzable\": {}}}",
                        quote(&l.name),
                        quote(&l.display),
                        l.analyzable
                    )
                })
                .collect();
            rows.sort();
            rows.dedup();
            out.push_str(&format!(
                "    \"loci\": [{}],\n",
                rows.join(", ")
            ));
        }
        out.push_str(&pair_catalog(
            "groups",
            vmodel
                .entities
                .groups
                .iter()
                .map(|g| (g.name.clone(), g.display.clone()))
                .collect(),
        ));
        // Topics carry their wire subject too — the selector
        // binding recomputes candidate sets from these rows with
        // the SAME matching rule the lowering used
        // (`hale_model::bus_ref_matches`).
        let mut topic_rows: Vec<String> = vmodel
            .entities
            .topics
            .iter()
            .map(|t| {
                let subject = vmodel
                    .entities
                    .subjects
                    .get(t.subject.index())
                    .map(|su| su.pattern.as_str())
                    .unwrap_or("");
                format!(
                    "{{\"name\": {}, \"display\": {}, \"subject\": {}}}",
                    quote(&t.name),
                    quote(&t.display),
                    quote(subject)
                )
            })
            .collect();
        topic_rows.sort();
        topic_rows.dedup();
        out.push_str(&format!(
            "    \"topics\": [{}],\n",
            topic_rows.join(", ")
        ));
        // The wire-subject universe — selector SUBJECT candidates
        // resolve against the model's subject table, which is
        // wider than the declared topics (raw publish subjects).
        let mut subject_rows: Vec<&str> = vmodel
            .entities
            .subjects
            .iter()
            .map(|su| su.pattern.as_str())
            .collect();
        subject_rows.sort_unstable();
        subject_rows.dedup();
        out.push_str(&format!(
            "    \"subjects\": [{}],\n",
            subject_rows
                .iter()
                .map(|s| quote(s))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    // …and the effect-class catalog: which user classes are
    // DECLARED and which are cyclic — the facts a consumer needs
    // to independently justify an `invalid` certificate verdict.
    {
        let mut rows: Vec<String> = vmodel
            .entities
            .effect_classes
            .iter()
            .map(|c| {
                format!(
                    "{{\"name\": {}, \"declared\": {}, \"cyclic\": {}}}",
                    quote(&c.name),
                    c.declared,
                    matches!(
                        c.definition,
                        hale_model::EffectClassDefinition::InvalidCycle
                    )
                )
            })
            .collect();
        rows.sort();
        out.push_str(&format!(
            "    \"effect_classes\": [{}],\n",
            rows.join(", ")
        ));
    }
    // Round 6: the typed LEGACY report — the old engines'
    // verdicts for the unmigrated non-budget families (`causes:` /
    // `depends:`), keyed by law ordinal and by the form
    // FINGERPRINT re-rendered from the typed operands. Admission
    // re-renders the fingerprint from the decoded payload: an
    // operand mutation orphans the report entry, so a bare
    // imported verdict cannot survive a payload edit.
    {
        let mut entries: Vec<String> = Vec::new();
        for row in &law_table.rows {
            let Some(form) = row.legacy_form() else { continue };
            let Some(vd) = legacy_unmigrated.get(&row.ordinal)
            else {
                continue;
            };
            entries.push(format!(
                "{{\"ordinal\": {}, \"form\": {}, \"result\": {}}}",
                row.ordinal,
                quote(&demangle_str(&form)),
                quote(vd.as_str())
            ));
        }
        out.push_str(&format!(
            "    \"legacy\": [{}],\n",
            entries.join(", ")
        ));
    }
    out.push_str(&format!(
        "    \"issues\": {},\n",
        law_issues_text
    ));
    out.push_str("    \"rows\": ");
    out.push_str(&law_rows_text);
    out.push_str("\n  }");

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
        && law_pass
        && law_issues.is_empty();
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
    out
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
    // Round 4 (#490): located STRUCTURALLY — the digest is the
    // TOP-LEVEL entry, and the covered body is everything before
    // the comma that terminates the preceding top-level entry
    // (byte-identical to what the emitter hashed). A nested
    // `artifact_digest` inside some section is data, never the
    // verified value.
    let top = scan_top_level(artifact).ok()?;
    let at = top
        .iter()
        .position(|(k, _, _)| k == "artifact_digest")?;
    let body_end = top.get(at.checked_sub(1)?)?.2;
    let body = &artifact[..body_end];
    let entry = &artifact[top[at].1..top[at].2];
    let claimed = entry.split('"').nth(3)?;
    Some(claimed == format!("{:016x}", fnv1a64(body.as_bytes())))
}

/// One RAW STRUCTURAL scan of an artifact document (GH #476
/// Change 7, rounds 3–4). The raw admission pass and the parsed
/// consumption must never disagree, which forces two properties
/// the earlier textual helpers lacked:
///
///  * duplicate object keys are detected on their DECODED names —
///    `"shape\u005fhash"` and `"shape_hash"` are one parsed key,
///    so they are one scanner key (serde's last-wins map would
///    otherwise consume a value the raw pass never verified);
///  * the identity fields are located STRUCTURALLY at the top
///    level — a nested `shape_hash` (or `artifact_digest`) inside
///    some other section is data, never the value the verifiers
///    check.
///
/// Returns the top-level entries in document order:
/// (decoded key, key position, terminator position — the byte
/// index of the `,` or `}` that ends the entry). `Err` names the
/// first duplicated key (any depth) or the malformation.
pub fn scan_top_level(
    doc: &str,
) -> Result<Vec<(String, usize, usize)>, String> {
    fn unescape(raw: &str) -> Option<String> {
        let mut out = String::with_capacity(raw.len());
        let mut it = raw.chars();
        while let Some(c) = it.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match it.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    let mut cp = 0u32;
                    for _ in 0..4 {
                        cp = cp * 16
                            + it.next()?.to_digit(16)?;
                    }
                    if (0xD800..0xDC00).contains(&cp) {
                        // Surrogate pair.
                        if it.next()? != '\\'
                            || it.next()? != 'u'
                        {
                            return None;
                        }
                        let mut lo = 0u32;
                        for _ in 0..4 {
                            lo = lo * 16
                                + it.next()?.to_digit(16)?;
                        }
                        if !(0xDC00..0xE000).contains(&lo) {
                            return None;
                        }
                        cp = 0x10000
                            + ((cp - 0xD800) << 10)
                            + (lo - 0xDC00);
                    }
                    out.push(char::from_u32(cp)?);
                }
                _ => return None,
            }
        }
        Some(out)
    }
    enum Scope {
        Object {
            keys: std::collections::BTreeSet<String>,
            expect_key: bool,
        },
        Array,
    }
    let mut stack: Vec<Scope> = Vec::new();
    let mut top: Vec<(String, usize, usize)> = Vec::new();
    let mut open_entry: Option<(String, usize)> = None;
    let mut chars = doc.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '{' => stack.push(Scope::Object {
                keys: std::collections::BTreeSet::new(),
                expect_key: true,
            }),
            '[' => stack.push(Scope::Array),
            '}' | ']' => {
                if stack.len() == 1 {
                    if let Some((k, pos)) = open_entry.take() {
                        top.push((k, pos, i));
                    }
                }
                stack.pop();
            }
            ',' => {
                if stack.len() == 1 {
                    if let Some((k, pos)) = open_entry.take() {
                        top.push((k, pos, i));
                    }
                }
                if let Some(Scope::Object {
                    expect_key, ..
                }) = stack.last_mut()
                {
                    *expect_key = true;
                }
            }
            '"' => {
                let start = i + 1;
                let mut end = start;
                let mut esc = false;
                for (j, sc) in chars.by_ref() {
                    if esc {
                        esc = false;
                        continue;
                    }
                    match sc {
                        '\\' => esc = true,
                        '"' => {
                            end = j;
                            break;
                        }
                        _ => {}
                    }
                }
                let at_top = stack.len() == 1;
                if let Some(Scope::Object {
                    keys,
                    expect_key,
                }) = stack.last_mut()
                {
                    if *expect_key {
                        *expect_key = false;
                        let key = unescape(&doc[start..end])
                            .ok_or_else(|| {
                                format!(
                                    "malformed string escape in \
                                     key at byte {}",
                                    start
                                )
                            })?;
                        if !keys.insert(key.clone()) {
                            return Err(format!(
                                "duplicate object key `{}`",
                                key
                            ));
                        }
                        if at_top {
                            open_entry = Some((key, i));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(top)
}

/// The canonical TOP-LEVEL key sequence the emitter writes
/// (GH #476 Change 7, round 5). Order is not JSON pedantry here:
/// it DEFINES the verified hash ranges — `shape_hash` covers the
/// bytes between its entry and `sources`, and `artifact_digest`
/// covers everything before itself — so a document that reorders
/// known keys (or introduces unknown ones) moves modeled facts in
/// or out of the verified ranges and is refused.
const CANONICAL_TOP_LEVEL: &[&str] = &[
    "schema",
    "semantics",
    "shape_hash",
    "sorts",
    "sealed",
    "relations",
    "groups",
    "labels",
    "phases",
    "seeds",
    "effects",
    "supervision",
    "unknowns",
    "endpoint_identity",
    "sources",
    "provenance",
    "topics",
    "endpoints",
    "declares_publish",
    "claims",
    "lowered",
    "law",
    "capabilities",
    "adequacy",
    "evaluation",
    "verdict",
    "artifact_digest",
];

/// Enforce the canonical top-level layout on a scanned document:
/// every key known, keys in canonical relative order (absences
/// allowed — a bus-free program has no `endpoint_identity`), and
/// `artifact_digest` the FINAL entry. Run at every consumption
/// boundary right after [`scan_top_level`].
pub fn verify_top_level_order(
    top: &[(String, usize, usize)],
) -> Result<(), String> {
    let mut cursor = 0usize;
    for (k, _, _) in top {
        let Some(pos) =
            CANONICAL_TOP_LEVEL.iter().position(|c| c == k)
        else {
            return Err(format!(
                "unknown top-level key `{}`",
                k
            ));
        };
        if pos < cursor {
            return Err(format!(
                "top-level key `{}` out of canonical order — \
                 order defines the verified hash ranges",
                k
            ));
        }
        cursor = pos + 1;
    }
    match top.last() {
        Some((k, _, _)) if k == "artifact_digest" => Ok(()),
        _ => Err(
            "artifact_digest must be the final top-level entry"
                .to_string(),
        ),
    }
}

/// Recompute the MODEL-HALF hash from the raw artifact text and
/// compare it with the declared `shape_hash` (GH #476 Change 7).
/// Rounds 2 + 4: the field is located STRUCTURALLY at the top
/// level (a nested `shape_hash` is never the checked value), and
/// the model half is exactly the bytes between the top-level
/// `shape_hash` entry and the top-level `sources` entry — the
/// substring the emitter hashed. `None` = the document has no
/// top-level shape_hash / sources to check.
pub fn verify_shape_hash(artifact: &str) -> Option<bool> {
    let top = scan_top_level(artifact).ok()?;
    let sh = top.iter().find(|(k, _, _)| k == "shape_hash")?;
    let sources_at = top
        .iter()
        .position(|(k, _, _)| k == "sources")?;
    // The model half runs from just after the shape_hash entry's
    // terminator (`,` + newline) to the terminator of the entry
    // preceding `sources` — the comma the tail's `,\n  "sources"`
    // begins with.
    let model_start = sh.2 + ",\n".len();
    let model_end = top.get(sources_at.checked_sub(1)?)?.2;
    if model_end <= model_start {
        return None;
    }
    let model = &artifact[model_start..model_end];
    // The declared value: the quoted string after the key.
    let val = artifact[sh.1..sh.2]
        .split('"')
        .nth(3)?;
    Some(val == format!("{:016x}", fnv1a64(model.as_bytes())))
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
