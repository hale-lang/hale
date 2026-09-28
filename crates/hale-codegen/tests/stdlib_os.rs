//! The `stdlib_os` integration-test binary: 8 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "std_process_exit.rs"]
mod std_process_exit;
#[path = "std_process_rss.rs"]
mod std_process_rss;
#[path = "stdlib_env.rs"]
mod stdlib_env;
#[path = "stdlib_fs.rs"]
mod stdlib_fs;
#[path = "stdlib_list_dir.rs"]
mod stdlib_list_dir;
#[path = "stdlib_locus.rs"]
mod stdlib_locus;
#[path = "stdlib_log.rs"]
mod stdlib_log;
#[path = "stdlib_log_sinks.rs"]
mod stdlib_log_sinks;
