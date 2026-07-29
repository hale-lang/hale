//! C4 — `std::os::getrandom`.
//!
//! Cryptographically-strong random bytes via `getrandom(2)` with
//! `/dev/urandom` fallback. The path-call returns `Bytes
//! fallible(IoError)`; tests exercise:
//!
//! 1. Happy-path 32-byte request returns 32 bytes.
//! 2. `n == 0` returns an empty Bytes (no error path).
//! 3. Two successive calls return different bytes (statistical
//!    certainty — 2^-256 collision probability for the 32-byte
//!    case).
//! 4. `n < 0` is treated as "empty" (no error), matching the C
//!    contract. (The friction-log ask explicitly admits empty
//!    Bytes on `n <= 0` so callers don't have to branch.)
//! 5. `n > 8192` errors with `IoError.kind == "invalid"`.
//!
//! Resolves pond/crypto FRICTION "no-csprng-getrandom" — backing
//! primitive for `random_bytes` flipping from xorshift64 to a
//! real CSPRNG.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(
    name: &str,
    src: &str,
) -> (String, String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!(
        "hale_test_os_getrandom_{}_{}",
        name,
        std::process::id()
    ));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status,
    )
}

#[test]
fn getrandom_32_returns_32_bytes() {
    // The happy path. Print `len=<n>` so we can assert the
    // length without depending on the (random) byte contents.
    let src = r#"
        fn main() {
            let b = std::os::getrandom(32) or raise;
            println("len=", len(b));
        }
    "#;
    let (stdout, _stderr, status) = build_and_run("len32", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("len=32"),
        "expected len=32; got: {:?}",
        stdout
    );
}

#[test]
fn getrandom_zero_returns_empty() {
    // `n == 0` is the "give me nothing" edge — empty Bytes, no
    // error. Matches the C contract and the friction-log ask.
    let src = r#"
        fn main() {
            let b = std::os::getrandom(0) or raise;
            println("len=", len(b));
        }
    "#;
    let (stdout, _stderr, status) = build_and_run("zero", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("len=0"),
        "expected len=0; got: {:?}",
        stdout
    );
}

#[test]
fn getrandom_two_calls_differ() {
    // Two 32-byte draws should differ. With 256 bits of
    // independent entropy per draw the collision probability is
    // 2^-256 — i.e. never. We surface the difference by printing
    // the first byte of each, but we also check that the
    // *Bytes* objects differ via a byte-level loop comparison.
    //
    // Stdout shape: print "diff=true" if any byte differs.
    let src = r#"
        fn main() {
            let a = std::os::getrandom(32) or raise;
            let b = std::os::getrandom(32) or raise;
            let n = len(a);
            let i = 0;
            let diff = false;
            while i < n {
                let ai = std::bytes::at(a, i) or 0;
                let bi = std::bytes::at(b, i) or 0;
                if ai != bi {
                    diff = true;
                }
                i = i + 1;
            }
            if diff {
                println("diff=true");
            } else {
                println("diff=false");
            }
        }
    "#;
    let (stdout, _stderr, status) = build_and_run("differ", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("diff=true"),
        "expected two CSPRNG draws to differ; got: {:?}",
        stdout
    );
}

#[test]
fn getrandom_negative_returns_empty() {
    // Negative n is treated as the "empty" sentinel — no error.
    let src = r#"
        fn main() {
            let b = std::os::getrandom(-1) or raise;
            println("len=", len(b));
        }
    "#;
    let (stdout, _stderr, status) = build_and_run("negative", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("len=0"),
        "expected len=0 for n=-1; got: {:?}",
        stdout
    );
}

#[test]
fn getrandom_too_large_surfaces_invalid() {
    // The per-call cap (8192) is the ergonomic floor. Asking for
    // more should surface `IoError.kind == "invalid"` so the
    // agent gets a typed signal rather than a silent failure.
    // The `or` substitute prints the kind+path and returns an
    // empty Bytes (zero-length is the natural "no data" shape on
    // this surface) to keep the let-binding's value-arm and
    // err-arm shapes aligned.
    let src = r#"
        fn report(e: IoError) -> Bytes {
            println("kind=", e.kind);
            println("path=", e.path);
            return std::os::getrandom(0) or raise;
        }
        fn main() {
            let _b = std::os::getrandom(9000) or report(err);
        }
    "#;
    let (stdout, _stderr, status) =
        build_and_run("too_large", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("kind=invalid"),
        "expected kind=invalid for over-cap n; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("path=std::os::getrandom"),
        "expected the surface label in IoError.path; got: {:?}",
        stdout
    );
}
