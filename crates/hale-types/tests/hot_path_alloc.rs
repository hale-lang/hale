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
