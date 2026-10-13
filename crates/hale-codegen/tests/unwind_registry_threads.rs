//! The unwind registry is main's alone (`locus/unwind.rs`; review of
//! #1533). In a program with an assertion outside `fn main`'s frame the
//! compiler emits the registry's upkeep in every deferred entry's
//! teardown — on whatever thread runs it. A pinned worker's helper used
//! to remove its local's slot by scanning the registry while main grew
//! it: a data race, and a read of a freed array after main's `realloc`,
//! even though every assertion passed. Every entry point now returns at
//! once off main.
//!
//! The shape: a pinned worker and main both build and tear down a
//! `let`-bound local in a helper, many times, while a passing assertion
//! in a helper keeps the upkeep on. Built under ThreadSanitizer and
//! AddressSanitizer, each run clean, with the program's own result.

use std::process::{Command, Output};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const BOTH_THREADS_CHURN: &str = r#"
    locus Child {
        params { n: Int = 0; }
        dissolve() { }
    }
    fn local_child(i: Int) -> Int {
        let c = Child { n: i };
        return c.n;
    }
    fn check(n: Int) { std::test::assert(n >= 0, "never fails"); }
    locus Worker {
        run() {
            let mut i = 0;
            let mut sum = 0;
            while i < 20000 { sum = sum + local_child(i); i = i + 1; }
            check(sum);
        }
    }
    main locus App {
        params { w: Worker = Worker { }; }
        placement { w: pinned; }
        run() {
            let mut i = 0;
            let mut sum = 0;
            while i < 20000 { sum = sum + local_child(i); i = i + 1; }
            check(sum);
            println("done");
        }
    }
    fn main() { App { }; }
"#;

fn run(tag: &str, options: hale_codegen::BuildOptions) -> Output {
    let bin = harness::unique_bin(tag);
    build_opts::build_source(BOTH_THREADS_CHURN, &bin, &options).expect("build");
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .env("TSAN_OPTIONS", "halt_on_error=1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    out
}

fn assert_clean(tag: &str, out: &Output) {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    for bad in ["ThreadSanitizer", "AddressSanitizer", "LeakSanitizer", "SUMMARY:"] {
        assert!(!stderr.contains(bad), "{tag}: {bad} in:\n{stderr}");
    }
    assert!(out.status.success(), "{tag}: {:?}\n{stdout}\n{stderr}", out.status);
    assert_eq!(stdout.trim(), "done", "{tag}: {stdout}");
}

#[test]
fn a_worker_tearing_down_locals_never_touches_mains_registry_under_tsan() {
    for round in 0..3 {
        let out = run("unwind_threads_tsan", hale_codegen::BuildOptions { tsan: true, ..build_opts::options() });
        assert_clean(&format!("tsan round {round}"), &out);
    }
}

#[test]
fn a_worker_tearing_down_locals_never_touches_mains_registry_under_asan() {
    for round in 0..3 {
        let out = run("unwind_threads_asan", hale_codegen::BuildOptions { asan: true, ..build_opts::options() });
        assert_clean(&format!("asan round {round}"), &out);
    }
}
