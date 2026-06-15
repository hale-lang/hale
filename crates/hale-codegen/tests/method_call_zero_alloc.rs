//! A locus method call must do zero heap allocations in steady state
//! (2026-06-15). Each call opens + closes a per-method scratch subregion;
//! the data chunk was already pooled, but the subregion `lotus_arena_t`
//! struct was malloc'd + freed per call (measured: 1 alloc/call). A
//! per-thread struct pool (mirroring the chunk pool) removes it — important
//! because the whole fast-protocol-I/O premise is zero-alloc hot loops, and
//! a parser/handler that calls `self.method(...)` per tick would otherwise
//! allocate on every call. Measured with the `std::diag` gate counter.

use hale_codegen::build_executable;
use std::process::Command;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_mcz_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

#[test]
fn locus_method_calls_are_zero_alloc() {
    // Warm up once (first call mallocs the struct + chunk), then 1000 calls
    // of both a no-arg and an arg-taking method: the struct comes from the
    // per-thread pool and the chunk from the chunk pool, so the heap counter
    // must not move.
    let src = r#"
        locus Counter {
            params { n: Int = 1; }
            fn tick() -> Int { return self.n + 1; }
            fn tick2(x: Int) -> Int { return self.n + x; }
        }
        fn main() {
            let c = Counter { };
            let _ = c.tick();
            let mut s = 0;
            let a0 = std::diag::heap_alloc_count();
            let mut i = 0;
            while i < 1000 { s = s + c.tick(); i = i + 1; }
            let noarg = std::diag::heap_alloc_count() - a0;
            let b0 = std::diag::heap_alloc_count();
            let mut j = 0;
            while j < 1000 { s = s + c.tick2(j); j = j + 1; }
            let arg = std::diag::heap_alloc_count() - b0;
            println("noarg=", noarg, " arg=", arg, " s_ok=", s > 0);
        }
    "#;
    let (out, status) = build_and_run("counter", src);
    assert!(status.success(), "exit {:?}\n{}", status, out);
    assert!(
        out.contains("noarg=0 arg=0"),
        "locus method calls must be zero-alloc in steady state; got: {:?}",
        out
    );
}
