//! F.32-1γ-v2 session 2 (2026-05-26) — ThreadSanitizer
//! validation for the lockfree `@form(hashmap)` path.
//!
//! Builds the headline cross-pool lockfree workload with
//! `LOTUS_TSAN=1` (the codegen-side hook that adds
//! `-fsanitize=thread` to clang). Asserts the resulting binary
//! exits clean and produces no `WARNING: ThreadSanitizer` lines
//! on stderr. Suppressions for pre-existing substrate races
//! (arena allocator, bus queue, shutdown) are embedded into the
//! C runtime via `__tsan_default_suppressions` and apply
//! automatically — any race in `lotus_hashmap_*_lockfree`
//! entry points is NOT suppressed, so a γ-v2 regression in the
//! hashmap state machine surfaces immediately.
//!
//! Marked `#[ignore]` so the default `cargo test` run skips it;
//! TSAN-instrumented binaries are 5-15× slower than release.
//! Opt in with:
//!
//!   LOTUS_TSAN=1 cargo test --release -p hale-codegen \
//!       --test form_hashmap_lockfree_tsan -- --ignored --test-threads=1
//!
//! The TSAN harness here covers the lockfree state machine;
//! exhaustive interleaving search via the relacy library is
//! deferred (separate C++ subproject, future session) — the
//! TSAN sweep catches the race patterns that occur in practice
//! under cooperative-scheduler workloads.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "lt-lockfree-tsan-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

fn build_and_run(tag: &str, src: &str) -> (String, String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path(tag);
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run binary");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    (stdout, stderr, out.status)
}

/// γ-v1 + γ-v2 session 1 lockfree code paths under TSAN. Skipped
/// unless `LOTUS_TSAN=1` is set in the environment (the env var
/// is the same hook that flips the codegen-side clang flag, so
/// running this test without it would produce a non-instrumented
/// binary and trivially pass).
#[test]
#[ignore]
fn lockfree_cross_pool_tsan_clean() {
    if std::env::var("LOTUS_TSAN").ok().as_deref() != Some("1") {
        panic!(
            "lockfree_cross_pool_tsan_clean requires LOTUS_TSAN=1 \
             in the environment; otherwise the binary is built \
             without ThreadSanitizer and the test trivially passes."
        );
    }
    // Two cooperative pools writing disjoint keys into one
    // shared lockfree map — the headline F.32-1γ workload.
    // Smaller batch size than the regular cross-pool test
    // (TSAN runs ~10× slower; 40k entries is enough to surface
    // races without bloating wall time).
    let src = r#"
        type Counter { id: Int; v: Int; }

        @form(hashmap, sync = lockfree, cap = 16000)
        locus Registry {
            capacity { pool entries of Counter indexed_by id; }
        }

        locus PoolHost {
            params { reg: Registry = Registry { }; }
            run() {
                let mut i = 0;
                while i < 5000 {
                    self.reg.set(Counter { id: i * 2 + 1, v: i });
                    i = i + 1;
                }
            }
        }

        main locus App {
            params { host: PoolHost = PoolHost { }; }
            placement { host: cooperative(pool = io); }
            run() {
                let mut i = 0;
                while i < 5000 {
                    self.host.reg.set(Counter { id: i * 2, v: i });
                    i = i + 1;
                }
                while self.host.reg.len() < 10000 {
                    std::time::sleep(1ms);
                }
                print("len="); println(self.host.reg.len());
            }
        }

        fn main() { App { }; }
    "#;
    let (stdout, stderr, status) = build_and_run("cross_pool", src);
    assert!(
        status.success(),
        "TSAN flagged a race (exit={:?}). stdout: {}\n--- TSAN stderr ---\n{}",
        status, stdout, stderr,
    );
    assert!(
        stdout.contains("len=10000"),
        "writer correctness regressed under TSAN: {:?}",
        stdout
    );
    assert!(
        !stderr.contains("WARNING: ThreadSanitizer"),
        "ThreadSanitizer surfaced an unsuppressed race in lockfree \
         path:\n{}",
        stderr,
    );
}

/// γ-v2 session 1 tombstone path under TSAN: set / remove /
/// set-same-key while a second pool churns disjoint keys.
/// Exercises the COMMITTED → TOMBSTONE CAS + the probe-past-
/// tombstone walk under concurrent writer pressure.
#[test]
#[ignore]
fn lockfree_remove_under_contention_tsan_clean() {
    if std::env::var("LOTUS_TSAN").ok().as_deref() != Some("1") {
        panic!(
            "lockfree_remove_under_contention_tsan_clean requires LOTUS_TSAN=1"
        );
    }
    let src = r#"
        type Counter { id: Int; v: Int; }

        @form(hashmap, sync = lockfree, cap = 8000)
        locus Registry {
            capacity { pool entries of Counter indexed_by id; }
        }

        locus PoolHost {
            params { reg: Registry = Registry { }; }
            run() {
                let mut i = 0;
                while i < 2000 {
                    self.reg.set(Counter { id: i * 2 + 1, v: i });
                    i = i + 1;
                }
            }
        }

        main locus App {
            params { host: PoolHost = PoolHost { }; }
            placement { host: cooperative(pool = io); }
            run() {
                // Set/remove/set churn on a disjoint key set
                // (even ids) while PoolHost writes odd ids.
                // Both pools hit the shared `Registry` map at
                // the same time — TSAN must see the lockfree
                // entry points cooperate cleanly via the
                // 4-state CAS machine.
                let mut i = 0;
                while i < 2000 {
                    self.host.reg.set(Counter { id: i * 2, v: i });
                    i = i + 1;
                }
                let mut j = 0;
                while j < 1000 {
                    self.host.reg.remove(j * 2) or raise;
                    j = j + 1;
                }
                let mut k = 0;
                while k < 1000 {
                    self.host.reg.set(Counter { id: k * 2, v: k + 9000 });
                    k = k + 1;
                }
                while self.host.reg.len() < 4000 {
                    std::time::sleep(1ms);
                }
                print("len="); println(self.host.reg.len());
            }
        }

        fn main() { App { }; }
    "#;
    let (stdout, stderr, status) = build_and_run("remove_churn", src);
    assert!(
        status.success(),
        "TSAN flagged a race in the remove/set churn workload \
         (exit={:?}). stdout: {}\n--- TSAN stderr ---\n{}",
        status, stdout, stderr,
    );
    assert!(
        stdout.contains("len=4000"),
        "remove/re-set churn correctness regressed under TSAN: {:?}",
        stdout
    );
    assert!(
        !stderr.contains("WARNING: ThreadSanitizer"),
        "ThreadSanitizer surfaced an unsuppressed race in the \
         tombstone path:\n{}",
        stderr,
    );
}
