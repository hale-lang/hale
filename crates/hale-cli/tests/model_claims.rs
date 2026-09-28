//! The `model_claims` integration-test binary: 7 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "claims_artifact_unknowns.rs"]
mod claims_artifact_unknowns;
#[path = "doc_effects_catalogue.rs"]
mod doc_effects_catalogue;
#[path = "effects_manifest.rs"]
mod effects_manifest;
#[path = "law_selection_reaches_the_artifact.rs"]
mod law_selection_reaches_the_artifact;
#[path = "model_cli.rs"]
mod model_cli;
#[path = "model_diff.rs"]
mod model_diff;
#[path = "styleguide_claims.rs"]
mod styleguide_claims;
