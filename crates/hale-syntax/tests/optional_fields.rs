//! Optional fields, `x?: T` (GH #1951 item 3, F.52): the parser declares
//! `x` and synthesizes its presence flag `has_x: Bool = false` right after
//! it (and `clear_x` for `clearable`), the slots a hand-written presence
//! pair or triple has. Optionality is a property of a record field, never
//! a type: `?` anywhere else is refused where it is written.

use hale_syntax::ast::{FieldOptional, TopDecl, TypeDeclBody};
use hale_syntax::parse_source;

fn fields(src: &str) -> Vec<(String, FieldOptional, bool)> {
    let prog = parse_source(src).expect("parse");
    for item in &prog.items {
        if let TopDecl::Type(td) = item {
            if let TypeDeclBody::Struct(fs) = &td.body {
                return fs.iter().map(|f| (f.name.name.clone(), f.optional, f.default.is_some())).collect();
            }
        }
    }
    panic!("no struct type in source");
}

/// The first error's message and the 1-based line and column it is at.
fn refusal(src: &str) -> (String, (usize, usize)) {
    let diags = parse_source(src).expect_err("refused");
    let d = &diags[0];
    (d.message.clone(), d.span.line_col(src))
}

#[test]
fn an_optional_field_declares_its_flags_right_after_it() {
    let got = fields("type T {\n    a: Int;\n    x?: String;\n    p?: Int clearable = 3;\n    z: Bool;\n}\n");
    assert_eq!(
        got,
        vec![
            ("a".into(), FieldOptional::No, false),
            ("x".into(), FieldOptional::Declared { clearable: false }, false),
            ("has_x".into(), FieldOptional::Presence, true),
            ("p".into(), FieldOptional::Declared { clearable: true }, true),
            ("has_p".into(), FieldOptional::Presence, true),
            ("clear_p".into(), FieldOptional::Clear, true),
            ("z".into(), FieldOptional::No, false),
        ]
    );
}

#[test]
fn a_flag_declared_beside_its_optional_field_is_a_duplicate() {
    let (msg, at) = refusal("type T {\n    x?: Int;\n    has_x: Bool;\n}\n");
    assert!(msg.contains("field `has_x` is already declared by `x?`"), "{msg}");
    assert_eq!(at, (3, 5), "at the hand-written flag");
    // before the optional field too
    let (msg, at) = refusal("type T {\n    has_x: Bool = false;\n    x?: Int;\n}\n");
    assert!(msg.contains("field `has_x` is already declared by `x?`"), "{msg}");
    assert_eq!(at, (2, 5));
    let (msg, _) = refusal("type T {\n    x?: Int clearable;\n    clear_x: Bool;\n}\n");
    assert!(msg.contains("field `clear_x` is already declared by `x?: … clearable`"), "{msg}");
    // a hand-written pair is untouched: it has no `?`
    let got = fields("type T {\n    x: Int;\n    has_x: Bool;\n}\n");
    assert!(got.iter().all(|(_, o, _)| *o == FieldOptional::No), "{got:?}");
}

#[test]
fn optionality_is_refused_outside_a_record_field() {
    for (src, needle, at) in [
        ("fn f(a?: Int) { }\n", "`a?`: a parameter is never optional", (1, 7)),
        ("fn main() {\n    let n?: Int = 1;\n}\n", "`n?`: a local is never optional", (2, 10)),
        ("locus L {\n    params { n?: Int = 1; }\n}\n", "`n?`: a locus param is never optional", (2, 15)),
        ("fn f() -> Int? { return 1; }\n", "Hale has no optional type `T?`", (1, 14)),
        ("type E = enum { A(Int?) };\n", "Hale has no optional type `T?`", (1, 22)),
        ("type T { x: Int?; }\n", "Hale has no optional type `T?`", (1, 16)),
        ("@positional type P { a?: Int; }\n", "a positional record has no optional field", (1, 22)),
        ("type T { x: Int clearable; }\n", "`clearable` marks an optional field", (1, 17)),
        ("type T { x?: Int = 1 clearable; }\n", "write `clearable` right after the type", (1, 22)),
    ] {
        let (msg, got) = refusal(src);
        assert!(msg.contains(needle), "{src}: {msg}");
        assert_eq!(got, at, "{src}: {msg}");
    }
}

#[test]
fn clearable_is_still_a_name() {
    // contextual: a word only after an optional field's type
    let got = fields("type T {\n    clearable: Int;\n    x?: clearable;\n}\ntype clearable = Int;\n");
    assert_eq!(got[0].0, "clearable");
    assert_eq!(got[1].1, FieldOptional::Declared { clearable: false });
}

#[test]
fn fmt_prints_the_mark_canonically() {
    let src = "type T {\n    x ?:Int  clearable;\n    y?:   bounded[Int; 4]   clearable;\n    z ? : String = \"\";\n}\n";
    let want = "type T {\n    x?: Int clearable;\n    y?: bounded[Int; 4] clearable;\n    z?: String = \"\";\n}\n";
    let got = hale_syntax::fmt::format_source(src).expect("formats");
    assert_eq!(got, want);
    assert_eq!(hale_syntax::fmt::format_source(want).expect("formats"), want, "a fixed point");
}
