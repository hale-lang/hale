//! 2026-06-01 — the unified per-child reclamation spine
//! (`__reclaim_<L>`) and its three callers:
//!
//!  - graceful-shutdown cascade (a parent reclaims its accept'd
//!    residents when it dissolves) — Task #5,
//!  - handler-body `terminate;` (a subscriber ends itself from a bus
//!    handler, not just run()) — Task #4,
//!  - the full-spine idempotency latch (`__arena`-null) so a flow
//!    reclaimed at run-completion isn't dissolved a second time by the
//!    parent cascade — Task #2,
//!  - children-tracker removal so an iterating parent never derefs a
//!    reclaimed child — Task #3.

use std::process::Command;

use hale_codegen::build_executable;

fn build_run(tag: &str, src: &str) -> (bool, String) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_reclaim_spine_{}", tag));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), combined)
}

/// Task #5: an iterating parent holding N *resident* accept'd children
/// (no `release`, no `terminate`) runs each child's dissolve at
/// graceful shutdown. Before this spine, residents skipped dissolve
/// entirely (reclaimed only by the parent's wholesale arena free at
/// process exit) — so the token count was 0.
#[test]
fn cascade_reclaims_residents_at_shutdown() {
    const N: usize = 12;
    let src = format!(
        r#"
        locus Worker {{
            params {{ id: Int = 0; }}
            dissolve() {{ println("RES"); }}
        }}
        locus Manager {{
            params {{ total: Int = 0; }}
            accept(c: Worker) {{ }}
            birth() {{
                let mut i: Int = 0;
                while i < {N} {{ Worker {{ id: i }}; i = i + 1; }}
            }}
            fn tally() {{
                for child in self.children {{ self.total = self.total + child.id; }}
            }}
        }}
        main locus App {{
            params {{ mgr: Manager = Manager {{ }}; }}
            run() {{ self.mgr.tally(); println("DONE"); }}
        }}
        fn main() {{ App {{ }}; }}
    "#
    );
    let (ok, out) = build_run("cascade_residents", &src);
    assert!(ok, "non-zero exit; out: {out}");
    assert!(out.contains("DONE"), "didn't reach DONE; out: {out}");
    let reclaimed = out.matches("RES").count();
    assert_eq!(
        reclaimed, N,
        "expected {N} resident dissolves at shutdown, got {reclaimed}; out: {out}"
    );
}

/// Task #4: a subscriber that calls `terminate;` from inside a bus
/// handler (not run()) is reclaimed when the handler returns — the
/// handler-wrapper runs the reclaim spine post-dispatch.
#[test]
fn handler_body_terminate_reclaims() {
    let src = r#"
        type Kill { n: Int = 0; }
        topic KillT { payload: Kill; subject: "rs.kill"; }
        locus Sub {
            bus { subscribe KillT as on_kill; }
            fn on_kill(k: Kill) { terminate; }
            dissolve() { println("KILLED"); }
        }
        main locus App {
            params { s: Sub = Sub { }; }
            bus { publish KillT; }
            run() {
                KillT <- Kill { n: 1 };
                std::time::sleep(150ms);
                println("DONE");
            }
        }
        fn main() { App { }; }
    "#;
    let (ok, out) = build_run("handler_terminate", src);
    assert!(ok, "non-zero exit; out: {out}");
    assert!(out.contains("DONE"), "didn't reach DONE; out: {out}");
    let killed = out.matches("KILLED").count();
    assert_eq!(
        killed, 1,
        "expected exactly one handler-terminate reclaim, got {killed}; out: {out}"
    );
}

/// Task #2 + #3: flow children reclaimed at run-completion remove
/// themselves from the parent's tracker, so the parent-dissolve
/// cascade doesn't dissolve them a second time. Each child dissolves
/// exactly once (not 0, not 2N).
#[test]
fn flow_reclaim_then_cascade_no_double() {
    const N: usize = 10;
    let src = format!(
        r#"
        locus Worker {{
            params {{ id: Int = 0; }}
            run() {{ }}
            dissolve() {{ println("ONCE"); }}
        }}
        locus Manager {{
            params {{ total: Int = 0; }}
            accept(c: Worker) {{ }}
            release(c: Worker) {{ }}
            birth() {{
                let mut i: Int = 0;
                while i < {N} {{ Worker {{ id: i }}; i = i + 1; }}
            }}
            fn tally() {{
                for child in self.children {{ self.total = self.total + child.id; }}
            }}
        }}
        main locus App {{
            params {{ mgr: Manager = Manager {{ }}; }}
            run() {{ self.mgr.tally(); println("DONE"); }}
        }}
        fn main() {{ App {{ }}; }}
    "#
    );
    let (ok, out) = build_run("flow_no_double", &src);
    assert!(ok, "non-zero exit; out: {out}");
    assert!(out.contains("DONE"), "didn't reach DONE; out: {out}");
    let once = out.matches("ONCE").count();
    assert_eq!(
        once, N,
        "expected {N} dissolves (each flow once), got {once}; out: {out}"
    );
}
