//! v1.x-15: string-builder primitive (`std::str::builder_*`).
//!
//! Resolves the `reader-list_item-quadratic-concat` friction —
//! a doubling-realloc-backed buffer turns N appends into amortized
//! O(N) total cost instead of the O(N²) shape that arena-anchored
//! immutable Strings collapsed to.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_strbuilder_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn builder_append_then_finish_round_trip() {
    let src = r#"
        fn main() {
            let b = std::str::builder_new();
            std::str::builder_append(b, "hello ");
            std::str::builder_append(b, "world");
            std::str::builder_append(b, "!");
            let s = std::str::builder_finish(b);
            println(s);
        }
    "#;
    let (stdout, status) = build_and_run("round_trip", src);
    assert!(status.success());
    assert!(
        stdout.contains("hello world!"),
        "got: {:?}",
        stdout
    );
}

#[test]
fn builder_len_tracks_running_size() {
    let src = r#"
        fn main() {
            let b = std::str::builder_new();
            let n0 = std::str::builder_len(b);
            std::str::builder_append(b, "abc");
            let n1 = std::str::builder_len(b);
            std::str::builder_append(b, "defg");
            let n2 = std::str::builder_len(b);
            println(f"{n0} {n1} {n2}");
        }
    "#;
    let (stdout, status) = build_and_run("len_tracks", src);
    assert!(status.success());
    assert!(stdout.contains("0 3 7"), "got: {:?}", stdout);
}

#[test]
fn builder_handles_many_small_appends_no_blow_up() {
    // The whole point of the primitive: 1000 small appends should
    // build a 1000-char string without performance issues. This
    // test asserts correctness, not perf (the binary returns fast),
    // but a quadratic implementation would still finish on this size
    // — the perf win is for the friction-source case (thousands of
    // continuation lines per ferryman node). Here we're verifying
    // the doubling-realloc actually concatenates the right bytes.
    let src = r#"
        fn main() {
            let b = std::str::builder_new();
            let mut i = 0;
            while i < 1000 {
                std::str::builder_append(b, "x");
                i = i + 1;
            }
            let s = std::str::builder_finish(b);
            let n = len(s);
            println(f"len={n}");
        }
    "#;
    let (stdout, status) = build_and_run("many_appends", src);
    assert!(status.success());
    assert!(stdout.contains("len=1000"), "got: {:?}", stdout);
}

#[test]
fn builder_empty_finish_returns_empty_string() {
    let src = r#"
        fn main() {
            let b = std::str::builder_new();
            let s = std::str::builder_finish(b);
            let n = len(s);
            println(f"len={n}");
        }
    "#;
    let (stdout, status) = build_and_run("empty", src);
    assert!(status.success());
    assert!(stdout.contains("len=0"), "got: {:?}", stdout);
}

#[test]
fn builder_appends_can_be_interleaved_with_other_work() {
    // The builder shouldn't be perturbed by surrounding code —
    // function calls, intermediate strings, etc. shouldn't leak
    // into its buffer.
    let src = r#"
        fn say(s: String) -> String {
            return s + "!";
        }
        fn main() {
            let b = std::str::builder_new();
            std::str::builder_append(b, "a");
            let unrelated = say("b");
            std::str::builder_append(b, "c");
            let unrelated2 = say("d");
            std::str::builder_append(b, "e");
            let out = std::str::builder_finish(b);
            println(out);
        }
    "#;
    let (stdout, status) = build_and_run("interleaved", src);
    assert!(status.success());
    assert!(stdout.contains("ace"), "got: {:?}", stdout);
}
