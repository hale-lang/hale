//! A classified correction, pinned (F.40 phase 3, E1 3 of 6).
//!
//! The effects manifest (`--dump-effects-manifest`,
//! `--check-effects-manifest`) and replay's live-effects gate used to
//! build their own summary with `summarize_with_stdlib`, which ignores
//! the bundle's import renames. A call into an imported seed was an
//! unresolved edge the walk did not follow, so a fn's row reported
//! only what its own seed did: a fail-open, the manifest
//! under-reporting what a seed does through the seeds it imports
//! (`GovernanceCli::review_profile` reported `{alloc}` where it
//! performs syscalls, blocking, time, entropy, environment and secret
//! use through imported code). Both now read the snapshot's effect
//! rows, whose walk resolves cross-seed calls through the renames.
//!
//! The per-seed tests pin, for each of the seven DNA seeds the
//! correction touches, every manifest row it changed or added, with
//! its classes as the rows now report them, and the row counts before
//! and after: the renames-blind rows are the same producer over the
//! same bundle with its renames cleared, which is what the old summary
//! saw. `no_other_seed_changes` checks the other DNA seeds' manifests
//! are untouched. `the_replay_gate_refuses_what_it_refused` pins that
//! the correction moves no replay refusal: the classes the live-effects
//! gate refuses on (`syscall`, `ffi`, `unclassified`) are the same,
//! seed by seed, with the renames as without them.

//! Current pins include the shared default/lifecycle coverage of #1327 and
//! indexed-receiver resolution. An `unclassified` upper bound preserves a
//! real execution gap; the separate lower-bound control below keeps known
//! imported effects visible. Historical count notes beside older cases
//! explain their earlier pins, rather than promise those old counts.
//!
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;
use hale_types::effect_rows::{derive_effect_rows, EffectRows};

fn dna() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dna").canonicalize().unwrap()
}

/// The DNA seeds: every directory of `dna/` holding a `.hl` file, but
/// the test files.
fn seeds() -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dna())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir() && !p.ends_with("tests"))
        .filter(|p| hale_corpus::holds_hl(p))
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    // the host's verb program, a seed inside the host's (GH #1490)
    out.push("host/verb".to_string());
    out.sort();
    out
}

/// Run `f` over the seed's snapshot, loaded as `hale check <dir>`
/// loads it, with its effect rows and the renames-blind rows beside
/// them. On a thread of its own: a whole DNA seed's walk is deep.
fn over_seed<T: Send>(seed: &str, f: impl FnOnce(&Snapshot, &EffectRows, &EffectRows) -> T + Send) -> T {
    let dir = dna().join(seed);
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn_scoped(s, move || {
                let Ok(snap) = Snapshot::load(&dir, LoadMode::WholeSeed, &Disk, Config::check(true, false)) else {
                    panic!("dna/{seed} does not load");
                };
                let Ok(rows) = snap.demand_effects() else { panic!("dna/{seed}: the effect rows are blocked") };
                let Ok(top) = snap.demand_scope() else { panic!("dna/{seed}: the scope is blocked") };
                let mut blind = snap.bundle();
                blind.import_renames.clear();
                let blind_summary = hale_types::alloc_summary::derive_alloc_summary(&blind);
                let blind_rows = derive_effect_rows(&blind, top, std::sync::Arc::new(blind_summary));
                f(&snap, rows, &blind_rows)
            })
            .unwrap()
            .join()
            .unwrap()
    })
}

/// The manifest's rows (the header line dropped).
fn manifest_rows(snap: &Snapshot, rows: &EffectRows) -> Vec<String> {
    hale_types::dump_effects_manifest(&snap.bundle(), rows)
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Per seed: (rows before, rows now, every row the correction changed
/// or added, as it reads now).
fn correction(seed: &str) -> (usize, usize, Vec<String>) {
    over_seed(seed, |snap, rows, blind| {
        let before = manifest_rows(snap, blind);
        let now = manifest_rows(snap, rows);
        let old: BTreeSet<&String> = before.iter().collect();
        let changed = now.iter().filter(|r| !old.contains(r)).cloned().collect();
        (before.len(), now.len(), changed)
    })
}

fn pinned(seed: &str, rows_before: usize, rows_now: usize, expected: &str) {
    let (before, now, changed) = correction(seed);
    let expected: Vec<&str> = expected.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    assert_eq!(
        changed, expected,
        "dna/{seed}: the rows the cross-seed correction changes are pinned; a difference here is \
         a change to what the manifest reports, and is classified before the pin moves"
    );
    assert_eq!((before, now), (rows_before, rows_now), "dna/{seed}: the manifest's row counts");
}

/// The nine seeds pinned below are the only ones whose manifest the
/// correction changes.
#[test]
fn no_other_seed_changes() {
    let mut corrected = Vec::new();
    for seed in seeds() {
        let (_, _, changed) = correction(&seed);
        if !changed.is_empty() {
            corrected.push(seed);
        }
    }
    assert_eq!(corrected, ["api", "core", "host", "host/verb", "operations", "organization_runtime", "organization_source", "reflexes", "ui"]);
}

/// Incomplete implicit execution may saturate an upper bound, but must
/// not erase the imported effects already established by the shared walk.
#[test]
fn known_cross_seed_effects_survive_incomplete_execution_coverage() {
    for (seed, function, expected) in [
        ("api", "Api::handle", vec!["syscall", "block", "time", "entropy", "env", "alloc", "secret_use", "stdio"]),
        ("host", "GovernanceCli::review_profile", vec!["syscall", "block", "time", "entropy", "env", "alloc", "secret_use", "stdio"]),
        ("core", "TaskRun::birth", vec!["publish", "alloc"]),
    ] {
        let (known, blind) = over_seed(seed, |_, rows, blind| {
            let classes = |rows: &EffectRows| {
                let row = rows.rows.values().find(|r| r.key.display() == function).expect("the function's source row");
                hale_types::frontier::render_effects_named(row.known, rows.classes.names()).into_iter().collect::<BTreeSet<_>>()
            };
            (classes(rows), classes(blind))
        });
        for effect in expected {
            assert!(known.contains(effect), "{seed}: {function} lost known {effect}: {known:?}");
        }
        if seed == "host" {
            assert!(!blind.contains("syscall"), "the renames-blind host control must miss the imported syscall");
        }
    }
}

/// The live-effects gate refuses on the same classes, seed by seed,
/// before and after the correction.
#[test]
fn the_replay_gate_refuses_what_it_refused() {
    let residue = |snap: &Snapshot, rows: &EffectRows| -> BTreeSet<String> {
        let bundle = snap.bundle();
        let programs: Vec<&hale_syntax::ast::Program> = bundle.programs.values().copied().collect();
        hale_types::effects::effect_manifest_with_inference(&programs, rows)
            .into_iter()
            .flat_map(|r| r.inferred)
            .filter(|c| c == "syscall" || c == "ffi" || c == "unclassified")
            .collect()
    };
    let mut refused = Vec::new();
    for seed in seeds() {
        let (now, before) = over_seed(&seed, |snap, rows, blind| (residue(snap, rows), residue(snap, blind)));
        assert_eq!(now, before, "dna/{seed}: the correction moved a replay refusal");
        refused.push(format!("{seed} {{{}}}", now.into_iter().collect::<Vec<_>>().join(",")));
    }
    assert_eq!(
        refused,
        [
            "api {syscall,unclassified}",
            "core {syscall,unclassified}",
            "host {syscall,unclassified}",
            "host/verb {syscall,unclassified}",
            "oidc {syscall}",
            "operations {syscall,unclassified}",
            "organism {syscall,unclassified}",
            "organization_runtime {syscall,unclassified}",
            "organization_source {syscall,unclassified}",
            "reflexes {syscall,unclassified}",
            "ui {syscall,unclassified}",
        ]
    );
}

/// api: 6 of 287 rows. Since the head serves a surface (R3) the
/// program-alone rows carry the surface's generated receiver methods
/// (`Commands::birth`, `::dissolve`, `::__rpc_call_1`, `::__rpc_ev`,
/// `::__rpc_hello_1`) and the rows adapter's `check`, plus `held_elsewhere`;
/// the own-rows projection closing `decl_index` up restored the six
/// generated ones the shifted index had dropped. The cross-seed rows are
/// unchanged. The row total fell from 361 (363) to 287 (289) when the
/// cutover (R4) removed the bus subscriptions from `dna/api`: the 13
/// command topics and the 8 Knowledge topics, each a handler method
/// with the fns only it reached, so the manifest lost 74 rows. None of
/// the six cross-seed rows listed is among them.
#[test]
fn api() {
    // GH #1530 step 3: the two exposures' eaters of `preview_batch`
    // (`__rpc_row_<n>_12`, generated per row of the served surface) call it,
    // so they carry what it does; every row's eater, decoder and encoder
    // is a manifest row of its own (56 more on each side).
    // The faces index row (`Commands::index`) shifts the served rows'
    // numbering by one (`__rpc_row_1_13`, `__rpc_row_2_13`) and adds 13 rows on
    // each side: the row's eater, decoder and encoder and the fns of
    // `record_index.hl`, none of them a row the correction changes.
    // GH #1554: two more rows on both sides, `{alloc}` from the stores
    // into a child locus's field from outside its methods (`self.reader.root
    // = self.root`), which lowering copies (GH #1561) and the survey now
    // sees through the checker's types.
    // Phase 4: the habitat's three reads (`Commands::allocations`, `::hosts`,
    // `::policy`) sort before `preview_batch`, so its eaters are row 16 now,
    // and add 26 rows on each side: each row's eater, decoder and encoder,
    // its admission and handler, and the fns of `habitat_rows.hl`, none of
    // them a row the correction changes. The nurseries' and saplings' four
    // rows (`add_nursery`, `nurseries`, `ratify_sapling`, `saplings`) add 31
    // more; `add_nursery` and `nurseries` sort before `preview_batch`, which
    // is row 18 now. The hosts' rows (`add_host` sorting before it) made it
    // row 19. The brief (`Commands::brief`, `habitat_brief.hl`) sorts before
    // it too, and the head serves the surface over a third transport
    // (`mcp::Rpc`), so each served row has a third eater (`__rpc_row_3_20`).
    // The json codec's encoders write through one string builder (a
    // record's text linear in its size), whose calls the survey reads as
    // pure: two encoders of records whose fields are all non-string
    // scalars did nothing else, and leave the manifest on both sides.
    pinned(
        "api",
        665,
        667,
        "
        CommandAdmission::admissible  does={alloc}
        CommandAdmission::preview_batch  does={unclassified}
        Commands::__rpc_row_1_28  does={unclassified}
        Commands::__rpc_row_2_28  does={unclassified}
        Commands::__rpc_row_3_28  does={unclassified}
        Commands::preview_batch  does={unclassified}
        DefinitionDraftWire::guard  does={alloc}
        DefinitionDraftWire::parse  does={alloc}
        JsonShape::valid  does={alloc}
        KnowledgeWire::texts  does={alloc}
        LocalKnowledge::supported  does={env}
        LocalKnowledgeCommands::admit  does={unclassified}
        LocalKnowledgeCommands::birth  does={unclassified}
        LocalKnowledgeCommands::lookup  does={unclassified}
        LocalKnowledgeCommands::submit  does={unclassified}
        OrganizationCandidateWire::ledger_head  does={unclassified}
        OrganizationCandidateWire::mask  does={unclassified}
        OrganizationCandidateWire::source_objects  does={unclassified}
        OrganizationCandidateWire::visible  does={unclassified}
        OrganizationDraftWire::same_base  does={alloc}
        OrganizationImpactWire::candidate_visible  does={unclassified}
        WorkflowRead::usage  does={unclassified}
        applied_at  does={syscall,env,alloc}
        command_guard  does={alloc}
        habitat_allocate_forge  does={syscall,block,env,alloc}
        habitat_graft  does={syscall,block,env,alloc}
        habitat_graft_log  does={syscall,env,alloc}
        habitat_host_add  does={unclassified}
        habitat_host_plan_remove  does={unclassified}
        habitat_host_probe  does={unclassified}
        habitat_journal  does={unclassified}
        habitat_journal_by  does={unclassified}
        habitat_organism_plan_remove  does={unclassified}
        habitat_organism_remove  does={unclassified}
        host_removal_refusal  does={unclassified}
        host_reply  does={unclassified}
        host_write  does={unclassified}
        knowledge_policy  does={unclassified}
        launch_token  does={syscall,block,alloc,stdio}
        local_person  does={unclassified}
        observe_row  does={syscall,block,alloc}
        organism_to_remove  does={unclassified}
        solo_done  does={unclassified}
        solo_read  does={unclassified}
        solo_write  does={unclassified}
        upstream_header  does={alloc}
        ",
    );
}

/// host/verb, the host's verb program (GH #1490): its one row, `main`,
/// whose every verb is an imported call.
#[test]
fn host_verb() {
    pinned(
        "host/verb",
        1,
        1,
        "
        main  does={unclassified}
        ",
    );
}

/// host: 114 of 517 rows.
#[test]
fn host() {
    // GH #1554: the renames-blind side gains a row, `{alloc}` from a copy
    // its walk, which resolves no imported type, cannot rule out.
    // Phase 4: `placement.hl` adds eight rows on each side; two of them,
    // `habitat_address` and `habitat_apply_verb`, reach imported code (the
    // core's `run_tool` and `ComposeProvider`), so the correction changes them.
    pinned(
        "host",
        568,
        575,
        "
        GitHubForge::checks  does={syscall,block,env,alloc,secret_use}
        GitHubForge::close_review  does={syscall,block,env,alloc,secret_use}
        GitHubForge::comment  does={syscall,block,env,alloc,secret_use}
        GitHubForge::open_review  does={syscall,block,env,alloc,secret_use}
        GitHubForge::review_body  does={syscall,block,env,alloc,secret_use}
        GitHubForge::review_comments  does={syscall,block,env,alloc,secret_use}
        GitHubForge::review_state  does={syscall,block,env,alloc,secret_use}
        GitHubForge::verdicts  does={syscall,block,env,alloc,secret_use}
        GovernanceCli::answered  does={unclassified}
        GovernanceCli::lookup  does={unclassified}
        GovernanceCli::review_profile  does={unclassified}
        GovernanceCli::settings  does={unclassified}
        Host::ack_reading  does={publish,alloc}
        Host::build_genome  does={syscall,block,env,alloc,stdio}
        Host::build_seed  does={syscall,block,env,alloc,stdio}
        Host::cut_artifact  does={syscall,block,alloc}
        Host::no_application  does={syscall,block,alloc}
        Host::say_nerves_read  does={syscall,env,alloc,stdio}
        Host::sha12  does={syscall,block,alloc}
        Host::start_organization_launch  does={unclassified}
        Host::stop_started  does={syscall,block,alloc,stdio}
        LocalTransport::execute  does={syscall,block,alloc}
        LocalTransport::probe  does={syscall,block,alloc}
        NatsExchange::birth  does={env,alloc}
        Node::begin  does={unclassified}
        Node::build  does={syscall,block,env,alloc,stdio}
        NodeRelay::publish_fact  does={unclassified}
        Projection::build_digest  does={syscall,block,alloc}
        Projection::review_of  does={unclassified}
        ReferenceInfrastructure::allocated  does={syscall,env,alloc}
        ReferenceInfrastructure::allocation  does={syscall,block,alloc}
        ReferenceInfrastructure::credential  does={syscall,block,env,alloc}
        ReferenceInfrastructure::install_unit  does={syscall,block,env,alloc}
        ReferenceInfrastructure::knowledge_database_if_up  does={syscall,block,env,alloc}
        ReferenceInfrastructure::logs  does={syscall,block,env,alloc}
        ReferenceInfrastructure::nerves_if_up  does={syscall,block,env,alloc}
        ReferenceInfrastructure::nerves_server  does={unclassified}
        ReferenceInfrastructure::probe  does={syscall,block,env,alloc}
        ReferenceInfrastructure::published  does={syscall,block,env,alloc}
        ReferenceInfrastructure::put_credential  does={syscall,block,env,alloc}
        ReferenceInfrastructure::restart_if_up  does={syscall,block,env,alloc}
        ReferenceInfrastructure::run_at  does={syscall,block,env,alloc}
        ReferenceInfrastructure::senses_if_up  does={syscall,block,env,alloc}
        ReferenceInfrastructure::senses_store  does={unclassified}
        ReferenceInfrastructure::start  does={syscall,block,env,alloc}
        ReferenceInfrastructure::status  does={syscall,block,env,alloc}
        ReferenceInfrastructure::stop  does={syscall,block,env,alloc}
        RepositoryCompose::check  does={syscall,block,alloc}
        RepositoryCompose::down  does={syscall,block,alloc}
        RepositoryCompose::own  does={syscall,block,alloc}
        RepositoryCompose::services  does={syscall,block,alloc}
        RepositoryCompose::up  does={syscall,block,time,alloc}
        SshTransport::execute  does={syscall,block,alloc}
        SshTransport::probe  does={syscall,block,alloc}
        admits_here  does={unclassified}
        alive  does={syscall,block,alloc}
        allocation_files  does={syscall,block,env,alloc}
        apply_drops  does={unclassified}
        attached_application  does={unclassified}
        base_of  does={unclassified}
        body_holder  does={syscall,block,env,alloc}
        body_key  does={syscall,block,alloc}
        body_target  does={unclassified}
        build_cache_dir  does={syscall,block,env,alloc}
        build_cache_keep  does={syscall,block,alloc}
        build_cache_touch  does={syscall,block,alloc}
        build_fingerprint  does={syscall,block,env,alloc}
        build_seed_at  does={syscall,block,env,alloc}
        compose_key  does={alloc}
        compose_sweep  does={syscall,block,alloc}
        definitions_verb  does={syscall,block,alloc}
        descendants  does={syscall,block,alloc}
        draw_secret  does={syscall,block,alloc}
        drop_here  does={syscall,block,alloc}
        drop_include_line  does={syscall,block,alloc}
        edge_admits_here  does={unclassified}
        exchange_key  does={syscall,alloc}
        exchange_migrate  does={unclassified}
        exit_code  does={syscall,block,alloc}
        fence_compose_down  does={syscall,block,alloc}
        fence_stop  does={syscall,block,alloc}
        fill_check  does={unclassified}
        fill_equipment  does={unclassified}
        fleet_export  does={syscall,block,env,alloc}
        fleet_export_at  does={syscall,block,env,alloc}
        fleet_rel_of  does={syscall,block,env,alloc}
        forge_kind  does={syscall,env,alloc}
        forge_name  does={syscall,env,alloc}
        forge_token_slot  does={syscall,env,alloc}
        forgejo_exec  does={syscall,block,env,alloc}
        forgejo_healthy  does={syscall,block,time,env,alloc}
        forgejo_push  does={syscall,block,env,alloc,secret_use}
        forgejo_up  does={syscall,block,time,env,alloc,secret_use}
        forgejo_user  does={syscall,block,time,env,alloc,secret_use}
        forget_allocation_items  does={unclassified}
        forget_newest_row  does={unclassified}
        forget_organism_slots  does={unclassified}
        forget_vault_item  does={unclassified}
        forget_volume  does={env,alloc}
        genome  does={syscall,block,alloc}
        genome_branch  does={syscall,block,alloc}
        genome_candidate  does={syscall,block,alloc}
        genome_candidates  does={syscall,block,alloc}
        genome_changed  does={syscall,block,alloc}
        genome_checkout  does={syscall,block,alloc}
        genome_commit  does={syscall,block,alloc}
        genome_default_branch  does={syscall,block,alloc}
        genome_diff  does={syscall,block,alloc}
        genome_fast_forward  does={syscall,block,alloc}
        genome_fetch_commit  does={syscall,block,alloc}
        genome_fetch_default  does={syscall,block,time,alloc}
        genome_push_branch  does={syscall,block,alloc}
        genome_push_url  does={syscall,block,env,alloc,secret_use}
        genome_remote  does={unclassified}
        genome_reset  does={syscall,block,alloc}
        genome_root  does={syscall,block,alloc}
        genome_show  does={syscall,block,alloc}
        genome_uncandidate  does={syscall,block,alloc}
        gh  does={syscall,block,env,alloc,secret_use}
        git_config  does={unclassified}
        git_config_all  does={unclassified}
        habitat_address  does={syscall,block,alloc}
        habitat_apply_verb  does={unclassified}
        habitat_forget_plan_verb  does={unclassified}
        habitat_teardown_run_verb  does={unclassified}
        heart_key  does={syscall,env,alloc}
        held_state  does={unclassified}
        include_names  does={syscall,block,env,alloc}
        input_digests  does={syscall,block,alloc}
        kill_faces  does={syscall,block,alloc}
        kill_tree  does={syscall,block,alloc}
        model_key_vault  does={alloc}
        models_verb  does={syscall,block,alloc}
        nats_conf_path  does={syscall,block,env,alloc}
        nats_secrets_path  does={syscall,block,env,alloc}
        nerves_listening  does={unclassified}
        nerves_migrate  does={unclassified}
        nerves_restart_hint  does={syscall,env,alloc}
        nerves_urls  does={unclassified}
        nursery_sources_verb  does={unclassified}
        oidc_issuer  does={syscall,block,alloc}
        organism_here  does={syscall,block,time,env,alloc}
        organism_secrets  does={unclassified}
        organizations_in_record  does={unclassified}
        own_nerves  does={syscall,block,env,alloc}
        owner_of_person_here  does={unclassified}
        owners_here  does={unclassified}
        ownership_graph_here  does={unclassified}
        ownership_note  does={unclassified}
        postgres_secrets_path  does={syscall,block,env,alloc}
        practice_candidates  does={unclassified}
        profile_name  does={syscall,block,time,env,alloc}
        profile_verb  does={unclassified}
        project_dir_name  does={syscall,block,alloc}
        project_name  does={syscall,block,alloc}
        provision_script  does={alloc}
        rec  does={unclassified}
        record_append  does={unclassified}
        record_append_batch  does={unclassified}
        record_body_verb  does={unclassified}
        record_identity  does={syscall,alloc}
        record_identity_verb  does={unclassified}
        review_knowledge  does={unclassified}
        revoke_app_secret  does={syscall,block,env,alloc}
        secret_present  does={syscall,block,env,alloc}
        seeded_listing  does={unclassified}
        slot_vault  does={unclassified}
        sync_record  does={unclassified}
        this_host  does={syscall,block,alloc}
        under_node  does={unclassified}
        vault_put  does={syscall,block,env,alloc,secret_use}
        work_verb  does={unclassified}
        write_app_secret  does={syscall,block,env,alloc,secret_use}
        write_include_secret  does={syscall,block,env,alloc,secret_use}
        write_nerves_secrets  does={syscall,block,env,alloc,secret_use}
        write_postgres_secrets  does={syscall,block,env,alloc,secret_use}
        ",
    );
}

/// operations: 23 of 452 rows.
#[test]
fn operations() {
    // GH #1554: the renames-blind side gains `ingest_ext  does={alloc}`
    // (a copy its walk, which resolves no imported type, cannot rule
    // out), so the row is no longer one the correction changes.
    pinned(
        "operations",
        517,
        536,
        "
        AttemptCommandCodec::classes_ok  does={alloc}
        AttemptCommandCodec::grant  does={alloc}
        AttemptCommandCodec::graph_position  does={alloc}
        AttemptCommandCodec::hat_ok  does={alloc}
        AttemptCommandCodec::receipts_ok  does={alloc}
        AttemptCommandCodec::valid  does={alloc}
        AuthorityPolicyCodec::fields  does={alloc}
        AuthorityPolicyCodec::text  does={alloc}
        DefinitionCodec::decode_exact  does={alloc}
        DefinitionCodec::decoded  does={alloc}
        DefinitionCodec::field_allowed  does={alloc}
        DefinitionDraftCodec::append  does={alloc}
        DefinitionDraftCodec::basis_equal  does={alloc}
        DefinitionDraftCodec::candidate_text_bytes  does={alloc}
        DefinitionDraftCodec::catalog_text_bytes  does={alloc}
        DefinitionDraftCodec::fields  does={alloc}
        DefinitionDraftCodec::integer  does={alloc}
        DefinitionDraftCodec::is_text  does={alloc}
        DefinitionDraftCodec::step_stores  does={alloc}
        DefinitionDraftCodec::text  does={alloc}
        DefinitionDraftCodec::text_bytes  does={alloc}
        DefinitionDraftCodec::texts  does={alloc}
        FixedAuthority::authorize  does={alloc}
        FixedKnowledgeAuthority::authorize  does={alloc}
        FixedOrganizationAuthority::authorize  does={alloc}
        GovernanceCommandCodec::bounded  does={alloc}
        GovernanceCommandCodec::context_valid  does={alloc}
        GovernanceCommandCodec::string  does={alloc}
        GovernanceCommandCodec::valid  does={alloc}
        GovernanceCommands::command_valid  does={alloc}
        GovernanceCommands::file_receipts  does={unclassified}
        GovernanceCommands::grant_valid  does={alloc}
        GovernanceCommands::supported  does={alloc}
        GovernanceProjection::texts  does={alloc}
        GraphIngest::list_files  does={syscall,block,alloc}
        KnowledgeBindingCommandCodec::valid  does={alloc}
        KnowledgeCommandCodec::event_kind  does={alloc}
        KnowledgeCommandCodec::grant_valid  does={alloc}
        KnowledgeNodeAdmission::scope  does={alloc}
        KnowledgeNodeCommandCodec::valid  does={alloc}
        KnowledgePolicyCodec::scopes  does={alloc}
        MemoryProjection::birth  does={syscall,block,env,alloc}
        MemoryProjection::ready  does={env}
        OrganizationCommandCodec::valid  does={alloc}
        OrganizationImpact::relevant  does={alloc}
        OrganizationImpact::restriction  does={alloc}
        OrganizationImpact::string  does={alloc}
        OrganizationImpact::wf  does={alloc}
        OrganizationProjection::magnitude  does={alloc}
        PersonRetirementCodec::valid  does={alloc}
        StructureCommandCodec::grant  does={alloc}
        StructureCommandCodec::grant_ok  does={alloc}
        StructureCommandCodec::seat  does={alloc}
        StructureCommandCodec::valid  does={alloc}
        TaskCommandCodec::valid  does={alloc}
        TaskCreateCommandCodec::grant  does={alloc}
        TaskCreateCommandCodec::valid  does={alloc}
        TaskProjection::relevant  does={alloc}
        TaskProjection::strings  does={alloc}
        TaskProjection::terminal  does={alloc}
        TaskProjection::text  does={alloc}
        application_asset  does={syscall,block,alloc}
        application_assets  does={syscall,block,alloc}
        attempt_ledger_for  does={unclassified}
        attempt_words_ok  does={alloc}
        attempt_words_within  does={alloc}
        graph_holder_ok  does={alloc}
        graph_ref_ok  does={alloc}
        hat_body  does={unclassified}
        hat_digest  does={unclassified}
        hat_json  does={unclassified}
        hat_position_of  does={alloc}
        held_machine  does={alloc}
        hold_person  does={alloc}
        hole_role_position  does={alloc}
        idea_of  does={unclassified}
        reader_over  does={alloc}
        receipts_over  does={alloc}
        record_trust_of  does={unclassified}
        under_node_of  does={unclassified}
        verdict_standing  does={unclassified}
        ",
    );
}

/// organization_runtime: 4 of 24 rows.
#[test]
fn organization_runtime() {
    pinned(
        "organization_runtime",
        23,
        23,
        "
        HostAuthority::lease_key  does={unclassified}
        HostAuthority::owner  does={unclassified}
        HostAuthority::record_head  does={unclassified}
        HostAuthority::row_verified  does={unclassified}
        HostAuthority::shared  does={unclassified}
        HostAuthority::trust  does={unclassified}
        OrganizationLaunchSource::file_digest  does={syscall,block,alloc}
        organization_exit_code  does={syscall,block,alloc}
        ",
    );
}

/// organization_source: 4 of 48 rows.
#[test]
fn organization_source() {
    // GH #1554: as `host`'s, the renames-blind side gains a row.
    pinned(
        "organization_source",
        49,
        49,
        "
        OrganizationInspection::execute  does={syscall,block,time,alloc}
        OrganizationInspection::input_error  does={syscall,block,time,alloc}
        OrganizationInspection::source_ok  does={syscall,block,time,alloc}
        OrganizationInspection::tree  does={syscall,block,time,alloc}
        OrganizationPublicationInputs::available  does={alloc}
        OrganizationPublicationInputs::basis  does={env,alloc}
        OrganizationPublicationInputs::birth  does={unclassified}
        OrganizationPublicationInputs::owner  does={env,alloc}
        OrganizationPublicationInputs::relative  does={alloc}
        OrganizationPublicationInputs::request_valid  does={unclassified}
        OrganizationPublicationInputs::safe_metadata  does={syscall,block,time,alloc}
        OrganizationPublicationInputs::source  does={alloc}
        OrganizationSource::birth  does={unclassified}
        OrganizationSource::declared  does={unclassified}
        OrganizationSource::head  does={unclassified}
        OrganizationSource::local_vendor  does={syscall,block,time,alloc}
        OrganizationSource::remove_scratch  does={syscall,block,alloc}
        OrganizationSource::scratch  does={syscall,block,alloc}
        ",
    );
}

/// Indexed receiver resolution exposes these additional cross-seed allocations.
#[test]
fn core() {
    pinned(
        "core",
        1795,
        1795,
        "
        AnthropicMessages::allows  does={syscall,block,env,alloc}
        Dna::organization_source_owner  does={alloc}
        ExchangeKey::public  does={syscall,block,env,alloc,secret_use}
        ExchangeKey::ready  does={syscall,block,env,alloc}
        ForgejoApi::call  does={syscall,block,env,alloc,secret_use}
        ForgejoForge::call  does={syscall,block,env,alloc,secret_use}
        ForgejoForge::checks  does={syscall,block,env,alloc,secret_use}
        ForgejoForge::review_body  does={syscall,block,env,alloc,secret_use}
        ForgejoForge::review_comments  does={syscall,block,env,alloc,secret_use}
        ForgejoForge::review_state  does={syscall,block,env,alloc,secret_use}
        ForgejoForge::verdicts  does={syscall,block,env,alloc,secret_use}
        GraphSelection::offer  does={alloc}
        HostedCredential::post_json  does={syscall,block,env,alloc,secret_use,external_model}
        HostedCredential::ready  does={syscall,block,env,alloc}
        ModelRouter::in_place  does={syscall,block,env,alloc}
        ModelRouter::price_micros  does={syscall,block,env,alloc}
        ModelRouter::standing  does={syscall,block,env,alloc}
        OpenAiChat::allows  does={syscall,block,env,alloc}
        OrganizationLaunchCodec::valid  does={alloc}
        OrganizationSourceRequestCodec::valid  does={alloc}
        RecordedModel::allows  does={syscall,block,env,alloc}
        SourceEditor::in_place  does={syscall,block,env,alloc}
        StepRecord::walked  does={alloc}
        TaskRun::birth  does={publish,alloc}
        TaskRun::start  does={publish,alloc}
        WorkflowCatalog::decode_bound  does={alloc}
        WorkflowCatalog::expand  does={alloc}
        WorkflowCatalog::refuse  does={alloc}
        WorkflowProjection::admitted  does={alloc}
        WorkflowProjection::apply  does={alloc}
        baseline_definitions  does={alloc}
        board_requirement  does={alloc}
        cosine  does={alloc}
        definitions_listing  does={alloc}
        derived_dsns  does={syscall,block,env,alloc}
        embed_text  does={alloc}
        forgejo_held  does={syscall,block,env,alloc,secret_use}
        graph_route  does={alloc}
        habitat_key_ensure  does={syscall,block,env,alloc}
        habitat_key_file  does={syscall,block,env,alloc,secret_use}
        habitat_public_key  does={syscall,block,env,alloc,secret_use}
        host_holdings  does={syscall,block,env,alloc,secret_use}
        kept_spine_dsn  does={syscall,block,env,alloc}
        nerves_missing  does={syscall,block,env,alloc}
        pq_conn  does={unclassified}
        provision_record_roles  does={syscall,block,env,alloc,secret_use}
        provision_role  does={syscall,block,env,alloc,secret_use}
        record_key  does={alloc}
        replay  none={syscall}  does={alloc}
        role_password  does={syscall,block,env,alloc,secret_use}
        role_sql  does={syscall,block,env,alloc,secret_use}
        route_held_by  does={alloc}
        route_holders  does={alloc}
        route_requirement  does={alloc}
        signers_held_now  does={alloc}
        ssh_probe  does={syscall,block,env,alloc,secret_use}
        ssh_run  does={syscall,block,env,alloc,secret_use}
        vault_provision_local  does={syscall,block,env,alloc,secret_use}
        ",
    );
}

#[test]
fn reflexes() {
    pinned("reflexes", 9, 9, "
        main  does={unclassified}
        ");
}

#[test]
fn ui() {
    pinned("ui", 27, 27, "
        CodeExchange::id_token  does={syscall,block,env,alloc,secret_use}
        CodeExchange::ready  does={syscall,block,env,alloc}
        Ui::finish_sign_in  does={syscall,block,time,entropy,env,alloc,secret_use}
        Ui::handle  does={syscall,block,time,entropy,env,alloc,secret_use}
        ");
}
