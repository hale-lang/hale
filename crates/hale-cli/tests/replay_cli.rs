//! GH #296: `hale replay` end to end — record with `hale run`
//! under LOTUS_OBS_RECORD, then re-execute the recording.
//!
//! What is pinned here, per RFC phase:
//!   - Phase 3: journaled inputs (time, rand) serve back with zero
//!     divergences — a program whose payloads depend on
//!     `std::rand` replays with identical payload bytes.
//!   - Phase 4: two racing pinned publishers produce a
//!     nondeterministic recorded interleaving; the replay must
//!     reproduce THAT interleaving, which only order enforcement
//!     can do reliably.
//!   - Admission: a recording made from a different model is
//!     rejected by shape_hash, and a truncated recording is
//!     refused outright.

use std::path::{Path, PathBuf};
use std::process::Command;

fn hale() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hale"))
}

fn workdir(name: &str) -> PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!("hale_replay_cli_{}_{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir");
    d
}

fn record(dir: &Path, prog: &Path) -> PathBuf {
    let rec = dir.join("run.halerec");
    let out = hale()
        .arg("run")
        .arg(prog)
        .env("LOTUS_OBS_RECORD", &rec)
        .output()
        .expect("hale run");
    assert!(
        out.status.success(),
        "recorded run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(rec.is_file(), "no recording produced");
    rec
}

const JOURNALED: &str = r#"
type Tick { n: Int = 0; }

locus Sink {
    params { seen: Int = 0; }
    bus { subscribe "e2e.tick" as on_t of type Tick; }
    fn on_t(t: Tick) { self.seen = self.seen + 1; }
}

locus Pub {
    bus { publish "e2e.tick" of type Tick; }
    run() {
        let mut i = 0;
        while i < 20 {
            let r = std::rand::next_int(1000000);
            "e2e.tick" <- Tick { n: r };
            i = i + 1;
        }
    }
}

main locus App {
    params { s: Sink = Sink { }; p: Pub = Pub { }; }
    placement { p: pinned(core = 0); }
    run() { std::time::sleep(800ms); }
}

fn main() { App { }; }
"#;

/// Two pinned publishers racing into one main-tree sink: the
/// recorded consume interleaving is one of astronomically many;
/// replay must serve back exactly it.
const RACING: &str = r#"
type A { n: Int = 0; }
type B { n: Int = 0; }

locus Sink {
    params { seen: Int = 0; }
    bus {
        subscribe "race.a" as on_a of type A;
        subscribe "race.b" as on_b of type B;
    }
    fn on_a(a: A) { self.seen = self.seen + 1; }
    fn on_b(b: B) { self.seen = self.seen + 1; }
}

locus PubA {
    bus { publish "race.a" of type A; }
    run() {
        let mut i = 0;
        while i < 40 {
            let r = std::rand::next_int(1000);
            "race.a" <- A { n: r };
            i = i + 1;
        }
    }
}

locus PubB {
    bus { publish "race.b" of type B; }
    run() {
        let mut i = 0;
        while i < 40 {
            let r = std::rand::next_int(1000);
            "race.b" <- B { n: r };
            i = i + 1;
        }
    }
}

main locus App {
    params {
        s: Sink = Sink { };
        a: PubA = PubA { };
        b: PubB = PubB { };
    }
    placement { a: pinned(core = 0); b: pinned(core = 1); }
    run() { std::time::sleep(900ms); }
}

fn main() { App { }; }
"#;

#[test]
fn journaled_inputs_replay_without_divergence() {
    let dir = workdir("journal");
    let prog = dir.join("demo.hl");
    std::fs::write(&prog, JOURNALED).unwrap();
    let rec = record(&dir, &prog);

    let out = hale()
        .arg("replay")
        .arg(&rec)
        .arg(&prog)
        .arg("--allow-live-effects")
        .arg("--diff")
        .output()
        .expect("hale replay");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "replay failed:\n{}\n{}",
        stdout,
        stderr
    );
    assert!(
        stdout.contains("replay matches the recording"),
        "no match report:\n{}\n{}",
        stdout,
        stderr
    );
    assert!(
        stderr.contains("journal served fully — 0 divergences"),
        "journal diverged:\n{}",
        stderr
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn racing_publishers_replay_in_the_recorded_interleaving() {
    let dir = workdir("racing");
    let prog = dir.join("race.hl");
    std::fs::write(&prog, RACING).unwrap();
    let rec = record(&dir, &prog);

    // Replay twice: each must reproduce the recorded interleaving
    // exactly (without Phase-4 enforcement this is a coin flip per
    // delivery — 80 queued deliveries from two racing threads).
    for round in 0..2 {
        let out = hale()
            .arg("replay")
            .arg(&rec)
            .arg(&prog)
            .arg("--allow-live-effects")
            .arg("--diff")
            .output()
            .expect("hale replay");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success() && stdout.contains("replay matches"),
            "round {}: replay diverged from the recorded \
             interleaving:\n{}\n{}",
            round,
            stdout,
            stderr
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn admission_rejects_model_mismatch_and_truncation() {
    let dir = workdir("admission");
    let prog = dir.join("demo.hl");
    std::fs::write(&prog, JOURNALED).unwrap();
    let rec = record(&dir, &prog);

    // Different model (one extra locus): rejected by shape_hash.
    let prog2 = dir.join("demo2.hl");
    std::fs::write(
        &prog2,
        JOURNALED.replace(
            "locus Sink {",
            "locus Extra { params { x: Int = 0; } }\nlocus Sink {",
        ),
    )
    .unwrap();
    let out = hale()
        .arg("replay")
        .arg(&rec)
        .arg(&prog2)
        .arg("--allow-live-effects")
        .output()
        .expect("hale replay");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The exec-digest check fires first: changed source = changed
    // build inputs, which is the stronger rejection.
    assert!(
        stderr.contains("different build inputs"),
        "wrong rejection:\n{}",
        stderr
    );

    // Truncated recording: refused before anything runs.
    let trunc = dir.join("trunc.halerec");
    let bytes = std::fs::read(&rec).unwrap();
    std::fs::write(&trunc, &bytes[..bytes.len() - 100]).unwrap();
    let out = hale()
        .arg("replay")
        .arg(&trunc)
        .arg(&prog)
        .arg("--allow-live-effects")
        .output()
        .expect("hale replay");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("truncated"),
        "wrong refusal:\n{}",
        stderr
    );

    // The review's sharper case: bytes removed from the MIDDLE with
    // the original trailer intact. Trailer magic at EOF alone would
    // call this clean; exact-end + entry-count validation must not.
    let corrupt = dir.join("corrupt.halerec");
    let mut mangled = bytes.clone();
    let cut = mangled.len() / 2;
    mangled.drain(cut..cut + 48);
    std::fs::write(&corrupt, &mangled).unwrap();
    let out = hale()
        .arg("replay")
        .arg(&corrupt)
        .arg(&prog)
        .arg("--allow-live-effects")
        .output()
        .expect("hale replay");
    assert!(
        !out.status.success(),
        "an internally corrupted recording was admitted"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Review round 2, finding 1's canary: an otherwise deterministic
/// program that publishes to a transport-BOUND topic (a unix
/// socket here; same class as udp) must be
/// refused by default — the user-level effect is `publish`, but
/// re-executing it sends real datagrams.
const UDP_BOUND: &str = r#"
type Tick { n: Int = 0; }
topic Wire { payload: Tick; subject: "wire.tick"; }

main locus App {
    bus { publish Wire; }
    bindings { Wire: unix("/tmp/hale_replay_canary.sock", role: listen); }
    run() {
        let mut i = 0;
        while i < 5 { Wire <- Tick { n: i }; i = i + 1; }
        std::time::sleep(300ms);
    }
}

fn main() { App { }; }
"#;

#[test]
fn externally_bound_publish_is_refused_by_default() {
    let dir = workdir("udpbound");
    let prog = dir.join("wire.hl");
    std::fs::write(&prog, UDP_BOUND).unwrap();
    let rec = record(&dir, &prog);

    let out = hale()
        .arg("replay")
        .arg(&rec)
        .arg(&prog)
        .output()
        .expect("hale replay");
    assert!(
        !out.status.success(),
        "a transport-bound program replayed without the safety flag"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("live world") && stderr.contains("--allow-live-effects"),
        "wrong refusal:\n{}",
        stderr
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Review round 2, finding 8: env VALUES are withheld by default.
/// The recording must say so, a replayed env read must surface as a
/// named divergence (never a substituted value), and opting in with
/// LOTUS_OBS_RECORD_ENV=full must restore exact replay.
#[test]
fn env_values_redact_by_default_and_opt_in_restores_replay() {
    let dir = workdir("envredact");
    let prog = dir.join("env.hl");
    std::fs::write(
        &prog,
        r#"
type Tick { n: Int = 0; }
locus Sink {
    params { seen: Int = 0; }
    bus { subscribe "e.t" as on_t of type Tick; }
    fn on_t(t: Tick) { self.seen = self.seen + 1; }
}
locus Pub {
    bus { publish "e.t" of type Tick; }
    run() {
        let v = std::env::var("HALE_REPLAY_TEST_ENV");
        let n = len(v);
        "e.t" <- Tick { n: n };
    }
}
main locus App {
    params { s: Sink = Sink { }; p: Pub = Pub { }; }
    placement { p: pinned(core = 0); }
    run() { std::time::sleep(600ms); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();

    // Default policy: value withheld → replay diverges, namedly.
    let rec = dir.join("redacted.halerec");
    let out = hale()
        .arg("run")
        .arg(&prog)
        .env("LOTUS_OBS_RECORD", &rec)
        .env("HALE_REPLAY_TEST_ENV", "hunter2-not-in-artifact")
        .output()
        .expect("record");
    assert!(out.status.success());
    let raw = std::fs::read(&rec).unwrap();
    assert!(
        !raw.windows(7).any(|w| w == b"hunter2"),
        "redacted recording contains the env value"
    );
    let out = hale()
        .arg("replay")
        .arg(&rec)
        .arg(&prog)
        .arg("--allow-live-effects")
        .arg("--diff")
        .env("HALE_REPLAY_TEST_ENV", "hunter2-not-in-artifact")
        .output()
        .expect("replay");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "a redacted recording must not claim an exact match:\n{}",
        stderr
    );
    assert!(
        stderr.contains("withholds env VALUES") || stderr.contains("divergence"),
        "redaction must be named:\n{}",
        stderr
    );

    // Opt-in: full values → exact replay.
    let rec2 = dir.join("full.halerec");
    let out = hale()
        .arg("run")
        .arg(&prog)
        .env("LOTUS_OBS_RECORD", &rec2)
        .env("LOTUS_OBS_RECORD_ENV", "full")
        .env("HALE_REPLAY_TEST_ENV", "hunter2-not-in-artifact")
        .output()
        .expect("record full");
    assert!(out.status.success());
    let out = hale()
        .arg("replay")
        .arg(&rec2)
        .arg(&prog)
        .arg("--allow-live-effects")
        .arg("--diff")
        .output()
        .expect("replay full");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("replay matches"),
        "full-env recording must replay exactly:\n{}\n{}",
        stdout,
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 3, finding 3's canaries: (a) two pinned publishers racing
/// on ONE subject — the old deliver probe reloaded the topic's
/// high-water mark and attributed both racing deliveries to the
/// later publish; (b) a handler that republishes to the SAME
/// subject before the outer fanout's remaining delivers, which
/// clobbers any TLS-based seq handoff (the token is a local).
const SAME_SUBJECT: &str = r#"
type T { n: Int = 0; }

locus Sink {
    params { seen: Int = 0; }
    bus {
        subscribe "one.subj" as on_t of type T;
        publish "one.subj" of type T;
    }
    fn on_t(t: T) {
        self.seen = self.seen + 1;
        if t.n == 777 { "one.subj" <- T { n: 1000 }; }
    }
}

locus PubA {
    bus { publish "one.subj" of type T; }
    run() {
        let mut i = 0;
        while i < 30 { "one.subj" <- T { n: i }; i = i + 1; }
        "one.subj" <- T { n: 777 };
    }
}

locus PubB {
    bus { publish "one.subj" of type T; }
    run() {
        let mut i = 0;
        while i < 30 { "one.subj" <- T { n: 100 + i }; i = i + 1; }
    }
}

main locus App {
    params {
        s: Sink = Sink { };
        a: PubA = PubA { };
        b: PubB = PubB { };
    }
    placement { a: pinned(core = 0); b: pinned(core = 1); }
    run() { std::time::sleep(900ms); }
}

fn main() { App { }; }
"#;

#[test]
fn same_subject_race_and_nested_republish_replay_exactly() {
    let dir = workdir("samesubj");
    let prog = dir.join("one.hl");
    std::fs::write(&prog, SAME_SUBJECT).unwrap();
    let rec = record(&dir, &prog);

    for round in 0..2 {
        let out = hale()
            .arg("replay")
            .arg(&rec)
            .arg(&prog)
            .arg("--allow-live-effects")
            .arg("--diff")
            .output()
            .expect("hale replay");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success() && stdout.contains("replay matches"),
            "round {}: same-subject replay diverged:\n{}\n{}",
            round,
            stdout,
            stderr
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 3, finding 2's negative controls: modules nest top
/// declarations, and both admission walkers (effect rows, binding
/// scan) must recurse into them — a module-contained live effect
/// or transport binding failing open was the review's exact case.
#[test]
fn module_contained_effects_and_bindings_are_refused() {
    let dir = workdir("modgate");
    // Inline-module fns don't lower through codegen yet, so these
    // programs never RUN — which is fine: the safety gate fires
    // before the build, and it is deliberately ordered before
    // identity admission so a program-inherent refusal is never
    // masked by a recording mismatch. Any valid recording arms the
    // test.
    let plain = dir.join("plain.hl");
    std::fs::write(&plain, JOURNALED).unwrap();
    let rec = record(&dir, &plain);

    // (a) module-contained fn with a live effect (subprocess run).
    let prog_fx = dir.join("modfx.hl");
    std::fs::write(
        &prog_fx,
        r#"
module inner {
    fn danger() {
        let out = std::process::run("true") or raise;
        let c = out.code;
    }
}
type Tick { n: Int = 0; }
locus Sink {
    params { seen: Int = 0; }
    bus { subscribe "m.t" as on_t of type Tick; }
    fn on_t(t: Tick) { self.seen = self.seen + 1; }
}
main locus App {
    params { s: Sink = Sink { }; }
    bus { publish "m.t" of type Tick; }
    run() {
        inner::danger();
        "m.t" <- Tick { n: 1 };
    }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let out = hale()
        .arg("replay")
        .arg(&rec)
        .arg(&prog_fx)
        .output()
        .expect("hale replay");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("live world"),
        "module-contained effect passed the gate:\n{}",
        stderr
    );

    // (b) module-contained transport binding.
    let prog_b = dir.join("modbind.hl");
    std::fs::write(
        &prog_b,
        r#"
module wired {
    type Tick { n: Int = 0; }
    topic Wire { payload: Tick; subject: "m.wire"; }
    main locus App {
        bus { publish Wire; }
        bindings { Wire: unix("/tmp/hale_replay_modcanary.sock", role: listen); }
        run() { Wire <- Tick { n: 1 }; std::time::sleep(200ms); }
    }
}
fn main() { wired::App { }; }
"#,
    )
    .unwrap();
    // The module-main shape may not even record (main-in-module is
    // its own question) — the load-bearing assertion is the GATE:
    // admission must see the binding regardless of a recording.
    let out = hale()
        .arg("replay")
        .arg(&rec)
        .arg(&prog_b)
        .output()
        .expect("hale replay");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("live world"),
        "module-contained binding passed the gate:\n{}",
        stderr
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Round 4, finding 1's canary: a probe-free program must still
/// produce a recording — and REPLACE whatever sat at the path. Lazy
/// creation let `fn main() { let x = 1 + 1; }` exit successfully
/// with no file, while a stale artifact at the path silently
/// impersonated the requested run.
#[test]
fn probe_free_run_replaces_the_artifact_at_the_path() {
    let dir = workdir("probefree");
    let prog = dir.join("quiet.hl");
    std::fs::write(&prog, "fn main() { let x = 1 + 1; }\n").unwrap();
    let rec = dir.join("run.halerec");
    std::fs::write(&rec, b"SENTINEL: not a recording").unwrap();

    let out = hale()
        .arg("run")
        .arg(&prog)
        .env("LOTUS_OBS_RECORD", &rec)
        .output()
        .expect("record quiet program");
    assert!(
        out.status.success(),
        "quiet run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bytes = std::fs::read(&rec).unwrap();
    assert!(
        !bytes.starts_with(b"SENTINEL"),
        "the stale artifact at the path was left impersonating \
         this run"
    );
    // It must parse as a CLEAN current recording with zero
    // semantic events and a real execution identity.
    let r = crate::parse_rec(&rec);
    assert!(r.0, "probe-free recording is not clean");
    assert_eq!(r.1, 0, "a probe-free run recorded semantic events");
    assert!(r.2, "probe-free recording carries no exec identity");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Minimal v0.3 reader for the canary above: (clean, semantic ring
/// records, exec_digest nonzero). Semantic = any ring record other
/// than the private CONSUMER identity stamp.
fn parse_rec(p: &Path) -> (bool, usize, bool) {
    let b = std::fs::read(p).unwrap();
    let u64at = |o: usize| {
        u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
    };
    let u32at = |o: usize| {
        u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
    };
    if b.len() < 112 || u64at(0) != 0x30434552454C4148 {
        return (false, usize::MAX, false);
    }
    let exec_nonzero = (0..4).any(|i| u64at(56 + i * 8) != 0);
    let mut end = b.len();
    let has_trailer = u64at(end - 16) == 0x30444E45454C4148;
    let trailer_count = if has_trailer { u64at(end - 8) } else { 0 };
    if has_trailer {
        end -= 16;
    }
    let mut off = 96usize;
    let mut total = 0u64;
    let mut semantic = 0usize;
    while off + 8 <= end {
        let tag = u32at(off);
        total += 1;
        if tag == 0 {
            if end - off < 24 {
                return (false, semantic, exec_nonzero);
            }
            let w0 = u64at(off + 8);
            let ekind = ((w0 >> 20) & 0x1F) as u32;
            let ring = u32at(off + 4);
            if !(ring & 0x8000_0000 != 0 && ekind == 1) {
                semantic += 1;
            }
            off += 24;
        } else if (1..=3).contains(&tag) {
            if end - off < 32 {
                return (false, semantic, exec_nonzero);
            }
            let size = u64at(off + 24) as usize;
            let padded = (size + 7) & !7;
            if padded > end - off - 32 {
                return (false, semantic, exec_nonzero);
            }
            off += 32 + padded;
        } else {
            return (false, semantic, exec_nonzero);
        }
    }
    (
        has_trailer && off == end && trailer_count == total,
        semantic,
        exec_nonzero,
    )
}

/// Round 4, finding 2's canary, reshaped by a checker fact: the
/// literal same-name collision CANNOT typecheck — module-scoped
/// names share the flat top-level namespace ("duplicate top-level
/// name `Worker`"), so the aliasing shape is unreachable through
/// any checked path (and `hale replay` checks before admitting).
/// The qualified summary key stays as defense-in-depth for any
/// future namespace relaxation and for unchecked callers of the
/// manifest API. What IS reachable — a module LOCUS whose
/// lifecycle body carries a live effect — must fail closed as
/// `unclassified` under its qualified key and be refused.
#[test]
fn module_locus_lifecycle_fails_closed_under_qualified_key() {
    let dir = workdir("modalias");
    let plain = dir.join("plain.hl");
    std::fs::write(&plain, JOURNALED).unwrap();
    let rec = record(&dir, &plain);

    let prog = dir.join("alias.hl");
    std::fs::write(
        &prog,
        r#"
module inner {
    locus Worker {
        params { n: Int = 0; }
        run() {
            let result = std::process::run("true") or raise;
        }
    }
}
type Tick { n: Int = 0; }
locus Sink {
    params { seen: Int = 0; }
    bus { subscribe "a.t" as on_t of type Tick; }
    fn on_t(t: Tick) { self.seen = self.seen + 1; }
}
main locus App {
    params { s: Sink = Sink { }; w: Worker = Worker { }; }
    bus { publish "a.t" of type Tick; }
    run() { "a.t" <- Tick { n: 1 }; }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let out = hale()
        .arg("replay")
        .arg(&rec)
        .arg(&prog)
        .output()
        .expect("hale replay");
    assert!(
        !out.status.success(),
        "a module locus aliased a pure top-level locus and passed \
         the gate"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("live world") && stderr.contains("unclassified"),
        "wrong refusal:\n{}",
        stderr
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- phase 5 review-round canaries ---------------------------------

/// A crash-cut tape under `--allow-truncated --diff`: events past the
/// recorded prefix are the unknown post-crash suffix, not
/// divergences — the runtime verdict and the prefix comparator must
/// AGREE on that (they used to contradict).
#[test]
fn truncated_diff_accepts_the_post_crash_surplus() {
    let dir = workdir("truncdiff");
    let prog = dir.join("steady.hl");
    std::fs::write(
        &prog,
        r#"
type Tick { n: Int = 0; }
locus Sink {
    params { seen: Int = 0; }
    bus { subscribe "td.tick" as on_t of type Tick; }
    fn on_t(t: Tick) { self.seen = self.seen + 1; }
}
main locus App {
    params { s: Sink = Sink { }; }
    bus { publish "td.tick" of type Tick; }
    run() {
        let mut i = 0;
        while i < 400 {
            let r = std::rand::next_int(1000000);
            "td.tick" <- Tick { n: r };
            std::time::sleep(5ms);
            i = i + 1;
        }
    }
}
fn main() { App { }; }
"#,
    )
    .unwrap();

    // Record via `hale run` (which execs the built binary — one pid)
    // and SIGKILL it mid-stream.
    let rec = dir.join("crash.halerec");
    let mut child = hale()
        .arg("run")
        .arg(&prog)
        .env("LOTUS_OBS_RECORD", &rec)
        .spawn()
        .expect("spawn recorded run");
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let sz = std::fs::metadata(&rec).map(|m| m.len()).unwrap_or(0);
        if sz > 8192 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "recording never grew (size {})",
            sz
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    child.kill().expect("kill");
    let _ = child.wait();

    // The full flow the review demanded: prefix must match, the
    // post-crash surplus must not fail the diff.
    let out = hale()
        .arg("replay")
        .arg(&rec)
        .arg(&prog)
        .arg("--allow-truncated")
        .arg("--diff")
        .arg("--allow-live-effects")
        .output()
        .expect("hale replay --allow-truncated --diff");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "truncated --diff must accept the surplus:\nstdout:{}\nstderr:{}",
        stdout,
        stderr
    );
    assert!(
        stdout.contains("replay matches the recording"),
        "expected the prefix match verdict:\n{}\n{}",
        stdout,
        stderr
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// ...and a divergence INSIDE the recorded prefix still fails.
/// (Corrupting the baseline cannot create one — replay is DRIVEN by
/// the baseline, and raw in-process payloads compare by declared
/// size by design. A withheld env VALUE is the honest in-prefix
/// mismatch: the default recording policy withholds it, so the
/// replayed read is a NAMED divergence inside the prefix.)
#[test]
fn withheld_read_inside_the_prefix_still_fails_truncated_diff() {
    let dir = workdir("truncbad");
    let prog = dir.join("envy.hl");
    std::fs::write(
        &prog,
        r#"
type Tick { n: Int = 0; }
locus Sink {
    params { seen: Int = 0; }
    bus { subscribe "tb.tick" as on_t of type Tick; }
    fn on_t(t: Tick) { self.seen = self.seen + 1; }
}
main locus App {
    params { s: Sink = Sink { }; }
    bus { publish "tb.tick" of type Tick; }
    run() {
        // An env read at the very start: inside any nonempty prefix.
        let home = std::env::var("HOME");
        let mut i = 0;
        while i < 40 {
            "tb.tick" <- Tick { n: i };
            i = i + 1;
        }
    }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let rec = record(&dir, &prog);

    // Cut the trailer + a tail chunk: an accepted truncated tape
    // whose surviving prefix contains the (value-withheld) env read.
    let b = std::fs::read(&rec).unwrap();
    let mut end = b.len();
    if &b[end - 16..end - 8] == b"HALEEND0" {
        end -= 16;
    }
    let cut = dir.join("cut.halerec");
    std::fs::write(&cut, &b[..end - 64]).unwrap();

    let out = hale()
        .arg("replay")
        .arg(&cut)
        .arg(&prog)
        .arg("--allow-truncated")
        .arg("--diff")
        .arg("--allow-live-effects")
        .output()
        .expect("hale replay withheld prefix");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "an in-prefix divergence must still fail truncated --diff:\n{}",
        stderr
    );
    assert!(
        stderr.contains("env.var") || stderr.contains("DIVERGED"),
        "the failure must be the named in-prefix divergence:\n{}",
        stderr
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--feed` bypasses identity admission, never effect safety: a
/// program whose frontier reaches syscall must still be refused
/// until --allow-live-effects joins it.
#[test]
fn feed_requires_the_effects_gate_for_live_effects() {
    let dir = workdir("feedgate");
    let pure = dir.join("pure.hl");
    std::fs::write(
        &pure,
        r#"
main locus App {
    params { x: Int = 1; }
    run() { let y = self.x + 1; }
}
fn main() { App { }; }
"#,
    )
    .unwrap();
    let rec = record(&dir, &pure);

    let effectful = dir.join("effectful.hl");
    std::fs::write(
        &effectful,
        r#"
main locus App {
    run() { std::time::sleep(1ms); }
}
fn main() { App { }; }
"#,
    )
    .unwrap();

    // --feed alone: refused, residue named.
    let out = hale()
        .arg("replay")
        .arg(&rec)
        .arg(&effectful)
        .arg("--feed")
        .output()
        .expect("hale replay --feed");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "--feed alone must not unlock live effects:\n{}",
        stderr
    );
    assert!(
        stderr.contains("live world") && stderr.contains("syscall"),
        "the refusal must name the residue:\n{}",
        stderr
    );

    // Both flags: the explicit backtest-with-live-effects spelling.
    let out = hale()
        .arg("replay")
        .arg(&rec)
        .arg(&effectful)
        .arg("--feed")
        .arg("--allow-live-effects")
        .output()
        .expect("hale replay --feed --allow-live-effects");
    assert!(
        out.status.success(),
        "--feed --allow-live-effects must proceed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two listener bindings mean two recorded ingress sources; under
/// --diff the verify recording's per-consumer public streams must
/// align with the original's — one global injector thread would
/// collapse them onto one identity.
#[test]
fn two_listeners_replay_clean_under_diff() {
    let dir = workdir("twolisten");
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let sock_a = format!(
        "{}/hale-2l-{}-{}.a.sock",
        std::env::temp_dir().display(),
        std::process::id(),
        nanos
    );
    let sock_b = format!("{}.b", sock_a);
    let sub = dir.join("sub.hl");
    std::fs::write(
        &sub,
        format!(
            r#"
type TA {{ n: Int = 0; }}
type TB {{ n: Int = 0; }}
topic A {{ payload: TA; subject: "tl.a"; }}
topic B {{ payload: TB; subject: "tl.b"; }}
locus SubA {{
    params {{ seen: Int = 0; }}
    bus {{ subscribe A as on_a; }}
    fn on_a(t: TA) {{ self.seen = self.seen + 1; println("a=", t.n); }}
}}
locus SubB {{
    params {{ seen: Int = 0; }}
    bus {{ subscribe B as on_b; }}
    fn on_b(t: TB) {{ self.seen = self.seen + 1; println("b=", t.n); }}
}}
main locus App {{
    params {{ sa: SubA = SubA {{ }}; sb: SubB = SubB {{ }}; }}
    bindings {{
        A: unix("{}", role: listen);
        B: unix("{}", role: listen);
    }}
    run() {{
        let mut waited = 0;
        while self.sa.seen < 1 || self.sb.seen < 1 {{
            std::time::sleep(100ms);
            waited = waited + 1;
            if waited > 200 {{ std::process::exit(3); }}
        }}
        std::time::sleep(400ms);
    }}
}}
fn main() {{ App {{ }}; }}
"#,
            sock_a, sock_b
        ),
    )
    .unwrap();
    let pubs = dir.join("pubs.hl");
    std::fs::write(
        &pubs,
        format!(
            r#"
type TA {{ n: Int = 0; }}
type TB {{ n: Int = 0; }}
topic A {{ payload: TA; subject: "tl.a"; }}
topic B {{ payload: TB; subject: "tl.b"; }}
main locus App {{
    bus {{ publish A; publish B; }}
    bindings {{
        A: unix("{}", role: connect);
        B: unix("{}", role: connect);
    }}
    run() {{
        std::time::sleep(900ms);
        A <- TA {{ n: 11 }};
        B <- TB {{ n: 21 }};
        std::time::sleep(50ms);
        A <- TA {{ n: 12 }};
        B <- TB {{ n: 22 }};
        std::time::sleep(200ms);
    }}
}}
fn main() {{ App {{ }}; }}
"#,
            sock_a, sock_b
        ),
    )
    .unwrap();

    let rec = dir.join("two.halerec");
    let mut subp = hale()
        .arg("run")
        .arg(&sub)
        .env("LOTUS_OBS_RECORD", &rec)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn recorded sub");
    let p = hale().arg("run").arg(&pubs).output().expect("pubs");
    assert!(
        p.status.success(),
        "publisher run failed: {}",
        String::from_utf8_lossy(&p.stderr)
    );
    let out = subp.wait_with_output().expect("sub exit");
    assert!(
        out.status.success(),
        "recorded two-listener run failed:\nstdout:{}\nstderr:{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let _ = std::fs::remove_file(&sock_a);
    let _ = std::fs::remove_file(&sock_b);
    let out = hale()
        .arg("replay")
        .arg(&rec)
        .arg(&sub)
        .arg("--diff")
        .arg("--allow-live-effects")
        .output()
        .expect("hale replay --diff");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() && stdout.contains("replay matches"),
        "two-source ingress must survive --diff:\nstdout:{}\nstderr:{}",
        stdout,
        stderr
    );
    let _ = std::fs::remove_dir_all(&dir);
}
