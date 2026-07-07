//! Regression for the refstore-shaped main.run() starvation
//! (a downstream issue tracker, surfaced 2026-05-28): when a main locus
//! declared a params field with a *qualified-name* locus type
//! (e.g. `std::http::Server`) and pinned it to a non-main
//! cooperative pool via `placement { }`, the codegen's
//! `params_locus_types` lookup filtered out qualified paths
//! (`segments.len() == 1` guard), so the corresponding entry
//! never landed in `coop_pool_locus_types`. The
//! `__coop_pool_run_<L>` wrapper synthesis then skipped that
//! type, and at run() emission the defensive "wrapper missing"
//! fallback called the child's `run()` *synchronously* on the
//! parent's thread — i.e. the http server's accept loop ran on
//! main, blocking everything else on the same pool.
//!
//! The fix resolves qualified type paths through `mangled_for_path`
//! so the locus type is recognized end-to-end. This test exercises
//! the shape via a stdlib locus (`std::io::tcp::Listener`) and
//! verifies that main.run() executes alongside the long-running
//! sibling instead of being starved.

use std::process::Command;

use hale_codegen::build_executable;

#[test]
fn qualified_stdlib_locus_on_non_main_pool_does_not_starve_main_run() {
    // Same shape as the a downstream app: main locus with a
    // qualified-path long-running child on its own pool, plus a
    // run() body that should fire on main. Pre-fix, the child's
    // run() ran synchronously on main and blocked the run() body
    // forever. Post-fix, the child's run() is posted to the io
    // pool worker, and main.run() fires.
    //
    // Uses `std::io::tcp::Listener` (qualified) rather than
    // `std::http::Server` so we don't pull HTTP-specific wiring
    // into this test; the routing-path bug is the same shape on
    // any qualified-name locus type.
    let src = r#"
        fn ignore_conn(s: std::io::tcp::Stream) {
            // No-op; the listener never gets a real connection in
            // this test (port is unused).
        }

        main locus App {
            params {
                listener: std::io::tcp::Listener = std::io::tcp::Listener {
                    host:         "127.0.0.1",
                    port:         0,
                    max_accepts:  -1,
                    on_connection: ignore_conn,
                };
            }
            placement {
                listener: cooperative(pool = io);
            }
            run() {
                println("[app] run fired");
            }
        }

        fn main() {
            println("[main] start");
            App { };
            println("[main] after App");
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("hale_test_qualified_locus_coop_pool");
    build_executable(&program, &bin).expect("build");
    // Run with a short timeout via `timeout` — the listener's
    // forever-accept loop on the io pool keeps the program alive
    // indefinitely, but main.run()'s println should land before
    // the timeout. We only care about the prints that DO land,
    // not the exit code (timeout returns 124).
    let output = Command::new("timeout")
        .arg("3")
        .arg(&bin)
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[main] start"),
        "missing [main] start: {:?}",
        stdout
    );
    assert!(
        stdout.contains("[app] run fired"),
        "main.run() was starved — child's run() likely ran \
         synchronously on main thread. Stdout: {:?}",
        stdout
    );
    assert!(
        stdout.contains("[main] after App"),
        "fn main() body didn't continue past App {{ }} — App's \
         instantiation appears to have blocked. Stdout: {:?}",
        stdout
    );
}
