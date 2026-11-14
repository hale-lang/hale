//! JSON Tier 2 typecheck (2026-06-09): after the generator runs,
//! `Type::from_json` resolves to a `fallible(JsonError)` function, so an
//! unaddressed call is flagged by the two-channel rule (and an addressed
//! one is clean). Mirrors the CLI pipeline: generate parsers, then check.

#[path = "support/entries.rs"]
mod entries;
use hale_syntax::json_gen::generate_json_parsers;
use hale_syntax::parse_source;

fn check(src: &str) -> Vec<String> {
    let mut prog = parse_source(src).expect("parse failed");
    generate_json_parsers(&mut prog);
    entries::check_program(&prog).into_iter().map(|d| d.message).collect()
}

#[test]
fn unaddressed_from_json_is_flagged() {
    let msgs = check(
        r#"
        type Order { id: Int `json:"id"`; }
        fn f() { let o = Order::from_json("{}"); let _ = o.id; }
    "#,
    );
    assert!(
        msgs.iter()
            .any(|m| m.contains("`Order::from_json` can fail (JsonError) and this call says nothing about it")),
        "unaddressed from_json must be flagged; got: {:?}",
        msgs
    );
}

#[test]
fn addressed_from_json_is_clean() {
    let msgs = check(
        r#"
        type Order { id: Int `json:"id"`; }
        fn f() -> Int { let o = Order::from_json("{}") or raise; return o.id; }
    "#,
    );
    assert!(
        !msgs.iter().any(|m| m.contains("says nothing about it")),
        "addressed from_json must be clean; got: {:?}",
        msgs
    );
}

// ---- the record codecs' law (`json_codec_laws`): a call the sequence
// left as written is refused by the check, naming the rule, so that a
// program the check admits builds (downstream handoff).

#[test]
fn from_json_of_an_untagged_record_is_refused_with_the_tag_to_write() {
    let msgs = check(
        r#"
        type Plain { id: Int; }
        fn f() -> Int { let o = Plain::from_json("{}") or raise; return o.id; }
    "#,
    );
    let want = "`Plain::from_json`: `Plain` declares no `json:` field tag, which is what gives a record its JSON codec (spec/semantics.md § Metadata tags); tag a field, `id: Int `json:\"id\"`;`";
    assert!(msgs.iter().any(|m| m == want), "got: {msgs:?}");
}

#[test]
fn from_json_of_a_record_with_an_uncarried_field_names_the_field() {
    let msgs = check(
        r#"
        type Export { id: Int `json:"id"`; raw: Bytes; }
        fn f() -> Int { let o = Export::from_json("{}") or raise; return o.id; }
    "#,
    );
    let want = "`Export::from_json`: `Export` has a field `raw: Bytes`, which the JSON codec does not carry";
    assert!(msgs.iter().any(|m| m == want), "got: {msgs:?}");
    // through a record it reaches, by the field's path
    let msgs = check(
        r#"
        type Inner { w: [Int]; }
        type Outer { id: Int `json:"id"`; i: Inner; }
        fn f() -> String { let o = Outer { id: 1, i: Inner { w: [1] } }; return Outer::to_json(o); }
    "#,
    );
    let want = "`Outer::to_json`: `Outer` has a field `i.w: [Int]`, which the JSON codec does not carry";
    assert!(msgs.iter().any(|m| m == want), "got: {msgs:?}");
}

#[test]
fn from_json_of_an_enum_and_of_the_wrong_arity_are_refused() {
    let msgs = check(
        r#"
        type Side = enum { Buy, Sell };
        type Ord { side: Side `json:"side"`; }
        fn f() -> Side { let s = Side::from_json("\"Buy\"") or raise; return s; }
        fn g() -> Ord { let o = Ord::from_json("{}", 1) or raise; return o; }
    "#,
    );
    assert!(
        msgs.iter().any(|m| m.starts_with("`Side::from_json`: `Side` is an enum, and `from_json` is a record's")),
        "got: {msgs:?}"
    );
    assert!(msgs.iter().any(|m| m == "`Ord::from_json` takes one argument (the JSON text), got 2"), "got: {msgs:?}");
}

#[test]
fn an_enum_field_and_a_list_field_are_carried() {
    // the codec is generated and the call rewritten: nothing to refuse,
    // and the fallible rule reads the generated signature
    let msgs = check(
        r#"
        type Side = enum { Buy, Sell };
        type Ord { side: Side `json:"side"`; tags: bounded[String; 4]; mode: Side = Side::Sell; }
        fn f() -> Side { let o = Ord::from_json("{}") or raise; return o.side; }
    "#,
    );
    assert!(msgs.is_empty(), "got: {msgs:?}");
}

// ---- the `json:` tag's options and the positional record (spec/api.md §
// Codecs): `string` is the one option and is a Decimal's; a positional
// record's fields have no key; `@positional` opts a record into its codec.

#[test]
fn a_tag_option_other_than_string_or_on_no_decimal_is_refused() {
    let msgs = check(
        r#"
        type Quote { px: Decimal<2> `json:"px,string"`; n: Int `json:"n,string"`; at: Decimal<2> `json:",omitempty"`; }
        fn f() -> String { return Quote::to_json(Quote { px: 1d, n: 1, at: 2d }); }
    "#,
    );
    let want = [
        "`Quote.n`: `json:` tag option `string` writes a Decimal as a JSON string, and the field is `Int`: it is for a `Decimal<S>` field or a list of them",
        "`Quote.at`: `json:` tag option `omitempty` is not one: the one option is `string`, a Decimal written as a JSON string (`json:\"price,string\"`)",
    ];
    for w in want {
        assert!(msgs.iter().any(|m| m == w), "want {w}; got: {msgs:?}");
    }
    assert_eq!(msgs.len(), 2, "the Decimal's `string` is admitted: {msgs:?}");
    // a list of Decimals takes it too
    let msgs = check(
        r#"
        type Stops { px: bounded[Decimal<2>; 4] `json:",string"`; }
        fn f() -> String { return Stops::to_json(Stops { }); }
    "#,
    );
    assert!(msgs.is_empty(), "got: {msgs:?}");
}

#[test]
fn a_positional_record_has_its_codec_and_no_keys() {
    // no tag needed: `@positional` is the opt-in
    let msgs = check(
        r#"
        @positional type Row { t: Int; px: Decimal<2> `json:",string"`; }
        fn f() -> Int { let r = Row::from_json("[1, 2]") or raise; return r.t; }
    "#,
    );
    assert!(msgs.is_empty(), "got: {msgs:?}");
    let msgs = check(
        r#"
        @positional type Row { t: Int `json:"time"`; px: Decimal<2>; }
        fn f() -> Int { let r = Row::from_json("[1, 2]") or raise; return r.t; }
    "#,
    );
    assert_eq!(
        msgs,
        vec![
            "`Row.t`: `Row` is a `@positional` record, which crosses JSON as an array of its fields in order, so its fields have no key and `json:\"time\"` names none; drop the key (an option stays: `json:\",string\"`)"
        ]
    );
}
