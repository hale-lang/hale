//! 3a regression — calling a locus method from a free fn (e.g.
//! `main`) must bind the receiver so the method body's `self.X`
//! resolves to the called-on locus, including across a nested
//! method-to-method call on the same receiver.
//!
//! Ported from the retired interpreter parity suite
//! (`hale-runtime/tests/method_call_from_free_fn.rs`). The
//! interpreter pre-fix errored with `'self' referenced outside a
//! locus body` because `read_field(Value::Locus, methodName)`
//! returned a bare FnRef with no receiver and `call_fn` never
//! pushed self. Codegen lowers the receiver explicitly, so this
//! locks the equivalent surface against a codegen regression.

use hale_codegen::build_executable;
use std::process::Command;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_method_from_freefn_{}_{}", name, std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
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
                println("FAIL count=", to_string(n));
                return 1;
            }
            println("OK count=", to_string(n));
        }
    "#;
    let (stdout, status) = build_and_run("self_read", src);
    assert!(status.success(), "non-zero exit: {:?}, stdout: {:?}", status, stdout);
    assert!(stdout.contains("OK count=3"), "stdout: {:?}", stdout);
}

#[test]
fn nested_method_call_chains_through_self() {
    // bump() calls another method on the same self; both levels
    // need the receiver bound for `self.X` to resolve.
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
                println("FAIL count=", to_string(a.count()));
                return 1;
            }
            println("OK count=", to_string(a.count()));
        }
    "#;
    let (stdout, status) = build_and_run("nested_self", src);
    assert!(status.success(), "non-zero exit: {:?}, stdout: {:?}", status, stdout);
    assert!(stdout.contains("OK count=2"), "stdout: {:?}", stdout);
}
