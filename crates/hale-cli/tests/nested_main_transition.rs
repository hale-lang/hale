//! F.40 phase 3, L4: a module-nested `main locus` after the transition
//! (the outside review of #1293, finding 1).
//!
//! Decision 2 says a `main locus` inside a `module { }` is not the
//! entry. During the transition lowering deployed the first `main
//! locus` that was not a library's, module-nested or not, and spawned
//! its pinned threads, so the checker guarded that root. The transition
//! is over: lowering deploys the entry and nothing else, and a seed
//! whose only `main locus` is module-nested is refused by the check at
//! that locus's name, since nothing in it is the entry. The refusal is
//! one more diagnostic, never a replacement: every other rule still
//! judges the nested `main` as it judges the same declarations at the
//! top level (GH #825).
//!
//! Each case runs end to end through the `hale` binary, the top-level
//! control beside the module-nested seed: (a) the reviewer's shape, a
//! direct call into a pinned child, is refused with the cross-pool
//! error, nested as at the top level, and the nested seed with the
//! refusal too; (b) a pinned child with no cross-pool call: the control
//! builds, and the program itself reports that the child's `run()`
//! executes on a thread other than the app's; the nested seed is the
//! case the transition changed, from "builds and runs the child off
//! thread" to the refusal, by `hale check` and by `hale build`, which
//! writes no binary; (c) a `main locus` booted in a loop is refused by
//! the pinned-in-a-loop rule, nested as at the top level, and the
//! nested seed with the refusal too.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hale_nested_main_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn seed(root: &Path, name: &str, text: &str) -> PathBuf {
    let d = root.join(name);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("main.hl"), text).unwrap();
    d
}

/// `decls` inside `module inner { }`, indented.
fn in_module(decls: &str) -> String {
    let body: String = decls.lines().map(|l| format!("    {l}\n")).collect();
    format!("module inner {{\n{body}}}\n")
}

/// Runs `cmd` to completion or kills it at the deadline: (exit success,
/// stdout and stderr).
fn run_with_deadline(mut cmd: Command, secs: u64, what: &str) -> (bool, String) {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("{what}: {e}"));
    let deadline = Instant::now() + Duration::from_secs(secs);
    let status = loop {
        match child.try_wait().expect("wait") {
            Some(s) => break Some(s),
            None if Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let out = child.wait_with_output().expect("output");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let status = status.unwrap_or_else(|| panic!("{what} did not finish in {secs} s: {text}"));
    (status.success(), text)
}

fn hale(args: &[&std::ffi::OsStr], what: &str) -> (bool, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hale"));
    cmd.args(args).current_dir(Path::new("/"));
    run_with_deadline(cmd, 300, what)
}

fn check(seed: &Path) -> (bool, String) {
    hale(&["check".as_ref(), seed.as_os_str()], "hale check")
}

/// The refusal of a seed whose only `main locus`, `App`, is in `module
/// inner`.
const REFUSED: &str = "the entry must be top-level: `main locus App` inside `module inner` is not the program's \
                       entry, and nothing else in the seed is — move it out of the module";

/// The nested seed's report: `own` (the error the top-level control
/// reports) and the refusal, each once, and nothing else.
fn assert_own_error_and_the_refusal(out: &str, own: &str) {
    assert!(out.contains(own), "the nested seed reports the control's error: {out}");
    assert_eq!(out.matches(REFUSED).count(), 1, "and the refusal, once: {out}");
    assert_eq!(out.matches("type error").count(), 2, "those two alone: {out}");
}

/// The reviewer's shape: a pinned child whose method the app calls
/// directly from its own `run()`.
const DIRECT_CALL: &str = "\
locus Worker {
    fn work() { }
}

main locus App {
    params { w: Worker = Worker { }; }
    placement { w: pinned; }
    run() {
        self.w.work();
    }
}
";

const CROSS_POOL: &str = "cross-pool method call: `self.w.work` invokes a method on locus `Worker` placed `pinned (at `w`)`, but the enclosing locus `App` is placed `cooperative(pool = main)`";

#[test]
fn a_nested_mains_direct_call_into_a_pinned_child_is_refused_as_a_top_level_ones_is() {
    let root = scratch("direct");
    let flat = seed(&root, "flat", &format!("{DIRECT_CALL}\nfn main() {{ App {{ }}; }}\n"));
    let nested = seed(&root, "nested", &format!("{}\nfn main() {{ App {{ }}; }}\n", in_module(DIRECT_CALL)));
    let (ok, out) = check(&flat);
    assert!(!ok, "the top-level control is refused: {out}");
    assert!(out.contains(CROSS_POOL), "with the cross-pool error: {out}");
    assert_eq!(out.matches("type error").count(), 1, "that error alone: {out}");
    let (ok, out) = check(&nested);
    assert!(!ok, "the module-nested main is refused: {out}");
    assert_own_error_and_the_refusal(&out, CROSS_POOL);
    let _ = std::fs::remove_dir_all(&root);
}

/// A pinned child with no cross-pool call: it records the app's thread
/// at the app's params-init, and its own `run()` says whether it runs on
/// another one.
const PINNED_CHILD: &str = "\
locus Worker {
    params { app_thread: Int = 0; }
    run() {
        println(\"worker run() on a thread other than the app's: \" + to_string(pthread_self() != self.app_thread));
    }
}

main locus App {
    params { w: Worker = Worker { app_thread: pthread_self() }; }
    placement { w: pinned; }
    run() {
        println(\"app run() on the app's thread: \" + to_string(pthread_self() == self.w.app_thread));
    }
}
";

const PROBE: &str = "@ffi(\"c\") fn pthread_self() -> Int;\n\n";

#[test]
fn a_nested_main_is_refused_where_the_top_level_one_runs_its_pinned_child_off_thread() {
    let root = scratch("pinned");
    let flat = seed(&root, "flat", &format!("{PROBE}{PINNED_CHILD}\nfn main() {{ App {{ }}; }}\n"));
    let nested = seed(&root, "nested", &format!("{PROBE}{}\nfn main() {{ App {{ }}; }}\n", in_module(PINNED_CHILD)));
    // The control: the entry is deployed and spawns its pinned child.
    let (ok, out) = check(&flat);
    assert!(ok, "the top-level control checks clean: {out}");
    let bin = root.join("flat.bin");
    let (ok, out) = hale(&["build".as_ref(), flat.as_os_str(), "-o".as_ref(), bin.as_os_str()], "hale build");
    assert!(ok, "the top-level control builds: {out}");
    let (ok, out) = run_with_deadline(Command::new(&bin), 30, "the top-level control");
    assert!(ok, "the top-level control runs: {out}");
    assert!(
        out.lines().any(|l| l == "worker run() on a thread other than the app's: true"),
        "lowering deployed the entry and spawned its pinned child: {out}"
    );
    assert!(out.lines().any(|l| l == "app run() on the app's thread: true"), "{out}");
    // The case the transition changed: nothing in the nested seed is the
    // entry, so it is refused, by the refusal alone, and no binary is
    // written.
    let (ok, out) = check(&nested);
    assert!(!ok, "the module-nested main is refused: {out}");
    assert_eq!(out.matches(REFUSED).count(), 1, "by the refusal: {out}");
    assert_eq!(out.matches("type error").count(), 1, "alone: {out}");
    let bin = root.join("nested.bin");
    let (ok, out) = hale(&["build".as_ref(), nested.as_os_str(), "-o".as_ref(), bin.as_os_str()], "hale build");
    assert!(!ok, "the module-nested main does not build: {out}");
    assert_eq!(out.matches(REFUSED).count(), 1, "the build says the same: {out}");
    assert!(!bin.exists(), "no binary is written");
    let _ = std::fs::remove_dir_all(&root);
}

/// A `main locus` with a pinned field, booted once per iteration.
const PINNED_MAIN: &str = "\
locus Worker {
    run() { }
}

main locus App {
    params { w: Worker = Worker { }; }
    placement { w: pinned; }
}
";

const IN_A_LOOP: &str = "\
fn main() {
    let mut i = 0;
    while i < 3 {
        App { };
        i = i + 1;
    }
}
";

const PINNED_IN_A_LOOP: &str = "locus `App` is instantiated inside a loop, but its `placement { }` block pins field `w` to its own OS thread";

#[test]
fn a_nested_main_booted_in_a_loop_is_refused_as_a_top_level_one_is() {
    let root = scratch("loop");
    let flat = seed(&root, "flat", &format!("{PINNED_MAIN}\n{IN_A_LOOP}"));
    let nested = seed(&root, "nested", &format!("{}\n{IN_A_LOOP}", in_module(PINNED_MAIN)));
    let (ok, out) = check(&flat);
    assert!(!ok, "the top-level control is refused: {out}");
    assert!(out.contains(PINNED_IN_A_LOOP), "by the pinned-in-a-loop rule: {out}");
    assert_eq!(out.matches("type error").count(), 1, "that error alone: {out}");
    let (ok, out) = check(&nested);
    assert!(!ok, "the module-nested main is refused: {out}");
    assert_own_error_and_the_refusal(&out, PINNED_IN_A_LOOP);
    let _ = std::fs::remove_dir_all(&root);
}
