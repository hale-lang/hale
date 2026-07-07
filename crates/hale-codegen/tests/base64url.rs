//! `std::text::base64::url_encode` — RFC 4648 §5 URL-safe base64,
//! unpadded. Added for the a downstream handoff (JWT/JWS minting needs
//! base64url; the existing `base64::encode` is the padded standard
//! alphabet). Compiled-path test: the interpreter has no base64
//! encode surface, so this exercises the codegen + C runtime.

use std::process::Command;

use hale_codegen::build_executable;

/// Compile `source` to a temp binary, run it, return stdout.
fn build_and_run(name: &str, source: &str) -> String {
    let program = hale_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_b64url_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn base64url_matches_rfc4648_unpadded() {
    // Expected values are `base64.urlsafe_b64encode(...).rstrip('=')`.
    // Covers: every padding tail (rem 0/1/2), the URL-safe alphabet
    // (`-` at index 62 via ">>>", `_` at 63 via "???"), and empty.
    let src = r#"
fn main() {
    println(std::text::base64::url_encode(std::bytes::from_string("Hello")));
    println(std::text::base64::url_encode(std::bytes::from_string("foobar")));
    println(std::text::base64::url_encode(std::bytes::from_string("any carnal pleasure.")));
    println(std::text::base64::url_encode(std::bytes::from_string(">>>")));
    println(std::text::base64::url_encode(std::bytes::from_string("???")));
    println(std::text::base64::url_encode(std::bytes::from_string("")));
}
"#;
    let out = build_and_run("rfc4648", src);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines,
        vec![
            "SGVsbG8",                       // "Hello"  (rem 2, no pad)
            "Zm9vYmFy",                      // "foobar" (rem 0)
            "YW55IGNhcm5hbCBwbGVhc3VyZS4",   // rem 1, no pad
            "Pj4-",                          // ">>>" -> '-' (index 62)
            "Pz8_",                          // "???" -> '_' (index 63)
            "",                              // empty input -> empty output
        ],
        "base64url output diverged from RFC 4648 §5 unpadded; got: {:?}",
        out
    );
}
