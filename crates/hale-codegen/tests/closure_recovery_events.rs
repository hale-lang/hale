//! What a recovery does to a closure's accumulators (F.40 phase 4, W3).
//!
//! A restart zeroes a closure's accumulators unless the closure persists
//! through `restart`; `resets_on(restart)` states that default and adds
//! nothing at run time. Lowering reads the clause's typed events, so the
//! three programs below differ only in the clause, and the persisted
//! total is the only one that fails a second time.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// A tracker whose closure sums its samples, restarted by its parent on a
/// violation: 200 breaks the band of 100, then 1 is within it on a fresh
/// total and outside it on a kept one (201).
fn program(clause: &str) -> String {
    format!(
        r#"type Sample {{ value: Int; }}

locus Tracker {{
    params {{ delta: Int = 0; }}
    closure band {{
        sum(self.delta) ~~ 0 within 100;
        epoch tick;
        {clause}
    }}
    bus {{ subscribe "data" as on_data of type Sample; }}
    fn on_data(s: Sample) {{ self.delta = s.value; }}
}}

locus Coordinator {{
    on_failure(t: Tracker, err: ClosureViolation) {{
        println("violation diff=", err.diff);
        restart(t) for 5;
    }}
    bus {{ publish "data" of type Sample; }}
    run() {{
        Tracker {{ }};
        "data" <- Sample {{ value: 200 }};
        "data" <- Sample {{ value: 1 }};
        println("done");
    }}
}}

fn main() {{ Coordinator {{ }}; }}
"#
    )
}

fn run(name: &str, clause: &str) -> String {
    let bin = harness::unique_bin(&format!("closure_recovery_events_{name}"));
    build_opts::build_source(&program(clause), &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "{name}: {:?}\n{}", out.status, String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn violations(stdout: &str) -> Vec<&str> {
    stdout.lines().filter(|l| l.starts_with("violation")).collect()
}

#[test]
fn a_restart_resets_the_total_unless_the_closure_persists_through_it() {
    let reset = run("default", "");
    assert_eq!(violations(&reset), ["violation diff=200"], "{reset}");
    let stated = run("resets_on", "resets_on(restart);");
    assert_eq!(stated, reset, "`resets_on(restart)` states the default");
    let kept = run("persists", "persists_through(restart);");
    assert_eq!(violations(&kept), ["violation diff=200", "violation diff=201"], "{kept}");
}
