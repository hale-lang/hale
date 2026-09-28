//! The `dna_orgs` integration-test binary: 7 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "dna_body_lease.rs"]
mod dna_body_lease;
#[path = "dna_body_provision.rs"]
mod dna_body_provision;
#[path = "dna_design.rs"]
mod dna_design;
#[path = "dna_effect_resolve.rs"]
mod dna_effect_resolve;
#[path = "dna_fleet.rs"]
mod dna_fleet;
#[path = "dna_nerves.rs"]
mod dna_nerves;
#[path = "dna_org_evolves.rs"]
mod dna_org_evolves;
