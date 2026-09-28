//! The `dna_records` integration-test binary: 11 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "dna_github_membrane.rs"]
mod dna_github_membrane;
#[path = "dna_handoff_ledger.rs"]
mod dna_handoff_ledger;
#[path = "dna_knowledge.rs"]
mod dna_knowledge;
#[path = "dna_law.rs"]
mod dna_law;
#[path = "dna_ledger.rs"]
mod dna_ledger;
#[path = "dna_record.rs"]
mod dna_record;
#[path = "dna_record_sync.rs"]
mod dna_record_sync;
#[path = "dna_record_trust.rs"]
mod dna_record_trust;
#[path = "dna_recorded_fixture.rs"]
mod dna_recorded_fixture;
#[path = "dna_review.rs"]
mod dna_review;
#[path = "dna_task_done.rs"]
mod dna_task_done;
