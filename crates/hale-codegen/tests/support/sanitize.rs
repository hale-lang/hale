//! The sanitizer a test run asks for, from the environment the CI job
//! sets (`LOTUS_ASAN`, `LOTUS_TSAN`, `LOTUS_UBSAN`).
//!
//! Codegen reads no environment variable: a sanitizer is a
//! `BuildOptions` field, and the CLI is what turns `LOTUS_ASAN=1` into
//! it. A test that is meant to be run under a sanitizer job builds with
//! [`options`], which does the same turning for the test process, so
//! `LOTUS_ASAN=1 cargo test ...` still means what it says. It only
//! READS the environment; no test mutates it.

use hale_codegen::BuildOptions;

#[path = "build.rs"]
mod build_opts;

fn on(name: &str) -> bool {
    std::env::var(name).map(|v| v == "1" || v == "true" || v == "TRUE").unwrap_or(false)
}

/// `BuildOptions` for the sanitizer the environment names, if any.
pub fn options() -> BuildOptions {
    BuildOptions {
        asan: on("LOTUS_ASAN"),
        tsan: on("LOTUS_TSAN"),
        ubsan: on("LOTUS_UBSAN"),
        ..build_opts::options()
    }
}
