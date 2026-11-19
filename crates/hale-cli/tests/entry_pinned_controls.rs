//! The `main locus` lowering deploys is the entry, and the placement
//! rules judge it (F.40 phase 3, L4; the outside review of #1293,
//! finding 1).
//!
//! Each case runs end to end through the `hale` binary: (a) the
//! reviewer's shape, a direct call into a pinned child, is refused with
//! the cross-pool error; (b) a pinned child with no cross-pool call
//! builds, and the program itself reports that the child's `run()`
//! executes on a thread other than the app's; (c) a `main locus` booted
//! in a loop is refused by the pinned-in-a-loop rule.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hale_entry_pinned_{}_{}", std::process::id(), tag));
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
fn the_entrys_direct_call_into_a_pinned_child_is_refused() {
    let root = scratch("direct");
    let flat = seed(&root, "flat", &format!("{DIRECT_CALL}\nfn main() {{ App {{ }}; }}\n"));
    let (ok, out) = check(&flat);
    assert!(!ok, "the call is refused: {out}");
    assert!(out.contains(CROSS_POOL), "with the cross-pool error: {out}");
    assert_eq!(out.matches("type error").count(), 1, "that error alone: {out}");
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
fn the_entry_runs_its_pinned_child_off_thread() {
    let root = scratch("pinned");
    let flat = seed(&root, "flat", &format!("{PROBE}{PINNED_CHILD}\nfn main() {{ App {{ }}; }}\n"));
    let (ok, out) = check(&flat);
    assert!(ok, "it checks clean: {out}");
    let bin = root.join("flat.bin");
    let (ok, out) = hale(&["build".as_ref(), flat.as_os_str(), "-o".as_ref(), bin.as_os_str()], "hale build");
    assert!(ok, "it builds: {out}");
    let (ok, out) = run_with_deadline(Command::new(&bin), 30, "the program");
    assert!(ok, "it runs: {out}");
    assert!(
        out.lines().any(|l| l == "worker run() on a thread other than the app's: true"),
        "lowering deployed the entry and spawned its pinned child: {out}"
    );
    assert!(out.lines().any(|l| l == "app run() on the app's thread: true"), "{out}");
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
fn the_entry_booted_in_a_loop_is_refused() {
    let root = scratch("loop");
    let flat = seed(&root, "flat", &format!("{PINNED_MAIN}\n{IN_A_LOOP}"));
    let (ok, out) = check(&flat);
    assert!(!ok, "it is refused: {out}");
    assert!(out.contains(PINNED_IN_A_LOOP), "by the pinned-in-a-loop rule: {out}");
    assert_eq!(out.matches("type error").count(), 1, "that error alone: {out}");
    let _ = std::fs::remove_dir_all(&root);
}
