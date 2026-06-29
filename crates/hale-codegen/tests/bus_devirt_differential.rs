//! Static-bus-dispatch devirtualization — DIFFERENTIAL gate (build #1b).
//!
//! The static devirt lowering (`lotus_bus_dispatch_static` +
//! `lotus_bus_register_static`, default ON) MUST be behaviorally
//! identical to the dynamic lowering (`lotus_bus_dispatch` scan,
//! forced by `LOTUS_NO_BUS_DEVIRT=1`). This harness is the hard merge
//! gate: every bus-using corpus fixture (plus a few targeted shapes —
//! single-subscriber, multi-instance, cross-pool, pinned) is compiled
//! BOTH ways and run; the two runs must produce IDENTICAL stdout AND
//! the identical exit status. Any divergence is a devirt bug.
//!
//! The static path enqueues to the same targets in the same order as
//! the dynamic path (it reads a per-subject bucket of indices into the
//! SAME `g_bus_entries` rows), so deferred-FIFO ordering is preserved
//! — this harness proves it end to end.
//!
//! Single-threaded by construction: the dynamic arm toggles the
//! `LOTUS_NO_BUS_DEVIRT` process env around an in-process
//! `build_executable`, so the whole harness lives in ONE test fn (run
//! with `--test-threads=1`, the repo default).

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hale_codegen::build_executable;

const DEADLINE: Duration = Duration::from_secs(20);

/// Servers never self-terminate — out of scope (matches corpus_oracle).
const SKIP_SERVERS: &[&str] = &["http-hello", "io-demo", "docs-server"];

fn examples_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("examples");
    p
}

/// A fixture exercises the bus iff its source declares a `bus {`
/// block, a `subscribe`, or a `<-` publish — the surfaces that route
/// through register/dispatch.
fn uses_bus(src: &str) -> bool {
    src.contains("bus {") || src.contains("subscribe ") || src.contains("<-")
}

/// Discover runnable single-file bus fixtures (mirrors
/// corpus_oracle::runnable_fixtures, narrowed to bus users).
fn bus_fixtures() -> Vec<(String, PathBuf)> {
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
            continue; // multi-file import project — needs the CLI path
        }
        let main_hl = entry.path().join("main.hl");
        if !main_hl.exists() {
            continue;
        }
        let src = std::fs::read_to_string(&main_hl).expect("read main.hl");
        if !src.contains("fn main()") || !uses_bus(&src) {
            continue;
        }
        out.push((name, main_hl));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

struct RunOutcome {
    code: Option<i32>,
    signal: Option<i32>,
    stdout: String,
}

/// Run `bin` (cwd = `cwd`, so a fixture's deployment.yaml is found)
/// with a deadline; capture stdout. Returns None on timeout.
fn run(bin: &Path, cwd: &Path) -> Option<RunOutcome> {
    let out_path = bin.with_extension("out");
    let out_file = File::create(&out_path).expect("create stdout file");
    let mut child = Command::new(bin)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(out_file)
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn fixture binary");
    let start = Instant::now();
    let res = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                #[cfg(unix)]
                let signal = {
                    use std::os::unix::process::ExitStatusExt;
                    status.signal()
                };
                #[cfg(not(unix))]
                let signal = None;
                let mut stdout = String::new();
                let _ = File::open(&out_path)
                    .and_then(|mut f| f.read_to_string(&mut stdout));
                break Some(RunOutcome { code: status.code(), signal, stdout });
            }
            Ok(None) => {
                if start.elapsed() > DEADLINE {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("try_wait: {e}"),
        }
    };
    let _ = std::fs::remove_file(&out_path);
    res
}

/// Build `src` to a unique temp binary; `devirt` selects the static
/// (default) vs dynamic (LOTUS_NO_BUS_DEVIRT=1) lowering. Returns the
/// binary path; the caller deletes it. `None` ⇒ build failed (the
/// caller records it — a build failure on either arm is a harness
/// failure, since both arms must compile identically).
fn build(src: &str, tag: &str, devirt: bool) -> Option<PathBuf> {
    let program = hale_syntax::parse_source(src).ok()?;
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "lotus_devirt_{}_{}_{}",
        tag.replace(['/', '-', '.'], "_"),
        if devirt { "stat" } else { "dyn" },
        std::process::id()
    ));
    // SAFETY: the whole harness is one test fn on one thread; no other
    // test in this binary mutates the environment concurrently.
    if devirt {
        std::env::remove_var("LOTUS_NO_BUS_DEVIRT");
    } else {
        std::env::set_var("LOTUS_NO_BUS_DEVIRT", "1");
    }
    let ok = build_executable(&program, &bin).is_ok();
    std::env::remove_var("LOTUS_NO_BUS_DEVIRT");
    if ok {
        Some(bin)
    } else {
        None
    }
}

/// Compile `src` both ways, run both, and assert identical stdout +
/// exit. `cwd` is where the binary runs (fixture dir for yaml, or a
/// temp dir for inline programs). Returns Err(reason) on divergence.
fn assert_identical(tag: &str, src: &str, cwd: &Path) -> Result<(), String> {
    let stat_bin = build(src, tag, true)
        .ok_or_else(|| format!("{tag}: static build failed"))?;
    let dyn_bin = build(src, tag, false)
        .ok_or_else(|| format!("{tag}: dynamic build failed"))?;

    let stat = run(&stat_bin, cwd);
    let dynr = run(&dyn_bin, cwd);
    let _ = std::fs::remove_file(&stat_bin);
    let _ = std::fs::remove_file(&dyn_bin);

    match (stat, dynr) {
        (Some(s), Some(d)) => {
            if s.stdout != d.stdout {
                return Err(format!(
                    "{tag}: STDOUT DIVERGED\n--- static ---\n{}\n--- dynamic ---\n{}",
                    s.stdout, d.stdout
                ));
            }
            if s.code != d.code || s.signal != d.signal {
                return Err(format!(
                    "{tag}: EXIT DIVERGED static=(code={:?},sig={:?}) dynamic=(code={:?},sig={:?})",
                    s.code, s.signal, d.code, d.signal
                ));
            }
            Ok(())
        }
        (None, _) | (_, None) => {
            Err(format!("{tag}: a run TIMED OUT (static-vs-dynamic deadlock?)"))
        }
    }
}

/// Targeted shapes beyond the corpus — exercise the eligibility +
/// placement axes explicitly. Each must be differentially identical.
fn targeted_programs() -> Vec<(&'static str, &'static str)> {
    vec![
        // Single subscriber, single-Int flat payload, pool=main → the
        // archetypal eligible singleton fast path.
        (
            "targeted_single_subscriber",
            r#"
type Tick { n: Int; }
locus Counter {
    params { total: Int = 0; }
    bus { subscribe "ticks" as on_tick of type Tick; }
    fn on_tick(t: Tick) {
        self.total = self.total + t.n;
        println("total=", self.total);
    }
}
locus Producer {
    bus { publish "ticks" of type Tick; }
    run() {
        Counter { };
        "ticks" <- Tick { n: 1 };
        "ticks" <- Tick { n: 2 };
        "ticks" <- Tick { n: 3 };
    }
}
fn main() { Producer { }; }
"#,
        ),
        // Two subscriber INSTANCES of the same locus on one subject →
        // multi-entry bucket; deferred-FIFO across both must match.
        (
            "targeted_multi_instance",
            r#"
type Ev { v: Int; }
locus Sink {
    params { tag: Int = 0; }
    bus { subscribe "ev" as on_ev of type Ev; }
    fn on_ev(e: Ev) { println("sink ", self.tag, " got ", e.v); }
}
locus Producer {
    bus { publish "ev" of type Ev; }
    run() {
        Sink { tag: 1 };
        Sink { tag: 2 };
        "ev" <- Ev { v: 10 };
        "ev" <- Ev { v: 20 };
    }
}
fn main() { Producer { }; }
"#,
        ),
        // Managed (non-flat) String payload → exercises the wire /
        // per-subscriber-arena static path, not the flat verbatim one.
        (
            "targeted_managed_payload",
            r#"
type Msg { text: String; }
locus Printer {
    bus { subscribe "msgs" as on_msg of type Msg; }
    fn on_msg(m: Msg) { println("msg: ", m.text); }
}
locus Producer {
    bus { publish "msgs" of type Msg; }
    run() {
        Printer { };
        "msgs" <- Msg { text: "hello" };
        "msgs" <- Msg { text: "world" };
    }
}
fn main() { Producer { }; }
"#,
        ),
        // Cross-pool subscriber (cooperative(pool = io)) → the
        // subscriber's handler runs on the pool worker thread; the
        // static path must still route through coop_pool_post (not a
        // same-thread call), and #3 no_pinned must NOT engage (an
        // off-thread placement exists). Differentially identical proves
        // the cross-thread routing matches.
        (
            "targeted_cross_pool",
            r#"
type Tick { n: Int; }
locus Sub {
    bus { subscribe "tick" as on_tick of type Tick; }
    fn on_tick(t: Tick) { println("tick ", t.n); }
    run() { println("sub run"); }
}
main locus App {
    params { s: Sub = Sub { }; }
    placement { s: cooperative(pool = io); }
    bus { publish "tick" of type Tick; }
    run() {
        std::time::sleep(20ms);
        "tick" <- Tick { n: 1 };
        "tick" <- Tick { n: 2 };
        std::time::sleep(50ms);
        println("main done");
    }
}
fn main() { App { }; }
"#,
        ),
        // Inline adapter `bindings { }` (→ a transport recv-thread,
        // makes the bus multithreaded) ALONGSIDE a plain local eligible
        // subject "local". The eligible subject must dispatch with
        // no_pinned=0 (see bus_devirt_no_pinned.rs for the IR-level
        // assertion); here we additionally prove behavioral identity.
        (
            "targeted_adapter_plus_eligible",
            r#"
type Ext { n: Int; }
type Local { v: Int; }
topic ExtTopic { payload: Ext; subject: "ext"; }
locus Adapter { fn send(subject: String, bytes: Bytes) { } }
locus Sink {
    bus { subscribe "local" as on_local of type Local; }
    fn on_local(l: Local) { println("local ", l.v); }
}
main locus App {
    bindings { ExtTopic: Adapter { }; }
    bus { publish "local" of type Local; }
    run() {
        Sink { };
        "local" <- Local { v: 1 };
        "local" <- Local { v: 2 };
    }
}
fn main() { App { }; }
"#,
        ),
    ]
}

#[test]
fn static_dispatch_is_differentially_identical() {
    let mut failures: Vec<String> = Vec::new();
    let mut checked = 0usize;

    // 1) Targeted shapes (inline, run from a temp cwd).
    let tmp = std::env::temp_dir();
    for (tag, src) in targeted_programs() {
        checked += 1;
        if let Err(e) = assert_identical(tag, src, &tmp) {
            failures.push(e);
        } else {
            eprintln!("ok (targeted): {tag}");
        }
    }

    // 2) Every bus-using corpus fixture, run from its own dir.
    for (name, main_hl) in bus_fixtures() {
        let src = std::fs::read_to_string(&main_hl).expect("read fixture");
        let cwd = main_hl.parent().expect("fixture dir");
        checked += 1;
        match assert_identical(&name, &src, cwd) {
            Ok(()) => eprintln!("ok (corpus): {name}"),
            Err(e) => failures.push(e),
        }
    }

    eprintln!("\nbus-devirt differential: checked {checked} program(s)");
    assert!(
        failures.is_empty(),
        "STATIC ≠ DYNAMIC on {} program(s):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
