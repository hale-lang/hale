//! GH #1069 — a failed child its owner still holds stays readable.
//!
//! A child's `violate` sets its drain latch, and its run wrapper used
//! to reclaim any child whose run() returned with that latch set. For
//! a child held in a param field or a binding, the handle kept pointing
//! at the freed arena: the parent's next `self.c.x` read was a
//! heap-use-after-free. spec/semantics.md § `terminate`, "A failed
//! child its owner holds is kept": the child stops, and its memory
//! stays until its owner's teardown, where drain / dissolve run once.
//!
//! The neighbouring shapes keep reclaiming themselves: a bare statement
//! literal (nothing else would; its statement's own teardown now steps
//! over it instead of running `dissolve()` a second time on the freed
//! arena) and an accept'd child (its owner's tracker expects it gone).
//!
//! Every field the program reads after a failure is a heap value
//! (`std::str::upper`), and it runs under ASan with the chunk pool off:
//! a literal would sit in static memory and hide the free.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;

fn run_asan(name: &str, src: &str) -> (String, String) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(name);
    harness::build_asan(&program, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "{name}: non-zero {:?}; stdout: {stdout}; stderr: {stderr}",
        out.status
    );
    (stdout, stderr)
}

const ONCE: &str = r#"
locus Once {
    params { tag: String = ""; note: String = ""; mode: Int = 0; }
    closure fuse { captures: note; epoch inline; }
    fn check() { self.note = std::str::upper("failed-" + self.tag); violate fuse; }
    run() { self.check(); }
    dissolve() { println("dissolve " + self.tag); }
}
"#;

#[test]
fn a_failed_child_stays_readable_until_its_owner_dissolves() {
    let src = format!(
        r#"{ONCE}
main locus App {{
    params {{
        seen: String = "";
        absorbed: Once = Once {{ tag: std::str::upper("absorb"), mode: 0 }};
        quarantined: Once = Once {{ tag: std::str::upper("quarantine"), mode: 1 }};
        spent: Once = Once {{ tag: std::str::upper("spent"), mode: 2 }};
    }}
    on_failure(c: Once, err: ClosureViolation) {{
        self.seen = self.seen + c.tag + ";";
        if c.mode == 1 {{ quarantine(c); }}
        if c.mode == 2 {{ restart(c) for 1; }}
    }}
    run() {{
        println("fields " + self.absorbed.note + " " + self.quarantined.note + " " + self.spent.note);
        let b = Once {{ tag: std::str::upper("binding") }};
        println("binding " + b.note);
        Once {{ tag: std::str::upper("statement") }};
        println("seen " + self.seen);
    }}
}}
fn main() {{ App {{ }}; }}
"#
    );
    let (stdout, stderr) = run_asan("hale_failed_child_kept", &src);
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        [
            "fields FAILED-ABSORB FAILED-QUARANTINE FAILED-SPENT",
            "binding FAILED-BINDING",
            // the statement literal is reclaimed at once, and only once
            "dissolve STATEMENT",
            "seen ABSORB;QUARANTINE;SPENT;SPENT;BINDING;STATEMENT;",
            // the held ones at their owners' teardown: the binding at
            // run()'s exit, the fields at App's
            "dissolve BINDING",
            "dissolve ABSORB",
            "dissolve QUARANTINE",
            "dissolve SPENT",
        ],
        "stderr: {stderr}"
    );
}

#[test]
fn an_accepted_child_that_fails_is_still_reclaimed_at_once() {
    let src = format!(
        r#"{ONCE}
locus Server {{
    params {{ n: Int = 0; fired: Int = 0; }}
    accept(c: Once) {{ self.n = self.n + 1; }}
    on_failure(c: Once, err: ClosureViolation) {{ self.fired = self.fired + 1; }}
    run() {{
        Once {{ tag: std::str::upper("conn") }};
        println("accepted " + to_string(self.n) + " fired " + to_string(self.fired));
    }}
}}
main locus App {{ params {{ s: Server = Server {{ }}; }} }}
fn main() {{ App {{ }}; }}
"#
    );
    let (stdout, stderr) = run_asan("hale_failed_child_accepted", &src);
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        ["dissolve CONN", "accepted 1 fired 1"],
        "stderr: {stderr}"
    );
}
