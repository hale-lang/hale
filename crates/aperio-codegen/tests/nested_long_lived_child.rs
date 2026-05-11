//! 3d + 3e regression — nested long-lived (bus-subscribing)
//! child loci.
//!
//! Two symptoms from the friction note
//! `nested-locus-child-field-reads-return-garbage` (2026-05-11):
//!   3e — `for c in self.children { c.X }` reads return
//!        uninitialized memory through the iteration handle.
//!   3d — bus subscriptions declared on the nested child
//!        never fire.
//!
//! Likely shared root: long-lived (with-subscription) loci
//! birthed in lifecycle bodies hit `alloca_in_entry_with_nulled
//! _arena` so they survive across early-return paths, but
//! something in the children-array append or the subscription
//! registration is reading the wrong pointer.

use std::process::Command;

use aperio_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_nested_long_lived_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn nested_long_lived_child_field_reads_through_children() {
    // 3e: ParentL.run() instantiates a child that subscribes
    // to "X". With the subscription making the child
    // long-lived, the deferred-dissolve path runs at run()'s
    // exit. Before that, `self.children` iteration in
    // `report()` should see the child's initialized fields.
    let src = r#"
        type Tick { n: Int; }

        locus ChildL {
            params { tag: Int = 42; }
            bus {
                subscribe "tick" as on_tick of type Tick;
            }
            fn on_tick(t: Tick) {
                self.tag = t.n;
            }
        }

        locus ParentL {
            params { unused: Int = 0; }
            accept(c: ChildL) { }
            fn report() {
                for c in self.children {
                    println("tag=", c.tag);
                }
            }
            run() {
                let _ = ChildL { tag: 7 };
                self.report();
            }
        }

        fn main() {
            ParentL { };
        }
    "#;
    let bin = build("field_reads", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("tag=7"),
        "field read through self.children returned garbage; got: {:?}",
        stdout
    );
}

#[test]
fn nested_long_lived_child_birthed_in_birth_lifecycle() {
    // Friction note's exact framing: ParentL.birth() spawns
    // the long-lived child. The bug was reported as
    // "reads return uninitialized memory" — pin the shape
    // here so any regression surfaces.
    let src = r#"
        type Tick { n: Int; }

        locus ChildL {
            params { tag: Int = 42; }
            bus {
                subscribe "tick" as on_tick of type Tick;
            }
            fn on_tick(t: Tick) {
                self.tag = t.n;
            }
        }

        locus ParentL {
            params { unused: Int = 0; }
            accept(c: ChildL) { }
            fn report() {
                for c in self.children {
                    println("tag=", c.tag);
                }
            }
            birth() {
                let _ = ChildL { tag: 11 };
            }
            run() {
                self.report();
            }
        }

        fn main() {
            ParentL { };
        }
    "#;
    let bin = build("birthed_in_birth", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("tag=11"),
        "child birthed in birth() — field reads through self.children returned garbage; got: {:?}",
        stdout
    );
}

#[test]
fn nested_long_lived_child_subscription_fires() {
    // 3d: ChildL is birthed inside ParentL.run() and
    // subscribes to "ping". main() publishes to "ping" via a
    // helper locus. The child's handler should fire and set
    // a flag the parent can observe via its children
    // iteration.
    let src = r#"
        type Tick { n: Int; }

        locus ChildL {
            params { saw: Int = 0; }
            bus {
                subscribe "ping" as on_ping of type Tick;
            }
            fn on_ping(t: Tick) {
                self.saw = t.n;
            }
        }

        locus PingerL {
            params { unused: Int = 0; }
            bus {
                publish "ping" of type Tick;
            }
            fn fire() {
                "ping" <- Tick { n: 9 };
            }
        }

        locus ParentL {
            params { unused: Int = 0; }
            accept(c: ChildL) { }
            fn report() {
                for c in self.children {
                    println("saw=", c.saw);
                }
            }
            run() {
                let _c = ChildL { saw: 0 };
                let p = PingerL { unused: 0 };
                p.fire();
                self.report();
            }
        }

        fn main() {
            ParentL { };
        }
    "#;
    let bin = build("subscription_fires", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("saw=9"),
        "nested child's bus subscription did not fire; got: {:?}",
        stdout
    );
}
