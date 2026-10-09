//! A classified correction, pinned (F.40 phase 3, E3a 6 of 9).
//!
//! `--dump-alloc-summary`, the unbounded-allocation advisory (the
//! check's warnings and the editor's) and the editor's
//! `hale/allocSummary` used to build their own summary with
//! `summarize_programs`: the checked programs alone, with no stdlib
//! bodies beside them and the import renames ignored. A call into an
//! imported seed or a stdlib body was an unresolved edge the walk did
//! not follow, and a stdlib loop that reaches a user implementation of a
//! stdlib interface (the interface fan-out, #533) never made that
//! implementation invoked unboundedly: a fail-open, the advisory missing
//! allocations that accumulate. All three now read the snapshot's
//! summary (`Snapshot::demand_alloc_summary`): the program's own rows,
//! judged over the stdlib's analysis copy and the renames.
//!
//! The old answer is the same bundle summarized alone, with the stdlib
//! bodies and the renames cleared, which is what the old summary saw.
//! Per target the tests pin the leak sites the advisory adds (none is
//! removed), the fns of the program's own now invoked unboundedly (none
//! stops being), how each changed site verdict moved (every move is
//! toward accumulation), and the number of call lines that now name
//! what they call. `no_other_target_changes` checks the other targets
//! of the corpus, `tests/hale` and the DNA seeds are untouched.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;
use hale_syntax::ast::Program;
use hale_types::alloc_summary::{advisory_leak_sites, summarize_identified, AllocSummary, LeakSite};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// Every target `hale check` is run over here: the corpus fixtures, the
/// `tests/hale` programs and the DNA seeds, by their path from the root.
fn targets() -> Vec<String> {
    let root = root();
    let mut out = Vec::new();
    let mut push_dir = |dir: &str, keep: &dyn Fn(&Path) -> bool| {
        for e in std::fs::read_dir(root.join(dir)).unwrap() {
            let p = e.unwrap().path();
            if keep(&p) {
                out.push(p.strip_prefix(&root).unwrap().to_string_lossy().to_string());
            }
        }
    };
    push_dir("crates/hale-codegen/tests/fixtures/examples", &|p| p.is_dir());
    push_dir("tests/hale", &|p| p.to_string_lossy().ends_with("_test.hl"));
    push_dir("dna", &|p| {
        p.is_dir()
            && !p.ends_with("tests")
            && std::fs::read_dir(p).unwrap().any(|f| f.unwrap().path().extension().is_some_and(|x| x == "hl"))
    });
    out.sort();
    out
}

/// What one target's correction changed.
#[derive(Debug, Default, PartialEq)]
struct Change {
    /// How many call lines now name what they call (a stdlib body, an
    /// imported seed's fn, a stdlib conformer of an interface).
    calls: usize,
    /// The program's own fns now invoked unboundedly.
    invoked: Vec<String>,
    /// How the changed site verdicts moved, `old -> new`, with how many.
    verdicts: BTreeMap<String, usize>,
    /// Leak sites the advisory adds, as `owner kind @start..end reason`.
    added: Vec<String>,
}

impl Change {
    fn is_empty(&self) -> bool {
        *self == Change::default()
    }
}

fn site(ls: &LeakSite) -> String {
    format!("{} {:?} @{}..{} {:?}", ls.owner.display(), ls.kind, ls.span.start.0, ls.span.end.0, ls.reason)
}

/// A dump's blocks: each `fn` (or `locus`, or header) line with its
/// non-call lines and its call lines.
fn blocks(dump: &str) -> Vec<(String, Vec<String>, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();
    for l in dump.lines() {
        if l.starts_with("fn ") || l.starts_with("locus ") || l.starts_with('#') || out.is_empty() {
            out.push((l.to_string(), Vec::new(), Vec::new()));
        } else if l.trim_start().starts_with("call ") {
            out.last_mut().unwrap().2.push(l.trim().to_string());
        } else {
            out.last_mut().unwrap().1.push(l.trim().to_string());
        }
    }
    out
}

/// A site line's verdict column.
fn verdict(line: &str) -> &str {
    ["once-per-invocation", "per-iteration-reclaim", "ACCUMULATES-UNBOUNDED"]
        .into_iter()
        .find(|v| line.contains(v))
        .unwrap_or_else(|| panic!("a site line with no verdict: {line}"))
}

/// `summary` with the reclaim boundary's correction (E3b) cleared: no
/// fn's frame classified, and no site reclaimed at its fn's return.
fn without_frames(summary: &AllocSummary) -> AllocSummary {
    let mut s = summary.clone();
    for f in s.fns.values_mut() {
        f.frame = None;
        for site in &mut f.sites {
            if site.reclaim == hale_types::alloc_summary::ReclaimScope::FnReturn {
                site.reclaim = hale_types::alloc_summary::ReclaimScope::EnclosingLocus;
            }
        }
    }
    s
}

/// The target loaded as `hale check <target>` loads it, the snapshot's
/// summary beside the old one. On a thread of its own: a whole DNA
/// seed's walk is deep.
fn change(target: &str) -> Change {
    let path = root().join(target);
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn_scoped(s, move || {
                let config = Config::check(path.is_dir(), false);
                let Ok(snap) = Snapshot::load(&path, LoadMode::WholeSeed, &Disk, config) else {
                    return Change::default();
                };
                let Ok(now) = snap.demand_alloc_summary() else { return Change::default() };
                let bundle = snap.bundle();
                let programs: Vec<&Program> = bundle.programs.values().copied().collect();
                // The old answer: the stdlib bodies and the renames cleared.
                let identified: Vec<(&Program, &hale_types::snapshot::Snapshot)> =
                    programs.iter().map(|p| (*p, &bundle.snapshot)).collect();
                let before: AllocSummary = without_frames(&summarize_identified(&identified, &[]));
                // This correction alone: the reclaim boundary's (E3b)
                // cleared on both sides.
                let now = &without_frames(now);
                let mut c = Change::default();
                let (old, new) = (blocks(&before.render()), blocks(&now.render()));
                assert_eq!(old.len(), new.len(), "{target}: the dump lists the same fns and loci");
                for ((ho, so, co), (hn, sn, cn)) in old.iter().zip(&new) {
                    let name = |h: &str| h.split_whitespace().nth(1).unwrap_or("").to_string();
                    assert_eq!(name(ho), name(hn), "{target}: the dump lists the same fns and loci");
                    if ho.starts_with("fn ") && ho != hn {
                        assert!(
                            !ho.contains("invoked-unboundedly") && hn.contains("invoked-unboundedly"),
                            "{target}: {ho} -> {hn}"
                        );
                        c.invoked.push(name(hn));
                    }
                    assert_eq!(so.len(), sn.len(), "{target}: {ho}'s sites and loops");
                    for (o, n) in so.iter().zip(sn) {
                        if o != n {
                            *c.verdicts.entry(format!("{} -> {}", verdict(o), verdict(n))).or_default() += 1;
                        }
                    }
                    let mut rest = co.clone();
                    for l in cn {
                        match rest.iter().position(|x| x == l) {
                            Some(i) => {
                                rest.remove(i);
                            }
                            None => c.calls += 1,
                        }
                    }
                }
                let sites = |summary: &AllocSummary| -> BTreeSet<String> {
                    advisory_leak_sites(summary, &programs, &bundle.snapshot, &bundle.sources).iter().map(site).collect()
                };
                let (was, is) = (sites(&before), sites(now));
                assert!(was.is_subset(&is), "{target}: the advisory lost a site");
                c.added = is.difference(&was).cloned().collect();
                c
            })
            .unwrap()
            .join()
            .unwrap()
    })
}

/// The 31 targets below are the only ones the correction changes.
#[test]
fn no_other_target_changes() {
    let corrected: Vec<String> = targets().into_iter().filter(|t| !change(t).is_empty()).collect();
    let mut expected: Vec<&str> = CALL_LINES_ONLY.iter().map(|(t, _)| *t).chain(VERDICT_TARGETS.iter().copied()).collect();
    expected.sort();
    assert_eq!(corrected, expected);
}

/// The targets whose dump only names more of what it calls: no verdict,
/// no fn's tag and no advisory site moves.
const CALL_LINES_ONLY: [(&str, usize); 19] = [
    ("dna/core", 1271),
    ("dna/oidc", 34),
    ("dna/organism", 1338),
    ("dna/organization_source", 1311),
    ("dna/reflexes", 68),
    ("tests/hale/api_big_reply_test.hl", 2),
    ("tests/hale/api_bearer_roles_test.hl", 2),
    ("tests/hale/api_binding_test.hl", 2),
    ("tests/hale/api_context_test.hl", 2),
    ("tests/hale/api_roles_test.hl", 2),
    ("tests/hale/api_roles_xseed_test.hl", 2),
    ("tests/hale/api_serve_test.hl", 2),
    // 4: the new test (S6) calls the writers and the readers, whose
    // stdlib bodies only the renames name.
    ("tests/hale/bytes_writer_offset_test.hl", 4),
    // 2: P3 2 of 3's classified correction resolves `let f = lib::add3;
    // f()` to the fn the local names, which only the renames reach; 3
    // more: E5 resolves the calls through a function value to the
    // program's function values, one of them an imported seed's, which
    // only the renames name.
    ("tests/hale/imported_fn_value_test.hl", 5),
    ("tests/hale/json_unicode_escapes_test.hl", 16),
    ("tests/hale/json_valid_test.hl", 27),
    ("tests/hale/log_fields_test.hl", 6),
    ("tests/hale/secret_write_private_test.hl", 10),
    ("tests/hale/std_secret_test.hl", 8),
];

/// The targets whose verdicts move, each pinned in `verdict_changes`.
const VERDICT_TARGETS: [&str; 11] = [
    "crates/hale-codegen/tests/fixtures/examples/69-http-router",
    "crates/hale-codegen/tests/fixtures/examples/92-build-an-api",
    "crates/hale-codegen/tests/fixtures/examples/docs-server",
    "crates/hale-codegen/tests/fixtures/examples/http-hello",
    "dna/api",
    "dna/host",
    "dna/operations",
    "dna/organization_runtime",
    "dna/ui",
    "tests/hale/api_binding_run_test.hl",
    "tests/hale/router_middleware_test.hl",
];

#[test]
fn call_lines_only() {
    for (t, calls) in CALL_LINES_ONLY {
        assert_eq!(change(t), Change { calls, ..Change::default() }, "{t}");
    }
}

fn pinned(target: &str, calls: usize, invoked: &[&str], verdicts: &[(&str, usize)], added: &[&str]) {
    let expected = Change {
        calls,
        invoked: invoked.iter().map(|s| s.to_string()).collect(),
        verdicts: verdicts.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        added: added.iter().map(|s| s.to_string()).collect(),
    };
    assert_eq!(
        change(target),
        expected,
        "{target}: what the correction changes is pinned; a difference here is a change to what the \
         dump or the advisory reports, and is classified before the pin moves"
    );
}

/// Per target with a verdict change: the call lines that now resolve,
/// the program's own fns now invoked unboundedly, how the site verdicts
/// moved, and the leak sites the advisory adds.
#[test]
fn verdict_changes() {
    pinned(
        "crates/hale-codegen/tests/fixtures/examples/69-http-router",
        8,
        &[
            "Count::handle",
            "Hello::handle",
            "Stamp::after",
            "Stamp::before",
        ],
        &[
            ("once-per-invocation -> per-iteration-reclaim", 5),
        ],
        &[
        ],
    );
    // F.40 E5: the stdlib listener's connection loop calls its
    // `on_conn` parameter, which resolves to the program's handler (the
    // one function value of its type), so the handler and what it calls
    // are invoked unboundedly; the old summary saw neither the loop nor
    // the call.
    pinned(
        "crates/hale-codegen/tests/fixtures/examples/docs-server",
        1,
        &[
            "__docs_dir",
            "__render_doc",
            "__render_index",
            "__safe_path",
            "__strip_leading_slash",
            "__wrap_html_page",
            "handle_request",
        ],
        &[
            ("once-per-invocation -> ACCUMULATES-UNBOUNDED", 1),
            ("per-iteration-reclaim -> ACCUMULATES-UNBOUNDED", 5),
        ],
        &[
        ],
    );
    pinned(
        "crates/hale-codegen/tests/fixtures/examples/http-hello",
        1,
        &[
            "handle_request",
        ],
        &[
        ],
        &[
        ],
    );
    pinned(
        "crates/hale-codegen/tests/fixtures/examples/92-build-an-api",
        0,
        &[
            "Tokens::principal",
            "Tokens::refused",
        ],
        &[
            ("once-per-invocation -> per-iteration-reclaim", 2),
        ],
        &[
        ],
    );
    pinned(
        "dna/api",
        3039,
        &[
            "__lib_dna__core___hat__effect_class_ok",
            "__lib_dna__core___hat__hat_effects_of",
            "__lib_dna__core___hat__hat_json",
            "__lib_dna__core___knowledge__url_encode",
            "__lib_dna__core___ownership__approver_owner",
            "__lib_dna__core___principal__audience_names",
            "__lib_dna__core___principal__b64url_text",
            "__lib_dna__core___principal__bearer_refusal",
            "__lib_dna__core___principal__bearer_token",
            "__lib_dna__core___principal__claims_refusal",
            "__lib_dna__core___principal__cookie_value",
            "__lib_dna__core___principal__discovery_refusal",
            "__lib_dna__core___principal__endpoint_allowed",
            "__lib_dna__core___principal__es256_refusal",
            "__lib_dna__core___principal__id_token_claims",
            "__lib_dna__core___principal__issuer_loopback",
            "__lib_dna__core___principal__jwks_keys",
            "__lib_dna__core___principal__spki_pem",
            "__lib_dna__core___principal__url_parts",
            "__lib_dna__core___record__line_at",
            "__lib_dna__core___record__row_admissible",
            "__lib_dna__core___roles__record_trust_local",
            "__lib_dna__core___route__show_list",
            "__lib_dna__core___topics__pressure_what",
            "__lib_dna__core___workflow_definition__baseline_definitions",
            "__lib_dna__core___workflow_definition__define_baseline",
            "__lib_dna__core___workflow_definition__organism_leaf",
            "__lib_dna__operations___context__hat_json",
            "__lib_dna__operations___usage__usage_json",
            "__lib_dna__operations___usage__usage_lines_json",
            "__lib_dna__ui___main__clean",
            "__lib_dna__ui___main__hale_bin",
            "__lib_dna__ui___main__page",
            "__lib_dna__ui___main__redirect",
            "__lib_dna__ui___main__run_dna",
            "__lib_dna__ui___main__run_failed",
            "__lib_dna__ui___main__text",
            "__lib_dna__ui___main__token_failed",
            "command_guard",
            "commands_port",
            "definition_drafts_over",
            "definitions_over",
            "exchange",
            "graph_read",
            "listed_commands",
            "query_token",
            "relay_command",
            "relay_malformed",
            "relay_upstream",
            "socket_live",
            "token_matches",
            "upstream_header",
            "Api::definition_draft_handle",
            "Api::definitions_read",
            "Api::handle",
            "Api::knowledge_read",
            "Api::organization_read",
            "Api::review_candidate_read",
            "Api::unauthenticated",
            "CommandWire::error",
            "CommandWire::fields",
            "CommandWire::guard",
            "CommandWire::header",
            "CommandWire::origin_valid",
            "CommandWire::raw",
            "CommandWire::recipients_valid",
            "CommandWire::text",
            "ContextWire::read",
            "DefinitionDraftWire::capability",
            "DefinitionDraftWire::data",
            "DefinitionDraftWire::error",
            "DefinitionDraftWire::guard",
            "DefinitionDraftWire::parse",
            "GraphHttp::basis",
            "GraphHttp::cursor",
            "GraphHttp::error",
            "GraphHttp::fields",
            "GraphHttp::plain",
            "GraphHttp::query",
            "GraphHttp::record_budget",
            "GraphHttp::request",
            "GraphHttp::response",
            "GraphHttp::text",
            "GraphHttp::unhex",
            "GraphItems::result",
            "Http::error",
            "Http::page",
            "Http::query",
            "Http::query_fields",
            "Http::response",
            "Http::success",
            "Http::task_query",
            "Http::unreadable",
            "KnowledgeWire::basis",
            "KnowledgeWire::basis_snapshot",
            "KnowledgeWire::failure",
            "KnowledgeWire::query",
            "KnowledgeWire::request_json",
            "KnowledgeWire::response",
            "KnowledgeWire::source_token",
            "KnowledgeWire::unavailable",
            "LaunchBearer::principal",
            "LaunchBearer::refused",
            "LocalKnowledge::ownership",
            "LocalKnowledge::read",
            "LocalKnowledge::supported",
            "NoKnowledge::ownership",
            "NoKnowledge::read",
            "NoKnowledge::supported",
            "NoOrganizationRuntime::current",
            "NoPeople::person",
            "NoPeople::present",
            "OidcBearer::principal",
            "OidcBearer::refused",
            "OrganizationCandidateWire::document",
            "OrganizationCandidateWire::read",
            "OrganizationDraftWire::base",
            "OrganizationDraftWire::capability",
            "OrganizationDraftWire::handle",
            "OrganizationDraftWire::project",
            "OrganizationDraftWire::same_base",
            "OrganizationImpactWire::candidate_visible",
            "OrganizationImpactWire::document",
            "OrganizationImpactWire::read",
            "OrganizationStatusWire::current",
            "OrganizationStatusWire::document",
            "OrganizationStatusWire::fact",
            "OrganizationStatusWire::matches",
            "OrganizationStatusWire::project",
            "OrganizationStatusWire::read",
            "OrganizationStatusWire::stages",
            "OrganizationStatusWire::trust",
            "PersonWire::read",
            "TaskWire::read",
            "TaskWire::unavailable",
            "WebAssets::response",
            "WebAssets::serves",
            "Wire::applications",
            "Wire::capabilities",
            "WorkflowRead::count",
            "WorkflowRead::fail",
            "WorkflowRead::load",
            "WorkflowWire::read",
            "__lib_dna__core___memory_store__GraphRules::projection_valid",
            "__lib_dna__core___memory_store__GraphRules::validate",
            "__lib_dna__core___memory_store__Pq::graph_abort",
            "__lib_dna__core___memory_store__Pq::graph_generation",
            "__lib_dna__core___memory_store__Pq::read_graph",
            "__lib_dna__core___record__GitJournal::genesis",
            "__lib_dna__core___record__MemRecord::signer",
            "__lib_dna__core___roles__HoldsReader::holds_any",
            "__lib_dna__core___roles__RecordRoles::holds",
            "__lib_dna__core___roles__RecordRoles::member_mapped",
            "__lib_dna__core___roles__RecordRoles::service",
            "__lib_dna__core___roles__RecordRoles::service_mapped",
            "__lib_dna__core___workflow_definition__WorkflowCatalog::encode",
            "__lib_dna__operations___definition_drafts__DefinitionDraftCapture::snapshot",
            "__lib_dna__operations___definition_drafts__DefinitionDraftCapture::supported",
            "__lib_dna__operations___definition_drafts__DefinitionDraftCodec::append",
            "__lib_dna__operations___definition_drafts__DefinitionDraftCodec::artifact",
            "__lib_dna__operations___definition_drafts__DefinitionDraftCodec::basis_equal",
            "__lib_dna__operations___definition_drafts__DefinitionDraftCodec::basis_shape",
            "__lib_dna__operations___definition_drafts__DefinitionDraftCodec::candidate_shape",
            "__lib_dna__operations___definition_drafts__DefinitionDraftCodec::candidate_text_bytes",
            "__lib_dna__operations___definition_drafts__DefinitionDraftCodec::catalog_text_bytes",
            "__lib_dna__operations___definition_drafts__DefinitionDraftCodec::dependents",
            "__lib_dna__operations___definition_drafts__DefinitionDraftCodec::reason",
            "__lib_dna__operations___definition_drafts__DefinitionDraftCodec::representable",
            "__lib_dna__operations___definition_drafts__DefinitionDraftCodec::step_count",
            "__lib_dna__operations___definition_drafts__DefinitionDraftCodec::step_stores",
            "__lib_dna__operations___definition_drafts__DefinitionDrafts::failure",
            "__lib_dna__operations___definition_drafts__DefinitionDrafts::recapture",
            "__lib_dna__operations___definition_drafts__DefinitionDrafts::semantic",
            "__lib_dna__operations___definition_drafts__DefinitionDrafts::validate",
            "__lib_dna__operations___definitions__DeclaredWorkflowCatalog::snapshot",
            "__lib_dna__operations___definitions__DeclaredWorkflowCatalog::supported",
            "__lib_dna__operations___definitions__DefinitionCodec::decode_exact",
            "__lib_dna__operations___definitions__DefinitionCodec::shape",
            "__lib_dna__operations___definitions__Definitions::basis_json",
            "__lib_dna__operations___definitions__Definitions::code",
            "__lib_dna__operations___definitions__Definitions::error",
            "__lib_dna__operations___definitions__Definitions::load",
            "__lib_dna__operations___definitions__Definitions::refuse",
            "__lib_dna__operations___definitions__Definitions::snapshot",
            "__lib_dna__operations___definitions__Definitions::status",
            "__lib_dna__operations___definitions__NoWorkflowCatalog::snapshot",
            "__lib_dna__operations___definitions__NoWorkflowCatalog::supported",
            "__lib_dna__operations___definitions__ProjectWorkflowCatalog::snapshot",
            "__lib_dna__operations___definitions__ProjectWorkflowCatalog::supported",
            "__lib_dna__organization_source___organization_source__OrganizationSource::declared",
            "__lib_dna__ui___main__CodeExchange::id_token",
            "__lib_dna__ui___main__CodeExchange::ready",
            "__lib_dna__ui___main__CodeExchange::vault_of",
            "__lib_dna__ui___main__Ui::bearer_subject",
            "__lib_dna__ui___main__Ui::begin_sign_in",
            "__lib_dna__ui___main__Ui::discovered",
            "__lib_dna__ui___main__Ui::end_session",
            "__lib_dna__ui___main__Ui::finish_sign_in",
            "__lib_dna__ui___main__Ui::handle",
            "__lib_dna__ui___main__Ui::read_keys",
            "__lib_dna__ui___main__Ui::session_name",
            "__lib_dna__ui___main__Ui::session_subject",
            "__lib_dna__ui___main__Ui::session_token",
            "__lib_dna__ui___main__Ui::take_pending",
            "__lib_dna__ui___main__Ui::token_subject",
            "__lib_dna__ui___main__Ui::verify",
        ],
        &[
            ("once-per-invocation -> ACCUMULATES-UNBOUNDED", 8),
            ("once-per-invocation -> per-iteration-reclaim", 144),
            ("per-iteration-reclaim -> ACCUMULATES-UNBOUNDED", 2),
        ],
        &[
            "__lib_dna__core___workspace__run_failed StructLit(\"std::process::ProcessOutput\") @1743491..1743570 InvokedUnboundedly",
            "__lib_dna__core___workspace__run_tool StructLit(\"__lib_dna__core___workspace__RunResult\") @1744203..1744271 InvokedUnboundedly",
            "__lib_dna__ui___main__Ui::begin_sign_in CollectionInsert(\"vec\") @2552788..2552891 InvokedUnboundedly",
            "__lib_dna__ui___main__Ui::discovered StringConcat @2548089..2548164 InvokedUnboundedly",
            "__lib_dna__ui___main__Ui::token_subject StringConcat @2550422..2550481 InvokedUnboundedly",
        ],
    );
    pinned(
        "dna/host",
        3364,
        &[
            "__lib_dna__core___decision__decision_line",
            "__lib_dna__core___handoff__classes_allow",
            "__lib_dna__core___handoff__classes_normal",
            "__lib_dna__core___hat__effect_class_ok",
            "__lib_dna__core___hat__hat_effects_of",
            "__lib_dna__core___memory_ledger__row_json",
            "__lib_dna__core___nerves__env_or_empty",
            "__lib_dna__core___nerves__nerves_app_name",
            "__lib_dna__core___nerves__nerves_durable_of",
            "__lib_dna__core___nerves__nerves_heart_prefix",
            "__lib_dna__core___nerves__nerves_heart_prefix_of",
            "__lib_dna__core___nerves__nerves_info_subject_of",
            "__lib_dna__core___nerves__nerves_org",
            "__lib_dna__core___nerves__nerves_password_var",
            "__lib_dna__core___nerves__nerves_role_vault",
            "__lib_dna__core___nerves__nerves_spine_url",
            "__lib_dna__core___nerves__nerves_stream",
            "__lib_dna__core___nerves__nerves_token",
            "__lib_dna__core___ownership__approver_owner",
            "__lib_dna__core___record__line_at",
            "__lib_dna__core___record__row_admissible",
            "__lib_dna__core___roles__record_trust_local",
            "__lib_dna__core___route__route_holders",
            "__lib_dna__core___route__show_list",
            "__lib_dna__core___routing__checkpoint_of",
            "__lib_dna__core___senses__part_labels",
            "__lib_dna__core___senses__senses_now",
            "__lib_dna__core___topics__pressure_what",
            "__lib_dna__core___workspace__body_mark",
            "__lib_dna__core___workspace__body_scan",
            "__lib_dna__core___workspace__in_new_session",
            "__lib_dna__core___workspace__join_lines",
            "__lib_dna__core___workspace__kill_marked",
            "__lib_dna__core___workspace__marked",
            "__lib_dna__core___workspace__session_members",
            "__lib_dna__core___workspace__sha256_file",
            "__lib_dna__core___workspace__wait_scale",
            "__lib_dna__core___workspace__wait_secs",
            "__lib_dna__core__pond__realtime__nats___types__nats_ack",
            "__lib_dna__core___record__MemRecord::signer",
        ],
        &[
            ("once-per-invocation -> ACCUMULATES-UNBOUNDED", 5),
            ("once-per-invocation -> per-iteration-reclaim", 8),
            ("per-iteration-reclaim -> ACCUMULATES-UNBOUNDED", 213),
        ],
        &[
            "__lib_dna__core___decision__decision_line StringConcat @917182..917289 InvokedUnboundedly",
            "__lib_dna__core___journal__MemJournal::read StructLit(\"__lib_dna__core___journal__Event\") @1001966..1001975 InvokedUnboundedly",
            "__lib_dna__core___knowledge__knowledge_review_id StringConcat @1025913..1025928 InvokedUnboundedly",
            "__lib_dna__core___memory_ledger__PqLedger::read StructLit(\"__lib_dna__core___journal__Event\") @1115466..1115475 InvokedUnboundedly",
            "__lib_dna__core___memory_schema__dsn_text StringConcat @1145673..1145803 InvokedUnboundedly",
            "__lib_dna__core___memory_schema__dsn_text StringConcat @1145818..1145945 InvokedUnboundedly",
            "__lib_dna__core___memory_schema__head_grants StringConcat @1168840..1169754 InvokedUnboundedly",
            "__lib_dna__core___memory_schema__owner_role StringConcat @1140770..1140813 InvokedUnboundedly",
            "__lib_dna__core___memory_schema__role_dsn StructLit(\"__lib_dna__core___memory_store__Dsn\") @1145305..1145453 InvokedUnboundedly",
            "__lib_dna__core___memory_schema__role_sql StringConcat @1167375..1167564 InvokedUnboundedly",
            "__lib_dna__core___memory_schema__role_vault_name StringConcat @1142422..1142461 InvokedUnboundedly",
            "__lib_dna__core___memory_spine__MemoryLedger::read StructLit(\"__lib_dna__core___journal__Event\") @1188105..1188114 InvokedUnboundedly",
            "__lib_dna__core___memory_store__parse_dsn StructLit(\"__lib_dna__core___memory_store__Dsn\") @1288113..1288149 InvokedUnboundedly",
            "__lib_dna__core___memory_store__parse_dsn StructLit(\"__lib_dna__core___memory_store__Dsn\") @1288169..1288185 InvokedUnboundedly",
            "__lib_dna__core___models__digest_of StringConcat @1305466..1305530 InvokedUnboundedly",
            "__lib_dna__core___models__model_key_slot StringConcat @1307841..1307855 InvokedUnboundedly",
            "__lib_dna__core___nerves__nerves_password_var StringConcat @1357288..1357333 InvokedUnboundedly",
            "__lib_dna__core___nerves__nerves_vault_name StringConcat @1354653..1354721 InvokedUnboundedly",
            "__lib_dna__core___ownership__word_add StringConcat @1422850..1422867 InvokedUnboundedly",
            "__lib_dna__core___record__GitJournal::read StructLit(\"__lib_dna__core___journal__Event\") @1528205..1528214 InvokedUnboundedly",
            "__lib_dna__core___workspace__body_mark StringConcat @2104965..2105030 InvokedUnboundedly",
            "__lib_dna__core___workspace__in_new_session StringConcat @2103208..2103501 InvokedUnboundedly",
            "__lib_dna__core___workspace__join_lines StringConcat @2104605..2104617 InvokedUnboundedly",
            "__lib_dna__core___workspace__run_failed StructLit(\"std::process::ProcessOutput\") @2108517..2108596 InvokedUnboundedly",
            "__lib_dna__core___workspace__run_tool StructLit(\"__lib_dna__core___workspace__RunResult\") @2109229..2109297 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::exec StructLit(\"db::DbError\") @2189564..2189634 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::exec StructLit(\"db::ExecResult\") @2189515..2189636 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::exec StructLit(\"db::ExecResult\") @2189663..2189834 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::exec_params StructLit(\"db::DbError\") @2191045..2191115 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::exec_params StructLit(\"db::ExecResult\") @2190996..2191117 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::exec_params StructLit(\"db::ExecResult\") @2191144..2191231 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::query_one StructLit(\"db::DbError\") @2189999..2190069 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::query_one StructLit(\"db::DbError\") @2190166..2190246 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::query_one StructLit(\"db::Row\") @2189957..2190071 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::query_one StructLit(\"db::Row\") @2190124..2190248 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::query_one StructLit(\"db::Row\") @2190439..2190472 InvokedUnboundedly",
            "__lib_dna__core__pond__realtime__nats___client__NatsClient::fail_with StructLit(\"__lib_dna__core__pond__realtime__nats___types__NatsError\") @3003688..3003728 InvokedUnboundedly",
            "__lib_dna__core__pond__realtime__nats___jetstream__js_error_of StringConcat @3021436..3021522 InvokedUnboundedly",
            "__lib_dna__operations___governance_admission__GovernanceJournal::read StructLit(\"dna::Event\") @2350016..2350030 InvokedUnboundedly",
            "__lib_dna__operations___graph__graph_node_id StringConcat @2480429..2480446 InvokedUnboundedly",
            "__lib_dna__operations___graph__graph_node_row StructLit(\"__lib_dna__operations___graph__GraphRow\") @2487849..2487902 InvokedUnboundedly",
            "__lib_dna__operations___graph__graph_refused StructLit(\"__lib_dna__operations___graph__GraphRow\") @2486974..2486995 InvokedUnboundedly",
            "__lib_dna__operations___graph__graph_retired_row StructLit(\"__lib_dna__operations___graph__GraphRow\") @2494369..2494414 InvokedUnboundedly",
            "__lib_dna__operations___graph__graph_retired_row StructLit(\"__lib_dna__operations___graph__GraphRow\") @2494989..2495034 InvokedUnboundedly",
            "__lib_dna__operations___graph_holes__hole_line StringConcat @2500812..2500829 InvokedUnboundedly",
            "__lib_dna__operations___person_retirement__RetirementPrefix::read StructLit(\"dna::Event\") @2796779..2796793 InvokedUnboundedly",
            "__lib_dna__operations___usage__usage_counts_text StringConcat @2897360..2897484 InvokedUnboundedly",
        ],
    );
    pinned(
        "dna/operations",
        2173,
        &[
            "__lib_dna__core___hat__effect_class_ok",
            "__lib_dna__core___hat__hat_effects_of",
            "__lib_dna__core___ownership__approver_owner",
            "__lib_dna__core___record__line_at",
            "__lib_dna__core___record__row_admissible",
            "__lib_dna__core___roles__record_trust_local",
            "__lib_dna__core___route__show_list",
            "__lib_dna__core___topics__pressure_what",
            "__lib_dna__core___record__GitRecord::signer",
            "__lib_dna__core___record__MemRecord::signer",
        ],
        &[
            ("once-per-invocation -> per-iteration-reclaim", 7),
        ],
        &[],
    );
    pinned(
        "dna/organization_runtime",
        2186,
        &[
            "__lib_dna__core___hat__effect_class_ok",
            "__lib_dna__core___hat__hat_effects_of",
            "__lib_dna__core___ownership__approver_owner",
            "__lib_dna__core___record__line_at",
            "__lib_dna__core___record__row_admissible",
            "__lib_dna__core___roles__record_trust_local",
            "__lib_dna__core___route__show_list",
            "__lib_dna__core___topics__pressure_what",
            "__lib_dna__core___record__GitRecord::signer",
            "__lib_dna__core___record__MemRecord::signer",
        ],
        &[
            ("once-per-invocation -> per-iteration-reclaim", 7),
        ],
        &[],
    );
    pinned(
        "dna/ui",
        1302,
        &[
            "__lib_dna__core___knowledge__url_encode",
            "__lib_dna__core___principal__audience_names",
            "__lib_dna__core___principal__b64url_text",
            "__lib_dna__core___principal__bearer_refusal",
            "__lib_dna__core___principal__claims_refusal",
            "__lib_dna__core___principal__cookie_value",
            "__lib_dna__core___principal__discovery_refusal",
            "__lib_dna__core___principal__endpoint_allowed",
            "__lib_dna__core___principal__es256_refusal",
            "__lib_dna__core___principal__id_token_claims",
            "__lib_dna__core___principal__issuer_loopback",
            "__lib_dna__core___principal__jwks_keys",
            "__lib_dna__core___principal__member_of",
            "__lib_dna__core___principal__spki_pem",
            "__lib_dna__core___principal__url_parts",
            "clean",
            "hale_bin",
            "page",
            "redirect",
            "run_dna",
            "run_failed",
            "text",
            "token_failed",
            "CodeExchange::id_token",
            "CodeExchange::ready",
            "CodeExchange::vault_of",
            "Ui::begin_sign_in",
            "Ui::discovered",
            "Ui::end_session",
            "Ui::finish_sign_in",
            "Ui::handle",
            "Ui::read_keys",
            "Ui::session_name",
            "Ui::take_pending",
            "Ui::verify",
        ],
        &[
            ("once-per-invocation -> ACCUMULATES-UNBOUNDED", 8),
            ("once-per-invocation -> per-iteration-reclaim", 26),
        ],
        &[
            "Ui::begin_sign_in CollectionInsert(\"vec\") @12489..12592 InvokedUnboundedly",
            "Ui::discovered StringConcat @7790..7865 InvokedUnboundedly",
            "run_failed StructLit(\"std::process::ProcessOutput\") @1507..1586 InvokedUnboundedly",
        ],
    );
    pinned(
        "tests/hale/api_binding_run_test.hl",
        2,
        &[
            "Table::holds",
            "Tokens::principal",
            "Tokens::refused",
        ],
        &[
            ("once-per-invocation -> per-iteration-reclaim", 1),
        ],
        &[
        ],
    );
    pinned(
        "tests/hale/router_middleware_test.hl",
        4,
        &[
            "Hello::handle",
            "Stamp::after",
            "Stamp::before",
        ],
        &[
            ("once-per-invocation -> per-iteration-reclaim", 4),
        ],
        &[
        ],
    );
}
