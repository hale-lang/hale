//! An optional field's absent value (GH #1951 item 3, F.52): a record
//! literal that leaves `x?: T` out builds it absent, so `x` holds its
//! default, or its type's zero where it declares none (the zero the JSON
//! codecs give an absent field). A type with no zero (a tuple, a point, a
//! record that reaches itself) is refused at the field, asking for a
//! default.

#[path = "support/entries.rs"]
mod entries;
use entries::check_program;
use hale_syntax::parse_source;

fn messages(src: &str) -> Vec<(String, usize)> {
    let prog = parse_source(src).expect("parse");
    check_program(&prog).iter().map(|d| (d.message.clone(), d.span.line_col(src).0)).collect()
}

#[test]
fn a_literal_may_leave_out_an_optional_field_of_any_zeroed_type() {
    let src = "unit cent;\n\
               type Lvl = Int { range: 1..5; }\n\
               type Id = distinct Int;\n\
               type Money = quantity Int in cent;\n\
               type Mood = enum { Calm, Busy };\n\
               type Shape = enum { Circle(Int), Square };\n\
               type Inner { a: Int; s: String; }\n\
               type Row {\n\
                   i?: Int; f?: Float; b?: Bool; s?: String; d?: Decimal<2>; u?: U256;\n\
                   t?: Time; dur?: Duration; v?: std::json::Value; l?: Lvl; id?: Id; m?: Money;\n\
                   mood?: Mood; sh?: Shape; inner?: Inner; xs?: [Int; 2]; tags?: bounded[String; 4];\n\
                   n?: Int = 7;\n\
               }\n\
               fn main() {\n    let r = Row { };\n    println(r.has_i);\n    println(r.n);\n}\n";
    let got = messages(src);
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn an_optional_field_with_no_zero_asks_for_a_default() {
    let src = "type Row {\n    pair?: (Int, String);\n    ok?: Int;\n}\nfn main() { }\n";
    let got = messages(src);
    assert!(
        got.iter().any(|(m, line)| *line == 2
            && m.contains("optional field `pair` of type `Row`")
            && m.contains("give it a default: `pair?: (Int, String) = <value>;`")),
        "{got:?}"
    );
    assert_eq!(got.len(), 1, "only the field with no zero: {got:?}");
    // a default is enough
    let src = "type Row {\n    pair?: (Int, String) = (1, \"a\");\n}\nfn main() { }\n";
    assert!(messages(src).is_empty(), "{:?}", messages(src));
}
