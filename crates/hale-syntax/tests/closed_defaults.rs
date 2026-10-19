//! Whether a field's default is one a codec can build on its own
//! (`json_gen::DefaultScope::is_closed`, spec/api.md § Codecs): a default
//! is evaluated in the scope of the literal that leaves its field, so it is
//! closed only when it resolves with no locals in scope. A record literal
//! in a default is closed only if every field it leaves out has a closed
//! default too, followed through nested literals and through aliases; a
//! chain of defaults that reaches itself is not closed, and the check ends.

use std::collections::BTreeMap;

use hale_syntax::ast::{TopDecl, TypeDeclBody};
use hale_syntax::json_gen::DefaultScope;
use hale_syntax::parse_source;

/// Each field default of `src`'s records, by `Record.field`: closed or not.
fn closed(src: &str) -> BTreeMap<String, bool> {
    let p = parse_source(src).unwrap_or_else(|d| panic!("parse: {}", d[0].message));
    let scope = DefaultScope::of(&[p.items.as_slice()], &[]);
    let mut out = BTreeMap::new();
    for item in &p.items {
        let TopDecl::Type(t) = item else { continue };
        let TypeDeclBody::Struct(fs) = &t.body else { continue };
        for f in fs {
            if let Some(d) = &f.default {
                out.insert(format!("{}.{}", t.name.name, f.name.name), scope.is_closed(d));
            }
        }
    }
    out
}

#[test]
fn a_literal_is_closed_only_if_what_it_leaves_out_is() {
    let got = closed(
        "const LIMIT: Int = 5;\n\
         type Inner { b: Int = a + 1; }\n\
         type Outer { inner: Inner = Inner { }; given: Inner = Inner { b: 2 }; }\n\
         type Inner2 { c: Int = 3; d: Int = LIMIT * 2; tags: bounded[Int; 4]; }\n\
         type Outer2 { inner: Inner2 = Inner2 { }; }\n\
         type Deep { o: Outer = Outer { given: Inner { b: 1 } }; o2: Outer2 = Outer2 { }; }\n",
    );
    let want: BTreeMap<String, bool> = [
        ("Inner.b", false),      // reads a name of the literal's scope
        ("Outer.inner", false),  // leaves `b`, whose default is not closed
        ("Outer.given", true),   // writes `b` itself
        ("Inner2.c", true),
        ("Inner2.d", true),      // a constant
        ("Outer2.inner", true),  // leaves `c`, `d` (closed) and a `bounded` field
        ("Deep.o", false),       // its literal leaves `inner`, which is not closed
        ("Deep.o2", true),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    assert_eq!(got, want);
}

#[test]
fn a_default_that_reaches_itself_is_not_closed_and_the_check_ends() {
    let got = closed(
        "type A { x: Int = 1; b: B = B { }; }\n\
         type B { y: Int = 2; a: A = A { }; }\n",
    );
    assert_eq!(got.get("A.b"), Some(&false), "{got:?}");
    assert_eq!(got.get("B.a"), Some(&false), "{got:?}");
    assert_eq!(got.get("A.x"), Some(&true), "{got:?}");
}

/// A name resolves the way the program does, with no locals in scope: a
/// constructor through its alias, a called name only when it is a module
/// fn, a builtin or a conversion, and a field's `bounded` type through its
/// alias too.
#[test]
fn every_name_resolves_with_no_locals_in_scope() {
    let got = closed(
        "fn value() -> Int { return 3; }\n\
         type OrderId = distinct Int;\n\
         type Inner { b: Int = a + 1; }\n\
         type Alias = Inner;\n\
         type Tags = bounded[Int; 4];\n\
         type Tagged { tags: Tags; c: Int = 1; }\n\
         type TagAlias = Tagged;\n\
         type Color = enum { red, blue };\n\
         type Hue = Color;\n\
         type R {\n\
             aliased: Inner = Alias { };\n\
             fn_value: Int = compute();\n\
             module_fn: Int = value();\n\
             builtin: Int = len(\"abc\");\n\
             conversion: OrderId = OrderId(3);\n\
             tagged: Tagged = Tagged { };\n\
             tagged_alias: Tagged = TagAlias { };\n\
             variant: Color = Hue::blue;\n\
             no_variant: Color = Hue::green;\n\
             unknown: Inner = Missing { };\n\
             qualified: Inner = l::Inner { };\n\
             stdlib: Int = std::math::abs_int(1);\n\
         }\n",
    );
    let want: BTreeMap<String, bool> = [
        ("Inner.b", false),
        ("Tagged.c", true),
        ("R.aliased", false),      // the alias's record leaves out `b`
        ("R.fn_value", false),     // `compute` is no module fn: a binding of the literal's scope
        ("R.module_fn", true),
        ("R.builtin", true),
        ("R.conversion", true),
        ("R.tagged", true),        // `tags` is `bounded` through its alias
        ("R.tagged_alias", true),
        ("R.variant", true),       // a variant through the enum's alias
        ("R.no_variant", false),   // no such variant
        ("R.unknown", false),      // no such record
        ("R.qualified", false),    // an import no rename resolves
        ("R.stdlib", true),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    assert_eq!(got, want);
}

/// A qualified constructor resolves through the build's import renames
/// (or the imported declaration's own spelling), and its omitted defaults
/// are followed there.
#[test]
fn a_qualified_constructor_follows_the_import() {
    let p = parse_source(
        "type __lib_Inner { b: Int = a + 1; }\n\
         type __lib_Closed { c: Int = 3; }\n\
         type R { open: __lib_Inner = l::Inner { }; closed: __lib_Closed = l::Closed { }; }\n",
    )
    .unwrap();
    let renames = vec![
        (vec!["l".to_string(), "Inner".to_string()], "__lib_Inner".to_string()),
        (vec!["l".to_string(), "Closed".to_string()], "__lib_Closed".to_string()),
    ];
    let scope = DefaultScope::of(&[p.items.as_slice()], &renames);
    let TopDecl::Type(r) = &p.items[2] else { panic!() };
    let TypeDeclBody::Struct(fs) = &r.body else { panic!() };
    let verdict: Vec<bool> = fs.iter().map(|f| scope.is_closed(f.default.as_ref().unwrap())).collect();
    assert_eq!(verdict, vec![false, true]);
}
