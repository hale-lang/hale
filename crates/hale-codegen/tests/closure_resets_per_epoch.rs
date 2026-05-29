//! Integration tests for F.34 (v1.x-WINDOWED) — the
//! `resets_per_epoch(...)` closure clause that zeros named locus
//! fields AFTER the assertion fires at a `duration(N)` epoch
//! boundary. Closes the `low_corrupt_rate` friction by giving
//! Hale a way to express a per-window rate budget without the
//! user re-implementing the reset dance.

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run(
    name: &str,
    source: &str,
) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_test_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

/// Runs the typecheck pass over `source` and returns the
/// collected diagnostic messages joined by newline. Mirrors the
/// CLI's parse → resolve → check pipeline so tests can assert on
/// type-side rejections without going through codegen.
fn typecheck_diags(source: &str) -> Vec<String> {
    let program = hale_syntax::parse_source(source).expect("parse");
    let mut programs = std::collections::BTreeMap::new();
    programs.insert("main".to_string(), &program);
    let bundle = hale_types::Bundle { programs };
    let (scope, _) = hale_types::resolve::build_top_scope(&bundle);
    let diags = hale_types::check::check_bundle(&bundle, &scope, true);
    diags.into_iter().map(|d| d.message).collect()
}

#[test]
fn duration_epoch_resets_named_int_field_after_assertion() {
    // The closure asserts the counter stays within budget AND
    // lists it in `resets_per_epoch`. We seed the counter at 5
    // (well within the budget of 100), let the duration fire
    // during the post-run drain, and verify the counter has been
    // zeroed by dissolve time — which is only possible if the
    // reset hook ran.
    let src = r#"
        locus RateBudget {
            params {
                count: Int = 5;
            }
            closure low_rate {
                self.count ~~ 0 within 100;
                epoch duration(10ms);
                resets_per_epoch(count);
            }
            run() {
                std::time::sleep(30ms);
            }
            dissolve() {
                println("count=", self.count);
            }
        }
        fn main() { RateBudget { }; }
    "#;
    let (stdout, status) = build_and_run("resets_per_epoch_int", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("count=0"),
        "expected count=0 after duration epoch reset, got: {:?}",
        stdout
    );
}

#[test]
fn duration_epoch_resets_named_float_field_after_assertion() {
    // Same shape, Float field. Confirms the reset hook handles
    // both numeric primitives the typecheck admits.
    let src = r#"
        locus Drift {
            params {
                err: Float = 0.25;
            }
            closure low_drift {
                self.err ~~ 0.0 within 10.0;
                epoch duration(10ms);
                resets_per_epoch(err);
            }
            run() {
                std::time::sleep(30ms);
            }
            dissolve() {
                println("err=", self.err);
            }
        }
        fn main() { Drift { }; }
    "#;
    let (stdout, status) = build_and_run("resets_per_epoch_float", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("err=0"),
        "expected err=0 after duration epoch reset, got: {:?}",
        stdout
    );
}

#[test]
fn resets_per_epoch_on_tick_is_rejected_at_typecheck() {
    // Pair with `epoch tick`: the rate-budget framing doesn't fit
    // a tick-frequency window. Typecheck rejects.
    let src = r#"
        locus Bad {
            params {
                n: Int = 0;
            }
            closure x {
                self.n ~~ 0 within 100;
                epoch tick;
                resets_per_epoch(n);
            }
        }
        fn main() { Bad { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("resets_per_epoch")
            && m.contains("epoch duration")),
        "expected diagnostic naming the duration-only restriction, \
         got: {:?}",
        diags
    );
}

#[test]
fn resets_per_epoch_unknown_field_is_rejected_at_typecheck() {
    let src = r#"
        locus Bad {
            params {
                n: Int = 0;
            }
            closure x {
                self.n ~~ 0 within 100;
                epoch duration(1s);
                resets_per_epoch(does_not_exist);
            }
        }
        fn main() { Bad { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("does_not_exist")
            && m.contains("not declared")),
        "expected diagnostic naming the missing field, got: {:?}",
        diags
    );
}

#[test]
fn resets_per_epoch_non_numeric_field_is_rejected_at_typecheck() {
    let src = r#"
        locus Bad {
            params {
                label: String = "hello";
            }
            closure x {
                self.label ~~ "" within 0;
                epoch duration(1s);
                resets_per_epoch(label);
            }
        }
        fn main() { Bad { }; }
    "#;
    let diags = typecheck_diags(src);
    assert!(
        diags.iter().any(|m| m.contains("non-numeric")
            && m.contains("label")),
        "expected diagnostic naming the non-numeric field, got: {:?}",
        diags
    );
}
