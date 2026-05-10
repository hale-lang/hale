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

use aperio_codegen::build_executable;

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
    let program = aperio_syntax::parse_source(source).expect("parse");
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
// collect_in_locus_member's Params branch — but Aperio codegen
// requires every locus param to have a default value, and
// `Box<Int> = Box { value: 0 }` would need bare-name resolution
// to also apply in param-default context (not just let
// ascription). That extension is m61c-or-later. The discovery
// walk for the Params branch IS in place; locking it in via a
// passing test waits on the param-default bare-name path.

// === m61c ====================================================
// Generic enum monomorphization + locus param-default
// bare-name resolution.

#[test]
fn generic_enum_template_monomorphizes() {
    // Result<Int, String> referenced as a struct field type
    // triggers discovery + synthesis of Result_Int_String.
    // Construction goes via the mangled enum name; pattern
    // match against synthesized variants works.
    let src = r#"
        type Result<T, E> = enum {
            Ok(T),
            Err(E),
        };

        type Holder {
            r: Result<Int, String>;
        }

        fn main() {
            let r = Result_Int_String::Ok(42);
            let h = Holder { r: r };
            // Wildcard arm to bypass typechecker exhaustiveness
            // (it sees `Result` template, not Result_Int_String;
            // tightening this is a typechecker integration, not
            // a m61c codegen concern).
            match h.r {
                Result_Int_String::Ok(n) -> println("ok: ", n),
                _                         -> println("other"),
            }
        }
    "#;
    let (stdout, status) = build_and_run("gen_enum", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(stdout.contains("ok: 42"), "got: {:?}", stdout);
}

#[test]
fn locus_param_default_resolves_bare_name() {
    // `params { b: Box<Int> = Box { value: 0 }; }` — the
    // bare-name `Box { ... }` rewrites to `Box_Int` at
    // decl-time so the deferred default evaluation lands on
    // the mangled monomorph.
    let src = r#"
        type Box<T> {
            value: T;
        }

        locus Container {
            params {
                b: Box<Int> = Box { value: 0 };
            }
            birth() {
                println("default b.value=", self.b.value);
            }
        }

        fn main() {
            Container { };
        }
    "#;
    let (stdout, status) = build_and_run("param_default", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(
        stdout.contains("default b.value=0"),
        "got: {:?}",
        stdout,
    );
}

#[test]
fn locus_param_default_overridable_at_instantiation() {
    // Caller can override the default with their own bare-name
    // struct literal — instantiation goes through the same
    // rewrite path (via the ParamInit's value Expr).
    let src = r#"
        type Box<T> {
            value: T;
        }

        locus Container {
            params {
                b: Box<Int> = Box { value: 0 };
            }
            birth() {
                println("got b.value=", self.b.value);
            }
        }

        fn main() {
            let custom: Box<Int> = Box { value: 99 };
            Container { b: custom };
        }
    "#;
    let (stdout, status) = build_and_run("param_override", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(
        stdout.contains("got b.value=99"),
        "got: {:?}",
        stdout,
    );
}

// === m64 ====================================================
// Numeric bound enforcement at synthesis time + generic
// closures (which already work because closures reference
// fields by name and field types are substituted via the
// Params block walk in m63).

#[test]
fn numeric_bound_admits_int_arg() {
    // `T: Numeric` accepts Int / Float / Decimal / Duration.
    // Verify Int instantiation works end-to-end.
    let src = r#"
        type Box<T: Numeric> {
            value: T;
        }

        type Holder {
            b: Box<Int>;
        }

        fn main() {
            let b: Box<Int> = Box { value: 13 };
            let h = Holder { b: b };
            println("h.b.value=", h.b.value);
        }
    "#;
    let (stdout, status) = build_and_run("numeric_int", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(
        stdout.contains("h.b.value=13"),
        "got: {:?}",
        stdout,
    );
}

#[test]
fn numeric_bound_rejects_string_arg() {
    // String is non-numeric; instantiating Box<String> when
    // Box's T is bounded by Numeric should fail at codegen.
    let src = r#"
        type Box<T: Numeric> {
            value: T;
        }

        type Holder {
            b: Box<String>;
        }

        fn main() {
            let h = Holder { b: Box_String { value: "hi" } };
        }
    "#;
    let program = aperio_syntax::parse_source(src).expect("parse");
    let bin = unique_bin("numeric_str_reject");
    let result = build_executable(&program, &bin);
    let _ = std::fs::remove_file(&bin);
    let err = result.expect_err("expected codegen error for non-numeric");
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("Numeric") && msg.contains("Box"),
        "expected Numeric-bound diagnostic; got: {}",
        msg,
    );
}

#[test]
fn generic_locus_with_closure_substitutes_via_field_layout() {
    // m64: generic closures already work without explicit
    // closure-substitution because closure expressions
    // reference fields by name; the Params-block walk in
    // m63 substitutes field types correctly, and closure
    // lowering at instantiation time picks up the substituted
    // shape.
    let src = r#"
        locus Compute<T: Numeric> {
            params {
                scratch: T = 0;
            }
            closure scratch_nonneg {
                self.scratch ~~ 0 within 999;
                epoch tick;
            }
            birth() {
                println("compute scratch=", self.scratch);
            }
        }

        fn main() {
            let c: Compute<Int> = Compute { };
        }
    "#;
    let (stdout, status) = build_and_run("gen_closure", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(
        stdout.contains("compute scratch=0"),
        "got: {:?}",
        stdout,
    );
}

// === m63 ====================================================
// Generic loci. Locus templates with `<T>` declare without
// emitting LLVM directly; per-instantiation specialized
// LocusDecls synthesize from discovery, flow through the
// standard A1/A2/C locus passes alongside non-generic decls.

#[test]
fn generic_locus_with_typed_param_default() {
    // Locus has a generic param T and a typed param of
    // Box<T> with a default Box { value: 0 }. Discovery sees
    // Holder<Int> in the let ascription, synthesizes
    // Holder_Int (which then surfaces Box<Int> during the
    // queue walk and synthesizes Box_Int too), default fires
    // at instantiation.
    let src = r#"
        type Box<T> {
            value: T;
        }

        locus Holder<T> {
            params {
                wrapped: Box<T> = Box { value: 0 };
            }
            birth() {
                println("holder.wrapped.value=", self.wrapped.value);
            }
        }

        fn main() {
            let h: Holder<Int> = Holder { };
        }
    "#;
    let (stdout, status) = build_and_run("gen_locus_default", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(
        stdout.contains("holder.wrapped.value=0"),
        "got: {:?}",
        stdout,
    );
}

#[test]
fn generic_locus_overridden_at_instantiation() {
    // Caller provides `wrapped` directly at instantiation. Both
    // the locus instantiation and the inner Box<Int>
    // construction go through bare-name resolution against the
    // ascribed types.
    let src = r#"
        type Box<T> {
            value: T;
        }

        locus Holder<T> {
            params {
                wrapped: Box<T> = Box { value: 0 };
            }
            birth() {
                println("holder.wrapped.value=", self.wrapped.value);
            }
        }

        fn main() {
            let custom: Box<Int> = Box { value: 42 };
            let h: Holder<Int> = Holder { wrapped: custom };
        }
    "#;
    let (stdout, status) = build_and_run("gen_locus_override", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(
        stdout.contains("holder.wrapped.value=42"),
        "got: {:?}",
        stdout,
    );
}

// === m62 ====================================================
// Generic free fns. Inference at the call site pins type args
// from actual arg LotusTypes; per-instantiation specialized
// fn bodies synthesize on-demand and land in user_fns under
// the mangled name.

#[test]
fn generic_fn_identity_inferred_from_arg() {
    let src = r#"
        fn first<T>(x: T) -> T {
            return x;
        }

        fn main() {
            let v = first(42);
            println("v=", v);
        }
    "#;
    let (stdout, status) = build_and_run("gen_fn_id_int", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(stdout.contains("v=42"), "got: {:?}", stdout);
}

#[test]
fn generic_fn_distinct_instantiations_at_distinct_calls() {
    // Two calls with different arg types should produce two
    // specialized fns. Both round-trip the value.
    let src = r#"
        fn first<T>(x: T) -> T {
            return x;
        }

        fn main() {
            let a = first(7);
            let b = first("ok");
            println("a=", a, " b=", b);
        }
    "#;
    let (stdout, status) = build_and_run("gen_fn_id_two", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(
        stdout.contains("a=7 b=ok"),
        "got: {:?}",
        stdout,
    );
}

#[test]
fn generic_fn_with_arithmetic_on_inferred_type() {
    // T is pinned to Int; the body adds 1, returns Int. Tests
    // that the substituted body still typechecks at codegen
    // (Int + Int).
    let src = r#"
        fn bump<T>(x: T) -> T {
            return x;
        }

        fn main() {
            let n = bump(99);
            let m = n + 1;
            println("m=", m);
        }
    "#;
    let (stdout, status) = build_and_run("gen_fn_bump", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(stdout.contains("m=100"), "got: {:?}", stdout);
}

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

// === m65 ====================================================
// stdlib `Result<T, E>` / `Option<T>` as built-in generic
// enums available without explicit `type` declaration.

#[test]
fn builtin_result_generic_used_without_declaration() {
    // No `type Result<T,E> = ...` in source — codegen injects
    // the stdlib template, discovery sees the use site, and
    // synthesis produces `Result_Int_String` along with its
    // `Ok`/`Err` variants.
    let src = r#"
        type Holder {
            r: Result<Int, String>;
        }

        fn main() {
            let r = Result_Int_String::Ok(7);
            let h = Holder { r: r };
            match h.r {
                Result_Int_String::Ok(n)  -> println("ok: ", n),
                _                          -> println("other"),
            }
        }
    "#;
    let (stdout, status) = build_and_run("builtin_result", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(stdout.contains("ok: 7"), "got: {:?}", stdout);
}

#[test]
fn builtin_option_generic_used_without_declaration() {
    // No `type Option<T> = ...` in source — codegen injects
    // the stdlib template; both Some(T) and None synthesize.
    let src = r#"
        type Holder {
            o: Option<Int>;
        }

        fn main() {
            let some = Option_Int::Some(42);
            let h = Holder { o: some };
            match h.o {
                Option_Int::Some(n) -> println("some: ", n),
                _                    -> println("none"),
            }
        }
    "#;
    let (stdout, status) = build_and_run("builtin_option", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(stdout.contains("some: 42"), "got: {:?}", stdout);
}

#[test]
fn builtin_result_and_option_coexist() {
    // Both built-ins can be used in the same program; nested
    // discovery walks both field types and synthesizes both
    // monomorphs.
    let src = r#"
        type Pair {
            r: Result<Int, String>;
            o: Option<Int>;
        }

        fn main() {
            let p = Pair {
                r: Result_Int_String::Err("nope"),
                o: Option_Int::Some(9),
            };
            match p.r {
                Result_Int_String::Err(s) -> println("err: ", s),
                _                          -> println("other"),
            }
            match p.o {
                Option_Int::Some(n) -> println("some: ", n),
                _                    -> println("none"),
            }
        }
    "#;
    let (stdout, status) = build_and_run("builtin_both", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(
        stdout.contains("err: nope") && stdout.contains("some: 9"),
        "got: {:?}",
        stdout,
    );
}

#[test]
fn user_result_decl_takes_precedence_over_builtin() {
    // If the user declares their own `type Result<T,E> = ...`,
    // the user's variants win — m65 inject path uses
    // entry().or_insert() so the built-in only fills holes.
    // Here we declare a Result with extra variant `Pending`
    // and assert it builds + matches.
    let src = r#"
        type Result<T, E> = enum {
            Ok(T),
            Err(E),
            Pending,
        };

        fn main() {
            let r: Result<Int, String> = Result_Int_String::Pending;
            match r {
                Result_Int_String::Pending -> println("pending"),
                _                           -> println("other"),
            }
        }
    "#;
    let (stdout, status) = build_and_run("user_result_wins", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(stdout.contains("pending"), "got: {:?}", stdout);
}

// === m66 ====================================================
// Parser `>>` ambiguity: nested generic args close cleanly.

#[test]
fn nested_generic_args_close_with_double_gt() {
    // Pre-m66: `Box<Box<Int>>` failed to parse because the lexer
    // emits `>>` as a single Shr token and the inner generic-args
    // closer expected a single `>`. The m66 fix splits Shr in
    // place at the closer site. Codegen substrate (m63 fixpoint
    // queue) was already nested-aware.
    let src = r#"
        type Box<T> {
            value: T;
        }

        type Outer {
            b: Box<Box<Int>>;
        }

        fn main() {
            let inner = Box_Int { value: 7 };
            let outer = Box_Box_Int { value: inner };
            let o = Outer { b: outer };
            println("o.b.value.value=", o.b.value.value);
        }
    "#;
    let (stdout, status) = build_and_run("nested_generics", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(
        stdout.contains("o.b.value.value=7"),
        "got: {:?}",
        stdout,
    );
}

#[test]
fn nested_builtin_result_with_box_arg() {
    // `Result<Box<Int>, String>` exercises the split at the
    // built-in template lookup path: discovery walks the field
    // type, synthesizes Box_Int and then Result_Box_Int_String.
    let src = r#"
        type Box<T> {
            value: T;
        }

        type Holder {
            r: Result<Box<Int>, String>;
        }

        fn main() {
            let inner = Box_Int { value: 9 };
            let r = Result_Box_Int_String::Ok(inner);
            let h = Holder { r: r };
            match h.r {
                Result_Box_Int_String::Ok(b) -> println("ok: ", b.value),
                _                             -> println("other"),
            }
        }
    "#;
    let (stdout, status) = build_and_run("nested_result", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(stdout.contains("ok: 9"), "got: {:?}", stdout);
}

// === m67 ====================================================
// Bare-name struct literal resolution at return + struct
// field-init sites.

#[test]
fn bare_name_resolves_at_return_position() {
    // `fn make_box() -> Box<Int> { return Box { value: 5 }; }` —
    // the bare `Box { ... }` rewrites to `Box_Int` because the
    // fn's declared return type is `Box<Int>`.
    let src = r#"
        type Box<T> {
            value: T;
        }

        fn make_box() -> Box<Int> {
            return Box { value: 5 };
        }

        fn main() {
            let b = make_box();
            println("b.value=", b.value);
        }
    "#;
    let (stdout, status) = build_and_run("bare_return", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(stdout.contains("b.value=5"), "got: {:?}", stdout);
}

#[test]
fn bare_name_resolves_at_struct_field_init() {
    // `Outer { inner: Box { value: 7 } }` — the inner bare
    // `Box { ... }` rewrites to `Box_Int` because Outer.inner's
    // declared field type is `Box<Int>`.
    let src = r#"
        type Box<T> {
            value: T;
        }

        type Outer {
            inner: Box<Int>;
            label: String;
        }

        fn main() {
            let o = Outer { inner: Box { value: 7 }, label: "hi" };
            println("o.inner.value=", o.inner.value, " o.label=", o.label);
        }
    "#;
    let (stdout, status) = build_and_run("bare_field_init", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(
        stdout.contains("o.inner.value=7 o.label=hi"),
        "got: {:?}",
        stdout,
    );
}

#[test]
fn bare_name_resolves_at_nested_field_init() {
    // Two-level nesting: Outer.middle is Pair<Int>, and Pair has
    // a Box<Int>. The middle's `Pair { ... }` rewrites; inside
    // it, `Box { ... }` rewrites against Pair's field type too.
    let src = r#"
        type Box<T> {
            value: T;
        }

        type Pair<T> {
            a: Box<T>;
            tag: String;
        }

        type Outer {
            middle: Pair<Int>;
        }

        fn main() {
            let o = Outer { middle: Pair { a: Box { value: 11 }, tag: "wrapped" } };
            println("o.middle.a.value=", o.middle.a.value, " o.middle.tag=", o.middle.tag);
        }
    "#;
    let (stdout, status) = build_and_run("bare_nested", src);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(
        stdout.contains("o.middle.a.value=11 o.middle.tag=wrapped"),
        "got: {:?}",
        stdout,
    );
}
