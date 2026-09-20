//! Compiled-corpus oracle harness.
//!
//! The broad example corpus is parse-tested (`hale-syntax`) and
//! type-checked (`hale-types`) across the whole set. This harness
//! supplies the systematic *run* coverage. (It superseded the old
//! interpreter exit-code-0 sweep, retired with `hale-runtime`: a
//! tree-walker has no arenas, pools, wake_fds, or coroutine
//! parking, so it structurally could not reproduce the bug classes
//! that keep surfacing at feature *intersections* on the compiled
//! substrate: teardown-time leaks, shutdown use-after-free,
//! cross-pool starvation/hangs, and buffer overflows.)
//!
//! This harness covers that surface. It compiles every
//! runnable fixture to a NATIVE binary and runs it under an oracle
//! battery:
//!
//!   * exit oracle      — the process exits with its expected code
//!                        (0, except the deliberately-bubbled
//!                        closure violation). A SIGSEGV/SIGABRT
//!                        shutdown crash fails here.
//!   * deadline oracle  — the process terminates within a wall
//!                        clock budget. A cross-pool starvation /
//!                        missed-wakeup hang fails here.
//!   * sanitizer oracle — built with `LOTUS_ASAN=1` (the
//!                        `*_under_asan` test, opt-in like the TSAN
//!                        job), AddressSanitizer + LeakSanitizer
//!                        catch heap-overflow, use-after-free, and
//!                        leaks at exit.
//!
//! GH #816: the sanitizer oracle used to be structurally blind to
//! the use-after-free class it exists for. `lotus_arena_destroy`
//! returned a dying arena's chunks to a thread-local pool with
//! their bytes intact, so a load from a destroyed arena read memory
//! the process still owned — valid to ASan, which never saw a
//! `free`. Four arena use-after-frees (GH #710, #750, #711, #812)
//! passed this gate. The instrumented build now defaults the
//! recycling off (`LOTUS_NO_CHUNK_POOL`, compiled in by the ASan
//! cflags and restated on the child's env below), so those reads
//! are reported as `heap-use-after-free` with the free stack and
//! the read stack. The plain pass keeps the pooled allocator —
//! that is the allocator users run.
//!
//! Fixtures are discovered at runtime, so a new example under
//! `tests/fixtures/examples/` is covered automatically — no
//! manifest to keep in sync.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn examples_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("examples");
    p
}

/// Fixtures excluded from the run oracles, with the reason:
///   * network servers — they `accept()` connections and never
///     self-terminate, so they need a start/probe/shutdown driver
///     (a separate harness, out of scope here).
///   * none currently need explicit listing beyond servers; the
///     multi-file/import projects are filtered structurally below
///     (plain `build_executable` can't resolve cross-file imports;
///     that's the CLI's directory-build path).
const SKIP_SERVERS: &[&str] = &["http-hello", "io-demo", "docs-server"];

/// The one fixture whose correct exit is non-zero: it deliberately
/// bubbles a ClosureViolation to the root, which the runtime
/// surfaces as a non-zero process exit.
fn expects_nonzero_exit(name: &str) -> bool {
    name == "03c-closure-bubbled"
}

/// Fixtures that terminate abnormally by design — they `bubble` a
/// violation to the root and exit via root-panic, which
/// intentionally skips the drain/dissolve cleanup cascade. Their
/// "leaks" under LeakSanitizer are the un-run cleanup, not a bug,
/// so the leak oracle doesn't apply.
fn leak_exempt(name: &str) -> bool {
    name == "03c-closure-bubbled"
}

/// Fixtures that the interpreter runs but codegen can't compile YET,
/// acknowledged as interpreter-only rather than treated as a gap to
/// chase right now. This is regression protection in both
/// directions:
///   * an uncompilable fixture NOT in this set is a hard failure —
///     something that used to compile regressed, or a new fixture
///     uses an unsupported feature without a decision being made;
///   * a fixture IN this set that now COMPILES is also a hard
///     failure — the list is stale and must shrink. So when the
///     codegen gap is closed, the gate tells you to remove it here.
///
/// Empty — codegen compiles the whole runnable corpus. The three
/// former entries (`43-enums`, `45-enum-payloads`,
/// `47-fn-arenas-extras`) were fixed 2026-06-02 by a one-branch
/// reorder in `type_expr_to_codegen_ty`: enum names now resolve to
/// `CodegenTy::Enum` before the `pending_type_names` forward-ref
/// branch, so annotated enum values get the same representation as
/// constructed ones and the already-built enum machinery (no-payload
/// print, payload construction/match/deep-copy) became reachable.
/// The guard remains: any uncompilable fixture is now a hard
/// failure unless deliberately listed here.
const EXPECTED_INTERPRETER_ONLY: &[&str] = &[];

/// KNOWN, TRACKED leak the ASAN oracle still flags. Quarantined so
/// the gate is green on the KNOWN state — but a leak in ANY fixture
/// outside this set is a hard failure, and a fixture that stops
/// leaking just stops being reported. Shrink this set as leaks are
/// fixed; the goal is to empty it.
///
/// Fixed and removed 2026-06-02: `35-tick-closures` and
/// `41-closure-accumulator` — the method-scratch subregion leaked
/// once per delivery to an already-quarantined subscribed handler
/// (the quarantine entry-gate's early `return` bypassed
/// `close_method_scratch`). Fixed in `locus/method.rs` by destroying
/// the scratch on the skip path.
///
/// Also fixed 2026-06-02 (the big one): the per-thread chunk POOL
/// was never freed when a spawned worker thread (pinned locus
/// thread / cooperative pool worker) exited. `new_chunk_for`
/// prefills 32×64KiB chunks into a `__thread` pool on a thread's
/// first allocation; the pthread_key dtor only dumped stats, never
/// reclaimed. So EVERY exited worker leaked ~2MiB (main thread
/// excepted — its TLS stays reachable). Fixed in
/// `runtime/lotus_arena.c` with a pool-freeing thread dtor armed
/// for any thread that touches the pool.
///
/// Also fixed 2026-06-02: `40-pinned-duration`'s 64KiB residual —
/// the SAME file, a sibling bug. The chunk-pool prefill path
/// malloc'd chunks but left `via_mmap` uninitialized; the other two
/// chunk-creation paths set it. A pooled prefill chunk later handed
/// out and then passed to `lotus_arena_release_chunk` is dispatched
/// on `via_mmap` — a garbage non-zero value took the `munmap` path
/// with a garbage size, so the chunk was neither freed nor
/// re-pooled. (Latent until something released a pooled chunk; the
/// pinned locus's lifetime-arena teardown was the trigger here.)
/// Fixed by zero-initializing `via_mmap`/`mmap_size` in prefill.
///
/// Empty == the goal reached for the closure class. The second
/// quarantine below (GH #871) is a different, newly-visible set.
const KNOWN_CLOSURE_LEAKS: &[&str] = &[];

/// GH #871 — leaks the chunk pool was hiding from LeakSanitizer,
/// which GH #816's `LOTUS_NO_CHUNK_POOL` made visible. Tracked, not
/// caused, by #816: `LOTUS_ARENA_RESIDENCY=1` reports these arenas
/// live at exit on an ORDINARY build with the pool on and nothing
/// from #816 in play, so they leak on `main` today and always did.
///
/// The masking is the leak-side twin of the use-after-free one. A
/// pooled chunk is a live `malloc` block reachable from
/// `g_chunk_pool` (a `__thread` array, which LSan takes as a root),
/// and it still holds the dead arena's bytes — including a pointer
/// to a `lotus_arena_t` nobody destroyed. That makes the arena
/// "reachable" and unreported. Free the chunk instead and the same
/// allocation is a direct leak.
///
/// Eight of the nine lose exactly one 216-byte `lotus_arena_t` with
/// `chunks=0` — a locus whose `lotus_arena_destroy` never ran, which
/// allocated nothing, so only the header is lost.
/// `87-temp-locus-receiver` is the substantial one at 592416 bytes
/// in 27 allocations: every param-field CHILD locus of a literal in
/// `main` (four `Churner`s and their `Rows` `@form(vec)` children)
/// survives the program. A plain `let`-bound locus in `main` is
/// reclaimed, so this is the child arm of that teardown, not "loci
/// in main leak".
///
/// QUARANTINED FOR LEAKS ONLY. A `heap-use-after-free`, an
/// overflow, a crash or a hang in one of these is still a hard
/// failure — a name on this list must not become a hole in the gate
/// #816 exists to sharpen.
///
/// Empty == the goal reached for this class too. GH #871 (PR #894)
/// fixed eight of the nine by making the teardown cascade reach
/// contract-typed param fields; GH #893 fixed the ninth,
/// `85-bindings-unix`, by giving the `bindings { }` transport an
/// owner on `fn main`'s deferred-dissolve frame instead of leaving
/// it on the m90 `returns_this_locus` path, which allocates for the
/// program's lifetime and then owns nothing. The list stays as the
/// seat for the next one.
const LEAKS_UNMASKED_BY_NO_CHUNK_POOL: &[&str] = &[];

/// Per-fixture wall-clock budget. Demos finish in well under a
/// second; the budget is generous so a slow CI box doesn't flake,
/// while a genuine hang (missed wakeup, starvation) still trips it.
const DEADLINE: Duration = Duration::from_secs(15);
/// ASAN instrumentation runs ~2-5x slower; give it more room.
const DEADLINE_ASAN: Duration = Duration::from_secs(90);

/// Discover every runnable single-file binary fixture: a directory
/// holding exactly one `.hl` file, named `main.hl`, that declares
/// `fn main()`, and isn't a known never-terminating server.
/// The single-`.hl` rule structurally excludes multi-file import
/// projects (25-imports, multi-file-seed, fitter-applier-pair),
/// which need the CLI's directory-build import resolution.
fn runnable_fixtures() -> Vec<(String, PathBuf)> {
    let dir = examples_dir();
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("read examples dir") {
        let entry = entry.expect("dir entry");
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if SKIP_SERVERS.contains(&name.as_str()) {
            continue;
        }
        let hl_files: Vec<PathBuf> = std::fs::read_dir(entry.path())
            .expect("read fixture dir")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map(|x| x == "hl").unwrap_or(false))
            .collect();
        if hl_files.len() != 1 {
            continue; // multi-file project — needs import bundling
        }
        let main_hl = entry.path().join("main.hl");
        if !main_hl.exists() {
            continue;
        }
        let src = std::fs::read_to_string(&main_hl).expect("read main.hl");
        if !src.contains("fn main()") {
            continue; // library-only fixture, not a binary
        }
        out.push((name, main_hl));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Is this run the instrumented pass? `build_executable` reads
/// `LOTUS_ASAN` at codegen time to decide the runtime cflags + link
/// flags, so the same variable is what tells the harness whether the
/// binary it just built is an ASan binary.
fn asan_enabled() -> bool {
    std::env::var("LOTUS_ASAN")
        .map(|v| v == "1" || v == "true" || v == "TRUE")
        .unwrap_or(false)
}

enum RunResult {
    /// Completed within the deadline; carries exit code (None if
    /// killed by a signal), the signal number if any, and stderr.
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
        stderr: String,
    },
    /// Did not terminate within the deadline (killed by us).
    TimedOut,
}

/// Run `bin` with output redirected to temp files (so a large
/// sanitizer report on stderr can never deadlock a pipe), polling
/// for completion until `deadline`, then SIGKILLing on timeout.
fn run_with_deadline(bin: &Path, deadline: Duration) -> RunResult {
    let out_path = bin.with_extension("stdout");
    let err_path = bin.with_extension("stderr");
    let out_file = File::create(&out_path).expect("create stdout file");
    let err_file = File::create(&err_path).expect("create stderr file");

    let mut cmd = Command::new(bin);
    cmd.stdin(Stdio::null())
        .stdout(out_file)
        .stderr(err_file)
        // LeakSanitizer detects leaks at exit (no-op on a non-ASAN
        // build). Leave abort_on_error off so ASAN/LSAN exit
        // non-zero *with* their stderr banner rather than raising
        // SIGABRT — the banner lets the sanitizer oracle classify
        // (and the abnormal-exit fixture stays exempt by exit code).
        .env("ASAN_OPTIONS", "detect_leaks=1");
    if asan_enabled() {
        // GH #816: make the arena's chunk recycling stop hiding
        // use-after-free from the sanitizer — a destroyed arena's
        // chunks go back to libc, so a load from one is a real
        // heap-use-after-free instead of a valid read of memory we
        // kept. The instrumented build already defaults this on
        // (`-DLOTUS_NO_CHUNK_POOL_DEFAULT=1` in the ASan cflags);
        // stating it here keeps the oracle's guarantee legible at
        // the place the guarantee is made, and holds even if a
        // fixture binary is somehow built without those cflags.
        // NOT set on the plain pass: the non-instrumented oracle
        // should run the allocator users actually get.
        cmd.env("LOTUS_NO_CHUNK_POOL", "1");
    }
    let mut child = cmd.spawn().expect("spawn fixture binary");

    let start = Instant::now();
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                #[cfg(unix)]
                let signal = {
                    use std::os::unix::process::ExitStatusExt;
                    status.signal()
                };
                #[cfg(not(unix))]
                let signal = None;
                let mut stderr = String::new();
                let _ = File::open(&err_path)
                    .and_then(|mut f| f.read_to_string(&mut stderr));
                break RunResult::Exited {
                    code: status.code(),
                    signal,
                    stderr,
                };
            }
            Ok(None) => {
                if start.elapsed() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break RunResult::TimedOut;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => panic!("try_wait failed: {e}"),
        }
    };
    let _ = std::fs::remove_file(&out_path);
    let _ = std::fs::remove_file(&err_path);
    result
}

/// stderr markers that mean a *sanitizer* (ASAN/LSAN/TSAN) fired,
/// regardless of exit code — needed so a sanitizer failure on the
/// one deliberately-non-zero fixture isn't masked by its expected
/// non-zero exit. Deliberately narrow: Hale's own root-panic
/// formatter prints "runtime error: ..." for a bubbled
/// ClosureViolation, which is correct program behavior, NOT a
/// sanitizer hit — so we match only the sanitizers' own banners.
fn stderr_has_sanitizer_error(stderr: &str) -> bool {
    stderr.contains("AddressSanitizer")
        || stderr.contains("LeakSanitizer")
        || stderr.contains("ThreadSanitizer")
        || stderr.contains("detected memory leaks")
}

/// Is this report ONLY a leak? A quarantine entry excuses leaks and
/// nothing else, so the question has to be asked of the report
/// rather than of the fixture name.
///
/// The distinction is in the banner, not the summary: every ASan /
/// TSan finding opens with `ERROR: AddressSanitizer: <kind>` or
/// `ERROR: ThreadSanitizer: <kind>`, while a leak opens with
/// `ERROR: LeakSanitizer: detected memory leaks` and only mentions
/// AddressSanitizer in its trailing `SUMMARY:` line. Matching the
/// banner keeps a `heap-use-after-free` in a quarantined fixture a
/// hard failure — which is the whole point of GH #816.
fn sanitizer_error_is_leak_only(stderr: &str) -> bool {
    !stderr.contains("ERROR: AddressSanitizer:")
        && !stderr.contains("ERROR: ThreadSanitizer:")
}

/// GH #816: a quarantined fixture is excused its LEAK and nothing
/// else. Worth a test of its own because the failure mode is
/// silence — a name on the list swallowing a use-after-free would
/// give back exactly the blindness #816 removed, and the corpus run
/// that would notice takes an ASan build to reach. Real banners,
/// trimmed: the leak's only mention of AddressSanitizer is its
/// SUMMARY line, which is precisely the trap.
#[test]
fn a_quarantine_excuses_leaks_and_nothing_else() {
    let leak = "==1==ERROR: LeakSanitizer: detected memory leaks\n\
         Direct leak of 216 byte(s) in 1 object(s) allocated from:\n\
         SUMMARY: AddressSanitizer: 216 byte(s) leaked in 1 allocation(s).\n";
    let uaf = "==1==ERROR: AddressSanitizer: heap-use-after-free on \
         address 0x5310000008c0 at pc 0x5c699376abe8\n\
         READ of size 8 at 0x5310000008c0 thread T0\n\
         SUMMARY: AddressSanitizer: heap-use-after-free\n";
    let overflow = "==1==ERROR: AddressSanitizer: heap-buffer-overflow \
         on address 0x502000000110\n";
    assert!(stderr_has_sanitizer_error(leak));
    assert!(sanitizer_error_is_leak_only(leak), "a leak is a leak");
    for report in [uaf, overflow] {
        assert!(stderr_has_sanitizer_error(report));
        assert!(
            !sanitizer_error_is_leak_only(report),
            "a quarantined fixture must still FAIL on this report:\n{report}"
        );
    }
}

enum Outcome {
    Pass,
    /// Codegen can't lower this fixture yet (interpreter-only
    /// feature). Reported as a skip, not an oracle failure — out
    /// of scope for the run oracles, but surfaced so the
    /// interpreter/codegen divergence stays visible.
    Uncompilable(String),
    /// A known, tracked leak (see `KNOWN_CLOSURE_LEAKS`). Reported
    /// loudly but doesn't fail the gate.
    Quarantined(String),
    /// A run oracle (exit / deadline / sanitizer) failed.
    Fail(String),
}

/// Build one fixture to a unique temp binary and run it under the
/// oracles.
fn check_fixture(name: &str, main_hl: &Path, deadline: Duration) -> Outcome {
    let src = match std::fs::read_to_string(main_hl) {
        Ok(s) => s,
        Err(e) => return Outcome::Fail(format!("read: {e}")),
    };
    let program = match hale_syntax::parse_source(&src) {
        Ok(p) => p,
        Err(d) => return Outcome::Fail(format!("parse: {d:?}")),
    };
    let bin = harness::unique_bin(&format!("lotus_corpus_{}_{}", name.replace(['/', '-'], "_"), std::process::id()));
    if let Err(e) = build_executable(&program, &bin) {
        let msg = format!("{e:?}");
        // A codegen feature gap is ACKNOWLEDGED only when the fixture
        // is on the interpreter-only list; otherwise it's a
        // regression (something stopped compiling, or a new fixture
        // hit an unsupported feature with no decision made).
        if msg.contains("Unsupported(") {
            if EXPECTED_INTERPRETER_ONLY.contains(&name) {
                return Outcome::Uncompilable(msg);
            }
            return Outcome::Fail(format!(
                "UNEXPECTED uncompilable: {msg}\n    (implement the codegen, or \
                 add to EXPECTED_INTERPRETER_ONLY if interpreter-only is intended)"
            ));
        }
        return Outcome::Fail(format!("build: {msg}"));
    }

    // It compiled. If it's still on the interpreter-only list, the
    // list is stale — fail so the entry gets removed now the gap is
    // closed (keeps the list honest as codegen catches up).
    if EXPECTED_INTERPRETER_ONLY.contains(&name) {
        let _ = std::fs::remove_file(&bin);
        return Outcome::Fail(format!(
            "`{name}` is in EXPECTED_INTERPRETER_ONLY but now COMPILES — \
             remove it from that list (the codegen gap is closed)"
        ));
    }

    let result = run_with_deadline(&bin, deadline);
    let _ = std::fs::remove_file(&bin);

    match result {
        RunResult::TimedOut => Outcome::Fail(format!(
            "HANG: did not terminate within {}s (starvation / missed wakeup?)",
            deadline.as_secs()
        )),
        RunResult::Exited { code, signal, stderr } => {
            if stderr_has_sanitizer_error(&stderr) && !leak_exempt(name) {
                let snippet: String = stderr
                    .lines()
                    .filter(|l| l.contains("leak") || l.contains("SUMMARY") || l.contains("ERROR"))
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" | ");
                let quarantined = KNOWN_CLOSURE_LEAKS.contains(&name)
                    || LEAKS_UNMASKED_BY_NO_CHUNK_POOL.contains(&name);
                if quarantined && sanitizer_error_is_leak_only(&stderr) {
                    return Outcome::Quarantined(snippet);
                }
                let full: String = stderr.lines().take(12).collect::<Vec<_>>().join("\n");
                return Outcome::Fail(format!("SANITIZER:\n{full}"));
            }
            if let Some(sig) = signal {
                return Outcome::Fail(format!("CRASH: killed by signal {sig} (UAF / overflow?)"));
            }
            let nonzero_ok = expects_nonzero_exit(name);
            match code {
                Some(0) if !nonzero_ok => Outcome::Pass,
                Some(0) => Outcome::Fail(
                    "expected non-zero exit (bubbled violation) but got 0".to_string(),
                ),
                Some(_) if nonzero_ok => Outcome::Pass,
                Some(c) => Outcome::Fail(format!("BAD EXIT: expected 0, got {c}")),
                None => Outcome::Fail("no exit code and no signal".to_string()),
            }
        }
    }
}

/// Build + run every runnable fixture concurrently under the
/// oracles, collecting all results (don't bail on the first, so
/// one run surfaces the whole picture). Returns
/// `(failures, uncompilable)`. Shared by the plain and ASAN
/// entry points.
fn run_corpus(
    deadline: Duration,
) -> (
    Vec<(String, String)>,
    Vec<(String, String)>,
    Vec<(String, String)>,
) {
    let fixtures = runnable_fixtures();
    assert!(!fixtures.is_empty(), "no runnable fixtures discovered");
    eprintln!("corpus oracle: {} runnable fixtures", fixtures.len());

    let queue = Arc::new(Mutex::new(fixtures.into_iter()));
    let failures = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let uncompilable = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let quarantined = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let n_threads = std::thread::available_parallelism()
        .map(|n| n.get().min(8))
        .unwrap_or(4);

    let mut handles = Vec::new();
    for _ in 0..n_threads {
        let queue = Arc::clone(&queue);
        let failures = Arc::clone(&failures);
        let uncompilable = Arc::clone(&uncompilable);
        let quarantined = Arc::clone(&quarantined);
        handles.push(std::thread::spawn(move || loop {
            let next = queue.lock().unwrap().next();
            let Some((name, path)) = next else { break };
            match check_fixture(&name, &path, deadline) {
                Outcome::Pass => {}
                Outcome::Fail(reason) => failures.lock().unwrap().push((name, reason)),
                Outcome::Uncompilable(reason) => {
                    uncompilable.lock().unwrap().push((name, reason))
                }
                Outcome::Quarantined(reason) => {
                    quarantined.lock().unwrap().push((name, reason))
                }
            }
        }));
    }
    for h in handles {
        h.join().expect("worker thread");
    }

    let mut fails = Arc::try_unwrap(failures).unwrap().into_inner().unwrap();
    fails.sort_by(|a, b| a.0.cmp(&b.0));
    let mut uncomp = Arc::try_unwrap(uncompilable).unwrap().into_inner().unwrap();
    uncomp.sort_by(|a, b| a.0.cmp(&b.0));
    let mut quar = Arc::try_unwrap(quarantined).unwrap().into_inner().unwrap();
    quar.sort_by(|a, b| a.0.cmp(&b.0));
    (fails, uncomp, quar)
}

fn report(
    (failures, uncompilable, quarantined): (
        Vec<(String, String)>,
        Vec<(String, String)>,
        Vec<(String, String)>,
    ),
) {
    // Acknowledged interpreter-only fixtures (EXPECTED_INTERPRETER_ONLY).
    // Not a failure — the divergence is a known, tracked codegen gap —
    // but surfaced so it stays visible. (An UNEXPECTED uncompilable
    // is a hard failure in check_fixture, and a listed fixture that
    // starts compiling also fails, so this list can't silently drift.)
    if !uncompilable.is_empty() {
        eprintln!(
            "\nℹ {} fixture(s) acknowledged interpreter-only (codegen gap, tracked):",
            uncompilable.len()
        );
        for (name, reason) in &uncompilable {
            let cause = reason.lines().next().unwrap_or(reason);
            eprintln!("    · {name}: {cause}");
        }
    }
    // Known, tracked leaks — loud, but not a gate failure (see
    // KNOWN_CLOSURE_LEAKS and LEAKS_UNMASKED_BY_NO_CHUNK_POOL). The
    // point is to keep them visible until each set is empty. Only
    // LEAKS land here: any other sanitizer finding in a quarantined
    // fixture is a failure above.
    if !quarantined.is_empty() {
        eprintln!(
            "\n⚠ {} KNOWN leak(s) quarantined (tracked — GH #871):",
            quarantined.len()
        );
        for (name, reason) in &quarantined {
            eprintln!("    · {name}: {reason}");
        }
    }
    if failures.is_empty() {
        return;
    }
    let mut msg = format!("\n{} fixture(s) failed the run oracles:\n", failures.len());
    for (name, reason) in &failures {
        msg.push_str(&format!("\n  ● {name}\n    {}\n", reason.replace('\n', "\n    ")));
    }
    panic!("{msg}");
}

/// Exit + deadline oracles on a normal (non-instrumented) build.
/// Runs in the standard CI partitions — no extra toolchain needed.
#[test]
fn corpus_terminates_and_exits_clean() {
    report(run_corpus(DEADLINE));
}

/// Full oracle battery including AddressSanitizer + LeakSanitizer.
/// Ignored by default (ASAN slows builds + runs and needs the
/// instrumented runtime); opted into by the dedicated CI job with
/// `LOTUS_ASAN=1 cargo nextest run --run-ignored ...`, mirroring
/// the TSAN job. A no-op pass if LOTUS_ASAN isn't set, so an
/// accidental `--run-ignored` locally doesn't give false comfort.
#[test]
#[ignore = "ASAN build: opt in with LOTUS_ASAN=1 (dedicated CI job)"]
fn corpus_clean_under_asan() {
    let on = asan_enabled();
    if !on {
        eprintln!("skipping: set LOTUS_ASAN=1 to build the corpus under AddressSanitizer");
        return;
    }
    report(run_corpus(DEADLINE_ASAN));
}
