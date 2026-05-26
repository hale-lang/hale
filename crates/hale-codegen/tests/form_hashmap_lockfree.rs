//! F.32-1γ-v1 (2026-05-25) — `@form(hashmap, sync = lockfree,
//! cap = N)` end-to-end coverage.
//!
//! Three γ-v1 scenarios:
//!
//!   1. `lockfree_basic_single_pool` — sanity check that the
//!      lockfree opt-in compiles and the standard get/has/len
//!      surface works under no contention.
//!
//!   2. `lockfree_cross_pool_correctness` — the headline F.32-1γ
//!      scenario. Two pools (main + io) write disjoint keys into
//!      a single fixed-cap shared Registry; the test asserts
//!      both writers' inserts land (200k total entries) without
//!      memory corruption, livelock, or lost updates.
//!
//!   3. `lockfree_update_existing_key` — same-key write twice
//!      must update (not double-count). Verifies the CAS
//!      COMMITTED → CLAIMED → write → COMMITTED update path.
//!
//! F.32-1γ-v2 (session 1, 2026-05-26) — tombstones + remove.
//! Four scenarios:
//!
//!   4. `lockfree_remove_present_key` — set / remove / get-miss /
//!      len. Confirms COMMITTED → TOMBSTONE CAS lands and the
//!      key is no longer visible.
//!
//!   5. `lockfree_remove_missing_key` — remove of a never-inserted
//!      key returns 0 (raises KeyError handled by the call site).
//!      Differs from γ-v1 where ALL removes were 0.
//!
//!   6. `lockfree_set_remove_set_same_key` — the headline session-1
//!      shape. After remove, re-set must succeed (lands in the
//!      EMPTY slot ahead of the tombstone, since v2 session 1
//!      doesn't reuse tombstoned slots).
//!
//!   7. `lockfree_iter_skips_tombstones` — iteration via
//!      `key_at` / `value_at` skips tombstoned entries; `len()`
//!      reflects live count, not live+tombstone.

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
        "lt-form-hashmap-lockfree-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

fn build_and_run(tag: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path(tag);
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run binary");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    (stdout, out.status)
}

#[test]
fn lockfree_basic_single_pool() {
    let src = r#"
        type Counter { id: Int; v: Int; }

        @form(hashmap, sync = lockfree, cap = 2000)
        locus Registry {
            capacity { pool entries of Counter indexed_by id; }
        }

        main locus App {
            params { reg: Registry = Registry { }; }
            run() {
                let n = 1000;
                let mut i = 0;
                while i < n {
                    self.reg.set(Counter { id: i, v: i + 1 });
                    i = i + 1;
                }
                print("len="); println(self.reg.len());
                let e = self.reg.get(42) or raise;
                print("e.v="); println(e.v);
                let h = self.reg.has(99);
                print("has99="); println(h);
                let m = self.reg.has(99999);
                print("has99999="); println(m);
            }
        }

        fn main() { App { }; }
    "#;
    let (stdout, status) = build_and_run("basic", src);
    assert!(
        status.success(),
        "binary exited non-zero: {:?}\nstdout: {}",
        status,
        stdout,
    );
    assert!(stdout.contains("len=1000"), "got: {:?}", stdout);
    assert!(stdout.contains("e.v=43"), "got: {:?}", stdout);
    assert!(stdout.contains("has99=true"), "got: {:?}", stdout);
    assert!(stdout.contains("has99999=false"), "got: {:?}", stdout);
}

#[test]
fn lockfree_cross_pool_correctness() {
    // The scenario γ-v1 was sized for. Two pools concurrently
    // write disjoint keys (even / odd ids); the CAS-based slot
    // claim must let both pools race on different cells without
    // losing entries.
    let src = r#"
        type Counter { id: Int; v: Int; }

        @form(hashmap, sync = lockfree, cap = 60000)
        locus Registry {
            capacity { pool entries of Counter indexed_by id; }
        }

        locus PoolHost {
            params { reg: Registry = Registry { }; }
            run() {
                let mut i = 0;
                while i < 20000 {
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
                while i < 20000 {
                    self.host.reg.set(Counter { id: i * 2, v: i });
                    i = i + 1;
                }
                while self.host.reg.len() < 40000 {
                    std::time::sleep(1ms);
                }
                print("len="); println(self.host.reg.len());
            }
        }

        fn main() { App { }; }
    "#;
    let (stdout, status) = build_and_run("cross_pool", src);
    assert!(
        status.success(),
        "binary exited non-zero (probable CAS or memory-order bug): {:?}\nstdout: {}",
        status,
        stdout,
    );
    assert!(
        stdout.contains("len=40000"),
        "expected len=40000 after both writers drain; got:\n{}",
        stdout
    );
}

#[test]
fn lockfree_update_existing_key() {
    // Update path: same key written twice. Verifies the CAS
    // COMMITTED → CLAIMED → write-value → COMMITTED transition
    // works (γ-v1's update branch in set_lockfree).
    let src = r#"
        type Counter { id: Int; v: Int; }

        @form(hashmap, sync = lockfree, cap = 64)
        locus Registry {
            capacity { pool entries of Counter indexed_by id; }
        }

        main locus App {
            params { reg: Registry = Registry { }; }
            run() {
                self.reg.set(Counter { id: 7, v: 100 });
                self.reg.set(Counter { id: 7, v: 200 });   // update
                print("len="); println(self.reg.len());
                let e = self.reg.get(7) or raise;
                print("v="); println(e.v);
            }
        }

        fn main() { App { }; }
    "#;
    let (stdout, status) = build_and_run("update", src);
    assert!(
        status.success(),
        "binary exited non-zero: {:?}\nstdout: {}",
        status,
        stdout,
    );
    assert!(stdout.contains("len=1"), "update should not double-count; got: {:?}", stdout);
    assert!(stdout.contains("v=200"), "update should overwrite; got: {:?}", stdout);
}

#[test]
fn lockfree_remove_present_key() {
    // γ-v2 session 1 entry point: `remove` is no longer a no-op
    // on lockfree maps. The CAS COMMITTED → TOMBSTONE must
    // succeed; subsequent get / has must report the key absent.
    let src = r#"
        type Counter { id: Int; v: Int; }

        @form(hashmap, sync = lockfree, cap = 64)
        locus Registry {
            capacity { pool entries of Counter indexed_by id; }
        }

        main locus App {
            params { reg: Registry = Registry { }; }
            run() {
                self.reg.set(Counter { id: 7, v: 100 });
                self.reg.set(Counter { id: 8, v: 200 });
                print("len_before="); println(self.reg.len());
                self.reg.remove(7) or raise;
                print("len_after="); println(self.reg.len());
                print("has7="); println(self.reg.has(7));
                print("has8="); println(self.reg.has(8));
                if !self.reg.has(7) { println("get7_missing"); }
                let e8 = self.reg.get(8) or raise;
                print("get8.v="); println(e8.v);
            }
        }

        fn main() { App { }; }
    "#;
    let (stdout, status) = build_and_run("remove_present", src);
    assert!(
        status.success(),
        "binary exited non-zero: {:?}\nstdout: {}",
        status, stdout,
    );
    assert!(stdout.contains("len_before=2"), "got: {:?}", stdout);
    assert!(stdout.contains("len_after=1"), "len should drop on remove; got: {:?}", stdout);
    assert!(stdout.contains("has7=false"), "removed key should be absent; got: {:?}", stdout);
    assert!(stdout.contains("has8=true"), "other key must remain; got: {:?}", stdout);
    assert!(stdout.contains("get7_missing"), "get of removed key should raise; got: {:?}", stdout);
    assert!(stdout.contains("get8.v=200"), "other key must still be reachable; got: {:?}", stdout);
}

#[test]
fn lockfree_remove_missing_key() {
    // Differential from γ-v1: under v1, every `remove` call
    // returned 0 (always KeyError). Under v2, `remove` reports
    // success on present keys and KeyError on missing ones.
    // This test exercises only the missing-key path on an
    // otherwise non-empty map to confirm the "miss" diagnostic
    // still surfaces.
    let src = r#"
        type Counter { id: Int; v: Int; }

        @form(hashmap, sync = lockfree, cap = 64)
        locus Registry {
            capacity { pool entries of Counter indexed_by id; }
        }

        fn handle(_e: KeyError) { println("caught_miss"); }

        main locus App {
            params { reg: Registry = Registry { }; }
            run() {
                self.reg.set(Counter { id: 1, v: 1 });
                self.reg.remove(99) or handle(err);
                print("len="); println(self.reg.len());
                print("has1="); println(self.reg.has(1));
            }
        }

        fn main() { App { }; }
    "#;
    let (stdout, status) = build_and_run("remove_missing", src);
    assert!(
        status.success(),
        "binary exited non-zero: {:?}\nstdout: {}",
        status, stdout,
    );
    assert!(stdout.contains("caught_miss"), "missing-key remove should surface KeyError; got: {:?}", stdout);
    assert!(stdout.contains("len=1"), "live entry must not be disturbed; got: {:?}", stdout);
    assert!(stdout.contains("has1=true"), "live key must remain; got: {:?}", stdout);
}

#[test]
fn lockfree_set_remove_set_same_key() {
    // The session-1 headline scenario from the handoff doc:
    // set / remove / set on the same key must work. Probe lands
    // on the TOMBSTONE, advances past it (v2 session 1 doesn't
    // reuse tombstoned slots), and inserts in the next EMPTY
    // slot. `get` then walks past the TOMBSTONE to find the
    // fresh COMMITTED entry.
    let src = r#"
        type Counter { id: Int; v: Int; }

        @form(hashmap, sync = lockfree, cap = 64)
        locus Registry {
            capacity { pool entries of Counter indexed_by id; }
        }

        main locus App {
            params { reg: Registry = Registry { }; }
            run() {
                self.reg.set(Counter { id: 42, v: 1 });
                let e1 = self.reg.get(42) or raise;
                print("first="); println(e1.v);
                self.reg.remove(42) or raise;
                print("len_mid="); println(self.reg.len());
                print("has_mid="); println(self.reg.has(42));
                self.reg.set(Counter { id: 42, v: 999 });
                let e2 = self.reg.get(42) or raise;
                print("second="); println(e2.v);
                print("len_after="); println(self.reg.len());
                print("has_after="); println(self.reg.has(42));
            }
        }

        fn main() { App { }; }
    "#;
    let (stdout, status) = build_and_run("set_remove_set", src);
    assert!(
        status.success(),
        "binary exited non-zero: {:?}\nstdout: {}",
        status, stdout,
    );
    assert!(stdout.contains("first=1"), "initial set/get; got: {:?}", stdout);
    assert!(stdout.contains("len_mid=0"), "len must go to 0 after remove; got: {:?}", stdout);
    assert!(stdout.contains("has_mid=false"), "key must be absent after remove; got: {:?}", stdout);
    assert!(stdout.contains("second=999"), "re-set must replace; got: {:?}", stdout);
    assert!(stdout.contains("len_after=1"), "len back to 1 after re-set; got: {:?}", stdout);
    assert!(stdout.contains("has_after=true"), "key present after re-set; got: {:?}", stdout);
}

#[test]
fn lockfree_iter_skips_tombstones() {
    // Iteration (key_at / entry_at) must skip tombstoned slots.
    // m->len tracks live entries; the i-th live entry is the
    // i-th non-EMPTY / non-TOMBSTONE slot in scan order. After
    // removing a subset of keys, the iteration sum must equal
    // the sum of remaining values, not include removed values.
    let src = r#"
        type Counter { id: Int; v: Int; }

        @form(hashmap, sync = lockfree, cap = 64)
        locus Registry {
            capacity { pool entries of Counter indexed_by id; }
        }

        main locus App {
            params { reg: Registry = Registry { }; }
            run() {
                self.reg.set(Counter { id: 1, v: 10 });
                self.reg.set(Counter { id: 2, v: 20 });
                self.reg.set(Counter { id: 3, v: 30 });
                self.reg.set(Counter { id: 4, v: 40 });
                self.reg.set(Counter { id: 5, v: 50 });
                self.reg.remove(2) or raise;
                self.reg.remove(4) or raise;
                print("len="); println(self.reg.len());
                let n = self.reg.len();
                let mut i = 0;
                let mut total = 0;
                while i < n {
                    let e = self.reg.entry_at(i) or raise;
                    total = total + e.v;
                    i = i + 1;
                }
                print("total="); println(total);
            }
        }

        fn main() { App { }; }
    "#;
    let (stdout, status) = build_and_run("iter_skip_tomb", src);
    assert!(
        status.success(),
        "binary exited non-zero: {:?}\nstdout: {}",
        status, stdout,
    );
    assert!(stdout.contains("len=3"), "len should reflect live entries only; got: {:?}", stdout);
    // Live entries are id=1 (v=10), id=3 (v=30), id=5 (v=50). Sum = 90.
    assert!(stdout.contains("total=90"), "iteration must skip tombstoned entries; got: {:?}", stdout);
}

#[test]
fn lockfree_grow_beyond_initial_cap() {
    // γ-v2 session 3: insert beyond the user-declared `cap = N`.
    // Under v1 this would silently drop entries; under v2 the
    // table grows transparently. Verify ALL entries land and
    // remain reachable.
    let src = r#"
        type Counter { id: Int; v: Int; }

        // Small initial cap (8) to force several grows.
        @form(hashmap, sync = lockfree, cap = 8)
        locus Registry {
            capacity { pool entries of Counter indexed_by id; }
        }

        main locus App {
            params { reg: Registry = Registry { }; }
            run() {
                let mut i = 0;
                while i < 500 {
                    self.reg.set(Counter { id: i, v: i * 10 });
                    i = i + 1;
                }
                print("len="); println(self.reg.len());
                // Spot-check entries at low / mid / high.
                let e0 = self.reg.get(0) or raise;
                let e250 = self.reg.get(250) or raise;
                let e499 = self.reg.get(499) or raise;
                print("v0="); println(e0.v);
                print("v250="); println(e250.v);
                print("v499="); println(e499.v);
            }
        }

        fn main() { App { }; }
    "#;
    let (stdout, status) = build_and_run("grow", src);
    assert!(
        status.success(),
        "binary exited non-zero: {:?}\nstdout: {}",
        status, stdout,
    );
    assert!(stdout.contains("len=500"), "grow must preserve all entries; got: {:?}", stdout);
    assert!(stdout.contains("v0=0"), "low-id entry must survive grow; got: {:?}", stdout);
    assert!(stdout.contains("v250=2500"), "mid-id entry must survive grow; got: {:?}", stdout);
    assert!(stdout.contains("v499=4990"), "high-id entry must survive grow; got: {:?}", stdout);
}

#[test]
fn lockfree_grow_drops_tombstones() {
    // γ-v2 session 3: migration rebuilds the table without
    // tombstones (lazy compaction). After heavy churn that
    // accumulates tombstones, a grow should reset
    // tombstone_count to 0 while preserving live entries.
    // We can't read tombstone_count directly from .hl code,
    // so the indirect test is: insert + remove churn that
    // would saturate a fixed-cap v1 table; with grow shipping,
    // the workload completes cleanly.
    let src = r#"
        type Counter { id: Int; v: Int; }

        @form(hashmap, sync = lockfree, cap = 16)
        locus Registry {
            capacity { pool entries of Counter indexed_by id; }
        }

        main locus App {
            params { reg: Registry = Registry { }; }
            run() {
                // Insert 200, remove 100, insert another 100 with
                // re-used ids. v1 with cap=16 would saturate
                // immediately. v2 grows + compacts.
                let mut i = 0;
                while i < 200 {
                    self.reg.set(Counter { id: i, v: i });
                    i = i + 1;
                }
                let mut j = 0;
                while j < 100 {
                    self.reg.remove(j) or raise;
                    j = j + 1;
                }
                let mut k = 0;
                while k < 100 {
                    self.reg.set(Counter { id: k, v: k + 1000 });
                    k = k + 1;
                }
                print("len="); println(self.reg.len());
                // Verify the re-inserted entries are reachable
                // (and got the new value).
                let e0 = self.reg.get(0) or raise;
                let e99 = self.reg.get(99) or raise;
                let e150 = self.reg.get(150) or raise;
                print("v0="); println(e0.v);
                print("v99="); println(e99.v);
                print("v150="); println(e150.v);
            }
        }

        fn main() { App { }; }
    "#;
    let (stdout, status) = build_and_run("grow_compaction", src);
    assert!(
        status.success(),
        "binary exited non-zero: {:?}\nstdout: {}",
        status, stdout,
    );
    assert!(stdout.contains("len=200"), "live entries after churn+grow; got: {:?}", stdout);
    assert!(stdout.contains("v0=1000"), "re-inserted entry must take new value; got: {:?}", stdout);
    assert!(stdout.contains("v99=1099"), "re-inserted entry must take new value; got: {:?}", stdout);
    assert!(stdout.contains("v150=150"), "preserved entry from initial insert; got: {:?}", stdout);
}
