//! GH #476 Change 8 — the dispatch plan is part of the execution
//! identity, so a build that lowers dispatch differently is a
//! different build and its recording is not admitted.
//!
//! The plan the identity frames is the resolved program's, the one
//! codegen lowers (F.40 phase 1.5). The model's plan was compared with
//! codegen's over the corpus here until then; the comparison ran once
//! more, over every corpus program, as a shadow against the resolved
//! program's plan, and was retired with every shared subject's flavor
//! agreeing.

use std::path::{Path, PathBuf};
use std::process::Command;

fn hale() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hale"))
}

fn workdir(name: &str) -> PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!("hale_dispatch_cli_{}_{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir");
    d
}

const BUS_PROG: &str = r#"
type T { n: Int = 0; }
topic Evt { payload: T; subject: "id.evt"; }
locus Sub {
    params { seen: Int = 0; }
    bus { subscribe Evt as on_e; }
    fn on_e(t: T) { self.seen = self.seen + 1; }
}
main locus App {
    params { s: Sub = Sub { }; }
    bus { publish Evt; }
    run() { Evt <- T { n: 1 }; println("done"); }
}
fn main() { App { }; }
"#;

/// The recording header's execution identity (4×u64 at offset 56).
fn recorded_exec_digest(rec: &Path) -> [u64; 4] {
    let b = std::fs::read(rec).expect("recording");
    assert!(b.len() >= 112, "truncated recording");
    let mut out = [0u64; 4];
    for (i, part) in out.iter_mut().enumerate() {
        let o = 56 + i * 8;
        *part = u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    }
    out
}

fn record_with(dir: &Path, prog: &Path, tag: &str, devirt: bool) -> PathBuf {
    let rec = dir.join(format!("{}.halerec", tag));
    let mut cmd = hale();
    cmd.arg("run").arg(prog).env("LOTUS_OBS_RECORD", &rec);
    if !devirt {
        cmd.env("LOTUS_NO_BUS_DEVIRT", "1");
    }
    let out = cmd.output().expect("hale run");
    assert!(
        out.status.success(),
        "recorded run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    rec
}

/// Same sources, same toolchain, same options — but one build
/// lowers every subject dynamically and the other devirtualizes.
/// Those are different executables, so they must not share an
/// execution identity, and a recording from one must not be
/// admitted against the other.
#[test]
fn a_different_lowering_is_a_different_build_identity() {
    let dir = workdir("identity");
    let prog = dir.join("app.hl");
    std::fs::write(&prog, BUS_PROG).unwrap();

    let devirt = record_with(&dir, &prog, "devirt", true);
    let dynamic = record_with(&dir, &prog, "dynamic", false);
    let d1 = recorded_exec_digest(&devirt);
    let d2 = recorded_exec_digest(&dynamic);
    assert_ne!(
        d1, d2,
        "the all-dynamic control arm and the devirtualized build \
         stamped the SAME execution identity — the dispatch plan is \
         not part of what a build is"
    );

    // And the boundary is enforced, not merely visible: replaying
    // the all-dynamic recording against the (devirtualized) default
    // build is refused by name.
    let out = hale()
        .arg("replay")
        .arg(&dynamic)
        .arg(&prog)
        // The program prints, so replay's live-effect gate fires
        // first; accept it explicitly to reach the identity check
        // this test is about.
        .arg("--allow-live-effects")
        .output()
        .expect("hale replay");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success()
            && err.contains("recorded from different build inputs"),
        "replay across the lowering boundary was admitted:\n{}",
        err
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Review round 1, blocker 4: `hale build` artifacts must carry an
/// execution identity too. The identity plumbing landed on `hale
/// run` and `hale replay` — whose temporary builds set it — while
/// the ordinary build path set only the model hash, so the PR's
/// central claim ("a different lowering is a different build") was
/// false for exactly the binaries users ship.
#[test]
fn hale_build_artifacts_carry_the_plan_in_their_identity() {
    let dir = workdir("buildident");
    let seed = dir.join("app");
    std::fs::create_dir_all(&seed).unwrap();
    std::fs::write(seed.join("main.hl"), BUS_PROG).unwrap();

    // Build twice from ONE source tree: default lowering, then the
    // all-dynamic control arm. Separate output dirs so the second
    // build cannot be mistaken for the first.
    let build = |tag: &str, devirt: bool| -> PathBuf {
        let out = dir.join(tag);
        std::fs::create_dir_all(&out).unwrap();
        std::fs::copy(seed.join("main.hl"), out.join("main.hl")).unwrap();
        let mut cmd = hale();
        cmd.arg("build").arg(&out);
        if !devirt {
            cmd.env("LOTUS_NO_BUS_DEVIRT", "1");
        }
        let o = cmd.output().expect("hale build");
        assert!(
            o.status.success(),
            "build failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        out.join(tag)
    };
    let fast = build("fast", true);
    let slow = build("slow", false);

    let record = |bin: &Path, tag: &str| -> PathBuf {
        let rec = dir.join(format!("{}.halerec", tag));
        let o = Command::new(bin)
            .env("LOTUS_OBS_RECORD", &rec)
            .output()
            .expect("run built binary");
        assert!(
            o.status.success(),
            "recorded run failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        rec
    };
    let d_fast = recorded_exec_digest(&record(&fast, "fast"));
    let d_slow = recorded_exec_digest(&record(&slow, "slow"));
    assert_ne!(
        d_fast, [0; 4],
        "a `hale build` artifact carries no execution identity at all"
    );
    assert_ne!(d_slow, [0; 4]);
    assert_ne!(
        d_fast, d_slow,
        "two `hale build` artifacts that lower dispatch differently \
         share one execution identity"
    );

    // Enforced, not merely visible.
    let out = hale()
        .arg("replay")
        .arg(dir.join("slow.halerec"))
        .arg(seed.join("main.hl"))
        .arg("--allow-live-effects")
        .output()
        .expect("hale replay");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success()
            && err.contains("recorded from different build inputs"),
        "replay across the lowering boundary was admitted:\n{}",
        err
    );
    let _ = std::fs::remove_dir_all(&dir);
}
