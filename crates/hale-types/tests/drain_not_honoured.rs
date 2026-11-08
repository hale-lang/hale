//! GH #1858: a locus that reads `draining` in its `run()` takes the drain
//! into its own hands, and the check says when it cannot see a loop of
//! that `run()` stop on it (spec/verification.md § Structural & design
//! rules, "Manual drain not honoured").
//!
//! The runtime stops every other `run()`'s loops at their back-edge, so a
//! locus that never reads the flag is never reported, whatever its loops
//! (this replaced GH #1838's "drain hook behind an endless run": no hook is
//! behind an endless run any more unless its locus chose manual control).
//! The runtime half is pinned by `crates/hale-codegen/tests/drain_is_the_runtimes.rs`.

#[path = "support/entries.rs"]
mod entries;
use entries::check_program;
use hale_syntax::{parse_source, DiagKind};

const MARK: &str = "this loop reads `draining` but I can't see it stop on it";

fn diags(src: &str) -> Vec<hale_syntax::Diag> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog).into_iter().filter(|d| d.message.contains(MARK)).collect()
}

fn located<'a>(src: &'a str, d: &hale_syntax::Diag) -> &'a str {
    &src[d.span.start.0 as usize..d.span.end.0 as usize]
}

/// The shapes the analysis proves: the flag in the condition leading out,
/// a test first in the body leading out, a loop that ends on its own (a
/// `for`, a condition over a local, an exit no drain test guards), and
/// those through a method `run()` calls.
#[test]
fn proven_shapes_are_silent() {
    let src = "locus Worker {\n\
    params { open: Bool = true; n: Int = 0; }\n\
    run() {\n\
        while !self.draining { std::time::sleep(10ms); }\n\
        while self.open && !self.draining { std::time::sleep(10ms); }\n\
        while true { if self.draining { break; } std::time::sleep(10ms); }\n\
        while true { if self.draining || self.n > 3 { return; } self.n = self.n + 1; }\n\
        let mut i = 0;\n\
        while i < 10 { i = i + 1; }\n\
        for x in [1, 2, 3] { self.n = self.n + x; }\n\
        while true { if self.n > 100 { break; } self.n = self.n + 1; }\n\
        self.pump();\n\
    }\n\
    fn pump() { while !self.draining { std::time::sleep(10ms); } }\n\
}\n\
main locus App { params { w: Worker = Worker { }; } }\n\
fn main() { App { }; }\n";
    assert_eq!(diags(src), Vec::new());
}

/// A locus that never reads `draining` is the runtime's to stop: its
/// endless loops are never reported, a `drain()` hook or not.
#[test]
fn a_locus_that_never_reads_the_flag_is_never_reported() {
    let src = "locus Worker {\n\
    run() { while true { std::time::sleep(50ms); } }\n\
    drain() { println(\"bye\"); }\n\
}\n\
main locus App { params { w: Worker = Worker { }; } placement { w: pinned; } }\n\
fn main() { App { }; }\n";
    assert_eq!(diags(src), Vec::new());
}

/// A read outside `run()` (a hook, a handler) does not make the locus
/// manual, so its loops are the runtime's and are not reported.
#[test]
fn a_read_outside_run_is_not_manual() {
    let src = "locus Worker {\n\
    run() { while true { std::time::sleep(50ms); } }\n\
    drain() { if self.draining { println(\"bye\"); } }\n\
}\n\
main locus App { params { w: Worker = Worker { }; } }\n\
fn main() { App { }; }\n";
    assert_eq!(diags(src), Vec::new());
}

/// The flag copied into a field and tested later: the loop waits on the
/// copy, which only the drain sets, and the analysis cannot see that.
#[test]
fn a_copied_flag_is_reported_at_the_loop() {
    let src = "locus Worker {\n\
    params { stopping: Bool = false; }\n\
    run() {\n\
        while !self.stopping {\n\
            std::time::sleep(10ms);\n\
            self.check();\n\
        }\n\
    }\n\
    fn check() { if self.draining { self.stopping = true; } }\n\
}\n\
main locus App { params { w: Worker = Worker { }; } }\n\
fn main() { App { }; }\n";
    let found = diags(src);
    assert_eq!(found.len(), 1, "{found:#?}");
    let d = &found[0];
    assert_eq!(d.kind, DiagKind::Warn, "a warning: the author chose manual control: {d:#?}");
    assert!(located(src, d).starts_with("while !self.stopping"), "at the loop: `{}`", located(src, d));
    assert!(d.message.contains("`Worker`"), "names the locus: {}", d.message);
    assert!(d.message.contains("while !self.draining"), "says what to write: {}", d.message);
    assert!(d.message.contains("if self.draining { break; }"), "and the other shape: {}", d.message);
}

/// A test on only some paths (under another condition) does not lead out
/// on every iteration.
#[test]
fn a_conditional_test_is_reported() {
    let src = "locus Worker {\n\
    params { busy: Bool = false; }\n\
    run() {\n\
        while true {\n\
            if !self.busy { if self.draining { break; } }\n\
            std::time::sleep(10ms);\n\
        }\n\
    }\n\
}\n\
main locus App { params { w: Worker = Worker { }; } }\n\
fn main() { App { }; }\n";
    let found = diags(src);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(located(src, &found[0]).starts_with("while true"), "{}", located(src, &found[0]));
}

/// An inner loop that never ends, inside a loop that honours the drain,
/// holds the outer loop's iteration for ever: reported at the inner one.
#[test]
fn an_endless_inner_loop_is_reported() {
    let src = "locus Worker {\n\
    params { n: Int = 0; }\n\
    run() {\n\
        while !self.draining {\n\
            while true { self.n = self.n + 1; }\n\
        }\n\
    }\n\
}\n\
main locus App { params { w: Worker = Worker { }; } }\n\
fn main() { App { }; }\n";
    let found = diags(src);
    assert_eq!(found.len(), 1, "{found:#?}");
    let at = located(src, &found[0]);
    assert!(at.starts_with("while true"), "the inner loop: `{at}`");
}

/// A loop in a fn the manual `run()` calls is held to the same rule, and
/// the warning says how `run()` reaches it.
#[test]
fn a_callee_s_endless_loop_is_reported_with_the_way_there() {
    let src = "locus Worker {\n\
    run() {\n\
        if self.draining { return; }\n\
        serve();\n\
    }\n\
}\n\
fn serve() { while true { std::time::sleep(10ms); } }\n\
main locus App { params { w: Worker = Worker { }; } }\n\
fn main() { App { }; }\n";
    let found = diags(src);
    assert_eq!(found.len(), 1, "{found:#?}");
    let d = &found[0];
    assert!(located(src, d).starts_with("while true"), "{}", located(src, d));
    assert_eq!(d.related.len(), 1, "{d:#?}");
    assert!(d.related[0].label.contains("`serve`"), "{:#?}", d.related);
}
