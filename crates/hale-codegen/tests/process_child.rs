//! C2 — `std::process::Child` lifecycle.
//!
//! Async subprocess via `spawn` / `wait` / `kill` / `write_stdin` /
//! `read_stdout` / `read_stderr`. The `Child` locus's dissolve()
//! reaps any unwaited child (TERM → wait 100ms → KILL → waitpid),
//! for a handle that an owner reclaims — see `dissolve_reaps_unwaited_child`
//! for which handles those are.
//!
//! Tests:
//!
//! 1. `spawn` → `wait` happy path on `true` — exit code 0.
//! 2. `spawn` → `read_stdout` after exit — captures output via
//!    non-blocking read after the child has closed stdout.
//! 3. `spawn` → `write_stdin` → `wait` — round-trips data into a
//!    `cat`-style filter via stdin and out via stdout.
//! 4. `kill` against a long-running `sleep 60` returns promptly
//!    (within ~200ms grace; well under the 60s the child would
//!    otherwise run).
//! 5. an owner's dissolve reaps an unwaited child — the pid is gone
//!    after the program exits, checked from outside it.
//!
//! Resolves pond/subprocess FRICTION "no-async-child-lifecycle"
//! and pond/agent/sandbox FRICTION "no-supervised-subprocess".

use std::process::Command;
use std::time::Instant;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(
    name: &str,
    src: &str,
) -> (String, String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!(
        "hale_test_process_child_{}_{}",
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
fn spawn_true_wait_yields_zero() {
    // Spawn `true` (exits immediately with code 0), wait, observe
    // the exit code. The simplest possible lifecycle test.
    let src = r#"
        fn main() {
            let c = std::process::spawn("true") or raise;
            let code = std::process::wait(c) or raise;
            println("code=", code);
        }
    "#;
    let (stdout, stderr, status) = build_and_run("spawn_true", src);
    assert!(
        status.success(),
        "non-zero exit: {:?}, stderr: {}",
        status,
        stderr
    );
    assert!(
        stdout.contains("code=0"),
        "expected code=0; got: {:?}",
        stdout
    );
}

#[test]
fn spawn_stdin_pipe_through_cat_round_trips() {
    // `cat` reads stdin → writes to stdout. We write a known
    // payload via write_stdin and read the echo via
    // read_stdout, then kill the child so the pipe-close
    // unblocks cat. The non-blocking read may need to poll a
    // few times to pick up the bytes that cat wrote between
    // the write_stdin and the kill — the small while loop is
    // bounded so a wedged child still terminates the test.
    let src = r#"
        fn main() {
            let c = std::process::spawn("cat") or raise;
            let _n = std::process::write_stdin(c, "hello-stdin\n") or raise;
            // Poll stdout for up to ~100 iterations so the child
            // has time to echo our line back. Each std::time::sleep
            // is 5ms, total budget 500ms.
            let mut got = "";
            let mut tries = 0;
            while tries < 100 {
                let chunk = std::process::read_stdout(c) or raise;
                got = got + chunk;
                if len(got) > 0 {
                    tries = 100;
                } else {
                    std::time::sleep(5ms);
                    tries = tries + 1;
                }
            }
            // Kill cat — kill_escalate also reaps via waitpid, so
            // we don't follow with an explicit wait() (which would
            // race against the kill_escalate's own waitpid and
            // surface ECHILD). dissolve() on scope exit would
            // also work, but kill makes the intent explicit.
            std::process::kill(c) or raise;
            println("got=", got);
        }
    "#;
    let (stdout, stderr, status) =
        build_and_run("stdin_cat", src);
    assert!(
        status.success(),
        "non-zero exit: {:?}, stderr: {}",
        status,
        stderr
    );
    assert!(
        stdout.contains("got=hello-stdin\n"),
        "expected echoed payload; got: {:?}",
        stdout
    );
}

#[test]
fn kill_on_long_running_returns_promptly() {
    // `sleep 60` would block the parent for a minute if we
    // weren't killing it. `kill` should escalate within the
    // 100ms TERM grace + KILL → reap loop; well under 60s.
    // The Rust harness measures wall time around the whole
    // subprocess to confirm.
    let src = r#"
        fn main() {
            let c = std::process::spawn("sleep\n60") or raise;
            std::process::kill(c) or raise;
            println("killed");
        }
    "#;
    let start = Instant::now();
    let (stdout, stderr, status) = build_and_run("kill_sleep", src);
    let elapsed = start.elapsed();
    assert!(
        status.success(),
        "non-zero exit: {:?}, stderr: {}",
        status,
        stderr
    );
    assert!(
        stdout.contains("killed"),
        "expected killed marker; got: {:?}",
        stdout
    );
    // The kill should finish well under 60s. Allow generous
    // headroom for CI / slow runners; the kill_escalate window
    // is 100ms + the SIGKILL waitpid, typically <200ms in total
    // but we cap at 10s to be defensive.
    assert!(
        elapsed.as_secs() < 10,
        "kill took {:?}, expected < 10s",
        elapsed
    );
}

/// True while `pid` is still a `sleep` in the process table — either
/// running or a zombie nobody reaped. The `comm` field of procfs's
/// per-process status line is the right place to look: a zombie keeps
/// its comm but has an EMPTY command line, so a command-line check
/// reads an unreaped child as gone. A recycled pid running something
/// else reads as gone too, which is the conservative direction.
fn sleep_alive(pid: &str) -> bool {
    let p = std::path::Path::new("/proc").join(pid).join("stat");
    match std::fs::read_to_string(&p) {
        Ok(line) => line.contains("(sleep)"),
        Err(_) => false,
    }
}

#[test]
fn dissolve_reaps_unwaited_child() {
    // A Child whose owner is reclaimed must be killed + reaped by
    // that owner's dissolve, so nothing is left running once the
    // program is gone.
    //
    // This test used to assert only that the program exited promptly
    // — "which *is* dissolve() running before process exit and
    // reaping". It is not: a program that spawns and never waits
    // exits promptly whether or not anything reaped, so the
    // assertion held vacuously while `sleep 60` was left ORPHANED.
    // The claim is now checked where it is observable: from outside,
    // after the parent is gone.
    //
    // The shape is the one that HAS an owner (GH #716): the handle is
    // moved into a locus's `params` field with
    // `std::process::adopt`, and the owner's dissolve tears it down.
    // A handle that is only `let`-bound from the factory does NOT
    // reap today, and cannot be used to assert this: an instantiation
    // that escapes by `return` is deliberately not pushed onto the
    // caller's `deferred_dissolves` frame (`returns_this_locus` in
    // `locus/instantiation.rs`, the m90 3f decision — "the locus
    // leaks … live until process exit"), and the `let` at the call
    // site registers nothing either, so no teardown runs for it
    // anywhere. That is the GH #383 factory-return class, reported
    // separately; `spec/memory.md` § Bound handles states it.
    let src = r#"
        locus Job {
            params { child: std::process::Child = std::process::Child { }; }
            fn start(argv: String) {
                let spawned = std::process::spawn(argv) or std::process::Child { };
                std::process::adopt(self.child, spawned);
            }
            fn pid() -> Int { return self.child.pid; }
        }
        fn main() {
            let j = Job { };
            j.start("sleep\n30");
            println(j.pid());
        }
    "#;
    let start = Instant::now();
    let (stdout, stderr, status) =
        build_and_run("dissolve_reaps", src);
    let elapsed = start.elapsed();
    assert!(
        status.success(),
        "non-zero exit: {:?}, stderr: {}",
        status,
        stderr
    );
    let pid = stdout.trim();
    assert!(
        pid.parse::<i64>().map(|n| n > 0).unwrap_or(false),
        "expected the spawned pid on stdout; got: {:?}",
        stdout
    );
    // The parent has exited. The child must not still be running.
    assert!(
        !sleep_alive(pid),
        "the child ({}) outlived the program — the owner's dissolve \
         did not kill + reap it",
        pid
    );
    // And the teardown must not block on the child's own 30s.
    assert!(
        elapsed.as_secs() < 10,
        "dissolve-driven reap took {:?}, expected < 10s",
        elapsed
    );
}

#[test]
fn spawn_nonexistent_command_surfaces_not_found() {
    // execvp ENOENT inside the child surfaces as _exit(127);
    // our parent decodes that as ENOENT when stderr is empty.
    // The user sees IoError.kind="not_found" via the spawn
    // path's fallible channel.
    //
    // Caveat: spawn() returns BEFORE the child exec runs — it
    // only fails at fork time. The "not_found" surface here
    // comes from the subsequent wait, not from spawn itself.
    // Test confirms that pattern: spawn succeeds, wait sees
    // 127, which is the agent-visible signal that exec failed
    // child-side.
    let src = r#"
        fn main() {
            let c = std::process::spawn("/no/such/hale_c2_child_cmd")
                or raise;
            let code = std::process::wait(c) or raise;
            println("code=", code);
        }
    "#;
    let (stdout, stderr, status) =
        build_and_run("spawn_not_found", src);
    assert!(
        status.success(),
        "non-zero exit: {:?}, stderr: {}",
        status,
        stderr
    );
    assert!(
        stdout.contains("code=127"),
        "expected code=127 (exec failure surfaced via exit); got: {:?}",
        stdout
    );
}
