//! a downstream tool F.4 — forward-refs between sibling field defaults.
//!
//! Pre-fix: `dispatcher: ProposalDispatcher = ProposalDispatcher
//! { gate: self.gate }` errored with "self.gate read outside a
//! locus method" because `current_self` was None during
//! params-init expression lowering. The recursive instantiation
//! set `params_init_self` to THIS locus (ProposalDispatcher),
//! not the caller (A downstream tool), so a `self.X` reference inside an
//! override couldn't see the caller's earlier-declared sibling
//! field.
//!
//! Post-fix: `Expr::Field` resolution for `self.X` falls back
//! to `params_init_self` when `current_self` is None; AND the
//! params-init override-expr lowering temporarily restores the
//! OUTER's params_init_self (saved as prev_params_init_self) so
//! `self.X` resolves against the CALLER, not this locus.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "lt-sibling-fwd-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path(name);
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

#[test]
fn override_self_x_resolves_to_outer_params_init() {
    // Workbench-shape: parent has `gate` and `dispatcher` fields;
    // dispatcher's default holds a borrow of gate via
    // `gate: self.gate`. Before the fix, self.gate failed to
    // resolve during ProposalDispatcher's instantiation. After:
    // resolves to the parent's already-stored gate field.
    let src = r#"
        locus PermissionGate {
            params { tag: String = "g"; }
        }
        locus ProposalDispatcher {
            params { gate: PermissionGate = PermissionGate { }; }
            run() { println("dispatcher tag=", self.gate.tag); }
        }
        main locus Workbench {
            params {
                gate: PermissionGate = PermissionGate { tag: "demo-gate" };
                dispatcher: ProposalDispatcher
                    = ProposalDispatcher { gate: self.gate };
            }
        }
        fn main() { Workbench { }; }
    "#;
    let (stdout, status) = build_and_run("borrow", src);
    assert!(
        status.success(),
        "binary exited non-zero: {:?}\nstdout: {}",
        status,
        stdout
    );
    assert!(
        stdout.contains("dispatcher tag=demo-gate"),
        "expected 'dispatcher tag=demo-gate' (proves dispatcher \
         received the parent's gate field, not a freshly-defaulted \
         one); got: {}",
        stdout
    );
}

#[test]
fn self_x_in_default_expr_reads_earlier_sibling() {
    // Direct sibling-field forward-ref in a DEFAULT expression
    // (not just via a nested override). `b`'s default reads
    // `self.a` directly — params_init_self carries the
    // already-initialized `a` slot.
    let src = r#"
        main locus L {
            params {
                a: Int = 10;
                b: Int = 5 + self.a;
            }
            run() {
                println("a=", self.a, " b=", self.b);
            }
        }
        fn main() { L { }; }
    "#;
    let (stdout, status) = build_and_run("direct", src);
    assert!(
        status.success(),
        "binary exited non-zero: {:?}\nstdout: {}",
        status,
        stdout
    );
    assert!(stdout.contains("a=10 b=15"), "got: {:?}", stdout);
}
