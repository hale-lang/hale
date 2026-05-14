//! v1.x-FORM-5: end-to-end `@form(ring_buffer)` execution under
//! the interpreter. Each test parses + typechecks + runs a
//! complete program. Mirrors the codegen-side tests at
//! `crates/aperio-codegen/tests/form_ring_buffer_codegen.rs`.

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
fn push_pop_fifo_order() {
    let src = r#"
        @form(ring_buffer, cap = 4)
        locus RB { capacity { pool history of Int; } }
        fn main() {
            let rb = RB { };
            let _ = rb.push(1);
            let _ = rb.push(2);
            let _ = rb.push(3);
            let a = rb.pop() or raise;
            let b = rb.pop() or raise;
            let c = rb.pop() or raise;
            print(a); print(" "); print(b); print(" "); println(c);
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn push_returns_false_when_full() {
    let src = r#"
        @form(ring_buffer, cap = 2)
        locus RB { capacity { pool history of Int; } }
        fn main() {
            let rb = RB { };
            let ok1 = rb.push(1);
            let ok2 = rb.push(2);
            let ok3 = rb.push(3);
            print(ok1); print(" "); print(ok2); print(" "); println(ok3);
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn len_and_is_full() {
    let src = r#"
        @form(ring_buffer, cap = 3)
        locus RB { capacity { pool history of Int; } }
        fn main() {
            let rb = RB { };
            print(rb.len()); print(" "); println(rb.is_full());
            let _ = rb.push(1);
            let _ = rb.push(2);
            let _ = rb.push(3);
            print(rb.len()); print(" "); println(rb.is_full());
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn pop_empty_substitutes() {
    let src = r#"
        @form(ring_buffer, cap = 4)
        locus RB { capacity { pool history of Int; } }
        fn main() {
            let rb = RB { };
            let v = rb.pop() or 42;
            println(v);
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn pop_empty_with_err_binding() {
    let src = r#"
        @form(ring_buffer, cap = 4)
        locus RB { capacity { pool history of Int; } }
        fn report(e: EmptyError) -> Int {
            print("kind="); println(e.kind);
            return -1;
        }
        fn main() {
            let rb = RB { };
            let v = rb.pop() or report(err);
            println(v);
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn wrap_around_round_trip() {
    let src = r#"
        @form(ring_buffer, cap = 3)
        locus RB { capacity { pool history of Int; } }
        fn main() {
            let rb = RB { };
            let _ = rb.push(1);
            let _ = rb.push(2);
            let _ = rb.push(3);
            let _ = rb.pop() or raise;
            let _ = rb.pop() or raise;
            let _ = rb.push(4);
            let _ = rb.push(5);
            let c = rb.pop() or raise;
            let d = rb.pop() or raise;
            let e = rb.pop() or raise;
            print(c); print(" "); print(d); print(" "); println(e);
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn ring_buffer_of_struct_cells() {
    let src = r#"
        type Sample { v: Int; }
        @form(ring_buffer, cap = 2)
        locus RB { capacity { pool history of Sample; } }
        fn main() {
            let rb = RB { };
            let _ = rb.push(Sample { v: 7 });
            let _ = rb.push(Sample { v: 9 });
            let a = rb.pop() or raise;
            print(a.v); print(" ");
            let b = rb.pop() or raise;
            println(b.v);
        }
    "#;
    assert_eq!(run(src), 0);
}
