//! v1.x-3 PR4 — codegen integration tests for the recognition pool
//! sub-modes. Builds an Hale program that uses
//! `: projection recognition(...)`, runs it, and asserts the
//! observable lifecycle. The bar is "compiles, runs, exits zero,
//! no leaks observable at the program level" — the recpool
//! release path must not double-free or leave dangling references.

use std::process::Command;

#[path = "support/build.rs"]
mod build_opts;
#[path = "support/harness.rs"]
mod harness;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let bin = harness::unique_bin(&format!(
        "hale_test_recpool_{}_{}_{}",
        name,
        std::process::id(),
        nanos
    ));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    bin
}

#[test]
fn fixed_cell_recognition_round_trip() {
    // Three children acquired through a recognition(cap=4,
    // fixed_cell(bytes=128)) parent. Each leaf prints its value;
    // the parent sums them. The dissolve path must clear the
    // bitmap (verified indirectly via reacquire-after-release
    // in the next test).
    let src = r#"
        locus Leaf {
            params {
                value: Int = 0;
            }
            contract {
                expose value: Int;
            }
        }
        locus RecCoord : projection recognition(cap=4, fixed_cell) {
            contract { consume value: Int; }
            accept(c: Leaf) { }
            run() {
                let _l1 = Leaf { value: 1 };
                let _l2 = Leaf { value: 2 };
                let _l3 = Leaf { value: 3 };
                let mut total: Int = 0;
                for child in self.children {
                    total = total + child.value;
                }
                println("fixed_cell total=", total);
            }
        }
        fn main() {
            RecCoord { };
        }
    "#;
    let bin = build("fixed_cell_round_trip", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("fixed_cell total=6"),
        "expected fixed_cell total=6, got: {:?}",
        stdout
    );
}

#[test]
fn shared_slab_recognition_round_trip() {
    let src = r#"
        locus Leaf {
            params {
                value: Int = 0;
            }
            contract {
                expose value: Int;
            }
        }
        locus SlabCoord : projection recognition(cap=4, shared_slab) {
            contract { consume value: Int; }
            accept(c: Leaf) { }
            run() {
                let _l1 = Leaf { value: 10 };
                let _l2 = Leaf { value: 20 };
                let _l3 = Leaf { value: 30 };
                let mut total: Int = 0;
                for child in self.children {
                    total = total + child.value;
                }
                println("shared_slab total=", total);
            }
        }
        fn main() {
            SlabCoord { };
        }
    "#;
    let bin = build("shared_slab_round_trip", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("shared_slab total=60"),
        "expected shared_slab total=60, got: {:?}",
        stdout
    );
}

#[test]
fn recognition_parent_dissolve_after_children_births() {
    // Verify the parent's dissolve method fires AFTER both
    // children's birth methods. The parent's recpool destroy is
    // the last allocator teardown — a corrupted dispatch would
    // either crash (use-after-free on the slab arena, double-free
    // on a fixed cell) or skip the parent dissolve entirely. Note
    // that v0 doesn't fire user `dissolve { }` bodies on accepted
    // children (only the arena teardown via cascade), so we don't
    // assert on per-leaf dissolve printlns.
    let src = r#"
        locus Leaf {
            params {
                tag: Int = 0;
            }
            birth { println("leaf birth ", self.tag); }
        }
        locus Parent : projection recognition(cap=2, fixed_cell) {
            accept(c: Leaf) { }
            birth { println("parent birth"); }
            run() {
                let _a = Leaf { tag: 1 };
                let _b = Leaf { tag: 2 };
            }
            dissolve { println("parent dissolve"); }
        }
        fn main() {
            Parent { };
        }
    "#;
    let bin = build("lifecycle_order", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    let parent_birth_idx = lines.iter().position(|l| l.contains("parent birth"));
    let parent_dissolve_idx = lines.iter().position(|l| l.contains("parent dissolve"));
    let leaf1_birth_idx = lines.iter().position(|l| l.contains("leaf birth 1"));
    let leaf2_birth_idx = lines.iter().position(|l| l.contains("leaf birth 2"));
    assert!(
        parent_birth_idx.is_some()
            && parent_dissolve_idx.is_some()
            && leaf1_birth_idx.is_some()
            && leaf2_birth_idx.is_some(),
        "missing expected lifecycle prints; stdout was:\n{stdout}"
    );
    assert!(
        parent_birth_idx.unwrap() < leaf1_birth_idx.unwrap()
            && leaf1_birth_idx.unwrap() < parent_dissolve_idx.unwrap()
            && leaf2_birth_idx.unwrap() < parent_dissolve_idx.unwrap(),
        "lifecycle order broken; stdout was:\n{stdout}"
    );
}

#[test]
fn recognition_fixed_cell_slots_reusable_after_release() {
    // These are flow children: each run returns, release runs, then
    // the cell becomes reusable. Three births must fit through cap=1.
    let src = r#"
        locus Leaf {
            params { tag: Int = 0; }
            run() { println("leaf run ", self.tag); }
        }
        main locus Parent : projection recognition(cap=1, fixed_cell) {
            params { released: Int = 0; }
            accept(c: Leaf) { }
            release(c: Leaf) { self.released = self.released + 1; }
            run() {
                Leaf { tag: 1 };
                Leaf { tag: 2 };
                Leaf { tag: 3 };
                println("released=", self.released);
            }
        }
        fn main() { Parent { }; }
    "#;
    for mode in ["fixed_cell", "shared_slab"] {
        let bin = build(mode, &src.replace("fixed_cell", mode));
        let out = Command::new(&bin).output().expect("run");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "leaf run 1\nleaf run 2\nleaf run 3\nreleased=3\n"
        );
    }
}

const EXHAUSTION: &str = r#"
    fn child_value() -> Int { println("child params"); return 0; }
    locus Kid {
        params { value: Int = child_value(); }
        birth() { println("child birth"); }
    }
    main locus App : projection recognition(cap=1, fixed_cell) {
        params { accepted: Int = 0; }
        accept(c: Kid) {
            self.accepted = self.accepted + 1;
            println("accepted=", self.accepted);
        }
        run() {
            let first = Kid { };
            let second = Kid { };
            let third = Kid { };
            println("after all births");
        }
    }
    fn main() { App { }; }
"#;

fn assert_exhausted(source: &str, tag: &str) {
    let bin = build(tag, source);
    let out = Command::new(&bin).output().expect("run");
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "child params\naccepted=1\nchild birth\n"
    );
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(error.contains("RecognitionPoolUnavailable"), "{error}");
    assert!(
        error.contains("owner `App`") && error.contains("`Kid`"),
        "{error}"
    );
}

#[test]
fn exhausted_recognition_is_refused_before_params_accept_and_birth() {
    for mode in ["fixed_cell", "shared_slab"] {
        let source = EXHAUSTION.replace("fixed_cell", mode);
        assert_exhausted(&source, mode);
        // A sufficient reservation retains ordinary construction behavior.
        let bin = build(mode, &source.replace("cap=1", "cap=3"));
        let out = Command::new(&bin).output().expect("run");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&out.stdout), "child params\naccepted=1\nchild birth\nchild params\naccepted=2\nchild birth\nchild params\naccepted=3\nchild birth\nafter all births\n");
    }
}

#[test]
fn bubbled_children_share_the_accepting_owners_recognition_limit() {
    let source = EXHAUSTION
        .replace(
            "main locus App",
            "locus Forwarder { fn spawn() { Kid { }; } } main locus App",
        )
        .replace(
            "params { accepted: Int = 0; }",
            "params { accepted: Int = 0; forwarder: Forwarder = Forwarder { }; }",
        )
        .replace("let first = Kid { };", "self.forwarder.spawn();")
        .replace("let second = Kid { };", "self.forwarder.spawn();")
        .replace("let third = Kid { };", "self.forwarder.spawn();");
    assert_eq!(source.matches("self.forwarder.spawn()").count(), 3);
    for mode in ["fixed_cell", "shared_slab"] {
        assert_exhausted(&source.replace("fixed_cell", mode), mode);
    }
}

#[test]
fn cross_pool_births_use_the_owners_recognition_reservation() {
    let source = EXHAUSTION
        .replace("main locus App", "locus Forwarder { run() { Kid { value: 1 }; Kid { value: 2 }; Kid { value: 3 }; } } main locus App")
        .replace("params { accepted: Int = 0; }", "params { accepted: Int = 0; forwarder: Forwarder = Forwarder { }; }")
        .replace(r#"        run() {
            let first = Kid { };
            let second = Kid { };
            let third = Kid { };
            println("after all births");
        }"#, r#"        placement { forwarder: cooperative(pool = remote); }
        run() { while self.accepted < 3 { std::time::sleep(1ms); } }"#);
    assert!(source.contains("placement { forwarder:"));
    for mode in ["fixed_cell", "shared_slab"] {
        let bin = build(mode, &source.replace("fixed_cell", mode));
        let mut process = Command::new(&bin)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while process.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let timed_out = process.try_wait().unwrap().is_none();
        if timed_out {
            let _ = process.kill();
        }
        let out = process.wait_with_output().unwrap();
        assert!(
            !timed_out,
            "cross-pool create did not reach its owner: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            out.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "accepted=1\nchild birth\n"
        );
        assert!(String::from_utf8_lossy(&out.stderr).contains("RecognitionPoolUnavailable"));
        // The same dispatcher must release all admitted arenas safely at
        // owner teardown when the reservation is sufficient.
        let bin = build(
            mode,
            &source.replace("fixed_cell", mode).replace("cap=1", "cap=3"),
        );
        let mut process = Command::new(&bin)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while process.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let timed_out = process.try_wait().unwrap().is_none();
        if timed_out {
            let _ = process.kill();
        }
        let out = process.wait_with_output().unwrap();
        assert!(
            !timed_out && out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "accepted=1\nchild birth\naccepted=2\nchild birth\naccepted=3\nchild birth\n"
        );
    }
}

/// A cross-pool empty flow must complete its run and bookends before
/// the next create cell can reuse cap 1. The child's struct must also
/// return to the owner's recycler, rather than growing with history.
const CROSS_POOL_FLOW: &str = r#"
    locus Kid {
        birth() { println("birth"); }
        drain() { println("drain"); }
        dissolve() { println("dissolve"); }
    }
    locus Forwarder {
        run() { Kid { }; Kid { }; Kid { }; }
    }
    main locus App : projection recognition(cap=1, fixed_cell) {
        params { forwarder: Forwarder = Forwarder { }; accepted: Int = 0; released: Int = 0; }
        placement { forwarder: cooperative(pool = remote); }
        accept(k: Kid) { self.accepted = self.accepted + 1; println("accept"); }
        release(k: Kid) { self.released = self.released + 1; println("release"); }
        run() {
            let mut spins = 0;
            while self.accepted < 3 && spins < 1000 { std::time::sleep(1ms); spins = spins + 1; }
            println(f"accepted={self.accepted} released={self.released} live={self.children.count}");
        }
    }
    fn main() { App { }; }
"#;

#[test]
fn cross_pool_empty_flows_reclaim_and_reuse_the_accepting_owners_capacity() {
    use hale_types::lifecycle::trace;
    use hale_types::lifecycle::{ObligationKind as K, Point};
    for mode in ["fixed_cell", "shared_slab", "chunked"] {
        for owner_releases in [true, false] {
            let source = if mode == "chunked" {
                CROSS_POOL_FLOW.replace("recognition(cap=1, fixed_cell)", "chunked")
            } else {
                CROSS_POOL_FLOW.replace("fixed_cell", mode)
            };
            let source = if owner_releases {
                source
            } else {
                source.replace("release(k: Kid) { self.released = self.released + 1; println(\"release\"); }", "")
                .replace("locus Kid {", "locus MarksFlow { accept(k: Kid) { } release(k: Kid) { println(\"wrong owner\"); } } locus Kid {")
            };
            let bin = harness::unique_bin("crosspool_flow_reclaim");
            let mut options = build_opts::options();
            options.asan = true;
            options.lifecycle_trace = true;
            build_opts::build_source(&source, &bin, &options).expect("build cross-pool flow");
            let out = Command::new(&bin)
                .env(
                    "ASAN_OPTIONS",
                    if cfg!(target_os = "macos") {
                        "detect_leaks=0"
                    } else {
                        "detect_leaks=1"
                    },
                )
                .output()
                .expect("run cross-pool flow");
            let _ = std::fs::remove_file(&bin);
            assert!(
                out.status.success(),
                "{mode}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            let cycle = if owner_releases {
                "accept\nbirth\ndrain\nrelease\ndissolve\n"
            } else {
                "accept\nbirth\ndrain\ndissolve\n"
            };
            let releases = if owner_releases { 3 } else { 0 };
            assert_eq!(
                String::from_utf8_lossy(&out.stdout),
                format!("{}accepted=3 released={releases} live=0\n", cycle.repeat(3))
            );
            let trace =
                trace::parse(&String::from_utf8_lossy(&out.stderr)).expect("parse lifecycle trace");
            assert!(trace::laws(&trace, true).is_empty(), "{:?}", trace::laws(&trace, true));
            let children: std::collections::BTreeSet<_> = trace
                .events
                .iter()
                .filter(|e| e.decl.as_deref() == Some("Kid"))
                .filter_map(|e| e.subject)
                .collect();
            assert_eq!(
                children.len(),
                3,
                "three births retain distinct runtime identities even if a struct is recycled"
            );
            for child in children {
                let at = |kind, point| {
                    let events: Vec<_> = trace
                        .events
                        .iter()
                        .filter(|e| e.subject == Some(child) && e.kind == kind && e.point == point)
                        .collect();
                    assert_eq!(events.len(), 1, "{kind:?} {point:?} {child:?}");
                    assert_eq!(
                        events[0].domain, "main",
                        "bookends execute on the accepting owner's thread"
                    );
                    events[0].seq
                };
                for (first, second) in [
                    (K::Accept, K::Birth),
                    (K::Birth, K::Run),
                    (K::Run, K::Drain),
                    (K::Drain, K::Dissolve),
                ] {
                    assert!(at(first, Point::Completed) < at(second, Point::Entered));
                }
                assert!(at(K::Dissolve, Point::Completed) < at(K::Reclaim, Point::Completed));
            }
        }
    }
}
