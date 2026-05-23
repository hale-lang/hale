//! G20 (2026-05-23) — Locus → Interface coercion at composite
//! construction sites (tuple and fixed-array elements).
//!
//! Closes the deferred half of F.20: prior to this, an
//! `[Interface; N]` or `(Interface, ...)` ascription failed
//! codegen with "array literal mixes element types
//! (LocusRef(L1) and LocusRef(L2))". The fix is a new
//! `lower_expr_into(expr, hint)` helper that propagates the
//! ascription's destination type into element-level lowering
//! so per-position `coerce_to_interface` fires before the
//! "mixes element types" check ever runs.
//!
//! Scope at this milestone:
//! - **let-RHS with ascription** — wired here.
//! - Fn-arg, struct-field-init, return position — same helper
//!   would extend cleanly; landed iteratively as workloads
//!   surface demand.
//! - Nested locus-escape through composite returns — remains
//!   gated on the broader m90 routing extension (the bigger
//!   half of F.20's "deferred" caveat).

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "aperio_g20_composite_{}_{}",
        name,
        std::process::id()
    ));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn array_of_interface_dispatches_heterogeneously() {
    let src = r#"
        interface Greeter {
            fn greet() -> String;
        }
        locus Hi {
            fn greet() -> String { return "hello"; }
        }
        locus Hey {
            fn greet() -> String { return "hey there"; }
        }

        fn main() {
            let arr: [Greeter; 2] = [Hi { }, Hey { }];
            println(arr[0].greet());
            println(arr[1].greet());
        }
    "#;
    let (stdout, status) = build_and_run("array_heterogeneous", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("hello"), "missing first: {:?}", stdout);
    assert!(stdout.contains("hey there"), "missing second: {:?}", stdout);
}

#[test]
fn tuple_of_interface_dispatches_per_position() {
    let src = r#"
        interface Greeter {
            fn greet() -> String;
        }
        locus Hi {
            fn greet() -> String { return "hello"; }
        }
        locus Hey {
            fn greet() -> String { return "hey there"; }
        }

        fn main() {
            let pair: (Greeter, Greeter) = (Hi { }, Hey { });
            let (a, b) = pair;
            println(a.greet());
            println(b.greet());
        }
    "#;
    let (stdout, status) = build_and_run("tuple_per_position", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("hello"), "missing first: {:?}", stdout);
    assert!(stdout.contains("hey there"), "missing second: {:?}", stdout);
}

#[test]
fn array_repeat_of_interface_fills_all_slots() {
    let src = r#"
        interface Greeter {
            fn greet() -> String;
        }
        locus Hi {
            fn greet() -> String { return "hello"; }
        }

        fn main() {
            let arr: [Greeter; 3] = [Hi { }; 3];
            let mut i = 0;
            while i < 3 {
                println(arr[i].greet());
                i = i + 1;
            }
        }
    "#;
    let (stdout, status) = build_and_run("array_repeat", src);
    assert!(status.success(), "non-zero: {:?}", status);
    let n = stdout.matches("hello").count();
    assert_eq!(n, 3, "expected 3 'hello' lines, got {}: {:?}", n, stdout);
}

#[test]
fn array_of_interface_passed_to_fn_arg() {
    // Once the let-binding coerces, the array's static type is
    // already `[Greeter; N]` — passing it to a fn whose param
    // is also `[Greeter; N]` goes through plain type matching,
    // no further coercion needed at the call site.
    let src = r#"
        interface Greeter {
            fn greet() -> String;
        }
        locus Hi {
            fn greet() -> String { return "hello"; }
        }
        locus Hey {
            fn greet() -> String { return "hey there"; }
        }

        fn pick(gs: [Greeter; 2], i: Int) -> String {
            if i == 0 { return gs[0].greet(); }
            return gs[1].greet();
        }

        fn main() {
            let arr: [Greeter; 2] = [Hi { }, Hey { }];
            println(pick(arr, 0));
            println(pick(arr, 1));
        }
    "#;
    let (stdout, status) = build_and_run("array_fn_arg", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("hello"), "missing first: {:?}", stdout);
    assert!(stdout.contains("hey there"), "missing second: {:?}", stdout);
}
