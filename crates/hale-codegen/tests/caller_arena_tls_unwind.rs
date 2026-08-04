//! GH #375 — the caller-arena TLS must not outlive the arena it
//! points to.
//!
//! The TLS (`lotus_current_caller_arena`) is a set-and-forget
//! channel: call sites publish before stdlib/method calls, and
//! nothing ever un-published. A fallible method chain that
//! published its own method scratch and then exited down the ERROR
//! edge left the TLS pointing at the destroyed scratch; the next
//! TLS reader without its own preceding publish — a cross-seed
//! free-fn factory building a `@form(vec)` locus was the reported
//! shape — allocated out of freed memory. Deterministic 5/5
//! SIGSEGV downstream; ASan shows heap-use-after-free inside
//! `lotus_arena_alloc` via `lotus_bus_payload_arena_alloc`.
//!
//! Three-layer fix, each pinned by this file running green:
//!  1. `lotus_arena_destroy` clears the TLS when it points at the
//!     dying arena (kills the UAF class at the single point every
//!     arena death passes through);
//!  2. free-fn prologues publish their `__caller_arena` param to
//!     the TLS (re-heals it on every fn entry, and gives TLS
//!     readers the inlined-call lifetime);
//!  3. method epilogues restore the entry-time snapshot on both
//!     the ok and error exits (method calls are TLS-neutral).
//!
//! The failing shape needs BOTH seeds — every single-file toy in
//! the issue's reduction matrix stayed clean, and so did ours
//! until the import dimension was added. The lib carries the
//! `@form(vec)` locus + a Runner whose fallible method publishes
//! TLS from its scratch (the String concat) before failing through
//! a nested `or raise` hop; the consumer catches with `or` and
//! then calls the factory. Verified to reproduce the ASan UAF on
//! the pre-fix compiler exactly as written here.

use std::process::Command;

use hale_codegen::build_executable_with_imports;
use hale_codegen::mangle;
use hale_syntax::ast::{Program, TopDecl};
use hale_syntax::parse_source;

#[path = "support/harness.rs"]
mod harness;

const LIB_SRC: &str = r#"
type Rec { version: Int; name: String; sql: String; }
type LibErr { kind: String; detail: String; }

@form(vec)
locus Recs {
    params { n: Int = 0; }
    capacity { heap data of Rec; }
    fn add(version: Int, name: String, sql: String) {
        self.push(Rec { version: version, name: name, sql: sql });
        self.n = self.n + 1;
    }
}

locus Runner {
    params { set: Recs = Recs { }; }
    fn __validate() -> Int fallible(LibErr) {
        // The load-bearing line: a String concat in THIS scratch
        // publishes the TLS right before the failure unwinds.
        let label = "validate:" + to_string(self.set.n);
        if self.set.n == 0 {
            fail LibErr { kind: "bad_set", detail: label };
        }
        return self.set.n;
    }
    fn up() -> Int fallible(LibErr) {
        let n = self.__validate() or raise;
        return n;
    }
}
"#;

const PROBE_SRC: &str = r#"
import "../lib" as lib;

fn caught(e: lib::LibErr) -> Int { return 0 - 1; }

fn factory() -> lib::Recs {
    let s = lib::Recs { };
    s.add(1, "one", "CREATE TABLE a (id int)");
    s.add(2, "two", "CREATE TABLE b (id int)");
    s.add(3, "three", "CREATE TABLE c (id int)");
    return s;
}

fn rec0() -> lib::Rec { return lib::Rec { version: 0, name: "?", sql: "" }; }

fn main() {
    let empty = lib::Recs { };
    let r = lib::Runner { set: empty };
    let b = r.up() or caught(err);
    print("caught="); println(b);
    let s = factory();
    print("n="); println(s.n);
    let first = s.get(0) or rec0();
    print("first="); println(first.name);
    println("clean exit");
}
"#;

/// Mirror of the cross_seed_imports.rs helper: parse the lib
/// source as seed "lib", mangle, and produce (items, renames).
fn mangle_lib(alias: &str) -> (Vec<TopDecl>, Vec<(Vec<String>, String)>) {
    let prog = parse_source(LIB_SRC).expect("parse lib");
    let parsed: Vec<(String, Program)> = vec![("lib".to_string(), prog)];
    let stem_refs: Vec<(String, &Program)> =
        parsed.iter().map(|(s, p)| (s.clone(), p)).collect();
    let seed_renames = mangle::build_seed_renames(&stem_refs, alias);
    let mut renames: Vec<(Vec<String>, String)> = Vec::new();
    for (name, mangled) in &seed_renames {
        renames.push((vec![alias.to_string(), name.clone()], mangled.clone()));
    }
    let mut items: Vec<TopDecl> = Vec::new();
    for (_, mut prog) in parsed {
        mangle::mangle_with_renames(&mut prog, &seed_renames);
        items.extend(prog.items);
    }
    (items, renames)
}

#[test]
fn factory_after_caught_cross_seed_failure_is_clean() {
    let mut consumer = parse_source(PROBE_SRC).expect("parse probe");
    consumer.imports.clear();
    let (lib_items, renames) = mangle_lib("lib");
    consumer.items.extend(lib_items);

    let bin = harness::unique_bin("caller_arena_tls_unwind");
    build_executable_with_imports(&consumer, &bin, &renames)
        .expect("build 2-seed probe");

    let out = Command::new(&bin).output().expect("run probe");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "the factory after a caught cross-seed failure must not \
         crash (GH #375; pre-fix this was a deterministic SIGSEGV \
         / ASan heap-use-after-free): {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status,
        stdout,
        String::from_utf8_lossy(&out.stderr),
    );
    // Output correctness matters too: pre-fix, a plain (non-ASan)
    // build could survive the UAF by reading recycled memory and
    // produce garbage instead of crashing.
    assert!(stdout.contains("caught=-1"), "bad catch: {:?}", stdout);
    assert!(stdout.contains("n=3"), "bad count: {:?}", stdout);
    assert!(stdout.contains("first=one"), "bad element: {:?}", stdout);
    assert!(stdout.contains("clean exit"), "no clean exit: {:?}", stdout);
}
