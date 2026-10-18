//! Whether a field's default is one a codec can build on its own
//! (`json_gen::default_is_closed`, spec/api.md § Codecs): a default is
//! evaluated in the scope of the literal that leaves its field, so a
//! record literal in a default is closed only if every field it leaves out
//! has a closed default too, followed through nested literals; a chain of
//! defaults that reaches itself is not closed, and the check ends.

use std::collections::{BTreeMap, HashSet};

use hale_syntax::ast::{StructField, TopDecl, TypeDeclBody};
use hale_syntax::json_gen::{default_is_closed, global_value_names, record_fields};
use hale_syntax::parse_source;

/// Each field default of `src`'s records, by `Record.field`: closed or not.
fn closed(src: &str) -> BTreeMap<String, bool> {
    let p = parse_source(src).unwrap_or_else(|d| panic!("parse: {}", d[0].message));
    let mut globals = HashSet::new();
    global_value_names(&p.items, &mut globals);
    let mut records: BTreeMap<String, &[StructField]> = BTreeMap::new();
    record_fields(&p.items, &mut records);
    let mut out = BTreeMap::new();
    for item in &p.items {
        let TopDecl::Type(t) = item else { continue };
        let TypeDeclBody::Struct(fs) = &t.body else { continue };
        for f in fs {
            if let Some(d) = &f.default {
                let c = default_is_closed(d, &|n| globals.contains(n), &|r| records.get(r).copied());
                out.insert(format!("{}.{}", t.name.name, f.name.name), c);
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
