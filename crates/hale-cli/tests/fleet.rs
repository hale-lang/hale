//! The `fleet` integration-test binary: 6 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "fleet_compose.rs"]
mod fleet_compose;
#[path = "deploy_render.rs"]
mod deploy_render;
#[path = "fleet_cross_tier.rs"]
mod fleet_cross_tier;
#[path = "fleet_model_soundness.rs"]
mod fleet_model_soundness;
#[path = "fleet_locus.rs"]
mod fleet_locus;
#[path = "fleet_sign.rs"]
mod fleet_sign;
