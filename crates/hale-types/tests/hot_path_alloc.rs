//! Hot-path allocation lint (2026-07-16) — Lever 3.
//!
//! Two loop-scoped anti-patterns get a warning, so the fast path is
//! the path of least resistance rather than expert folklore:
//!   1. a locus (its own arena / heap buffer) instantiated per loop
//!      iteration — hoist to a reused field;
//!   2. an allocating `recv` in a loop — use `recv_into` with a reused
//!      buffer.
//! The zero-alloc equivalents (reused field, `recv_into`) stay silent.

use hale_syntax::parse_source;
use hale_types::check_program;

fn warnings(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog)
        .into_iter()
        .map(|d| d.message)
        .filter(|m| m.contains("hot-path allocation"))
        .collect()
}

// ---- positives: the anti-patterns fire -------------------------------

#[test]
fn locus_instantiated_in_loop_is_flagged() {
    let src = r#"
locus Conn { run() { } }

locus Server {
    run() {
        let mut n = 0;
        while n < 100 {
            let c = Conn { };
            n = n + 1;
        }
    }
}

fn main() { }
"#;
    let ws = warnings(src);
    assert!(
        ws.iter().any(|m| m.contains("locus `Conn`") && m.contains("loop")),
        "expected loop-scoped locus-instantiation warning, got: {:?}",
        ws
    );
}

#[test]
fn bytesbuilder_in_loop_is_flagged() {
    let src = r#"
locus Server {
    run() {
        let mut n = 0;
        while n < 100 {
            let b = std::bytes::BytesBuilder { initial_cap: 4096 };
            n = n + 1;
        }
    }
}

fn main() { }
"#;
    let ws = warnings(src);
    assert!(
        ws.iter().any(|m| m.contains("std::bytes::BytesBuilder")),
        "expected loop-scoped BytesBuilder warning, got: {:?}",
        ws
    );
}

#[test]
fn allocating_recv_path_call_in_loop_is_flagged() {
    let src = r#"
locus Reader {
    params { fd: Int = 0; }
    run() {
        let mut n = 0;
        while n < 100 {
            let msg = std::io::udp::recv(self.fd, 2048) or discard;
            n = n + 1;
        }
    }
}

fn main() { }
"#;
    let ws = warnings(src);
    assert!(
        ws.iter().any(|m| m.contains("std::io::udp::recv") && m.contains("recv_into")),
        "expected loop-scoped allocating-recv warning, got: {:?}",
        ws
    );
}

#[test]
fn allocating_recv_method_call_in_loop_is_flagged() {
    // Method-call form `stream.recv_bytes(n)` — the receiver types as
    // Unknown (stdlib handle locus), so the lint keys off the method
    // name.
    let src = r#"
locus Sink {
    params { s: Int = 0; }
    run() {
        let mut n = 0;
        while n < 100 {
            let chunk = self.s.recv_bytes(4096) or discard;
            n = n + 1;
        }
    }
}

fn main() { }
"#;
    let ws = warnings(src);
    assert!(
        ws.iter().any(|m| m.contains("recv_bytes")),
        "expected method-form allocating-recv warning, got: {:?}",
        ws
    );
}

#[test]
fn for_loop_body_is_a_hot_loop_too() {
    let src = r#"
locus Conn { run() { } }

locus Server {
    run() {
        for i in 0..100 {
            let c = Conn { };
        }
    }
}

fn main() { }
"#;
    let ws = warnings(src);
    assert!(
        ws.iter().any(|m| m.contains("locus `Conn`")),
        "expected for-loop body to count as a hot loop, got: {:?}",
        ws
    );
}

// ---- negatives: the fast path (and non-loop code) stays silent -------

#[test]
fn hoisted_reused_field_is_silent() {
    // The fast path: the builder is a field (allocated once at birth),
    // reused per iteration via recv_into. Nothing instantiated in the
    // loop, nothing flagged.
    let src = r#"
locus Reader {
    params {
        fd: Int = 0;
        buf: std::bytes::BytesBuilder = std::bytes::BytesBuilder { initial_cap: 4096 };
    }
    run() {
        let mut n = 0;
        while n < 100 {
            let got = std::io::udp::recv_into(self.fd, self.buf, 2048) or discard;
            n = n + 1;
        }
    }
}

fn main() { }
"#;
    assert!(
        warnings(src).is_empty(),
        "fast path must be silent, got: {:?}",
        warnings(src)
    );
}

#[test]
fn instantiation_outside_a_loop_is_silent() {
    // A per-invocation instantiation reclaims when the method returns;
    // only loop-scoped allocation is the unambiguous hot-path case.
    let src = r#"
locus Conn { run() { } }

locus Server {
    run() {
        let c = Conn { };
    }
}

fn main() { }
"#;
    assert!(
        warnings(src).is_empty(),
        "non-loop instantiation must be silent, got: {:?}",
        warnings(src)
    );
}

#[test]
fn plain_struct_literal_in_loop_is_silent() {
    // A plain struct/type literal is a value — no arena, no heap
    // buffer — so it's not flagged even in a loop.
    let src = r#"
type Point { x: Int; y: Int; }

locus Server {
    run() {
        let mut n = 0;
        while n < 100 {
            let p = Point { x: 1, y: 2 };
            n = n + 1;
        }
    }
}

fn main() { }
"#;
    assert!(
        warnings(src).is_empty(),
        "plain struct literal must be silent, got: {:?}",
        warnings(src)
    );
}

// ---- Gap D (2026-07-17): handler context, @hot, accept/release ------

fn all_diags(src: &str) -> Vec<(bool, String)> {
    // (is_error, message) pairs — Gap D promotes hot-path findings to
    // errors inside `@hot` fns, so tests need the severity too.
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog)
        .into_iter()
        .map(|d| (d.is_error(), d.message))
        .collect()
}

#[test]
fn builder_in_bus_handler_flagged_at_depth_zero() {
    // A bus handler runs per message — a builder instantiated anywhere
    // in it (no loop needed) is the ~4.5 KB/frame class.
    let src = r#"
type Msg { text: String = ""; }
topic T { payload: Msg; subject: "t"; }
locus Sub {
    params { n: Int = 0; }
    bus { subscribe T as on_msg; }
    fn on_msg(m: Msg) {
        let b = std::bytes::BytesBuilder { };
        self.n = self.n + 1;
    }
}
fn main() { }
"#;
    let ws = warnings(src);
    assert!(
        ws.iter().any(|m| m.contains("bus handler")),
        "expected handler-scoped builder warning, got: {:?}",
        ws
    );
}

#[test]
fn non_handler_method_at_depth_zero_stays_silent() {
    // The same builder in a PLAIN method (not a handler, no loop) is
    // once-per-call scratch — silent.
    let src = r#"
locus L {
    params { n: Int = 0; }
    fn helper() {
        let b = std::bytes::BytesBuilder { };
        self.n = self.n + 1;
    }
    run() { self.helper(); }
}
fn main() { }
"#;
    let ws = warnings(src);
    assert!(ws.is_empty(), "expected no warnings, got: {:?}", ws);
}

#[test]
fn hot_promotes_loop_finding_to_error() {
    let src = r#"
locus L {
    @hot fn spin(x: Int) {
        let mut i = 0;
        while i < x {
            let b = std::bytes::BytesBuilder { };
            i = i + 1;
        }
    }
    run() { }
}
fn main() { }
"#;
    let ds = all_diags(src);
    assert!(
        ds.iter().any(|(is_err, m)| *is_err
            && m.contains("@hot")
            && m.contains("hot-path allocation")),
        "expected @hot-promoted ERROR, got: {:?}",
        ds
    );
}

#[test]
fn hot_snapshot_in_loop_suggests_view() {
    let src = r#"
locus L {
    params { n: Int = 0; }
    @hot fn drainy(b: std::bytes::BytesBuilder, x: Int) {
        let mut i = 0;
        while i < x {
            let s = b.snapshot();
            self.n = self.n + len(s);
            i = i + 1;
        }
    }
    run() { }
}
fn main() { }
"#;
    let ds = all_diags(src);
    assert!(
        ds.iter().any(|(is_err, m)| *is_err && m.contains(".view()")),
        "expected @hot snapshot()-in-loop hint, got: {:?}",
        ds
    );
}

#[test]
fn snapshot_in_loop_without_hot_stays_silent() {
    // The snapshot hint is @hot-tier — legitimate cold-path uses must
    // not warn by default.
    let src = r#"
locus L {
    params { n: Int = 0; }
    fn drainy(b: std::bytes::BytesBuilder, x: Int) {
        let mut i = 0;
        while i < x {
            let s = b.snapshot();
            self.n = self.n + len(s);
            i = i + 1;
        }
    }
    run() { }
}
fn main() { }
"#;
    let ds = all_diags(src);
    assert!(
        !ds.iter().any(|(_, m)| m.contains(".view()")),
        "snapshot hint must be @hot-gated, got: {:?}",
        ds
    );
}

#[test]
fn hot_whole_struct_replace_hinted() {
    let src = r#"
type State { a: Int = 0; b: String = ""; }
locus L {
    params { st: State = State { }; }
    @hot fn tick(i: Int, x: Int) {
        let mut j = 0;
        while j < x {
            self.st = State { a: j, b: "z" };
            j = j + 1;
        }
    }
    run() { }
}
fn main() { }
"#;
    let ds = all_diags(src);
    assert!(
        ds.iter().any(|(is_err, m)| *is_err
            && m.contains("whole-struct replace")),
        "expected @hot whole-struct-replace hint, got: {:?}",
        ds
    );
}

#[test]
fn hot_budget_stacking_parses_and_zero_alloc_passes() {
    let src = r#"
locus L {
    @hot @budget(alloc_per_call = 0) fn tight(x: Int) -> Int {
        let mut i = 0;
        let mut acc = 0;
        while i < x { acc = acc + i; i = i + 1; }
        return acc;
    }
    run() { }
}
fn main() { }
"#;
    let ds = all_diags(src);
    assert!(
        !ds.iter().any(|(is_err, _)| *is_err),
        "zero-alloc @hot @budget fn must be clean, got: {:?}",
        ds
    );
}

#[test]
fn accept_without_release_on_daemon_warns() {
    let src = r#"
locus Conn {
    params { fd: Int = -1; }
    run() { }
}
locus Gateway {
    params { served: Int = 0; }
    accept(c: Conn) { self.served = self.served + 1; }
    run() {
        while true {
            std::time::sleep(1ms);
        }
    }
}
fn main() { }
"#;
    let ds = all_diags(src);
    assert!(
        ds.iter().any(|(is_err, m)| !*is_err
            && m.contains("RESIDENT")
            && m.contains("release(c: Conn)")),
        "expected accept-without-release daemon warning, got: {:?}",
        ds
    );
}

#[test]
fn accept_without_release_run_to_exit_stays_silent() {
    // The corpus's accept examples are run-to-exit — the warn is gated
    // on the daemon shape (a literal `while true` in run()).
    let src = r#"
locus Conn { run() { } }
locus Gateway {
    params { served: Int = 0; }
    accept(c: Conn) { self.served = self.served + 1; }
    run() {
        let mut i = 0;
        while i < 3 { i = i + 1; }
    }
}
fn main() { }
"#;
    let ds = all_diags(src);
    assert!(
        !ds.iter().any(|(_, m)| m.contains("RESIDENT")),
        "run-to-exit accept must stay silent, got: {:?}",
        ds
    );
}

#[test]
fn accept_with_release_on_daemon_stays_silent() {
    let src = r#"
locus Conn { run() { } }
locus Gateway {
    params { served: Int = 0; }
    accept(c: Conn) { self.served = self.served + 1; }
    release(c: Conn) { }
    run() {
        while true {
            std::time::sleep(1ms);
        }
    }
}
fn main() { }
"#;
    let ds = all_diags(src);
    assert!(
        !ds.iter().any(|(_, m)| m.contains("RESIDENT")),
        "accept+release must stay silent, got: {:?}",
        ds
    );
}

// ---- GH #402: the factory shape -------------------------------------
//
// The lint matched `Expr::Struct` only, so it saw a locus LITERAL in a
// loop and nothing else. But m90 forbids a method returning a locus,
// which means factoring construction out leaves you with a free fn —
// and `let m = zeros(r, c)` in a loop allocates exactly the same fresh
// arena per iteration, reclaimed only when the enclosing fn returns.
//
// That silence is what let the #402 residual go unnoticed: the leaking
// workload allocated entirely through factories, so the compiler had
// nothing to say while it grew 3888 bytes per training step.

fn factory_warnings(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog)
        .into_iter()
        .map(|d| d.message)
        .filter(|m| m.contains("returns the locus"))
        .collect()
}

const FACTORY: &str = r#"
locus Matrix { params { n: Int = 0; } }

fn zeros(n: Int) -> Matrix {
    let m = Matrix { n: n };
    return m;
}
"#;

#[test]
fn a_let_bound_factory_result_in_a_loop_is_flagged() {
    let src = format!(
        "{}\nfn main() {{\n  let mut i = 0;\n  while i < 10 {{\n    \
         let m = zeros(i);\n    i = i + 1;\n  }}\n}}",
        FACTORY
    );
    let w = factory_warnings(&src);
    assert_eq!(w.len(), 1, "expected exactly one finding: {:?}", w);
    assert!(
        w[0].contains("`zeros`") && w[0].contains("`Matrix`"),
        "the finding must name both the factory and the locus it \
         returns — the author has to know WHICH call allocates: {:?}",
        w
    );
}

/// The advisory tells you to drop the binding, so the unbound form
/// must actually be silent — #403 registers and reclaims an unbound
/// factory temporary at the statement. A lint that flags the fix it
/// recommends is worse than no lint.
#[test]
fn an_unbound_factory_result_in_a_loop_is_silent() {
    let src = format!(
        "{}\nfn size_of(m: Matrix) -> Int {{ return m.n; }}\n\
         fn main() {{\n  let mut i = 0;\n  while i < 10 {{\n    \
         let t = size_of(zeros(i));\n    i = i + 1;\n  }}\n}}",
        FACTORY
    );
    assert!(
        factory_warnings(&src).is_empty(),
        "an unbound factory result is reclaimed at the statement: {:?}",
        factory_warnings(&src)
    );
}

/// Depth 0 is not a hot path — a factory call in straight-line code
/// dissolves at fn exit, which is simply the documented `let` rule.
#[test]
fn a_factory_result_outside_a_loop_is_silent() {
    let src = format!("{}\nfn main() {{ let m = zeros(3); }}", FACTORY);
    assert!(
        factory_warnings(&src).is_empty(),
        "straight-line construction is not a finding"
    );
}

/// A fn that returns a plain value, not a locus, must never trip it.
#[test]
fn a_non_locus_returning_fn_in_a_loop_is_silent() {
    let src = "
fn double(n: Int) -> Int { return n * 2; }
fn main() {
  let mut i = 0;
  while i < 10 { let d = double(i); i = i + 1; }
}
";
    assert!(
        factory_warnings(src).is_empty(),
        "only locus-returning fns allocate an arena per call"
    );
}

// ---- GH #526 (2026-09-05): `@unbounded` acknowledges the advisory ---
//
// Every hot-path advisory ends with "or acknowledge an intentional
// shape with `@unbounded` on the enclosing fn/hook". The walker never
// read the flag, so the acknowledgement did nothing and `hale verify`
// stayed red on a param-bounded fan-out loop (the DNA Phase 0 fixtures
// birth one flow child per `fan`). `@unbounded` silences the advisory;
// `@hot` still hard-errors.

#[test]
fn unbounded_hook_silences_locus_in_loop_advisory() {
    let src = r#"
locus Conn { run() { } }

locus Server {
    params { fan: Int = 4; }
    @unbounded
    run() {
        let mut n = 0;
        while n < self.fan {
            Conn { };
            n = n + 1;
        }
    }
}

fn main() { }
"#;
    let w = warnings(src);
    assert!(w.is_empty(), "@unbounded run() must silence the advisory, got: {:?}", w);
}

#[test]
fn unbounded_fn_silences_locus_in_loop_advisory() {
    let src = r#"
locus Conn { run() { } }

@unbounded
fn spawn_many(n: Int) {
    let mut i = 0;
    while i < n {
        let c = Conn { };
        i = i + 1;
    }
}

fn main() { spawn_many(3); }
"#;
    let w = warnings(src);
    assert!(w.is_empty(), "@unbounded fn must silence the advisory, got: {:?}", w);
}

// (`@hot` and `@unbounded` cannot stack — the parser admits only
// `@budget` after `@hot` — so "hot still errors under unbounded" has
// no representable program; the `emit` gate keeps the precedence
// anyway.)
