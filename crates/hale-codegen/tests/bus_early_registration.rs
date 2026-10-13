//! A parent hears what its params' children publish while they are built
//! (spec/runtime.md § "Lifecycle obligations", line 6; friction sweep
//! 2026-10-12, downstream handoff). A cooperative instance whose
//! subscriptions take no key registers them, its readiness window open,
//! before its params are built: a cell a child's `birth()` or inline
//! `run()` publishes to it parks there and is delivered once its own
//! `birth()` has completed. Before, it registered after its params, and
//! the cell reached no subscription of it: "got 0".

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const CHILDREN_PUBLISH_TO_THEIR_PARENT: &str = r#"
    type D { n: Int; }
    topic Done { payload: D; }
    locus Born {
        params { x: Int = 0; }
        bus { publish Done; }
        birth() { Done <- D { n: 7 }; }
    }
    locus Ran {
        params { x: Int = 0; }
        bus { publish Done; }
        run() { Done <- D { n: 30 }; }
    }
    main locus App {
        params { b: Born = Born { }; r: Ran = Ran { }; got: Int = 0; born: Bool = false; early: Int = 0; }
        bus { subscribe Done as on_done; }
        birth() { self.born = true; }
        fn on_done(d: D) {
            if !self.born { self.early = self.early + 1; }
            self.got = self.got + d.n;
        }
        run() {
            yield;
            println("got " + to_string(self.got) + " before birth " + to_string(self.early));
        }
    }
    fn main() { App { }; }
"#;

fn run(tag: &str, asan: bool) -> String {
    let bin = harness::unique_bin(tag);
    if asan {
        harness::build_source_asan(CHILDREN_PUBLISH_TO_THEIR_PARENT, &bin);
    } else {
        build_opts::build_source(CHILDREN_PUBLISH_TO_THEIR_PARENT, &bin, &build_opts::options()).expect("build");
    }
    let out = Command::new(&bin).env("LOTUS_NO_CHUNK_POOL", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    for bad in ["AddressSanitizer", "LeakSanitizer", "SUMMARY:"] {
        assert!(!stderr.contains(bad), "{bad} in:\n{stderr}");
    }
    assert!(out.status.success(), "{:?}\n{stdout}\n{stderr}", out.status);
    stdout
}

#[test]
fn a_parent_hears_its_childrens_birth_and_inline_run_after_its_own_birth() {
    // 7 from the child's birth, 30 from the other's inline run(), both
    // delivered once the parent's birth() completed, none before it
    let out = run("early_registration", false);
    assert_eq!(out.trim(), "got 37 before birth 0", "{out}");
}

#[test]
fn a_parent_hears_its_children_under_asan() {
    let out = run("early_registration_asan", true);
    assert_eq!(out.trim(), "got 37 before birth 0", "{out}");
}
