//! F.30 (2026-05-20): BytesView / StringView as distinct types.
//! Verifies the type-system distinction (coerce-to-Bytes at fn-
//! arg READ sites, reject at storage sites unless explicitly
//! declared as a view) and the explicit clone path
//! (`std::bytes::clone` / `std::str::clone`) for upgrades to
//! owned storage.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_test_bytesview_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

#[test]
fn view_coerces_to_bytes_at_read_sites() {
    // std::bytes::at + len accept BytesView no-op'd through to
    // their underlying Bytes path. No storage of the view, no
    // diagnostic.
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("hello"));
            let v = b.view();
            println("len=", len(v));
            println("b0=", std::bytes::at(v, 0) or -1);
            println("b4=", std::bytes::at(v, 4) or -1);
        }
    "#;
    let (stdout, status) = build_and_run("view_read", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("len=5"), "got: {:?}", stdout);
    assert!(stdout.contains("b0=104"), "got: {:?}", stdout);
    assert!(stdout.contains("b4=111"), "got: {:?}", stdout);
}

#[test]
fn text_view_coerces_to_string_at_read_sites() {
    // println accepts StringView for %s formatting; len(view)
    // routes through lotus_str_len.
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("hello world"));
            let s = b.text_view();
            println("len=", len(s));
            println("s=", s);
        }
    "#;
    let (stdout, status) = build_and_run("text_view_read", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("len=11"), "got: {:?}", stdout);
    assert!(stdout.contains("s=hello world"), "got: {:?}", stdout);
}

#[test]
fn view_stored_as_bytesview_field_works() {
    // Storing as BytesView is the declared-non-owning form;
    // accepted at the type-ascription site, runtime semantics
    // identical to view().
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("hi"));
            let stored: BytesView = b.view();
            println("len=", len(stored));
        }
    "#;
    let (stdout, status) = build_and_run("view_field", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("len=2"), "got: {:?}", stdout);
}

#[test]
fn view_into_bytes_let_rejected() {
    // `let stored: Bytes = b.view()` is the footgun BytesView
    // exists to catch. Storage-site rejects the coercion;
    // codegen errors with the F.30 diagnostic pointing at the
    // explicit clone path.
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("hi"));
            let stored: Bytes = b.view();
            println("unreachable", len(stored));
        }
    "#;
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("lotus_test_bytesview_reject");
    let result = build_executable(&program, &bin);
    let _ = std::fs::remove_file(&bin);
    assert!(
        result.is_err(),
        "expected build error on view → Bytes storage site"
    );
    let msg = format!("{:?}", result.unwrap_err());
    assert!(
        msg.contains("non-owning view")
            || msg.contains("BytesView"),
        "expected F.30 diagnostic to mention view/BytesView: {}",
        msg
    );
}

#[test]
fn bytes_clone_upgrades_view_to_owned() {
    // `std::bytes::clone(view)` deep-copies into caller arena;
    // the resulting Bytes survives subsequent mutations on the
    // source builder (clear, re-append, etc.) — that's the
    // owned-vs-non-owning distinction in action.
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("hello"));
            let owned: Bytes = std::bytes::clone(b.view());
            b.clear();
            b.append(std::bytes::from_string("changed"));
            // `owned` is unchanged despite the builder churn:
            println("owned_len=", len(owned));
            println("buf_now=", b.text_view());
        }
    "#;
    let (stdout, status) = build_and_run("bytes_clone", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("owned_len=5"), "got: {:?}", stdout);
    assert!(stdout.contains("buf_now=changed"), "got: {:?}", stdout);
}

#[test]
fn str_clone_upgrades_text_view_to_owned() {
    // Companion test for std::str::clone: text_view → owned
    // String via deep-copy; survives builder mutation.
    let src = r#"
        fn main() {
            let b = std::bytes::BytesBuilder { initial_cap: 64 };
            b.append(std::bytes::from_string("hello"));
            let owned: String = std::str::clone(b.text_view());
            b.clear();
            b.append(std::bytes::from_string("changed"));
            println("owned=", owned);
            println("buf_now=", b.text_view());
        }
    "#;
    let (stdout, status) = build_and_run("str_clone", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("owned=hello"), "got: {:?}", stdout);
    assert!(stdout.contains("buf_now=changed"), "got: {:?}", stdout);
}
