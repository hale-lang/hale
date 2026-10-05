//! GH #1141: a struct, an enum, a locus's or perspective's `params`,
//! a `contract` and an `interface` declare each name once. A second
//! declaration used to pass `hale check` silently — one of the two
//! won the slot and the source could not say which — while the json
//! codecs and the api description assumed one slot per name. It is a
//! type error at the second declaration, naming the first.

#[path = "support/entries.rs"]
mod entries;
use hale_syntax::parse_source;
use entries::check_program;

fn diag_naming(src: &str, needle: &str) -> hale_syntax::error::Diag {
    let prog = parse_source(src).expect("parse");
    let diags = check_program(&prog);
    let hit = diags
        .iter()
        .find(|d| d.message.contains(needle))
        .unwrap_or_else(|| {
            panic!(
                "expected a diagnostic containing {needle:?}, got: {:?}",
                diags.iter().map(|d| &d.message).collect::<Vec<_>>()
            )
        })
        .clone();
    assert!(
        hit.related.iter().any(|r| r.label.contains("the first declaration")),
        "carries the first declaration as related: {:?}",
        hit.related
    );
    hit
}

#[test]
fn a_struct_field_declared_twice_is_an_error_at_the_second() {
    let src = "type T {\n    a: Int = 1;\n    b: String = \"\";\n    a: Int = 2;\n}\nfn main() { println(T { }.a); }\n";
    let hit = diag_naming(src, "field `a` is already declared in type `T`");
    // at the second declaration (line 4), naming the first (line 2)
    assert_eq!(hit.span.line_col(src).0, 4, "{:?}", hit.span);
    assert_eq!(hit.related[0].span.line_col(src).0, 2, "{:?}", hit.related);
}

#[test]
fn an_enum_variant_declared_twice_is_an_error() {
    let src = "type Shape = enum {\n    Circle(Int),\n    Square(Int),\n    Circle(String),\n};\nfn main() { }\n";
    let hit = diag_naming(src, "variant `Circle` is already declared in type `Shape`");
    // at the second declaration (line 4), naming the first (line 2)
    assert_eq!(hit.span.line_col(src).0, 4, "{:?}", hit.span);
    assert_eq!(hit.related[0].span.line_col(src).0, 2, "{:?}", hit.related);
}

#[test]
fn a_perspective_param_declared_twice_is_an_error() {
    diag_naming(
        "perspective Router {\n    params { n: Int = 0; m: Int = 1; n: Int = 2; }\n    fn route(code: Int) -> Int;\n}\nfn main() { }\n",
        "param `n` is already declared in perspective `Router`",
    );
}

#[test]
fn a_locus_param_declared_twice_is_an_error() {
    diag_naming(
        "main locus App {\n    params { n: Int = 0; m: Int = 1; n: Int = 2; }\n    run() { }\n}\nfn main() { App { }; }\n",
        "param `n` is already declared in locus `App`",
    );
}

#[test]
fn a_contract_member_declared_twice_is_an_error() {
    diag_naming(
        "main locus App {\n    contract { expose count: Int; expose count: Int; }\n    params { count: Int = 0; }\n    run() { }\n}\nfn main() { App { }; }\n",
        "contract member `count` is already declared in locus `App`",
    );
}

#[test]
fn an_interface_method_declared_twice_is_an_error() {
    diag_naming(
        "interface Shape {\n    fn area() -> Int;\n    fn area() -> Int;\n}\nfn main() { }\n",
        "method `area` is already declared in interface `Shape`",
    );
}

#[test]
fn distinct_names_raise_nothing() {
    let prog = parse_source(
        "type T { a: Int = 1; b: Int = 2; }\ntype Shape = enum { Circle(Int), Square(Int) };\nperspective Router {\n    params { n: Int = 0; m: Int = 1; }\n    fn route(code: Int) -> Int;\n}\nmain locus App {\n    contract { expose count: Int; }\n    params { count: Int = 0; total: Int = 0; }\n    run() { }\n}\nfn main() { println(T { }.a); App { }; }\n",
    )
    .expect("parse");
    let diags = check_program(&prog);
    assert!(
        !diags.iter().any(|d| d.message.contains("already declared")),
        "{:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}
