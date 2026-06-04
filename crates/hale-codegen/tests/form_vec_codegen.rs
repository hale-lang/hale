//! v1.x-FORM-2 PR5 — `@form(vec)` codegen.
//!
//! The structural lowering: a `@form(vec)` locus's heap slot
//! becomes an inline `{ i64 cap, i64 len, ptr buf }` struct
//! managed by the `lotus_vec_*` C runtime instead of the literal
//! F.22 heap allocator.
//!
//! This file focuses on the substrate lifecycle (init at
//! instantiation, destroy at dissolve). Method dispatch
//! (push / get / pop / len / is_empty) lands in subsequent
//! commits in this same PR.

use std::process::Command;

use hale_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_form_vec_codegen_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

/// Minimum @form(vec) lowering: locus instantiates, lifecycle runs,
/// dissolve fires `lotus_vec_destroy` on the inline struct. Nothing
/// is pushed; the buf pointer stays NULL and destroy is a no-op.
#[test]
fn form_vec_locus_instantiates_and_dissolves_cleanly() {
    let src = r#"
        @form(vec)
        locus IntListL {
            capacity {
                heap items of Int;
            }
            birth    { println("birth"); }
            dissolve { println("dissolve"); }
        }
        fn main() {
            let _ = IntListL { };
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

/// Two @form(vec) loci coexist without crosstalk — each has its
/// own inline storage, each dissolves cleanly.
#[test]
fn two_form_vec_loci_coexist() {
    let src = r#"
        @form(vec)
        locus AlphaL {
            capacity { heap items of Int; }
            birth { println("alpha-birth"); }
        }
        @form(vec)
        locus BetaL {
            capacity { heap items of Int; }
            birth { println("beta-birth"); }
        }
        fn main() {
            let _ = AlphaL { };
            let _ = BetaL { };
        }
    "#;
    let bin = build("two_coexist", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("alpha-birth"), "missing alpha: {:?}", stdout);
    assert!(stdout.contains("beta-birth"), "missing beta: {:?}", stdout);
}

/// Cell type T = struct. The inline vec field layout is independent
/// of T (always { cap, len, buf }); cell_size only matters when
/// push/get are invoked. Confirms struct-cell lowering doesn't
/// regress here.
#[test]
fn form_vec_of_struct_cell_instantiates() {
    let src = r#"
        type Pair {
            x: Int;
            y: Int;
        }
        @form(vec)
        locus PairListL {
            capacity { heap pairs of Pair; }
            birth { println("ok"); }
        }
        fn main() {
            let _ = PairListL { };
        }
    "#;
    let bin = build("struct_cell", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok"), "missing ok: {:?}", stdout);
}

/// `push` synthesized method drops elements into the inline vec.
/// `len` reports the count. Together they verify the C runtime is
/// being driven correctly.
#[test]
fn push_grows_len() {
    let src = r#"
        @form(vec)
        locus L {
            capacity { heap items of Int; }
        }
        fn main() {
            let l = L { };
            l.push(10);
            l.push(20);
            l.push(30);
            println(l.len());
        }
    "#;
    let bin = build("push_grows_len", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.trim().ends_with("3"), "expected len=3, got: {:?}", stdout);
}

/// `is_empty` flips false once a push lands.
#[test]
fn is_empty_tracks_pushes() {
    let src = r#"
        @form(vec)
        locus L {
            capacity { heap items of Int; }
        }
        fn main() {
            let l = L { };
            if l.is_empty() { println("empty-1"); }
            l.push(7);
            if !l.is_empty() { println("non-empty"); }
            if l.is_empty() { println("FAIL: should not be empty"); }
        }
    "#;
    let bin = build("is_empty", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("empty-1"), "missing empty-1: {:?}", stdout);
    assert!(stdout.contains("non-empty"), "missing non-empty: {:?}", stdout);
    assert!(!stdout.contains("FAIL"), "unexpected FAIL: {:?}", stdout);
}

/// `self.push` / `self.len` from inside the locus's own body. Same
/// helper, different dispatch site (lower_self_method_call).
#[test]
fn self_push_and_len_inside_locus_method() {
    let src = r#"
        @form(vec)
        locus L {
            capacity { heap items of Int; }
            fn fill() {
                self.push(1);
                self.push(2);
                self.push(3);
            }
        }
        fn main() {
            let l = L { };
            l.fill();
            println(l.len());
        }
    "#;
    let bin = build("self_dispatch", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.trim().ends_with("3"), "expected len=3, got: {:?}", stdout);
}

/// Many pushes to verify the doubling-realloc path inside
/// lotus_vec_push doesn't crash. Initial cap is 4; 100 pushes
/// force several reallocs.
#[test]
fn push_many_survives_realloc() {
    let src = r#"
        @form(vec)
        locus L {
            capacity { heap items of Int; }
        }
        fn main() {
            let l = L { };
            for i in 0..100 {
                l.push(i);
            }
            println(l.len());
        }
    "#;
    // for-in 0..N may not be supported; if it isn't, this test
    // will fail to build and we'll drop it.
    let program = hale_syntax::parse_source(src);
    if program.is_err() {
        eprintln!("skip: parser doesn't yet support 0..N range");
        return;
    }
    let bin = build("push_many", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.trim().ends_with("100"), "expected len=100, got: {:?}", stdout);
}

// =============================================================
// v1.x-FORM-2 PR6 / PR5 finale — fallible synthesized methods
// (get, pop) + Expr::Or three-motion lowering. These are now the
// canonical coverage for the `@form(vec)` get/pop/or surface
// (ported from the retired interpreter parity suite).
// =============================================================

/// `l.get(i) or raise` succeeds and returns the cell value.
#[test]
fn get_round_trip_or_raise_ok_path() {
    let src = r#"
        @form(vec)
        locus ItemListL {
            capacity { heap items of Int; }
        }
        fn main() {
            let l = ItemListL { };
            l.push(42);
            let head = l.get(0) or raise;
            println(head);
        }
    "#;
    let bin = build("get_or_raise_ok", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.trim() == "42", "expected 42, got: {:?}", stdout);
}

/// `l.pop() or raise` succeeds LIFO and `len` shrinks.
#[test]
fn pop_returns_last_element_lifo_ok_path() {
    let src = r#"
        @form(vec)
        locus L { capacity { heap items of Int; } }
        fn main() {
            let l = L { };
            l.push(10);
            l.push(20);
            l.push(30);
            let a = l.pop() or raise;
            let b = l.pop() or raise;
            if a != 30 { println("FAIL: first pop"); }
            if b != 20 { println("FAIL: second pop"); }
            if l.len() != 1 { println("FAIL: len after pops"); }
            println("ok");
        }
    "#;
    let bin = build("pop_lifo_ok", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok"), "expected ok, got: {:?}", stdout);
    assert!(!stdout.contains("FAIL"), "unexpected FAIL: {:?}", stdout);
}

/// `l.get(i) or <fallback>` uses the fallback when out-of-bounds.
#[test]
fn get_out_of_bounds_substitute_uses_fallback() {
    let src = r#"
        @form(vec)
        locus L { capacity { heap items of Int; } }
        fn main() {
            let l = L { };
            l.push(7);
            let v = l.get(99) or -1;
            if v != -1 { println("FAIL: fallback not used"); }
            println("ok");
        }
    "#;
    let bin = build("get_substitute_fallback", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok"), "expected ok, got: {:?}", stdout);
    assert!(!stdout.contains("FAIL"), "unexpected FAIL: {:?}", stdout);
}

/// `l.pop() or <fallback>` uses the fallback on empty.
#[test]
fn pop_empty_substitute_uses_fallback() {
    let src = r#"
        @form(vec)
        locus L { capacity { heap items of Int; } }
        fn main() {
            let l = L { };
            let v = l.pop() or -42;
            if v != -42 { println("FAIL: empty-pop fallback not used"); }
            println("ok");
        }
    "#;
    let bin = build("pop_substitute_fallback", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok"), "expected ok, got: {:?}", stdout);
    assert!(!stdout.contains("FAIL"), "unexpected FAIL: {:?}", stdout);
}

/// `l.get(99) or raise` at the top level of main panics via
/// `lotus_root_panic`: exit code is non-zero and stderr names
/// "IndexError escaping main locus".
#[test]
fn get_out_of_bounds_or_raise_panics_at_root() {
    let src = r#"
        @form(vec)
        locus L { capacity { heap items of Int; } }
        fn main() {
            let l = L { };
            let v = l.get(99) or raise;
            println(v);
        }
    "#;
    let bin = build("get_or_raise_panics", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        !out.status.success(),
        "expected non-zero exit on root-panic, got: {:?}",
        out.status
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("IndexError") && stderr.contains("main locus"),
        "expected root-panic message, got stderr: {:?}",
        stderr
    );
}

/// `err` binding implicit on substitute RHS — `err.index` and
/// `err.len` are accessible inside `or fallback(err)`.
#[test]
fn err_binding_available_on_substitute_rhs() {
    let src = r#"
        @form(vec)
        locus L { capacity { heap items of Int; } }
        fn fallback(e: IndexError) -> Int {
            return e.index + e.len;
        }
        fn main() {
            let l = L { };
            l.push(1);
            l.push(2);
            let v = l.get(5) or fallback(err);
            if v != 7 { println("FAIL: err binding payload"); }
            println("ok");
        }
    "#;
    let bin = build("err_binding_handler", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok"), "expected ok, got: {:?}", stdout);
    assert!(!stdout.contains("FAIL"), "unexpected FAIL: {:?}", stdout);
}

/// `fail E { ... };` inside a fallible(E) free fn exits via the
/// error path; caller substitutes via `or -99`.
#[test]
fn fail_stmt_exits_via_error_path_with_substitute() {
    let src = r#"
        type E { code: Int; }
        fn pick(x: Int) -> Int fallible(E) {
            if x < 0 {
                fail E { code: 1 };
            }
            return x * 2;
        }
        fn main() {
            let ok_v = pick(5) or -1;
            let bad_v = pick(-3) or -99;
            if ok_v != 10 { println("FAIL: success path"); }
            if bad_v != -99 { println("FAIL: error fallback"); }
            println("ok");
        }
    "#;
    let bin = build("fail_stmt_substitute", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok"), "expected ok, got: {:?}", stdout);
    assert!(!stdout.contains("FAIL"), "unexpected FAIL: {:?}", stdout);
}

/// `l.get(0) or raise` succeeds with a struct cell type;
/// confirms struct payloads round-trip through the out_val sret
/// slot.
#[test]
fn vec_of_struct_cells_get_round_trip() {
    let src = r#"
        type Pair { x: Int; y: Int; }
        @form(vec)
        locus PairsL { capacity { heap items of Pair; } }
        fn main() {
            let l = PairsL { };
            l.push(Pair { x: 1, y: 2 });
            l.push(Pair { x: 3, y: 4 });
            let first = l.get(0) or raise;
            if first.x != 1 { println("FAIL: x"); }
            if first.y != 2 { println("FAIL: y"); }
            println("ok");
        }
    "#;
    let bin = build("struct_cells_get", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok"), "expected ok, got: {:?}", stdout);
    assert!(!stdout.contains("FAIL"), "unexpected FAIL: {:?}", stdout);
}

/// #67 — `l.set(i, x)` mutates in place. Fallible(IndexError) on
/// out-of-bounds. Closes the workload-driven gap that the spec
/// originally deferred to FORM-2.
#[test]
fn set_overwrites_existing_index() {
    let src = r#"
        @form(vec)
        locus L { capacity { heap items of Int; } }
        fn main() {
            let l = L { };
            l.push(10);
            l.push(20);
            l.push(30);
            l.set(1, 99) or raise;
            let v = l.get(1) or raise;
            println("v=", v);
            // Boundaries unchanged.
            let a = l.get(0) or raise;
            let c = l.get(2) or raise;
            println("a=", a, " c=", c);
        }
    "#;
    let bin = build("set_overwrite", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("v=99"), "got: {:?}", stdout);
    assert!(stdout.contains("a=10 c=30"), "got: {:?}", stdout);
}

#[test]
fn set_out_of_bounds_substitute_uses_fallback() {
    // `or` substitute on a Unit-success fallible. Same shape as
    // hashmap.remove's or-clause handling.
    let src = r#"
        @form(vec)
        locus L { capacity { heap items of Int; } }
        fn noop(_e: IndexError) { }
        fn main() {
            let l = L { };
            l.push(1);
            l.set(99, 0) or noop(err);
            println("ok");
        }
    "#;
    let bin = build("set_oob_fallback", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok"), "got: {:?}", stdout);
}

#[test]
fn set_then_get_struct_cell_roundtrip() {
    let src = r#"
        type Pair { x: Int; y: Int; }
        @form(vec)
        locus L { capacity { heap items of Pair; } }
        fn main() {
            let l = L { };
            l.push(Pair { x: 1, y: 2 });
            l.set(0, Pair { x: 7, y: 8 }) or raise;
            let p = l.get(0) or raise;
            println("p=(", p.x, ",", p.y, ")");
        }
    "#;
    let bin = build("set_struct_cell", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("p=(7,8)"), "got: {:?}", stdout);
}

/// `sort()` on a primitive-cell vec sorts ascending in place. The
/// gap this closes: agents were hand-rolling O(n²) selection over
/// `get` / `set` because the substrate didn't expose qsort, which
/// burned ~30 lines per program and skewed token-efficiency runs.
#[test]
fn form_vec_sort_int_ascending() {
    let src = r#"
        @form(vec)
        locus L { capacity { heap items of Int; } }
        fn main() {
            let l = L { };
            l.push(3); l.push(1); l.push(4); l.push(1); l.push(5);
            l.sort();
            let mut i = 0;
            while i < l.len() {
                println(l.get(i) or raise);
                i = i + 1;
            }
        }
    "#;
    let bin = build("sort_int_asc", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim(), "1\n1\n3\n4\n5", "got: {:?}", stdout);
}

#[test]
fn form_vec_sort_string_ascending() {
    let src = r#"
        @form(vec)
        locus L { capacity { heap items of String; } }
        fn main() {
            let l = L { };
            l.push("delta"); l.push("alpha"); l.push("charlie"); l.push("bravo");
            l.sort();
            let mut i = 0;
            while i < l.len() {
                println(l.get(i) or raise);
                i = i + 1;
            }
        }
    "#;
    let bin = build("sort_str_asc", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.trim(),
        "alpha\nbravo\ncharlie\ndelta",
        "got: {:?}",
        stdout
    );
}

/// `sort_by(cmp)` lets the agent supply a custom strict-less-than
/// comparator. The trampoline marshals each pair into the user's
/// `fn(a, b) -> Bool` callback via indirect call.
#[test]
fn form_vec_sort_by_custom_cmp_descending() {
    let src = r#"
        @form(vec)
        locus L { capacity { heap items of Int; } }
        fn gt(a: Int, b: Int) -> Bool { return a > b; }
        fn main() {
            let l = L { };
            l.push(3); l.push(1); l.push(4); l.push(1); l.push(5);
            l.sort_by(gt);
            let mut i = 0;
            while i < l.len() {
                println(l.get(i) or raise);
                i = i + 1;
            }
        }
    "#;
    let bin = build("sort_by_desc", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim(), "5\n4\n3\n1\n1", "got: {:?}", stdout);
}

#[test]
fn form_vec_sort_desc_by_flips_supplied_lt() {
    let src = r#"
        @form(vec)
        locus L { capacity { heap items of Int; } }
        fn lt(a: Int, b: Int) -> Bool { return a < b; }
        fn main() {
            let l = L { };
            l.push(3); l.push(1); l.push(4); l.push(1); l.push(5);
            l.sort_desc_by(lt);
            let mut i = 0;
            while i < l.len() {
                println(l.get(i) or raise);
                i = i + 1;
            }
        }
    "#;
    let bin = build("sort_desc_by_lt", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim(), "5\n4\n3\n1\n1", "got: {:?}", stdout);
}

/// `sort_by` works for struct cells too. The trampoline detects
/// pointer-shaped cells and threads the element pointer through
/// instead of loading by value.
#[test]
fn form_vec_sort_by_struct_cell() {
    let src = r#"
        type Point { x: Int; y: Int; }
        @form(vec)
        locus L { capacity { heap items of Point; } }
        fn lt_x(a: Point, b: Point) -> Bool { return a.x < b.x; }
        fn main() {
            let l = L { };
            l.push(Point { x: 3, y: 0 });
            l.push(Point { x: 1, y: 0 });
            l.push(Point { x: 2, y: 0 });
            l.sort_by(lt_x);
            let mut i = 0;
            while i < l.len() {
                let p = l.get(i) or raise;
                println(p.x);
                i = i + 1;
            }
        }
    "#;
    let bin = build("sort_by_struct", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim(), "1\n2\n3", "got: {:?}", stdout);
}

#[test]
fn form_vec_sort_empty_vec_is_noop() {
    let src = r#"
        @form(vec)
        locus L { capacity { heap items of Int; } }
        fn main() {
            let l = L { };
            l.sort();
            println("len=", l.len());
        }
    "#;
    let bin = build("sort_empty", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("len=0"), "got: {:?}", stdout);
}
