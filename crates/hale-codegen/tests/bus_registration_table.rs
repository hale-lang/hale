//! The registration table grows while another thread walks it (F.40
//! phase 3, L5's fourth part; inventory row R50).
//!
//! `tests/fixtures/bus_table/registration_under_dispatch.hl`: one pinned
//! anchor publishes for 300 ms from its own thread while a second pinned
//! anchor's initialization registers 160 subscriptions on its own (C49),
//! growing the table and each topic's static bucket several times over
//! under the walks. Before the table was published as a (pointer, count)
//! pair with replaced arrays retired, `lotus_bus_register_keyed` grew it
//! by `realloc` with no lock: under AddressSanitizer with chunk pooling
//! off, 5 of 10 runs reported a heap-use-after-free in
//! `lotus_bus_local_dispatch` (dynamic dispatch) and 10 of 10 a
//! heap-use-after-free or a SEGV in `lotus_bus_dispatch_static` (the
//! bucket directory's `realloc`).

use std::path::PathBuf;
use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const FIXTURE: &str = "registration_under_dispatch.hl";

/// Runs per dispatch mode: the defect showed in at least half of them.
const RUNS: usize = 3;

fn source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bus_table").join(FIXTURE);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn the_fixture_is_formatted() {
    let src = source();
    let formatted = hale_syntax::fmt::format_source(&src).unwrap_or_else(|e| panic!("{FIXTURE}: fmt: {e:?}"));
    assert!(formatted == src, "{FIXTURE} is not `hale fmt` clean; formatted:\n{formatted}");
}

/// Under ASan, chunk pooling off (GH #816), in both dispatch modes: every
/// run prints both lines, exits 0, and reports nothing.
#[test]
fn registrations_on_an_anchors_thread_never_free_the_table_under_a_walk() {
    for no_bus_devirt in [false, true] {
        let bin = harness::unique_bin(&format!("bus_table_{no_bus_devirt}"));
        let options = hale_codegen::BuildOptions { asan: true, no_bus_devirt, ..build_opts::options() };
        build_opts::build_source(&source(), &bin, &options)
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
            let what = format!("no_bus_devirt={no_bus_devirt}, run {run}");
            assert!(
                !stderr.contains("ERROR: AddressSanitizer") && !stderr.contains("ERROR: LeakSanitizer"),
                "{what}: the sanitizer reported\n{stderr}"
            );
            assert_eq!(out.status.code(), Some(0), "{what}: exit\n{stdout}\n{stderr}");
            assert_eq!(stdout.lines().collect::<Vec<_>>(), ["ev published", "ev done"], "{what}\n{stderr}");
        }
        let _ = std::fs::remove_file(&bin);
    }
}
