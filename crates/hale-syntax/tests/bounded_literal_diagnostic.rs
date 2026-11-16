//! GH #1948: `let x = bounded[String; 4] { };` read as an index of a
//! name `bounded` and stopped at the `;` with a bare "expected ], got
//! Semi". A bounded value is an array literal at a declared site (GH #1951):
//! the refusal says so where it is written, and what to write.

use hale_syntax::parse_source;

#[test]
fn a_bounded_type_in_value_position_says_what_to_write() {
    let src = "fn main() {\n    let x = bounded[String; 4] { };\n}\n";
    let err = parse_source(src).expect_err("a bounded literal does not parse");
    let d = &err[0];
    assert!(d.message.contains("`bounded[T; N]` is a type, not a value"), "{}", d.message);
    assert!(d.message.contains("let mut xs: bounded[String; 4] = [];"), "{}", d.message);
    assert_eq!(&src[d.span.start.as_usize()..d.span.end.as_usize()], "bounded[String;", "from the name to the `;`");
}

#[test]
fn an_index_of_a_local_named_bounded_still_parses() {
    parse_source("fn main() {\n    let bounded = [1, 2];\n    let x = bounded[0];\n}\n")
        .expect("`bounded` is not reserved; an index of it parses");
}
