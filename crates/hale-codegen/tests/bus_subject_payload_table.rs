//! The subject-to-payload table grows while another thread reads it
//! (F.40 phase 3, L1's fifth part; the twin of inventory row R50).
//!
//! `tests/fixtures/bus_table/subject_payload_under_publish.hl`: one
//! pinned anchor publishes to a computed subject for 300 ms from its own
//! thread, and every such publish reads the whole table
//! (`lotus_bus_subject_payload_conflicts`), while a second pinned
//! anchor's initialization records 128 subjects on its own thread
//! (`lotus_bus_declare_subject_payload`, C49), growing the table four
//! times under the reads.

use std::path::PathBuf;
use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const FIXTURE: &str = "subject_payload_under_publish.hl";
const MISMATCH: &str = "subject_payload_mismatch.hl";

/// Runs per dispatch mode. Before the change, 18 of 20 runs (9 of 10 in
/// each mode, twice over) reported a heap-use-after-free in
/// `lotus_bus_subject_payload_conflicts` on the array
/// `lotus_bus_declare_subject_payload` had grown by `realloc` on the
/// anchor's thread, or a SEGV in its `strstr`.
const RUNS: usize = 3;

fn source(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bus_table").join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn the_fixtures_check_and_are_formatted() {
    for file in [FIXTURE, MISMATCH] {
        let src = source(file);
        let formatted = hale_syntax::fmt::format_source(&src).unwrap_or_else(|e| panic!("{file}: fmt: {e:?}"));
        assert!(formatted == src, "{file} is not `hale fmt` clean; formatted:\n{formatted}");
        let program = hale_syntax::parse_source(&src).unwrap_or_else(|e| panic!("{file}: parse: {e:?}"));
        let errs: Vec<String> =
            hale_types::check_program(&program).iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect();
        assert!(errs.is_empty(), "{file}: `hale check` refuses it: {errs:?}");
    }
}

/// The table still answers: a computed publish reaching a subscriber
/// declared for another payload is refused, in both dispatch modes.
#[test]
fn a_computed_publish_reaching_another_payloads_subscriber_is_refused() {
    let program = hale_syntax::parse_source(&source(MISMATCH)).unwrap_or_else(|e| panic!("{MISMATCH}: parse: {e:?}"));
    for no_bus_devirt in [false, true] {
        let bin = harness::unique_bin(&format!("subj_pay_mismatch_{no_bus_devirt}"));
        let options = hale_codegen::BuildOptions { no_bus_devirt, ..build_opts::options() };
        hale_codegen::build_executable_with_options(&program, &bin, &[], &options)
            .unwrap_or_else(|e| panic!("{MISMATCH}: build: {e:?}"));
        let out = Command::new("timeout").arg("60").arg(&bin).output().expect("run the fixture");
        let _ = std::fs::remove_file(&bin);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let what = format!("no_bus_devirt={no_bus_devirt}");
        assert!(!stdout.contains("ev delivered"), "{what}: delivered and reinterpreted\n{stdout}");
        assert!(!out.status.success(), "{what}: a mismatched publish must not succeed quietly\n{stderr}");
        assert!(stderr.contains("BusPayloadMismatch"), "{what}: expected the mismatch panic\n{stderr}");
    }
}

/// Under ASan, chunk pooling off (GH #816), in both dispatch modes: every
/// run prints both lines, exits 0, and reports nothing.
#[test]
fn recording_a_subject_on_an_anchors_thread_never_frees_the_table_under_a_read() {
    let program = hale_syntax::parse_source(&source(FIXTURE)).unwrap_or_else(|e| panic!("{FIXTURE}: parse: {e:?}"));
    let mut failed = Vec::new();
    for no_bus_devirt in [false, true] {
        let bin = harness::unique_bin(&format!("subj_pay_{no_bus_devirt}"));
        let options = hale_codegen::BuildOptions { asan: true, no_bus_devirt, ..build_opts::options() };
        hale_codegen::build_executable_with_options(&program, &bin, &[], &options)
            .unwrap_or_else(|e| panic!("{FIXTURE}: build: {e:?}"));
        let image = std::fs::read(&bin).expect("read the ASan binary");
        assert!(image.windows(b"__asan_init".len()).any(|w| w == b"__asan_init"), "ASan instrumentation is required");
        for run in 0..RUNS {
            let out = Command::new("timeout")
                .arg("60")
                .arg(&bin)
                .env("ASAN_OPTIONS", "detect_leaks=1")
                .env("LOTUS_NO_CHUNK_POOL", "1")
                .output()
                .expect("run the fixture");
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let clean = !stderr.contains("ERROR: AddressSanitizer")
                && !stderr.contains("ERROR: LeakSanitizer")
                && out.status.code() == Some(0)
                && stdout.lines().collect::<Vec<_>>() == ["ev published", "ev done"];
            if !clean {
                failed.push(format!("no_bus_devirt={no_bus_devirt}, run {run}: exit {:?}\n{stdout}\n{stderr}", out.status.code()));
            }
        }
        let _ = std::fs::remove_file(&bin);
    }
    assert!(failed.is_empty(), "{} of {} runs failed:\n{}", failed.len(), 2 * RUNS, failed.join("\n"));
}
