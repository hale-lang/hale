//! The `os_files` integration-test binary: 7 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "compress_tar.rs"]
mod compress_tar;
#[path = "file_locus.rs"]
mod file_locus;
#[path = "fs.rs"]
mod fs;
#[path = "fs_index_and_status.rs"]
mod fs_index_and_status;
#[path = "fs_rename_unlink_mktemp.rs"]
mod fs_rename_unlink_mktemp;
#[path = "log_routing.rs"]
mod log_routing;
#[path = "read_file_proc.rs"]
mod read_file_proc;
