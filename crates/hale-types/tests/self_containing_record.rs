//! A record type that contains itself by value is a located check
//! error, not a compiler crash.
//!
//! `type Cfg { child: Cfg = Cfg { }; }` passed `hale check`, and the
//! build expanded the default (a `Cfg` whose `child` is a `Cfg` whose
//! `child` is ...) until the compiler's own stack ran out ("thread
//! 'main' has overflowed its stack"), downstream handoff. A ring with no
//! default built a type no literal could ever write. Every value of a
//! record holds a whole value of each field's type, so a record that
//! holds its own kind by value has no floor; the declaration is the
//! error, reported at the field that closes the ring.
//!
//! What is deliberately NOT a ring, pinned below: a `bounded[T; N]` of
//! the record (it starts empty) and an enum holding it (a variant beside
//! one that holds none). Both build and run (`self_containing_record.rs`
//! in hale-codegen, `tests/hale/recursive_record_test.hl`).

use std::collections::BTreeMap;

use hale_syntax::ast::Program;
use hale_syntax::parse_source;
use hale_types::symbol::Bundle;

/// Every check diagnostic of `src`, with its span's text.
fn diags(src: &str) -> Vec<(String, String)> {
    let mut parsed = vec![("test.hl".to_string(), parse_source(src).expect("parse"))];
    let ids = hale_types::snapshot::mint(parsed.iter_mut().map(|(n, p)| (n.as_str(), p)), &[]);
    let mut programs: BTreeMap<String, &Program> = BTreeMap::new();
    for (name, p) in &parsed {
        programs.insert(name.clone(), p);
    }
    let mut bundle = Bundle::new(programs);
    bundle.snapshot = ids;
    let (scope, mut ds) = hale_types::resolve::build_top_scope(&bundle);
    ds.extend(hale_types::check::check_bundle(&bundle, &scope, true));
    ds.into_iter().map(|d| (d.message.clone(), d.span.slice(src).to_string())).collect()
}

/// The reports of this rule: (message, the text the span covers).
fn rings(src: &str) -> Vec<(String, String)> {
    diags(src).into_iter().filter(|(m, _)| m.starts_with("a record cannot contain itself by value")).collect()
}

/// The downstream handoff's program, in a neutral spelling.
#[test]
fn a_record_holding_itself_with_a_default_is_refused_at_the_field() {
    let src = "type Cfg { limit: String = \"\"; has_child: Bool = false; child: Cfg = Cfg { }; }\n\
               fn main() { let c = Cfg { limit: \"1\" }; std::test::assert_eq_str(c.limit, \"1\", \"r\"); }\n";
    let hits = rings(src);
    assert_eq!(hits.len(), 1, "one report: {:?}", diags(src));
    assert_eq!(
        hits[0].0,
        "a record cannot contain itself by value: `Cfg.child: Cfg` puts a whole `Cfg` inside every `Cfg`, so none \
         can be built. Hold the `Cfg` values a `Cfg` may have in a `bounded[Cfg; N]` (it starts empty), or in an \
         enum variant beside one that holds none (`type Child = enum { Nothing, One(Cfg) }`), or keep them \
         elsewhere and hold a key"
    );
    assert_eq!(hits[0].1, "child: Cfg = Cfg { };", "located at the field");
}

/// With no default the type is as impossible: a literal must write
/// `child`, which needs a `Cfg`.
#[test]
fn a_record_holding_itself_without_a_default_is_refused() {
    let hits = rings("type Cfg { limit: String = \"\"; child: Cfg; }\nfn main() { println(\"x\"); }\n");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].1, "child: Cfg;");
}

/// A ring through two records: one report, at the field that closes
/// it, naming each field on the ring.
#[test]
fn a_ring_through_two_records_is_reported_once_with_its_fields() {
    let src = "type A { n: Int = 0; b: B = B { }; }\n\
               type B { a: A = A { }; }\n\
               fn main() { println(\"x\"); }\n";
    let hits = rings(src);
    assert_eq!(hits.len(), 1, "one report per ring: {hits:?}");
    assert!(hits[0].0.contains("the ring `A.b: B` → `B.a: A` puts a whole `A` inside every `A`"), "{}", hits[0].0);
    assert_eq!(hits[0].1, "a: A = A { };", "at the field that closes the ring");
}

/// Through a tuple's part, a fixed array's element, a generic record's
/// field and an alias.
#[test]
fn a_ring_through_a_tuple_an_array_a_generic_and_an_alias_is_refused() {
    for (src, field, at) in [
        ("type Cfg { pair: (Int, Cfg); }\n", "`Cfg.pair: (Int, Cfg)`", "pair: (Int, Cfg);"),
        ("type Cfg { arr: [Cfg; 2]; }\n", "`Cfg.arr: [Cfg; 2]`", "arr: [Cfg; 2];"),
        ("type Box<T> { v: T; }\ntype Cfg { b: Box<Cfg>; }\n", "`Cfg.b: Box<Cfg>`", "b: Box<Cfg>;"),
        ("type Same = Cfg;\ntype Cfg { s: Same; }\n", "`Cfg.s: Same`", "s: Same;"),
    ] {
        let src = format!("{src}fn main() {{ println(\"x\"); }}\n");
        let hits = rings(&src);
        assert_eq!(hits.len(), 1, "{src}: {hits:?}");
        assert!(hits[0].0.contains(field), "{src}: {}", hits[0].0);
        assert_eq!(hits[0].1, at, "{src}");
    }
}

/// The shapes that hold a record of their own kind with a floor are not
/// rings: a `bounded` starts empty, an enum has a variant that holds
/// none, and a record holding a different record is a tree.
#[test]
fn a_bounded_an_enum_and_a_plain_nesting_are_not_refused() {
    for src in [
        "type Cfg { limit: String = \"\"; kids: bounded[Cfg; 4]; }\n",
        "type Cfg { limit: String = \"\"; child: Sub = Sub::Nothing; }\ntype Sub = enum { Nothing, One(Cfg) };\n",
        "type Leaf { n: Int = 0; }\ntype Cfg { a: Leaf = Leaf { }; b: (Leaf, Leaf); c: [Leaf; 2]; }\n",
        "type Box<T> { v: T; }\ntype Cfg { b: Box<Int>; }\n",
    ] {
        let src = format!("{src}fn main() {{ println(\"x\"); }}\n");
        let errors: Vec<_> = diags(&src).into_iter().filter(|(m, _)| m.contains("contain itself")).collect();
        assert!(errors.is_empty(), "{src}: {errors:?}");
    }
}
