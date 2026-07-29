//! `std::log::StdoutSink` routes WARN/ERROR events to stderr;
//! INFO/DEBUG/TRACE stay on stdout. Verifies the routing fix
//! in `../../hale-stdlib/hl/log.hl`.

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run_split_streams(
    name: &str,
    source: &str,
) -> (String, String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_test_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status,
    )
}

#[test]
fn info_goes_to_stdout_warn_and_error_to_stderr() {
    let src = r#"
fn main() {
    std::log::StdoutSink { };
    let log = std::log::Logger { name: "test" };
    log.info("info-line");
    log.warn("warn-line");
    log.error("error-line");
    log.debug("debug-line");
    log.trace("trace-line");
}
"#;
    let (stdout, stderr, status) =
        build_and_run_split_streams("log_routing", src);
    assert!(status.success(), "non-zero: {:?}", status);

    // INFO / DEBUG / TRACE go to stdout.
    assert!(stdout.contains("info-line"), "stdout missing info: {:?}", stdout);
    assert!(stdout.contains("debug-line"), "stdout missing debug: {:?}", stdout);
    assert!(stdout.contains("trace-line"), "stdout missing trace: {:?}", stdout);
    // WARN / ERROR go to stderr.
    assert!(stderr.contains("warn-line"), "stderr missing warn: {:?}", stderr);
    assert!(stderr.contains("error-line"), "stderr missing error: {:?}", stderr);
    // WARN / ERROR do NOT leak into stdout.
    assert!(!stdout.contains("warn-line"), "warn leaked to stdout: {:?}", stdout);
    assert!(!stdout.contains("error-line"), "error leaked to stdout: {:?}", stdout);
    // INFO / DEBUG / TRACE do NOT leak into stderr.
    assert!(!stderr.contains("info-line"), "info leaked to stderr: {:?}", stderr);
    assert!(!stderr.contains("debug-line"), "debug leaked to stderr: {:?}", stderr);
}

#[test]
fn level_labels_appear_in_output() {
    let src = r#"
fn main() {
    std::log::StdoutSink { };
    let log = std::log::Logger { name: "lvl" };
    log.info("a");
    log.warn("b");
}
"#;
    let (stdout, stderr, _) = build_and_run_split_streams("log_level_labels", src);
    assert!(stdout.contains("[INFO lvl] a"), "got stdout: {:?}", stdout);
    assert!(stderr.contains("[WARN lvl] b"), "got stderr: {:?}", stderr);
}
