//! v1.x-3 PR5 — interpreter parity for recognition sub-modes.
//!
//! The interpreter doesn't model the actual recpool memory layout
//! (no malloc'd bitmap; loci are Rust `Value`s) but it must still
//! accept the parsed sub-mode annotation, run the same lifecycle,
//! and produce the same observable output as the codegen path.
//! Without this, `aperio run examples/14-projection-classes`
//! would diverge from `aperio build && ./examples/14-...`.

use aperio_runtime::run_program;

fn run(src: &str) -> i32 {
    let program = aperio_syntax::parse_source(src)
        .map_err(|d| {
            d.iter()
                .map(|x| x.render(src))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .expect("parse");
    run_program(&program).expect("run")
}

#[test]
fn fixed_cell_recognition_runs_in_interpreter() {
    let src = r#"
        locus Leaf {
            params {
                value: Int = 0;
            }
            contract {
                expose value: Int;
            }
        }
        locus RecCoord : projection recognition(cap=4, fixed_cell(bytes=128)) {
            contract { consume value: Int; }
            accept(c: Leaf) { }
            run() {
                let _l1 = Leaf { value: 1 };
                let _l2 = Leaf { value: 2 };
                let _l3 = Leaf { value: 3 };
                let mut total: Int = 0;
                for child in self.children {
                    total = total + child.value;
                }
                if total == 6 {
                    return 0;
                }
                return 1;
            }
        }
        fn main() -> Int {
            RecCoord { };
            return 0;
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn shared_slab_recognition_runs_in_interpreter() {
    let src = r#"
        locus Leaf {
            params {
                value: Int = 0;
            }
            contract {
                expose value: Int;
            }
        }
        locus SlabCoord : projection recognition(cap=4, shared_slab(bytes=2048)) {
            contract { consume value: Int; }
            accept(c: Leaf) { }
            run() {
                let _l1 = Leaf { value: 10 };
                let _l2 = Leaf { value: 20 };
                let _l3 = Leaf { value: 30 };
                let mut total: Int = 0;
                for child in self.children {
                    total = total + child.value;
                }
                if total == 60 {
                    return 0;
                }
                return 1;
            }
        }
        fn main() -> Int {
            SlabCoord { };
            return 0;
        }
    "#;
    assert_eq!(run(src), 0);
}
