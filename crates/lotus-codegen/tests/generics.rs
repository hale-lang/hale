//! m61: generic struct monomorphization (narrow first slice).
//!
//! Verifies the substrate slice that lifts codegen's pre-m61
//! "generic type X" rejection. Discovery walks
//! `TypeDeclBody::Struct` field types for `Foo<Args>` references
//! to known generic templates; synthesis produces a mangled-name
//! concrete decl per unique (template, args) tuple; the
//! synthesized decls flow through declare_user_type like any
//! user-written struct.
//!
//! v0.1 narrow scope:
//! - Discovery only in struct field positions (not fn / locus
//!   signatures or let ascriptions — those are m61b).
//! - Construction uses the **mangled name** explicitly
//!   (`Box_Int { value: 42 }`); inferring the mangle from a
//!   bare `Box { ... }` is m61b.
//! - Args must be primitives or non-generic user types (or
//!   themselves recursive generic instantiations).

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use lotus_codegen::build_executable;

fn unique_bin(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "lt-m61-{}-{}-{}",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = lotus_syntax::parse_source(source).expect("parse");
    let bin = unique_bin(name);
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

#[test]
fn generic_struct_referenced_in_field_monomorphizes() {
    // Holder has a field of type `Box<Int>`. Discovery sees
    // the reference, synthesizes `Box_Int`, and the user
    // constructs it via the mangled name. Reads through the
    // field also work because the synthesized struct has the
    // same field layout as `Box<T>` with T → Int.
    let src = r#"
        type Box<T> {
            value: T;
        }

        type Holder {
            b: Box<Int>;
            label: String;
        }

        fn main() {
            let inner = Box_Int { value: 42 };
            let h = Holder { b: inner, label: "wrapped" };
            println("h.b.value=", h.b.value, " h.label=", h.label);
        }
    "#;
    let (stdout, status) = build_and_run("box_int", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(
        stdout.contains("h.b.value=42 h.label=wrapped"),
        "got: {:?}",
        stdout,
    );
}

#[test]
fn distinct_generic_args_get_distinct_monomorphs() {
    // Two instantiations of the same template with different
    // args should produce two synthesized structs:
    // `Box_Int` and `Box_String`. Both work independently.
    let src = r#"
        type Box<T> {
            value: T;
        }

        type Pair {
            first: Box<Int>;
            second: Box<String>;
        }

        fn main() {
            let i = Box_Int { value: 7 };
            let s = Box_String { value: "ok" };
            let p = Pair { first: i, second: s };
            println("p.first.value=", p.first.value, " p.second.value=", p.second.value);
        }
    "#;
    let (stdout, status) = build_and_run("box_int_str", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(
        stdout.contains("p.first.value=7 p.second.value=ok"),
        "got: {:?}",
        stdout,
    );
}

// Nested generics like `Box<Box<Int>>` would exercise the
// recurse-into-args branch of collect_generic_uses, but the
// parser today lexes `>>` as a single `Shr` token rather than
// two `>` tokens in type-argument position. The codegen
// substrate for nested instantiation IS in place
// (collect_generic_uses recurses into args first so inner
// instantiations are declared before outer; mangle is
// recursive); the parser fix is m61b. Leaving this comment
// here as a marker — when `>>`-disambiguation lands, add a
// nested test.

// === m61b ====================================================
// Broader discovery (fn signatures, locus signatures, bus
// payloads, let ascriptions) + bare-name struct literal
// resolution via let ascription.

#[test]
fn bare_name_struct_literal_resolves_via_let_ascription() {
    // `let b: Box<Int> = Box { value: 42 };` should resolve
    // the bare `Box { ... }` to `Box_Int { ... }` because the
    // ascription names the instantiation.
    let src = r#"
        type Box<T> {
            value: T;
        }

        type Holder {
            b: Box<Int>;
        }

        fn main() {
            let b: Box<Int> = Box { value: 42 };
            let h = Holder { b: b };
            println("h.b.value=", h.b.value);
        }
    "#;
    let (stdout, status) = build_and_run("bare_let", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(
        stdout.contains("h.b.value=42"),
        "got: {:?}",
        stdout,
    );
}

#[test]
fn discovery_walks_fn_signature_param_and_return() {
    // Generic uses in fn params + return types should be
    // discovered without needing a struct field reference.
    let src = r#"
        type Box<T> {
            value: T;
        }

        fn make() -> Box<Int> {
            return Box_Int { value: 99 };
        }

        fn unwrap(b: Box<Int>) -> Int {
            return b.value;
        }

        fn main() {
            let b = make();
            let v = unwrap(b);
            println("v=", v);
        }
    "#;
    let (stdout, status) = build_and_run("fn_sig", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(stdout.contains("v=99"), "got: {:?}", stdout);
}

// Locus params with generic types — discovery walks them via
// collect_in_locus_member's Params branch — but lotus codegen
// requires every locus param to have a default value, and
// `Box<Int> = Box { value: 0 }` would need bare-name resolution
// to also apply in param-default context (not just let
// ascription). That extension is m61c-or-later. The discovery
// walk for the Params branch IS in place; locking it in via a
// passing test waits on the param-default bare-name path.

#[test]
fn discovery_walks_locus_lifecycle_signatures() {
    // Discovery covers locus lifecycle method signatures via
    // collect_in_locus_member's Lifecycle branch. We register
    // Box<Int> through the body's `let` ascription; the
    // lifecycle params themselves don't take a payload here
    // (lifecycle params are reserved for the implicit self),
    // so this exercises the body-walk path inside the
    // lifecycle.
    let src = r#"
        type Box<T> {
            value: T;
        }

        locus Demo {
            params {
                seed: Int = 0;
            }
            birth() {
                let b: Box<Int> = Box { value: 21 };
                println("birth saw b.value=", b.value);
            }
        }

        fn main() {
            Demo { };
        }
    "#;
    let (stdout, status) = build_and_run("locus_lifecycle", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(
        stdout.contains("birth saw b.value=21"),
        "got: {:?}",
        stdout,
    );
}

#[test]
fn discovery_walks_bus_payload_types() {
    // Bus subscribe + publish with generic payloads. Discovery
    // walks BusMember::Subscribe.ty + BusMember::Publish.ty;
    // m60's serializer-shape pass picks up the synthesized
    // Box_Int and emits __serialize_Box_Int / __deserialize_Box_Int.
    let src = r#"
        type Box<T> {
            value: T;
        }

        locus Sub {
            bus {
                subscribe "ev" as on_ev of type Box<Int>;
            }
            fn on_ev(b: Box<Int>) {
                println("got ", b.value);
            }
        }

        locus Pub {
            bus {
                publish "ev" of type Box<Int>;
            }
            birth() {
                let b: Box<Int> = Box { value: 100 };
                "ev" <- b;
            }
        }

        fn main() {
            Sub { };
            Pub { };
        }
    "#;
    let (stdout, status) = build_and_run("bus_payload", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(stdout.contains("got 100"), "got: {:?}", stdout);
}

#[test]
fn two_distinct_generic_types_monomorphize_independently() {
    // Two unrelated generic templates each get their own
    // monomorphization. Cross-type references still resolve
    // because the synthesis pass runs before any concrete
    // decl's fields are walked.
    let src = r#"
        type Box<T> {
            value: T;
        }
        type Tagged<U> {
            tag: String;
            payload: U;
        }

        type Combined {
            b: Box<Int>;
            t: Tagged<Int>;
        }

        fn main() {
            let b = Box_Int { value: 11 };
            let t = Tagged_Int { tag: "k", payload: 22 };
            let c = Combined { b: b, t: t };
            println("b.value=", c.b.value, " t.tag=", c.t.tag, " t.payload=", c.t.payload);
        }
    "#;
    let (stdout, status) = build_and_run("two_templates", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(
        stdout.contains("b.value=11 t.tag=k t.payload=22"),
        "got: {:?}",
        stdout,
    );
}
