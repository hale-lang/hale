//! 3a regression — calling a locus method from a free fn
//! (e.g. `main`) must push the receiver onto self_stack so the
//! method body's `self.X` resolves to the captured locus.
//!
//! Pre-fix the interpreter errored with
//! `runtime error: 'self' referenced outside a locus body` on
//! the FIRST self-read inside the method body, because
//! `read_field(Value::Locus, methodName)` returned a bare
//! FnRef with no receiver and `call_fn` never pushed self.

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
fn free_fn_calls_method_with_self_read() {
    let src = r#"
        locus CounterL {
            params { n: Int = 0; }
            fn bump() { self.n = self.n + 1; }
            fn count() -> Int { return self.n; }
        }

        fn main() {
            let c = CounterL { n: 0 };
            c.bump();
            c.bump();
            c.bump();
            let n = c.count();
            if n != 3 {
                println("FAIL count=", n);
                return 1;
            }
            println("OK count=", n);
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn nested_method_call_chains_through_self() {
    // bump() calls another method on the same self; both
    // levels need the receiver visible on self_stack.
    let src = r#"
        locus AccL {
            params { n: Int = 0; }
            fn bump_inner() { self.n = self.n + 1; }
            fn bump() { self.bump_inner(); }
            fn count() -> Int { return self.n; }
        }

        fn main() {
            let a = AccL { n: 0 };
            a.bump();
            a.bump();
            if a.count() != 2 {
                return 1;
            }
        }
    "#;
    assert_eq!(run(src), 0);
}
