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
    // the host's verb program, a seed inside the host's (GH #1490)
    out.push("dna/host/verb".to_string());
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
                let mut before: AllocSummary = without_frames(&summarize_identified(&identified, &[]));
                // This correction alone: the reclaim boundary's (E3b) and
                // the startable loci's (GH #1490) cleared on both sides.
                let mut now = without_frames(now);
                before.startable = None;
                now.startable = None;
                let now = &now;
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

/// The 32 targets below are the only ones the correction changes.
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
    ("dna/core", 1568),
    ("dna/oidc", 34),
    ("dna/organism", 1635),
    ("dna/organization_source", 1608),
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
const VERDICT_TARGETS: [&str; 12] = [
    "crates/hale-codegen/tests/fixtures/examples/69-http-router",
    "crates/hale-codegen/tests/fixtures/examples/92-build-an-api",
    "crates/hale-codegen/tests/fixtures/examples/docs-server",
    "crates/hale-codegen/tests/fixtures/examples/http-hello",
    "dna/api",
    "dna/host",
    "dna/host/verb",
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
        3463,
        &[
            "__lib_dna__core___batch__batch_diff",
            "__lib_dna__core___batch__batch_filing",
            "__lib_dna__core___batch__batch_pending",
            "__lib_dna__core___batch__batch_summary",
            "__lib_dna__core___hat__effect_class_ok",
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
            "__lib_dna__organization_source___organization_dependencies__OrganizationInspection::archive_ok",
            "__lib_dna__organization_source___organization_dependencies__OrganizationInspection::input_error",
            "__lib_dna__organization_source___organization_dependencies__OrganizationInspection::source_ok",
            "__lib_dna__organization_source___organization_dependencies__OrganizationInspection::tree",
            "__lib_dna__organization_source___organization_source__OrganizationSource::declared",
            "__lib_dna__organization_source___organization_source__OrganizationSource::draft",
            "__lib_dna__organization_source___organization_source__OrganizationSource::head",
            "__lib_dna__organization_source___organization_source__OrganizationSource::inspect",
            "__lib_dna__organization_source___organization_source__OrganizationSource::inspect_version",
            "__lib_dna__organization_source___organization_source__OrganizationSource::local_vendor",
            "__lib_dna__organization_source___organization_source__OrganizationSource::read",
            "__lib_dna__organization_source___organization_source__OrganizationSource::remove_scratch",
            "__lib_dna__organization_source___organization_source__OrganizationSource::scratch",
            "__lib_dna__organization_source___organization_source__OrganizationSource::unchanged",
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
            ("once-per-invocation -> per-iteration-reclaim", 179),
            ("per-iteration-reclaim -> ACCUMULATES-UNBOUNDED", 2),
        ],
        &[
            "__lib_dna__core___workspace__run_failed StructLit(\"std::process::ProcessOutput\") @1878257..1878336 InvokedUnboundedly",
            "__lib_dna__core___workspace__run_tool StructLit(\"__lib_dna__core___workspace__RunResult\") @1878969..1879037 InvokedUnboundedly",
            "__lib_dna__ui___main__Ui::begin_sign_in CollectionInsert(\"vec\") @2762126..2762229 InvokedUnboundedly",
            "__lib_dna__ui___main__Ui::discovered StringConcat @2757427..2757502 InvokedUnboundedly",
            "__lib_dna__ui___main__Ui::token_subject StringConcat @2759760..2759819 InvokedUnboundedly",
        ],
    );
    pinned(
        "dna/host/verb",
        4119,
        &[
            "__lib_dna__core___batch__batch_filing",
            "__lib_dna__core___batch__batch_pending",
            "__lib_dna__core___batch__batch_summary",
            "__lib_dna__core___decision__decision_line",
            "__lib_dna__core___handoff__classes_allow",
            "__lib_dna__core___handoff__classes_normal",
            "__lib_dna__core___hat__effect_class_ok",
            "__lib_dna__core___memory_ledger__row_json",
            "__lib_dna__core___models__model_key_slot",
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
            "__lib_dna__operations___governance_policy__FixedAuthority::authorize",
            "__lib_dna__operations___knowledge_policy__FixedKnowledgeAuthority::authorize",
            "__lib_dna__operations___organization_policy__FixedOrganizationAuthority::authorize",
            "__lib_dna__operations___task_policy__FixedTaskAuthority::authorize",
        ],
        &[
            ("once-per-invocation -> ACCUMULATES-UNBOUNDED", 5),
            ("once-per-invocation -> per-iteration-reclaim", 24),
            ("per-iteration-reclaim -> ACCUMULATES-UNBOUNDED", 1389),
        ],
        &[
            "__lib_dna__core___batch__batch_position StringConcat @916607..916625 InvokedUnboundedly",
            "__lib_dna__core___batch__batch_row StringConcat @961076..961093 InvokedUnboundedly",
            "__lib_dna__core___batch__batch_usage StringConcat @918243..918281 InvokedUnboundedly",
            "__lib_dna__core___batch__plan_assign StringConcat @945652..945778 InvokedUnboundedly",
            "__lib_dna__core___batch__plan_assign StringConcat @945796..945845 InvokedUnboundedly",
            "__lib_dna__core___batch__plan_fill StringConcat @945107..945216 InvokedUnboundedly",
            "__lib_dna__core___batch__plan_fill StringConcat @945234..945275 InvokedUnboundedly",
            "__lib_dna__core___batch__plan_open StringConcat @944621..944695 InvokedUnboundedly",
            "__lib_dna__core___batch__plan_open StringConcat @944733..944849 InvokedUnboundedly",
            "__lib_dna__core___batch__plan_open StringConcat @944872..944912 InvokedUnboundedly",
            "__lib_dna__core___batch__plan_open StringConcat @944930..944983 InvokedUnboundedly",
            "__lib_dna__core___batch__plan_unassign StringConcat @945981..946040 InvokedUnboundedly",
            "__lib_dna__core___batch__plan_vacate StringConcat @945403..945454 InvokedUnboundedly",
            "__lib_dna__core___decision__decision_line StringConcat @1021992..1022099 InvokedUnboundedly",
            "__lib_dna__core___forge__MemForge::open_review StructLit(\"__lib_dna__core___forge__ReviewRef\") @1080547..1080584 InvokedUnboundedly",
            "__lib_dna__core___forge__NoForge::open_review StructLit(\"__lib_dna__core___forge__ReviewRef\") @1083053..1083082 InvokedUnboundedly",
            "__lib_dna__core___journal__MemJournal::append StructLit(\"__lib_dna__core___journal__AppendResult\") @1111548..1111621 InvokedUnboundedly",
            "__lib_dna__core___journal__MemJournal::append StructLit(\"__lib_dna__core___journal__AppendResult\") @1111816..1111860 InvokedUnboundedly",
            "__lib_dna__core___journal__MemJournal::read StructLit(\"__lib_dna__core___journal__Event\") @1112039..1112048 InvokedUnboundedly",
            "__lib_dna__core___knowledge__knowledge_review_id StringConcat @1135986..1136001 InvokedUnboundedly",
            "__lib_dna__core___memory_ledger__PqLedger::read StructLit(\"__lib_dna__core___journal__Event\") @1237092..1237101 InvokedUnboundedly",
            "__lib_dna__core___memory_schema__dsn_text StringConcat @1267598..1267728 InvokedUnboundedly",
            "__lib_dna__core___memory_schema__dsn_text StringConcat @1267743..1267870 InvokedUnboundedly",
            "__lib_dna__core___memory_schema__head_grants StringConcat @1290765..1291679 InvokedUnboundedly",
            "__lib_dna__core___memory_schema__owner_role StringConcat @1262695..1262738 InvokedUnboundedly",
            "__lib_dna__core___memory_schema__role_dsn StructLit(\"__lib_dna__core___memory_store__Dsn\") @1267230..1267378 InvokedUnboundedly",
            "__lib_dna__core___memory_schema__role_sql StringConcat @1289300..1289489 InvokedUnboundedly",
            "__lib_dna__core___memory_schema__role_vault_name StringConcat @1264347..1264386 InvokedUnboundedly",
            "__lib_dna__core___memory_spine__MemoryLedger::append StructLit(\"__lib_dna__core___journal__AppendResult\") @1311266..1311325 InvokedUnboundedly",
            "__lib_dna__core___memory_spine__MemoryLedger::read StructLit(\"__lib_dna__core___journal__Event\") @1310563..1310572 InvokedUnboundedly",
            "__lib_dna__core___memory_store__parse_dsn StructLit(\"__lib_dna__core___memory_store__Dsn\") @1410932..1410968 InvokedUnboundedly",
            "__lib_dna__core___memory_store__parse_dsn StructLit(\"__lib_dna__core___memory_store__Dsn\") @1410988..1411004 InvokedUnboundedly",
            "__lib_dna__core___models__digest_of StringConcat @1438047..1438111 InvokedUnboundedly",
            "__lib_dna__core___models__model_key_slot StringConcat @1443823..1443837 InvokedUnboundedly",
            "__lib_dna__core___nerves__nerves_password_var StringConcat @1493637..1493682 InvokedUnboundedly",
            "__lib_dna__core___nerves__nerves_vault_name StringConcat @1491002..1491070 InvokedUnboundedly",
            "__lib_dna__core___ownership__word_add StringConcat @1559199..1559216 InvokedUnboundedly",
            "__lib_dna__core___record__GitJournal::read StructLit(\"__lib_dna__core___journal__Event\") @1672886..1672895 InvokedUnboundedly",
            "__lib_dna__core___record__GitRecord::ids StringConcat @1642527..1642592 InvokedUnboundedly",
            "__lib_dna__core___record__GitRecord::text_at StringConcat @1642400..1642408 InvokedUnboundedly",
            "__lib_dna__core___record_rows__graph_body_put StringConcat @1689068..1689120 InvokedUnboundedly",
            "__lib_dna__core___record_rows__record_graph_put StringConcat @1685271..1685292 InvokedUnboundedly",
            "__lib_dna__core___record_rows__record_graph_put StringConcat @1685390..1685411 InvokedUnboundedly",
            "__lib_dna__core___routing__RoutedJournal::append StructLit(\"__lib_dna__core___journal__AppendResult\") @1792525..1792598 InvokedUnboundedly",
            "__lib_dna__core___routing__RoutedJournal::append StructLit(\"__lib_dna__core___journal__AppendResult\") @1792835..1792912 InvokedUnboundedly",
            "__lib_dna__core___routing__RoutedJournal::append StructLit(\"__lib_dna__core___journal__AppendResult\") @1792956..1793009 InvokedUnboundedly",
            "__lib_dna__core___routing__RoutedJournal::read StructLit(\"__lib_dna__core___journal__Event\") @1791277..1791371 InvokedUnboundedly",
            "__lib_dna__core___types__words_in_both StringConcat @1923521..1923534 InUnboundedLoop",
            "__lib_dna__core___workspace__in_new_session StringConcat @2266584..2266877 InvokedUnboundedly",
            "__lib_dna__core___workspace__run_failed StructLit(\"std::process::ProcessOutput\") @2271893..2271972 InvokedUnboundedly",
            "__lib_dna__core___workspace__run_tool StructLit(\"__lib_dna__core___workspace__RunResult\") @2272605..2272673 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::exec StructLit(\"db::DbError\") @2354831..2354901 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::exec StructLit(\"db::ExecResult\") @2354782..2354903 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::exec StructLit(\"db::ExecResult\") @2354930..2355101 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::exec_params StructLit(\"db::DbError\") @2356312..2356382 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::exec_params StructLit(\"db::ExecResult\") @2356263..2356384 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::exec_params StructLit(\"db::ExecResult\") @2356411..2356498 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::query_one StructLit(\"db::DbError\") @2355266..2355336 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::query_one StructLit(\"db::DbError\") @2355433..2355513 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::query_one StructLit(\"db::Row\") @2355224..2355338 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::query_one StructLit(\"db::Row\") @2355391..2355515 InvokedUnboundedly",
            "__lib_dna__core__pond__pq___pq__PgConn::query_one StructLit(\"db::Row\") @2355706..2355739 InvokedUnboundedly",
            "__lib_dna__core__pond__realtime__nats___client__NatsClient::fail_with StructLit(\"__lib_dna__core__pond__realtime__nats___types__NatsError\") @3241636..3241676 InvokedUnboundedly",
            "__lib_dna__core__pond__realtime__nats___jetstream__js_error_of StringConcat @3259384..3259470 InvokedUnboundedly",
            "__lib_dna__host___connections__admission_refusal StringConcat @44294..44417 InvokedUnboundedly",
            "__lib_dna__host___connections__admission_refusal StringConcat @44475..44566 InvokedUnboundedly",
            "__lib_dna__host___connections__connection_state StringConcat @28251..28272 InvokedUnboundedly",
            "__lib_dna__host___connections__connection_state StringConcat @28315..28361 InvokedUnboundedly",
            "__lib_dna__host___connections__connection_state StringConcat @28376..28469 InvokedUnboundedly",
            "__lib_dna__host___connections__outbound_mailbox StructLit(\"dna::Head\") @21481..21511 InvokedUnboundedly",
            "__lib_dna__host___connections__peer_cache StringConcat @19972..20016 InvokedUnboundedly",
            "__lib_dna__host___connections__peer_identity StringConcat @20606..20639 InvokedUnboundedly",
            "__lib_dna__host___connections__peer_identity StructLit(\"__lib_dna__host___connections__PeerIdentity\") @20586..20641 InvokedUnboundedly",
            "__lib_dna__host___connections__peer_identity StructLit(\"__lib_dna__host___connections__PeerIdentity\") @20751..20778 InvokedUnboundedly",
            "__lib_dna__host___connections__peer_identity StructLit(\"__lib_dna__host___connections__PeerIdentity\") @20807..20832 InvokedUnboundedly",
            "__lib_dna__host___connections__peer_identity StructLit(\"__lib_dna__host___connections__PeerIdentity\") @20847..20921 InvokedUnboundedly",
            "__lib_dna__host___forge_github__FileForge::name StringConcat @64051..64078 InvokedUnboundedly",
            "__lib_dna__host___forge_github__FileForge::open_review StringConcat @64923..64949 InvokedUnboundedly",
            "__lib_dna__host___forge_github__FileForge::open_review StringConcat @65007..65025 InvokedUnboundedly",
            "__lib_dna__host___forge_github__FileForge::open_review StructLit(\"dna::ReviewRef\") @64901..64951 InvokedUnboundedly",
            "__lib_dna__host___forge_github__FileForge::open_review StructLit(\"dna::ReviewRef\") @64970..65027 InvokedUnboundedly",
            "__lib_dna__host___forge_github__GitHubForge::checks StringConcat @62652..62697 InvokedUnboundedly",
            "__lib_dna__host___forge_github__GitHubForge::name StringConcat @60275..60296 InvokedUnboundedly",
            "__lib_dna__host___forge_github__GitHubForge::open_review StringConcat @60676..60710 InvokedUnboundedly",
            "__lib_dna__host___forge_github__GitHubForge::open_review StructLit(\"dna::ReviewRef\") @60654..60712 InvokedUnboundedly",
            "__lib_dna__host___forge_github__GitHubForge::open_review StructLit(\"dna::ReviewRef\") @60731..60792 InvokedUnboundedly",
            "__lib_dna__host___main__parse_args StringConcat @250325..250332 InUnboundedLoop",
            "__lib_dna__host___main__parse_args StringConcat @250325..250338 InUnboundedLoop",
            "__lib_dna__host___main__parse_args StringConcat @250563..250581 InUnboundedLoop",
            "__lib_dna__host___main__parse_args StringConcat @250563..250585 InUnboundedLoop",
            "__lib_dna__host___memory__memory_migrate_verb StringConcat @254652..254681 InUnboundedLoop",
            "__lib_dna__host___memory__memory_migrate_verb StringConcat @254652..254712 InUnboundedLoop",
            "__lib_dna__host___memory__memory_migrate_verb StringConcat @254652..254718 InUnboundedLoop",
            "__lib_dna__host___memory__memory_migrate_verb StringConcat @254652..254746 InUnboundedLoop",
            "__lib_dna__host___memory__memory_migrate_verb StringConcat @254652..254753 InUnboundedLoop",
            "__lib_dna__host___projection__Projection::last_of StructLit(\"dna::Event\") @305815..305837 InvokedUnboundedly",
            "__lib_dna__host___projection__Projection::task_of StructLit(\"__lib_dna__host___projection__TaskRow\") @299255..299266 InvokedUnboundedly",
            "__lib_dna__host___record__append_as_at StringConcat @349541..349590 InvokedUnboundedly",
            "__lib_dna__host___record__receipt_read StructLit(\"__lib_dna__host___record__Receipt\") @339018..339029 InvokedUnboundedly",
            "__lib_dna__host___record__receipt_read StructLit(\"__lib_dna__host___record__Receipt\") @339044..339083 InvokedUnboundedly",
            "__lib_dna__host___record_verbs__design_document StringConcat @400527..401004 InvokedUnboundedly",
            "__lib_dna__host___record_verbs__design_terms StringConcat @400293..400327 InvokedUnboundedly",
            "__lib_dna__host___record_verbs__family_pending StringConcat @409221..409240 InUnboundedLoop",
            "__lib_dna__host___record_verbs__family_pending StringConcat @409221..409246 InUnboundedLoop",
            "__lib_dna__host___record_verbs__library_document StringConcat @404357..404735 InvokedUnboundedly",
            "__lib_dna__host___record_verbs__library_family_document StringConcat @405997..406206 InvokedUnboundedly",
            "__lib_dna__host___record_verbs__library_idea_row StructLit(\"__lib_dna__host___record_verbs__LibraryRow\") @405689..405775 InvokedUnboundedly",
            "__lib_dna__host___record_verbs__review_for_mutation StructLit(\"__lib_dna__host___projection__ReviewRow\") @375777..375790 InvokedUnboundedly",
            "__lib_dna__host___secrets__vault_put StringConcat @436286..436327 InvokedUnboundedly",
            "__lib_dna__host___secrets__vault_put StringConcat @436530..436552 InvokedUnboundedly",
            "__lib_dna__host___secrets__vault_put StringConcat @436724..436756 InvokedUnboundedly",
            "__lib_dna__host___verbs__answer StructLit(\"__lib_dna__host___verbs__Answer\") @464854..464885 InvokedUnboundedly",
            "__lib_dna__host___verbs__body_block StringConcat @482007..482073 InvokedUnboundedly",
            "__lib_dna__host___verbs__body_block StringConcat @482315..482364 InvokedUnboundedly",
            "__lib_dna__host___verbs__body_block StringConcat @482549..482627 InvokedUnboundedly",
            "__lib_dna__host___verbs__body_block StringConcat @482646..482765 InvokedUnboundedly",
            "__lib_dna__host___verbs__fail StructLit(\"__lib_dna__host___verbs__Answer\") @464771..464802 InvokedUnboundedly",
            "__lib_dna__host___verbs__magnitude_facets StringConcat @488117..488124 InUnboundedLoop",
            "__lib_dna__host___verbs__magnitude_facets StringConcat @488340..488394 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513235..513247 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513235..513258 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513355..513364 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513355..513371 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513394..513410 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513556..513571 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513556..513580 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513556..513627 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513556..513703 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513556..513714 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513556..513759 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513556..513766 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513556..513774 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513655..513683 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513655..513689 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513826..513841 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513826..513849 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513826..513897 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513826..514009 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513826..514020 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513826..514065 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513826..514072 InUnboundedLoop",
            "__lib_dna__host___verbs__model_map_verb StringConcat @513826..514085 InUnboundedLoop",
            "__lib_dna__host___verbs__no_record StringConcat @465952..466023 InvokedUnboundedly",
            "__lib_dna__host___verbs__record_refusal StringConcat @466283..466361 InvokedUnboundedly",
            "__lib_dna__host___verbs__review_targets StringConcat @490668..490726 InvokedUnboundedly",
            "__lib_dna__host___writers__RecordTaskWriterAdmission::append StringConcat @564660..564709 InvokedUnboundedly",
            "__lib_dna__host___writers__find_after StringConcat @563981..563993 InUnboundedLoop",
            "__lib_dna__host___writers__find_after StringConcat @563981..563999 InUnboundedLoop",
            "__lib_dna__host___writers__find_after StructLit(\"dna::Event\") @564051..564073 InvokedUnboundedly",
            "__lib_dna__operations___governance_admission__GovernanceJournal::append StructLit(\"dna::AppendResult\") @2517084..2517124 InvokedUnboundedly",
            "__lib_dna__operations___governance_admission__GovernanceJournal::read StructLit(\"dna::Event\") @2516778..2516792 InvokedUnboundedly",
            "__lib_dna__operations___graph__graph_node_id StringConcat @2661160..2661177 InvokedUnboundedly",
            "__lib_dna__operations___graph__graph_node_row StructLit(\"__lib_dna__operations___graph__GraphRow\") @2668580..2668633 InvokedUnboundedly",
            "__lib_dna__operations___graph__graph_refused StructLit(\"__lib_dna__operations___graph__GraphRow\") @2667705..2667726 InvokedUnboundedly",
            "__lib_dna__operations___graph__graph_retired_row StructLit(\"__lib_dna__operations___graph__GraphRow\") @2675100..2675145 InvokedUnboundedly",
            "__lib_dna__operations___graph__graph_retired_row StructLit(\"__lib_dna__operations___graph__GraphRow\") @2675720..2675765 InvokedUnboundedly",
            "__lib_dna__operations___graph_holes__hole_line StringConcat @2682828..2682845 InvokedUnboundedly",
            "__lib_dna__operations___person_retirement__RetirementPrefix::append StructLit(\"dna::AppendResult\") @3005088..3005128 InvokedUnboundedly",
            "__lib_dna__operations___person_retirement__RetirementPrefix::read StructLit(\"dna::Event\") @3004914..3004928 InvokedUnboundedly",
            "__lib_dna__operations___usage__usage_counts_text StringConcat @3135308..3135432 InvokedUnboundedly",
        ],
    );
    pinned(
        "dna/host",
        3859,
        &[
            "__lib_dna__core___batch__batch_filing",
            "__lib_dna__core___batch__batch_pending",
            "__lib_dna__core___batch__batch_summary",
            "__lib_dna__core___decision__decision_line",
            "__lib_dna__core___handoff__classes_allow",
            "__lib_dna__core___handoff__classes_normal",
            "__lib_dna__core___hat__effect_class_ok",
            "__lib_dna__core___memory_ledger__row_json",
            "__lib_dna__core___models__model_key_slot",
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
            "__lib_dna__operations___governance_policy__FixedAuthority::authorize",
            "__lib_dna__operations___knowledge_policy__FixedKnowledgeAuthority::authorize",
            "__lib_dna__operations___organization_policy__FixedOrganizationAuthority::authorize",
            "__lib_dna__operations___task_policy__FixedTaskAuthority::authorize",
        ],
        &[
            ("once-per-invocation -> ACCUMULATES-UNBOUNDED", 4),
            ("once-per-invocation -> per-iteration-reclaim", 25),
            ("per-iteration-reclaim -> ACCUMULATES-UNBOUNDED", 14),
        ],
        &[
            "__lib_dna__core___journal__MemJournal::read StructLit(\"__lib_dna__core___journal__Event\") @1095660..1095669 InvokedUnboundedly",
            "__lib_dna__core___memory_ledger__PqLedger::read StructLit(\"__lib_dna__core___journal__Event\") @1220713..1220722 InvokedUnboundedly",
            "__lib_dna__core___memory_schema__role_vault_name StringConcat @1247968..1248007 InvokedUnboundedly",
            "__lib_dna__core___memory_spine__MemoryLedger::read StructLit(\"__lib_dna__core___journal__Event\") @1294184..1294193 InvokedUnboundedly",
            "__lib_dna__core___models__digest_of StringConcat @1421668..1421732 InvokedUnboundedly",
            "__lib_dna__core___models__model_key_slot StringConcat @1427444..1427458 InvokedUnboundedly",
            "__lib_dna__core___nerves__nerves_vault_name StringConcat @1474623..1474691 InvokedUnboundedly",
            "__lib_dna__core___ownership__word_add StringConcat @1542820..1542837 InvokedUnboundedly",
            "__lib_dna__core___record__GitJournal::read StructLit(\"__lib_dna__core___journal__Event\") @1656507..1656516 InvokedUnboundedly",
            "__lib_dna__core___routing__RoutedJournal::read StructLit(\"__lib_dna__core___journal__Event\") @1774898..1774992 InvokedUnboundedly",
            "__lib_dna__core___workspace__body_mark StringConcat @2251962..2252027 InvokedUnboundedly",
            "__lib_dna__core___workspace__in_new_session StringConcat @2250205..2250498 InvokedUnboundedly",
            "__lib_dna__core___workspace__join_lines StringConcat @2251602..2251614 InvokedUnboundedly",
            "__lib_dna__core___workspace__run_failed StructLit(\"std::process::ProcessOutput\") @2255514..2255593 InvokedUnboundedly",
            "__lib_dna__core___workspace__run_tool StructLit(\"__lib_dna__core___workspace__RunResult\") @2256226..2256294 InvokedUnboundedly",
            "__lib_dna__operations___governance_admission__GovernanceJournal::read StructLit(\"dna::Event\") @2500399..2500413 InvokedUnboundedly",
            "__lib_dna__operations___person_retirement__RetirementPrefix::read StructLit(\"dna::Event\") @2988535..2988549 InvokedUnboundedly",
        ],
    );
    pinned(
        "dna/operations",
        2572,
        &[
            "__lib_dna__core___batch__batch_filing",
            "__lib_dna__core___batch__batch_pending",
            "__lib_dna__core___batch__batch_summary",
            "__lib_dna__core___hat__effect_class_ok",
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
            ("once-per-invocation -> per-iteration-reclaim", 8),
        ],
        &[
        ],
    );
    pinned(
        "dna/organization_runtime",
        2585,
        &[
            "__lib_dna__core___batch__batch_filing",
            "__lib_dna__core___batch__batch_pending",
            "__lib_dna__core___batch__batch_summary",
            "__lib_dna__core___hat__effect_class_ok",
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
            ("once-per-invocation -> per-iteration-reclaim", 8),
        ],
        &[
        ],
    );
    pinned(
        "dna/ui",
        1599,
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
