//! market-book friction `self-array-field-index-assign-
//! unsupported` — `self.bid_prices[i] = price` rejected at
//! codegen with "2 segment(s) not yet supported". Fix routes
//! the 2-segment Self.Field.Index target through the same GEP
//! machinery as let-bound `arr[i] = x`.

use std::process::Command;

use aperio_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_self_field_idx_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn ladder_style_single_slot_update() {
    // BookL-style: fixed-cap array on self, mutate one slot
    // at a time. Pre-fix `self.prices[i] = x` errored out.
    let src = r#"
        locus BookL {
            params {
                prices: [Int; 4] = [0; 4];
                n: Int = 0;
            }
            fn set_at(i: Int, v: Int) {
                self.prices[i] = v;
            }
            fn get_at(i: Int) -> Int {
                return self.prices[i];
            }
        }

        fn main() {
            let b = BookL { prices: [0; 4], n: 0 };
            b.set_at(0, 11);
            b.set_at(1, 22);
            b.set_at(2, 33);
            b.set_at(3, 44);
            println("p0=", b.get_at(0));
            println("p1=", b.get_at(1));
            println("p2=", b.get_at(2));
            println("p3=", b.get_at(3));
        }
    "#;
    let bin = build("ladder", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("p0=11"), "got: {:?}", stdout);
    assert!(stdout.contains("p1=22"), "got: {:?}", stdout);
    assert!(stdout.contains("p2=33"), "got: {:?}", stdout);
    assert!(stdout.contains("p3=44"), "got: {:?}", stdout);
}

#[test]
fn float_array_field_index_assign() {
    let src = r#"
        locus FxL {
            params { xs: [Float; 3] = [0.0; 3]; }
            fn set_at(i: Int, v: Float) {
                self.xs[i] = v;
            }
            fn at(i: Int) -> Float {
                return self.xs[i];
            }
        }

        fn main() {
            let f = FxL { xs: [0.0; 3] };
            f.set_at(0, 1.5);
            f.set_at(2, 2.5);
            println("x0=", f.at(0));
            println("x1=", f.at(1));
            println("x2=", f.at(2));
        }
    "#;
    let bin = build("float_arr", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("x0=1.5"), "got: {:?}", stdout);
    assert!(stdout.contains("x1=0"), "got: {:?}", stdout);
    assert!(stdout.contains("x2=2.5"), "got: {:?}", stdout);
}
