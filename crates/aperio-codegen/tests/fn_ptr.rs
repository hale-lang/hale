//! m80: function-pointer language addition.
//!
//! Aperio gains first-class function values: `CodegenTy::FnPtr`,
//! `fn(T) -> R` parser support, fn-name-as-value lowering, and
//! indirect calls through fn-pointer fields on locus self. The
//! end-to-end test:
//!
//!   1. Declare a free function.
//!   2. Reference it by bare name (Expr::Ident) → emits the
//!      function's address as a fn-pointer value.
//!   3. Stash that pointer on a locus param of type `fn(Int)`.
//!   4. From inside the locus, indirect-call through the field:
//!      `self.cb(42)`.
//!
//! Failure modes (wrong signature, fn-pointer in arithmetic,
//! etc.) get smaller targeted tests too.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_fn_ptr_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn fn_pointer_round_trips_through_locus_param() {
    // The whole chain: handler is a free fn; passed by name to L's
    // cb param; L's birth() invokes self.cb(42) via the indirect
    // call path. Output proves the call ran.
    let src = r#"
        fn handler(x: Int) {
            println("handler got=", x);
        }

        locus L {
            params {
                cb: fn(Int) = handler;
            }
            birth() {
                self.cb(42);
            }
        }

        fn main() {
            L { };
        }
    "#;
    let (stdout, status) = build_and_run("round_trip", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("handler got=42"),
        "indirect call didn't fire; got: {:?}",
        stdout
    );
}

#[test]
fn fn_pointer_can_be_overridden_at_instantiation() {
    // The locus's default cb is one fn; an instantiation
    // overrides with a different fn. Confirms field overrides
    // flow through fn-typed params normally.
    let src = r#"
        fn original(x: Int) {
            println("original=", x);
        }
        fn override_(x: Int) {
            println("override=", x);
        }

        locus L {
            params {
                cb: fn(Int) = original;
            }
            birth() {
                self.cb(7);
            }
        }

        fn main() {
            L { cb: override_ };
        }
    "#;
    let (stdout, status) = build_and_run("override", src);
    assert!(status.success());
    assert!(
        stdout.contains("override=7"),
        "override fn didn't fire; got: {:?}",
        stdout
    );
    assert!(
        !stdout.contains("original="),
        "original fn shouldn't have fired; got: {:?}",
        stdout
    );
}

#[test]
fn fn_pointer_with_two_args_round_trips() {
    let src = r#"
        fn combiner(a: Int, b: Int) {
            println("a+b=", a + b);
        }

        locus L {
            params {
                f: fn(Int, Int) = combiner;
            }
            birth() {
                self.f(10, 32);
            }
        }

        fn main() {
            L { };
        }
    "#;
    let (stdout, status) = build_and_run("two_args", src);
    assert!(status.success());
    assert!(
        stdout.contains("a+b=42"),
        "two-arg call didn't compute right; got: {:?}",
        stdout
    );
}

#[test]
fn fn_pointer_with_locus_arg_round_trips() {
    // m83 prerequisite: fn(SomeLocus) — a function pointer whose
    // arg is a locus reference. Listener.on_connection: fn(Stream)
    // depends on this. The m82 lifecycle fix means the locus
    // passed in (b) is alive throughout the callback because the
    // caller's let-binding scope owns its dissolve.
    let src = r#"
        locus Box {
            params { value: Int = 0; }
        }

        fn show(b: Box) {
            println("box value=", b.value);
        }

        locus L {
            params {
                cb: fn(Box) = show;
            }
            birth() {
                let b = Box { value: 99 };
                self.cb(b);
            }
        }

        fn main() {
            L { };
        }
    "#;
    let (stdout, status) = build_and_run("locus_arg", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("box value=99"),
        "fn(Locus) indirect call didn't fire; got: {:?}",
        stdout
    );
}

#[test]
fn fn_pointer_with_string_arg_round_trips() {
    let src = r#"
        fn say(msg: String) {
            println("said: ", msg);
        }

        locus L {
            params {
                speak: fn(String) = say;
            }
            birth() {
                self.speak("hello fn-ptr");
            }
        }

        fn main() {
            L { };
        }
    "#;
    let (stdout, status) = build_and_run("string_arg", src);
    assert!(status.success());
    assert!(
        stdout.contains("said: hello fn-ptr"),
        "String-arg call didn't propagate; got: {:?}",
        stdout
    );
}
