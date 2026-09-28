//! The `tooling_services` integration-test binary: 10 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "api_description.rs"]
mod api_description;
#[path = "dispatch_plan_cli.rs"]
mod dispatch_plan_cli;
#[path = "embedded_dna_provenance.rs"]
mod embedded_dna_provenance;
#[path = "iris_cli.rs"]
mod iris_cli;
#[path = "iris_seeds_check.rs"]
mod iris_seeds_check;
#[path = "lsp.rs"]
mod lsp;
#[path = "obs_entity_ids.rs"]
mod obs_entity_ids;
#[path = "obs_model_hash.rs"]
mod obs_model_hash;
#[path = "observe_session_lifetime.rs"]
mod observe_session_lifetime;
#[path = "stale_dna_warning.rs"]
mod stale_dna_warning;
