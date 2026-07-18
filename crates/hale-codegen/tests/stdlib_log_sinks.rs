//! std::log::FileSink + ConsoleSink (promoted from pond/logfmt,
//! 2026-07-18). FileSink: append + size-based rotation chain with
//! atomic rename shifts; ConsoleSink: badge/path rendering with the
//! WARN/ERROR stderr lane split; both are "log.**" drop-ins beside
//! StdoutSink.

use std::process::Command;

use hale_codegen::build_executable;

#[test]
fn file_sink_rotates_and_console_sink_renders() {
    let dir = std::env::temp_dir().join(format!(
        "hale_log_sinks_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let log_path = dir.join("app.log");

    let src = format!(
        r#"
        fn main() {{
            std::log::FileSink {{
                path: "{log}", max_size_bytes: 100, keep_files: 3
            }};
            std::log::ConsoleSink {{ color: false, show_time: false }};
            let log = std::log::Logger {{ name: "app" }};
            let db = std::log::Logger {{ name: "db", parent_path: "app" }};
            let mut i = 0;
            while i < 12 {{
                log.info("event number " + i + " with some padding text");
                i = i + 1;
            }}
            db.warn("retrying");
            db.error("gave up");
        }}
    "#,
        log = log_path.display()
    );
    let program = hale_syntax::parse_source(&src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_log_sinks_bin_{}", std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin)
        .env("NO_COLOR", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "exit: {:?}", out.status);

    // ConsoleSink: INFO on stdout, WARN/ERROR on stderr, badge +
    // cascaded path + message.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("INFO  app event number 0"),
        "stdout:\n{}",
        stdout
    );
    assert!(!stdout.contains("retrying"), "WARN must not hit stdout");
    assert!(stderr.contains("WARN  app.db retrying"), "stderr:\n{}", stderr);
    assert!(stderr.contains("ERROR app.db gave up"), "stderr:\n{}", stderr);
    // color: false + NO_COLOR ⇒ no SGR escapes anywhere.
    assert!(!stdout.contains('\u{1b}'), "no SGR expected:\n{}", stdout);

    // FileSink: the rotation chain exists and the ACTIVE file holds
    // the newest events (the WARN/ERROR tail), older chunks shifted
    // into .1/.2/.3, each rotated file over the 100-byte cap's
    // trigger point.
    let active =
        std::fs::read_to_string(&log_path).expect("active log file");
    assert!(active.contains("[WARN app.db] retrying"), "{}", active);
    assert!(active.contains("[ERROR app.db] gave up"), "{}", active);
    for i in 1..=3 {
        let p = dir.join(format!("app.log.{}", i));
        let content = std::fs::read_to_string(&p)
            .unwrap_or_else(|_| panic!("rotated file {} missing", i));
        assert!(
            content.contains("[INFO app] event number"),
            "rotated {} content:\n{}",
            i,
            content
        );
    }
    // keep_files = 3 ⇒ no .4 ever.
    assert!(!dir.join("app.log.4").exists(), "chain must cap at keep_files");

    let _ = std::fs::remove_dir_all(&dir);
}
