//! 2026-07-02 @form iteration surface: `for e in m.entries`
//! (hashmap — cluster-aware slot-cursor walk via
//! lotus_hashmap_iter_next, O(cap) total vs key_at/entry_at's
//! O(cap×len)) and `for x in v.items` (vec — fully inline buf
//! walk, zero per-element calls). The loop variable is a
//! per-iteration copy of the cell.

use std::process::Command;

use hale_codegen::build_executable;
use hale_syntax::parse_source;

fn build_and_run(name: &str, src: &str) -> String {
    let program = parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    static NEXT: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    bin.push(format!(
        "hale_form_iter_{}_{}_{}",
        name,
        std::process::id(),
        n
    ));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn hashmap_entries_iteration_visits_each_once() {
    let out = build_and_run(
        "hm",
        r#"
        type Entry { id: Int; val: Int; }
        @form(hashmap)
        locus Reg {
            capacity { pool cells of Entry indexed_by id; }
        }
        fn main() {
            let m = Reg { };
            let mut i = 0;
            while i < 1000 {
                m.set(Entry { id: i, val: i * 3 });
                i = i + 1;
            }
            // Overwrite a key — count must stay 1000 (no dup visit).
            m.set(Entry { id: 5, val: 15 });
            let mut sum = 0;
            let mut cnt = 0;
            for e in m.entries {
                sum = sum + e.val;
                cnt = cnt + 1;
            }
            println("cnt=", cnt);
            println("sum=", sum);
        }
    "#,
    );
    assert!(out.contains("cnt=1000"), "got: {:?}", out);
    // sum = Σ 3i for i in 0..1000 = 1498500
    assert!(out.contains("sum=1498500"), "got: {:?}", out);
}

#[test]
fn hashmap_entries_after_remove_skips_removed() {
    let out = build_and_run(
        "hmrm",
        r#"
        type Entry { id: Int; val: Int; }
        @form(hashmap)
        locus Reg {
            capacity { pool cells of Entry indexed_by id; }
        }
        fn main() {
            let m = Reg { };
            let mut i = 0;
            while i < 100 {
                m.set(Entry { id: i, val: i });
                i = i + 1;
            }
            m.remove(7) or raise;
            m.remove(42) or raise;
            let mut cnt = 0;
            let mut sum = 0;
            for e in m.entries {
                cnt = cnt + 1;
                sum = sum + e.val;
            }
            println("cnt=", cnt);
            println("sum=", sum);
        }
    "#,
    );
    assert!(out.contains("cnt=98"), "got: {:?}", out);
    // Σ 0..99 − 7 − 42 = 4950 − 49 = 4901
    assert!(out.contains("sum=4901"), "got: {:?}", out);
}

#[test]
fn vec_items_iteration_with_break_continue() {
    let out = build_and_run(
        "vec",
        r#"
        @form(vec)
        locus IntVec {
            capacity { heap cells of Int; }
        }
        fn main() {
            let v = IntVec { };
            let mut j = 0;
            while j < 1000 {
                v.push(j);
                j = j + 1;
            }
            let mut vsum = 0;
            for x in v.items {
                vsum = vsum + x;
            }
            println("vsum=", vsum);
            let mut partial = 0;
            for x in v.items {
                if x % 2 == 1 { continue; }
                if x >= 100 { break; }
                partial = partial + x;
            }
            println("partial=", partial);
        }
    "#,
    );
    assert!(out.contains("vsum=499500"), "got: {:?}", out);
    // Σ even x in 0..100 = 2450
    assert!(out.contains("partial=2450"), "got: {:?}", out);
}

#[test]
fn vec_items_over_struct_cells() {
    let out = build_and_run(
        "veccell",
        r#"
        type Pt { x: Int; y: Int; }
        @form(vec)
        locus Pts {
            capacity { heap cells of Pt; }
        }
        fn main() {
            let v = Pts { };
            let mut j = 0;
            while j < 50 {
                v.push(Pt { x: j, y: j * 2 });
                j = j + 1;
            }
            let mut s = 0;
            for p in v.items {
                s = s + p.x + p.y;
            }
            println("s=", s);
        }
    "#,
    );
    // Σ (j + 2j) for j in 0..50 = 3 * 1225 = 3675
    assert!(out.contains("s=3675"), "got: {:?}", out);
}
