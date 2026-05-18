//! G20 / F.20 Phase B follow-up — free-fn return of interface value.
//!
//! Three surfaces verified:
//!
//! 1. **Fresh-instantiation return.** `fn make() -> Greeter { return
//!    Hi {}; }` — the locus is instantiated inside the fn, coerced to
//!    the interface, and returned. m90 routing extension keeps the
//!    underlying locus in the program-lifetime payload arena;
//!    emit_return_value_deep_copy copies the 16-byte fat pointer into
//!    caller_arena.
//!
//! 2. **Polymorphic return through control flow.** Multiple loci
//!    satisfying the same interface, returned from different branches
//!    of a fn. Each branch's locus is routed to payload arena, the
//!    fat-pointer struct deep-copied at the epilogue.
//!
//! 3. **Pass-through + escaped mutation.** An interface value built
//!    by one fn is passed to another, returned, and mutated. The
//!    mutation must alias the underlying locus across all aliases
//!    (the original caller's binding, the passthrough binding).

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "aperio_iface_return_{}_{}",
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
fn fresh_instantiation_return() {
    let src = r#"
        interface Greeter {
            fn greet() -> String;
        }
        locus Hi {
            fn greet() -> String { return "hello"; }
        }
        fn make() -> Greeter {
            return Hi {};
        }
        fn main() {
            let g = make();
            println(g.greet());
        }
    "#;
    let (stdout, status) = build_and_run("fresh", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("hello"), "missing greeting: {:?}", stdout);
}

#[test]
fn polymorphic_return_through_control_flow() {
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
        fn make(which: Int) -> Greeter {
            if which == 0 {
                let h = Hi {};
                return h;
            }
            return Hey {};
        }
        fn main() {
            let a = make(0);
            let b = make(1);
            println(a.greet());
            println(b.greet());
        }
    "#;
    let (stdout, status) = build_and_run("polymorphic", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("hello"), "missing first: {:?}", stdout);
    assert!(stdout.contains("hey there"), "missing second: {:?}", stdout);
}

#[test]
fn passthrough_and_escaped_mutation() {
    let src = r#"
        interface Counter {
            fn tick();
            fn current() -> Int;
        }
        locus Cnt {
            params { n: Int = 0; }
            fn tick() { self.n = self.n + 1; }
            fn current() -> Int { return self.n; }
        }
        fn passthrough(c: Counter) -> Counter {
            return c;
        }
        fn build_and_bump() -> Counter {
            let c = Cnt {};
            c.tick();
            c.tick();
            return c;
        }
        fn main() {
            let c1 = build_and_bump();
            println("from build_and_bump: " + c1.current());
            c1.tick();
            println("after extra tick: " + c1.current());
            let c2 = passthrough(c1);
            println("passthrough: " + c2.current());
            c2.tick();
            println("aliases c1: " + c1.current());
        }
    "#;
    let (stdout, status) = build_and_run("passthrough", src);
    assert!(status.success(), "non-zero: {:?}", status);
    for needle in [
        "from build_and_bump: 2",
        "after extra tick: 3",
        "passthrough: 3",
        "aliases c1: 4",
    ] {
        assert!(stdout.contains(needle), "missing `{}`: {:?}", needle, stdout);
    }
}
