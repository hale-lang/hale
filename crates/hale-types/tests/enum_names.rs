//! RFC #1995 E1 / E1b: an enum's external names (spec/types.md § An enum's
//! names, F.53). `E::name(v)`, `E::parse(s)`, `E::variants` and
//! `E::index(v)` call functions the desugar sequence writes for `E`; a use
//! they do not apply to, or of the wrong shape, is left as written and the
//! check refuses it with the reason, where the build used to be the first
//! to say (`Color::variants()` checked clean and the build refused it,
//! G12). `tests/hale/enum_names_test.hl` runs what they compute.

#[path = "support/entries.rs"]
mod entries;

fn check(src: &str) -> Vec<String> {
    entries::check_files(&[("main.hl", src)]).into_iter().map(|d| d.message).collect()
}

fn refused(msgs: &[String], want: &str) {
    assert!(msgs.iter().any(|m| m == want), "want: {want}\ngot: {msgs:#?}");
}

#[test]
fn the_four_check_clean_on_the_enums_they_apply_to() {
    let msgs = check(
        r#"
        type Lifecycle = enum { Active, Archived };
        type Venue = enum { Spot `json:"spot"`, Other(String) `json:",other"` };
        @wire_case(kebab) type Tier = enum { FreeTier, ProTier `json:"pro"` };
        type Shape = enum { Circle(Float), Empty };
        fn main() {
            let a = Lifecycle::parse("Active") or Lifecycle::Archived;
            let n = Lifecycle::name(a) + Shape::name(Shape::Circle(1.0)) + Tier::name(Tier::FreeTier);
            let i = Lifecycle::index(a) + Shape::index(Shape::Empty) + len(Lifecycle::variants);
            let v: Venue = Venue::parse("x");
            let all: [Venue; 1] = Venue::variants;
            println(n, i, Venue::name(v), len(all));
        }
    "#,
    );
    assert!(msgs.is_empty(), "{msgs:#?}");
}

#[test]
fn a_payload_enum_is_refused_parse_and_variants_naming_the_variant() {
    let msgs = check(
        r#"
        type Shape = enum { Circle(Float), Rect(Float, Float), Empty };
        fn main() {
            let s = Shape::parse("Empty") or Shape::Empty;
            let all = Shape::variants;
        }
    "#,
    );
    refused(
        &msgs,
        "`Shape::parse` builds a variant from its name alone, so it applies to an enum whose variants carry no \
         values (and at most one catch-all, `Other(String) `json:\",other\"``), and `Shape::Circle` carries 1 \
         value; match on the name and build the variant yourself",
    );
    refused(
        &msgs,
        "`Shape::variants` lists every value of the enum, so it applies to an enum whose variants carry no values \
         (and at most one catch-all, `Other(String) `json:\",other\"``), and `Shape::Circle` carries 1 value; list \
         the values you need yourself",
    );
    // a catch-all with values is no lossless one: refused as any payload
    let msgs = check(
        r#"
        type E = enum { A, B(Int, Int) };
        fn main() { let e = E::parse("A") or E::A; }
    "#,
    );
    assert!(msgs.iter().any(|m| m.contains("and `E::B` carries 2 values")), "{msgs:#?}");
}

#[test]
fn variants_is_a_value_and_the_others_are_called_with_one_argument() {
    let msgs = check(
        r#"
        type Color = enum { Red, Blue };
        fn main() {
            let a = Color::variants();
            let b = Color::name;
            let c = Color::name(Color::Red, 2);
            let d = Color::parse() or Color::Red;
        }
    "#,
    );
    refused(
        &msgs,
        "`Color::variants` is a value, the array of `Color`'s variants in declaration order, not a function: write \
         `Color::variants`, without the parentheses",
    );
    refused(
        &msgs,
        "`Color::name` is a function the compiler writes for the enum, called as `Color::name(v)`; it is no value \
         of its own",
    );
    refused(&msgs, "`Color::name` takes one argument, a `Color`, and this passes 2");
    refused(&msgs, "`Color::parse` takes one argument, the name, a `String`, and this passes 0");
}

#[test]
fn a_generic_enum_is_refused_with_the_reason() {
    let msgs = check(
        r#"
        type Maybe<T> = enum { Some(T), Nothing };
        fn main() {
            let m: Maybe<Int> = Maybe::Nothing;
            let s = Maybe::name(m);
        }
    "#,
    );
    refused(
        &msgs,
        "`Maybe::name`: `Maybe` is generic, and hale can't write `name`, `parse`, `variants` or `index` for a \
         generic enum yet; match on the value",
    );
}

#[test]
fn a_variant_named_like_one_keeps_meaning_the_variant() {
    // F.43 discipline: the names are contextual
    let msgs = check(
        r#"
        type Field = enum { name, variants, index, title };
        fn main() {
            let a: Field = Field::name;
            let b: Field = Field::variants;
            let c: Field = Field::index;
            let d = Field::parse("title") or Field::title;
            println(a, b, c, d);
        }
    "#,
    );
    assert!(msgs.is_empty(), "{msgs:#?}");
    // and `Field::index` builds the variant, so its value count is the
    // variant's, not the function's
    let msgs = check(
        r#"
        type Field = enum { index(Int), title };
        fn main() { let c = Field::index(1, 2); }
    "#,
    );
    refused(&msgs, "`Field::index` carries 1 value, and this passes 2");
}

#[test]
fn an_argument_of_another_type_is_refused() {
    let msgs = check(
        r#"
        type Color = enum { Red, Blue };
        fn main() { let n = Color::name(3); }
    "#,
    );
    assert!(msgs.iter().any(|m| m.contains("expected `Color`, got `Int`")), "{msgs:#?}");
}

#[test]
fn an_enum_of_another_file_of_the_seed_is_named() {
    let msgs = entries::check_files(&[
        ("a.hl", "@wire_case(snake) type Mode = enum { FastLane, Slow };\n"),
        (
            "main.hl",
            "fn main() {\n    let m = Mode::parse(\"fast_lane\") or Mode::Slow;\n    println(Mode::name(m), \
             Mode::index(m), len(Mode::variants));\n}\n",
        ),
    ]);
    assert!(msgs.is_empty(), "{msgs:#?}");
}

#[test]
fn wire_case_spellings_that_meet_are_refused_as_one_wire_name() {
    let msgs = check("@wire_case(snake) type B = enum { FooBar, Foo_Bar };\nfn main() {}\n");
    assert!(msgs.iter().any(|m| m.contains("`B::FooBar` and `B::Foo_Bar` both cross JSON as \"foo_bar\"")), "{msgs:#?}");
}

#[test]
fn wire_case_marks_an_enum_with_a_case_it_knows() {
    let parse = |src: &str| match hale_syntax::parse_source(src) {
        Ok(_) => Vec::new(),
        Err(ds) => ds.into_iter().map(|d| d.message).collect::<Vec<_>>(),
    };
    let msgs = parse("@wire_case(camel) type A = enum { X };");
    refused(&msgs, "`@wire_case(camel)`: the cases are `snake`, `screaming_snake`, `kebab`, `lower`");
    let msgs = parse("@wire_case(snake) type B { x: Int; }");
    refused(
        &msgs,
        "`@wire_case` spells an enum's variants on the wire, and `B` is no enum; a record's field is renamed by its \
         `json:` tag",
    );
    let msgs = parse("type A { x: Int; }\ntype C { x: Int; }\n@wire_case(snake) type B = A | C;");
    assert!(msgs.iter().any(|m| m.contains("and `B` is a set")), "{msgs:#?}");
}
