//! market-book friction `bus-payload-primitives-only` —
//! nested user-struct fields in a bus payload type. Pre-fix
//! `emit_per_field_serialize` / `emit_per_field_deserialize`
//! rejected non-primitive field types at codegen-build with
//! `wire format supports primitives and String only`. This
//! widens the walker to recursive nested-struct support so
//! `type Msg { price: Fixed; ... }` rides the bus natively
//! without the flatten-to-primitives workaround.

use std::process::Command;

use aperio_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_nested_bus_payload_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn nested_struct_field_round_trips_via_bus() {
    // type Fixed { raw: Int; } — single-field wrapper.
    // type SnapshotMsg { side: Int; price: Fixed; qty: Fixed; }
    // Publisher writes a Snapshot with two Fixed values;
    // subscriber reads them back and prints both raws.
    let src = r#"
        type Fixed { raw: Int; }
        type SnapshotMsg {
            side: Int;
            price: Fixed;
            qty: Fixed;
        }

        locus SubL {
            bus {
                subscribe "snap" as on_snap of type SnapshotMsg;
            }
            fn on_snap(m: SnapshotMsg) {
                println("side=", m.side);
                println("price=", m.price.raw);
                println("qty=", m.qty.raw);
            }
        }

        locus PubL {
            bus {
                publish "snap" of type SnapshotMsg;
            }
            birth() {
                "snap" <- SnapshotMsg {
                    side: 1,
                    price: Fixed { raw: 12345 },
                    qty: Fixed { raw: 100 },
                };
            }
        }

        fn main() {
            SubL { };
            PubL { };
        }
    "#;
    let bin = build("two_nested", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("side=1"), "got: {:?}", stdout);
    assert!(stdout.contains("price=12345"), "got: {:?}", stdout);
    assert!(stdout.contains("qty=100"), "got: {:?}", stdout);
}

#[test]
fn nested_struct_with_string_leaf_round_trips() {
    // Mixed primitive + nested + nested-with-String. Pin
    // that the recursion handles a String leaf inside a
    // nested struct (not just Int leaves).
    let src = r#"
        type Author { name: String; id: Int; }
        type Post { who: Author; topic: String; views: Int; }

        locus SubL {
            bus {
                subscribe "post" as on_post of type Post;
            }
            fn on_post(p: Post) {
                println("who.name=", p.who.name);
                println("who.id=", p.who.id);
                println("topic=", p.topic);
                println("views=", p.views);
            }
        }

        locus PubL {
            bus {
                publish "post" of type Post;
            }
            birth() {
                "post" <- Post {
                    who: Author { name: "ada", id: 7 },
                    topic: "lotus",
                    views: 42,
                };
            }
        }

        fn main() {
            SubL { };
            PubL { };
        }
    "#;
    let bin = build("mixed_string_leaf", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("who.name=ada"), "got: {:?}", stdout);
    assert!(stdout.contains("who.id=7"), "got: {:?}", stdout);
    assert!(stdout.contains("topic=lotus"), "got: {:?}", stdout);
    assert!(stdout.contains("views=42"), "got: {:?}", stdout);
}
