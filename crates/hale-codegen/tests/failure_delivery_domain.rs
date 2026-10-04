//! Decision L0-1 (spec/runtime.md § Failure handling): a child's failure
//! is delivered to its owner's `on_failure` on the owner's execution
//! domain, never on the failing child's thread, and the owner's reclaim
//! of the child waits for a delivery in flight (F.40 phase 3, L5's
//! fourth part; inventory rows C36 and C39).
//!
//! Two fixtures under `tests/fixtures/failure_delivery/`, each judged
//! into one outcome word from its `ev` lines:
//!
//!   * `fd_pinned_owner_state.hl`: a pinned child fails in a bus handler
//!     while its owner, on main, reads the state the handler writes in a
//!     window with no yield. The word names whether that state changed
//!     inside the window, the thread the handler ran on, and how often
//!     it ran.
//!   * `fd_reclaim_under_delivery.hl`: a pool-placed child fails in a bus
//!     handler, and its owner replaces it while the delivery is in
//!     flight. Gate files force the order. The word names the handler's
//!     thread and whether the handler read the old child before that
//!     child dissolved.
//!
//! [`KNOWN_OPEN`] gives each fixture's word today, with the row it
//! departs at; the test asserts it, so the protocol that delivers on the
//! owner's domain fails the assertion and the entry has to go.
//!
//! The fixtures stay out of the example corpus: the corpus oracle runs
//! every example to completion, and these read gate files and the
//! thread's identity (`pthread_self`).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hale_codegen::build_executable_with_options;
use hale_types::lifecycle::trace::{self, Trace};
use hale_types::lifecycle::{ObligationKind, Point};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const DEADLINE: Duration = Duration::from_secs(20);

/// The outcome each fixture is adopted to give.
const ADOPTED: &[(&str, &str)] = &[
    ("fd_pinned_owner_state.hl", "held-in-window on-owner heard-1"),
    ("fd_reclaim_under_delivery.hl", "on-owner read-before-dissolve dissolved-once-each"),
];

/// What each fixture gives today, where it differs: (fixture, inventory
/// row, today's word). Empty since the protocol landed: before it, the
/// handler ran in place on the pinned thread, beside the owner's own code
/// in its window (`changed-in-window off-owner heard-1`), and on the
/// pool's worker while the owner's replacement reclaimed the old child
/// under it (`off-owner read-after-dissolve dissolved-once-each`, a
/// heap-use-after-free under ASan).
const KNOWN_OPEN: &[(&str, &str, &str)] = &[];

struct Ran {
    stdout: String,
    stderr: String,
    trace: Trace,
    code: Option<i32>,
    timed_out: bool,
}

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/failure_delivery")
}

fn program(file: &str) -> hale_syntax::ast::Program {
    let path = dir().join(file);
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let program = hale_syntax::parse_source(&src).unwrap_or_else(|e| panic!("{file}: parse: {e:?}"));
    let errs: Vec<String> =
        hale_types::check_program(&program).iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect();
    assert!(errs.is_empty(), "{file}: `hale check` refuses it: {errs:?}");
    program
}

fn build(file: &str, asan: bool, no_bus_devirt: bool) -> PathBuf {
    let bin = harness::unique_bin(&format!("hale_fd_{}", file.trim_end_matches(".hl")));
    let options = hale_codegen::BuildOptions { asan, lifecycle_trace: !asan, no_bus_devirt, ..build_opts::options() };
    build_executable_with_options(&program(file), &bin, &[], &options).unwrap_or_else(|e| panic!("{file}: build: {e:?}"));
    if asan {
        let image = std::fs::read(&bin).expect("read the ASan binary");
        assert!(image.windows(b"__asan_init".len()).any(|w| w == b"__asan_init"), "ASan instrumentation is required");
    }
    bin
}

fn run_bin(bin: &Path, env: &[(&str, &str)]) -> Ran {
    let mut child = Command::new(bin)
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the fixture");
    let mut out = child.stdout.take().expect("stdout");
    let mut err = child.stderr.take().expect("stderr");
    let out_reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let err_reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break Some(status);
        }
        if start.elapsed() > DEADLINE {
            let _ = child.kill();
            let _ = child.wait();
            timed_out = true;
            break None;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let stderr = err_reader.join().unwrap_or_default();
    let trace = trace::parse(&stderr).unwrap_or_else(|e| panic!("{}: the trace does not parse: {e}", bin.display()));
    Ran { stdout: out_reader.join().unwrap_or_default(), stderr: trace.rest.clone(), trace, code: status.and_then(|s| s.code()), timed_out }
}

fn count(r: &Ran, line: &str) -> usize {
    r.stdout.lines().filter(|l| *l == line).count()
}

fn pos(r: &Ran, line: &str) -> Option<usize> {
    r.stdout.lines().position(|l| l == line)
}

fn exit_word(r: &Ran) -> Option<String> {
    if r.timed_out {
        return Some("timeout".into());
    }
    match r.code {
        Some(0) => None,
        Some(c) => Some(format!("exit-{c}")),
        None => Some("signal".into()),
    }
}

fn thread_word(r: &Ran) -> &'static str {
    match (count(r, "ev handler-on-owner"), count(r, "ev handler-off-owner")) {
        (n, 0) if n > 0 => "on-owner",
        (0, n) if n > 0 => "off-owner",
        (0, 0) => "unheard",
        _ => "both",
    }
}

/// `fd_pinned_owner_state.hl`: the window, the handler's thread, how
/// often it ran.
fn owner_state(r: &Ran) -> String {
    if let Some(w) = exit_word(r) {
        return w;
    }
    let window = match (count(r, "ev state-held-in-window"), count(r, "ev state-changed-in-window")) {
        (1, 0) => "held-in-window",
        (0, 1) => "changed-in-window",
        _ => "no-window",
    };
    let heard = if count(r, "ev heard 1 rows 257") == 1 { "heard-1" } else { "heard-other" };
    format!("{window} {} {heard}", thread_word(r))
}

/// `fd_reclaim_under_delivery.hl`: the handler's thread, whether it read
/// the old child before that child dissolved, and the dissolve counts.
fn reclaim_under_delivery(r: &Ran) -> String {
    if let Some(w) = exit_word(r) {
        return w;
    }
    // The handler's read: what it printed of the old child's name (a
    // freed String reads as whatever reused its bytes), and where.
    let read_at = r.stdout.lines().position(|l| l.starts_with("ev handler "));
    let whole = count(r, "ev handler kid-0-name") == 1;
    let read = match (read_at, pos(r, "ev kid-dissolve 0")) {
        (Some(h), Some(d)) if h < d && whole => "read-before-dissolve",
        (Some(h), Some(d)) if h < d => "read-torn-before-dissolve",
        (Some(_), Some(_)) => "read-after-dissolve",
        (None, _) => "never-read",
        (Some(_), None) => "never-dissolved",
    };
    let once = if count(r, "ev kid-dissolve 0") == 1 && count(r, "ev kid-dissolve 1") == 1 {
        "dissolved-once-each"
    } else {
        "dissolved-other"
    };
    format!("{} {read} {once}", thread_word(r))
}

fn judge(file: &str, r: &Ran) -> String {
    match file {
        "fd_pinned_owner_state.hl" => owner_state(r),
        "fd_reclaim_under_delivery.hl" => reclaim_under_delivery(r),
        _ => panic!("{file} has no judge"),
    }
}

const SANITIZER_MARKERS: &[&str] = &[
    "ERROR: AddressSanitizer",
    "ERROR: LeakSanitizer",
    "heap-use-after-free",
    "double-free",
    "heap-buffer-overflow",
    "SEGV on unknown address",
];

fn sanitizer_hits(r: &Ran) -> Vec<&'static str> {
    let report = [r.stdout.as_str(), r.stderr.as_str()].concat();
    SANITIZER_MARKERS.iter().copied().filter(|m| report.contains(m)).collect()
}

/// The domains `Kid`'s failure delivery ran on, entry and end alike.
fn delivery_domains(r: &Ran) -> Vec<String> {
    r.trace
        .events
        .iter()
        .filter(|e| e.kind == ObligationKind::FailureDelivery && e.decl.as_deref() == Some("Kid"))
        .filter(|e| matches!(e.point, Point::Entered | Point::Completed))
        .map(|e| e.domain.clone())
        .collect()
}

fn report(r: &Ran) -> String {
    format!("--- stdout\n{}--- stderr\n{}", r.stdout, r.stderr)
}

fn adopted(file: &str) -> &'static str {
    ADOPTED.iter().find(|(f, _)| *f == file).map(|(_, w)| *w).unwrap_or_else(|| panic!("{file} has no adopted word"))
}

fn known_open(file: &str) -> Option<(&'static str, &'static str)> {
    KNOWN_OPEN.iter().find(|(f, ..)| *f == file).map(|(_, row, w)| (*row, *w))
}

/// The trace build: today's word where the fixture is known open, the
/// adopted one otherwise; and where the delivery ran.
fn assert_traced(file: &str) {
    let bin = build(file, false, false);
    let ran = run_bin(&bin, &[]);
    let _ = std::fs::remove_file(&bin);
    let got = judge(file, &ran);
    let domains = delivery_domains(&ran);
    eprintln!("{file}: {got}; FailureDelivery on {domains:?}\n{}", report(&ran));
    match known_open(file) {
        Some((row, today)) => assert_eq!(
            got, today,
            "{file} (KNOWN_OPEN at {row}) no longer gives today's word; adopted is `{}`, so the entry has to go\n{}",
            adopted(file),
            report(&ran)
        ),
        None => {
            assert_eq!(got, adopted(file), "{file}\n{}", report(&ran));
            assert!(!domains.is_empty() && domains.iter().all(|d| d == "main"), "{file}: the delivery ran on {domains:?}, not main");
        }
    }
}

/// Every fixture is judged here and is `hale fmt` clean (the CI gate
/// walks every `.hl` file).
#[test]
fn every_fixture_is_listed_and_formatted() {
    let mut on_disk: Vec<String> = std::fs::read_dir(dir())
        .expect("read the fixture directory")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".hl"))
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> = ADOPTED.iter().map(|(f, _)| f.to_string()).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "a fixture on disk has no adopted word, or one listed is missing");
    for file in &on_disk {
        let src = std::fs::read_to_string(dir().join(file)).expect("read the fixture");
        let formatted = hale_syntax::fmt::format_source(&src).unwrap_or_else(|e| panic!("{file}: fmt: {e:?}"));
        assert!(formatted == src, "{file} is not `hale fmt` clean; formatted:\n{formatted}");
    }
}

#[test]
fn a_pinned_childs_failure_handler_runs_on_its_owners_thread() {
    assert_traced("fd_pinned_owner_state.hl");
}

#[test]
fn an_owner_never_reclaims_a_child_under_its_failure_delivery() {
    assert_traced("fd_reclaim_under_delivery.hl");
}

/// Both fixtures under AddressSanitizer, with chunk pooling off (GH
/// #816), in both dispatch modes: the adopted word and nothing reported.
/// Before the protocol, the reclaim fixture's handler read the old
/// child's freed arena (a heap-use-after-free).
#[test]
fn both_fixtures_hold_under_asan_in_both_dispatch_modes() {
    for (file, _) in ADOPTED {
        for no_bus_devirt in [false, true] {
            let bin = build(file, true, no_bus_devirt);
            let ran = run_bin(&bin, &[("ASAN_OPTIONS", "detect_leaks=1"), ("LOTUS_NO_CHUNK_POOL", "1")]);
            let _ = std::fs::remove_file(&bin);
            let hits = sanitizer_hits(&ran);
            assert!(hits.is_empty(), "{file}, no_bus_devirt={no_bus_devirt}: {hits:?}\n{}", report(&ran));
            assert_eq!(judge(file, &ran), adopted(file), "{file} under ASan, no_bus_devirt={no_bus_devirt}\n{}", report(&ran));
        }
    }
}
