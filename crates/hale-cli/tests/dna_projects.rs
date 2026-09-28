//! The `dna_projects` integration-test binary: 9 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "dna_apply.rs"]
mod dna_apply;
#[path = "dna_init.rs"]
mod dna_init;
#[path = "dna_long_path.rs"]
mod dna_long_path;
#[path = "dna_models.rs"]
mod dna_models;
#[path = "dna_new.rs"]
mod dna_new;
#[path = "dna_run.rs"]
mod dna_run;
#[path = "dna_status.rs"]
mod dna_status;
#[path = "dna_twelve_steps.rs"]
mod dna_twelve_steps;
#[path = "dna_ui.rs"]
mod dna_ui;
