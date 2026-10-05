//! When the drain grace expires, the one line the runtime prints says
//! what the drain was still waiting on.
//!
//! A SIGTERM begins a drain; a program that has not exited within the
//! grace (`LOTUS_DRAIN_GRACE_MS`, 5000 ms by default) ends as the
//! signal's default action would. The line it printed then named no one
//! — "a run() that never reads self.draining?" — and a CI run that
//! reached it (the head's stop, GH #1148's neighbour) left nothing to
//! say WHICH run(). It now adds what the runtime knows: each cooperative
//! pool with its mode, whether a worker is mid-iteration and in which
//! locus, and how many cells queue; then the live loci that read
//! `draining`, by name. Nothing else changes: the process still ends
//! the same way, at the same time.

use std::io::{BufRead, BufReader};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use hale_codegen::build_executable_with_options;

#[path = "support/build.rs"]
mod build_opts;
#[path = "support/harness.rs"]
mod harness;

const GRACE_MS: u64 = 400;

struct Ran {
    status: std::process::ExitStatus,
    stderr: String,
}

/// Build `src`, wait for its first stdout line ("up"), send SIGTERM and
/// collect how it ended.
fn term_after_up(tag: &str, src: &str) -> Ran {
    let bin = harness::unique_bin(tag);
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let mut child = Command::new(&bin)
        .env("LOTUS_DRAIN_GRACE_MS", GRACE_MS.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(line);
    });
    let line = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("the program prints `up` once it runs");
    assert_eq!(line.trim(), "up");
    let killed = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("kill");
    assert!(killed.success(), "sent SIGTERM");
    let out = child.wait_with_output().expect("wait");
    let _ = std::fs::remove_file(&bin);
    Ran {
        status: out.status,
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// A run() on a cooperative pool that never returns: the pool's worker
/// is mid-iteration in `Stuck` when the grace runs out.
#[test]
fn a_run_blocking_a_pool_worker_past_the_grace_is_named() {
    let ran = term_after_up(
        "drain_grace_pool",
        r#"
locus Stuck {
    params { _u: Int = 0; }
    run() {
        println("up");
        let mut i = 0;
        while i < 100000 { std::time::sleep(20ms); i = i + 1; }
    }
}
main locus Root {
    params { s: Stuck = Stuck { }; }
    placement { s: cooperative(pool = io); }
    run() { while !self.draining { std::time::sleep(10ms); } }
}
fn main() { Root { }; }
"#,
    );
    let e = &ran.stderr;
    assert_eq!(
        ran.status.signal(),
        Some(15),
        "the process still ends as SIGTERM's default action would: {:?}\n{e}",
        ran.status
    );
    assert!(e.contains(&format!("did not finish within {GRACE_MS} ms")), "{e}");
    assert!(
        e.contains("still waiting on: pool `io` (blocking): a worker is mid-iteration, 0 queued in locus `Stuck`"),
        "names the pool and the locus its worker is inside: {e}"
    );
    assert!(
        e.contains("live loci that read draining: `Root` x1"),
        "names the locus that could have answered: {e}"
    );
    assert_eq!(
        e.matches("did not finish within").count(),
        1,
        "one line, not several: {e}"
    );
}

/// The same wait on a pinned thread, which is on no pool: the report says
/// so and still names the locus that reads `draining` and has not ended.
#[test]
fn a_pinned_locus_that_outlasts_the_grace_is_named() {
    let ran = term_after_up(
        "drain_grace_pinned",
        r#"
locus Napper {
    params { _u: Int = 0; }
    run() {
        println("up");
        // a wait a drain cannot cut short (a `sleep` is a timed park, which
        // the drain expires): a subprocess the loop is inside of. The
        // subprocess runs BEFORE `draining` is first tested, so a SIGTERM
        // that lands between `up` and here still finds this locus inside
        // a 3 s wait, past the grace, and never ends the loop early.
        let mut going = true;
        while going {
            let r = std::process::run("sleep\n3")
                or std::process::ProcessOutput { code: -1, signal: 0, stdout: "", stderr: "" };
            going = !self.draining;
        }
    }
}
main locus Root {
    params { n: Napper = Napper { }; }
    placement { n: pinned; }
    run() { while !self.draining { std::time::sleep(10ms); } }
}
fn main() { Root { }; }
"#,
    );
    let e = &ran.stderr;
    assert_eq!(ran.status.signal(), Some(15), "{:?}\n{e}", ran.status);
    assert!(
        e.contains("still waiting on: no cooperative pool; live loci that read draining:"),
        "{e}"
    );
    assert!(e.contains("`Napper` x1"), "names the pinned locus: {e}");
}

/// The control: a drain that finishes inside the grace says nothing.
#[test]
fn a_drain_that_finishes_prints_no_report() {
    let program = hale_syntax::parse_source(
        r#"
main locus Root {
    params { _u: Int = 0; }
    run() { println("up"); while !self.draining { std::time::sleep(10ms); } }
}
fn main() { Root { }; }
"#,
    )
    .expect("parse");
    let bin = harness::unique_bin("drain_grace_quiet");
    build_executable_with_options(&program, &bin, &[], &build_opts::options()).expect("build");
    let mut child = Command::new(&bin)
        .env("LOTUS_DRAIN_GRACE_MS", GRACE_MS.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut first = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut first)
        .expect("up");
    Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("kill");
    let out = child.wait_with_output().expect("wait");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{:?}\n{stderr}", out.status);
    assert!(!stderr.contains("did not finish"), "{stderr}");
}
