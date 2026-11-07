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

/// A main locus with no `run()` whose children do the work (downstream
/// handoff, 2026-11-06): a pinned subscriber declared first, a pinned
/// publisher declared after it whose `run()` publishes three records,
/// sleeps, publishes a trigger, sleeps and returns. The main locus's
/// teardown used to join its pinned children in declaration order, so
/// the subscriber's mailbox was shut down while the publisher was still
/// running and every delivery was lost: the program printed nothing and
/// exited 0. The record names are heap values (`std::str::upper`), so the
/// ASan twin reads them from memory a wrong teardown order would have
/// freed.
const STORE_AND_PUB: &str = r#"
    type Rec { name: String = ""; }
    type Go { n: Int; }
    topic Recs { payload: Rec; subject: "recs"; }
    topic Gos { payload: Go; subject: "gos"; }
    locus Store {
        params { items: bounded[Rec; 16]; }
        bus { subscribe Recs as on_rec; subscribe Gos as on_go; }
        fn on_rec(r: Rec) { push(self.items, r) or discard; }
        fn on_go(g: Go) {
            for x in self.items { println("stored=" + x.name); }
            println("count=" + to_string(count(self.items)));
        }
    }
    locus Pub {
        bus { publish Recs; publish Gos; }
        run() {
            let mut i = 0;
            while i < 3 { Recs <- Rec { name: std::str::upper("rec") + to_string(i) }; i = i + 1; }
            std::time::sleep(100ms);
            Gos <- Go { n: 1 };
            std::time::sleep(100ms);
            println("pub run done");
        }
    }
    main locus App {
        params { s: Store = Store { }; p: Pub = Pub { }; }
        placement { s: pinned; p: pinned; }
    }
    fn main() { App { }; println("after"); }
"#;

fn assert_three_deliveries(out: &Output) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{:?}\nstderr:\n{stderr}", out.status);
    let l = lines(out);
    for i in 0..3 {
        pos(&l, &format!("stored=REC{i}"));
    }
    pos(&l, "count=3");
    pos(&l, "pub run done");
    assert_eq!(l.last().map(String::as_str), Some("after"), "{}", l.join("\n"));
}

#[test]
fn a_main_with_no_run_delivers_what_its_pinned_children_publish() {
    let bin = harness::unique_bin("tree_drain_no_run_pinned");
    build_opts::build_source(STORE_AND_PUB, &bin, &build_opts::options()).expect("build");
    let out = run_bounded(&bin, 20, &[]);
    let _ = std::fs::remove_file(&bin);
    assert_three_deliveries(&out);
}

#[test]
fn a_main_with_no_run_delivers_what_its_pinned_children_publish_under_asan() {
    let bin = harness::unique_bin("tree_drain_no_run_pinned_asan");
    harness::build_source_asan(STORE_AND_PUB, &bin);
    let out = run_bounded(&bin, 60, &[("LOTUS_NO_CHUNK_POOL", "1")]);
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    for bad in ["AddressSanitizer", "LeakSanitizer", "SUMMARY:"] {
        assert!(!stderr.contains(bad), "{bad} in:\n{stderr}");
    }
    assert_three_deliveries(&out);
}

/// The same program, with a `draining` read the publisher's `run()` does
/// not make: one in its `drain()` hook, one in a method nothing calls. A
/// read outside `run()` (and what it calls) does not make the run one the
/// drain ends, so the main locus still waits for it before its tree's
/// drain (GH #1858; the #1869 review's finding 2: the read in the hook
/// made the publisher "watch" the drain, the main locus did not wait for
/// it, and every delivery was lost again, silently).
#[test]
fn a_draining_read_outside_run_does_not_skip_the_wait_for_it() {
    let extra = "        drain() { if self.draining { println(\"pub saw it\"); } println(\"pub drain hook\"); }\n\
                 \x20       fn stopping() -> Bool { return self.draining; }\n";
    let src = STORE_AND_PUB.replacen(
        "            println(\"pub run done\");\n        }\n",
        &format!("            println(\"pub run done\");\n        }}\n{extra}"),
        1,
    );
    assert!(src.contains("pub drain hook"), "the hook is in the program");
    let bin = harness::unique_bin("tree_drain_hook_reads");
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let out = run_bounded(&bin, 20, &[]);
    let _ = std::fs::remove_file(&bin);
    assert_three_deliveries(&out);
    let l = lines(&out);
    assert!(pos(&l, "pub run done") < pos(&l, "pub drain hook"), "{}", l.join("\n"));
}

/// The same main locus with a pinned worker that does not read
/// `draining` beside a pinned child that loops on it: the tree's drain
/// begins once the worker's `run()` has returned, so the loop sees it
/// after the worker is done — and still sees it, so the program ends.
/// Alone, a child that reads `draining` sees the drain at once (the
/// shape before this: `loop out` printed first).
/// The watches-drain column reaches the read through any receiver that
/// is the locus by declared type, not only a literal `self` (the #1869
/// review's follow-ups, 2026-11-07): each `Worker` here loops until the
/// drain, so a column that missed its read would have the no-run() main
/// await a run() only the drain can end — the program would loop until
/// the 20 s bound. With the read seen the child is not awaited and the
/// program ends on its own under the tree drain: a `let` alias of `self`
/// calling a method, the alias handed to a free fn, `self` handed to
/// another locus's method that reads its parameter, a holder built with
/// `target: self` whose method reads `self.target.draining`, a two-level
/// holder chain, and an alias an inner block shadows (with a control
/// that renames the inner binding).
#[test]
fn a_run_reading_draining_through_an_alias_an_argument_or_a_field_is_not_awaited() {
    let main = "main locus App {
            params { w: Worker = Worker { }; }
            placement { w: pinned(core = 0); }
        }
        fn main() { App { }; println(\"after\"); }
        ";
    let shapes: [(&str, &str); 7] = [
        ("alias_method", "locus Worker {
            fn stopped() -> Bool { return self.draining; }
            run() { let w = self; while !w.stopped() { std::time::sleep(10ms); } println(\"worker out\"); }
        }
        "),
        ("free_fn_alias", "fn stopped(w: Worker) -> Bool { return w.draining; }
        locus Worker {
            run() { let w = self; while !stopped(w) { std::time::sleep(10ms); } println(\"worker out\"); }
        }
        "),
        ("watch_arg", "locus Watch {
            fn stopped(w: Worker) -> Bool { return w.draining; }
        }
        locus Worker {
            params { watcher: Watch = Watch { }; }
            run() { while !self.watcher.stopped(self) { std::time::sleep(10ms); } println(\"worker out\"); }
        }
        "),
        ("field", "locus Watch {
            params { target: Worker; }
            fn stopped() -> Bool { return self.target.draining; }
        }
        locus Worker {
            run() {
                let watcher = Watch { target: self };
                while !watcher.stopped() { std::time::sleep(10ms); }
                println(\"worker out\");
            }
        }
        "),
        ("field_chain", "locus Holder {
            params { target: Worker; }
        }
        locus Watch {
            params { holder: Holder; }
            fn stopped() -> Bool { return self.holder.target.draining; }
        }
        locus Worker {
            run() {
                let watcher = Watch { holder: Holder { target: self } };
                while !watcher.stopped() { std::time::sleep(10ms); }
                println(\"worker out\");
            }
        }
        "),
        ("shadow", "locus Other { params { n: Int = 0; } }
        locus Worker {
            run() {
                let w = self;
                if true { let w = Other { }; println(w.n); }
                while !w.draining { std::time::sleep(10ms); }
                println(\"worker out\");
            }
        }
        "),
        ("shadow_control", "locus Other { params { n: Int = 0; } }
        locus Worker {
            run() {
                let w = self;
                if true { let v = Other { }; println(v.n); }
                while !w.draining { std::time::sleep(10ms); }
                println(\"worker out\");
            }
        }
        "),
    ];
    for (tag, loci) in shapes {
        let src = format!("{loci}{main}");
        let bin = harness::unique_bin(&format!("tree_drain_reads_{tag}"));
        build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
        let out = run_bounded(&bin, 20, &[]);
        let _ = std::fs::remove_file(&bin);
        assert!(out.status.success(), "{tag}: {:?}\n{}", out.status, String::from_utf8_lossy(&out.stderr));
        let l = lines(&out);
        assert!(pos(&l, "worker out") < pos(&l, "after"), "{tag}: the drain ended the loop:\n{}", l.join("\n"));
    }
}

/// The column's other side: a holder's method that reads only its own
/// `self.draining` — the holder declaring a `Worker` field, or handed the
/// `Worker` as an argument — reads another locus's drain, so the finite
/// `Worker` does not watch and is awaited: `worker done` before `after`,
/// its sleeps uncut.
#[test]
fn a_run_reading_only_another_locus_s_draining_is_awaited() {
    let main = "main locus App {
            params { w: Worker = Worker { }; }
            placement { w: pinned(core = 0); }
        }
        fn main() { App { }; println(\"after\"); }
        ";
    let shapes: [(&str, &str); 2] = [
        ("field_self", "locus Watch {
            params { target: Worker; }
            fn stopped() -> Bool { return self.draining; }
        }
        locus Worker {
            run() {
                let watcher = Watch { target: self };
                let mut i = 0;
                while i < 3 && !watcher.stopped() { i = i + 1; std::time::sleep(20ms); }
                println(\"worker done\");
            }
        }
        "),
        ("arg_self", "locus Watch {
            fn stopped(w: Worker) -> Bool { return self.draining; }
        }
        locus Worker {
            params { watcher: Watch = Watch { }; }
            run() {
                let mut i = 0;
                while i < 3 && !self.watcher.stopped(self) { i = i + 1; std::time::sleep(20ms); }
                println(\"worker done\");
            }
        }
        "),
    ];
    for (tag, loci) in shapes {
        let src = format!("{loci}{main}");
        let bin = harness::unique_bin(&format!("tree_drain_reads_{tag}"));
        build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
        let out = run_bounded(&bin, 20, &[]);
        let _ = std::fs::remove_file(&bin);
        assert!(out.status.success(), "{tag}: {:?}\n{}", out.status, String::from_utf8_lossy(&out.stderr));
        let l = lines(&out);
        assert!(pos(&l, "worker done") < pos(&l, "after"), "{tag}: the finite worker was awaited:\n{}", l.join("\n"));
    }
}

#[test]
fn a_main_with_no_run_drains_its_tree_once_its_workers_have_returned() {
    let src = format!(
        "{SLEEPER}
        locus Worker {{
            run() {{ std::time::sleep(150ms); println(\"worker done\"); }}
        }}
        main locus App {{
            params {{ l: Sleeper = Sleeper {{ tag: \"pinned\" }}; w: Worker = Worker {{ }}; }}
            placement {{ l: pinned; w: pinned; }}
        }}
        fn main() {{ App {{ }}; println(\"after\"); }}
        "
    );
    let bin = harness::unique_bin("tree_drain_no_run_worker");
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let out = run_bounded(&bin, 20, &[]);
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "{:?}\n{}", out.status, String::from_utf8_lossy(&out.stderr));
    let l = lines(&out);
    assert!(pos(&l, "worker done") < pos(&l, "pinned saw the drain"), "{}", l.join("\n"));
    assert!(laps(&l, "pinned") >= 3, "the loop was drained before the worker returned:\n{}", l.join("\n"));
    assert_eq!(l.last().map(String::as_str), Some("after"), "{}", l.join("\n"));

    // alone, the loop sees the drain at once and the program ends
    let src = format!(
        "{SLEEPER}
        main locus App {{
            params {{ l: Sleeper = Sleeper {{ tag: \"pinned\" }}; }}
            placement {{ l: pinned; }}
        }}
        fn main() {{ App {{ }}; println(\"after\"); }}
        "
    );
    let bin = harness::unique_bin("tree_drain_no_run_alone");
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let out = run_bounded(&bin, 20, &[]);
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "{:?}\n{}", out.status, String::from_utf8_lossy(&out.stderr));
    let l = lines(&out);
    pos(&l, "pinned saw the drain");
    assert_eq!(l.last().map(String::as_str), Some("after"), "{}", l.join("\n"));
}
