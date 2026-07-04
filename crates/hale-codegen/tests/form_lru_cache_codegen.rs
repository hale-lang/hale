//! v1.x-FORM-6 — `@form(lru_cache)` codegen.
//!
//! A `@form(lru_cache, cap = N)` locus's pool slot becomes an
//! inline `{ i64 cap, i64 len, i64 key_size, i64 value_size,
//! i32 key_type_tag, i64 tick, i64 table_cap, ptr slots }` struct
//! managed by `lotus_lru_*`. `cap` is baked in at `lotus_lru_init`
//! from the annotation arg; the table is pre-allocated at locus
//! birth and NEVER grows — inserting a new key over `cap` silently
//! evicts the least-recently-USED entry.
//!
//! Method surface (synthesized):
//!   put(x: S) -> ()                  infallible; silent LRU evict
//!   get(k: K) -> S fallible(KeyError) lookup + recency touch
//!   contains(k: K) -> Bool           membership, NO recency touch
//!   len() -> Int                     current entry count (<= cap)

use std::path::PathBuf;
use std::process::Command;

use hale_codegen::build_executable;

fn build(name: &str, src: &str) -> PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_form_lru_codegen_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

fn run(bin: &PathBuf) -> (String, bool) {
    let out = Command::new(bin).output().expect("run");
    let _ = std::fs::remove_file(bin);
    (String::from_utf8_lossy(&out.stdout).into_owned(), out.status.success())
}

/// Bounded: a cap-3 cache never holds more than 3 entries no
/// matter how many distinct keys are inserted.
#[test]
fn form_lru_cache_is_bounded() {
    let src = r#"
        type Entry { id: Int; val: Int; }
        @form(lru_cache, cap = 3)
        locus C { capacity { pool rows of Entry indexed_by id; } }
        fn main() {
            let c = C { };
            let mut i = 1;
            while i <= 20 {
                c.put(Entry { id: i, val: i });
                i = i + 1;
            }
            println(c.len());
        }
    "#;
    let bin = build("bounded", src);
    let (stdout, ok) = run(&bin);
    assert!(ok, "non-zero exit");
    assert!(stdout.contains("3"), "len must stay capped at 3: {:?}", stdout);
}

/// The discriminating LRU-vs-FIFO case. Insert A,B,C at cap=3;
/// `get(A)` (touch A → most-recently-used); insert D. A pure-FIFO
/// policy evicts A (oldest inserted); LRU must evict B instead.
#[test]
fn form_lru_cache_evicts_lru_not_fifo() {
    let src = r#"
        type Entry { id: Int; val: Int; }
        @form(lru_cache, cap = 3)
        locus C { capacity { pool rows of Entry indexed_by id; } }
        fn main() {
            let c = C { };
            c.put(Entry { id: 1, val: 100 });   // A
            c.put(Entry { id: 2, val: 200 });   // B
            c.put(Entry { id: 3, val: 300 });   // C
            let a = c.get(1) or raise;           // touch A
            c.put(Entry { id: 4, val: 400 });    // D -> evict LRU
            print(c.contains(1)); print(" ");    // A survives
            print(c.contains(2)); print(" ");    // B evicted
            print(c.contains(3)); print(" ");    // C survives
            print(c.contains(4)); print(" ");    // D present
            println(a.val);                      // round-trip
        }
    "#;
    let bin = build("lru_not_fifo", src);
    let (stdout, ok) = run(&bin);
    assert!(ok, "non-zero exit");
    // A=true B=false C=true D=true  — B evicted, NOT A (FIFO would
    // have taken A).
    assert!(
        stdout.contains("true false true true"),
        "expected LRU eviction of B (not FIFO of A): {:?}",
        stdout
    );
    assert!(stdout.contains("100"), "get(A) value round-trip: {:?}", stdout);
}

/// `contains` must NOT bump recency (unlike `get`). Mirror of the
/// case above: contains(A) then insert D must evict A, proving
/// contains left A as the LRU entry.
#[test]
fn form_lru_cache_contains_does_not_touch_recency() {
    let src = r#"
        type Entry { id: Int; val: Int; }
        @form(lru_cache, cap = 3)
        locus C { capacity { pool rows of Entry indexed_by id; } }
        fn main() {
            let c = C { };
            c.put(Entry { id: 1, val: 100 });   // A
            c.put(Entry { id: 2, val: 200 });   // B
            c.put(Entry { id: 3, val: 300 });   // C
            print(c.contains(1)); print(" ");    // membership only, no touch
            c.put(Entry { id: 4, val: 400 });    // D -> evict LRU (== A)
            print(c.contains(1)); print(" ");    // A evicted despite contains
            print(c.contains(2)); print(" ");    // B survives
            println(c.contains(4));              // D present
        }
    "#;
    let bin = build("contains_no_touch", src);
    let (stdout, ok) = run(&bin);
    assert!(ok, "non-zero exit");
    // first contains(1)=true; after inserting D, contains(1)=false
    // (A was NOT saved by the earlier contains), contains(2)=true,
    // contains(4)=true.
    assert!(
        stdout.contains("true false true true"),
        "contains must not touch recency (A should evict): {:?}",
        stdout
    );
}

/// Update-in-place: putting an existing key overwrites the value
/// and does not grow len; the update also refreshes recency.
#[test]
fn form_lru_cache_update_in_place() {
    let src = r#"
        type Entry { id: Int; val: Int; }
        @form(lru_cache, cap = 3)
        locus C { capacity { pool rows of Entry indexed_by id; } }
        fn main() {
            let c = C { };
            c.put(Entry { id: 5, val: 500 });
            c.put(Entry { id: 5, val: 999 });    // update, not insert
            print(c.len()); print(" ");
            let g = c.get(5) or raise;
            println(g.val);
        }
    "#;
    let bin = build("update", src);
    let (stdout, ok) = run(&bin);
    assert!(ok, "non-zero exit");
    assert!(stdout.contains("1 999"), "update must overwrite, keep len 1: {:?}", stdout);
}

/// String keys work too (shares the @form(hashmap) key ABI).
#[test]
fn form_lru_cache_string_keys() {
    let src = r#"
        type Rec { name: String; hits: Int; }
        @form(lru_cache, cap = 2)
        locus C { capacity { pool rows of Rec indexed_by name; } }
        fn main() {
            let c = C { };
            c.put(Rec { name: "alpha", hits: 1 });
            c.put(Rec { name: "beta",  hits: 2 });
            let a = c.get("alpha") or raise;      // touch alpha
            c.put(Rec { name: "gamma", hits: 3 }); // evict LRU (beta)
            print(c.contains("alpha")); print(" ");
            print(c.contains("beta"));  print(" ");
            print(c.contains("gamma")); print(" ");
            println(a.hits);
        }
    "#;
    let bin = build("string_keys", src);
    let (stdout, ok) = run(&bin);
    assert!(ok, "non-zero exit");
    assert!(
        stdout.contains("true false true 1"),
        "string-key LRU: alpha survives (touched), beta evicted: {:?}",
        stdout
    );
}

/// The in-tree fixture compiles, runs, and passes its own
/// assertions end-to-end (corpus_oracle also exercises it under
/// the exit / deadline / ASan oracles).
#[test]
fn form_lru_cache_fixture_runs() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/examples/60-lru-cache/main.hl");
    let src = std::fs::read_to_string(&path).expect("read fixture");
    let program = hale_syntax::parse_source(&src).expect("parse fixture");
    let mut bin = std::env::temp_dir();
    bin.push("hale_test_form_lru_fixture");
    build_executable(&program, &bin).expect("build fixture");
    let (stdout, ok) = run(&bin);
    assert!(ok, "fixture exited non-zero: {:?}", stdout);
    assert!(
        stdout.contains("all lru_cache tests passed"),
        "fixture assertions failed: {:?}",
        stdout
    );
}
