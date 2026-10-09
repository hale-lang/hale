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
//! The per-seed tests pin, for each of the five DNA seeds the
//! correction touches, every manifest row it changed or added, with
//! its classes as the rows now report them, and the row counts before
//! and after: the renames-blind rows are the same producer over the
//! same bundle with its renames cleared, which is what the old summary
//! saw. `no_other_seed_changes` checks the other DNA seeds' manifests
//! are untouched. `the_replay_gate_refuses_what_it_refused` pins that
//! the correction moves no replay refusal: the classes the live-effects
//! gate refuses on (`syscall`, `ffi`, `unclassified`) are the same,
//! seed by seed, with the renames as without them.

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
        .filter(|p| {
            std::fs::read_dir(p)
                .unwrap()
                .any(|e| e.unwrap().path().extension().is_some_and(|x| x == "hl"))
        })
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
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

/// The five seeds pinned below are the only ones whose manifest the
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
    assert_eq!(corrected, ["api", "host", "operations", "organization_runtime", "organization_source"]);
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
            "core {syscall}",
            "host {syscall,unclassified}",
            "oidc {syscall}",
            "operations {syscall,unclassified}",
            "organism {syscall}",
            "organization_runtime {syscall,unclassified}",
            "organization_source {syscall}",
            "reflexes {syscall}",
            "ui {syscall}",
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
    pinned(
        "api",
        287,
        289,
        "
        Api::definition_draft_handle  does={syscall,block,env,alloc}
        Api::definitions_read  does={syscall,block,env,alloc}
        Api::handle  does={syscall,block,time,entropy,env,alloc,secret_use,journal_io}
        ContextWire::read  does={syscall,block,time,entropy,env,alloc,secret_use}
        KnowledgeWire::texts  does={alloc}
        LocalKnowledge::supported  does={env}
        launch_token  does={syscall,block,alloc}
        ",
    );
}

/// host: 90 of 456 rows.
#[test]
fn host() {
    pinned(
        "host",
        456,
        463,
        "
        GovernanceCli::review_profile  does={syscall,block,time,entropy,env,alloc,secret_use}
        Host::ack_reading  does={publish,alloc}
        Host::birth  does={env}
        Host::build_genome  does={syscall,block,env,alloc}
        Host::build_seed  does={syscall,block,env,alloc}
        Host::cut_artifact  does={syscall,block,alloc}
        Host::no_application  does={syscall,block,alloc}
        Host::sha12  does={syscall,block,alloc}
        Host::start_memory  does={syscall,block,time,entropy,env,alloc,secret_use}
        Host::start_organization_launch  does={syscall,block,time,env,alloc}
        Host::stop_started  does={syscall,block,alloc}
        Host::wait_nerves  does={syscall,block,time,entropy,env,alloc}
        LocalTransport::execute  does={syscall,block,alloc}
        LocalTransport::probe  does={syscall,block,alloc}
        Node::birth  does={env}
        Node::build  does={syscall,block,env,alloc}
        Projection::build_digest  does={syscall,block,alloc}
        Projection::review_of  does={syscall,block,time,entropy,env,alloc,secret_use}
        ReferenceInfrastructure::credential  does={syscall,block,env,alloc}
        ReferenceInfrastructure::install_unit  does={syscall,block,env,alloc}
        ReferenceInfrastructure::logs  does={syscall,block,env,alloc}
        ReferenceInfrastructure::nerves_server  does={syscall,block,alloc}
        ReferenceInfrastructure::probe  does={syscall,block,env,alloc}
        ReferenceInfrastructure::put_credential  does={syscall,block,env,alloc}
        ReferenceInfrastructure::restart_if_up  does={syscall,block,alloc}
        ReferenceInfrastructure::run_at  does={syscall,block,env,alloc}
        ReferenceInfrastructure::senses_store  does={syscall,block,alloc}
        ReferenceInfrastructure::start  does={syscall,block,env,alloc}
        ReferenceInfrastructure::status  does={syscall,block,env,alloc}
        ReferenceInfrastructure::stop  does={syscall,block,env,alloc}
        SshTransport::execute  does={syscall,block,alloc}
        SshTransport::probe  does={syscall,block,alloc}
        admits_here  does={syscall,block,time,env,alloc}
        alive  does={syscall,block,alloc}
        attached_application  does={syscall,block,alloc}
        base_of  does={syscall,block,time,entropy,env,alloc,secret_use}
        body_holder  does={syscall,block,env,alloc}
        build_cache_dir  does={syscall,block,env,alloc}
        build_cache_keep  does={syscall,block,alloc}
        build_cache_touch  does={syscall,block,alloc}
        build_fingerprint  does={syscall,block,env,alloc}
        build_seed_at  does={syscall,block,env,alloc}
        checkpoint_here  does={syscall,block,time,entropy,env,alloc,secret_use}
        definitions_verb  does={syscall,block,alloc}
        descendants  does={syscall,block,alloc}
        draw_secret  does={syscall,block,alloc}
        drop_include_line  does={syscall,block,alloc}
        edge_admits_here  does={syscall,block,time,env,alloc}
        exit_code  does={syscall,block,alloc}
        fence_stop  does={syscall,block,alloc}
        genome  does={syscall,block,alloc}
        genome_branch  does={syscall,block,alloc}
        genome_changed  does={syscall,block,alloc}
        genome_checkout  does={syscall,block,alloc}
        genome_commit  does={syscall,block,alloc}
        genome_default_branch  does={syscall,block,alloc}
        genome_diff  does={syscall,block,alloc}
        genome_fast_forward  does={syscall,block,alloc}
        genome_fetch_default  does={syscall,block,time,alloc}
        genome_push_branch  does={syscall,block,alloc}
        genome_reset  does={syscall,block,alloc}
        genome_show  does={syscall,block,alloc}
        held_machine  does={alloc}
        host_main  does={syscall,block,time,entropy,env,alloc,secret_use}
        include_names  does={syscall,block,alloc}
        input_digests  does={syscall,block,alloc}
        kill_tree  does={syscall,block,alloc}
        ledger_status  does={syscall,block,time,entropy,env,alloc,secret_use}
        memory_fence_for  does={syscall,block,time,entropy,env,alloc,secret_use}
        memory_migrate_verb  does={syscall,block,time,env,alloc,secret_use}
        model_key_vault  does={alloc}
        models_verb  does={syscall,block,alloc}
        nerves_drop_verb  does={syscall,block,time,entropy,env,alloc}
        nerves_listening  does={syscall,block,time,entropy,alloc}
        nerves_migrate  does={syscall,block,time,entropy,alloc}
        nerves_migrate_verb  does={syscall,block,time,entropy,env,alloc}
        organism_here  does={syscall,block,time,env,alloc}
        owner_of_person_here  does={syscall,block,time,env,alloc}
        owners_here  does={syscall,block,time,env,alloc}
        ownership_graph_here  does={syscall,block,time,env,alloc}
        ownership_note  does={syscall,block,time,env,alloc}
        plan_rel_of  does={syscall,block,env,alloc}
        project_dir_name  does={syscall,block,alloc}
        project_name  does={syscall,block,alloc}
        route_verb  does={syscall,block,time,env,alloc}
        seeded_listing  does={syscall,block,time,entropy,env,alloc,secret_use}
        senses_up_verb  does={syscall,block,alloc}
        show_verb  does={syscall,block,time,env,alloc}
        this_host  does={syscall,block,alloc}
        work_verb  does={syscall,block,alloc}
        ",
    );
}

/// operations: 23 of 452 rows.
#[test]
fn operations() {
    pinned(
        "operations",
        466,
        484,
        "
        AttemptCommandCodec::receipts_ok  does={alloc}
        DefinitionCodec::field_allowed  does={alloc}
        DefinitionDrafts::recapture  does={syscall,block,env,alloc}
        DefinitionDrafts::semantic  does={syscall,block,env,alloc}
        DefinitionDrafts::validate  does={syscall,block,env,alloc}
        Definitions::load  does={syscall,block,env,alloc}
        GraphIngest::ingest  does={syscall,block,alloc}
        MemoryProjection::ready  does={env}
        OrganizationImpact::relevant  does={alloc}
        OrganizationImpact::restriction  does={alloc}
        OrganizationImpact::wf  does={alloc}
        ProjectWorkflowCatalog::snapshot  does={syscall,block,env,alloc}
        TaskProjection::relevant  does={alloc}
        TaskProjection::terminal  does={alloc}
        attempt_words_within  does={alloc}
        graph_holder_ok  does={alloc}
        graph_ref_ok  does={alloc}
        hat_body  does={alloc}
        hat_digest  does={alloc}
        hat_json  does={alloc}
        hat_position_of  does={alloc}
        hold_person  does={alloc}
        hole_role_position  does={alloc}
        ingest_ext  does={alloc}
        ",
    );
}

/// organization_runtime: 4 of 24 rows.
#[test]
fn organization_runtime() {
    pinned(
        "organization_runtime",
        24,
        25,
        "
        HostAuthority::routing  does={syscall,block,time,entropy,env,alloc,secret_use}
        OrganizationLaunchSource::file_digest  does={syscall,block,alloc}
        OrganizationRuntime::birth  does={env}
        organization_exit_code  does={syscall,block,alloc}
        ",
    );
}

/// organization_source: 4 of 48 rows.
#[test]
fn organization_source() {
    pinned(
        "organization_source",
        48,
        49,
        "
        OrganizationPublicationInputs::birth  does={env,alloc}
        OrganizationSource::birth  does={syscall,block,env,alloc}
        OrganizationSource::remove_scratch  does={syscall,block,alloc}
        OrganizationSource::scratch  does={syscall,block,alloc}
        ",
    );
}
