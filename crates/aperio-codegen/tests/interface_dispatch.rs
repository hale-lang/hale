//! F.20 Phase B: structural-interface vtable dispatch.
//!
//! Phase A landed the typechecker's structural-impl rule; Phase B
//! is the codegen half — interface values become fat pointers
//! `{data, vtable}`, with per-(locus, interface) static vtable
//! globals indexed by interface-method declaration order. A locus
//! flowing into an interface slot coerces at the call site; method
//! calls on the interface value dispatch indirect through the
//! vtable.
//!
//! These tests exercise the end-to-end shape: declare an interface,
//! declare two satisfying loci with different bodies, pass each to
//! a fn taking the interface, and observe that the indirect call
//! routed to the right body. The Sink stdlib migration that this
//! unblocks lives behind a follow-up commit (it touches the
//! stdlib seed); this test file's loci are user-level.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_iface_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn interface_dispatches_to_concrete_locus() {
    // One interface, one satisfying locus, one fn taking the
    // interface. Confirms the coercion (build fat pointer at call
    // site) + dispatch (indirect-call through vtable) round-trip
    // produces the same output as a direct method call.
    let src = r#"
        interface Greeter {
            fn greet(who: String);
        }

        locus Hello {
            params { }
            fn greet(who: String) {
                println("hello, ", who);
            }
        }

        fn shout(g: Greeter) {
            g.greet("world");
        }

        fn main() {
            let h = Hello { };
            shout(h);
        }
    "#;
    let (stdout, status) = build_and_run("basic", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("hello, world"),
        "indirect dispatch didn't reach the body; got: {:?}",
        stdout
    );
}

#[test]
fn interface_dispatches_to_two_different_loci() {
    // Two loci satisfying the same interface, each producing a
    // distinct output. The same fn invocation dispatches to
    // different bodies depending on which locus was coerced —
    // proves the vtable slot resolves per-(locus, interface) and
    // not by some shortcut that bakes the concrete callee into
    // the call site.
    let src = r#"
        interface Voice {
            fn say();
        }

        locus Cat {
            params { }
            fn say() {
                println("meow");
            }
        }

        locus Dog {
            params { }
            fn say() {
                println("woof");
            }
        }

        fn speak(v: Voice) {
            v.say();
        }

        fn main() {
            let c = Cat { };
            let d = Dog { };
            speak(c);
            speak(d);
        }
    "#;
    let (stdout, status) = build_and_run("two_loci", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(stdout.contains("meow"), "Cat dispatch missing: {:?}", stdout);
    assert!(stdout.contains("woof"), "Dog dispatch missing: {:?}", stdout);
    // Order matters — proves both calls actually fired with the
    // right vtable, not just one twice.
    let meow_idx = stdout.find("meow").expect("meow present");
    let woof_idx = stdout.find("woof").expect("woof present");
    assert!(
        meow_idx < woof_idx,
        "expected Cat before Dog; got: {:?}",
        stdout
    );
}

#[test]
fn interface_with_multiple_methods_picks_right_slot() {
    // Multi-method interface: vtable slots are indexed by
    // declaration order. If the wrong slot is read, the wrong
    // method body runs — the output catches it.
    let src = r#"
        interface Op {
            fn one();
            fn two();
            fn three();
        }

        locus K {
            params { }
            fn one()   { println("K.one"); }
            fn two()   { println("K.two"); }
            fn three() { println("K.three"); }
        }

        fn drive(op: Op) {
            op.two();
            op.three();
            op.one();
        }

        fn main() {
            let k = K { };
            drive(k);
        }
    "#;
    let (stdout, status) = build_and_run("multi_method", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    let two_idx = stdout.find("K.two").expect("K.two");
    let three_idx = stdout.find("K.three").expect("K.three");
    let one_idx = stdout.find("K.one").expect("K.one");
    assert!(
        two_idx < three_idx && three_idx < one_idx,
        "vtable slots resolved to wrong methods; got: {:?}",
        stdout
    );
}

#[test]
fn interface_method_args_pass_through_vtable_call() {
    // Args travel through the indirect call. If the implicit
    // self arg isn't prepended, or the user-visible args land in
    // the wrong slots, the output's `n=` won't match.
    let src = r#"
        interface Adder {
            fn add(a: Int, b: Int);
        }

        locus Sum {
            params { }
            fn add(a: Int, b: Int) {
                println("n=", a + b);
            }
        }

        fn dispatch(adder: Adder, a: Int, b: Int) {
            adder.add(a, b);
        }

        fn main() {
            let s = Sum { };
            dispatch(s, 3, 4);
        }
    "#;
    let (stdout, status) = build_and_run("args", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(stdout.contains("n=7"), "args misrouted; got: {:?}", stdout);
}

#[test]
fn locus_state_visible_via_interface_dispatch() {
    // The dispatch path's data slot must point at the actual
    // locus instance (with its params populated), not a fresh
    // empty locus. Reading a param from inside the dispatched
    // method body verifies the data ptr is the live one.
    let src = r#"
        interface Named {
            fn announce();
        }

        locus Person {
            params {
                name: String = "anon";
            }
            fn announce() {
                println("I am ", self.name);
            }
        }

        fn intro(n: Named) {
            n.announce();
        }

        fn main() {
            let p = Person { name: "Ada" };
            intro(p);
        }
    "#;
    let (stdout, status) = build_and_run("state", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("I am Ada"),
        "data ptr didn't reach the live locus; got: {:?}",
        stdout
    );
}
