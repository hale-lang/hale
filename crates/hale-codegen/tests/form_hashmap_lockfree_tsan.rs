//! F.32-1γ-v2 session 2 (2026-05-26) — ThreadSanitizer
//! validation for the lockfree `@form(hashmap)` path.
//!
//! Builds the headline cross-pool lockfree workload with
//! `LOTUS_TSAN=1` (`support/sanitize.rs` turns it into
//! `BuildOptions::tsan`, which adds `-fsanitize=thread` to clang). Asserts the resulting binary
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


#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/sanitize.rs"]
mod sanitize;

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = harness::unique_bin(&format!(
        "lt-lockfree-tsan-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

fn build_and_run(tag: &str, src: &str) -> (String, String, std::process::ExitStatus) {
    let bin = unique_path(tag);
    build_opts::build_source(src, &bin, &sanitize::options()).expect("build");
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

/// γ-v2 session 3 grow path under TSAN. Two pools insert disjoint
/// keys past the initial cap, forcing several grows during the
/// concurrent-write window. The lf_enter/lf_exit + grow_phase
/// protocol must serialize the migration cleanly with no
/// unsuppressed race reports.
#[test]
#[ignore]
fn lockfree_grow_under_contention_tsan_clean() {
    if std::env::var("LOTUS_TSAN").ok().as_deref() != Some("1") {
        panic!(
            "lockfree_grow_under_contention_tsan_clean requires LOTUS_TSAN=1"
        );
    }
    let src = r#"
        type Counter { id: Int; v: Int; }

        // Small initial cap so the cross-pool writers force
        // several grows during execution.
        @form(hashmap, sync = lockfree, cap = 32)
        locus Registry {
            capacity { pool entries of Counter indexed_by id; }
        }

        locus PoolHost {
            params { reg: Registry = Registry { }; }
            run() {
                let mut i = 0;
                while i < 1000 {
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
                while i < 1000 {
                    self.host.reg.set(Counter { id: i * 2, v: i });
                    i = i + 1;
                }
                while self.host.reg.len() < 2000 {
                    std::time::sleep(1ms);
                }
                print("len="); println(self.host.reg.len());
            }
        }

        fn main() { App { }; }
    "#;
    let (stdout, stderr, status) = build_and_run("grow_under_contention", src);
    assert!(
        status.success(),
        "TSAN flagged a race during the grow path \
         (exit={:?}). stdout: {}\n--- TSAN stderr ---\n{}",
        status, stdout, stderr,
    );
    assert!(
        stdout.contains("len=2000"),
        "writer correctness regressed under grow + TSAN: {:?}",
        stdout
    );
    assert!(
        !stderr.contains("WARNING: ThreadSanitizer"),
        "ThreadSanitizer surfaced an unsuppressed race in the grow \
         protocol:\n{}",
        stderr,
    );
}

/// String rows, replaced concurrently from two pools on a `sync =
/// lockfree` and on a `sync = striped` map: the first replaces publish the
/// map arena's String slot state (GH #1490's size classes, which a
/// replaced cell's Strings join) while the other pool's cell clone reads
/// it before its set claims a cell. Each pool first inserts its own keys,
/// then both replace them with Strings of varying length, so the first
/// replacement of each pool races the other's. The publication and every
/// read of it are atomic; an unsynchronized one is a ThreadSanitizer
/// report here.
fn string_rows_replaced_from_two_pools(sync: &str) {
    if std::env::var("LOTUS_TSAN").ok().as_deref() != Some("1") {
        panic!("requires LOTUS_TSAN=1 in the environment; otherwise the binary is built without ThreadSanitizer");
    }
    let src = format!(
        r#"
        type Row {{ id: Int; name: String = ""; note: String = ""; }}

        @form(hashmap, sync = {sync}, cap = 4096)
        locus Registry {{
            capacity {{ pool entries of Row indexed_by id; }}
        }}

        locus PoolHost {{
            params {{ reg: Registry = Registry {{ }}; done: Bool = false; }}
            run() {{
                let mut round = 0;
                while round < 20 {{
                    let mut i = 0;
                    while i < 200 {{
                        self.reg.set(Row {{ id: i * 2 + 1, name: std::str::repeat("ab", (i + round) % 23), note: "odd " + to_string(round) }});
                        i = i + 1;
                    }}
                    round = round + 1;
                }}
                self.done = true;
            }}
        }}

        main locus App {{
            params {{ host: PoolHost = PoolHost {{ }}; }}
            placement {{ host: cooperative(pool = io); }}
            run() {{
                let mut round = 0;
                while round < 20 {{
                    let mut i = 0;
                    while i < 200 {{
                        self.host.reg.set(Row {{ id: i * 2, name: std::str::repeat("cd", (i + round) % 19), note: "even " + to_string(round) }});
                        i = i + 1;
                    }}
                    round = round + 1;
                }}
                while self.host.reg.len() < 400 {{
                    std::time::sleep(1ms);
                }}
                let r = self.host.reg.get(0) or Row {{ id: -1 }};
                print("len="); println(self.host.reg.len());
                print("row0="); println(r.note);
            }}
        }}

        fn main() {{ App {{ }}; }}
    "#
    );
    let (stdout, stderr, status) = build_and_run(&format!("string_rows_{sync}"), &src);
    assert!(status.success(), "exit={status:?}. stdout: {stdout}\n--- TSAN stderr ---\n{stderr}");
    assert!(stdout.contains("len=400") && stdout.contains("row0=even 19"), "{stdout}");
    assert!(!stderr.contains("WARNING: ThreadSanitizer"), "ThreadSanitizer surfaced a race:\n{stderr}");
}

#[test]
#[ignore]
fn lockfree_string_rows_replaced_from_two_pools_tsan_clean() {
    string_rows_replaced_from_two_pools("lockfree");
}

#[test]
#[ignore]
fn striped_string_rows_replaced_from_two_pools_tsan_clean() {
    string_rows_replaced_from_two_pools("striped");
}

/// The bus registry hands a dissolved subscriber's entry to the next
/// registration into its bucket, rewriting it under a sequence lock while
/// another pool's publishes walk the table and the bucket. A pool churns
/// scope-local subscribers (Int-keyed, String-keyed and unkeyed) while a
/// second pool publishes to all of them, with cells queued for each
/// subscriber as it dissolves: the walks must read each entry whole or
/// skip it.
#[test]
#[ignore]
fn bus_register_dissolve_churn_under_publishes_tsan_clean() {
    if std::env::var("LOTUS_TSAN").ok().as_deref() != Some("1") {
        panic!("requires LOTUS_TSAN=1 in the environment; otherwise the binary is built without ThreadSanitizer");
    }
    let src = r#"
        type Ev { lane: Int; n: Int; }
        type Memo { who: String; n: Int; }
        topic Lanes { payload: Ev; subject: "churn.lanes"; keyed_by lane; }
        topic Memos { payload: Memo; subject: "churn.memos"; keyed_by who; }
        locus Conn {
            params { lane: Int = 0; who: String = ""; got: Int = 0; }
            bus {
                subscribe Lanes as on_ev where key == self.lane;
                subscribe Memos as on_memo where key == self.who;
            }
            fn on_ev(e: Ev) { self.got = self.got + 1; }
            fn on_memo(m: Memo) { self.got = self.got + 1; }
        }
        locus Ear {
            params { got: Int = 0; }
            bus { subscribe Lanes as on_ev; }
            fn on_ev(e: Ev) { self.got = self.got + 1; }
        }
        locus Pub {
            params { x: Int = 0; }
            bus { publish Lanes; publish Memos; }
            run() {
                let mut i = 0;
                while i < 40000 {
                    Lanes <- Ev { lane: i % 4, n: i };
                    Memos <- Memo { who: "c" + to_string(i % 4), n: i };
                    i = i + 1;
                }
                println("published");
            }
        }
        locus Churn {
            params { x: Int = 0; }
            bus { publish Lanes; }
            fn once(i: Int) {
                let c = Conn { lane: i % 4, who: "c" + to_string(i % 4) };
                let a = Ear { };
                Lanes <- Ev { lane: i % 4, n: i };
            }
            run() {
                let mut i = 0;
                while i < 10000 {
                    self.once(i);
                    i = i + 1;
                }
                println("churned");
            }
        }
        main locus App {
            params { p: Pub = Pub { }; c: Churn = Churn { }; e: Ear = Ear { }; }
            placement { p: cooperative(pool = pa); c: cooperative(pool = pb) where async_io; }
        }
        fn main() { App { }; }
    "#;
    let (stdout, stderr, status) = build_and_run("bus_churn", src);
    assert!(status.success(), "exit={status:?}. stdout: {stdout}\n--- TSAN stderr ---\n{stderr}");
    assert!(stdout.contains("published") && stdout.contains("churned"), "{stdout}");
    assert!(!stderr.contains("WARNING: ThreadSanitizer"), "ThreadSanitizer surfaced a race:\n{stderr}");
}
