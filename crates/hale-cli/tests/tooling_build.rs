//! The `tooling_build` integration-test binary: 16 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "bench.rs"]
mod bench;
#[path = "build_output_path.rs"]
mod build_output_path;
#[path = "debug_info.rs"]
mod debug_info;
#[path = "doc.rs"]
mod doc;
#[path = "ffi_export_reentry.rs"]
mod ffi_export_reentry;
#[path = "fmt.rs"]
mod fmt;
#[path = "hale_test_runner.rs"]
mod hale_test_runner;
#[path = "init.rs"]
mod init;
#[path = "pkg_fetch.rs"]
mod pkg_fetch;
#[path = "replay_cli.rs"]
mod replay_cli;
#[path = "run_dir_resolves_imports.rs"]
mod run_dir_resolves_imports;
#[path = "run_reports_signal.rs"]
mod run_reports_signal;
#[path = "target_model.rs"]
mod target_model;
#[path = "test_ffi_pickup.rs"]
mod test_ffi_pickup;
#[path = "verify.rs"]
mod verify;
#[path = "wasm_package_csrc.rs"]
mod wasm_package_csrc;
