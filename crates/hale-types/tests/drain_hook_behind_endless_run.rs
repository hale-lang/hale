//! GH #1838: a `drain()` hook behind an endless `run()`.
//!
//! A hook runs at teardown, and teardown follows `run()`. A locus that
//! declares `drain()` counts as able to answer a drain, so a SIGTERM
//! drains the program — but a `run()` that never returns and never reads
//! `self.draining` never lets the drain reach the hook: the grace runs
//! out and the signal ends the process with the hook unrun. The check
//! says so at the hook (spec/verification.md § Structural & design
//! rules, "Drain hook behind an endless run").
//!
//! The runtime half — the hook running on SIGTERM once `run()` reads
//! `draining`, or once a `run()` that reads nothing ends on the drain —
//! is pinned by `tests/hale/sigterm_runs_drain_hook_test.hl`. These
//! tests assert the diagnostic fires on the issue's shape and stays
//! quiet on every shape whose hook a drain does reach.

#[path = "support/entries.rs"]
mod entries;
use entries::check_program;
use hale_syntax::{parse_source, DiagKind};

const MARK: &str = "drain()` runs only when `run()` returns";

fn diags(src: &str) -> Vec<hale_syntax::Diag> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog).into_iter().filter(|d| d.message.contains(MARK)).collect()
}

fn program(run: &str, extra: &str) -> String {
    format!(
        "main locus App {{\n    params {{ mark: String = \"/tmp/hale-drain-repro.mark\"; }}\n    run() {{\n{run}\n    }}\n    drain() {{\n        std::io::fs::write_file(self.mark, \"drained\") or discard;\n        println(\"drained\");\n    }}\n{extra}}}\n\nfn main() {{\n    App {{ }};\n}}\n"
    )
}

/// The issue's repro, verbatim: `while true` with a sleep, and a
/// `drain()` that writes a mark.
#[test]
fn fires_on_the_issue_s_repro() {
    let src = program("        println(\"running\");\n        while true {\n            std::time::sleep(200ms);\n        }", "");
    let found = diags(&src);
    assert_eq!(found.len(), 1, "one warning, at the hook: {found:#?}");
    let d = &found[0];
    assert_eq!(d.kind, DiagKind::Warn, "an advisory, as the other never-returns laws are: {d:#?}");
    assert!(d.message.contains("`App`"), "names the locus: {}", d.message);
    assert!(d.message.contains("while !self.draining"), "says what to write: {}", d.message);
    assert!(d.message.contains("SIGTERM"), "says what it costs: {}", d.message);
    let at = &src[d.span.start.0 as usize..d.span.end.0 as usize];
    assert!(at.starts_with("drain()"), "reported at the hook, got `{at}`");
    assert_eq!(d.related.len(), 1, "the loop is the witness: {d:#?}");
    let loop_at = &src[d.related[0].span.start.0 as usize..d.related[0].span.end.0 as usize];
    assert!(loop_at.starts_with("while true"), "the witness is the loop, got `{loop_at}`");
}

/// A child locus is judged as `main` is.
#[test]
fn fires_on_a_child_locus() {
    let src = "locus Worker {\n    run() { while true { std::time::sleep(50ms); } }\n    drain() { println(\"bye\"); }\n}\n\
               main locus App {\n    params { w: Worker = Worker { }; }\n    placement { w: pinned; }\n    run() { while !self.draining { std::time::sleep(50ms); } }\n}\n\
               fn main() { App { }; }\n";
    let found = diags(src);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].message.contains("`Worker`"), "{}", found[0].message);
}

/// The fix the message names: the loop reads `self.draining`.
#[test]
fn silent_when_the_loop_reads_draining() {
    let src = program("        println(\"running\");\n        while !self.draining {\n            std::time::sleep(200ms);\n        }", "");
    assert!(diags(&src).is_empty(), "{:#?}", diags(&src));
}

/// A read inside the body, with a way out of the loop, is the same fix.
#[test]
fn silent_when_the_body_reads_draining_and_breaks() {
    let src = program("        while true {\n            if self.draining { break; }\n            std::time::sleep(200ms);\n        }", "");
    assert!(diags(&src).is_empty(), "{:#?}", diags(&src));
}

/// A read in a method `run()` calls counts, as GH #1077 counts it.
#[test]
fn silent_when_a_called_method_reads_draining() {
    let src = program(
        "        while true {\n            self.tick();\n            std::time::sleep(200ms);\n        }",
        "    fn tick() { if self.stopping() { println(\"draining\"); } }\n    fn stopping() -> Bool { return self.draining; }\n",
    );
    assert!(diags(&src).is_empty(), "a read through a call, two hops deep: {:#?}", diags(&src));
    let src = format!(
        "{}fn stopping(a: App) -> Bool {{ return a.draining; }}\n",
        program("        while true {\n            if stopping(self) { println(\"draining\"); }\n            std::time::sleep(200ms);\n        }", "")
    );
    assert!(diags(&src).is_empty(), "a read through a free fn: {:#?}", diags(&src));
}

/// A `run()` that ends lets teardown run the hook: nothing to say.
#[test]
fn silent_when_run_terminates() {
    let src = program("        std::time::sleep(30s);\n        println(\"done\");", "");
    assert!(diags(&src).is_empty(), "{:#?}", diags(&src));
    let src = program(
        "        let mut i = 0;\n        while i < 10 {\n            std::time::sleep(20ms);\n            i = i + 1;\n        }",
        "",
    );
    assert!(diags(&src).is_empty(), "{:#?}", diags(&src));
    let src = program("        while true {\n            std::time::sleep(20ms);\n            return;\n        }", "");
    assert!(diags(&src).is_empty(), "a loop with an exit can end: {:#?}", diags(&src));
}

/// No `run()`, or an empty hook: nothing to say.
#[test]
fn silent_without_a_run_or_with_an_empty_hook() {
    let src = "main locus App {\n    drain() { println(\"bye\"); }\n}\nfn main() { App { }; }\n";
    assert!(diags(src).is_empty(), "{:#?}", diags(src));
    let src = "main locus App {\n    run() { while true { std::time::sleep(50ms); } }\n    drain() { }\n}\nfn main() { App { }; }\n";
    assert!(diags(src).is_empty(), "an empty hook runs nothing: {:#?}", diags(src));
}

/// A call that reads nothing is no read: the loop still never ends on
/// the drain.
#[test]
fn fires_when_the_calls_read_nothing() {
    let src = program(
        "        while true {\n            self.tick();\n            std::time::sleep(200ms);\n        }",
        "    fn tick() { println(\"tick\"); }\n",
    );
    assert_eq!(diags(&src).len(), 1, "{:#?}", diags(&src));
}
