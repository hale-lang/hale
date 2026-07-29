//! F.30 / F.30b (5a) — view coercion at user-defined fallible-fn
//! and method-call arg sites. The codegen predicate
//! `view_coerces_to` was previously consulted only at non-fallible
//! free-fn arg sites; user-defined fallible fns and method-call
//! sites rejected views with "type mismatch: expected Bytes, got
//! BytesView". E1 adds the consultation at those sites — the
//! BytesView is unpacked (with F.30b epoch check) and the
//! underlying Bytes-shaped ptr is passed.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_view_e1_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

#[test]
fn bytes_view_flows_into_user_fallible_fn_bytes_param() {
    // pond/websocket's `peek_header(b: Bytes, off: Int) ->
    // ... fallible(WsError)` shape. Pre-E1 this would reject
    // with "fallible fn arg 0 type mismatch: expected Bytes,
    // got BytesView" — the user-defined fallible-fn arg site
    // didn't consult view_coerces_to. Post-E1 the view is
    // unpacked at the call site.
    let src = r#"
        fn first_byte(b: Bytes) -> Int fallible(IndexError) {
            return std::bytes::at(b, 0) or raise;
        }

        fn main() {
            let buf = std::bytes::BytesBuilder { initial_cap: 64 };
            buf.append(std::bytes::from_string("hello"));
            let h = first_byte(buf.view()) or 0;
            println("h=", h);
        }
    "#;
    let (stdout, status) = build_and_run("fallible_fn_arg", src);
    assert!(status.success(), "non-zero: {:?}", status);
    // 'h' = 104
    assert!(stdout.contains("h=104"), "got: {:?}", stdout);
}

#[test]
fn string_view_flows_into_user_fallible_fn_string_param() {
    let src = r#"
        fn first_char(s: String) -> Int fallible(IndexError) {
            let b = std::bytes::from_string(s);
            return std::bytes::at(b, 0) or raise;
        }

        fn main() {
            let buf = std::bytes::BytesBuilder { initial_cap: 64 };
            buf.append(std::bytes::from_string("abc"));
            let c = first_char(buf.text_view()) or 0;
            println("c=", c);
        }
    "#;
    let (stdout, status) = build_and_run("fallible_str_arg", src);
    assert!(status.success(), "non-zero: {:?}", status);
    // 'a' = 97
    assert!(stdout.contains("c=97"), "got: {:?}", stdout);
}

#[test]
fn bytes_view_flows_into_method_arg() {
    // The `dest.append_slice(b, lo, hi)` shape where `dest` is a
    // BytesBuilder and `b` is a BytesView. Pre-E1 this would
    // reject at codegen with "{}.{} arg 0 type mismatch:
    // expected Bytes, got BytesView". Post-E1 the view is
    // unpacked and the data ptr is passed.
    let src = r#"
        fn main() {
            let src_buf = std::bytes::BytesBuilder { initial_cap: 64 };
            src_buf.append(std::bytes::from_string("hello world"));
            let dest = std::bytes::BytesBuilder { initial_cap: 64 };
            dest.append_slice(src_buf.view(), 6, 11);
            println("len=", dest.len());
        }
    "#;
    let (stdout, status) = build_and_run("method_arg", src);
    assert!(status.success(), "non-zero: {:?}", status);
    // "world" is 5 chars.
    assert!(stdout.contains("len=5"), "got: {:?}", stdout);
}

#[test]
fn user_non_fallible_fn_already_accepts_view_via_f30() {
    // Sanity: the non-fallible free-fn arg site has been
    // accepting views since F.30 shipped. Run this to confirm
    // E1's changes didn't regress that path.
    let src = r#"
        fn count_b(b: Bytes) -> Int {
            return len(b);
        }

        fn main() {
            let buf = std::bytes::BytesBuilder { initial_cap: 64 };
            buf.append(std::bytes::from_string("abcde"));
            println("n=", count_b(buf.view()));
        }
    "#;
    let (stdout, status) = build_and_run("non_fallible_view", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("n=5"), "got: {:?}", stdout);
}
