//! m95: std::log namespace.
//!
//! Structured logging on the bus. Logger publishes typed
//! LogEvents on `log.<full_path>`; StdoutSink subscribes to
//! `log.**` and prints `[LEVEL path] msg` per event. Cascading
//! namespace is built from `name` + `parent_path` params.
//!
//! These tests run the full codegen + native-binary path
//! because the interpreter (`hale run`) doesn't yet support
//! qualified-name struct/locus literals like
//! `std::log::Logger { ... }`.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_hale(name: &str, source: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_log_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn logger_basic_levels_print_to_stdout() {
    // Routing: WARN/ERROR go to stderr; INFO/DEBUG/TRACE go to
    // stdout. Verify the per-stream split.
    let src = r#"
        fn main() {
            std::log::StdoutSink { };
            let log = std::log::Logger { name: "app" };
            log.info("starting");
            log.warn("watch out");
            log.error("kaboom");
            log.debug("noise");
            log.trace("ultra-noise");
        }
    "#;
    let bin = build_hale("levels", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // stdout: INFO / DEBUG / TRACE.
    assert!(stdout.contains("[INFO app] starting"), "stdout: {:?}", stdout);
    assert!(stdout.contains("[DEBUG app] noise"), "stdout: {:?}", stdout);
    assert!(stdout.contains("[TRACE app] ultra-noise"), "stdout: {:?}", stdout);
    // stderr: WARN / ERROR.
    assert!(stderr.contains("[WARN app] watch out"), "stderr: {:?}", stderr);
    assert!(stderr.contains("[ERROR app] kaboom"), "stderr: {:?}", stderr);
}

#[test]
fn logger_cascades_via_parent_path() {
    let src = r#"
        fn main() {
            std::log::StdoutSink { };
            let app = std::log::Logger { name: "app" };
            let db  = std::log::Logger { name: "db", parent_path: "app" };
            let api = std::log::Logger { name: "api", parent_path: "app" };
            app.info("starting");
            db.info("connected");
            api.warn("slow");
            db.error("query failed");
        }
    "#;
    let bin = build_hale("cascade", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // INFO lands on stdout; WARN / ERROR on stderr.
    assert!(stdout.contains("[INFO app] starting"), "stdout: {:?}", stdout);
    assert!(stdout.contains("[INFO app.db] connected"), "stdout: {:?}", stdout);
    assert!(stderr.contains("[WARN app.api] slow"), "stderr: {:?}", stderr);
    assert!(stderr.contains("[ERROR app.db] query failed"), "stderr: {:?}", stderr);
}

#[test]
fn logger_three_level_nesting() {
    let src = r#"
        fn main() {
            std::log::StdoutSink { };
            let q = std::log::Logger { name: "query", parent_path: "app.db" };
            q.info("running");
        }
    "#;
    let bin = build_hale("nest3", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("[INFO app.db.query] running"),
        "got: {:?}",
        stdout
    );
}

#[test]
fn user_subscriber_can_match_subtree_pattern() {
    // m94 + m95 in concert: a user-defined sink subscribes to
    // a specific sub-tree pattern (`log.app.**`) and only
    // receives events from that branch — events from peer
    // sub-trees are filtered out by the wildcard match.
    let src = r#"
        locus DbOnlySinkL {
            bus {
                subscribe "log.app.db.**" as on_db of type std::log::LogEvent;
            }
            fn on_db(e: std::log::LogEvent) {
                println("db-only: ", e.path, " ", e.msg);
            }
        }

        fn main() {
            DbOnlySinkL { };
            let app = std::log::Logger { name: "app" };
            let db  = std::log::Logger { name: "db", parent_path: "app" };
            let api = std::log::Logger { name: "api", parent_path: "app" };
            app.info("root");
            db.info("db-event");
            api.info("api-event");
        }
    "#;
    let bin = build_hale("subtree", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("db-only: app.db db-event"), "got: {:?}", stdout);
    assert!(!stdout.contains("api-event"), "subscriber should not see peer subtree; got: {:?}", stdout);
    assert!(!stdout.contains("db-only: app root"), "subscriber should not see root; got: {:?}", stdout);
}
