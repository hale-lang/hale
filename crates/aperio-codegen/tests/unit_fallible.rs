//! A2 (G2) — `-> () fallible(E)` codegen.
//!
//! Before A2, declaring a user fn with Unit success type and a
//! fallible(E) payload (`fn touch() -> () fallible(IoError)`)
//! errored at codegen with "tuple type must have at least 2
//! elements". The fix:
//!
//! 1. Recognize `Some(TypeExpr::Tuple([]))` as Unit (None) ret_ty
//!    at the fn-declaration site.
//! 2. Skip the `out_val` sret slot in both ABI shapes: callee
//!    pushes only `(__caller_arena, params..., out_err) -> i1`;
//!    caller allocates only `out_err_slot` and matches.
//! 3. `FallibleCallResult` carries `out_val_slot: None` /
//!    `success_ty: None`; the `or` machinery already handled this
//!    case for stdlib `() fallible(IoError)` path-calls.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_unit_fallible_{}_{}", name, std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn unit_fallible_ok_path_runs_through() {
    // `fn touch() -> () fallible(IoError)` with no body that calls
    // `fail` should always take the ok path.
    let src = r#"
        fn touch() -> () fallible(IoError) {
        }
        fn main() {
            touch() or raise;
            println("ok");
        }
    "#;
    let (stdout, status) = build_and_run("ok_path", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("ok"), "stdout: {:?}", stdout);
}

#[test]
fn unit_fallible_fail_path_routes_to_substitute() {
    // `fail` in a unit-fallible fn routes the err payload to the
    // caller's `or` clause; the substitute RHS replaces the value
    // (here printed via the err binding's `kind` field).
    let src = r#"
        fn touch() -> () fallible(IoError) {
            fail IoError { kind: "synthetic", errno: 0, path: "/x" };
        }
        fn report(e: IoError) -> () {
            println("got: " + e.kind);
        }
        fn main() {
            touch() or report(err);
            println("after");
        }
    "#;
    let (stdout, status) = build_and_run("fail_substitute", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("got: synthetic"), "stdout: {:?}", stdout);
    assert!(stdout.contains("after"), "stdout: {:?}", stdout);
}

#[test]
fn unit_fallible_or_discard_swallows_err() {
    // `or discard` is the canonical Unit-success swallow form.
    let src = r#"
        fn touch() -> () fallible(IoError) {
            fail IoError { kind: "ignored", errno: 0, path: "" };
        }
        fn main() {
            touch() or discard;
            println("survived");
        }
    "#;
    let (stdout, status) = build_and_run("or_discard", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("survived"), "stdout: {:?}", stdout);
}

#[test]
fn unit_fallible_or_raise_bubbles_to_root() {
    // `or raise` past a unit-fallible call with no enclosing
    // fallible frame should hit lotus_root_panic and exit non-zero.
    let src = r#"
        fn touch() -> () fallible(IoError) {
            fail IoError { kind: "boom", errno: 0, path: "" };
        }
        fn main() {
            touch() or raise;
            println("UNREACHABLE");
        }
    "#;
    let (stdout, status) = build_and_run("or_raise_root", src);
    assert!(!status.success(), "expected non-zero exit");
    assert!(!stdout.contains("UNREACHABLE"), "stdout: {:?}", stdout);
}

#[test]
fn unit_fallible_or_discard_user_error_type() {
    // G2 follow-up — the `or discard` typecheck was special-cased
    // around stdlib `() fallible(IoError)`; user-declared error
    // types went through `resolve_type_expr` which built
    // `Ty::Tuple([])` instead of `Ty::Unit` for `-> ()`, and the
    // check rejected the tuple. Fixed by normalizing the empty
    // tuple to Unit at the resolver. This test pins the gap shut.
    let src = r#"
        type E { kind: String; detail: String; }
        fn touch() -> () fallible(E) {
            fail E { kind: "synthetic", detail: "user-type fail" };
        }
        fn main() {
            touch() or discard;
            println("survived user-type discard");
        }
    "#;
    let (stdout, status) = build_and_run("or_discard_user_e", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("survived user-type discard"),
        "stdout: {:?}",
        stdout
    );
}

#[test]
fn unit_fallible_chains_through_fallible_caller() {
    // A unit-fallible fn calling another unit-fallible fn with
    // `or raise` should propagate the err through the static
    // call stack without ever allocating an out_val slot.
    let src = r#"
        fn leaf() -> () fallible(IoError) {
            fail IoError { kind: "deep", errno: 0, path: "" };
        }
        fn mid() -> () fallible(IoError) {
            leaf() or raise;
        }
        fn main() {
            mid() or println("err kind=" + err.kind);
            println("done");
        }
    "#;
    let (stdout, status) = build_and_run("chain", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("err kind=deep"), "stdout: {:?}", stdout);
    assert!(stdout.contains("done"), "stdout: {:?}", stdout);
}
