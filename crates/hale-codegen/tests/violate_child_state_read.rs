//! Probe: can the parent's on_failure body read frozen child
//! state directly via the child handle, rather than via
//! `err.<capture_name>`? If yes, the captures-codegen gap is
//! non-blocking: users have a workaround that lands the same
//! audit-log access without growing the ClosureViolation type.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("lotus_test_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

#[test]
fn parent_reads_child_state_after_violate() {
    let src = r#"
locus Child {
    params { last_error: String = "default"; }
    closure fatal_io { captures: last_error; epoch inline; }
    fn step() {
        self.last_error = "send_failed";
        violate fatal_io;
    }
}

locus Parent {
    accept(c: Child) { }
    on_failure(c: Child, err: ClosureViolation) {
        println("closure=", err.closure, " detail=", c.last_error);
    }
    run() {
        let c = Child { };
        c.step();
    }
}

fn main() { Parent { }; }
"#;
    let (stdout, status) = build_and_run("violate_child_state", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("closure=fatal_io detail=send_failed"),
        "expected parent to read child's frozen state; got: {:?}",
        stdout
    );
}
