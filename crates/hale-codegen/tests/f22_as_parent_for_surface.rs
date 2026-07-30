//! v1.x-4 (surface) + v1.x-4b (runtime): F.22
//! `as_parent_for ChildL` slot trailing clause. The parser
//! shipped in v1.x-4; the typecheck (cross-locus validation)
//! shipped in v1.x-4; the runtime mechanic — copy parent's
//! allocator pointer into the child's same-named slot at
//! instantiation, skip-destroy on borrowed slots at the child's
//! dissolve — shipped in v1.x-4b.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_f22_apf_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

/// A well-formed `as_parent_for` declaration compiles end-to-end
/// and runs cleanly. The parent owns the allocator; the child
/// borrows it at instantiation; both dissolve without leaking
/// (verified by clean process exit — a double-free or leak in
/// the child's dissolve would surface as a non-zero exit / ASAN
/// trip in debug builds).
#[test]
fn as_parent_for_compiles_and_runs() {
    let src = r#"
        locus ChildL {
            capacity { pool entries of Int; }
        }
        locus ParentL {
            capacity {
                pool entries of Int as_parent_for ChildL;
            }
            accept(c: ChildL) {
                println("accept");
            }
            run() {
                ChildL { };
                println("ok");
            }
        }
        fn main() {
            ParentL { };
        }
    "#;
    let bin = build("compiles_and_runs", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("accept"), "missing accept: {:?}", stdout);
    assert!(stdout.contains("ok"), "missing ok: {:?}", stdout);
}

/// Typecheck rejects `as_parent_for NonExistentL` when the
/// referenced locus doesn't exist. The diagnostic mentions the
/// missing locus name.
#[test]
fn as_parent_for_typecheck_rejects_unknown_locus() {
    let src = r#"
        locus ParentL {
            capacity {
                pool entries of Int as_parent_for NonExistentL;
            }
        }
        fn main() { }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let diags = hale_types::check_program(&program);
    let joined: String = diags
        .iter()
        .map(|d| format!("{:?}", d))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        diags.iter().any(|d| format!("{:?}", d).contains("NonExistentL")),
        "expected diagnostic mentioning NonExistentL, got: {}",
        joined
    );
}

/// Typecheck rejects when the named child has no slot with the
/// matching name.
#[test]
fn as_parent_for_typecheck_rejects_mismatched_slot() {
    let src = r#"
        locus ChildL {
            capacity { pool other of Int; }
        }
        locus ParentL {
            capacity {
                pool entries of Int as_parent_for ChildL;
            }
            accept(c: ChildL) { }
        }
        fn main() { }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let diags = hale_types::check_program(&program);
    let joined: String = diags
        .iter()
        .map(|d| format!("{:?}", d))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        diags.iter().any(|d| {
            let s = format!("{:?}", d);
            s.contains("no slot named") || s.contains("entries")
        }),
        "expected mismatched-slot diagnostic, got: {}",
        joined
    );
}

/// Codegen rejects an `as_parent_for` whose parent slot kind
/// (Pool) differs from the child's same-named slot kind (Heap).
/// Typecheck does name-based validation only at v1; codegen has
/// the defensive kind+ty check that surfaces this mismatch.
#[test]
fn as_parent_for_codegen_rejects_kind_mismatch() {
    let src = r#"
        locus ChildL {
            capacity { heap entries of Int; }
        }
        locus ParentL {
            capacity {
                pool entries of Int as_parent_for ChildL;
            }
            accept(c: ChildL) { }
            run() {
                ChildL { };
            }
        }
        fn main() {
            ParentL { };
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin("hale_test_f22_apf_kind_mismatch");
    let err = build_executable(&program, &bin)
        .expect_err("should reject pool/heap kind mismatch");
    let msg = format!("{}", err);
    assert!(
        msg.contains("kind mismatch") || msg.contains("kind"),
        "expected kind-mismatch diagnostic, got: {}",
        msg
    );
}

/// End-to-end: child can acquire + release a cell from the
/// borrowed slot. Verifies the parent's allocator is actually
/// reachable from the child's slot — a stale ptr would crash
/// `release` or produce wrong reads.
#[test]
fn as_parent_for_child_uses_parent_allocator() {
    let src = r#"
        locus ChildL {
            capacity { pool entries of Int; }
            run() {
                let c = self.entries.acquire();
                self.entries.release(c);
                println("child-released");
            }
        }
        locus ParentL {
            capacity {
                pool entries of Int as_parent_for ChildL;
            }
            accept(c: ChildL) { }
            run() {
                ChildL { };
                println("parent-after-child");
            }
        }
        fn main() {
            ParentL { };
        }
    "#;
    let bin = build("child_uses_parent_alloc", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("child-released"),
        "missing child-released: {:?}",
        stdout
    );
    assert!(
        stdout.contains("parent-after-child"),
        "missing parent-after-child: {:?}",
        stdout
    );
}
