//! WASM plan — stdlib target-gating (Phase 2). A program declaring
//! `target wasm` (or `browser_js`) may not call the POSIX-only stdlib
//! (no syscalls in the browser sandbox); typecheck rejects it with
//! guidance. The portable surface (str/bytes/json/math/...) is allowed,
//! and a program with no `target` decl is never gated.

use hale_syntax::parse_source;
use hale_types::check_program;

fn check(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

#[test]
fn target_wasm_rejects_posix_stdlib() {
    // One representative call per gated namespace.
    let cases = [
        ("std::io::fs::write_file(\"/x\", \"h\") or raise;", "std::io::fs::write_file"),
        ("let _ = std::io::tcp::connect(\"h\", 80) or raise;", "std::io::tcp::connect"),
        ("std::io::tls::close(0);", "std::io::tls::close"),
        ("let _ = std::term::is_tty(1);", "std::term::is_tty"),
        ("let _ = std::process::pid();", "std::process::pid"),
    ];
    for (call, path) in cases {
        let src = format!("target wasm {{ }}\nfn main() {{ {} }}", call);
        let msgs = check(&src);
        assert!(
            msgs.iter().any(|m| m.contains(path) && m.contains("target wasm")),
            "expected a `target wasm` gating diagnostic for `{}`, got: {:?}",
            path,
            msgs
        );
    }
}

#[test]
fn target_wasm_allows_portable_stdlib() {
    let src = r#"
        target wasm { }
        fn main() {
            let n = std::str::parse_int("42") or 0;
            let b = std::bytes::BytesBuilder { };
            b.append_u32_le(n);
            println("n=", n);
        }
    "#;
    let msgs = check(src);
    assert!(
        msgs.is_empty(),
        "portable stdlib must not be gated under target wasm, got: {:?}",
        msgs
    );
}

#[test]
fn no_target_decl_does_not_gate() {
    // The same fs call is fine with no `target` decl (native intent).
    let src = r#"fn main() { std::io::fs::write_file("/x", "h") or raise; }"#;
    let msgs = check(src);
    assert!(
        !msgs.iter().any(|m| m.contains("target wasm")),
        "a program with no `target` decl must not be wasm-gated, got: {:?}",
        msgs
    );
}
