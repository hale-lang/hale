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
    let bin = harness::unique_bin(name);
    harness::build_source_asan(src, &bin);
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

/// A held child stops as it fails, not when the handler it failed in
/// returns. The handler goes on after `violate` (here it tells its owner
/// the call failed, then waits); a delivery published in answer to what
/// it said used to find it still subscribed, and ran on the failed child
/// while that handler waited, or was dropped at the stop with no word to
/// anyone. That is the window a `std::api` exposure's next call fell into
/// after a receiver's server error, and waited forever for its answer:
/// GH #2091 made main quick enough to land in it on a loaded machine.
/// Now the publish finds no subscriber, and its `or` says so.
#[test]
fn a_held_child_stops_when_it_fails_not_when_its_handler_returns() {
    let src = r#"
type Ask { key: Int; n: Int; }
type Told { n: Int; failed: Bool; }
topic Asks { payload: Ask; subject: "t.ask"; keyed_by key; on_unmatched: fail; }
topic Tells { payload: Told; subject: "t.tell"; }

locus Worker {
    params { open: Int = 0; }
    closure fuse { captures: open; epoch inline; }
    bus {
        subscribe Asks as on_ask where key == 1;
        publish Tells;
    }
    fn work(n: Int) -> Int fallible(ClosureViolation) {
        if n == 2 { violate fuse; }
        return n;
    }
    fn on_ask(a: Ask) {
        let v = self.work(a.n) or {
            Tells <- Told { n: a.n, failed: true };
            std::time::sleep(200ms);
            return;
        };
        Tells <- Told { n: v, failed: false };
    }
}

main locus App {
    params {
        w: Worker = Worker { };
        failed: Int = 0;
        done: Bool = false;
    }
    placement { w: cooperative(pool = work) where async_io; }
    bus {
        publish Asks;
        subscribe Tells as on_tell;
    }
    on_failure(w: Worker, err: ClosureViolation) { self.failed = self.failed + 1; }
    fn ask(n: Int) {
        Asks <- Ask { key: 1, n: n } or self.unmatched(n);
    }
    fn unmatched(n: Int) {
        println("unmatched " + to_string(n));
        self.done = true;
    }
    fn on_tell(t: Told) {
        println("told " + to_string(t.n) + " failed=" + to_string(t.failed));
        self.ask(t.n + 1);
    }
    run() {
        self.ask(1);
        let mut i = 0;
        while !self.done && i < 300 {
            std::time::sleep(10ms);
            i = i + 1;
        }
        println("failed=" + to_string(self.failed) + " done=" + to_string(self.done));
    }
}

fn main() { App { }; }
"#;
    let (stdout, stderr) = run_asan("hale_failed_child_stops_at_failure", src);
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        ["told 1 failed=false", "told 2 failed=true", "unmatched 3", "failed=1 done=true"],
        "stderr: {stderr}"
    );
}
