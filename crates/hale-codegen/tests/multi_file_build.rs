//! Per-directory seed model — top-level decls in any .hl file
//! are visible to every other file in the same seed (one binary).
//! This is the regression for the dir-seeds milestone (resolves
//! `notes/hale-friction.md` 2026-05-10 single-file-app-monolith).

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

#[test]
fn cross_file_fn_call() {
    // Two files in one seed; main.hl calls helpers defined in
    // helpers.hl. Ordering matters only for the merge order
    // (helpers.hl sorts before main.hl alphabetically); the
    // typechecker's flat scope makes resolution order-free.
    let helpers = r#"
        fn say(s: String) { println("seed says: ", s); }
    "#;
    let main = r#"
        locus AppL {
            params { msg: String = "from main"; }
            run() { say(self.msg); }
        }
        fn main() { AppL { }; }
    "#;
    let dir = harness::unique_dir("hale_multi_file_cross_fn");
    std::fs::write(dir.join("helpers.hl"), helpers).expect("write helpers.hl");
    std::fs::write(dir.join("main.hl"), main).expect("write main.hl");
    let bin = dir.join("app");
    build_opts::build_seed_dir(&dir, &bin, &build_opts::options()).expect("build merged");

    let out = Command::new(&bin).output().expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(out.status.success(), "binary exited non-zero: {:?}", out.status);
    assert!(
        stdout.contains("seed says: from main"),
        "expected seed message in output, got: {:?}",
        stdout
    );
}

#[test]
fn cross_file_locus_referenced() {
    // Helpers file declares a type and a fn that returns it;
    // main constructs the type via the helper. Tests that
    // user-defined types in one file are resolvable from another.
    let helpers = r#"
        type Config { who: String; n: Int; }
        fn make_config() -> Config {
            return Config { who: "world", n: 7 };
        }
    "#;
    let main = r#"
        locus AppL {
            params { }
            run() {
                let c = make_config();
                println("who=", c.who, " n=", c.n);
            }
        }
        fn main() { AppL { }; }
    "#;
    let dir = harness::unique_dir("hale_multi_file_cross_type");
    std::fs::write(dir.join("helpers.hl"), helpers).expect("write helpers.hl");
    std::fs::write(dir.join("main.hl"), main).expect("write main.hl");
    let bin = dir.join("app");
    build_opts::build_seed_dir(&dir, &bin, &build_opts::options()).expect("build merged");

    let out = Command::new(&bin).output().expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(out.status.success(), "binary exited non-zero: {:?}", out.status);
    assert!(
        stdout.contains("who=world n=7"),
        "expected combined output from cross-file type + fn, got: {:?}",
        stdout
    );
}
