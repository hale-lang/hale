//! std::process::try_wait + signal (2026-07-17).
//!
//! `try_wait(c)` is the non-blocking reap — `-2` means "still
//! running, poll again" (the stdlib retryable-sentinel shape recv
//! uses), `0..255` a normal exit, `-1` killed-by-signal; ECHILD
//! surfaces via the IoError channel. Closes the "daemons can't
//! non-blocking-reap children" gap (styleguide §7): a supervisor
//! tick polls per child without parking its pool. `signal(c, sig)`
//! sends an arbitrary POSIX signal to the pid (promoted from
//! pond/subprocess's Process.signal surface).

use std::process::Command;
use std::time::Instant;

use hale_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_try_wait_{}_{}", name, std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

#[test]
fn try_wait_polls_to_exit_without_blocking() {
    let src = r#"
        fn main() {
            let c = std::process::spawn("sleep\n0.3") or raise;
            let first = std::process::try_wait(c) or raise;
            println("running=", first);
            let mut code = first;
            let mut polls = 0;
            while code == -2 && polls < 100 {
                std::time::sleep(20ms);
                code = std::process::try_wait(c) or raise;
                polls = polls + 1;
            }
            println("code=", code, " polled=", polls > 0);
            // Reaped: a second try_wait hits ECHILD -> error channel.
            let again = std::process::try_wait(c) or -99;
            println("again=", again);
        }
    "#;
    let start = Instant::now();
    let (out, status) = build_and_run("poll", src);
    assert!(status.success(), "exit: {:?}\n{}", status, out);
    // The first probe on a 300ms sleeper must be the still-running
    // sentinel (nothing blocked), then polling observes the exit.
    assert!(out.contains("running=-2"), "got:\n{}", out);
    assert!(out.contains("code=0 polled=true"), "got:\n{}", out);
    assert!(out.contains("again=-99"), "double reap must error:\n{}", out);
    // Sanity: nothing waited 60s anywhere.
    assert!(start.elapsed().as_secs() < 30);
}

#[test]
fn signal_term_observed_as_signal_kill() {
    let src = r#"
        fn main() {
            let c = std::process::spawn("sleep\n30") or raise;
            let pre = std::process::try_wait(c) or raise;
            std::process::signal(c, 15) or raise;
            let mut code = -2;
            let mut polls = 0;
            while code == -2 && polls < 100 {
                std::time::sleep(20ms);
                code = std::process::try_wait(c) or -99;
                polls = polls + 1;
            }
            println("pre=", pre, " post=", code);
            // Signal a reaped child: ESRCH -> error channel, discardable.
            std::process::signal(c, 15) or discard;
            println("signal-after-reap ok");
        }
    "#;
    let start = Instant::now();
    let (out, status) = build_and_run("term", src);
    assert!(status.success(), "exit: {:?}\n{}", status, out);
    // Killed-by-signal is code -1 through try_wait, same decode as wait.
    assert!(out.contains("pre=-2 post=-1"), "got:\n{}", out);
    assert!(out.contains("signal-after-reap ok"), "got:\n{}", out);
    // The 30s sleeper died from the TERM, not from running out.
    assert!(start.elapsed().as_secs() < 25);
}

#[test]
fn manual_child_sentinel_conventions_hold() {
    // pid <= 0 mirrors wait's manual-Child convention: "already
    // exited with code 0", and signal no-ops.
    let src = r#"
        fn main() {
            let manual = std::process::Child { };
            println("tw=", std::process::try_wait(manual) or -99);
            std::process::signal(manual, 15) or raise;
            println("sig-noop ok");
        }
    "#;
    let (out, status) = build_and_run("manual", src);
    assert!(status.success(), "exit: {:?}\n{}", status, out);
    assert!(out.contains("tw=0"), "got:\n{}", out);
    assert!(out.contains("sig-noop ok"), "got:\n{}", out);
}
