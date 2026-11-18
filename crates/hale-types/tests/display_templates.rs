//! RFC #2035 (decisions F.54): `@display` templates, refused where they
//! are wrong, each at the template.
//!
//! The parser refuses what it can see in the declaration: a hole that
//! names no field (or no payload position), a hole holding an expression,
//! a spec outside the f-string grammar, and a template on a locus, a set,
//! an enum as a whole or a generic type. The check refuses the rest: a
//! template on a record that is not printable (a template never widens the
//! printable set), a hole whose field cannot render, and a spec that does
//! not fit its field. `?` is a format spec of its own.

#[path = "support/entries.rs"]
mod entries;

/// The source's first error and the text it points at.
fn first_error(src: &str) -> (String, String) {
    match hale_syntax::parse_source(src) {
        Err(ds) => {
            let d = &ds[0];
            (d.message.clone(), d.span.slice(src).to_string())
        }
        Ok(program) => {
            let errors: Vec<_> = entries::check_program(&program).into_iter().filter(|d| d.is_error()).collect();
            let d = errors.first().unwrap_or_else(|| panic!("refused:\n{src}"));
            (d.message.clone(), d.span.slice(src).to_string())
        }
    }
}

fn refused(src: &str, at: &str, says: &str) {
    let src = format!("{src}\nfn main() {{ }}\n");
    let (msg, slice) = first_error(&src);
    assert!(msg.contains(says), "`{says}` in: {msg}\n{src}");
    assert_eq!(slice, at, "located at `{at}`: {msg}\n{src}");
}

#[test]
fn a_hole_names_a_field_or_a_payload_position() {
    refused(
        "@display(\"{nope}\") type A { x: Int; }",
        "{nope}",
        "`A` has no field `nope`: a `@display` hole names a field of the record (`{x}`)",
    );
    refused("@display(\"{0}\") type A { x: Int; }", "{0}", "names a payload position, and `A` is a record");
    refused(
        "type E = enum { @display(\"{1}\") A(Int) };",
        "{1}",
        "a variant's template names its payload by position, and `A` carries 1 (`{0}`)",
    );
    refused(
        "type E = enum { @display(\"{x}\") A(Int) };",
        "{x}",
        "names its payload by position",
    );
    refused("type E = enum { @display(\"{0}\") A };", "{0}", "carries no payload, so its template has no hole");
}

#[test]
fn a_template_holds_no_expression_and_only_a_spec() {
    refused("@display(\"{x + 1}\") type A { x: Int; }", "{x + 1}", "a template holds no expression");
    refused("@display(\"{x:q}\") type A { x: Int; }", "{x:q}", "unknown format kind `q`");
    refused("@display(\"{x\") type A { x: Int; }", "{x", "opens a hole that never closes");
    refused("@display(\"x}\") type A { x: Int; }", "}", "closes no hole");
}

#[test]
fn only_a_record_or_a_variant_takes_one() {
    refused(
        "@display(\"L\") locus L { params { n: Int = 0; } }",
        "@display(\"L\")",
        "a locus takes no `@display`: a locus is never printable",
    );
    refused(
        "type A { x: Int; }\ntype B { y: Int; }\n@display(\"s\") type S = A | B;",
        "@display(\"s\")",
        "`S` is a set, and a set takes no `@display`",
    );
    refused(
        "@display(\"e\") type E = enum { A, B };",
        "@display(\"e\")",
        "an enum's templates are its variants'",
    );
    refused(
        "@display(\"{value}\") type Box<T> { value: T; }",
        "@display(\"{value}\")",
        "a generic record takes no `@display` yet",
    );
    refused(
        "type E<T> = enum { @display(\"v\") V(T) };",
        "@display(\"v\")",
        "a generic enum's variant takes no `@display` yet",
    );
    refused("@display(\"n\") type N = Int;", "@display(\"n\")", "`N` is no record");
}

#[test]
fn a_template_never_widens_the_printable_set() {
    refused(
        "@display(\"{x}\") type A { x: Int; raw: Bytes; }",
        "@display(\"{x}\")",
        "`A` takes no `@display`: a template is declared only on a type that is already printable",
    );
}

#[test]
fn each_hole_is_held_to_the_f_string_rules() {
    refused(
        "@display(\"{x}\") type A { x: Int; b: Bytes; }\n@display(\"{b}\") type B { b: Bytes; }",
        "@display(\"{x}\")",
        "takes no `@display`",
    );
    let src = "@display(\"{b}\") type B { b: Bytes; }\nfn main() { }\n";
    let program = hale_syntax::parse_source(src).expect("parses");
    let ds = entries::check_program(&program);
    let hole = ds
        .iter()
        .find(|d| d.message.starts_with("a `@display` hole cannot render a value of type `Bytes`"))
        .unwrap_or_else(|| panic!("the hole is refused in the template's words: {ds:?}"));
    assert_eq!(hole.span.slice(src), "{b}");
    refused("@display(\"{s:x}\") type A { s: String; }", "{s:x}", "hexadecimal formatting applies to `Int`, not `String`");
    refused("@display(\"{n:.2}\") type A { n: Int; }", "{n:.2}", "a decimal precision applies to `Float` or `Decimal`");
}

#[test]
fn a_question_mark_spec_is_the_structural_form() {
    let ok = "@display(\"{x}\") type A { x: Int; }\nfn main() { let a = A { x: 1 }; println(f\"{a:?} {a:>8?} {3:?}\"); }\n";
    let program = hale_syntax::parse_source(ok).expect("parses");
    let errors: Vec<_> = entries::check_program(&program).into_iter().filter(|d| d.is_error()).collect();
    assert!(errors.is_empty(), "`?`, with a width too, takes any printable value: {errors:?}");
    let src = "fn main() { let x = 1.5; println(f\"{x:.2?}\"); }\n";
    let ds = hale_syntax::parse_source(src).expect_err("a precision and `?` do not combine");
    assert!(ds[0].message.contains("a precision and `?` cannot be combined"), "{}", ds[0].message);
}

#[test]
fn a_template_is_no_error_where_it_is_right() {
    let src = r#"
@display("${dollars}.{cents:0>2} {currency}")
type Money { dollars: Int; cents: Int; currency: String; }

type Side = enum {
    @display("buy") Buy,
    @display("other: {0}") Other(String),
    Plain(Int),
};

type Cash = Money;

fn main() {
    let m = Money { dollars: 1, cents: 2, currency: "USD" };
    let c: Cash = m;
    println(m, f"{m:?}", Money::display(m), Cash::display(c), Side::display(Side::Buy), Side::Other("x"));
    let f = Money::display;
    println(f(m));
}
"#;
    let program = hale_syntax::parse_source(src).expect("parses");
    let ds = entries::check_program(&program);
    assert!(ds.iter().all(|d| !d.is_error()), "{ds:?}");
}

/// GH #2061 review: what a rendering allocates includes the templates its
/// structural rendering reaches. `Outer { v: Inner }` renders `v` by
/// `Inner`'s template, so `to_string(o)` costs what that template
/// allocates, at any depth, and a zero-alloc budget refuses it as it
/// refuses `to_string(inner)`. `{d:?}` reaches no template.
#[test]
fn a_budget_counts_templates_reached_through_a_structural_record() {
    let src = r#"
@display("inner {x} and {y}")
type Inner { x: Int; y: Int; }

type Outer { v: Inner; }

type Deep { o: Outer; }

@budget(alloc_per_call = 0)
fn direct(i: Inner) -> String { return to_string(i); }

@budget(alloc_per_call = 0)
fn show(o: Outer) -> String { return to_string(o); }

@budget(alloc_per_call = 0)
fn deep(d: Deep) -> String { return to_string(d); }

fn main() {
    let o = Outer { v: Inner { x: 1, y: 2 } };
    println(direct(o.v), show(o), deep(Deep { o: o }));
}
"#;
    let program = hale_syntax::parse_source(src).expect("parses");
    let ds = entries::check_program(&program);
    for f in ["direct", "show", "deep"] {
        assert!(
            ds.iter().any(|d| d.is_error()
                && d.message.contains(&format!("`{f}` declares `@budget(alloc_per_call = 0)` but the compiler counts 3"))),
            "`{f}` renders through `Inner`'s template, three allocations: {ds:?}"
        );
    }
}

/// GH #2061 review: each rendered occurrence counts. Two fields of one
/// templated type render it twice, so the record costs twice the template.
#[test]
fn a_budget_counts_each_rendered_field() {
    let src = r#"
@display("inner {x} and {y}")
type Inner { x: Int; y: Int; }

type Outer { a: Inner; b: Inner; }

@budget(alloc_per_call = 3)
fn show(o: Outer) -> String { return to_string(o); }

fn main() {
    let i = Inner { x: 1, y: 2 };
    println(show(Outer { a: i, b: i }));
}
"#;
    let program = hale_syntax::parse_source(src).expect("parses");
    let ds = entries::check_program(&program);
    assert!(
        ds.iter().any(|d| d.is_error() && d.message.contains("`show` declares `@budget(alloc_per_call = 3)` but the compiler counts 6")),
        "two renderings of `Inner`'s template, three allocations each: {ds:?}"
    );
}

/// GH #2061 review: the rendering edges run through every printable shape,
/// as lowering's do: an enum's payload, a tuple's element, an alias, a
/// record holding an enum. The check records them from the value's type,
/// so the allocation summary counts what lowering calls. `{x:?}` reaches no
/// template.
#[test]
fn a_budget_counts_templates_reached_through_enums_tuples_and_aliases() {
    let src = r#"
@display("inner {x} and {y}")
type Inner { x: Int; y: Int; }

type Maybe = enum { Some(Inner), Nothing };
type Holder { m: Maybe; }
type Pair { a: Inner; b: Inner; }
type Twin = Pair;

@budget(alloc_per_call = 0)
fn payload(m: Maybe) -> String { return to_string(m); }

@budget(alloc_per_call = 0)
fn element(t: (Inner, Int)) -> String { return to_string(t); }

@budget(alloc_per_call = 0)
fn held(h: Holder) -> String { return to_string(h); }

@budget(alloc_per_call = 3)
fn aliased(t: Twin) -> String { return to_string(t); }

@budget(alloc_per_call = 0)
fn structural(h: Holder) -> String { return f"{h:?}"; }

fn main() {
    let i = Inner { x: 1, y: 2 };
    println(payload(Maybe::Some(i)), element((i, 3)), held(Holder { m: Maybe::Nothing }));
    println(aliased(Pair { a: i, b: i }), structural(Holder { m: Maybe::Some(i) }));
}
"#;
    let program = hale_syntax::parse_source(src).expect("parses");
    let ds = entries::check_program(&program);
    for (f, budget, counted) in [("payload", 0, 3), ("element", 0, 3), ("held", 0, 3), ("aliased", 3, 6)] {
        assert!(
            ds.iter().any(|d| d.is_error()
                && d.message.contains(&format!(
                    "`{f}` declares `@budget(alloc_per_call = {budget})` but the compiler counts {counted}"
                ))),
            "`{f}` renders through `Inner`'s template, {counted} allocations: {ds:?}"
        );
    }
    let structural: Vec<_> = ds.iter().filter(|d| d.is_error() && d.message.contains("`structural`")).collect();
    assert!(
        structural.iter().all(|d| !d.message.contains("counts 3")),
        "`{{h:?}}` reaches no template: {structural:?}"
    );
}
