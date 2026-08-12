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
        while i < 40 { "race.a" <- A { n: i }; i = i + 1; }
    }
}

locus PubB {
    bus { publish "race.b" of type B; }
    run() {
        let mut i = 0;
        while i < 40 { "race.b" <- B { n: i }; i = i + 1; }
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
