//! GH #2027: three advisories that read too little.
//!
//! (b) The unbounded-allocation survey flagged a flow child (a type its
//!     parent `release`s) instantiated per iteration of the parent's
//!     `run()`. A flow is reclaimed when its `run()` completes
//!     (spec/semantics.md § "`release(c)` and flow children"), so the
//!     literal is that iteration's, as an eager-only locus's is; the
//!     measured program stayed flat over 2M children.
//! (c) The GH #737 re-key notice walked only the top-level statements of
//!     `birth()`: an assignment to the key field inside an `if` (or a
//!     `match` arm, a loop, a block, or a method `birth()` calls) was
//!     silent.
//! (a) The accept-without-release advisory took only a literal
//!     `while true` as its daemon signal; it now also takes the
//!     starvation law's never-returns column (`while !self.draining`).

use std::sync::atomic::{AtomicUsize, Ordering};

/// The warnings `hale check main.hl` prints for `src`, one message each
/// (the text after `warning: `, through the end of its line).
fn warnings(src: &str) -> Vec<String> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "hale-gh2027-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.hl");
    std::fs::write(&file, src).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&file)
        .env("HALE_SKIP_STALE_CHECK", "1")
        .output()
        .expect("run hale check");
    let _ = std::fs::remove_dir_all(&dir);
    let text = String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "the program checks: {text}");
    text.lines().filter_map(|l| l.split_once("warning: ").map(|(_, m)| m.to_string())).collect()
}

fn flow_program(release: bool) -> String {
    let release = if release { "release(c: Conn) { self.closed = self.closed + 1; }" } else { "" };
    format!(
        r#"
locus Conn {{
    params {{ id: Int = 0; }}
    run() {{ }}
}}

locus Server {{
    params {{ n: Int = 0; closed: Int = 0; i: Int = 0; limit: Int = 2000000; }}
    accept(c: Conn) {{ self.n = self.n + 1; }}
    {release}
    run() {{
        while self.i < self.limit {{
            Conn {{ id: self.i }};
            self.i = self.i + 1;
        }}
        println(self.n, self.closed);
    }}
}}

main locus App {{
    params {{ s: Server = Server {{ }}; }}
}}

fn main() {{ App {{ }}; }}
"#
    )
}

/// (b) A flow child built per iteration of its parent's `run()` is that
/// iteration's: no unbounded-allocation warning.
#[test]
fn a_released_child_per_iteration_is_not_unbounded() {
    let ws = warnings(&flow_program(true));
    assert!(
        !ws.iter().any(|m| m.contains("unbounded allocation") && m.contains("struct Conn")),
        "a flow child is reclaimed when its run() completes: {ws:?}"
    );
}

/// (b) The true positive stays: without `release`, every accepted child
/// is resident until the parent dissolves.
#[test]
fn an_accepted_child_without_release_is_still_unbounded() {
    let ws = warnings(&flow_program(false));
    assert!(
        ws.iter().any(|m| m.contains("unbounded allocation") && m.contains("struct Conn")),
        "a resident child per iteration grows without bound: {ws:?}"
    );
}

/// (b) A flow whose own `run()` never returns keeps its arena for as long
/// as it runs, so its literal in a parent loop still accumulates.
#[test]
fn a_flow_whose_run_loops_forever_is_still_unbounded() {
    let src = flow_program(true).replace(
        "    run() { }\n}\n\nlocus Server",
        "    run() { while true { std::time::sleep(1s); } }\n}\n\nlocus Server",
    );
    assert!(src.contains("while true { std::time::sleep"), "the edit applied");
    let ws = warnings(&src);
    assert!(
        ws.iter().any(|m| m.contains("unbounded allocation") && m.contains("struct Conn")),
        "a flow that never completes is not reclaimed per iteration: {ws:?}"
    );
}

const KEYED: &str = r#"
type Ev { id: Int; }

topic K {
    payload: Ev;
    subject: "k";
    keyed_by id;
}

locus Nested {
    params { my_id: Int = 0; enabled: Bool = true; }
    bus { subscribe K as on_k where key == self.my_id; publish K; }
    birth() {
        BODY
    }
    fn set_key() { self.my_id = 9; }
    fn make() -> Ev { self.my_id = 7; return Ev { id: 7 }; }
    fn on_k(e: Ev) { println("got ", e.id); }
}

main locus App {
    params { n: Nested = Nested { }; }
    bus { publish K; }
    run() { K <- Ev { id: 7 }; }
}

fn main() { App { }; }
"#;

/// (c) An assignment to the key field anywhere `birth()` runs is named.
#[test]
fn the_rekey_notice_sees_nested_assignments_in_birth() {
    for body in [
        "if self.enabled { self.my_id = 7; }",
        "if !self.enabled { } else { self.my_id = 7; }",
        "match self.my_id { 0 -> { self.my_id = 7; }, _ -> { }, }",
        "let mut i = 0; while i < 1 { self.my_id = 7; i = i + 1; }",
        "{ self.my_id = 7; }",
        "self.set_key();",
        "if self.enabled { self.set_key(); }",
        // expression-valued forms run their statements too
        "let x = if self.enabled { self.my_id = 7; 1 } else { 0 }; println(x);",
        "let x = match self.my_id { 0 -> { self.my_id = 7; 1 }, _ -> 0, }; println(x);",
        "let x = { self.my_id = 7; 1 }; println(x);",
        "let x = if self.enabled { self.set_key(); 1 } else { 0 }; println(x);",
        // a publish's payload runs too
        "K <- self.make();",
    ] {
        let ws = warnings(&KEYED.replace("BODY", body));
        assert!(
            ws.iter().any(|m| m.contains("keys its subscription to `K` by `self.my_id`") && m.contains("GH #737")),
            "`{body}` in birth() re-keys nothing and must be named: {ws:?}"
        );
    }
}

/// (c) An assignment to another field, or none, stays silent.
#[test]
fn the_rekey_notice_is_silent_without_a_key_write() {
    for body in ["if self.enabled { self.enabled = false; }", "println(self.my_id);"] {
        let ws = warnings(&KEYED.replace("BODY", body));
        assert!(!ws.iter().any(|m| m.contains("GH #737")), "`{body}` writes no key: {ws:?}");
    }
}

/// (a) A parent whose `run()` is the drain-aware loop accepts children it
/// never releases: the advisory names it, as it names `while true`.
#[test]
fn accept_without_release_sees_a_drain_aware_loop() {
    for cond in ["true", "!self.draining"] {
        let src = format!(
            r#"
locus Conn {{
    params {{ id: Int = 0; }}
    run() {{ }}
}}

locus Server {{
    params {{ n: Int = 0; }}
    accept(c: Conn) {{ self.n = self.n + 1; }}
    run() {{
        while {cond} {{
            Conn {{ id: self.n }};
            std::time::sleep(10ms);
        }}
    }}
}}

main locus App {{
    params {{ s: Server = Server {{ }}; }}
}}

fn main() {{ App {{ }}; }}
"#
        );
        let ws = warnings(&src);
        assert!(
            ws.iter().any(|m| m.contains("accepts `Conn` children but declares no `release")),
            "`while {cond}` is a daemon loop: {ws:?}"
        );
    }
}
