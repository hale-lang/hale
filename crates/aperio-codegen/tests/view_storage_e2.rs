//! F.30b (5b) — String/Bytes literal default coercion to
//! StringView/BytesView at storage-site defaults. F.30's
//! storage-site type discipline rejects `let x: StringView = "";`
//! and `WsMessage.text: StringView = "";` patterns because the
//! literal types as String, not StringView. E2 carves out an
//! exception for *literal* defaults: the underlying data lives
//! in the global string table at program-lifetime, so wrapping
//! it in a view struct with the static-epoch sentinel is
//! structurally safe. The unpack helper sees the sentinel and
//! returns `src` directly (the literal's data ptr) without an
//! epoch check.
//!
//! Non-literal expressions (e.g. `let x: StringView = some_fn();`)
//! still reject — those values might not have program-lifetime
//! and shouldn't silently bypass the F.30 owned-vs-view distinction.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_view_e2_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

#[test]
fn struct_field_string_view_default_empty_literal_works() {
    // The captured-message pattern: a struct field declared
    // `text: StringView = ""`. Pre-E2 this rejected at codegen.
    let src = r#"
        type Msg {
            text: StringView = "";
            fin: Bool = false;
        }

        fn main() {
            let m = Msg { };
            println("fin=", m.fin);
            println("text=", m.text);
        }
    "#;
    let (stdout, status) = build_and_run("struct_default_empty", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("fin=false"), "got: {:?}", stdout);
    assert!(stdout.contains("text="), "got: {:?}", stdout);
}

#[test]
fn struct_field_string_view_default_nonempty_literal_works() {
    // Non-empty String literals are also program-lifetime, so
    // they qualify for the same coercion. The "empty" framing in
    // the friction was a partial story; the generalization is
    // "any String/Bytes literal."
    let src = r#"
        type Tag {
            label: StringView = "default";
        }

        fn main() {
            let t = Tag { };
            println("label=", t.label);
        }
    "#;
    let (stdout, status) = build_and_run("struct_default_nonempty", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("label=default"), "got: {:?}", stdout);
}

#[test]
fn struct_field_bytes_view_default_literal_works() {
    let src = r#"
        type Frame {
            payload: BytesView = b"";
        }

        fn main() {
            let f = Frame { };
            println("len=", len(f.payload));
        }
    "#;
    let (stdout, status) = build_and_run("struct_default_bytes", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("len=0"), "got: {:?}", stdout);
}

#[test]
fn locus_field_string_view_default_empty_literal_works() {
    // Same coercion at a locus param site. The locus's prefix
    // field is StringView-typed; the default is the literal
    // "hello, ". Field read goes through the unpack-with-NULL-
    // builder path which returns the underlying String ptr.
    let src = r#"
        locus Greeter {
            params {
                prefix: StringView = "hello, ";
            }
            fn show() {
                println("prefix=", self.prefix);
            }
        }

        fn main() {
            let g = Greeter { };
            g.show();
        }
    "#;
    let (stdout, status) = build_and_run("locus_default", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("prefix=hello, "), "got: {:?}", stdout);
}

#[test]
fn struct_field_view_default_then_overridden_with_view_works() {
    // The full pipeline: a struct declares `text: StringView = ""`
    // for the construction-site default, but the actual fill is
    // a real view from a BytesBuilder. Mirrors the
    // pond/websocket `text: frag_buf.text_view()` shape — the
    // default is a placeholder; the real value is a builder view.
    let src = r#"
        type Msg {
            text: StringView = "";
        }

        fn main() {
            let buf = std::bytes::BytesBuilder { initial_cap: 64 };
            buf.append(std::bytes::from_string("hello"));
            let m = Msg { text: buf.text_view() };
            println("len=", len(m.text));
        }
    "#;
    let (stdout, status) = build_and_run("override_with_view", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("len=5"), "got: {:?}", stdout);
}
