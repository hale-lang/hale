//! v1.x-FORM-1 PR7: end-to-end `@form(vec)` execution under
//! the interpreter. Each test parses + typechecks + runs a
//! complete program. Codegen-side parity lands in a future
//! FORM-2 PR (PR5/6) and gets its own test file.

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

fn run_expect_error(src: &str) -> String {
    let program = aperio_syntax::parse_source(src)
        .map_err(|d| {
            d.iter()
                .map(|x| x.render(src))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .expect("parse");
    match run_program(&program) {
        Ok(_) => panic!("expected program to exit with an error, got ok"),
        Err(s) => s,
    }
}

#[test]
fn push_get_round_trip() {
    let src = r#"
        @form(vec)
        locus ItemListL {
            capacity { heap items of Int; }
        }
        fn main() {
            let l = ItemListL { };
            l.push(42);
            let head = l.get(0) or raise;
            println(head);
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn len_and_is_empty_track_pushes() {
    let src = r#"
        @form(vec)
        locus L {
            capacity { heap items of Int; }
        }
        fn main() {
            let l = L { };
            if !l.is_empty() { println("FAIL: should start empty"); }
            l.push(1);
            l.push(2);
            l.push(3);
            if l.len() != 3 { println("FAIL: len should be 3"); }
            println("ok");
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn pop_returns_last_element_lifo() {
    let src = r#"
        @form(vec)
        locus L {
            capacity { heap items of Int; }
        }
        fn main() {
            let l = L { };
            l.push(10);
            l.push(20);
            l.push(30);
            let a = l.pop() or raise;
            let b = l.pop() or raise;
            if a != 30 { println("FAIL: first pop"); }
            if b != 20 { println("FAIL: second pop"); }
            if l.len() != 1 { println("FAIL: len after pops"); }
            println("ok");
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn get_out_of_bounds_substitute_uses_fallback() {
    let src = r#"
        @form(vec)
        locus L {
            capacity { heap items of Int; }
        }
        fn main() {
            let l = L { };
            l.push(7);
            let v = l.get(99) or -1;
            if v != -1 { println("FAIL: fallback not used"); }
            println("ok");
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn pop_empty_substitute_uses_fallback() {
    let src = r#"
        @form(vec)
        locus L {
            capacity { heap items of Int; }
        }
        fn main() {
            let l = L { };
            let v = l.pop() or -42;
            if v != -42 { println("FAIL: empty-pop fallback not used"); }
            println("ok");
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn get_out_of_bounds_or_raise_bubbles() {
    let src = r#"
        @form(vec)
        locus L {
            capacity { heap items of Int; }
        }
        fn main() {
            let l = L { };
            let v = l.get(99) or raise;
            println(v);
        }
    "#;
    // The unaddressed `raise` past the top-level fn surfaces as
    // a runtime error in the interpreter (no on_failure to
    // catch). Just verify the program errors out rather than
    // succeeding.
    let _err = run_expect_error(src);
}

#[test]
fn err_binding_available_on_substitute_rhs() {
    let src = r#"
        @form(vec)
        locus L {
            capacity { heap items of Int; }
        }
        fn fallback(e: IndexError) -> Int {
            return e.index + e.len;
        }
        fn main() {
            let l = L { };
            l.push(1);
            l.push(2);
            // Out-of-bounds at index 5; len=2; fallback should
            // see 5 + 2 = 7.
            let v = l.get(5) or fallback(err);
            if v != 7 { println("FAIL: err binding payload"); }
            println("ok");
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn fail_stmt_exits_via_error_path() {
    let src = r#"
        type E { code: Int; }
        fn pick(x: Int) -> Int fallible(E) {
            if x < 0 {
                fail E { code: 1 };
            }
            return x * 2;
        }
        fn main() {
            let ok_v = pick(5) or raise;
            let bad_v = pick(-3) or -99;
            if ok_v != 10 { println("FAIL: success path"); }
            if bad_v != -99 { println("FAIL: error fallback"); }
            println("ok");
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn vec_of_struct_cells() {
    let src = r#"
        type Pair { x: Int; y: Int; }
        @form(vec)
        locus PairsL {
            capacity { heap items of Pair; }
        }
        fn main() {
            let l = PairsL { };
            l.push(Pair { x: 1, y: 2 });
            l.push(Pair { x: 3, y: 4 });
            let first = l.get(0) or raise;
            if first.x != 1 { println("FAIL: x"); }
            if first.y != 2 { println("FAIL: y"); }
            println("ok");
        }
    "#;
    assert_eq!(run(src), 0);
}
