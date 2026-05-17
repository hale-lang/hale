//! C2 — `std::process::run`.
//!
//! Synchronous fork+exec+wait. Captures stdout, stderr, and exit
//! code in one call. Exercises:
//!
//! 1. Happy-path stdout capture via `echo hello` — output round-
//!    trips through the captured String.
//! 2. Non-zero exit code via `false` — IoError isn't raised, the
//!    ProcessOutput just carries `code != 0`.
//! 3. ENOENT exec failure via a guaranteed-nonexistent path —
//!    surfaces as IoError with `kind="not_found"`.
//! 4. stderr capture via `sh -c 'echo oops 1>&2; exit 3'` — both
//!    stderr and the exit code populate correctly.
//!
//! Resolves pond/subprocess FRICTION "no-std-process-run".

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(
    name: &str,
    src: &str,
) -> (String, String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "aperio_test_process_run_{}_{}",
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
fn run_echo_captures_stdout() {
    // `echo hello` writes "hello\n" to stdout, exits with code 0.
    // The captured ProcessOutput.stdout field carries the
    // trailing newline because `echo` prints one.
    let src = r#"
        fn main() {
            let out = std::process::run("echo\nhello") or raise;
            println("code=", out.code);
            println("signal=", out.signal);
            println("stdout=", out.stdout);
            println("stderr_len=", len(out.stderr));
        }
    "#;
    let (stdout, _stderr, status) = build_and_run("echo", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("code=0"),
        "expected code=0; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("signal=0"),
        "expected signal=0; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("stdout=hello\n"),
        "expected captured 'hello\\n'; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("stderr_len=0"),
        "expected empty stderr; got: {:?}",
        stdout
    );
}

#[test]
fn run_false_returns_nonzero_code() {
    // `false` exits with code 1 — no IoError raised; the user
    // inspects ProcessOutput.code to detect the failure.
    let src = r#"
        fn main() {
            let out = std::process::run("false") or raise;
            println("code=", out.code);
        }
    "#;
    let (stdout, _stderr, status) = build_and_run("false", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("code=1"),
        "expected code=1 from `false`; got: {:?}",
        stdout
    );
}

#[test]
fn run_nonexistent_command_surfaces_not_found() {
    // exec of a nonexistent path surfaces IoError with
    // kind="not_found" — same shape as fs::read_file on a
    // missing path.
    let src = r#"
        fn report(e: IoError) -> std::process::ProcessOutput {
            println("kind=", e.kind);
            println("path=", e.path);
            return std::process::run("true") or raise;
        }
        fn main() {
            let _out = std::process::run("/no/such/aperio_c2_test_cmd")
                or report(err);
        }
    "#;
    let (stdout, _stderr, status) =
        build_and_run("not_found", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("kind=not_found"),
        "expected kind=not_found; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("path=std::process::run"),
        "expected surface label in IoError.path; got: {:?}",
        stdout
    );
}

#[test]
fn run_captures_stderr_and_exit_code() {
    // `sh -c 'echo oops 1>&2; exit 3'` writes "oops" to stderr
    // and exits with code 3. Both fields populate.
    let src = r#"
        fn main() {
            let out = std::process::run("sh\n-c\necho oops 1>&2; exit 3")
                or raise;
            println("code=", out.code);
            println("stderr=", out.stderr);
            println("stdout_len=", len(out.stdout));
        }
    "#;
    let (stdout, _stderr, status) =
        build_and_run("stderr_and_code", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("code=3"),
        "expected code=3; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("stderr=oops\n"),
        "expected captured 'oops\\n' on stderr; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("stdout_len=0"),
        "expected empty stdout; got: {:?}",
        stdout
    );
}

#[test]
fn run_passes_multi_arg_argv() {
    // Multi-arg argv: `printf "%s-%s\n" a b` → "a-b\n". Verifies
    // newline-split argv survives the split-and-exec round trip.
    let src = r#"
        fn main() {
            let out = std::process::run("printf\n%s-%s\na\nb") or raise;
            println("stdout=", out.stdout);
        }
    "#;
    let (stdout, _stderr, status) = build_and_run("printf", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("stdout=a-b\n"),
        "expected 'a-b\\n'; got: {:?}",
        stdout
    );
}
