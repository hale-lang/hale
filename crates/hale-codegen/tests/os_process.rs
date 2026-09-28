//! The `os_process` integration-test binary: 9 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "alloc_model_rss.rs"]
mod alloc_model_rss;
#[path = "dump_pool_residency.rs"]
mod dump_pool_residency;
#[path = "no_chunk_pool_knob.rs"]
mod no_chunk_pool_knob;
#[path = "panic_atexit.rs"]
mod panic_atexit;
#[path = "process_child.rs"]
mod process_child;
#[path = "process_child_adopt.rs"]
mod process_child_adopt;
#[path = "process_run.rs"]
mod process_run;
#[path = "process_try_wait.rs"]
mod process_try_wait;
#[path = "stack_budget_premises.rs"]
mod stack_budget_premises;
