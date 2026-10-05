//! `verification/cascade_model.c`, the GenMC model of the failure
//! cascade (F.40 phase 3, L5), held to compile on every PR, since GenMC
//! itself is not in CI: the model and each of its negative controls
//! pass `clang -std=c11 -Wall -Wextra -Werror -pthread -fsyntax-only`.
//!
//! The model is also an ordinary threaded C program, so it runs natively
//! too. One native run is one interleaving, not the proof GenMC's
//! exhaustive exploration is (`verification/run_genmc.sh`), but it is a
//! control the suite can afford: the model passes, and
//! `-DMODEL_BUG_DELIVER_IN_PLACE` (a failure off the owner's domain
//! calls the handler in place, as before L5's fourth part) fails
//! assertion (6), deterministically, in phase 3, and
//! `-DMODEL_BUG_SIBLING_RECLAIM_WAITS` (a reclaim inside a handler waits
//! for a sibling's delivery held for its own thread, as before PR
//! #1348's review) fails assertion (8) in phase 4.

use std::path::PathBuf;
use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// The negative controls the model's header lists.
const CONTROLS: &[&str] = &[
    "MODEL_BUG_NO_HOLD",
    "MODEL_BUG_RECLAIM_NOW",
    "MODEL_BUG_DELIVERED_BEFORE_HANDLER",
    "MODEL_BUG_ADMIT_IGNORES_CANCEL",
    "MODEL_BUG_RECLAIM_SKIPS_WAIT",
    "MODEL_BUG_DELIVER_IN_PLACE",
    "MODEL_BUG_NO_CELL_HOLD",
    "MODEL_BUG_SIBLING_RECLAIM_WAITS",
];

fn model() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../verification/cascade_model.c")
}

const FLAGS: &[&str] = &["-std=c11", "-Wall", "-Wextra", "-Werror", "-pthread"];

#[test]
fn the_model_and_every_control_compile() {
    let src = std::fs::read_to_string(model()).expect("read the model");
    for control in std::iter::once("").chain(CONTROLS.iter().copied()) {
        if !control.is_empty() {
            assert!(src.contains(&format!("-D{control}")), "the header names no control {control}");
        }
        let mut cmd = Command::new("clang");
        cmd.args(FLAGS).arg("-fsyntax-only");
        if !control.is_empty() {
            cmd.arg(format!("-D{control}"));
        }
        let out = cmd.arg(model()).output().expect("run clang");
        assert!(out.status.success(), "clang {control}:\n{}", String::from_utf8_lossy(&out.stderr));
    }
}

/// Builds the model natively (with or without one control) and runs it
/// once under a deadline: (exit code, stderr).
fn run_native(control: Option<&str>) -> (Option<i32>, String) {
    let bin = harness::unique_bin(&format!("cascade_model_{}", control.unwrap_or("model").to_lowercase()));
    let mut cmd = Command::new("clang");
    cmd.args(FLAGS).arg("-O1");
    if let Some(c) = control {
        cmd.arg(format!("-D{c}"));
    }
    let out = cmd.arg(model()).arg("-o").arg(&bin).output().expect("run clang");
    assert!(out.status.success(), "clang {control:?}:\n{}", String::from_utf8_lossy(&out.stderr));
    let run = Command::new("timeout").arg("60").arg(&bin).output().expect("run the model");
    let _ = std::fs::remove_file(&bin);
    (run.status.code(), String::from_utf8_lossy(&run.stderr).into_owned())
}

#[test]
fn one_native_run_passes_and_the_in_place_control_fails_assertion_6() {
    let (code, stderr) = run_native(None);
    assert_eq!(code, Some(0), "the model, one native run:\n{stderr}");
    let (code, stderr) = run_native(Some("MODEL_BUG_DELIVER_IN_PLACE"));
    assert_ne!(code, Some(0), "-DMODEL_BUG_DELIVER_IN_PLACE passed a native run");
    assert_ne!(code, Some(124), "-DMODEL_BUG_DELIVER_IN_PLACE hung instead of failing:\n{stderr}");
    assert!(stderr.contains("self_tid == o->domain"), "-DMODEL_BUG_DELIVER_IN_PLACE failed elsewhere than (6):\n{stderr}");
}

/// The sibling replacement (phase 4): without the deferral behind a
/// delivery held for the reclaiming thread, the reclaim inside A's
/// handler reaches the wait for B's delivery, which only that thread
/// could run, and assertion (8) fails, deterministically (the handler
/// waits for B's post before it replaces B).
#[test]
fn the_sibling_control_fails_assertion_8() {
    let (code, stderr) = run_native(Some("MODEL_BUG_SIBLING_RECLAIM_WAITS"));
    assert_ne!(code, Some(0), "-DMODEL_BUG_SIBLING_RECLAIM_WAITS passed a native run");
    assert_ne!(code, Some(124), "-DMODEL_BUG_SIBLING_RECLAIM_WAITS hung instead of failing:\n{stderr}");
    assert!(stderr.contains("n->posted == self_tid"), "-DMODEL_BUG_SIBLING_RECLAIM_WAITS failed elsewhere than (8):\n{stderr}");
}
