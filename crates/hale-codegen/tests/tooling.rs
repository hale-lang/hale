//! The `tooling` integration-test binary: 6 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "effects_conformance.rs"]
mod effects_conformance;
#[path = "effects_corpus_conformance.rs"]
mod effects_corpus_conformance;
#[path = "hale_self_test.rs"]
mod hale_self_test;
#[path = "lto_modes.rs"]
mod lto_modes;
#[path = "test_assert_granularity.rs"]
mod test_assert_granularity;
#[path = "vocabulary_gates.rs"]
mod vocabulary_gates;
