//! A main locus's tree drain (spec/runtime.md § "Lifecycle obligations",
//! line 15; friction sweep 2026-10-12, downstream handoff): teardown
//! follows the ownership tree. Once the main locus's `run()` has
//! returned, the first step of its teardown raises the drain for its
//! tree, so a child whose `run()` loops on `while !self.draining` — on a
//! pinned thread, a classic pool or an `async_io` pool — sees it, returns
//! and is joined. Before, the pinned and classic children's loops held
//! the process forever after main's `run()` returned.
//!
//! The drain lasts as long as the tree: a main locus built after it
//! starts undrained, and a main locus torn down at its fn's exit (it
//! subscribes, so its teardown is deferred) drains its tree only then.

use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

#[path = "support/build.rs"]
mod build_opts;
#[path = "support/harness.rs"]
mod harness;

const SLEEPER: &str = r#"
    locus Sleeper {
        params { tag: String = ""; laps: Int = 0; }
        run() {
            while !self.draining { self.laps = self.laps + 1; std::time::sleep(20ms); }
            println(self.tag + " saw the drain after " + to_string(self.laps) + " laps");
        }
        drain() { println(self.tag + " drain"); }
    }
"#;

/// Run `bin` to its end, or fail once `secs` have passed (the process
/// is killed: a child that never saw the drain holds it forever).
fn run_bounded(bin: &std::path::Path, secs: u64, env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(bin);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn");
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if child.try_wait().expect("try_wait").is_some() {
            return child.wait_with_output().expect("output");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let out = child.wait_with_output().expect("output");
            panic!(
                "still running after {secs}s: a child's run() never saw the drain\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn lines(out: &Output) -> Vec<String> {
    String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect()
}

fn pos(lines: &[String], prefix: &str) -> usize {
    lines
        .iter()
        .position(|l| l.starts_with(prefix))
        .unwrap_or_else(|| panic!("no `{prefix}` line in:\n{}", lines.join("\n")))
}

fn laps(lines: &[String], tag: &str) -> i64 {
    let l = &lines[pos(lines, &format!("{tag} saw the drain"))];
    l.rsplit(' ').nth(1).and_then(|n| n.parse().ok()).expect("a lap count")
}

fn three_placements() -> String {
    format!(
        "{SLEEPER}
        main locus App {{
            params {{
                p: Sleeper = Sleeper {{ tag: \"pinned\" }};
                c: Sleeper = Sleeper {{ tag: \"classic\" }};
                a: Sleeper = Sleeper {{ tag: \"async\" }};
            }}
            placement {{ p: pinned; c: cooperative(pool = cls); a: cooperative(pool = aio) where async_io; }}
            run() {{ std::time::sleep(100ms); println(\"main run done\"); }}
        }}
        fn main() {{ App {{ }}; println(\"after\"); }}
        "
    )
}

fn assert_tree_drained(out: &Output) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{:?}\nstderr:\n{stderr}", out.status);
    let l = lines(out);
    let main_done = pos(&l, "main run done");
    for tag in ["pinned", "classic", "async"] {
        let saw = pos(&l, &format!("{tag} saw the drain"));
        assert!(saw > main_done, "{tag} saw a drain before main's run() returned:\n{}", l.join("\n"));
        assert!(pos(&l, &format!("{tag} drain")) > saw, "{}", l.join("\n"));
        assert!(laps(&l, tag) >= 1, "{}", l.join("\n"));
    }
    assert_eq!(l.last().map(String::as_str), Some("after"), "{}", l.join("\n"));
}

#[test]
fn main_run_returning_drains_its_children_on_every_placement() {
    let bin = harness::unique_bin("tree_drain_three");
    build_opts::build_source(&three_placements(), &bin, &build_opts::options()).expect("build");
    let out = run_bounded(&bin, 20, &[]);
    let _ = std::fs::remove_file(&bin);
    assert_tree_drained(&out);
}

#[test]
fn main_run_returning_drains_its_children_under_asan() {
    let bin = harness::unique_bin("tree_drain_three_asan");
    harness::build_source_asan(&three_placements(), &bin);
    let out = run_bounded(&bin, 60, &[("LOTUS_NO_CHUNK_POOL", "1")]);
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    for bad in ["AddressSanitizer", "LeakSanitizer", "SUMMARY:"] {
        assert!(!stderr.contains(bad), "{bad} in:\n{stderr}");
    }
    assert_tree_drained(&out);
}

/// A main locus built after another starts undrained: the first tree's
/// drain ended with its teardown, so the second's child runs its laps
/// until the second main's own `run()` returns.
#[test]
fn a_main_locus_built_after_a_drained_one_starts_undrained() {
    let src = format!(
        "{SLEEPER}
        main locus App {{
            params {{ p: Sleeper = Sleeper {{ }}; }}
            placement {{ p: pinned; }}
            run() {{ std::time::sleep(200ms); }}
        }}
        fn main() {{
            App {{ p: Sleeper {{ tag: \"first\" }} }};
            App {{ p: Sleeper {{ tag: \"second\" }} }};
        }}
        "
    );
    let bin = harness::unique_bin("tree_drain_twice");
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let out = run_bounded(&bin, 20, &[]);
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "{:?}\n{}", out.status, String::from_utf8_lossy(&out.stderr));
    let l = lines(&out);
    assert!(laps(&l, "first") >= 3, "{}", l.join("\n"));
    assert!(laps(&l, "second") >= 3, "the second tree started drained:\n{}", l.join("\n"));
}

/// A main locus that subscribes is torn down at its fn's exit (the
/// deferred spine): its tree is drained then, not where its `run()`
/// returned, and the program still ends.
#[test]
fn a_deferred_main_locus_drains_its_tree_at_its_teardown() {
    let src = format!(
        "{SLEEPER}
        type Note {{ n: Int; }}
        main locus App {{
            params {{ p: Sleeper = Sleeper {{ tag: \"pinned\" }}; }}
            placement {{ p: pinned; }}
            bus {{ publish \"tree.note\" of type Note; subscribe \"tree.note\" as on_note of type Note; }}
            fn on_note(n: Note) {{ }}
            run() {{ std::time::sleep(50ms); println(\"main run done\"); }}
        }}
        fn main() {{
            App {{ }};
            std::time::sleep(100ms);
            println(\"fn main ends\");
        }}
        "
    );
    let bin = harness::unique_bin("tree_drain_deferred");
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let out = run_bounded(&bin, 20, &[]);
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "{:?}\n{}", out.status, String::from_utf8_lossy(&out.stderr));
    let l = lines(&out);
    assert!(pos(&l, "pinned saw the drain") > pos(&l, "fn main ends"), "{}", l.join("\n"));
}
