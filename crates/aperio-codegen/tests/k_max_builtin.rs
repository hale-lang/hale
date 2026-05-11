//! 3b — F.16 built-in `self.k_max` synthesis in codegen.
//!
//! Formula: `k_max = B / [(1-phi)c + phi*sigma]`.
//! Pre-fix `aperio build` errored with
//! `no field 'k_max' on locus 'X'` because the interpreter
//! computed k_max in read_field but codegen treated it as a
//! plain struct field lookup. m... — wired the synthesis to
//! match the interpreter behavior.

use std::process::Command;

use aperio_codegen::build_executable;

fn build(name: &str, source: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_k_max_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn k_max_matches_formula_for_canonical_params() {
    // B=100, c=10, sigma=1, phi=0.5 →
    //   denom = (1-0.5)*10 + 0.5*1 = 5.0 + 0.5 = 5.5
    //   k_max = 100 / 5.5 ≈ 18.1818...
    let src = r#"
        locus CapL {
            params {
                B: Int = 100;
                c: Int = 10;
                sigma: Int = 1;
                phi: Float = 0.5;
            }
            fn report() {
                println("k_max=", self.k_max);
            }
        }

        fn main() {
            let l = CapL { B: 100, c: 10, sigma: 1, phi: 0.5 };
            l.report();
        }
    "#;
    let bin = build("canonical", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // float printing is `18.181818...` — match the integer
    // part + first decimal so display-precision drift
    // doesn't break the test.
    assert!(
        stdout.contains("k_max=18.18"),
        "got: {:?}",
        stdout
    );
}

#[test]
fn k_max_floats_with_mutated_params() {
    // Bump phi from 0.5 → 0.9 between reads; denom changes,
    // so k_max should change too — confirming the formula
    // is recomputed on each read, not cached at instantiation.
    let src = r#"
        locus CapL {
            params {
                B: Int = 100;
                c: Int = 10;
                sigma: Int = 1;
                phi: Float = 0.5;
            }
            fn before() { println("before=", self.k_max); }
            fn after() {
                self.phi = 0.9;
                println("after=", self.k_max);
            }
        }

        fn main() {
            let l = CapL { B: 100, c: 10, sigma: 1, phi: 0.5 };
            l.before();
            l.after();
        }
    "#;
    let bin = build("mutable", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // before: 100 / 5.5 ≈ 18.18
    // after: 100 / ((0.1*10) + (0.9*1)) = 100 / 1.9 ≈ 52.63
    assert!(stdout.contains("before=18.18"), "got: {:?}", stdout);
    assert!(stdout.contains("after=52.63"), "got: {:?}", stdout);
}
