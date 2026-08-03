//! Static drain elision (2026-08-03) — both directions.
//!
//! A bundle that can never enqueue a bus cell emits no
//! `lotus_bus_queue_drain` calls at all. The gate is three-tier:
//! the USER program must be bus-free, and it must not reference a
//! stdlib namespace that can transitively reach a bus-surfaced
//! stdlib decl (`std::log`'s sinks were the CI failure that forced
//! tier 2/3: a bus-free user program whose log events were enqueued
//! by `std::log::Logger` and then never drained — empty stdout).
//!
//! The behavioral direction (std::log still delivers) is pinned by
//! `log_routing.rs`; this file pins the structural direction — that
//! elision actually HAPPENS for the programs it is meant for, and
//! does NOT happen when a tainted stdlib namespace is referenced.
//! Disassembly-based, so Linux-only (macOS runners lack binutils
//! objdump); the behavioral tests carry the other platforms.

#![cfg(target_os = "linux")]

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;

use hale_codegen::build_executable;

/// Count `lotus_bus_queue_drain` call sites in the binary's `main`.
fn drain_call_sites(bin: &std::path::Path) -> usize {
    let out = Command::new("objdump")
        .args(["-d", "--disassemble=main"])
        .arg(bin)
        .output()
        .expect("objdump runs");
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .filter(|l| l.contains("call") && l.contains("lotus_bus_queue_drain"))
        .count()
}

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(name);
    build_executable(&program, &bin).expect("build");
    bin
}

/// A compute-only program — loci, methods, sleeps, `std::time` and
/// `std::process` references, but no bus surface anywhere — carries
/// zero drain call sites. The two per-instantiation drains were ~30%
/// of the birth+dissolve microbench.
#[test]
fn compute_only_program_has_no_drain_sites() {
    const SRC: &str = r#"
        locus Empty {
            params { v: Int = 0; }
            fn read() -> Int { return self.v; }
        }
        fn one(seed: Int) -> Int {
            let e = Empty { v: seed };
            return e.read();
        }
        fn main() {
            let t0 = std::time::monotonic();
            let mut sink = 0;
            let mut i = 0;
            while i < 100 { sink = sink ^ one(i); i = i + 1; }
            let t1 = std::time::monotonic();
            if t1 < t0 { println("clock went backwards"); }
            println(sink);
        }
    "#;
    let bin = build("drain_elision_inert", SRC);
    let out = Command::new(&bin).output().expect("run");
    assert!(out.status.success(), "program must still run");
    let sites = drain_call_sites(&bin);
    let _ = std::fs::remove_file(&bin);
    assert_eq!(
        sites, 0,
        "a bus-inert program must emit no drain calls in main"
    );
}

/// Referencing `std::log` (a tainted namespace: its sinks are bus
/// subscribers) keeps the drains — this is the exact shape whose
/// events were silently dropped when tier 1 gated alone.
#[test]
fn std_log_reference_keeps_the_drains() {
    const SRC: &str = r#"
        fn main() {
            std::log::StdoutSink { };
            let log = std::log::Logger { name: "t" };
            log.info("delivered");
        }
    "#;
    let bin = build("drain_elision_log", SRC);
    let out = Command::new(&bin).output().expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let sites = drain_call_sites(&bin);
    let _ = std::fs::remove_file(&bin);
    assert!(
        sites > 0,
        "a std::log-using program must keep its drain calls"
    );
    assert!(
        stdout.contains("delivered"),
        "and the log event must actually be delivered: {:?}",
        stdout
    );
}

/// A user program with its own bus surface obviously keeps the
/// drains — tier 1.
#[test]
fn user_bus_surface_keeps_the_drains() {
    const SRC: &str = r#"
        type T { v: Int; }
        locus Sub {
            params { got: Int = 0; }
            bus { subscribe "s.t" as on_t of type T; }
            fn on_t(t: T) { self.got = t.v; }
        }
        fn main() {
            let s = Sub { };
            "s.t" <- T { v: 7 };
            yield;
            println(s.got);
        }
    "#;
    let bin = build("drain_elision_userbus", SRC);
    let out = Command::new(&bin).output().expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let sites = drain_call_sites(&bin);
    let _ = std::fs::remove_file(&bin);
    assert!(sites > 0, "a subscribing program keeps its drains");
    assert!(
        stdout.contains('7'),
        "and delivery works: {:?}",
        stdout
    );
}
