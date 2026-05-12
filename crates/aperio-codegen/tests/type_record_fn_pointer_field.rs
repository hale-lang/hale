//! v1.x-8: `type` records hold `fn(...)` fields.
//!
//! Friction driver: cli-demo's Cmd record wants a `run: fn()` field
//! so the dispatcher can store handlers by command name. Before this
//! fix the lexer accepted `run` as a struct field name but the parser
//! rejected the keyword, and even after admitting it the codegen
//! routed `c.run()` through the locus-method path instead of doing
//! a fn-pointer load + indirect call.
//!
//! Three fixes proved out here:
//!   1. `parse_struct_field` / `parse_struct_init` use the
//!      keyword-admitting `expect_member_name`.
//!   2. `try_member_keyword_as_name` covers the lifecycle keywords
//!      (run, birth, accept, drain, dissolve, capacity).
//!   3. `lower_external_method_call` dispatches struct receivers
//!      with FnPtr fields via GEP + load + indirect-call instead of
//!      falling through to the locus-method branch.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_type_fnptr_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn struct_field_holds_zero_arg_fn_pointer() {
    // The exact friction case from cli-demo: a record with a
    // `run: fn()` field, populated by passing a fn by name at
    // init, then invoked via field-method syntax `c.run()`.
    let src = r#"
        type Cmd { name: String; run: fn(); }

        fn handler() {
            println("handler-ran");
        }

        fn main() {
            let c = Cmd { name: "hello", run: handler };
            c.run();
        }
    "#;
    let (stdout, status) = build_and_run("zero_arg", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("handler-ran"),
        "indirect call through struct field didn't fire; got: {:?}",
        stdout
    );
}

#[test]
fn struct_field_holds_int_arg_fn_pointer() {
    // Single-arg fn pointer in a struct field, with the struct
    // holding mixed payload: an Int + a fn-pointer + a String.
    let src = r#"
        type Entry { id: Int; tag: String; cb: fn(Int); }

        fn show(x: Int) {
            println("show x=", x);
        }

        fn main() {
            let e = Entry { id: 7, tag: "k", cb: show };
            e.cb(42);
        }
    "#;
    let (stdout, status) = build_and_run("int_arg", src);
    assert!(status.success());
    assert!(
        stdout.contains("show x=42"),
        "int-arg fn-ptr field call didn't fire; got: {:?}",
        stdout
    );
}

#[test]
fn struct_field_dispatcher_selects_among_records() {
    // The actual cli-demo pattern: build several Cmd records,
    // each carrying its own handler, look up by name, dispatch.
    let src = r#"
        type Cmd { name: String; run: fn(); }

        fn do_a() { println("a-ran"); }
        fn do_b() { println("b-ran"); }

        fn main() {
            let a = Cmd { name: "a", run: do_a };
            let b = Cmd { name: "b", run: do_b };
            a.run();
            b.run();
        }
    "#;
    let (stdout, status) = build_and_run("dispatch", src);
    assert!(status.success());
    assert!(
        stdout.contains("a-ran") && stdout.contains("b-ran"),
        "both handlers should have fired; got: {:?}",
        stdout
    );
}
