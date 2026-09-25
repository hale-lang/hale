//! A child whose failure its parent is holding outlives the handler
//! that reads it.
//!
//! #1065 made `on_failure` wait for its locus's params to settle: a
//! failure that arrives while they are being set is held and delivered
//! after the last one (spec/semantics.md § "on_failure(c, err)"). But a
//! cooperative child's `run()` returns right after its `violate`, with
//! `__drain_requested` set, and the run wrapper reclaims the child on
//! the spot — so by the time the parent settled and delivered, the
//! handler read `c.name` out of a destroyed arena (heap-use-after-free
//! under ASan; garbage or a crash without it). `__reclaim_<L>` now asks
//! the runtime first, and a child with a held failure is reclaimed
//! right after its handler instead.
//!
//! The child's strings are heap values in its own arena (a literal
//! would sit in static memory and hide the free). The run is under
//! ASan with the chunk pool off, which is what lets the sanitizer see
//! a freed arena chunk (GH #816).

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;

const SRC: &str = r#"
locus Boom {
    params { name: String = ""; why: String = ""; }
    closure fuse { captures: why; epoch inline; }
    fn check() { self.why = std::str::upper("lit-" + self.name); violate fuse; }
    run() { self.check(); }
    dissolve() { println("dissolve " + self.name); }
}
main locus App {
    params {
        b: Boom = Boom { name: std::str::upper("heap-name") };
        prefix: String = "got";
        seen: String = "";
    }
    on_failure(c: Boom, err: ClosureViolation) {
        println("handler");
        self.seen = self.prefix + " " + c.name + " " + c.why;
    }
    run() { println("seen=[" + self.seen + "]"); }
}
fn main() { App { }; }
"#;

#[test]
fn a_held_failures_child_is_reclaimed_after_its_handler() {
    let program = hale_syntax::parse_source(SRC).expect("parse");
    let bin = harness::unique_bin("hale_held_failure_reclaim");
    harness::build_asan(&program, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "non-zero {:?}; stdout: {stdout}; stderr: {stderr}",
        out.status
    );
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        [
            "handler",
            "dissolve HEAP-NAME",
            "seen=[got HEAP-NAME LIT-HEAP-NAME]",
        ],
        "the handler reads the child, then the child is reclaimed once; stderr: {stderr}"
    );
}
