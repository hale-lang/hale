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
