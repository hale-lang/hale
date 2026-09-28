//! What a test builds with.
//!
//! `BuildOptions` has no default for the runtime-object cache: the caller
//! chooses where it lives. A test's choice is its checkout's own
//! `CARGO_TARGET_TMPDIR` (Cargo sets it for integration tests: a
//! directory under `target/`, per checkout, never the system temp dir and
//! never shared between users). The cache is content-addressed and its
//! objects are written by a unique temp name and renamed into place, so
//! every test process of the run can share it safely, and it stays warm
//! from run to run; a directory of each test's own would recompile the
//! runtime's C (about 3 s) in every one of the suite's 1,600 test
//! processes.
//!
//! Start from [`options`] and set what a test needs on top of it:
//! `BuildOptions { asan: true, ..build::options() }`.

use std::path::PathBuf;

use hale_codegen::BuildOptions;

/// The directory a test caches compiled runtime objects in.
pub fn cache_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("hale-runtime-cache")
}

/// The default build options for a test, caching in [`cache_dir`].
pub fn options() -> BuildOptions {
    BuildOptions::new(cache_dir())
}
