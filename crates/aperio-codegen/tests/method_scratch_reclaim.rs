//! Bus-arena reclaim (2026-05-21): locus method bodies now open
//! a per-call scratch subregion of `self.__arena`, route transient
//! allocations through it, and destroy it at method exit. Heap-
//! typed `self.X = ...` stores deep-copy into `self.__arena` so
//! the persisted pointer outlives the scratch destroy. Heap return
//! values land in a per-method caller-arena snapshot taken at the
//! body's entry block.
//!
//! Closes the substrate side of a measured multi-MB/sec leak
//! in a long-running daemon workload — every recv-loop frame's
//! transient allocations (JSON parse, String concat, metric
//! labels) used to pile into the locus's lifetime arena for
//! the whole process.
//!
//! This file pins three claims:
//!   1. The IR for a locus lifecycle method body contains the
//!      scratch open + destroy pair.
//!   2. The IR for a user-fn method body that returns String
//!      contains the caller-arena snapshot + return-value
//!      deep-copy.
//!   3. A long-running `run()` loop allocating per-iteration
//!      transients does NOT balloon the binary's heap — under
//!      a tight RSS cap, 1M iterations complete without OOM.
//!
//! The behavioral test runs the binary inside a `setrlimit(RSS)`-
//! capped child process and asserts it exits cleanly.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use aperio_codegen::build_executable;

fn unique_path(tag: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "lt-method-scratch-{}-{}-{}.{}",
        tag,
        std::process::id(),
        nanos,
        ext,
    ));
    p
}

fn dump_ir(src: &str, tag: &str) -> String {
    let bin = unique_path(tag, "bin");
    let ir = bin.with_extension("ll");
    let program = aperio_syntax::parse_source(src).expect("parse");
    std::env::set_var("LOTUS_DUMP_IR", "1");
    let result = build_executable(&program, &bin);
    std::env::remove_var("LOTUS_DUMP_IR");
    result.expect("build");
    let ir_text = std::fs::read_to_string(&ir).expect("read IR");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ir);
    ir_text
}

fn build_and_run(src: &str, tag: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let bin = unique_path(tag, "bin");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

/// Find the substring between `define void @{name}(...)` and the
/// matching closing `}\n` and return it. Panics if `{name}` isn't
/// defined in `ir`.
fn carve_fn_body<'a>(ir: &'a str, name: &str) -> &'a str {
    let header = format!("define void @{}", name);
    let start = ir
        .find(&header)
        .or_else(|| ir.find(&format!("define ptr @{}", name)))
        .or_else(|| ir.find(&format!("define i64 @{}", name)))
        .unwrap_or_else(|| panic!("`{}` not defined in IR", name));
    let end = ir[start..]
        .find("\n}")
        .map(|i| start + i)
        .unwrap_or(ir.len());
    &ir[start..end]
}

#[test]
fn run_body_opens_and_destroys_scratch_subregion() {
    // The hot-loop shape from a long-running pinned daemon: run() loops, body
    // allocates Strings via to_string and `+`, doesn't store
    // anywhere. Without the per-method scratch every iteration's
    // transients pile into the locus's lifetime arena.
    let src = r#"
        locus Loop {
            params { iters: Int = 4; }
            run() {
                let mut i = 0;
                while i < self.iters {
                    let s = to_string(i);
                    let t = "iter=" + s;
                    println(t);
                    i = i + 1;
                }
            }
        }
        fn main() { Loop { iters: 2 }; }
    "#;
    let ir = dump_ir(src, "scratch-shape");
    let body = carve_fn_body(&ir, "Loop.run");
    assert!(
        body.contains("@lotus_arena_create_subregion"),
        "Loop.run body must call lotus_arena_create_subregion \
         to open per-call scratch; body:\n{}",
        body,
    );
    assert!(
        body.contains("@lotus_arena_destroy"),
        "Loop.run body must call lotus_arena_destroy to reclaim \
         the scratch at exit; body:\n{}",
        body,
    );
    // Sanity: the destroy must come AFTER the create (entry block
    // does the create; exit block does the destroy). Doing it the
    // other way around would mean destroying before allocating
    // — degenerate, so the ordering is a useful soundness check.
    let create_idx = body.find("@lotus_arena_create_subregion").unwrap();
    let destroy_idx = body.rfind("@lotus_arena_destroy").unwrap();
    assert!(
        create_idx < destroy_idx,
        "scratch destroy must follow create; body:\n{}",
        body,
    );
}

#[test]
fn user_fn_method_returning_string_emits_caller_arena_deep_copy() {
    // Pins the second half of the contract: a user-fn member
    // returning String snapshot caller-arena at entry and
    // deep-copies the return through it before destroying scratch.
    let src = r#"
        locus B {
            params { _u: Int = 0; }
            fn wrap(s: String) -> String {
                return "[" + s + "]";
            }
            run() {
                println(self.wrap("hi"));
            }
        }
        fn main() { B { }; }
    "#;
    let ir = dump_ir(src, "user-fn-ret");
    let body = carve_fn_body(&ir, "B.wrap");
    assert!(
        body.contains("@lotus_arena_create_subregion"),
        "B.wrap must open a per-call scratch; body:\n{}",
        body,
    );
    assert!(
        body.contains("@lotus_caller_arena_or_global"),
        "B.wrap must snapshot caller_arena at entry; body:\n{}",
        body,
    );
    assert!(
        body.contains("@lotus_str_clone"),
        "B.wrap must deep-copy its String return into the \
         caller arena; body:\n{}",
        body,
    );
    assert!(
        body.contains("@lotus_arena_destroy"),
        "B.wrap must destroy its scratch before returning; body:\n{}",
        body,
    );
}

#[test]
fn self_field_heap_store_deep_copies_into_self_arena() {
    // birth() stores a fresh String concat to self.label. The
    // concat lives in the method scratch; without the deep-copy
    // the field would hold a dangling pointer to freed memory.
    // We pin the IR shape: `self.label =` emits a `lotus_str_clone`
    // before the GEP'd store.
    let src = r#"
        locus L {
            params { label: String = ""; }
            birth() {
                self.label = "hello-" + "world";
            }
            run() {
                println("label=", self.label);
            }
        }
        fn main() { L { }; }
    "#;
    let ir = dump_ir(src, "self-field-copy");
    let body = carve_fn_body(&ir, "L.birth");
    // The birth body should:
    //   1. concat → lives in scratch
    //   2. lotus_str_clone(self.__arena, concat) — the deep-copy
    //   3. store the cloned ptr into self.label
    assert!(
        body.contains("@lotus_str_clone"),
        "L.birth must deep-copy heap-typed self.label store \
         via lotus_str_clone; body:\n{}",
        body,
    );
    // Behavioral: actually run the binary and check label survives
    // birth() exit + the scratch-destroy.
    let (stdout, status) = build_and_run(src, "self-field-copy-run");
    assert!(status.success(), "binary exited non-zero: {:?}", status);
    assert!(
        stdout.contains("label=hello-world"),
        "self.label string didn't survive birth() exit; stdout: {:?}",
        stdout,
    );
}

#[test]
fn run_loop_allocates_per_iter_without_unbounded_growth() {
    // Behavioral: 1M iterations, each allocating a sizeable
    // String via repeat() so the leak rate per iter is well
    // above any one-shot LLVM/runtime overhead. Pre-fix codegen
    // would balloon the locus arena (each repeat returns a fresh
    // ~256-byte String pinned to the locus's lifetime arena →
    // 256 MB leaked across the loop, instantly blowing the
    // ulimit). Post-fix the per-method scratch resets each call
    // so steady-state memory stays small.
    //
    // We route the per-iter work through a user-fn method so
    // the scratch open/destroy fires on the hot path (lifecycle
    // run() is one big scope; user-fn methods reset more often).
    let src = r#"
        locus Worker {
            params { _u: Int = 0; }
            fn step(i: Int) {
                let s = to_string(i);
                // 16-byte string repeated 16 times → ~256-byte
                // alloc per call. Don't store it; scratch reclaims.
                let blob = std::str::repeat("xxxxxxxxxxxxxxxx", 16);
                let _ = s;
                let _ = blob;
            }
        }
        locus Driver {
            params {
                iters: Int = 1000000;
                w: Worker = Worker { };
            }
            run() {
                let mut i = 0;
                while i < self.iters {
                    self.w.step(i);
                    i = i + 1;
                }
                println("done iters=", self.iters);
            }
        }
        fn main() { Driver { }; }
    "#;
    let program = aperio_syntax::parse_source(src).expect("parse");
    let bin = unique_path("loop-no-leak", "bin");
    build_executable(&program, &bin).expect("build");
    // bash -c 'ulimit -v 65536; ./bin' — virtual-memory ceiling
    // of 64 MiB. Pre-fix leak: 256B × 1M = 256 MB → instant
    // ENOMEM. Post-fix steady-state: a single chunk in the
    // scratch (~64 KiB) reused every call.
    let output = Command::new("bash")
        .arg("-c")
        .arg(format!("ulimit -v 65536; {}", bin.display()))
        .output()
        .expect("run under ulimit");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "binary did not exit cleanly under 64 MiB ulimit -v — \
         method scratch reclaim likely regressed.\n\
         exit: {:?}\nstdout: {:?}\nstderr: {:?}",
        output.status,
        stdout,
        stderr,
    );
    assert!(
        stdout.contains("done iters=1000000"),
        "loop did not complete; stdout: {:?}",
        stdout,
    );
}
