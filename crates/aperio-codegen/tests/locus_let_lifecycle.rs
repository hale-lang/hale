//! m82: locus-all-the-way-down lifecycle. Let-bound locus
//! literals defer their dissolve to the enclosing fn's
//! scope-exit flush, so the user-visible binding is the
//! handle and the locus instance lives until that handle
//! goes out of scope.
//!
//! These tests pin the contract independent of the stdlib
//! Stream — pure user-locus shape with custom dissolve()
//! observable via println. The m81 Stream regression test
//! lives in stdlib_stream.rs::stream_let_binding_round_trips
//! _via_send_recv; this file proves the language-level
//! semantics on shapes that don't drag TCP into the picture.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_aperio(name: &str, source: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_let_lifecycle_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn let_bound_locus_dissolves_after_method_calls() {
    // Custom dissolve must fire AFTER all method calls, not
    // before. Pre-m82 ordering would print "dissolve" first
    // because eager-dissolve fires on the struct literal,
    // then methods run on the dissolved locus (UB; usually
    // crashed for resource-bearing dissolves).
    let src = r#"
        locus Counter {
            params { tally: Int = 0; }
            fn bump() {
                self.tally = self.tally + 1;
                println("bump tally=", self.tally);
            }
            dissolve() {
                println("dissolve tally=", self.tally);
            }
        }

        fn main() {
            let c = Counter { tally: 0 };
            c.bump();
            c.bump();
            c.bump();
            println("after-method");
        }
    "#;
    let bin = build_aperio("ordering", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);

    let bump1 = stdout.find("bump tally=1").expect("bump1");
    let bump2 = stdout.find("bump tally=2").expect("bump2");
    let bump3 = stdout.find("bump tally=3").expect("bump3");
    let after = stdout.find("after-method").expect("after-method");
    let dissolve = stdout
        .find("dissolve tally=3")
        .expect("dissolve fires with final state");
    assert!(
        bump1 < bump2 && bump2 < bump3 && bump3 < after && after < dissolve,
        "ordering wrong; got: {:?}",
        stdout
    );
}

#[test]
fn multiple_let_bound_loci_dissolve_in_reverse_instantiation_order() {
    // Two let-bound loci in the same fn. F.4 cascade rule:
    // dissolves fire in reverse-instantiation order at scope
    // exit. The flush_dissolve_frame iterator is `.rev()`,
    // so `b` (instantiated second) dissolves first, then `a`.
    let src = r#"
        locus Marker {
            params { tag: String = ""; }
            dissolve() {
                println("dissolve ", self.tag);
            }
        }

        fn main() {
            let a = Marker { tag: "A" };
            let b = Marker { tag: "B" };
            println("body-end");
        }
    "#;
    let bin = build_aperio("reverse_order", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);

    let body_end = stdout.find("body-end").expect("body-end");
    let dis_b = stdout.find("dissolve B").expect("dissolve B");
    let dis_a = stdout.find("dissolve A").expect("dissolve A");
    assert!(
        body_end < dis_b && dis_b < dis_a,
        "expected body-end < dissolve B < dissolve A; got {:?}",
        stdout
    );
}

#[test]
fn statement_position_locus_literal_still_dissolves_eagerly() {
    // The m82 deferral applies ONLY to let-bound locus literals
    // (the user signaled they want to keep using the handle).
    // Statement-position literals — `Marker { ... };` with no
    // binding — preserve fire-and-forget behavior: birth, run,
    // drain, dissolve all fire at the statement boundary, BEFORE
    // the next statement runs.
    let src = r#"
        locus Marker {
            params { tag: String = ""; }
            dissolve() {
                println("dissolve ", self.tag);
            }
        }

        fn main() {
            Marker { tag: "X" };
            println("after-stmt");
        }
    "#;
    let bin = build_aperio("eager_stmt", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);

    let dis = stdout.find("dissolve X").expect("dissolve X");
    let after = stdout.find("after-stmt").expect("after-stmt");
    assert!(
        dis < after,
        "statement-position dissolve must precede the next statement; \
         got {:?}",
        stdout
    );
}

#[test]
fn let_bound_locus_field_reads_remain_valid_through_fn() {
    // The handle's pointer stays valid for field reads across
    // arbitrarily many statements between the let and end-of-fn.
    // Pre-m82 the locus's arena was destroyed at end of struct-
    // literal-expression, so any field read against `c.X` after
    // that point was a use-after-free.
    let src = r#"
        locus Box {
            params { value: Int = 0; }
        }

        fn main() {
            let b = Box { value: 42 };
            let x = b.value;
            let y = b.value;
            let z = b.value;
            println("sum=", x + y + z);
        }
    "#;
    let bin = build_aperio("field_reads", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("sum=126"),
        "got: {:?}",
        stdout
    );
}
