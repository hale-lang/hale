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

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn build_hale(name: &str, source: &str) -> std::path::PathBuf {
    let bin = harness::unique_bin(&format!("hale_test_log_{}", name));
    build_opts::build_source(source, &bin, &build_opts::options()).expect("build");
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
    let out = Command::new(&bin).env("HALE_LOG_FORMAT", "text").output().expect("run");
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
    let out = Command::new(&bin).env("HALE_LOG_FORMAT", "text").output().expect("run");
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
    let out = Command::new(&bin).env("HALE_LOG_FORMAT", "text").output().expect("run");
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
    let out = Command::new(&bin).env("HALE_LOG_FORMAT", "text").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("db-only: app.db db-event"), "got: {:?}", stdout);
    assert!(!stdout.contains("api-event"), "subscriber should not see peer subtree; got: {:?}", stdout);
    assert!(!stdout.contains("db-only: app root"), "subscriber should not see root; got: {:?}", stdout);
}

/// GH #1592: a sink writes records. With no format asked for, a
/// `StdoutSink` writes logfmt on stdout, every level on the one stream:
/// an RFC 3339 time, the level, the logger's path, the message and the
/// event's fields, quoted where they need it, and a line break in a value
/// written `\n`, so a record is one line.
const RECORDS: &str = r#"
    fn main() {
        std::log::StdoutSink { };
        let log = std::log::Logger { name: "app" };
        let db = std::log::Logger { name: "db", parent_path: "app" };
        log.info("starting");
        db.warn_kv("retrying", std::log::kv("attempt", "2") + " " + std::log::kv("why", "conn \"reset\" by=peer"));
        log.error("gave up\nfor good");
    }
"#;

fn run_records(format: Option<&str>) -> (String, String) {
    let bin = build_hale(&format!("records_{}", format.unwrap_or("default")), RECORDS);
    let mut cmd = Command::new(&bin);
    match format {
        Some(f) => {
            cmd.env("HALE_LOG_FORMAT", f);
        }
        None => {
            cmd.env_remove("HALE_LOG_FORMAT");
        }
    }
    let out = cmd.output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    (String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

/// `time=<RFC 3339 UTC>` and the rest of the line after it.
fn split_time(line: &str) -> (&str, &str) {
    let rest = line.strip_prefix("time=").unwrap_or_else(|| panic!("a record starts with its time: {line:?}"));
    let (t, rest) = rest.split_once(' ').expect("fields after the time");
    assert!(t.len() == 20 && t.ends_with('Z') && t.as_bytes()[10] == b'T', "RFC 3339 UTC: {t:?}");
    (t, rest)
}

#[test]
fn a_sink_writes_logfmt_records_by_default() {
    let (stdout, stderr) = run_records(None);
    assert!(stderr.is_empty(), "every record on stdout: {stderr:?}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "one line per record: {stdout:?}");
    assert_eq!(split_time(lines[0]).1, "level=info logger=app msg=starting");
    assert_eq!(
        split_time(lines[1]).1,
        r#"level=warn logger=app.db msg=retrying attempt=2 why="conn \"reset\" by=peer""#
    );
    assert_eq!(split_time(lines[2]).1, r#"level=error logger=app msg="gave up\nfor good""#);
    // an unknown format is logfmt too
    let (bogus, _) = run_records(Some("yaml"));
    assert_eq!(bogus.lines().count(), 3);
    assert!(bogus.lines().all(|l| l.starts_with("time=")), "{bogus:?}");
}

#[test]
fn hale_log_format_json_writes_one_object_per_record() {
    let (stdout, stderr) = run_records(Some("json"));
    assert!(stderr.is_empty(), "{stderr:?}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "{stdout:?}");
    let tail = |l: &str| l.split_once("\",\"level\"").map(|(_, t)| t.to_string()).expect("time, then level");
    assert!(lines[0].starts_with("{\"time\":\""), "{}", lines[0]);
    assert_eq!(tail(lines[0]), r#":"info","logger":"app","msg":"starting"}"#);
    assert_eq!(
        tail(lines[1]),
        r#":"warn","logger":"app.db","msg":"retrying","attempt":"2","why":"conn \"reset\" by=peer"}"#
    );
    assert_eq!(tail(lines[2]), r#":"error","logger":"app","msg":"gave up\nfor good"}"#);
}

#[test]
fn hale_log_format_text_keeps_the_format_before_records() {
    let (stdout, stderr) = run_records(Some("text"));
    assert!(stdout.contains("[INFO app] starting"), "{stdout:?}");
    assert!(stderr.contains(r#"[WARN app.db] retrying attempt=2 why="conn \"reset\" by=peer""#), "{stderr:?}");
    assert!(stderr.contains("[ERROR app] gave up"), "{stderr:?}");
}
