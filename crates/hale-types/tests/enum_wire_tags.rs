//! GH #1951 item 5: the `json:` tags of an enum's variants (spec/api.md §
//! Codecs, spec/semantics.md § Metadata tags). A tag's key renames the
//! variant on the JSON wire and its one option, `other`, makes it the
//! enum's catch-all; the check refuses what the codecs could not honour,
//! each at the variant, and the parser refuses a tag on a set's member.

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
fn tagged_variants_check_clean() {
    let msgs = check(
        r#"
        type Side = enum { Buy `json:"buy"`, Sell `json:"sell"` };
        type Kind = enum { Limit, Unknown `json:"unknown,other"` };
        type Venue = enum { Spot `json:"spot"`, Other(String) `json:",other"` };
        type Event = enum { Fill(Int) `json:"fill"`, Cancel `json:"cancel" wire:"x"` };
        fn f() -> String { return Side::api_encode(Side::Buy) + Kind::api_encode(Kind::Limit) + Venue::api_encode(Venue::Spot); }
    "#,
    );
    assert!(msgs.is_empty(), "tagged variants are clean: {msgs:?}");
}

#[test]
fn two_variants_one_wire_name_are_refused_naming_both() {
    let msgs = check(r#"type Side = enum { Buy `json:"Sell"`, Sell };"#);
    let want = "`Side::Sell`: `Side::Buy` and `Side::Sell` both cross JSON as \"Sell\": a variant's wire name (its `json:` tag's, else its enum's `@wire_case` spelling, else its own) names one variant; rename one";
    assert!(msgs.iter().any(|m| m == want), "want {want}; got: {msgs:?}");
    // a payload variant's key shares the names
    let msgs = check(r#"type E = enum { Fill(Int) `json:"x"`, Gone `json:"x"` };"#);
    assert!(msgs.iter().any(|m| m.contains("`E::Fill` and `E::Gone` both cross JSON as \"x\"")), "{msgs:?}");
    // a lossless catch-all has no wire name, so its name clashes with none
    let msgs = check(r#"type E = enum { Other `json:"Other"`, Spare(String) `json:",other"` };"#);
    assert!(msgs.is_empty(), "{msgs:?}");
}

#[test]
fn a_second_catch_all_is_refused() {
    let msgs = check(r#"type E = enum { A, Lost `json:",other"`, Other(String) `json:",other"` };"#);
    let want = "`E::Other`: `E` has two catch-all variants, `Lost` and `Other`: an unknown string decodes to one, so an enum has at most one `json:\",other\"`";
    assert!(msgs.iter().any(|m| m == want), "want {want}; got: {msgs:?}");
}

#[test]
fn a_catch_all_holding_other_than_one_string_is_refused() {
    for (decl, held) in [("Other(Int)", "(Int)"), ("Other(String, Int)", "(String, Int)")] {
        let src = format!("type E = enum {{ A, {decl} `json:\",other\"` }};");
        let msgs = check(&src);
        let want = format!(
            "`E::Other`: a catch-all variant (`json:\",other\"`) is payload-free, keeping only that the string was unknown, or holds the one `String` it caught, and `Other` holds `{held}`; declare `Other(String)` or `Other`"
        );
        assert!(msgs.iter().any(|m| *m == want), "want {want}; got: {msgs:?}");
    }
    // an alias of `String` is a `String`
    let msgs = check(r#"type Name = String; type E = enum { A, Other(Name) `json:",other"` };"#);
    assert!(msgs.is_empty(), "{msgs:?}");
}

#[test]
fn a_name_on_a_lossless_catch_all_is_refused() {
    let msgs = check(r#"type E = enum { A, Other(String) `json:"other,other"` };"#);
    let want = "`E::Other`: a catch-all that holds the string writes the string it holds, so it has no wire name of its own and `json:\"other,other\"` names none; drop the name (`json:\",other\"`)";
    assert!(msgs.iter().any(|m| m == want), "want {want}; got: {msgs:?}");
}

#[test]
fn a_field_option_on_a_variant_is_refused() {
    let msgs = check(r#"type E = enum { A `json:"a,string"`, B };"#);
    let want = "`E::A`: `json:` tag option `string` is not a variant's: a variant's tag takes a wire name (`json:\"sell\"`) and the option `other`, which makes it the enum's catch-all (`json:\",other\"`)";
    assert!(msgs.iter().any(|m| m == want), "want {want}; got: {msgs:?}");
}

#[test]
fn a_tag_on_a_set_member_is_refused_where_it_is_written() {
    for src in [
        "type Book { a: Int; } type Trade { b: Int; } type S = Book `json:\"book\"` | Trade;",
        "type Book { a: Int; } type Trade { b: Int; } type S = Book | Trade `json:\",other\"`;",
    ] {
        let diags = parse_source(src).expect_err("a tag on a set's member does not parse");
        assert!(
            diags.iter().any(|d| d.message.starts_with("a set's member takes no tag: it is its own discriminant")),
            "{src}: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }
    let diags = parse_source("type T = Int `json:\"t\"`;").expect_err("an alias takes no tag");
    assert!(diags.iter().any(|d| d.message.starts_with("an alias takes no tag")), "{:?}", diags.iter().map(|d| &d.message).collect::<Vec<_>>());
}
