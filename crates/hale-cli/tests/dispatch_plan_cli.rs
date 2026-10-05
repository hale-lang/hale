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
///
/// F.40 phase 4, I2: this test claimed the refusal and could not see
/// its cause. Its builds were directory builds, whose recordings no
/// replay admitted at all (`build` fingerprinted `debug`, and a
/// directory frames its paths unlike its entry file, I3's), so the
/// refusal held whatever the plan was. The builds are single files now,
/// two of one program differing only in the lowering, and each is
/// admitted by the replay that lowers as it did and refused by the
/// other. The lowering is selected by `LOTUS_NO_BUS_DEVIRT`, which is
/// also a knob of the options fingerprint (GH #843), so end to end the
/// plan cannot differ alone; that the plan's own frame moves the
/// identity is `build_env`'s unit test.
#[test]
fn hale_build_artifacts_carry_the_plan_in_their_identity() {
    let dir = workdir("buildident");

    // Build one program twice: default lowering, then the all-dynamic
    // control arm. Separate dirs so the second build cannot be mistaken
    // for the first.
    let build = |tag: &str, devirt: bool| -> (PathBuf, PathBuf) {
        let out = dir.join(tag);
        std::fs::create_dir_all(&out).unwrap();
        let prog = out.join("app.hl");
        std::fs::write(&prog, BUS_PROG).unwrap();
        let mut cmd = hale();
        cmd.arg("build").arg(&prog);
        if !devirt {
            cmd.env("LOTUS_NO_BUS_DEVIRT", "1");
        }
        let o = cmd.output().expect("hale build");
        assert!(
            o.status.success(),
            "build failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        (prog, out.join("app"))
    };
    let (fast_prog, fast) = build("fast", true);
    let (slow_prog, slow) = build("slow", false);

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
    let fast_rec = record(&fast, "fast");
    let slow_rec = record(&slow, "slow");
    let d_fast = recorded_exec_digest(&fast_rec);
    let d_slow = recorded_exec_digest(&slow_rec);
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

    // Enforced, not merely visible: each recording is admitted by the
    // replay that lowers as its build did, and refused by the other.
    // (The program prints, so replay's live-effect gate is accepted
    // explicitly to reach the identity.)
    let replay = |rec: &Path, prog: &Path, devirt: bool| -> (bool, String) {
        let mut cmd = hale();
        cmd.arg("replay").arg(rec).arg(prog).arg("--allow-live-effects");
        if !devirt {
            cmd.env("LOTUS_NO_BUS_DEVIRT", "1");
        }
        let out = cmd.output().expect("hale replay");
        (out.status.success(), String::from_utf8_lossy(&out.stderr).into_owned())
    };
    for (rec, prog, devirt, tag) in [(&fast_rec, &fast_prog, true, "fast"), (&slow_rec, &slow_prog, false, "slow")] {
        let (ok, err) = replay(rec, prog, devirt);
        assert!(
            ok && !err.contains("different build inputs"),
            "the {tag} build's own lowering admits its recording:\n{}",
            err
        );
        let (ok, err) = replay(rec, prog, !devirt);
        assert!(
            !ok && err.contains("recorded from different build inputs"),
            "replay of the {tag} build's recording across the lowering boundary was admitted:\n{}",
            err
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
