//! Integration test: build hello-world via the codegen path
//! and run the produced binary. This is the milestone-0
//! end-to-end gate — if it passes, the LLVM toolchain is
//! wired correctly for the simplest lotus program.

use std::path::PathBuf;
use std::process::Command;

use lotus_codegen::build_executable;

fn examples_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("examples");
    p
}

#[test]
fn hello_world_builds_and_runs() {
    let mut src_path = examples_dir();
    src_path.push("hello-world");
    src_path.push("main.lt");
    let source = std::fs::read_to_string(&src_path).expect("read source");
    let program = lotus_syntax::parse_source(&source).expect("parse");

    // Use a temp file so the test is hermetic and doesn't
    // collide with any existing binary in examples/.
    let temp_dir = std::env::temp_dir();
    let mut bin_path = temp_dir.clone();
    bin_path.push("lotus_test_hello_world");

    build_executable(&program, &bin_path).expect("build");

    let output = Command::new(&bin_path)
        .output()
        .expect("run produced binary");
    let _ = std::fs::remove_file(&bin_path);

    assert!(
        output.status.success(),
        "binary exited non-zero: {:?}",
        output.status
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("hello, world"),
        "expected greeting in stdout; got: {:?}",
        stdout
    );
}
