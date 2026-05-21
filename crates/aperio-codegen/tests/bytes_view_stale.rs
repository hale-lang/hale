//! F.30b (2026-05-20): view-stale runtime guard.
//!
//! BytesBuilder gains a monotonic `mutation_epoch` field bumped
//! by every mutating op (append / append_slice / shift_front /
//! clear / advance). view() and text_view() return a 16-byte
//! by-value struct (`{src, epoch}`) — no arena allocation.
//! Read-site coercions (`view_coerces_to` and the println / len
//! builtin arms) unpack via lotus_bytes_view_data /
//! lotus_str_view_data, which compare the view's stamped epoch
//! against the builder's current epoch and `_exit(1)` with a
//! clear diagnostic on stderr on mismatch.
//!
//! These tests exercise the panic path — a view captured before
//! a mutation, then read after, should exit non-zero with
//! "view_stale" on stderr. The OK path (read fresh view; capture-
//! then-discard before mutation; etc.) is covered by the existing
//! bytes_builder_view tests.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> std::process::Output {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_view_stale_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    out
}

#[test]
fn bytes_view_held_across_append_panics_on_read() {
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("hello"));
            let v = b.view();
            b.append(std::bytes::from_string(" world"));
            // v was captured before the second append. Reading
            // through it now should panic via lotus_view_stale_panic.
            println("len=", len(v));
            println("unreachable");
        }
    "#;
    let out = build_and_run("bytes_held_across_append", src);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "expected non-zero exit on stale read; stdout: {:?} stderr: {:?}",
        String::from_utf8_lossy(&out.stdout),
        stderr,
    );
    assert!(
        stderr.contains("BytesView read after source BytesBuilder mutated"),
        "expected stale diagnostic on stderr: {:?}",
        stderr,
    );
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("unreachable"),
        "main should not have reached the unreachable println; stdout: {:?}",
        out.stdout,
    );
}

#[test]
fn bytes_view_held_across_shift_front_panics_on_read() {
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("AAAA-BBBB"));
            let v = b.view();
            b.shift_front(5);
            let x = std::bytes::at(v, 0) or -1;
            println("x=", x);
        }
    "#;
    let out = build_and_run("bytes_held_across_shift", src);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(
        stderr.contains("BytesView"),
        "expected BytesView diagnostic on stderr: {:?}",
        stderr,
    );
}

#[test]
fn bytes_view_held_across_clear_panics_on_read() {
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("payload"));
            let v = b.view();
            b.clear();
            println("len=", len(v));
        }
    "#;
    let out = build_and_run("bytes_held_across_clear", src);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("BytesView"),
        "expected stale diagnostic"
    );
}

#[test]
fn text_view_held_across_append_panics_on_read() {
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("hi"));
            let t = b.text_view();
            b.append(std::bytes::from_string(" there"));
            println("t=", t);
        }
    "#;
    let out = build_and_run("text_held_across_append", src);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(
        stderr.contains("StringView read after source BytesBuilder mutated"),
        "expected StringView stale diagnostic: {:?}",
        stderr,
    );
}

#[test]
fn fresh_view_after_mutation_works() {
    // The complement of the panic tests: capturing a view AFTER
    // the mutation is the supported pattern — the stamped epoch
    // matches the current epoch, no panic.
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("hello"));
            b.append(std::bytes::from_string(" world"));
            let v = b.view();
            println("len=", len(v));
        }
    "#;
    let out = build_and_run("fresh_after_mutation", src);
    assert!(out.status.success(), "expected clean run: {:?}", out);
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("len=11"),
        "got: {:?}",
        String::from_utf8_lossy(&out.stdout),
    );
}

#[test]
fn view_then_consume_then_mutate_works() {
    // Capture, consume, THEN mutate — the consumption happens at
    // the println call site, before the mutation. The view's
    // lifetime is effectively the println call's argument
    // evaluation. Past that, the builder can be safely mutated.
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("hello"));
            println("len=", len(b.view()));
            b.append(std::bytes::from_string(" world"));
            println("after_len=", len(b.view()));
        }
    "#;
    let out = build_and_run("consume_then_mutate", src);
    assert!(out.status.success(), "expected clean run: {:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("len=5"), "got: {:?}", stdout);
    assert!(stdout.contains("after_len=11"), "got: {:?}", stdout);
}
