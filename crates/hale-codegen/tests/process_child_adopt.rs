//! GH #716 — the memory and process-table side of
//! `std::process::adopt`.
//!
//! The behavioural half (the handle survives the factory's return, a
//! re-spawn into the same field leaves the new child readable, a
//! failed spawn empties the field) is asserted in
//! `tests/hale/process_child_adopt_test.hl`, where the assertions sit
//! next to the code. Two claims can only be checked from outside the
//! program, and they live here:
//!
//!   * **the field owner reaps.** A child moved into a locus field
//!     must be gone when the program exits — both the one the field
//!     still holds (reclaimed by the owner's dissolve) and the one a
//!     later `adopt` displaced (reclaimed by `adopt`'s release). An
//!     orphaned `sleep` outliving its parent is the failure this
//!     catches, and `.hl` cannot observe it: the observer has to
//!     survive the program.
//!
//!   * **adopt adds no leak of its own.** Under `LOTUS_ASAN=1`
//!     nothing may be attributable to `__std_process_adopt`, and no
//!     use-after-free, double-free or invalid free may appear
//!     anywhere — the double-close the hand-rolled handle copy
//!     risked. Until GH #793 there was also one pre-existing leak to
//!     tolerate: the arena of each locus a factory RETURNS was never
//!     reclaimed, so `spawn` leaked one ~216-byte arena per call.
//!     That is now reclaimed by the binding that names it, so the
//!     bounded allowance below should find nothing left to allow.
//!
//! The ASan arm asks for the instrumented build through
//! `BuildOptions::asan`, so the request belongs to that one build
//! (GH #843; it used to be `LOTUS_ASAN` in the process environment,
//! which any test building concurrently would also have read).

use std::path::{Path, PathBuf};
use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

/// Adopt a spawned child into a locus field, twice, printing each
/// pid. The first pid is the one `adopt` displaces (its release must
/// terminate and reap it); the second is the one the field still holds
/// at exit (the owner's dissolve must). `sleep 5` outlives the program
/// by far, so an unreaped child is a live orphan the parent can see.
const SRC: &str = r#"
    locus Job {
        params { child: std::process::Child = std::process::Child { }; }
        fn start(argv: String) {
            let spawned = std::process::spawn(argv) or std::process::Child { };
            std::process::adopt(self.child, spawned);
        }
        fn pid() -> Int { return self.child.pid; }
    }

    fn main() {
        let j = Job { };
        j.start("sleep\n5");
        println(j.pid());
        j.start("sleep\n5");
        println(j.pid());
    }
"#;

fn build(name: &str) -> PathBuf {
    let program = hale_syntax::parse_source(SRC).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_process_child_adopt_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

/// The same program instrumented with AddressSanitizer. GH #843 —
/// this used to set `LOTUS_ASAN` in the *process* environment, so
/// any build racing it in this binary was silently sanitized too.
/// The harness checks the artifact really carries the sanitizer;
/// the assertions on it are all negative, so an uninstrumented
/// build would pass them vacuously.
fn build_asan(name: &str) -> PathBuf {
    let program = hale_syntax::parse_source(SRC).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_process_child_adopt_{}", name));
    harness::build_asan(&program, &bin);
    bin
}

/// True while `pid` is still a `sleep` in the process table — running
/// OR a zombie nobody reaped. The `comm` field of procfs's
/// per-process status line is what to read: a zombie keeps its comm
/// but has an empty command line, so a command-line check would call
/// an unreaped child gone — which is the failure this test is for. A
/// recycled pid running something else reads as gone, the
/// conservative direction.
fn sleep_alive(pid: &str) -> bool {
    let p = Path::new("/proc").join(pid).join("stat");
    match std::fs::read_to_string(&p) {
        Ok(line) => line.contains("(sleep)"),
        Err(_) => false,
    }
}

#[test]
fn an_adopted_child_is_owned_by_the_field_and_adds_no_leak() {
    // === the field owner reaps ===================================
    let bin = build("reap");
    let out = Command::new(&bin).output().expect("run hale");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "non-zero exit: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let pids: Vec<&str> = stdout.split_whitespace().collect();
    assert_eq!(pids.len(), 2, "expected two pids, got {:?}", stdout);
    assert_ne!(pids[0], pids[1], "the second spawn reused the handle's pid?");
    for pid in &pids {
        assert!(
            pid.parse::<i64>().map(|n| n > 0).unwrap_or(false),
            "not a spawned pid: {:?}",
            pid
        );
    }
    // The parent has exited. Neither child may still be running:
    // pids[0] was released by the second `adopt`, pids[1] by the
    // owner's dissolve. A `sleep 5` still here is an orphan.
    assert!(
        !sleep_alive(pids[0]),
        "the child `adopt` displaced ({}) outlived the program — \
         adopt's release did not terminate + reap it",
        pids[0]
    );
    assert!(
        !sleep_alive(pids[1]),
        "the child still in the field ({}) outlived the program — \
         the owner's dissolve did not terminate + reap it",
        pids[1]
    );

    // === adopt adds no leak of its own ===========================
    let asan = build_asan("asan");
    let out = Command::new(&asan)
        .env("ASAN_OPTIONS", "detect_leaks=1")
        .output()
        .expect("run asan hale");
    let _ = std::fs::remove_file(&asan);
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    for bad in [
        "use-after-free",
        "heap-use-after-free",
        "double-free",
        "attempting double-free",
        "attempting free on address which was not malloc",
        "heap-buffer-overflow",
        "SEGV",
    ] {
        assert!(
            !report.contains(bad),
            "ASan reported `{}` — the one-owner invariant broke:\n{}",
            bad,
            report
        );
    }
    assert!(
        !report.contains("__std_process_adopt"),
        "a leak is attributable to adopt itself:\n{}",
        report
    );
    // The factory-return leak is gone (GH #793 — the binding that
    // names a factory result reclaims it), so this branch should not
    // be taken at all. It stays as the bound: if anything does leak
    // here it must be that ONE shape, a small arena allocated inside
    // `__std_process_spawn`, and nothing else — a per-adopt or
    // per-spawn runaway blows the bound or adds a second shape.
    let direct = report.matches("Direct leak").count();
    if direct > 0 {
        assert_eq!(
            direct, 1,
            "more than one leak shape — only the factory-return arena \
             is expected:\n{}",
            report
        );
        assert!(
            report.contains("__std_process_spawn"),
            "the leak is not the factory-return arena:\n{}",
            report
        );
        let leaked: usize = report
            .split("SUMMARY: AddressSanitizer: ")
            .nth(1)
            .and_then(|s| s.split(' ').next())
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| panic!("no leak summary to read:\n{}", report));
        assert!(
            leaked <= 1024,
            "leaked {} bytes over two spawns — more than the \
             factory-return arenas:\n{}",
            leaked,
            report
        );
    }
}
