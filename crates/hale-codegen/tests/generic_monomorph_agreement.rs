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

#[path = "../../hale-types/tests/support/entries.rs"]
mod entries;
use std::process::Command;

use hale_frontend::snapshot::{Config, Snapshot, Target};
use hale_types::placement::HoleKind;

use hale_syntax::parse_source;
use entries::check_program;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

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
    let built = build_opts::build_source(src, &bin, &build_opts::options());
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
    // ownership pre-pass never gives the node a row (F.39), whether or
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
fn a_generic_literal_at_a_locus_literals_field_uses_its_declared_type() {
    // The explicit field override takes its generic arguments from
    // the declared field type, like a param default.
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
        &Expect::Runs("v=8"),
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

#[test]
fn generic_locus_calls_select_each_concrete_body() {
    agree(
        "typed_locus_calls",
        r#"
fn first<T>(x: T) -> T { return x; }
fn twice<T>(x: T) -> T { return first(first(x)); }
fn external(n: Int = first(7)) { println(n); }
locus Holder<T> {
    params { v: T; }
    run() {
        external();
        println(first(self.v));
        let x = self.v;
        println(twice(x));
    }
}
fn main() {
    let i: Holder<Int> = Holder { v: 42 };
    let s: Holder<String> = Holder { v: "text" };
}
"#,
        &Expect::Runs("7\n42\n42\n7\ntext\ntext\n"),
    );
}

#[test]
fn generic_fn_discovers_locus_calls_and_defaults_keep_their_declaring_context() {
    agree(
        "typed_locus_defaults",
        r#"
fn first<T>(x: T) -> T { return x; }
locus Holder<T> {
    params { v: T; copied: T = first(self.v); }
    fn read() -> T { return first(self.copied); }
    run() { println(first(self.read())); }
}
fn spawn<T>(x: T) {
    let h: Holder<T> = Holder { v: first(x) };
}
fn main() {
    spawn(42);
    spawn("text");
}
"#,
        &Expect::Runs("42\ntext\n"),
    );
}

#[test]
fn function_and_method_defaults_use_each_caller_scope() {
    agree(
        "typed_caller_defaults",
        r#"
fn first<T>(x: T) -> T { return x; }
fn show(n: String = to_string(first(value))) { println(n); }
locus Display {
    fn show(n: String = to_string(first(value))) { println(n); }
}
fn caller<T>(value: T) { show(); }
fn int_caller() {
    let value = 42;
    show();
    let d = Display { };
    d.show();
}
fn string_caller() {
    let value = "text";
    show();
    let d = Display { };
    d.show();
}
fn main() {
    int_caller();
    string_caller();
    caller(7);
    caller("other");
    show("supplied");
}
"#,
        &Expect::Runs("42\n42\ntext\ntext\n7\nother\nsupplied\n"),
    );
}

#[test]
fn nested_defaults_keep_the_outer_invocation_and_helpers_keep_their_own_body() {
    agree(
        "typed_nested_defaults",
        r#"
fn first<T>(x: T) -> T { return x; }
fn twice<T>(x: T) -> T { return first(first(x)); }
fn read(n: Int = twice(value)) -> Int { return n; }
fn nested(n: Int = read()) -> Int { return n; }
fn main() {
    let value = 42;
    println(nested());
    { let value = 7; println(nested()); }
}
"#,
        &Expect::Runs("42\n7\n"),
    );
}

#[test]
fn defaults_use_generic_locus_callers_and_addressed_fallible_calls() {
    agree(
        "typed_specialized_defaults",
        r#"
type Fault { n: Int; }
fn first<T>(x: T) -> T { return x; }
fn show(n: String = to_string(first(value))) { println(n); }
fn read(n: Int = first(value)) -> Int fallible(Fault) { return n; }
locus Holder<T> {
    params { v: T; }
    fn show(n: String = to_string(first(value))) { println(n); }
    run() { let value = self.v; show(); self.show(); }
}
fn main() {
    let value = 9;
    println(read() or - 1);
    let i: Holder<Int> = Holder { v: 42 };
    let s: Holder<String> = Holder { v: "text" };
}
"#,
        &Expect::Runs("9\n42\n42\ntext\ntext\n"),
    );
}

/// The same program built with AddressSanitizer and run with the arena's
/// chunk recycling off: what it prints, after a clean exit.
fn asan_runs(tag: &str, src: &str, wanted: &str) {
    let bin = harness::unique_bin(&format!("hale_test_genmono_asan_{tag}"));
    harness::build_source_asan(src, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .unwrap_or_else(|e| panic!("[{tag}] asan run: {e:?}"));
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "[{tag}] asan exit: {:?}; stderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains(wanted), "[{tag}] asan stdout {stdout:?} lacks {wanted:?}");
}

// === an instantiation over a user record ======================
// A monomorph was declared before the program's own records, so one
// whose field is a user record (`inner: In`, `[In; 2]`) never
// resolved: "generic-type dependency cycle or unresolvable dep among
// synthesized monomorphs: [\"Holder_In\"]". `Holder<Int>` built.

const ARRAY_OF_THE_PARAMETER: &str = r#"
type In {
    k: Int = 0;
    label: String = "";
}

type Holder<T> {
    items: [T; 2];
}

fn main() {
    let h: Holder<In> = Holder { items: [In { k: 1, label: "a" }, In { k: 2, label: "b" }] };
    let n: Holder<Int> = Holder { items: [3, 4] };
    println(h.items[1].label, h.items[0].k, n.items[1]);
}
"#;

#[test]
fn an_array_of_the_parameter_holds_a_user_record() {
    agree("array_of_param", ARRAY_OF_THE_PARAMETER, &Expect::Runs("b14"));
    asan_runs("array_of_param", ARRAY_OF_THE_PARAMETER, "b14");
}

const ARRAY_OF_AN_INSTANTIATION: &str = r#"
type In {
    k: Int = 0;
    label: String = "";
}

type Holder<T> {
    items: [T; 2];
}

type Shelf<T> {
    rows: [Holder<T>; 2];
}

type Wrap {
    h: Holder<In>;
    tag: String = "";
}

type Box<T> {
    v: T;
}

fn main() {
    let a: Holder<In> = Holder { items: [In { k: 1, label: "a" }, In { k: 2, label: "b" }] };
    let b: Holder<In> = Holder { items: [In { k: 3, label: "c" }, In { k: 4, label: "d" }] };
    let s: Shelf<In> = Shelf { rows: [a, b] };
    println(s.rows[1].items[0].label, s.rows[1].items[0].k);
    let w = Wrap { h: s.rows[0], tag: "w" };
    let x: Box<Wrap> = Box { v: w };
    println(x.v.tag, x.v.h.items[1].label);
}
"#;

#[test]
fn an_array_of_an_instantiation_nests() {
    // `Shelf<In>`'s rows are `Holder<In>`s, and `Box<Wrap>` holds a
    // record that holds a monomorph: each resolves by name, whatever
    // order the declarations land in.
    agree("array_of_instantiation", ARRAY_OF_AN_INSTANTIATION, &Expect::Runs("c3\nwb\n"));
    asan_runs("array_of_instantiation", ARRAY_OF_AN_INSTANTIATION, "c3\nwb\n");
}

const LOCUS_ARRAY_OF_THE_PARAMETER: &str = r#"
type In {
    k: Int = 0;
    label: String = "";
}

locus Bag<T> {
    params {
        items: [T; 2];
    }
    fn second() -> T {
        return self.items[1];
    }
}

locus Rack {
    params {
        bag: Bag<In>;
    }
    run() {
        let s = self.bag.second();
        println(s.label, s.k);
    }
}

locus Tally {
    params {
        bag: Bag<Int>;
    }
    run() {
        println(self.bag.second());
    }
}

fn main() {
    let r = Rack { bag: Bag { items: [In { k: 1, label: "x" }, In { k: 2, label: "y" }] } };
    let t = Tally { bag: Bag { items: [5, 6] } };
}
"#;

#[test]
fn a_generic_locus_holds_an_array_of_its_parameter() {
    agree("locus_array_of_param", LOCUS_ARRAY_OF_THE_PARAMETER, &Expect::Runs("y2\n6\n"));
    asan_runs("locus_array_of_param", LOCUS_ARRAY_OF_THE_PARAMETER, "y2\n6\n");
}

// === an assignment's place supplies the arguments ==============
// `self.cfg = Cfg { n: 2 };` with `cfg: Cfg<Int>` was refused by the
// checker ("assignment: target type `Cfg_Int` not assignable from
// `Cfg`"), and the build, which skips the checker, died unlocated at
// `expression form Discriminant(12)`. The place's declared type
// supplies the arguments, as a `let` ascription's does.

const ASSIGNED_RECORDS: &str = r#"
type Cfg<T> {
    n: T;
    label: String = "";
}

type Outer {
    c: Cfg<Int>;
}

locus Keeper {
    params {
        cfg: Cfg<Int> = Cfg { n: 1, label: "one" };
    }
    run() {
        self.cfg = Cfg { n: 2, label: "two" };
        println(self.cfg.label, self.cfg.n);
    }
}

fn main() {
    let k = Keeper { };
    let mut c: Cfg<Int> = Cfg { n: 1, label: "one" };
    c = Cfg { n: 3, label: "three" };
    println(c.label, c.n);
    let o = Outer { c: Cfg { n: 1, label: "one" } };
    o.c = Cfg { n: 4, label: "four" };
    println(o.c.label, o.c.n);
    let a: Cfg<Int> = Cfg { n: 1, label: "a" };
    let b: Cfg<Int> = Cfg { n: 2, label: "b" };
    let mut cs = [a, b];
    cs[1] = Cfg { n: 5, label: "five" };
    println(cs[1].label, cs[1].n);
}
"#;

#[test]
fn an_assignment_takes_the_arguments_from_its_place() {
    // A locus's field, a local, a record's field, an array's element.
    let wanted = "two2\nthree3\nfour4\nfive5\n";
    agree("assign_record", ASSIGNED_RECORDS, &Expect::Runs(wanted));
    asan_runs("assign_record", ASSIGNED_RECORDS, wanted);
}

const ASSIGNED_LOCI: &str = r#"
locus Cache<K, V> {
    params {
        cap: Int = 1;
    }
    birth() {
        println("cap=", self.cap);
    }
}

locus Keeper {
    params {
        c: Cache<Int, String>;
    }
    run() {
        self.c = Cache { cap: 5 };
        println("now=", self.c.cap);
    }
}

fn main() {
    let k = Keeper { c: Cache { cap: 2 } };
    let mut c: Cache<Int, String> = Cache { cap: 3 };
    c = Cache { cap: 6 };
    println("local=", c.cap);
}
"#;

#[test]
fn an_assignment_of_a_generic_locus_takes_them_too() {
    // Each replacement is born where it is stored.
    let wanted = "cap=2\ncap=5\nnow=5\ncap=3\ncap=6\nlocal=6\n";
    agree("assign_locus", ASSIGNED_LOCI, &Expect::Runs(wanted));
    asan_runs("assign_locus", ASSIGNED_LOCI, wanted);
}

#[test]
fn a_compound_assignment_of_a_generic_literal_stays_refused() {
    agree(
        "assign_compound",
        r#"
type Cfg<T> {
    n: T;
}

fn main() {
    let mut c: Cfg<Int> = Cfg { n: 1 };
    c += Cfg { n: 2 };
    println(c.n);
}
"#,
        &Expect::Refused("not assignable"),
    );
}

// === the placement table reads an assignment's place ==========
// A generic locus literal assigned to a place takes the arguments the
// place declares, in the placement table as at a `let`: the table reads
// the place's type from the checker's walk. With a local's ascription or
// `self.<param>` read off the syntax alone, `h.c = Cache { .. }` from
// `main`, a nested field and an array's element were "unresolved
// arguments" holes.

const ASSIGNED_AT_ANY_PLACE: &str = r#"
locus Cache<K, V> {
    params {
        cap: Int = 1;
    }
    birth() {
        println("cap=", self.cap);
    }
}

locus Inner {
    params {
        c: Cache<Int, String>;
    }
}

locus Holder {
    params {
        c: Cache<Int, String>;
        inner: Inner;
        row: [Cache<Int, String>; 2];
    }
    run() {
        self.c = Cache { cap: 8 };
    }
}

fn main() {
    let a: Cache<Int, String> = Cache { cap: 3 };
    let b: Cache<Int, String> = Cache { cap: 4 };
    let h: Holder = Holder { c: Cache { cap: 1 }, inner: Inner { c: Cache { cap: 2 } }, row: [a, b] };
    h.c = Cache { cap: 5 };
    h.inner.c = Cache { cap: 6 };
    h.row[1] = Cache { cap: 7 };
    let mut l: Cache<Int, String> = Cache { cap: 9 };
    l = Cache { cap: 10 };
    println("c=", h.c.cap, " inner=", h.inner.c.cap, " row=", h.row[1].cap, " l=", l.cap);
}
"#;

#[test]
fn the_placement_table_reads_an_assignments_place() {
    let program = parse_source(ASSIGNED_AT_ANY_PLACE).expect("parses");
    let Ok(snap) = Snapshot::from_program(program, Vec::new(), Config::harness(Target::host())) else {
        panic!("the program does not load");
    };
    let table = snap.demand_placement().expect("the placement table");
    let unresolved: Vec<_> = table
        .holes
        .iter()
        .filter(|h| matches!(h.kind, HoleKind::UnresolvedArguments | HoleKind::UnresolvedDeclaration { .. }))
        .collect();
    assert!(unresolved.is_empty(), "every Cache literal's arguments resolve: {unresolved:?}");
    let realized: Vec<&str> = table
        .instances
        .values()
        .filter_map(|r| r.realizes.as_ref())
        .chain(table.dynamic.iter().filter_map(|d| d.realizes.as_ref()))
        .map(|r| r.lowered.as_str())
        .filter(|l| l.starts_with("Cache"))
        .collect();
    assert!(realized.iter().all(|l| *l == "Cache_Int_String"), "{realized:?}");
    // Ten literals of the template, each realized.
    let literals = ASSIGNED_AT_ANY_PLACE.matches("Cache { cap").count();
    assert_eq!(literals, 10);
    assert!(realized.len() >= literals, "each literal is realized: {realized:?}");
    let wanted = "c=5 inner=6 row=7 l=10";
    agree("assign_any_place", ASSIGNED_AT_ANY_PLACE, &Expect::Runs(wanted));
    asan_runs("assign_any_place", ASSIGNED_AT_ANY_PLACE, wanted);
}

// === an array or tuple literal of template literals ============
// `let cs: [Cfg<Int>; 2] = [Cfg { .. }, Cfg { .. }];` was refused
// ("expected `[Cfg_Int; 2]`, got `[Cfg; 2]`"), and the build, which
// skips the checker, could not lower the elements. Each element takes
// the declared element type's arguments, at every site a single literal
// takes the declared type's: a `let`, a param default, an assignment, a
// return slot, a record field's init.

const ARRAYS_AND_TUPLES: &str = r#"
type Cfg<T> {
    n: T;
    label: String = "";
}

type Shelf {
    pair: [Cfg<Int>; 2];
}

locus Keeper {
    params {
        cfgs: [Cfg<Int>; 2] = [Cfg { n: 1, label: "a" }, Cfg { n: 2, label: "b" }];
        tagged: (Cfg<Int>, Int) = (Cfg { n: 3, label: "c" }, 4);
    }
    run() {
        println(self.cfgs[1].label, self.cfgs[1].n, self.tagged.0.label, self.tagged.1);
        self.cfgs = [Cfg { n: 5, label: "e" }, Cfg { n: 6, label: "f" }];
        println(self.cfgs[0].label, self.cfgs[1].n);
    }
}

fn pair() -> [Cfg<Int>; 2] {
    return [Cfg { n: 7, label: "g" }, Cfg { n: 8, label: "h" }];
}

fn main() {
    let k = Keeper { };
    let cs: [Cfg<Int>; 2] = [Cfg { n: 9, label: "i" }, Cfg { n: 10, label: "j" }];
    let t: (Cfg<Int>, String) = (Cfg { n: 11, label: "k" }, "l");
    let s = Shelf { pair: [Cfg { n: 12, label: "m" }, Cfg { n: 13, label: "n" }] };
    let p = pair();
    println(cs[1].label, cs[0].n, t.0.label, t.1, s.pair[1].label, p[0].n);
}
"#;

#[test]
fn an_array_or_tuple_literal_takes_the_element_types_arguments() {
    let wanted = "b2c4\ne6\nj9kln7\n";
    agree("array_tuple", ARRAYS_AND_TUPLES, &Expect::Runs(wanted));
    asan_runs("array_tuple", ARRAYS_AND_TUPLES, wanted);
}

const ARRAY_OF_GENERIC_LOCI: &str = r#"
locus Cache<K, V> {
    params {
        cap: Int = 1;
    }
    birth() {
        println("cap=", self.cap);
    }
}

fn main() {
    let cs: [Cache<Int, String>; 2] = [Cache { cap: 2 }, Cache { cap: 3 }];
    println("sum=", cs[0].cap + cs[1].cap);
}
"#;

#[test]
fn an_array_of_generic_locus_literals_takes_them_too() {
    let wanted = "cap=2\ncap=3\nsum=5\n";
    agree("array_loci", ARRAY_OF_GENERIC_LOCI, &Expect::Runs(wanted));
    asan_runs("array_loci", ARRAY_OF_GENERIC_LOCI, wanted);
    // The placement table reads each element under the element type.
    let program = parse_source(ARRAY_OF_GENERIC_LOCI).expect("parses");
    let Ok(snap) = Snapshot::from_program(program, Vec::new(), Config::harness(Target::host())) else {
        panic!("the program does not load");
    };
    let table = snap.demand_placement().expect("the placement table");
    assert!(
        !table.holes.iter().any(|h| matches!(h.kind, HoleKind::UnresolvedArguments)),
        "{:?}",
        table.holes
    );
    let realized: Vec<&str> = table
        .instances
        .values()
        .filter_map(|r| r.realizes.as_ref())
        .map(|r| r.lowered.as_str())
        .filter(|l| l.starts_with("Cache"))
        .collect();
    assert_eq!(realized, ["Cache_Int_String", "Cache_Int_String"]);
}

#[test]
fn an_unannotated_array_of_generic_literals_asks_for_the_arguments() {
    // It checked clean and died at build, unlocated, at
    // `expression form Discriminant(12)`.
    agree(
        "array_unannotated",
        r#"
type Box<T> {
    value: T = 0;
}

fn main() {
    let bs = [Box { value: 1 }, Box { value: 2 }];
    println(bs[0].value);
}
"#,
        &Expect::Refused("`Box` is a generic type"),
    );
}

// === every site admitting a template literal holds its fields ==
// Review of #1568: a literal spelled with a generic template's name was
// typed against the template, whose parameters are unknown, so at every
// site that admits it (`admit_monomorph_literal`'s table) a field of a
// parameter's type took anything: `self.b = Box { value: "bad" }` with
// `b: Box<Int>` checked, and the build refused `Box_Int.value`. One row
// per site: the bad field refused by the check (and so never built) at
// the field, the good one checked, built and run. `[x; N]` is one of the
// array rows: the checker typed it as an array, the build could not
// lower its element.

const SITE_DECLS: &str = r#"
type Box<T> {
    value: T;
}

type Outer {
    b: Box<Int>;
}

type E<T> {
    code: T;
}

locus Cache<K, V> {
    params {
        cap: K;
    }
}
"#;

/// (site, body with `VALUE` where the field's value goes, what it prints
/// with `VALUE` = 7).
const SITES: &[(&str, &str, &str)] = &[
    ("let", "fn main() { let b: Box<Int> = Box { value: VALUE }; println(b.value); }", "7"),
    (
        "assign_local",
        "fn main() { let mut b: Box<Int> = Box { value: 1 }; b = Box { value: VALUE }; println(b.value); }",
        "7",
    ),
    (
        "assign_self_field",
        "locus H { params { b: Box<Int> = Box { value: 1 }; } fn set() { self.b = Box { value: VALUE }; } }\n\
         fn main() { let h = H { }; h.set(); println(h.b.value); }",
        "7",
    ),
    ("return", "fn make() -> Box<Int> { return Box { value: VALUE }; }\nfn main() { println(make().value); }", "7"),
    (
        "return_fallible",
        "fn make() -> Box<Int> fallible(E<Int>) { return Box { value: VALUE }; }\n\
         fn main() { let z: Box<Int> = Box { value: 0 }; let b = make() or z; println(b.value); }",
        "7",
    ),
    (
        "param_default",
        "locus L { params { b: Box<Int> = Box { value: VALUE }; } }\nfn main() { let l = L { }; println(l.b.value); }",
        "7",
    ),
    ("record_field", "fn main() { let o = Outer { b: Box { value: VALUE } }; println(o.b.value); }", "7"),
    (
        "locus_override",
        "locus L { params { b: Box<Int> = Box { value: 1 }; } }\n\
         fn main() { let l = L { b: Box { value: VALUE } }; println(l.b.value); }",
        "7",
    ),
    (
        "array_let",
        "fn main() { let bs: [Box<Int>; 2] = [Box { value: 1 }, Box { value: VALUE }]; println(bs[1].value); }",
        "7",
    ),
    ("array_repeat_let", "fn main() { let bs: [Box<Int>; 2] = [Box { value: VALUE }; 2]; println(bs[1].value); }", "7"),
    (
        "array_repeat_assign",
        "locus H { params { bs: [Box<Int>; 2] = [Box { value: 1 }; 2]; } fn set() { self.bs = [Box { value: VALUE }; 2]; } }\n\
         fn main() { let h = H { }; h.set(); println(h.bs[0].value); }",
        "7",
    ),
    (
        "array_repeat_default",
        "locus H { params { bs: [Box<Int>; 2] = [Box { value: VALUE }; 2]; } }\n\
         fn main() { let h = H { }; println(h.bs[1].value); }",
        "7",
    ),
    ("tuple_let", "fn main() { let t: (Box<Int>, Int) = (Box { value: VALUE }, 1); println(t.0.value); }", "7"),
    ("locus_let", "fn main() { let c: Cache<Int, String> = Cache { cap: VALUE }; println(c.cap); }", "7"),
    (
        "locus_assign",
        "fn main() { let mut c: Cache<Int, String> = Cache { cap: 1 }; c = Cache { cap: VALUE }; println(c.cap); }",
        "7",
    ),
];

#[test]
fn every_admitting_site_holds_the_literals_fields_to_the_monomorphs() {
    for (site, body, prints) in SITES {
        let good = format!("{SITE_DECLS}{}\n", body.replace("VALUE", "7"));
        agree(&format!("site_{site}"), &good, &Expect::Runs(prints));
        let bad = format!("{SITE_DECLS}{}\n", body.replace("VALUE", "\"bad\""));
        let program = parse_source(&bad).unwrap_or_else(|e| panic!("[{site}] parse: {e:?}"));
        let diags = check_program(&program);
        let refusal = diags
            .iter()
            .find(|d| d.message.contains("expects `Int`, got `String`"))
            .unwrap_or_else(|| panic!("[{site}] the bad field is refused: {:?}", diags.iter().map(|d| &d.message).collect::<Vec<_>>()));
        assert_eq!(refusal.span.slice(&bad), "\"bad\"", "[{site}] located at the field's value");
    }
}

#[test]
fn an_array_repeat_of_generic_literals_runs_under_asan() {
    let src = format!("{SITE_DECLS}{}\n", SITES.iter().find(|s| s.0 == "array_repeat_assign").unwrap().1.replace("VALUE", "7"));
    asan_runs("array_repeat_assign", &src, "7");
}

#[test]
fn a_field_of_an_aliased_argument_meets_the_aliased_type() {
    // `Supervisor<Scalar>`'s `Cell<Scalar>` holds an `Int`: the bound
    // type is read through the alias before the field's value meets it.
    agree(
        "alias_argument",
        r#"
type Scalar = Int;

locus Cell<T> {
    params {
        value: T;
    }
}

locus Supervisor<T> {
    params {
        child: Cell<T>;
    }
}

fn main() {
    let a: Supervisor<Scalar> = Supervisor { child: Cell { value: 17 } };
    println(a.child.value);
}
"#,
        &Expect::Runs("17"),
    );
}

// === a nested instantiation is read from its structure ==========
// `let n: Box<Box<Int>> = ..; n.value.value` was refused, "no field
// `value` on `Box_Box_Int`": the monomorph table parsed each mangled name
// back into a template and `_`-split tokens, and `Box_Box_Int` splits
// into two tokens for a one-parameter template. The table now reads an
// instantiation from the type expression that writes it (the nested one's
// row first), and the bus graph's payload flatness reads that row too, so
// neither nesting nor a `_` in an argument's name is split back out.

const NESTED: &str = r#"
type Box<T> {
    value: T;
}

type Pair<A, B> {
    left: A;
    right: B;
}

locus Keep<T> {
    params {
        held: T;
    }
    fn get() -> T {
        return self.held;
    }
}

locus Show {
    params {
        k: Keep<Box<String>>;
    }
    run() {
        println(self.k.get().value);
    }
}

fn main() {
    let n: Box<Box<Int>> = Box { value: Box { value: 7 } };
    println(n.value.value);
    let p: Pair<Box<Int>, Box<String>> = Pair { left: Box { value: 1 }, right: Box { value: "two" } };
    println(p.left.value, p.right.value);
    let deep: Box<Pair<Box<Int>, Box<Box<String>>>> = Box { value: Pair { left: Box { value: 3 }, right: Box { value: Box { value: "four" } } } };
    println(deep.value.left.value, deep.value.right.value.value);
    let s = Show { k: Keep { held: Box { value: "kept" } } };
}
"#;

#[test]
fn a_nested_instantiation_is_its_structure() {
    // Box<Box<Int>>, Pair<Box<Int>, Box<String>>, three levels, and a
    // generic locus over a generic record.
    let wanted = "7\n1two\n3four\nkept\n";
    agree("nested", NESTED, &Expect::Runs(wanted));
    asan_runs("nested", NESTED, wanted);
}

const ENUM_ARGUMENT: &str = r#"
type Mode = enum { Fast, Slow };

type Box<T> {
    value: T;
}

fn main() {
    let b: Box<Mode> = Box { value: Mode::Slow };
    match b.value {
        Mode::Fast -> println("fast"),
        Mode::Slow -> println("slow"),
    }
}
"#;

#[test]
fn an_enum_argument_is_an_enum_in_its_monomorph() {
    // The monomorphs are declared before the program's enums: a field
    // of a parameter bound to one resolved as a record, so `Box<Mode>`
    // failed ("declared TypeRef(\"Mode\"), got Enum(\"Mode\")").
    agree("enum_argument", ENUM_ARGUMENT, &Expect::Runs("slow\n"));
}

const BUS_PAYLOAD_INSTANCE: &str = r#"
type Big_Mode = enum { Fast, Slow };

type Pair<A, B> {
    left: A;
    right: B;
}

locus Rx {
    bus {
        subscribe "pairs" as on_pair of type Pair<Big_Mode, Int>;
    }
    fn on_pair(p: Pair<Big_Mode, Int>) {
        println("right=", p.right);
    }
}

main locus App {
    params {
        rx: Rx = Rx { };
    }
    bus {
        publish "pairs" of type Pair<Big_Mode, Int>;
    }
    run() {
        let p: Pair<Big_Mode, Int> = Pair { left: Big_Mode::Fast, right: 5 };
        "pairs" <- p;
    }
}

fn main() { App { }; }
"#;

#[test]
fn a_bus_payload_instance_over_an_underscored_name_lowers() {
    // `Pair<Big_Mode, Int>`: the payload's flatness is the row's fields,
    // `Big_Mode` a payload-less enum, so the plan and the codec agree.
    agree("bus_payload_instance", BUS_PAYLOAD_INSTANCE, &Expect::Runs("right=5\n"));
}

// === a variant's construction holds its arguments ==============
// `Wrap::Held("str")` with `Held(Box<Int>)` checked clean (the callee path
// types as the enum and no argument was held to the payload) and the
// build refused it, unlocated: "Wrap::Held arg 0: expected type
// TypeRef(\"Box_Int\"), got String". `Wrap::Held(Box { value: 5 })` checked
// clean too and died at build at `expression form Discriminant(12)`. Each
// argument now meets the payload field's type, a template literal taking
// its arguments, in the check and the build.

#[test]
fn a_variants_arguments_meet_its_payload_types() {
    agree(
        "variant_template_literal",
        r#"
type Box<T> {
    value: T;
}

type Wrap = enum { Held(Box<Int>), Empty };

fn main() {
    let w = Wrap::Held(Box { value: 5 });
    let s = match w {
        Wrap::Held(b) -> b.value,
        Wrap::Empty -> 0,
    };
    println(s);
}
"#,
        &Expect::Runs("5\n"),
    );
    agree(
        "variant_mistyped",
        r#"
type Box<T> {
    value: T;
}

type Wrap = enum { Held(Box<Int>), Empty };

fn main() {
    let w = Wrap::Held("str");
    println(1);
}
"#,
        &Expect::Refused("`Wrap::Held` argument 0: expected `Box_Int`, got `String`"),
    );
    agree(
        "variant_arity",
        r#"
type Wrap = enum { Held(Int), Empty };

fn main() {
    let w = Wrap::Held(1, 2);
    println(1);
}
"#,
        &Expect::Refused("`Wrap::Held` takes 1 argument, got 2"),
    );
}
