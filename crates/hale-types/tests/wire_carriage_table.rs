//! The bus wire format's carriage table (GH #1951 item 4): one table,
//! `hale_types::wire_carriage::WIRE_TABLE`, says what the wire carries,
//! and the check's refusal of a payload prints its list from it, so the
//! refusal and the serializer (which dispatches on the table's shapes)
//! cannot drift apart. Held here: the carried list is generated from the
//! table's rows, every shape has a row, and the refusals at a topic and at
//! a literal subject print exactly that list.

use hale_syntax::parse_source;
use hale_types::wire_carriage::{carried_list, wire_row, WireShape, WIRE_TABLE};

#[path = "support/entries.rs"]
mod entries;

const SHAPES: &[WireShape] = &[
    WireShape::Scalar,
    WireShape::PayloadFreeEnum,
    WireShape::String,
    WireShape::Bytes,
    WireShape::Struct,
    WireShape::BoundedInline,
    WireShape::BoundedCounted,
    WireShape::Array,
    WireShape::PayloadEnum,
    WireShape::Tuple,
    WireShape::JsonValue,
];

#[test]
fn every_shape_has_one_row_and_the_carried_list_is_the_rows() {
    for s in SHAPES {
        assert_eq!(WIRE_TABLE.iter().filter(|r| r.shape == *s).count(), 1, "{s:?} has one row");
        assert!(!wire_row(*s).encoding.is_empty());
    }
    assert_eq!(WIRE_TABLE.len(), SHAPES.len(), "a row for a shape this test does not name");
    // the list is each row's words once, in the table's order
    let list = carried_list();
    let mut at = 0;
    let mut seen: Vec<&str> = Vec::new();
    for r in WIRE_TABLE {
        if seen.contains(&r.carried) {
            continue;
        }
        seen.push(r.carried);
        let i = list[at..].find(r.carried).unwrap_or_else(|| panic!("`{}` is not in, or is out of order in: {list}", r.carried));
        at += i + r.carried.len();
    }
    assert_eq!(at, list.len(), "the list holds nothing but the rows: {list}");
}

fn refusal(src: &str) -> String {
    let program = parse_source(src).expect("parses");
    let diags = entries::check_program(&program);
    diags
        .iter()
        .map(|d| d.message.clone())
        .find(|m| m.contains("which the wire format does not carry") || m.contains("which hale can't carry on the bus"))
        .unwrap_or_else(|| panic!("no wire refusal in: {diags:#?}"))
}

#[test]
fn a_topic_refusal_prints_the_tables_list() {
    let m = refusal(
        "type Node { n: Int = 0; kids: bounded[Node; 2]; }\n\
         type Msg { root: Node; n: Int = 0; }\n\
         topic Ev { payload: Msg; subject: \"ev.x\"; }\n\
         locus Sub { bus { subscribe Ev as on_ev; } fn on_ev(m: Msg) { println(m.n); } }\n\
         fn main() { let s = Sub { }; }\n",
    );
    assert!(m.contains("whose field `root.kids[]` is a recursive type (`Node`)"), "{m}");
    assert!(m.contains(&format!("carries {}.", carried_list())), "{m}");
}

#[test]
fn a_literal_subject_refusal_prints_the_tables_list() {
    let m = refusal(
        "type Tree = enum { Leaf(Int), Fork(Tree) };\n\
         type M { t: Tree; }\n\
         locus Rx { bus { subscribe \"m\" as on_m of type M; } fn on_m(m: M) { println(\"x\"); } }\n\
         fn main() { Rx { }; }\n",
    );
    assert!(m.contains("whose field `t.Fork.0` is a recursive type (`Tree`)"), "{m}");
    assert!(m.contains(&format!("carries {}.", carried_list())), "{m}");
}

#[test]
fn every_new_shape_checks_clean_as_a_payload() {
    let src = "type P { x: Int = 0; label: String = \"\"; }\n\
               type S = enum { Ok, Other(String), At(P), Many(bounded[String; 2]), Both((Int, String)) };\n\
               type Msg {\n\
                   tags: bounded[String; 4]; blobs: bounded[Bytes; 2]; rows: bounded[P; 2]; states: bounded[S; 2];\n\
                   docs: bounded[std::json::Value; 2]; names: [String; 2]; chunks: [Bytes; 2]; picks: [S; 2];\n\
                   status: S = S::Ok; pair: (Int, String); config: std::json::Value;\n\
               }\n\
               topic Ev { payload: Msg; subject: \"ev.x\"; }\n\
               topic Sv { payload: S; subject: \"ev.s\"; }\n\
               locus Sub {\n\
                   bus { subscribe Ev as on_ev; subscribe Sv as on_sv; }\n\
                   fn on_ev(m: Msg) { println(count(m.tags)); }\n\
                   fn on_sv(s: S) { println(\"s\"); }\n\
               }\n\
               fn main() { let s = Sub { }; }\n";
    let program = parse_source(src).expect("parses");
    let diags = entries::check_program(&program);
    assert!(diags.iter().all(|d| !d.message.contains("carry")), "{diags:#?}");
}
