//! v1.x-FORM-5 — `@form(ring_buffer)` codegen.
//!
//! The structural lowering: a `@form(ring_buffer, cap = N)`
//! locus's pool slot becomes an inline
//! `{ i64 cap, i64 head, i64 len, i64 elem_size, ptr buf }`
//! struct managed by `lotus_ring_buffer_*`. `cap` is baked in at
//! `lotus_ring_buffer_init` from the annotation arg; the buffer
//! is pre-allocated at locus birth and never grows.

use std::process::Command;

use aperio_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_form_rb_codegen_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn form_ring_buffer_push_pop_round_trip() {
    let src = r#"
        @form(ring_buffer, cap = 4)
        locus RB {
            capacity { pool history of Int; }
        }
        fn main() {
            let rb = RB { };
            let _ = rb.push(10);
            let _ = rb.push(20);
            let _ = rb.push(30);
            let a = rb.pop() or raise;
            let b = rb.pop() or raise;
            print(a); print(" "); println(b);
        }
    "#;
    let bin = build("push_pop", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("10 20"), "FIFO order broken: {:?}", stdout);
}

#[test]
fn form_ring_buffer_push_returns_false_when_full() {
    let src = r#"
        @form(ring_buffer, cap = 2)
        locus RB {
            capacity { pool history of Int; }
        }
        fn main() {
            let rb = RB { };
            print(rb.push(1)); print(" ");
            print(rb.push(2)); print(" ");
            print(rb.push(3)); print(" ");
            print(rb.push(4)); println("");
        }
    "#;
    let bin = build("push_full", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("true true false false"),
        "expected first two pushes to succeed, rest to fail: {:?}",
        stdout
    );
}

#[test]
fn form_ring_buffer_len_and_is_full() {
    let src = r#"
        @form(ring_buffer, cap = 3)
        locus RB {
            capacity { pool history of Int; }
        }
        fn main() {
            let rb = RB { };
            print(rb.len()); print(" ");
            print(rb.is_full()); print(" ");
            let _ = rb.push(1);
            let _ = rb.push(2);
            let _ = rb.push(3);
            print(rb.len()); print(" ");
            print(rb.is_full()); println("");
        }
    "#;
    let bin = build("len_is_full", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("0 false 3 true"), "got: {:?}", stdout);
}

#[test]
fn form_ring_buffer_pop_empty_substitutes() {
    let src = r#"
        @form(ring_buffer, cap = 4)
        locus RB {
            capacity { pool history of Int; }
        }
        fn main() {
            let rb = RB { };
            let v = rb.pop() or 99;
            println(v);
        }
    "#;
    let bin = build("pop_empty", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("99"), "got: {:?}", stdout);
}

#[test]
fn form_ring_buffer_pop_empty_with_err_binding() {
    let src = r#"
        @form(ring_buffer, cap = 4)
        locus RB {
            capacity { pool history of Int; }
        }
        fn report(e: EmptyError) -> Int { print("kind="); println(e.kind); return -1; }
        fn main() {
            let rb = RB { };
            let v = rb.pop() or report(err);
            println(v);
        }
    "#;
    let bin = build("pop_err_binding", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("kind=empty"), "got: {:?}", stdout);
    assert!(stdout.contains("-1"), "got: {:?}", stdout);
}

#[test]
fn form_ring_buffer_fifo_wrap_around() {
    let src = r#"
        @form(ring_buffer, cap = 3)
        locus RB {
            capacity { pool history of Int; }
        }
        fn main() {
            let rb = RB { };
            let _ = rb.push(1);
            let _ = rb.push(2);
            let _ = rb.push(3);
            let a = rb.pop() or raise;       // 1
            let b = rb.pop() or raise;       // 2
            let _ = rb.push(4);
            let _ = rb.push(5);
            let c = rb.pop() or raise;       // 3
            let d = rb.pop() or raise;       // 4
            let e = rb.pop() or raise;       // 5
            print(a); print(" "); print(b); print(" ");
            print(c); print(" "); print(d); print(" "); println(e);
        }
    "#;
    let bin = build("wrap_around", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("1 2 3 4 5"), "wrap-around order: {:?}", stdout);
}

#[test]
fn form_ring_buffer_struct_cells() {
    let src = r#"
        type Sample { v: Int; }
        @form(ring_buffer, cap = 2)
        locus RB {
            capacity { pool history of Sample; }
        }
        fn main() {
            let rb = RB { };
            let _ = rb.push(Sample { v: 7 });
            let _ = rb.push(Sample { v: 9 });
            let a = rb.pop() or raise;
            print(a.v); print(" ");
            let b = rb.pop() or raise;
            println(b.v);
        }
    "#;
    let bin = build("struct_cells", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("7 9"), "struct cells: {:?}", stdout);
}
