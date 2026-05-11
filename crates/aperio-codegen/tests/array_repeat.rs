//! Phase 2d — `[val; N]` array-literal repetition.
//!
//! Closes the last sub-bullet of notes/aperio-friction.md
//! 2026-05-10 float-surface-gaps: "No Float array-literal
//! repetition syntax (`[0.0; 8]`). Array defaults must enumerate
//! every element."

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_arep_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn float_array_repeat() {
    let src = r#"
        fn main() {
            let r: [Float; 4] = [0.0; 4];
            println("len4 first=", r[0], " last=", r[3]);
        }
    "#;
    let (stdout, status) = build_and_run("float", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("len4 first=0 last=0"), "got: {:?}", stdout);
}

#[test]
fn int_array_repeat() {
    let src = r#"
        fn main() {
            let r: [Int; 5] = [42; 5];
            println("v0=", r[0], " v4=", r[4]);
        }
    "#;
    let (stdout, status) = build_and_run("int", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("v0=42 v4=42"), "got: {:?}", stdout);
}

#[test]
fn repeat_evaluates_val_once() {
    // Sanity: val is evaluated exactly once. If it were
    // evaluated N times, the side-effect would print twice.
    // (Aperio side-effects are bounded; this test uses a fn
    // call with stdout side-effect to verify.)
    let src = r#"
        fn one_and_log() -> Int {
            println("computed");
            return 7;
        }
        fn main() {
            let r: [Int; 3] = [one_and_log(); 3];
            println("sum=", r[0] + r[1] + r[2]);
        }
    "#;
    let (stdout, status) = build_and_run("once", src);
    assert!(status.success(), "exit: {:?}", status);
    // "computed" should print exactly once.
    let occurrences = stdout.matches("computed").count();
    assert_eq!(occurrences, 1, "val should evaluate once; got: {:?}", stdout);
    assert!(stdout.contains("sum=21"), "got: {:?}", stdout);
}

#[test]
fn repeat_indexable_at_runtime() {
    let src = r#"
        fn main() {
            let r: [Int; 8] = [0; 8];
            let mut i = 0;
            let mut total = 0;
            while i < 8 {
                total = total + r[i];
                i = i + 1;
            }
            println("total=", total);
        }
    "#;
    let (stdout, status) = build_and_run("index", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("total=0"), "got: {:?}", stdout);
}
