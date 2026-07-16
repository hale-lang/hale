//! Item 5 (downstream handoff 2026-07-15) — an UNADDRESSED fallible
//! stdlib `Stream` method call must be rejected with a clean,
//! actionable error, NOT an LLVM verifier ICE.
//!
//! After #209 migrated `Stream.send` / `send_bytes` / `recv` /
//! `recv_bytes` to `fallible(IoError)`, a call site that omits the
//! `or` clause (a bare statement, or a plain value-binding) reached
//! codegen's non-fallible method-call lowering and emitted a call to
//! the fallible callee with the wrong arity — surfacing only as
//! `module verification failed ... Incorrect number of arguments
//! passed to called function`. The typechecker can't catch it because
//! a `std::io::tcp::Stream` literal types as `Unknown` there (stdlib
//! handle loci aren't in the type table), so codegen is the backstop.
//! It now rejects the call by name instead of emitting invalid IR.

use hale_codegen::build_executable;

fn build_err(src: &str) -> Option<String> {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale-stream-unaddr-{}", std::process::id()));
    match build_executable(&program, &bin) {
        Ok(()) => {
            let _ = std::fs::remove_file(&bin);
            None
        }
        Err(e) => Some(e.to_string()),
    }
}

#[test]
fn bare_fallible_send_bytes_statement_rejected_cleanly() {
    let err = build_err(
        r#"
        fn main() {
            let s = std::io::tcp::Stream { conn_fd: 0 - 1, owns_fd: false };
            s.send_bytes(std::bytes::from_string("x"));
        }
    "#,
    )
    .expect("bare fallible send_bytes must not build");
    assert!(
        err.contains("error not addressed") && err.contains("send_bytes"),
        "expected an actionable 'error not addressed' message naming the \
         method; got: {}",
        err
    );
    // The whole point: it must NOT be the raw LLVM verifier dump.
    assert!(
        !err.contains("module verification failed")
            && !err.contains("Incorrect number of arguments"),
        "regressed to the LLVM verifier ICE instead of a clean error: {}",
        err
    );
}

#[test]
fn bare_fallible_recv_value_binding_rejected_cleanly() {
    // recv has a String success value, so the "returns no value" guard
    // doesn't catch it — it too used to ICE.
    let err = build_err(
        r#"
        fn main() {
            let s = std::io::tcp::Stream { conn_fd: 0 - 1, owns_fd: false };
            let got = s.recv(64);
            println(got);
        }
    "#,
    )
    .expect("unaddressed fallible recv must not build");
    assert!(
        err.contains("error not addressed") && err.contains("recv"),
        "expected 'error not addressed' naming recv; got: {}",
        err
    );
    assert!(
        !err.contains("module verification failed"),
        "regressed to the LLVM verifier ICE: {}",
        err
    );
}

#[test]
fn addressed_fallible_calls_still_build() {
    // The `or`-addressed path routes through a different lowering and
    // must be unaffected by the reject above.
    let err = build_err(
        r#"
        fn main() {
            let s = std::io::tcp::Stream { conn_fd: 0 - 1, owns_fd: false };
            s.send("x") or discard;
            s.send_bytes(std::bytes::from_string("y")) or discard;
            let got = s.recv(64) or "FB";
            println("ok ", got);
        }
    "#,
    );
    assert!(
        err.is_none(),
        "addressed fallible Stream calls must still build; got error: {:?}",
        err
    );
}
