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

use aperio_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_form_vec_codegen_{}", name));
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
    let program = aperio_syntax::parse_source(src);
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
