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
                let before: AllocSummary = summarize_identified(&identified, &[]);
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

/// The 27 targets below are the only ones the correction changes.
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
    ("crates/hale-codegen/tests/fixtures/examples/docs-server", 1),
    ("crates/hale-codegen/tests/fixtures/examples/http-hello", 1),
    ("dna/core", 1248),
    ("dna/oidc", 34),
    ("dna/organism", 1315),
    ("dna/organization_source", 1288),
    ("dna/reflexes", 68),
    ("tests/hale/api_big_reply_test.hl", 2),
    ("tests/hale/api_binding_test.hl", 2),
    ("tests/hale/api_context_test.hl", 2),
    ("tests/hale/api_roles_test.hl", 2),
    ("tests/hale/api_roles_xseed_test.hl", 2),
    ("tests/hale/api_serve_test.hl", 2),
    ("tests/hale/imported_fn_value_test.hl", 1),
    ("tests/hale/json_unicode_escapes_test.hl", 16),
    ("tests/hale/json_valid_test.hl", 27),
    ("tests/hale/log_fields_test.hl", 6),
    ("tests/hale/secret_write_private_test.hl", 10),
    ("tests/hale/std_secret_test.hl", 8),
];

/// The targets whose verdicts move, each pinned in `verdict_changes`.
const VERDICT_TARGETS: [&str; 8] = [
    "crates/hale-codegen/tests/fixtures/examples/69-http-router",
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
    pinned(
        "dna/api",
        3025,
        &[
            "__lib_dna_core_hat_effect_class_ok",
            "__lib_dna_core_hat_hat_effects_of",
            "__lib_dna_core_hat_hat_json",
            "__lib_dna_core_knowledge_url_encode",
            "__lib_dna_core_ownership_approver_owner",
            "__lib_dna_core_principal_audience_names",
            "__lib_dna_core_principal_b64url_text",
            "__lib_dna_core_principal_bearer_refusal",
            "__lib_dna_core_principal_bearer_token",
            "__lib_dna_core_principal_claims_refusal",
            "__lib_dna_core_principal_cookie_value",
            "__lib_dna_core_principal_discovery_refusal",
            "__lib_dna_core_principal_endpoint_allowed",
            "__lib_dna_core_principal_es256_refusal",
            "__lib_dna_core_principal_id_token_claims",
            "__lib_dna_core_principal_issuer_loopback",
            "__lib_dna_core_principal_jwks_keys",
            "__lib_dna_core_principal_spki_pem",
            "__lib_dna_core_principal_url_parts",
            "__lib_dna_core_record_line_at",
            "__lib_dna_core_record_row_admissible",
            "__lib_dna_core_roles_record_trust_local",
            "__lib_dna_core_route_show_list",
            "__lib_dna_core_topics_pressure_what",
            "__lib_dna_core_workflow_definition_baseline_definitions",
            "__lib_dna_core_workflow_definition_define_baseline",
            "__lib_dna_core_workflow_definition_organism_leaf",
            "__lib_dna_operations_context_hat_json",
            "__lib_dna_operations_usage_usage_json",
            "__lib_dna_operations_usage_usage_lines_json",
            "__lib_dna_ui_main_clean",
            "__lib_dna_ui_main_hale_bin",
            "__lib_dna_ui_main_page",
            "__lib_dna_ui_main_redirect",
            "__lib_dna_ui_main_run_dna",
            "__lib_dna_ui_main_run_failed",
            "__lib_dna_ui_main_text",
            "__lib_dna_ui_main_token_failed",
            "command_guard",
            "commands_port",
            "definition_drafts_over",
            "definitions_over",
            "exchange",
            "graph_read",
            "query_token",
            "relay_command",
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
            "__lib_dna_core_memory_store_GraphRules::projection_valid",
            "__lib_dna_core_memory_store_GraphRules::validate",
            "__lib_dna_core_memory_store_Pq::graph_abort",
            "__lib_dna_core_memory_store_Pq::graph_generation",
            "__lib_dna_core_memory_store_Pq::read_graph",
            "__lib_dna_core_record_GitJournal::genesis",
            "__lib_dna_core_record_MemRecord::signer",
            "__lib_dna_core_roles_HoldsReader::holds_any",
            "__lib_dna_core_roles_RecordRoles::holds",
            "__lib_dna_core_roles_RecordRoles::member_mapped",
            "__lib_dna_core_roles_RecordRoles::service",
            "__lib_dna_core_roles_RecordRoles::service_mapped",
            "__lib_dna_core_workflow_definition_WorkflowCatalog::encode",
            "__lib_dna_operations_definition_drafts_DefinitionDraftCapture::snapshot",
            "__lib_dna_operations_definition_drafts_DefinitionDraftCapture::supported",
            "__lib_dna_operations_definition_drafts_DefinitionDraftCodec::append",
            "__lib_dna_operations_definition_drafts_DefinitionDraftCodec::artifact",
            "__lib_dna_operations_definition_drafts_DefinitionDraftCodec::basis_equal",
            "__lib_dna_operations_definition_drafts_DefinitionDraftCodec::basis_shape",
            "__lib_dna_operations_definition_drafts_DefinitionDraftCodec::candidate_shape",
            "__lib_dna_operations_definition_drafts_DefinitionDraftCodec::candidate_text_bytes",
            "__lib_dna_operations_definition_drafts_DefinitionDraftCodec::catalog_text_bytes",
            "__lib_dna_operations_definition_drafts_DefinitionDraftCodec::dependents",
            "__lib_dna_operations_definition_drafts_DefinitionDraftCodec::reason",
            "__lib_dna_operations_definition_drafts_DefinitionDraftCodec::representable",
            "__lib_dna_operations_definition_drafts_DefinitionDraftCodec::step_count",
            "__lib_dna_operations_definition_drafts_DefinitionDraftCodec::step_stores",
            "__lib_dna_operations_definition_drafts_DefinitionDrafts::failure",
            "__lib_dna_operations_definition_drafts_DefinitionDrafts::recapture",
            "__lib_dna_operations_definition_drafts_DefinitionDrafts::semantic",
            "__lib_dna_operations_definition_drafts_DefinitionDrafts::validate",
            "__lib_dna_operations_definitions_DeclaredWorkflowCatalog::snapshot",
            "__lib_dna_operations_definitions_DeclaredWorkflowCatalog::supported",
            "__lib_dna_operations_definitions_DefinitionCodec::decode_exact",
            "__lib_dna_operations_definitions_DefinitionCodec::shape",
            "__lib_dna_operations_definitions_Definitions::basis_json",
            "__lib_dna_operations_definitions_Definitions::code",
            "__lib_dna_operations_definitions_Definitions::error",
            "__lib_dna_operations_definitions_Definitions::load",
            "__lib_dna_operations_definitions_Definitions::refuse",
            "__lib_dna_operations_definitions_Definitions::snapshot",
            "__lib_dna_operations_definitions_Definitions::status",
            "__lib_dna_operations_definitions_NoWorkflowCatalog::snapshot",
            "__lib_dna_operations_definitions_NoWorkflowCatalog::supported",
            "__lib_dna_operations_definitions_ProjectWorkflowCatalog::snapshot",
            "__lib_dna_operations_definitions_ProjectWorkflowCatalog::supported",
            "__lib_dna_organization_source_organization_source_OrganizationSource::declared",
            "__lib_dna_ui_main_CodeExchange::id_token",
            "__lib_dna_ui_main_CodeExchange::ready",
            "__lib_dna_ui_main_CodeExchange::vault_of",
            "__lib_dna_ui_main_Ui::bearer_subject",
            "__lib_dna_ui_main_Ui::begin_sign_in",
            "__lib_dna_ui_main_Ui::discovered",
            "__lib_dna_ui_main_Ui::end_session",
            "__lib_dna_ui_main_Ui::finish_sign_in",
            "__lib_dna_ui_main_Ui::handle",
            "__lib_dna_ui_main_Ui::read_keys",
            "__lib_dna_ui_main_Ui::session_name",
            "__lib_dna_ui_main_Ui::session_subject",
            "__lib_dna_ui_main_Ui::session_token",
            "__lib_dna_ui_main_Ui::take_pending",
            "__lib_dna_ui_main_Ui::token_subject",
            "__lib_dna_ui_main_Ui::verify",
        ],
        &[
            ("once-per-invocation -> ACCUMULATES-UNBOUNDED", 8),
            ("once-per-invocation -> per-iteration-reclaim", 139),
            ("per-iteration-reclaim -> ACCUMULATES-UNBOUNDED", 2),
        ],
        &[
            "__lib_dna_core_workspace_run_failed StructLit(\"std::process::ProcessOutput\") @1706666..1706745 InvokedUnboundedly",
            "__lib_dna_core_workspace_run_tool StructLit(\"__lib_dna_core_workspace_RunResult\") @1707378..1707446 InvokedUnboundedly",
            "__lib_dna_ui_main_Ui::begin_sign_in CollectionInsert(\"vec\") @1847003..1847106 InvokedUnboundedly",
            "__lib_dna_ui_main_Ui::discovered StringConcat @1842304..1842379 InvokedUnboundedly",
            "__lib_dna_ui_main_Ui::token_subject StringConcat @1844637..1844696 InvokedUnboundedly",
        ],
    );
    pinned(
        "dna/host",
        3297,
        &[
            "__lib_dna_core_decision_decision_line",
            "__lib_dna_core_handoff_classes_allow",
            "__lib_dna_core_handoff_classes_normal",
            "__lib_dna_core_hat_effect_class_ok",
            "__lib_dna_core_hat_hat_effects_of",
            "__lib_dna_core_memory_ledger_row_json",
            "__lib_dna_core_nerves_env_or_empty",
            "__lib_dna_core_nerves_nerves_app_name",
            "__lib_dna_core_nerves_nerves_durable_of",
            "__lib_dna_core_nerves_nerves_heart_prefix",
            "__lib_dna_core_nerves_nerves_heart_prefix_of",
            "__lib_dna_core_nerves_nerves_info_subject_of",
            "__lib_dna_core_nerves_nerves_org",
            "__lib_dna_core_nerves_nerves_password_var",
            "__lib_dna_core_nerves_nerves_role_vault",
            "__lib_dna_core_nerves_nerves_spine_url",
            "__lib_dna_core_nerves_nerves_stream",
            "__lib_dna_core_nerves_nerves_token",
            "__lib_dna_core_ownership_approver_owner",
            "__lib_dna_core_pond_realtime_nats_types_nats_ack",
            "__lib_dna_core_record_line_at",
            "__lib_dna_core_record_row_admissible",
            "__lib_dna_core_roles_record_trust_local",
            "__lib_dna_core_route_route_holders",
            "__lib_dna_core_route_show_list",
            "__lib_dna_core_routing_checkpoint_of",
            "__lib_dna_core_senses_part_labels",
            "__lib_dna_core_senses_senses_now",
            "__lib_dna_core_topics_pressure_what",
            "__lib_dna_core_workspace_body_mark",
            "__lib_dna_core_workspace_body_scan",
            "__lib_dna_core_workspace_in_new_session",
            "__lib_dna_core_workspace_join_lines",
            "__lib_dna_core_workspace_kill_marked",
            "__lib_dna_core_workspace_marked",
            "__lib_dna_core_workspace_session_members",
            "__lib_dna_core_workspace_sha256_file",
            "__lib_dna_core_workspace_wait_scale",
            "__lib_dna_core_workspace_wait_secs",
            "__lib_dna_core_record_MemRecord::signer",
        ],
        &[
            ("once-per-invocation -> ACCUMULATES-UNBOUNDED", 5),
            ("once-per-invocation -> per-iteration-reclaim", 8),
            ("per-iteration-reclaim -> ACCUMULATES-UNBOUNDED", 213),
        ],
        &[
            "__lib_dna_core_decision_decision_line StringConcat @900537..900644 InvokedUnboundedly",
            "__lib_dna_core_journal_MemJournal::read StructLit(\"__lib_dna_core_journal_Event\") @976134..976143 InvokedUnboundedly",
            "__lib_dna_core_knowledge_knowledge_review_id StringConcat @999629..999644 InvokedUnboundedly",
            "__lib_dna_core_memory_ledger_PqLedger::read StructLit(\"__lib_dna_core_journal_Event\") @1084004..1084013 InvokedUnboundedly",
            "__lib_dna_core_memory_schema_dsn_text StringConcat @1114211..1114341 InvokedUnboundedly",
            "__lib_dna_core_memory_schema_dsn_text StringConcat @1114356..1114483 InvokedUnboundedly",
            "__lib_dna_core_memory_schema_head_grants StringConcat @1137378..1138292 InvokedUnboundedly",
            "__lib_dna_core_memory_schema_owner_role StringConcat @1109308..1109351 InvokedUnboundedly",
            "__lib_dna_core_memory_schema_role_dsn StructLit(\"__lib_dna_core_memory_store_Dsn\") @1113843..1113991 InvokedUnboundedly",
            "__lib_dna_core_memory_schema_role_sql StringConcat @1135913..1136102 InvokedUnboundedly",
            "__lib_dna_core_memory_schema_role_vault_name StringConcat @1110960..1110999 InvokedUnboundedly",
            "__lib_dna_core_memory_spine_MemoryLedger::read StructLit(\"__lib_dna_core_journal_Event\") @1156643..1156652 InvokedUnboundedly",
            "__lib_dna_core_memory_store_parse_dsn StructLit(\"__lib_dna_core_memory_store_Dsn\") @1256077..1256113 InvokedUnboundedly",
            "__lib_dna_core_memory_store_parse_dsn StructLit(\"__lib_dna_core_memory_store_Dsn\") @1256133..1256149 InvokedUnboundedly",
            "__lib_dna_core_models_digest_of StringConcat @1273430..1273494 InvokedUnboundedly",
            "__lib_dna_core_models_model_key_slot StringConcat @1275805..1275819 InvokedUnboundedly",
            "__lib_dna_core_nerves_nerves_password_var StringConcat @1325243..1325288 InvokedUnboundedly",
            "__lib_dna_core_nerves_nerves_vault_name StringConcat @1322608..1322676 InvokedUnboundedly",
            "__lib_dna_core_ownership_word_add StringConcat @1390697..1390714 InvokedUnboundedly",
            "__lib_dna_core_pond_pq_pq_PgConn::exec StructLit(\"db::DbError\") @2142505..2142575 InvokedUnboundedly",
            "__lib_dna_core_pond_pq_pq_PgConn::exec StructLit(\"db::ExecResult\") @2142456..2142577 InvokedUnboundedly",
            "__lib_dna_core_pond_pq_pq_PgConn::exec StructLit(\"db::ExecResult\") @2142604..2142775 InvokedUnboundedly",
            "__lib_dna_core_pond_pq_pq_PgConn::exec_params StructLit(\"db::DbError\") @2143986..2144056 InvokedUnboundedly",
            "__lib_dna_core_pond_pq_pq_PgConn::exec_params StructLit(\"db::ExecResult\") @2143937..2144058 InvokedUnboundedly",
            "__lib_dna_core_pond_pq_pq_PgConn::exec_params StructLit(\"db::ExecResult\") @2144085..2144172 InvokedUnboundedly",
            "__lib_dna_core_pond_pq_pq_PgConn::query_one StructLit(\"db::DbError\") @2142940..2143010 InvokedUnboundedly",
            "__lib_dna_core_pond_pq_pq_PgConn::query_one StructLit(\"db::DbError\") @2143107..2143187 InvokedUnboundedly",
            "__lib_dna_core_pond_pq_pq_PgConn::query_one StructLit(\"db::Row\") @2142898..2143012 InvokedUnboundedly",
            "__lib_dna_core_pond_pq_pq_PgConn::query_one StructLit(\"db::Row\") @2143065..2143189 InvokedUnboundedly",
            "__lib_dna_core_pond_pq_pq_PgConn::query_one StructLit(\"db::Row\") @2143380..2143413 InvokedUnboundedly",
            "__lib_dna_core_pond_realtime_nats_client_NatsClient::fail_with StructLit(\"__lib_dna_core_pond_realtime_nats_types_NatsError\") @2950706..2950746 InvokedUnboundedly",
            "__lib_dna_core_pond_realtime_nats_jetstream_js_error_of StringConcat @2968454..2968540 InvokedUnboundedly",
            "__lib_dna_core_record_GitJournal::read StructLit(\"__lib_dna_core_journal_Event\") @1485118..1485127 InvokedUnboundedly",
            "__lib_dna_core_workspace_body_mark StringConcat @2057906..2057971 InvokedUnboundedly",
            "__lib_dna_core_workspace_in_new_session StringConcat @2056149..2056442 InvokedUnboundedly",
            "__lib_dna_core_workspace_join_lines StringConcat @2057546..2057558 InvokedUnboundedly",
            "__lib_dna_core_workspace_run_failed StructLit(\"std::process::ProcessOutput\") @2061458..2061537 InvokedUnboundedly",
            "__lib_dna_core_workspace_run_tool StructLit(\"__lib_dna_core_workspace_RunResult\") @2062170..2062238 InvokedUnboundedly",
            "__lib_dna_operations_governance_admission_GovernanceJournal::read StructLit(\"dna::Event\") @2300616..2300630 InvokedUnboundedly",
            "__lib_dna_operations_graph_graph_node_id StringConcat @2430091..2430108 InvokedUnboundedly",
            "__lib_dna_operations_graph_graph_node_row StructLit(\"__lib_dna_operations_graph_GraphRow\") @2437511..2437564 InvokedUnboundedly",
            "__lib_dna_operations_graph_graph_refused StructLit(\"__lib_dna_operations_graph_GraphRow\") @2436636..2436657 InvokedUnboundedly",
            "__lib_dna_operations_graph_graph_retired_row StructLit(\"__lib_dna_operations_graph_GraphRow\") @2444031..2444076 InvokedUnboundedly",
            "__lib_dna_operations_graph_graph_retired_row StructLit(\"__lib_dna_operations_graph_GraphRow\") @2444651..2444696 InvokedUnboundedly",
            "__lib_dna_operations_graph_holes_hole_line StringConcat @2450474..2450491 InvokedUnboundedly",
            "__lib_dna_operations_person_retirement_RetirementPrefix::read StructLit(\"dna::Event\") @2744429..2744443 InvokedUnboundedly",
            "__lib_dna_operations_usage_usage_counts_text StringConcat @2844423..2844547 InvokedUnboundedly",
        ],
    );
    pinned(
        "dna/operations",
        2147,
        &[
            "__lib_dna_core_hat_effect_class_ok",
            "__lib_dna_core_hat_hat_effects_of",
            "__lib_dna_core_ownership_approver_owner",
            "__lib_dna_core_record_line_at",
            "__lib_dna_core_record_row_admissible",
            "__lib_dna_core_roles_record_trust_local",
            "__lib_dna_core_route_show_list",
            "__lib_dna_core_topics_pressure_what",
            "__lib_dna_core_record_GitRecord::signer",
            "__lib_dna_core_record_MemRecord::signer",
        ],
        &[
            ("once-per-invocation -> per-iteration-reclaim", 7),
        ],
        &[
        ],
    );
    pinned(
        "dna/organization_runtime",
        2160,
        &[
            "__lib_dna_core_hat_effect_class_ok",
            "__lib_dna_core_hat_hat_effects_of",
            "__lib_dna_core_ownership_approver_owner",
            "__lib_dna_core_record_line_at",
            "__lib_dna_core_record_row_admissible",
            "__lib_dna_core_roles_record_trust_local",
            "__lib_dna_core_route_show_list",
            "__lib_dna_core_topics_pressure_what",
            "__lib_dna_core_record_GitRecord::signer",
            "__lib_dna_core_record_MemRecord::signer",
        ],
        &[
            ("once-per-invocation -> per-iteration-reclaim", 7),
        ],
        &[
        ],
    );
    pinned(
        "dna/ui",
        1279,
        &[
            "__lib_dna_core_knowledge_url_encode",
            "__lib_dna_core_principal_audience_names",
            "__lib_dna_core_principal_b64url_text",
            "__lib_dna_core_principal_bearer_refusal",
            "__lib_dna_core_principal_claims_refusal",
            "__lib_dna_core_principal_cookie_value",
            "__lib_dna_core_principal_discovery_refusal",
            "__lib_dna_core_principal_endpoint_allowed",
            "__lib_dna_core_principal_es256_refusal",
            "__lib_dna_core_principal_id_token_claims",
            "__lib_dna_core_principal_issuer_loopback",
            "__lib_dna_core_principal_jwks_keys",
            "__lib_dna_core_principal_member_of",
            "__lib_dna_core_principal_spki_pem",
            "__lib_dna_core_principal_url_parts",
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
        13,
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
