//! Decision L0-1 (spec/runtime.md § Failure handling): a child's failure
//! is delivered to its owner's `on_failure` on the owner's execution
//! domain, never on the failing child's thread, and the owner's reclaim
//! of the child waits for a delivery in flight (F.40 phase 3, L5's
//! fourth part; inventory rows C36 and C39).
//!
//! The fixtures under `tests/fixtures/failure_delivery/`, each judged
//! into one outcome word from its output:
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
//!   * `fd_sibling_replace*.hl`: the owner's handler for one child
//!     replaces a sibling whose own failure is posted to the owner and
//!     not yet delivered. That delivery can only run on the owner's
//!     thread, which is inside the handler (handlers do not nest), so a
//!     reclaim that waited for it would never end: the reclaim is
//!     deferred behind it instead (PR #1348's review). The bare case and
//!     its control, a child carrying a heap String the late handler
//!     reads, three siblings, and the replacement through a method. The
//!     word is the program's output, or for the heap and three-sibling
//!     cases the order the handlers read and the children dissolved in.
//!     Each run is under [`DEADLINE`], so a hang is a `timeout` word, not
//!     a stuck suite.
//!   * `fd_settle_no_nest.hl`: a failure held while its parent's params
//!     were open is delivered at the parent's settle, and its handler
//!     sleeps while another failure is posted to the same thread. The
//!     posted handler starts after the held one returns (handlers do not
//!     nest). The word is the order of the handlers' entry and exit lines.
//!   * `fd_handler_cell*.hl`, inventory row R52: a bus handler's cell
//!     holds its subscriber, as a started run holds its child. A
//!     pool-placed child is replaced while its bus handler runs, with no
//!     failure involved (`fd_handler_cell_no_hold.hl`, the window the
//!     sibling fixtures leave 250 ms to stay out of); the handler reads
//!     its heap name whole, since the reclaim waits for the handler's
//!     hold. The two waits that could close on themselves: the owner's
//!     own failure handler replaces and restarts the child whose bus
//!     handler is waiting for that delivery
//!     (`fd_handler_cell_restart_replaced.hl`), and the reclaim waits for
//!     a handler that fails while main is inside that wait, so the
//!     handler waits for main (`fd_handler_cell_blocked_on_owner.hl`).
//!     A pool worker carries a handler's hold to its next cell when that
//!     cell is the same subscriber's, and ends it for a reclaim that waits
//!     for it: a subscriber its worker never finds idle, flooded by a
//!     publisher, is replaced mid-flood (`fd_handler_cell_flooded.hl`),
//!     and two subscribers sharing a worker, their cells interleaved, one
//!     of them replaced mid-stream (`fd_handler_cell_interleaved.hl`).
//!     Each is run under [`DEADLINE`].
//!   * `fd_restart*_replaced*.hl`: the reclaim wins. A handler asks for a
//!     restart (`restart`, `restart_in_place`) of a child whose reclaim is
//!     owed: the child its own delivery is about, after replacing it; a
//!     sibling it replaced while that sibling's failure was posted; or a
//!     child its owner replaced outside any handler while its failure was
//!     posted, whose reclaim runs the delivery as it waits for it. The
//!     decision is not performed. The word counts the old child's births,
//!     runs and dissolves, whether its handler read its heap name before
//!     it dissolved, and the new child's counts.
//!   * `fd_pool_owner_worker.hl`: the owner is placed on a pool (decision
//!     line 1's pool-placed owner). One child fails while the owner's
//!     params are open on the worker (held to the settle), another later
//!     on main (posted). The word names the thread each handler ran on
//!     against the worker's, read with `pthread_self` inside the handler.
//!
//! [`KNOWN_OPEN`] gives each fixture's word today, with the row it
//! departs at; the test asserts it, so the protocol that delivers on the
//! owner's domain fails the assertion and the entry has to go.
//!
//! The fixtures stay out of the example corpus: the corpus oracle runs
//! every example to completion, and these read gate files and the
//! thread's identity (`pthread_self`).

#[path = "../../hale-types/tests/support/entries.rs"]
mod entries;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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
    ("fd_sibling_replace.hl", SIBLING_OUTPUT),
    ("fd_sibling_replace_control.hl", SIBLING_OUTPUT),
    ("fd_sibling_replace_heap.hl", "on-owner read-before-dissolve dissolved-once-each"),
    ("fd_sibling_replace_three.hl", "handled-0-1-2 reclaimed-after-own-handler dissolved-once-each"),
    ("fd_sibling_replace_method.hl", SIBLING_OUTPUT),
    ("fd_restart_replaced.hl", RECLAIM_WINS),
    ("fd_restart_in_place_replaced.hl", RECLAIM_WINS),
    ("fd_restart_sibling_replaced.hl", RECLAIM_WINS),
    ("fd_restart_replaced_by_owner.hl", RECLAIM_WINS),
    ("fd_settle_no_nest.hl", "ev held-enter / ev held-exit / ev posted-enter / ev posted-exit / ev finished"),
    ("fd_handler_cell_no_hold.hl", "read-whole"),
    (
        "fd_handler_cell_restart_replaced.hl",
        "old-born-1 old-dissolved-1 read-whole delivered-whole new-born-1 new-dissolved-1 finished",
    ),
    ("fd_handler_cell_blocked_on_owner.hl", "read-whole delivered-whole dissolved-once-each finished"),
    ("fd_handler_cell_flooded.hl", "replaced flood-running read-whole dissolved-once-each finished"),
    ("fd_handler_cell_interleaved.hl", "replaced a-after read-whole dissolved-once-each finished"),
    ("fd_pool_owner_worker.hl", "raised-off-worker held-on-worker posted-on-worker heard-2"),
];

/// The domain each traced fixture's deliveries are traced on, and the
/// declarations whose deliveries those are.
const DELIVERED_ON: &[(&str, &str, &[&str])] = &[
    ("fd_pinned_owner_state.hl", "main", &["Kid"]),
    ("fd_reclaim_under_delivery.hl", "main", &["Kid"]),
    ("fd_sibling_replace.hl", "main", &["Kid"]),
    ("fd_sibling_replace_control.hl", "main", &["Kid"]),
    ("fd_sibling_replace_heap.hl", "main", &["Kid"]),
    ("fd_sibling_replace_three.hl", "main", &["Kid"]),
    ("fd_sibling_replace_method.hl", "main", &["Kid"]),
    ("fd_restart_replaced.hl", "main", &["Kid"]),
    ("fd_restart_in_place_replaced.hl", "main", &["Kid"]),
    ("fd_restart_sibling_replaced.hl", "main", &["Kid"]),
    ("fd_restart_replaced_by_owner.hl", "main", &["Kid"]),
    ("fd_settle_no_nest.hl", "main", &["Kid"]),
    ("fd_handler_cell_restart_replaced.hl", "main", &["Kid"]),
    ("fd_handler_cell_blocked_on_owner.hl", "main", &["Kid"]),
    ("fd_handler_cell_flooded.hl", "main", &[]),
    ("fd_handler_cell_interleaved.hl", "main", &[]),
    ("fd_pool_owner_worker.hl", "pool:side", &["Boom", "Kid"]),
];

/// A restart asked for about a child whose reclaim is owed: the old child
/// is born and run once and dissolved once, after its handler read it;
/// the new child in its field is untouched.
const RECLAIM_WINS: &str =
    "old-born-1 old-ran-1 old-dissolved-1 read-before-dissolve new-born-1 new-ran-1 new-dissolved-1";

/// The review's reproducer's output, with the replacement and without:
/// the replaced sibling's failure is still delivered, after the handler
/// that replaced it returns. Before the rule, the replacing run printed
/// `handling 0 / replacing sibling` and never ended.
const SIBLING_OUTPUT: &str = "handling 0 / replacing sibling / replaced sibling / handling 1 / finished";

/// What each fixture gives today, where it differs: (fixture, inventory
/// row, today's word). The delivery protocol's own entries went when it
/// landed: before it, the handler ran in place on the pinned thread,
/// beside the owner's own code in its window (`changed-in-window
/// off-owner heard-1`), and on the pool's worker while the owner's
/// replacement reclaimed the old child under it (`off-owner
/// read-after-dissolve dissolved-once-each`, a heap-use-after-free under
/// ASan). `fd_handler_cell_no_hold.hl`'s went when a bus handler's cell
/// came to hold its subscriber (R52): before, replacing a pool-placed
/// child freed it under its running handler, a `heap-use-after-free`
/// under ASan in both dispatch modes.
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

/// The fixture's text, once `hale check` takes it.
fn checked_source(file: &str) -> String {
    let path = dir().join(file);
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let program = hale_syntax::parse_source(&src).unwrap_or_else(|e| panic!("{file}: parse: {e:?}"));
    let errs: Vec<String> =
        entries::check_program(&program).iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect();
    assert!(errs.is_empty(), "{file}: `hale check` refuses it: {errs:?}");
    src
}

fn build(file: &str, asan: bool, no_bus_devirt: bool) -> PathBuf {
    build_with(file, asan, !asan, no_bus_devirt)
}

fn build_with(file: &str, asan: bool, lifecycle_trace: bool, no_bus_devirt: bool) -> PathBuf {
    let bin = harness::unique_bin(&format!("hale_fd_{}", file.trim_end_matches(".hl")));
    let options = hale_codegen::BuildOptions { asan, lifecycle_trace, no_bus_devirt, ..build_opts::options() };
    build_opts::build_source(&checked_source(file), &bin, &options).unwrap_or_else(|e| panic!("{file}: build: {e:?}"));
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

/// The sibling replacement's plain cases: what the program printed, in
/// order.
fn output(r: &Ran) -> String {
    if let Some(w) = exit_word(r) {
        return format!("{w} after: {}", r.stdout.lines().collect::<Vec<_>>().join(" / "));
    }
    r.stdout.lines().collect::<Vec<_>>().join(" / ")
}

/// `fd_sibling_replace_heap.hl`: the handlers' thread, whether the
/// replaced child's late handler read its name before it dissolved, and
/// the dissolve counts.
fn sibling_heap(r: &Ran) -> String {
    if let Some(w) = exit_word(r) {
        return w;
    }
    let read = match (pos(r, "ev handler kid-1-name"), pos(r, "ev kid-dissolve 1"), pos(r, "ev replaced sibling")) {
        (Some(h), Some(d), Some(s)) if s < h && h < d => "read-before-dissolve",
        (Some(_), Some(_), Some(_)) => "read-out-of-order",
        _ => "never-read",
    };
    let once = if (0..3).all(|t| count(r, &format!("ev kid-dissolve {t}")) == 1) {
        "dissolved-once-each"
    } else {
        "dissolved-other"
    };
    format!("{} {read} {once}", thread_word(r))
}

/// `fd_sibling_replace_three.hl`: the order the handlers ran in, whether
/// each replaced child dissolved after its own handler and before the
/// owner went on, and the dissolve counts (three old children, two
/// replacements).
fn sibling_three(r: &Ran) -> String {
    if let Some(w) = exit_word(r) {
        return w;
    }
    let handled: Vec<&str> = r.stdout.lines().filter_map(|l| l.strip_prefix("ev handling ")).collect();
    let after = [1, 2].iter().all(|t| {
        match (pos(r, &format!("ev handling {t}")), pos(r, &format!("ev kid-dissolve {t}")), pos(r, "ev finished")) {
            (Some(h), Some(d), Some(f)) => h < d && d < f,
            _ => false,
        }
    });
    let after = if after { "reclaimed-after-own-handler" } else { "reclaimed-out-of-order" };
    let once = if (0..5).all(|t| count(r, &format!("ev kid-dissolve {t}")) == 1) {
        "dissolved-once-each"
    } else {
        "dissolved-other"
    };
    format!("handled-{} {after} {once}", handled.join("-"))
}

/// `fd_restart*_replaced*.hl`: the old child's and the new child's
/// births, runs and dissolves, and whether the old child's handler read
/// its heap name before it dissolved.
fn reclaim_wins(file: &str, r: &Ran) -> String {
    if let Some(w) = exit_word(r) {
        return format!("{w} after: {}", r.stdout.lines().collect::<Vec<_>>().join(" / "));
    }
    let (old, new) = if file == "fd_restart_sibling_replaced.hl" { (1, 2) } else { (0, 5) };
    let lines = |prefix: String| r.stdout.lines().filter(|l| l.starts_with(&prefix)).count();
    let counts = |who: &str, t: i32| {
        format!(
            "{who}-born-{} {who}-ran-{} {who}-dissolved-{}",
            count(r, &format!("ev kid-birth {t}")),
            lines(format!("ev kid-run {t} ")),
            lines(format!("ev kid-dissolve {t} ")),
        )
    };
    let name = format!("kid-{old}-name");
    let read = match (pos(r, &format!("ev handler {name}")), pos(r, &format!("ev kid-dissolve {old} {name}"))) {
        (Some(h), Some(d)) if h < d => "read-before-dissolve",
        (Some(_), Some(_)) => "read-after-dissolve",
        (None, _) => "never-read",
        (Some(_), None) => "never-dissolved-whole",
    };
    format!("{} {read} {}", counts("old", old), counts("new", new))
}

/// `fd_handler_cell_no_hold.hl`: whether the running handler read its
/// child's name whole, or, under ASan, the use-after-free it reported.
fn handler_cell(r: &Ran) -> String {
    if sanitizer_hits(r).contains(&"heap-use-after-free") {
        return "heap-use-after-free".into();
    }
    if let Some(w) = exit_word(r) {
        return w;
    }
    read_word(r).into()
}

fn read_word(r: &Ran) -> &'static str {
    if count(r, "ev handler-read kid-1-name") == 1 { "read-whole" } else { "read-torn" }
}

fn delivered_word(r: &Ran) -> &'static str {
    if count(r, "ev delivered kid-1-name") == 1 { "delivered-whole" } else { "delivered-torn" }
}

fn finished_word(r: &Ran) -> &'static str {
    if count(r, "ev finished") == 1 { "finished" } else { "unfinished" }
}

/// `fd_handler_cell_restart_replaced.hl`: the old child's and the new
/// one's births and dissolves (the restart is not performed), and whether
/// the bus handler and the delivery each read the old child whole.
fn handler_cell_restart(r: &Ran) -> String {
    if let Some(w) = exit_word(r) {
        return format!("{w} after: {}", r.stdout.lines().collect::<Vec<_>>().join(" / "));
    }
    format!(
        "old-born-{} old-dissolved-{} {} {} new-born-{} new-dissolved-{} {}",
        count(r, "ev kid-birth 1"),
        count(r, "ev kid-dissolve 1"),
        read_word(r),
        delivered_word(r),
        count(r, "ev kid-birth 2"),
        count(r, "ev kid-dissolve 2"),
        finished_word(r),
    )
}

/// `fd_handler_cell_blocked_on_owner.hl`: whether the bus handler and the
/// delivery it waited for read the old child whole, and the dissolves.
fn handler_cell_blocked(r: &Ran) -> String {
    if let Some(w) = exit_word(r) {
        return format!("{w} after: {}", r.stdout.lines().collect::<Vec<_>>().join(" / "));
    }
    let once = if count(r, "ev kid-dissolve 1") == 1 && count(r, "ev kid-dissolve 2") == 1 {
        "dissolved-once-each"
    } else {
        "dissolved-other"
    };
    format!("{} {} {once} {}", read_word(r), delivered_word(r), finished_word(r))
}

/// `fd_handler_cell_flooded.hl` and `fd_handler_cell_interleaved.hl`:
/// whether the replacement returned, what ran beside it (the flood, or
/// the other subscriber's cells), whether every handler read its own
/// name whole, and each child's dissolve.
fn handler_cell_carried(file: &str, r: &Ran) -> String {
    if let Some(w) = exit_word(r) {
        return format!("{w} after: {}", r.stdout.lines().collect::<Vec<_>>().join(" / "));
    }
    let (beside, kids) = if file == "fd_handler_cell_flooded.hl" { ("flood-running", 1..=2) } else { ("a-after", 1..=3) };
    let replaced = if count(r, "ev replaced") == 1 { "replaced" } else { "unreplaced" };
    let beside = if count(r, &format!("ev {beside}")) == 1 { beside } else { "alone" };
    let read = if r.stdout.lines().any(|l| l.starts_with("ev handler-torn")) { "read-torn" } else { "read-whole" };
    let once = if kids.clone().all(|t| count(r, &format!("ev kid-dissolve {t} kid-{t}-name")) == 1) {
        "dissolved-once-each"
    } else {
        "dissolved-other"
    };
    format!("{replaced} {beside} {read} {once} {}", finished_word(r))
}

/// `fd_pool_owner_worker.hl`: whether the posted failure was raised off
/// the worker, the thread each handler ran on, how many ran.
fn pool_owner_worker(r: &Ran) -> String {
    if let Some(w) = exit_word(r) {
        return w;
    }
    let raised = if count(r, "ev raised-off-worker") == 1 { "raised-off-worker" } else { "raised-on-worker" };
    let on = |what: &str| -> String {
        match (count(r, &format!("ev {what}-on-worker")), count(r, &format!("ev {what}-off-worker"))) {
            (1, 0) => format!("{what}-on-worker"),
            (0, 1) => format!("{what}-off-worker"),
            _ => format!("{what}-unheard"),
        }
    };
    let heard = r.stdout.lines().find_map(|l| l.strip_prefix("ev heard ")).unwrap_or("?");
    format!("{raised} {} {} heard-{heard}", on("held"), on("posted"))
}

fn judge(file: &str, r: &Ran) -> String {
    match file {
        "fd_handler_cell_no_hold.hl" => handler_cell(r),
        "fd_handler_cell_restart_replaced.hl" => handler_cell_restart(r),
        "fd_handler_cell_blocked_on_owner.hl" => handler_cell_blocked(r),
        f @ ("fd_handler_cell_flooded.hl" | "fd_handler_cell_interleaved.hl") => handler_cell_carried(f, r),
        f if f.starts_with("fd_restart") => reclaim_wins(f, r),
        "fd_pinned_owner_state.hl" => owner_state(r),
        "fd_reclaim_under_delivery.hl" => reclaim_under_delivery(r),
        "fd_sibling_replace_heap.hl" => sibling_heap(r),
        "fd_sibling_replace_three.hl" => sibling_three(r),
        f if f.starts_with("fd_sibling_replace") || f == "fd_settle_no_nest.hl" => output(r),
        "fd_pool_owner_worker.hl" => pool_owner_worker(r),
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

/// The domains the failure deliveries of `decls` ran on, entry and end
/// alike, each with its declaration.
fn delivery_domains(r: &Ran, decls: &[&str]) -> Vec<(String, String)> {
    r.trace
        .events
        .iter()
        .filter(|e| e.kind == ObligationKind::FailureDelivery && e.decl.as_deref().is_some_and(|d| decls.contains(&d)))
        .filter(|e| matches!(e.point, Point::Entered | Point::Completed))
        .map(|e| (e.decl.clone().unwrap_or_default(), e.domain.clone()))
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
    let _ = assert_traced_in(file, false);
}

fn assert_traced_in(file: &str, no_bus_devirt: bool) -> Ran {
    let bin = build(file, false, no_bus_devirt);
    let ran = run_bin(&bin, &[]);
    let _ = std::fs::remove_file(&bin);
    let got = judge(file, &ran);
    let (_, on, decls) = DELIVERED_ON.iter().find(|(f, ..)| *f == file).unwrap_or_else(|| panic!("{file}: no domain"));
    let domains = delivery_domains(&ran, decls);
    eprintln!("{file}, no_bus_devirt={no_bus_devirt}: {got}; FailureDelivery on {domains:?}\n{}", report(&ran));
    match known_open(file) {
        Some((row, today)) => assert_eq!(
            got, today,
            "{file} (KNOWN_OPEN at {row}) no longer gives today's word; adopted is `{}`, so the entry has to go\n{}",
            adopted(file),
            report(&ran)
        ),
        None => {
            assert_eq!(got, adopted(file), "{file}, no_bus_devirt={no_bus_devirt}\n{}", report(&ran));
            for decl in *decls {
                assert!(domains.iter().any(|(d, _)| d == decl), "{file}: no delivery of {decl} traced: {domains:?}");
            }
            assert!(domains.iter().all(|(_, d)| d.starts_with(on)), "{file}: the delivery ran on {domains:?}, not {on}");
        }
    }
    ran
}

/// Under AddressSanitizer, with chunk pooling off (GH #816), in both
/// dispatch modes: the adopted word and nothing reported.
fn assert_asan(file: &str) {
    for no_bus_devirt in [false, true] {
        let bin = build(file, true, no_bus_devirt);
        let ran = run_bin(&bin, &[("ASAN_OPTIONS", "detect_leaks=1"), ("LOTUS_NO_CHUNK_POOL", "1")]);
        let _ = std::fs::remove_file(&bin);
        let hits = sanitizer_hits(&ran);
        assert!(hits.is_empty(), "{file}, no_bus_devirt={no_bus_devirt}: {hits:?}\n{}", report(&ran));
        assert_eq!(judge(file, &ran), adopted(file), "{file} under ASan, no_bus_devirt={no_bus_devirt}\n{}", report(&ran));
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

/// Decision line 1's pool-placed owner, and L0-1 for it: the held
/// delivery at its settle and a later one posted from main both run on
/// the pool's worker.
#[test]
fn a_pool_placed_owners_handler_runs_on_its_worker() {
    assert_traced("fd_pool_owner_worker.hl");
}

/// The three domain fixtures under AddressSanitizer, with chunk pooling
/// off (GH #816), in both dispatch modes: the adopted word and nothing
/// reported (the other fixtures' own tests run theirs, and the known-open
/// one is asserted to fail). Before the protocol, the reclaim fixture's
/// handler read the old child's freed arena (a heap-use-after-free).
#[test]
fn the_domain_fixtures_hold_under_asan_in_both_dispatch_modes() {
    assert_asan("fd_pinned_owner_state.hl");
    assert_asan("fd_reclaim_under_delivery.hl");
    assert_asan("fd_pool_owner_worker.hl");
}

/// The review's reproducer: the owner's handler for `a` replaces `b`
/// while `b`'s failure is posted to the owner. Before the rule this
/// printed `handling 0 / replacing sibling` and hung (the word was
/// `timeout`); its control, without the replacement, never did.
#[test]
fn a_handler_replacing_a_failing_sibling_never_waits_on_itself() {
    for no_bus_devirt in [false, true] {
        assert_traced_in("fd_sibling_replace.hl", no_bus_devirt);
        assert_traced_in("fd_sibling_replace_control.hl", no_bus_devirt);
    }
}

/// The replaced sibling's late handler reads its heap String through
/// `c`: whole, before the old child dissolves, under ASan as well.
#[test]
fn a_replaced_siblings_late_handler_reads_it_whole() {
    for no_bus_devirt in [false, true] {
        assert_traced_in("fd_sibling_replace_heap.hl", no_bus_devirt);
    }
    assert_asan("fd_sibling_replace_heap.hl");
}

/// Two siblings replaced from one handler: both deliveries run after it,
/// in posting order, and each old child is reclaimed once, after its own.
#[test]
fn two_replaced_siblings_are_each_delivered_then_reclaimed_once() {
    for no_bus_devirt in [false, true] {
        assert_traced_in("fd_sibling_replace_three.hl", no_bus_devirt);
    }
}

/// The replacement reached through a method the handler calls. (An owner
/// placed on a pool has no such case: only `main locus` places, and a
/// nested instance runs on its owner's domain, so a pool-placed owner's
/// children fail on its worker and their deliveries are in place, never
/// posted to it.)
#[test]
fn a_sibling_replaced_through_a_method_is_deferred_too() {
    for no_bus_devirt in [false, true] {
        assert_traced_in("fd_sibling_replace_method.hl", no_bus_devirt);
    }
}

/// The reclaim wins (PR #1348's review): the owner's handler replaces the
/// child its delivery is about and then asks for its restart, plainly and
/// in place. Before the rule `restart (c)` re-ran the replaced child's
/// birth() and run() beside its dissolve, and `restart_in_place (c)` hung
/// (the word was `timeout`).
#[test]
fn a_restart_of_the_child_its_handler_replaced_is_not_performed() {
    for no_bus_devirt in [false, true] {
        assert_traced_in("fd_restart_replaced.hl", no_bus_devirt);
        assert_traced_in("fd_restart_in_place_replaced.hl", no_bus_devirt);
    }
}

/// The sibling case: the old `b` was replaced from `a`'s handler while its
/// failure was posted, and its own handler asks for its restart.
#[test]
fn a_restart_of_a_replaced_sibling_is_not_performed() {
    for no_bus_devirt in [false, true] {
        assert_traced_in("fd_restart_sibling_replaced.hl", no_bus_devirt);
    }
}

/// Handlers do not nest at settle (PR #1348's review): a posted delivery
/// waits for the held handler running at settle to return. Before the
/// guard the posted handler's entry and exit lines came inside the held
/// handler's.
#[test]
fn a_held_handler_at_settle_does_not_nest_a_posted_one() {
    for no_bus_devirt in [false, true] {
        assert_traced_in("fd_settle_no_nest.hl", no_bus_devirt);
    }
}

/// Inventory row R52: a bus handler's cell holds its subscriber. Replacing
/// a pool-placed child while its handler runs leaves the child's storage
/// to that handler until it returns, under ASan in both dispatch modes.
/// Before the hold the handler read freed storage (`heap-use-after-free`).
#[test]
fn a_bus_handlers_cell_holds_its_subscriber() {
    assert_asan("fd_handler_cell_no_hold.hl");
}

/// The negative control: the same fixture under ASan, with the trace
/// build's `HandlerHold` skip, which takes no hold, reads freed storage
/// again in both dispatch modes.
#[test]
fn without_the_handler_hold_the_subscriber_is_freed_under_its_handler() {
    let file = "fd_handler_cell_no_hold.hl";
    for no_bus_devirt in [false, true] {
        let bin = build_with(file, true, true, no_bus_devirt);
        let ran = run_bin(
            &bin,
            &[("ASAN_OPTIONS", "detect_leaks=1"), ("LOTUS_NO_CHUNK_POOL", "1"), ("LOTUS_LIFECYCLE_SKIP", "HandlerHold")],
        );
        let _ = std::fs::remove_file(&bin);
        assert_eq!(judge(file, &ran), "heap-use-after-free", "{file} without the hold, no_bus_devirt={no_bus_devirt}\n{}", report(&ran));
    }
}
/// The owner's failure handler replaces, and asks to restart, the child
/// whose bus handler is waiting on main for that very delivery: the
/// reclaim is deferred behind the delivery, then waits for the bus
/// handler's hold, and the restart is not performed. No wait closes on
/// itself (each run is under [`DEADLINE`]).
#[test]
fn a_handler_replacing_the_child_whose_handler_waits_on_it_does_not_deadlock() {
    for no_bus_devirt in [false, true] {
        assert_traced_in("fd_handler_cell_restart_replaced.hl", no_bus_devirt);
    }
    assert_asan("fd_handler_cell_restart_replaced.hl");
}

/// The reclaim waits for a handler that is waiting for the reclaiming
/// thread: the wait runs the failure posted to main, so the handler
/// returns and the reclaim completes.
#[test]
fn a_reclaim_waiting_for_a_handler_runs_what_the_handler_waits_for() {
    for no_bus_devirt in [false, true] {
        assert_traced_in("fd_handler_cell_blocked_on_owner.hl", no_bus_devirt);
    }
    assert_asan("fd_handler_cell_blocked_on_owner.hl");
}

/// A worker that never finds its queue empty carries its handler's hold
/// from cell to cell for the one subscriber it serves; replacing that
/// subscriber mid-flood ends the carry after the handler in progress (the
/// reclaim's `wanted`, or the dead-self check of the next cell), so the
/// reclaim completes while the flood still runs, and the program ends.
#[test]
fn a_reclaim_of_a_flooded_subscriber_ends_its_workers_carried_hold() {
    for no_bus_devirt in [false, true] {
        let ran = assert_traced_in("fd_handler_cell_flooded.hl", no_bus_devirt);
        assert_a_handler_per_cell(&ran, 100);
    }
    assert_asan("fd_handler_cell_flooded.hl");
}

/// A carried hold covers many handlers, and the trace still names each
/// (R52): every subject's `Handler` entries each have an end, one
/// subscriber ran at least `at_least` of them, and the trace's laws hold.
fn assert_a_handler_per_cell(r: &Ran, at_least: usize) {
    let mut per_subject: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    for e in r.trace.events.iter().filter(|e| e.kind == ObligationKind::Handler) {
        let n = per_subject.entry(format!("{:?}", e.subject)).or_default();
        if e.point == Point::Entered {
            n.0 += 1;
        } else if e.point.satisfies(Point::Ended) {
            n.1 += 1;
        }
    }
    assert!(per_subject.values().all(|(entered, ended)| entered == ended), "a handler without its end: {per_subject:?}");
    assert!(per_subject.values().any(|(entered, _)| *entered >= at_least), "fewer handlers traced than ran: {per_subject:?}");
    let laws = trace::laws(&r.trace, true);
    assert!(laws.is_empty(), "the trace's laws: {laws:?}");
}

/// Two subscribers on one worker, their cells interleaved: each cell ends
/// the other subscriber's carried hold, and the one replaced mid-stream is
/// reclaimed while the other's cells go on.
#[test]
fn a_carried_hold_ends_at_another_subscribers_cell() {
    for no_bus_devirt in [false, true] {
        let ran = assert_traced_in("fd_handler_cell_interleaved.hl", no_bus_devirt);
        assert_a_handler_per_cell(&ran, 50);
    }
    assert_asan("fd_handler_cell_interleaved.hl");
}

/// The owner's own reclaim reached the child first: App's run() replaces
/// `a` while `a`'s failure is posted to main, the reclaim runs the
/// delivery while it waits for it, and the handler asks for a restart.
/// The claim is owed before the handler runs, so the failing child, which
/// reads the decision once the handler returns, never sees it clear.
#[test]
fn a_restart_of_a_child_its_owner_is_reclaiming_is_not_performed() {
    for no_bus_devirt in [false, true] {
        assert_traced_in("fd_restart_replaced_by_owner.hl", no_bus_devirt);
    }
}

/// The refused restarts under AddressSanitizer, chunk pooling off: the
/// handlers read the old child's heap name, and its dissolve prints it.
#[test]
fn a_refused_restart_holds_under_asan() {
    assert_asan("fd_restart_replaced.hl");
    assert_asan("fd_restart_in_place_replaced.hl");
    assert_asan("fd_restart_sibling_replaced.hl");
    assert_asan("fd_restart_replaced_by_owner.hl");
}
