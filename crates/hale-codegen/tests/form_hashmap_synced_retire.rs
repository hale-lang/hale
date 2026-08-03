//! `@form(hashmap, sync = serialized)` clone-on-read + retirement
//! (2026-08-03, downstream handoff P1).
//!
//! Until now a synced map never installed a retire descriptor, so
//! replaced String clones accumulated in its arena for the life of
//! the process. The reason was not oversight: a synced map's `get`
//! memcpy's the cell, so its String fields come out as raw pointers
//! into the MAP's arena, and a reader on another pool can hold one
//! across the writer's activation boundaries. **The leak was the
//! safety mechanism** — nothing was ever freed, so nothing dangled.
//!
//! The fix makes the reader own its copy: every read path on a
//! synced String-bearing map (`get`, `entry_at`, and `for` iteration)
//! clones the cell's Strings into the CALLER's arena, inside the same
//! critical section that read the cell. With no off-thread reader of
//! the map's blobs, the writer can retire exactly like `sync = none`
//! and the shipped flush is sound — no epoch scheme needed.
//!
//! Covering all three read paths is load-bearing, not thoroughness:
//! enabling the writer's retirement while leaving ANY read path
//! handing out raw cell pointers would convert the old leak into a
//! use-after-free, which is strictly worse than what it replaced.
//! These tests exist mostly to be run under the ASan corpus oracle,
//! where that failure is loud.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;

use hale_codegen::build_executable;

fn run_src(name: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(name);
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "binary exited {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// A value read out of a synced map stays intact across later
/// activations that churn the same key. Without clone-on-get the
/// held pointer is a retired blob that a subsequent set may already
/// have recycled.
#[test]
fn value_read_from_a_synced_map_survives_later_churn() {
    const SRC: &str = r#"
        type Bal { acct: String; amt: String; }

        @form(hashmap, sync = serialized)
        locus Table { capacity { pool entries of Bal indexed_by acct; } }

        main locus App {
            params { t: Table = Table { }; seen: String = ""; }

            fn seed() {
                self.t.set(Bal { acct: "a" + "1", amt: "original-value" });
            }
            fn read_one() {
                let r = self.t.get("a1") or { return; };
                self.seen = r.amt;
            }
            fn churn(i: Int) {
                self.t.set(Bal { acct: "a" + "1", amt: "replacement-" + to_string(i) });
            }

            run() {
                self.seed();
                self.read_one();
                let mut i = 0;
                while i < 500 { self.churn(i); i = i + 1; }
                print("held="); println(self.seen);
                let cur = self.t.get("a1") or { return; };
                print("cur="); println(cur.amt);
            }
        }
        fn main() { App { }; }
    "#;
    let out = run_src("synced_retire_survives", SRC);
    assert!(
        out.contains("held=original-value"),
        "the earlier read must still hold its own copy after 500 \
         replacements of the same key:\n{}",
        out
    );
    assert!(
        out.contains("cur=replacement-499"),
        "the map must still hold the latest write:\n{}",
        out
    );
}

/// Iteration is the third read path. A `for` over a synced map
/// batches VALUES out of the slots; those cells' Strings need the
/// same clone, or an iteration overlapping a set reads freed bytes.
#[test]
fn iterating_a_synced_map_copies_its_strings() {
    const SRC: &str = r#"
        type Bal { acct: String; amt: String; }

        @form(hashmap, sync = serialized)
        locus Table { capacity { pool entries of Bal indexed_by acct; } }

        main locus App {
            params { t: Table = Table { }; total: Int = 0; }

            fn fill(i: Int) {
                self.t.set(Bal {
                    acct: "k" + to_string(i),
                    amt: "amount-" + to_string(i),
                });
            }
            fn sweep() {
                let mut n = 0;
                for e in self.t.entries {
                    if e.amt != "" { n = n + 1; }
                }
                self.total = n;
            }

            run() {
                let mut i = 0;
                while i < 64 { self.fill(i); i = i + 1; }
                let mut r = 0;
                while r < 20 { self.sweep(); r = r + 1; }
                print("count="); println(self.total);
            }
        }
        fn main() { App { }; }
    "#;
    let out = run_src("synced_retire_iter", SRC);
    assert!(
        out.contains("count=64"),
        "every cell's cloned String must be readable:\n{}",
        out
    );
}

/// The shape that forces `sync = serialized` in the first place: two
/// pools writing one map, with the owner reading it. Exercises the
/// retire push (under the map mutex) against the flush (at each
/// writer's own activation boundary, no map lock held) — the race
/// the arena's `retire_lock` exists to serialize.
#[test]
fn cross_pool_churn_with_reads_stays_consistent() {
    const SRC: &str = r#"
        type Bal { acct: String; amt: String; }

        @form(hashmap, sync = serialized)
        locus Table { capacity { pool entries of Bal indexed_by acct; } }

        locus Writer {
            params { t: Table = Table { }; }
            fn put(i: Int) {
                self.t.set(Bal {
                    acct: "k" + to_string(i % 32),
                    amt: "written-" + to_string(i),
                });
            }
            run() {
                let mut i = 0;
                while i < 4000 { self.put(i); i = i + 1; }
                println("writer done");
            }
        }

        main locus App {
            params { w: Writer = Writer { }; hits: Int = 0; }
            placement { w: cooperative(pool = io); }

            fn probe(i: Int) {
                let r = self.w.t.get("k" + to_string(i % 32)) or { return; };
                if r.amt != "" { self.hits = self.hits + 1; }
            }
            run() {
                let mut i = 0;
                while i < 4000 { self.probe(i); i = i + 1; }
                std::time::sleep(200ms);
                print("hits_positive="); println(self.hits > 0);
            }
        }
        fn main() { App { }; }
    "#;
    let out = run_src("synced_retire_crosspool", SRC);
    assert!(
        out.contains("hits_positive=true"),
        "cross-pool reads must return live cloned values:\n{}",
        out
    );
    assert!(
        out.contains("writer done"),
        "the io-pool writer must finish:\n{}",
        out
    );
}
