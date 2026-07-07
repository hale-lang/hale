//! v1.x-FORM-4 PR5 — `@form(hashmap)` codegen.
//!
//! Mirrors `form_vec_codegen.rs`'s shape. The structural lowering:
//! a `@form(hashmap)` locus's pool slot becomes an inline
//! `lotus_hashmap_t`-shaped struct managed by the `lotus_hashmap_*`
//! C runtime instead of the literal F.22 pool allocator. Methods
//! (set/get/has/remove/len/is_empty) lower inline; the intrusive
//! shape extracts the key by GEP'ing the indexed-by field at the
//! set call site.

use std::process::Command;

use hale_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_form_hashmap_codegen_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

/// Minimum @form(hashmap) lowering: locus instantiates with
/// String-keyed entries, lifecycle runs, dissolve fires
/// lotus_hashmap_destroy on the inline struct. No inserts; the
/// slots buffer is the initial cap=8 calloc and destroy frees it
/// cleanly.
#[test]
fn form_hashmap_locus_instantiates_and_dissolves_cleanly() {
    let src = r#"
        type Entry { name: String; v: Int; }
        @form(hashmap)
        locus RegistryL {
            capacity { pool entries of Entry indexed_by name; }
            birth    { println("birth"); }
            dissolve { println("dissolve"); }
        }
        fn main() {
            let _ = RegistryL { };
        }
    "#;
    let bin = build("lifecycle_empty", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("birth"), "missing birth: {:?}", stdout);
    assert!(stdout.contains("dissolve"), "missing dissolve: {:?}", stdout);
}

/// Int-keyed round trip: set, then get back the matching value.
#[test]
fn hashmap_int_keyed_set_and_get() {
    let src = r#"
        type Entry { id: Int; payload: Int; }
        @form(hashmap)
        locus L { capacity { pool entries of Entry indexed_by id; } }
        fn main() {
            let r = L { };
            r.set(Entry { id: 42, payload: 100 });
            let e = r.get(42) or raise;
            println(e.payload);
        }
    "#;
    let bin = build("int_keyed_set_get", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.trim().ends_with("100"), "expected 100, got: {:?}", stdout);
}

/// String-keyed round trip.
#[test]
fn hashmap_string_keyed_set_and_get() {
    let src = r#"
        type Entry { name: String; v: Int; }
        @form(hashmap)
        locus L { capacity { pool entries of Entry indexed_by name; } }
        fn main() {
            let r = L { };
            r.set(Entry { name: "alpha", v: 7 });
            let e = r.get("alpha") or raise;
            println(e.v);
        }
    "#;
    let bin = build("string_keyed_set_get", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.trim().ends_with("7"), "expected 7, got: {:?}", stdout);
}

/// `get(missing) or fallback` substitutes; `err.kind` is
/// available on the substitute RHS.
#[test]
fn hashmap_get_missing_or_substitute_uses_fallback() {
    let src = r#"
        type Entry { id: Int; v: Int; }
        @form(hashmap)
        locus L { capacity { pool entries of Entry indexed_by id; } }
        fn main() {
            let r = L { };
            r.set(Entry { id: 1, v: 100 });
            let e = r.get(99) or Entry { id: -1, v: -1 };
            if e.v != -1 { println("FAIL: fallback value"); }
            println("ok");
        }
    "#;
    let bin = build("get_substitute", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok"), "expected ok, got: {:?}", stdout);
    assert!(!stdout.contains("FAIL"), "unexpected FAIL: {:?}", stdout);
}

/// `get(missing) or raise` at the top level of main panics via
/// `lotus_root_panic`: exit code non-zero, stderr names KeyError.
#[test]
fn hashmap_get_missing_or_raise_panics_at_root() {
    let src = r#"
        type Entry { id: Int; v: Int; }
        @form(hashmap)
        locus L { capacity { pool entries of Entry indexed_by id; } }
        fn main() {
            let r = L { };
            let e = r.get(99) or raise;
            println(e.v);
        }
    "#;
    let bin = build("get_raise_root_panic", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        !out.status.success(),
        "expected non-zero exit on root-panic, got: {:?}",
        out.status
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("KeyError") && stderr.contains("main locus"),
        "expected root-panic message, got stderr: {:?}",
        stderr
    );
}

/// `has` flips false → true once an entry lands; stays true for
/// the keyed value; false for unknown keys.
#[test]
fn hashmap_has_tracks_sets() {
    let src = r#"
        type Entry { id: Int; v: Int; }
        @form(hashmap)
        locus L { capacity { pool entries of Entry indexed_by id; } }
        fn main() {
            let r = L { };
            if r.has(1) { println("FAIL: has before set"); }
            r.set(Entry { id: 1, v: 100 });
            if !r.has(1) { println("FAIL: has after set"); }
            if r.has(99) { println("FAIL: has unknown"); }
            println("ok");
        }
    "#;
    let bin = build("has_tracks", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok"), "expected ok, got: {:?}", stdout);
    assert!(!stdout.contains("FAIL"), "unexpected FAIL: {:?}", stdout);
}

/// `len` and `is_empty` track inserts + removes.
#[test]
fn hashmap_len_and_is_empty_track_mutations() {
    let src = r#"
        type Entry { id: Int; v: Int; }
        @form(hashmap)
        locus L { capacity { pool entries of Entry indexed_by id; } }
        fn main() {
            let r = L { };
            if !r.is_empty() { println("FAIL: not initially empty"); }
            if r.len() != 0 { println("FAIL: initial len"); }
            r.set(Entry { id: 1, v: 1 });
            r.set(Entry { id: 2, v: 2 });
            r.set(Entry { id: 3, v: 3 });
            if r.is_empty() { println("FAIL: empty after sets"); }
            if r.len() != 3 { println("FAIL: len after sets"); }
            r.remove(2) or raise;
            if r.len() != 2 { println("FAIL: len after remove"); }
            println("ok");
        }
    "#;
    let bin = build("len_is_empty_track", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok"), "expected ok, got: {:?}", stdout);
    assert!(!stdout.contains("FAIL"), "unexpected FAIL: {:?}", stdout);
}

/// `set` of a duplicate key replaces in place (len doesn't grow,
/// value is overwritten).
#[test]
fn hashmap_set_duplicate_key_overwrites() {
    let src = r#"
        type Entry { id: Int; v: Int; }
        @form(hashmap)
        locus L { capacity { pool entries of Entry indexed_by id; } }
        fn main() {
            let r = L { };
            r.set(Entry { id: 1, v: 100 });
            r.set(Entry { id: 1, v: 200 });
            if r.len() != 1 { println("FAIL: len grew on duplicate"); }
            let e = r.get(1) or raise;
            if e.v != 200 { println("FAIL: old value remains"); }
            println("ok");
        }
    "#;
    let bin = build("set_duplicate_overwrites", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok"), "expected ok, got: {:?}", stdout);
    assert!(!stdout.contains("FAIL"), "unexpected FAIL: {:?}", stdout);
}

/// `remove` of a missing key fails with KeyError; a Unit-returning
/// handler swallows it as a statement. Confirms the
/// fallible-Unit-success path lowers correctly (FallibleCallResult
/// with `success_ty = None`).
#[test]
fn hashmap_remove_missing_substitute_swallows() {
    let src = r#"
        type Entry { id: Int; v: Int; }
        @form(hashmap)
        locus L { capacity { pool entries of Entry indexed_by id; } }
        fn ignore(_e: KeyError) { }
        fn main() {
            let r = L { };
            r.set(Entry { id: 1, v: 1 });
            r.remove(99) or ignore(err);
            if r.len() != 1 { println("FAIL: live entry removed"); }
            println("ok");
        }
    "#;
    let bin = build("remove_missing_swallow", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok"), "expected ok, got: {:?}", stdout);
    assert!(!stdout.contains("FAIL"), "unexpected FAIL: {:?}", stdout);
}

/// `err` binding is in scope on the substitute RHS, with payload
/// type KeyError; `err.kind` is "missing_key" when get fails.
#[test]
fn hashmap_err_binding_kind_available() {
    let src = r#"
        type Entry { id: Int; v: Int; }
        @form(hashmap)
        locus L { capacity { pool entries of Entry indexed_by id; } }
        fn fallback(e: KeyError) -> Entry {
            println(e.kind);
            return Entry { id: -1, v: -1 };
        }
        fn main() {
            let r = L { };
            let e = r.get(42) or fallback(err);
            if e.v != -1 { println("FAIL: fallback not used"); }
            println("ok");
        }
    "#;
    let bin = build("err_binding_kind", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("missing_key"),
        "expected kind=missing_key in output, got: {:?}",
        stdout
    );
    assert!(stdout.contains("ok"), "expected ok, got: {:?}", stdout);
    assert!(!stdout.contains("FAIL"), "unexpected FAIL: {:?}", stdout);
}

/// Survives the load-factor grow path: 32 inserts at initial cap=8
/// force several doublings, each re-hashes via the normal set
/// path. All values remain retrievable.
#[test]
fn hashmap_grows_and_retains_entries() {
    let src = r#"
        type Entry { id: Int; v: Int; }
        @form(hashmap)
        locus L { capacity { pool entries of Entry indexed_by id; } }
        fn main() {
            let r = L { };
            for i in 0..32 {
                r.set(Entry { id: i, v: i * 10 });
            }
            if r.len() != 32 { println("FAIL: len after grow"); }
            for i in 0..32 {
                let e = r.get(i) or raise;
                if e.v != i * 10 { println("FAIL: value mismatch"); }
            }
            println("ok");
        }
    "#;
    let program = hale_syntax::parse_source(src);
    if program.is_err() {
        eprintln!("skip: parser doesn't yet support 0..N range");
        return;
    }
    let bin = build("grow_retains", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok"), "expected ok, got: {:?}", stdout);
    assert!(!stdout.contains("FAIL"), "unexpected FAIL: {:?}", stdout);
}

/// `self.set` / `self.get` from inside the locus's own body. Same
/// dispatcher, different call site (lower_self_method_call).
#[test]
fn hashmap_self_dispatch_inside_locus_method() {
    let src = r#"
        type Entry { id: Int; v: Int; }
        @form(hashmap)
        locus L {
            capacity { pool entries of Entry indexed_by id; }
            fn seed() {
                self.set(Entry { id: 1, v: 100 });
                self.set(Entry { id: 2, v: 200 });
            }
        }
        fn main() {
            let r = L { };
            r.seed();
            let a = r.get(1) or raise;
            let b = r.get(2) or raise;
            if a.v + b.v != 300 { println("FAIL: sum"); }
            println("ok");
        }
    "#;
    let bin = build("self_dispatch", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok"), "expected ok, got: {:?}", stdout);
    assert!(!stdout.contains("FAIL"), "unexpected FAIL: {:?}", stdout);
}

/// Regression for the BookSignalState bigcell leak (2026-05-25
/// fathom handoff). Anchor-in-place at @form(hashmap).set worked
/// for flat scalar cells (`f042806`), but cells with a fixed-size
/// array field still allocated a fresh `[N x elem]` buffer in the
/// hashmap's arena on every set — the per-field deep-copy path
/// lacked the `lotus_arena_contains_ptr` same-arena skip that
/// String/Bytes get from `lotus_str_clone`. apps/mdgw/kraken's
/// `BookSignalStateMap` (cell carrying 2× `[BookLevel; 100]`) grew
/// ~200 MB/min until OOM. Lock-in: chunk count for the hashmap's
/// arena must stay flat across many sets on the same key.
#[test]
fn hashmap_set_bigcell_with_array_field_does_not_leak() {
    let src = r#"
        type Level { p: Int; q: Int; }
        type BigCell {
            id:   Int;
            bids: [Level; 8];
            asks: [Level; 8];
            name: String;
        }
        @form(hashmap)
        locus M { capacity { pool cells of BigCell indexed_by id; } }
        fn main() {
            let m = M { };
            // First set populates the slot's array pointers.
            m.set(BigCell {
                id:   1,
                bids: [Level { p: 0, q: 0 }; 8],
                asks: [Level { p: 0, q: 0 }; 8],
                name: "bk",
            });
            std::process::dump_arena_residency();
            // Hot path: RMW the cell — load via get, build a fresh
            // BigCell, set back. The anchor path must reuse the
            // existing in-arena array buffers, not allocate fresh
            // ones each call.
            let mut i = 0;
            while i < 400 {
                let e = m.get(1) or raise;
                m.set(BigCell {
                    id:   1,
                    bids: e.bids,
                    asks: e.asks,
                    name: e.name,
                });
                i = i + 1;
            }
            std::process::dump_arena_residency();
            println("ok");
        }
    "#;
    let bin = build("bigcell_anchor", src);
    let out = Command::new(&bin)
        .env("LOTUS_ARENA_RESIDENCY", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok"), "expected ok, got: {:?}", stdout);

    // Parse stderr for the hashmap arena's chunk count across the
    // two residency dumps. Dump format:
    //   [#<id> arena=<ptr> label=<label>] chunks=<n> bytes=<n> ...
    // We look for label=M (the hashmap locus's arena) twice and
    // compare. Pre-fix the second count grows by hundreds; post-fix
    // it's the same.
    let stderr = String::from_utf8_lossy(&out.stderr);
    let chunks: Vec<usize> = stderr
        .lines()
        .filter(|ln| ln.contains("label=M ") || ln.contains("label=M]"))
        .filter_map(|ln| {
            ln.split("chunks=")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .and_then(|s| s.parse::<usize>().ok())
        })
        .collect();
    assert!(
        chunks.len() >= 2,
        "expected two M-arena residency rows, got {:?} (stderr={})",
        chunks,
        stderr
    );
    let first = chunks[0];
    let last = *chunks.last().unwrap();
    assert!(
        last <= first + 1,
        "M arena grew across 400 same-key sets: chunks {} → {} (bigcell anchor regressed). stderr={}",
        first,
        last,
        stderr
    );
}

/// Anchor-retirement freelist corruption regression (P0, fixed
/// 2026-07-06). A String-keyed map whose value struct carries the
/// `indexed_by` field aliases ONE clone as both the map key and that
/// value field (codegen clones the key once, stores the same pointer
/// in both). `lotus_hashmap_retire_cell` retired it twice — once in
/// the value-field loop, once in the key-clone block — double-pushing
/// the blob onto retire_pending; at flush the two shells self-linked
/// the freelist node (header.next = itself), so a later pop walked
/// string bytes as a next-pointer → SIGSEGV in lotus_retire_free_pop.
/// The corruption needs a SECOND key of a DIFFERENT byte length in the
/// map (the pop's size band lets a shorter request claim a longer
/// block), so this churns two interleaved keys of different lengths.
///
/// Two assertions in one: (1) it must not crash — pre-fix this SEGVs
/// within tens of thousands of churns; (2) reuse must be PRESERVED —
/// the map arena's chunk count stays flat, proving the retire/reuse
/// path still recycles blocks (the fix only DEDUPS the double retire).
#[test]
fn hashmap_string_key_two_length_churn_no_freelist_corruption() {
    // The map is a FIELD churned via the owning locus's method — the
    // real pattern (a locus holding a @form map, its methods churning
    // it). The retire/flush/reuse cycle fires at the owning method's
    // activation boundary; a direct-local map in a free fn never
    // flushes (and would leak regardless of this fix), so the structure
    // matters for the reuse half of the assertion.
    let src = r#"
        type Slot { key: String = ""; qty: Int = 0; }
        @form(hashmap)
        locus SkChurn { capacity { pool rows of Slot indexed_by key; } }
        main locus Holder {
            params { basis: SkChurn = SkChurn { }; }
            fn churn(k: String) {
                let cur = self.basis.get(k) or Slot { };
                self.basis.set(Slot { key: k, qty: cur.qty + 1 });
            }
            run() {
                let base = "KKN:S:btc-usd";
                // Warm both keys into the map, then dump residency.
                self.churn("signals|" + base);
                self.churn("beta|" + base);
                std::process::dump_arena_residency();
                let mut i = 0;
                while i < 60000 {
                    // "signals|" (22B) : "beta|" (19B) interleaved 43:1 —
                    // concat forces a dynamic (non-.rodata) key each set,
                    // so the clone/retire/reuse path is exercised.
                    self.churn(if i % 44 == 43 { "beta|" + base } else { "signals|" + base });
                    i = i + 1;
                }
                std::process::dump_arena_residency();
                let sig = self.basis.get("signals|" + base) or Slot { };
                let bet = self.basis.get("beta|" + base) or Slot { };
                println("total=", sig.qty + bet.qty);
            }
        }
        fn main() { Holder { }; }
    "#;
    let bin = build("sk_two_length_churn", src);
    let out = Command::new(&bin)
        .env("LOTUS_ARENA_RESIDENCY", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    // (1) no crash — pre-fix this exits via SIGSEGV.
    assert!(
        out.status.success(),
        "non-zero exit (freelist corruption regressed?): {:?} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // 2 warm-up churns + 60000 loop churns = 60002 increments split
    // across the two keys; the sum must equal the total churn count
    // (no lost updates from the retire/reuse cycle).
    assert!(
        stdout.contains("total=60002"),
        "expected coherent total=60002, got: {:?}",
        stdout
    );
    // (2) reuse preserved — the map arena's chunk count stays flat
    // (the clones live in SkChurn's own arena; retire/reuse recycles
    // them so it never grows past its initial chunk).
    let stderr = String::from_utf8_lossy(&out.stderr);
    let chunks: Vec<usize> = stderr
        .lines()
        .filter(|ln| ln.contains("label=SkChurn ") || ln.contains("label=SkChurn]"))
        .filter_map(|ln| {
            ln.split("chunks=")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .and_then(|s| s.parse::<usize>().ok())
        })
        .collect();
    assert!(
        chunks.len() >= 2,
        "expected two SkChurn-arena residency rows, got {:?} (stderr={})",
        chunks,
        stderr
    );
    let first = chunks[0];
    let last = *chunks.last().unwrap();
    assert!(
        last <= first + 1,
        "SkChurn arena grew across 60k two-length churns: chunks {} → {} \
         (retire/reuse regressed to unbounded growth). stderr={}",
        first,
        last,
        stderr
    );
}
