//! 2026-06-01 — std::http::Server on a classic (non-async_io)
//! cooperative pool must shut down cleanly when the main locus's
//! run() returns.
//!
//! Regression for the a downstream app boot-blocker: the server's
//! blocking accept() ran on a classic `cooperative(pool = io)`
//! worker. When App.run() returned, the main locus dissolved its
//! io-placed server field's arena while the io worker was still in
//! run() — a use-after-free (the worker's next subregion op locked
//! the freed parent arena → SIGSEGV), or, depending on timing, the
//! join hung forever (the blocking accept couldn't be woken).
//!
//! Two-part fix: (1) the classic accept polls + checks the pool's
//! shutdown flag so the worker leaves run() cleanly; (2) the main
//! locus joins all pool workers before tearing down its fields.
//! This test asserts the program exits 0 (neither 139/SIGSEGV nor a
//! timeout/hang) across several runs — it's a timing-sensitive race,
//! so we repeat it.

use std::process::Command;

use hale_codegen::build_executable;

#[test]
fn http_server_on_classic_pool_shuts_down_cleanly() {
    let src = r#"
        locus Routes {
            fn handle(req: std::http::Request) -> std::http::Response {
                return std::http::Response { status: 200, body: "ok" };
            }
        }
        main locus App {
            params {
                srv: std::http::Server = std::http::Server {
                    port: 19137, handler: Routes { }, ready_signal: "READY"
                };
            }
            placement { srv: cooperative(pool = io); }
            run() {
                std::time::sleep(120ms);
                println("APP DONE");
            }
        }
        fn main() { App { }; }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_http_classic_shutdown_{}", std::process::id()));
    build_executable(&program, &bin).expect("build");

    for run in 0..4 {
        let out = Command::new(&bin).output().expect("run binary");
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(
            out.status.success(),
            "run {run}: http-server-on-classic-pool did not exit cleanly: \
             {:?} (139 = SIGSEGV use-after-free; a hang shows as a non-zero \
             signal once the harness times out)\nstdout: {stdout}",
            out.status,
        );
        assert!(
            stdout.contains("READY") && stdout.contains("APP DONE"),
            "run {run}: expected READY + APP DONE; stdout: {stdout}"
        );
    }
    let _ = std::fs::remove_file(&bin);
}
