//! GH #911 B5: `hale check` and `hale build` must agree on a
//! generic instantiation.
//!
//! The checker had no monomorph path for a generic LOCUS at all, and
//! no path for a literal spelled with a generic TEMPLATE name at a
//! `let` ascription or a return slot — so `let h: Holder<Int> =
//! Holder { };` and `let c: Cache<Int, String> = Cache { cap: 2 };`
//! were refused ("expected `Holder_Int`, got `Holder`") while
//! `build_executable` lowered, linked and ran them. `generics.rs`
//! only ever exercised the build half, which is why the whole family
//! was invisible: it uses `build_executable`, which skips the
//! checker.
//!
//! The other half of agreement is the sites codegen CANNOT lower: a
//! literal with no declared type anywhere to take its type arguments
//! from died at build with an unlocated `expression form
//! Discriminant(12)`, and a generic locus's mangled monomorph name
//! (`Cache_Int_String { }`) dies in the ownership pre-pass (F.39).
//! Those are located check errors now.
//!
//! This file is the SITE-BY-SITE table the checker's comments point
//! at. It lives in `hale-codegen` rather than `hale-types` because
//! agreement is the assertion: `hale-types` has no dev-dependency on
//! codegen, so a test there could only check half of it.

use std::process::Command;

use hale_codegen::build_executable;
use hale_syntax::parse_source;
use hale_types::check_program;

#[path = "support/harness.rs"]
mod harness;

/// What a shape is supposed to do at BOTH layers.
enum Expect {
    /// Checks clean, builds, and prints this on stdout.
    Runs(&'static str),
    /// Refused by the checker with a message containing this, and
    /// refused by the build too.
    Refused(&'static str),
}

/// Check, build, and (when it builds) run — asserting the
/// expectation AND the invariant that the two layers agree.
fn agree(tag: &str, src: &str, expect: &Expect) {
    let program = parse_source(src).unwrap_or_else(|e| {
        panic!("[{tag}] parse: {e:?}");
    });
    let diags = check_program(&program);
    let messages: Vec<String> =
        diags.iter().map(|d| d.message.clone()).collect();
    let bin = harness::unique_bin(&format!("hale_test_genmono_{tag}"));
    let built = build_executable(&program, &bin);
    // The invariant, stated once and independent of the row: a
    // program the checker passes must build, and a program it
    // refuses must not build.
    assert_eq!(
        diags.is_empty(),
        built.is_ok(),
        "[{tag}] check and build disagree — check: {:?}, build: {:?}",
        messages,
        built.as_ref().err()
    );
    match expect {
        Expect::Runs(wanted) => {
            assert!(
                diags.is_empty(),
                "[{tag}] expected to check clean, got {:?}",
                messages
            );
            built.unwrap_or_else(|e| panic!("[{tag}] build: {e:?}"));
            let out = Command::new(&bin)
                .output()
                .unwrap_or_else(|e| panic!("[{tag}] run: {e:?}"));
            let _ = std::fs::remove_file(&bin);
            assert!(out.status.success(), "[{tag}] exit: {:?}", out.status);
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                stdout.contains(wanted),
                "[{tag}] stdout {:?} lacks {:?}",
                stdout,
                wanted
            );
        }
        Expect::Refused(wanted) => {
            let _ = std::fs::remove_file(&bin);
            assert!(
                messages.iter().any(|m| m.contains(wanted)),
                "[{tag}] expected a diagnostic containing {:?}, got {:?}",
                wanted,
                messages
            );
        }
    }
}

// === the sites codegen lowers: check must accept them =========
// Each one builds and runs on `main` today; only `check` refused.

#[test]
fn let_ascription_pins_a_generic_struct() {
    agree(
        "let_struct",
        r#"
type Box<T> {
    value: T = 0;
}

fn main() {
    let b: Box<Int> = Box { value: 1 };
    println("v=", b.value);
}
"#,
        &Expect::Runs("v=1"),
    );
}

#[test]
fn let_ascription_pins_a_generic_locus() {
    agree(
        "let_locus",
        r#"
locus Cache<K, V> {
    params {
        cap: Int = 1;
    }
    birth() {
        println("birth cap=", self.cap);
    }
}

fn main() {
    let c: Cache<Int, String> = Cache { cap: 2 };
    println("read cap=", c.cap);
}
"#,
        &Expect::Runs("read cap=2"),
    );
}

#[test]
fn a_generic_locus_substitutes_into_its_params() {
    agree(
        "let_locus_generic_param",
        r#"
type Box<T> {
    value: T = 0;
}

locus Holder<T> {
    params {
        wrapped: Box<T> = Box { value: 7 };
    }
    birth() {
        println("wrapped=", self.wrapped.value);
    }
}

fn main() {
    let h: Holder<Int> = Holder { };
}
"#,
        &Expect::Runs("wrapped=7"),
    );
}

#[test]
fn a_return_slot_pins_the_arguments() {
    agree(
        "return_struct",
        r#"
type Box<T> {
    value: T = 0;
}

fn make() -> Box<Int> {
    return Box { value: 4 };
}

fn main() {
    let b: Box<Int> = make();
    println("v=", b.value);
}
"#,
        &Expect::Runs("v=4"),
    );
}

#[test]
fn a_return_slot_pins_a_generic_locus_too() {
    agree(
        "return_locus",
        r#"
locus Cache<K, V> {
    params {
        cap: Int = 1;
    }
    birth() {
        println("birth");
    }
}

fn make() -> Cache<Int, String> {
    return Cache { cap: 3 };
}

fn main() {
    let c: Cache<Int, String> = make();
    println("cap=", c.cap);
}
"#,
        &Expect::Runs("cap=3"),
    );
}

#[test]
fn a_struct_literal_field_init_pins_the_arguments() {
    agree(
        "field_init",
        r#"
type Box<T> {
    value: T = 0;
}

type Outer {
    inner: Box<Int> = Box { value: 0 };
}

fn main() {
    let o: Outer = Outer { inner: Box { value: 9 } };
    println("v=", o.inner.value);
}
"#,
        &Expect::Runs("v=9"),
    );
}

#[test]
fn several_arguments_mangle_in_order() {
    agree(
        "two_args",
        r#"
type Pair<A, B> {
    a: A = 0;
    b: B = false;
}

fn main() {
    let p: Pair<Int, Bool> = Pair { a: 1, b: true };
    println("a=", p.a, " b=", p.b);
}
"#,
        &Expect::Runs("a=1 b=true"),
    );
}

// === the sites codegen cannot lower: check must refuse them ====

#[test]
fn an_unannotated_generic_literal_asks_for_the_arguments() {
    // The pinned answer to "what does `Box { value: 1 }` mean on its
    // own": nothing. Codegen infers the arguments from the declared
    // type at the site and from nowhere else — there is no path from
    // the literal's fields back to `T` — so the rule is "say them".
    agree(
        "unannotated_let",
        r#"
type Box<T> {
    value: T = 0;
}

fn main() {
    let b = Box { value: 1 };
    println("v=", b.value);
}
"#,
        &Expect::Refused(
            "`Box` is a generic type: a literal spelled with the \
             template name takes its type arguments from the declared \
             type at the site",
        ),
    );
}

#[test]
fn a_generic_literal_in_statement_position_asks_too() {
    agree(
        "unannotated_stmt",
        r#"
locus Cache<K, V> {
    params {
        cap: Int = 1;
    }
    birth() {
        println("cap=", self.cap);
    }
}

fn main() {
    Cache { cap: 2 };
}
"#,
        &Expect::Refused("`Cache` is a generic locus"),
    );
}

#[test]
fn a_generic_locus_monomorph_name_is_not_a_spelling() {
    // `Cache_Int_String` is the name codegen SYNTHESIZES, and it is
    // not instantiable under that spelling at any use site: the
    // ownership pre-pass never numbers the node (F.39), whether or
    // not the monomorph was discovered elsewhere. The struct twin
    // (`Box_Int { }`) IS lowerable and stays accepted — see
    // `generics.rs`.
    agree(
        "mangled_locus",
        r#"
locus Cache<K, V> {
    params {
        cap: Int = 1;
    }
    birth() {
        println("cap=", self.cap);
    }
}

fn main() {
    let c: Cache_Int_String = Cache_Int_String { cap: 2 };
}
"#,
        &Expect::Refused(
            "`Cache_Int_String` is the compiler's name for the generic \
             locus `Cache<Int, String>`",
        ),
    );
}

#[test]
fn a_wrong_arity_instantiation_is_a_located_error() {
    agree(
        "arity_type",
        r#"
type Box<T> {
    value: T = 0;
}

fn main() {
    let b: Box<Int, String> = Box { value: 1 };
}
"#,
        &Expect::Refused("generic type `Box` takes 1 type argument, not 2"),
    );
    agree(
        "arity_locus",
        r#"
locus Cache<K, V> {
    params {
        cap: Int = 1;
    }
    birth() {
        println("cap=", self.cap);
    }
}

fn main() {
    let c: Cache<Int> = Cache { cap: 2 };
}
"#,
        &Expect::Refused("generic locus `Cache` takes 2 type arguments, not 1"),
    );
}

#[test]
fn the_new_diagnostics_point_at_the_source_they_are_about() {
    // An unlocated diagnostic is the failure mode this whole family
    // is about (the build's own errors carry no span), so pin the
    // span to the exact text, not just to "some span".
    let src = r#"
type Box<T> {
    value: T = 0;
}

fn main() {
    let b = Box { value: 1 };
}
"#;
    let program = parse_source(src).expect("parse");
    let diags = check_program(&program);
    let hit = diags
        .iter()
        .find(|d| d.message.contains("is a generic type"))
        .expect("the un-annotated literal is reported");
    assert!(
        src[hit.span.start.as_usize()..].starts_with("Box { value: 1 }"),
        "span points at {:?}",
        &src[hit.span.start.as_usize()
            ..(hit.span.start.as_usize() + 20).min(src.len())]
    );

    let src = r#"
locus Cache<K, V> {
    params {
        cap: Int = 1;
    }
    birth() {
        println("cap=", self.cap);
    }
}

fn main() {
    let c: Cache<Int> = Cache { cap: 2 };
}
"#;
    let program = parse_source(src).expect("parse");
    let diags = check_program(&program);
    let hit = diags
        .iter()
        .find(|d| d.message.contains("takes 2 type arguments"))
        .expect("the wrong arity is reported");
    assert!(
        src[hit.span.start.as_usize()..].starts_with("Cache<Int>"),
        "span points at {:?}",
        &src[hit.span.start.as_usize()
            ..(hit.span.start.as_usize() + 20).min(src.len())]
    );
}

// === the sites that were already agreed, kept as a ratchet =====
// Each of these looks like one of the shapes above and is decided
// the other way by codegen; a future widening of the rule that
// forgets one of them turns the agreement assertion red here
// instead of shipping.

#[test]
fn a_generic_locus_as_a_locus_param_default_stays_refused() {
    // Discovery does not reach a generic LOCUS through another
    // locus's params block in time to declare it, so the build
    // refuses with "not synthesized — discovery missed the use
    // site". The generic-STRUCT twin below is fine, which is why
    // the tolerance is per-site rather than per-question.
    agree(
        "locus_param_default_locus",
        r#"
locus Cache<K, V> {
    params {
        cap: Int = 1;
    }
    birth() {
        println("cap=", self.cap);
    }
}

locus App {
    params {
        c: Cache<Int, String> = Cache { cap: 7 };
    }
    birth() {
        println("app");
    }
}

fn main() {
    let a: App = App { };
}
"#,
        &Expect::Refused("param `c`: declared `Cache_Int_String`"),
    );
}

#[test]
fn a_generic_struct_as_a_locus_param_default_stays_accepted() {
    agree(
        "locus_param_default_struct",
        r#"
type Box<T> {
    value: T = 0;
}

locus L {
    params {
        b: Box<Int> = Box { value: 3 };
    }
    birth() {
        println("v=", self.b.value);
    }
}

fn main() {
    let l: L = L { };
}
"#,
        &Expect::Runs("v=3"),
    );
}

#[test]
fn a_generic_literal_at_a_locus_literals_field_stays_refused() {
    // The struct-literal twin (`Outer { inner: Box { value: 9 } }`)
    // builds; codegen's locus-literal path rewrites no bare generic
    // name, so this one does not.
    agree(
        "locus_literal_field",
        r#"
type Box<T> {
    value: T = 0;
}

locus L {
    params {
        b: Box<Int> = Box { value: 0 };
    }
    birth() {
        println("v=", self.b.value);
    }
}

fn main() {
    let l: L = L { b: Box { value: 8 } };
}
"#,
        &Expect::Refused("locus `L`: field `b` expects `Box_Int`, got `Box`"),
    );
}

#[test]
fn a_generic_literal_as_a_call_argument_stays_refused() {
    agree(
        "call_arg",
        r#"
type Box<T> {
    value: T = 0;
}

fn take(b: Box<Int>) {
    println("v=", b.value);
}

fn main() {
    take(Box { value: 5 });
}
"#,
        &Expect::Refused("argument 0 type mismatch"),
    );
}

#[test]
fn a_generic_struct_monomorph_name_stays_a_spelling() {
    // The mangled name IS instantiable for a generic TYPE, once the
    // monomorph has been discovered — `generics.rs` has a dozen
    // programs that rely on it. The B5 rule must not take it away.
    agree(
        "mangled_struct",
        r#"
type Box<T> {
    value: T = 0;
}

type Holder {
    b: Box<Int> = Box { value: 0 };
}

fn main() {
    let inner = Box_Int { value: 42 };
    println("v=", inner.value);
}
"#,
        &Expect::Runs("v=42"),
    );
}

#[test]
fn an_alias_of_an_instantiation_stays_a_spelling() {
    agree(
        "alias_monomorph",
        r#"
type Box<T> {
    value: T = 0;
}

type IntBox = Box<Int>;

fn main() {
    let b: IntBox = IntBox { value: 6 };
    println("v=", b.value);
}
"#,
        &Expect::Runs("v=6"),
    );
}
