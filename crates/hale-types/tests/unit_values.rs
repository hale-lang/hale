//! GH #1076, step U2: the values of identities and ranges.
//!
//! An identity (`distinct Int`) and a range (`Int { range: a..b; }`) are
//! nominal types represented as an `Int` (`hale_types::unit_values`). Each
//! rule is pinned here by its message and the source text at its span:
//! an integer literal where one is expected is a value of it, inside its
//! range; a range widens to its parent and to `Int` for free, an identity
//! to nothing; equality and ordering hold within one type; an identity has
//! no arithmetic and a range's is its `Int`'s. Every predicate that asks
//! how a value is laid out answers through the representation.

use std::collections::BTreeMap;

use hale_frontend::snapshot::{Config, Snapshot};
use hale_syntax::{parse_source, Diag};
use hale_types::capability::{FfiTypeClass, TargetClass};
use hale_types::resolve::build_top_scope;
use hale_types::ty::{is_flat_shapeable, is_key_eligible, Ty};
use hale_types::typed_bodies::{ConversionKind, ConversionRow, ConversionSite, Discharge};
use hale_types::unit_values::ScalarTypes;

#[path = "support/entries.rs"]
mod entries;
use entries::check_program;

/// The declarations every program below starts with.
const DECLS: &str = "\
type OrderId = distinct Int;
type SeqNo = distinct Int;
type Session = distinct Int { range: 0..64; }
type Byte = Int { range: 0..256; }
type Nibble = Byte { range: 0..16; }
type Lane = Session { range: 0..8; }
type RegisterId = Int { range: 0..16; }
fn wide(n: Int) -> Int { return n; }
fn byte(b: Byte) -> Byte { return b; }
fn order(o: OrderId) -> OrderId { return o; }
fn sink(e: RangeError) -> Session { return 0; }
";

fn diags(body: &str) -> (String, Vec<Diag>) {
    let src = format!("{DECLS}fn main() {{\n{body}    println(1);\n}}\n");
    let all = check_program(&parse_source(&src).expect("parses"));
    (src, all)
}

/// Every error of `body`, as (the text at its span, its message).
fn errors(body: &str) -> Vec<(String, String)> {
    let (src, all) = diags(body);
    all.iter().filter(|d| d.is_error()).map(|d| (d.span.slice(&src).to_string(), d.message.clone())).collect()
}

fn clean(body: &str) {
    let found = errors(body);
    assert!(found.is_empty(), "checks clean: {found:#?}");
}

fn one(body: &str, at: &str, message: &str) {
    assert_eq!(errors(body), [(at.to_string(), message.to_string())]);
}

#[test]
fn a_literal_where_a_scalar_is_expected_is_a_value_of_it() {
    clean(
        "    let a: OrderId = 5;\n    let s: Session = 63;\n    let b: Byte = 0;\n    let n: Nibble = 15;\n\
         let o = order(-3);\n    let c = byte(255);\n",
    );
    one("    let r: RegisterId = 20;\n", "20", "`20` is outside `RegisterId`'s range `0..16`");
    one("    let s: Session = 64;\n", "64", "`64` is outside `Session`'s range `0..64`");
    one("    let b = byte(-1);\n", "-1", "`-1` is outside `Byte`'s range `0..256`");
    // A sub-range's literal is held to its own range, not its parent's.
    one("    let n: Nibble = 16;\n", "16", "`16` is outside `Nibble`'s range `0..16`");
    // The error names the declaration.
    let (src, all) = diags("    let r: RegisterId = 20;\n");
    let related: Vec<(&str, &str)> =
        all[0].related.iter().map(|r| (r.span.slice(&src), r.label.as_str())).collect();
    assert_eq!(related, [("RegisterId", "`RegisterId` is declared here")]);
}

#[test]
fn a_literal_in_an_array_literal_is_a_value_of_its_element_type() {
    clean("    let ids: [OrderId; 2] = [1, 2];\n    let bs: [Byte; 2] = [0, 255];\n");
    one("    let bs: [Byte; 2] = [1, 300];\n", "300", "`300` is outside `Byte`'s range `0..256`");
}

#[test]
fn a_literal_with_no_expected_type_is_an_int() {
    one(
        "    let n = 5;\n    let a: OrderId = n;\n",
        "n",
        "`Int` is not `OrderId`: an identity is reached only through the explicit conversion `OrderId(…)`",
    );
    one(
        "    let n = 5;\n    let s: Session = n;\n",
        "n",
        "`Int` does not narrow to `Session` implicitly: write `Session(…) or …`, which says what becomes of a \
         value outside `0..64`",
    );
}

#[test]
fn equality_and_ordering_hold_within_one_type() {
    clean(
        "    let a: OrderId = 1;\n    let b: OrderId = 2;\n    let x = a == b;\n    let y = a < b;\n\
         let z = a != 7;\n    let s: Session = 3;\n    let t = s >= 2;\n",
    );
    // Along a widening: a sub-range and its parent, a range and `Int`.
    clean(
        "    let b: Byte = 200;\n    let n: Nibble = 3;\n    let k = 9;\n    let x = n < b;\n    let y = b == k;\n\
         let z = b < 300;\n",
    );
    one(
        "    let a: OrderId = 1;\n    let q: SeqNo = 1;\n    let x = a == q;\n",
        "a == q",
        "`OrderId` and `SeqNo` are distinct identities; convert one explicitly",
    );
    one(
        "    let a: OrderId = 1;\n    let k = 1;\n    let x = a < k;\n",
        "a < k",
        "`OrderId` and `Int` are distinct types; convert one explicitly",
    );
    one(
        "    let b: Byte = 1;\n    let s: Session = 1;\n    let x = b == s;\n",
        "b == s",
        "`Byte` and `Session` are distinct types; convert one explicitly",
    );
    // A literal facing an identity is a value of it, inside its range.
    one("    let s: Session = 1;\n    let x = s == 99;\n", "s == 99", "`99` is outside `Session`'s range `0..64`");
}

#[test]
fn an_identity_has_no_arithmetic() {
    let refused = "`OrderId` is an identity; it has no arithmetic";
    one("    let a: OrderId = 1;\n    let x = a + 1;\n", "a + 1", refused);
    one("    let a: OrderId = 1;\n    let x = a * a;\n", "a * a", refused);
    one("    let a: OrderId = 1;\n    let x = -a;\n", "-a", refused);
    one("    let mut a: OrderId = 1;\n    a += 1;\n", "a += 1;", refused);
    one(
        "    let l: Lane = 1;\n    let x = l + 1;\n",
        "l + 1",
        "`Lane` is a range of the identity `Session`; it has no arithmetic",
    );
}

#[test]
fn a_ranges_arithmetic_is_its_ints() {
    clean(
        "    let x: Byte = 200;\n    let y: Byte = 100;\n    let sum: Int = x + y;\n    let w = wide(x - y);\n\
         let neg: Int = -x;\n",
    );
    let narrows = "`Int` does not narrow to `Byte` implicitly: write `Byte(…) or …`, which says what becomes of a \
                   value outside `0..256`";
    one("    let x: Byte = 200;\n    let back: Byte = x + 1;\n", "x + 1", narrows);
    one("    let mut x: Byte = 200;\n    x += 1;\n", "x += 1;", narrows);
}

#[test]
fn a_range_widens_for_free_and_an_identity_to_nothing() {
    // Argument, return, `let`, and a sub-range into its parent.
    clean(
        "    let x: Byte = 7;\n    let w = wide(x);\n    let n: Int = x;\n    let nib: Nibble = 3;\n\
         let b: Byte = nib;\n    let c = byte(nib);\n    let l: Lane = 2;\n    let s: Session = l;\n",
    );
    one(
        "    let a: OrderId = 1;\n    let n: Int = a;\n",
        "a",
        "`OrderId` is an identity and widens to nothing: write `Int(…)` for its `Int`",
    );
    one(
        "    let a: OrderId = 1;\n    let w = wide(a);\n",
        "a",
        "`OrderId` is an identity and widens to nothing: write `Int(…)` for its `Int`",
    );
    // A parent does not narrow into its sub-range implicitly.
    one(
        "    let b: Byte = 7;\n    let n: Nibble = b;\n",
        "b",
        "`Byte` does not narrow to `Nibble` implicitly: write `Nibble(…) or …`, which says what becomes of a \
         value outside `0..16`",
    );
    one(
        "    let a: OrderId = 1;\n    let q: SeqNo = a;\n",
        "a",
        "`SeqNo` and `OrderId` are distinct identities; convert one explicitly",
    );
}

/// An integer literal is a value of an identity or a range where one is
/// expected; of a quantity it is not (U3): a count becomes a quantity by
/// a unit.
#[test]
fn an_int_literal_is_a_value_of_a_range_and_never_of_a_quantity() {
    let src = format!(
        "unit cent;\ntype Money = quantity Int in cent;\n{DECLS}\
         fn main() {{\n    let s: Session = 1;\n    let m: Money = 5;\n    println(1);\n}}\n"
    );
    let all = check_program(&parse_source(&src).expect("parses"));
    let found: Vec<(&str, &str)> =
        all.iter().filter(|d| d.is_error()).map(|d| (d.span.slice(&src), d.message.as_str())).collect();
    assert_eq!(found, [("5", "`Int` is not `Money`: a count becomes a quantity by a unit (`n * 1cent`)")]);
}

/// The program the predicate pins read: an identity and a range as a
/// hashmap key field, a topic's routing key and its payload's fields. A
/// raw literal, so the corpus harvests it and `topology_projection`'s
/// baseline holds its shape hash.
const LAYOUT: &str = r#"type OrderId = distinct Int;
type Byte = Int { range: 0..256; }
type Order { id: OrderId; qty: Byte; note: String; }
type Tick { id: OrderId; level: Byte; }
@form(hashmap)
locus Book { capacity { pool rows of Order indexed_by id; } }
topic Ticks { payload: Tick; subject: "t.ticks"; keyed_by id; }
@ffi("c") fn ext(o: OrderId, b: Byte) -> Byte;
fn main() {
    let o: OrderId = 4;
    let b: Byte = 9;
    println(o);
    println("order " + o + " byte " + b);
    let book = Book { };
    book.set(Order { id: 4, qty: 2, note: "n" });
    let got = book.get(o) or Order { id: 0, qty: 0, note: "miss" };
    println(got.qty);
}
"#;

/// On the wire an identity or a range is the `Int` it is (decision 9):
/// the topic's shape string, compiled into the binary and hashed for the
/// observer protocol, is the one the same payload with `Int` fields has.
/// (The topology's `shape_hash` renders no field type at all; its
/// baseline holds this program's row unmoved by the scalars.)
#[test]
fn a_scalar_field_hashes_as_the_int_it_is() {
    let hash = |src: &str| {
        let program = parse_source(src).expect("parses");
        let topic = program
            .items
            .iter()
            .find_map(|i| match i {
                hale_syntax::ast::TopDecl::Topic(t) => Some(t.clone()),
                _ => None,
            })
            .expect("a topic");
        let shape = hale_types::topic_identity::canonical_topic_shape(&program.items, &topic);
        (shape.clone(), hale_types::topic_identity::topic_shape_hash("t.ticks", &shape))
    };
    let as_int = LAYOUT
        .replace("type Order { id: OrderId; qty: Byte;", "type Order { id: Int; qty: Int;")
        .replace("type Tick { id: OrderId; level: Byte; }", "type Tick { id: Int; level: Int; }");
    assert_ne!(as_int, LAYOUT);
    assert_eq!(hash(LAYOUT), hash(&as_int));
    // The control: a field whose tag does change moves the hash.
    let as_text = LAYOUT.replace("type Tick { id: OrderId; level: Byte; }", "type Tick { id: OrderId; level: String; }");
    assert_ne!(hash(LAYOUT), hash(&as_text));
}

#[test]
fn every_layout_predicate_answers_through_the_representation() {
    let program = parse_source(LAYOUT).expect("parses");
    // Printable, the hashmap key field, the routing key, the FFI cells:
    // the checker accepts each.
    let errors: Vec<String> =
        check_program(&program).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
    assert!(errors.is_empty(), "{errors:#?}");

    let bundle = hale_types::Bundle::new(BTreeMap::from([(String::new(), &program)]));
    let (top, _) = build_top_scope(&bundle);
    let int = Ty::Prim(hale_syntax::ast::PrimType::Int);
    for name in ["OrderId", "Byte"] {
        let t = Ty::Named(name.to_string());
        assert_eq!(is_key_eligible(&t, &top), is_key_eligible(&int, &top), "{name}: a routing key as an `Int`");
        assert_eq!(is_flat_shapeable(&t, &top), is_flat_shapeable(&int, &top), "{name}: flat as an `Int`");
    }
    assert!(is_flat_shapeable(&Ty::Named("Tick".to_string()), &top), "a record of scalars is a flat payload");
    // The wire: tagged as the `Int` it is (decision 9).
    assert_eq!(hale_types::topic_identity::canonical_type_shape(&program.items, "Tick"), "id:i;level:i");
    assert_eq!(hale_types::topic_identity::canonical_type_shape(&program.items, "Order"), "id:i;qty:i;note:s");
    // The FFI class the cells judge is the representation's, `Int`, never
    // the `Named` class a record crosses as (its C struct).
    let snapshot = Snapshot::from_program(program.clone(), Vec::new(), Config::check(true, false))
        .unwrap_or_else(|_| panic!("shapes"));
    let rows = snapshot.demand_units().unwrap_or_else(|_| panic!("rows"));
    let scope = snapshot.demand_scope().unwrap_or_else(|_| panic!("scope"));
    let scalars = ScalarTypes::new(rows, scope);
    for name in ["OrderId", "Byte"] {
        let laid_out = scalars.representation(&Ty::Named(name.to_string()));
        assert_eq!(FfiTypeClass::of(&laid_out), FfiTypeClass::Int, "{name}");
    }
    assert_eq!(FfiTypeClass::of(&scalars.representation(&Ty::Named("Order".into()))), FfiTypeClass::Named);
    let (class, abi) = (TargetClass::PosixAsync, hale_types::capability::Abi::C);
    assert!(hale_types::capability::ffi_type_refusal(class, &int, abi).is_none());
}

/// The conversions of `body` (after [`DECLS`]), as the typed bodies hold
/// them: (the text at the row's span, from, to, kind, range, policy), in
/// source order.
fn conversions(body: &str) -> Vec<(String, String, String, ConversionKind, Option<(i128, i128)>, Option<Discharge>)> {
    let src = format!("{DECLS}fn main() {{\n{body}    println(1);\n}}\n");
    let program = parse_source(&src).expect("parses");
    let errors: Vec<String> =
        check_program(&program).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
    assert!(errors.is_empty(), "{errors:#?}");
    let snapshot = Snapshot::from_program(program, Vec::new(), Config::check(true, false))
        .unwrap_or_else(|_| panic!("shapes"));
    let table = snapshot.demand_typed_bodies().unwrap_or_else(|_| panic!("typed bodies"));
    let mut rows: Vec<&ConversionRow> = table.conversions().collect();
    rows.sort_by_key(|r| r.span.start.as_usize());
    rows.iter()
        .map(|r| (r.span.slice(&src).to_string(), r.from.display(), r.to.display(), r.kind, r.range, r.policy))
        .collect()
}

#[test]
fn a_conversion_is_a_row_of_the_typed_bodies() {
    use ConversionKind::*;
    let row = |at: &str, from: &str, to: &str, kind, range, policy| {
        (at.to_string(), from.to_string(), to.to_string(), kind, range, policy)
    };
    let session = Some((0, 64));
    assert_eq!(
        conversions(
            "    let n = 70;\n    let o = OrderId(n);\n    let i = Int(o);\n    let b: Byte = 9;\n\
             let w = Int(b);\n    let nib: Nibble = 3;\n    let up = Byte(nib);\n    let v = wide(b);\n\
             let s1 = Session(n) or 0;\n    let s2 = Session(n) or clamp;\n    let s3 = Session(n) or wrap;\n\
             let s4 = Session(n) or sink(err);\n    let down = Nibble(b) or clamp;\n"
        ),
        [
            row("OrderId(n)", "Int", "OrderId", Total, None, None),
            row("Int(o)", "OrderId", "Int", Total, None, None),
            row("Int(b)", "Byte", "Int", Widening, None, None),
            row("Byte(nib)", "Nibble", "Byte", Widening, None, None),
            row("b", "Byte", "Int", Widening, None, None),
            row("Session(n)", "Int", "Session", Narrowing, session, Some(Discharge::Substitute)),
            row("Session(n)", "Int", "Session", Narrowing, session, Some(Discharge::Clamp)),
            row("Session(n)", "Int", "Session", Narrowing, session, Some(Discharge::Wrap)),
            row("Session(n)", "Int", "Session", Narrowing, session, Some(Discharge::Handler)),
            row("Nibble(b)", "Byte", "Nibble", Narrowing, Some((0, 16)), Some(Discharge::Clamp)),
        ]
    );
}

#[test]
fn a_raise_discharges_into_the_enclosing_error_path() {
    let src = format!(
        "{DECLS}fn strict(n: Int) -> Session fallible(RangeError) {{\n    let s = Session(n) or raise;\n    return s;\n}}\n\
         fn main() {{\n    let s = strict(3) or 0;\n    println(s);\n}}\n"
    );
    let program = parse_source(&src).expect("parses");
    let errors: Vec<String> =
        check_program(&program).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
    assert!(errors.is_empty(), "{errors:#?}");
    let snapshot = Snapshot::from_program(program, Vec::new(), Config::check(true, false))
        .unwrap_or_else(|_| panic!("shapes"));
    let table = snapshot.demand_typed_bodies().unwrap_or_else(|_| panic!("typed bodies"));
    let policies: Vec<Option<Discharge>> = table.conversions().map(|r| r.policy).collect();
    assert_eq!(policies, [Some(Discharge::Raise)]);
}

#[test]
fn a_bare_narrowing_is_refused_like_a_bare_fallible_call() {
    one(
        "    let n = 70;\n    let s = Session(n);\n",
        "Session(n)",
        "`Session(…)` narrows `Int` into `Session`'s range `0..64` and this conversion says nothing about a \
         value outside it: write `or <fallback>` for a value to use instead, `or clamp` for the nearest bound, \
         `or wrap` to wrap around the range, `or handler(err)` to deal with the `RangeError` here, or `or raise` \
         to hand it to the caller",
    );
    // A total conversion has nothing to discharge.
    one(
        "    let n = 70;\n    let o = OrderId(n) or 0;\n",
        "OrderId(n)",
        "`OrderId` is not fallible (it returns `OrderId`); drop the `or` clause",
    );
    // The substitute is a value of the target, held to its range.
    one("    let n = 70;\n    let s = Session(n) or 64;\n", "64", "`64` is outside `Session`'s range `0..64`");
}

/// The sites of `kind` in `program`, in the walk's order, each with the
/// source text at its span.
fn sites_of(program: &hale_syntax::ast::Program, src: &str, kind: hale_syntax::sites::SiteKind) -> Vec<(u32, String)> {
    let mut found = Vec::new();
    hale_syntax::sites::for_each_site(program, &mut |k, span, id| {
        if k == kind {
            found.push((id.0, span.slice(src).to_string()));
        }
    });
    found
}

/// The conversion rows of the fn `name`'s body, each as (its evaluation
/// path, every step's source text from the outermost in, joined by ` / `,
/// or nothing for a cast the body writes itself; the cast; its
/// discharge), sorted.
fn rows_of(
    table: &hale_types::typed_bodies::TypedBodies,
    program: &hale_syntax::ast::Program,
    src: &str,
    name: &str,
) -> Vec<(String, String, Option<Discharge>)> {
    let steps: BTreeMap<u32, String> = sites_of(program, src, hale_syntax::sites::SiteKind::StructLiteral)
        .into_iter()
        .chain(sites_of(program, src, hale_syntax::sites::SiteKind::Call))
        .collect();
    let id = program
        .items
        .iter()
        .find_map(|i| match i {
            hale_syntax::ast::TopDecl::Fn(f) if f.name.name == name => Some(f.id),
            _ => None,
        })
        .unwrap_or_else(|| panic!("`{name}` is declared"));
    let mut rows: Vec<(String, String, Option<Discharge>)> = table
        .body(id)
        .map(|b| {
            b.conversions
                .iter()
                .map(|(site, r)| {
                    let path = match site {
                        ConversionSite::DefaultCast { path, .. } => {
                            path.iter().map(|at| steps[at].as_str()).collect::<Vec<_>>().join(" / ")
                        }
                        _ => String::new(),
                    };
                    (path, r.span.slice(src).to_string(), r.policy)
                })
                .collect()
        })
        .unwrap_or_default();
    rows.sort_by(|x, y| (&x.0, &x.1).cmp(&(&y.0, &y.1)));
    rows
}

/// U2 (fix): an omitted struct field default is typed at each literal
/// that leaves the field, and its casts' rows are that evaluation's:
/// kept in the typed body of the declaration that constructs the value,
/// by the evaluation path and the cast's call
/// (`ConversionSite::DefaultCast`), where lowering evaluates the default
/// and reads them. A literal that writes the field evaluates no default.
#[test]
fn an_omitted_defaults_casts_are_rows_of_each_evaluation() {
    let src = format!(
        "{DECLS}type S {{ o: OrderId = OrderId(1); s: Session = Session(70) or clamp; }}\n\
         fn first() -> Int {{ let s = S {{}}; return Int(s.o); }}\n\
         fn main() {{\n    let s = S {{}};\n    let t = S {{ o: OrderId(2) }};\n    println(first() + Int(s.s) + Int(t.o));\n}}\n"
    );
    let program = parse_source(&src).expect("parses");
    let errors: Vec<String> =
        check_program(&program).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
    assert!(errors.is_empty(), "{errors:#?}");
    let snapshot =
        Snapshot::from_program(program, Vec::new(), Config::check(true, false)).unwrap_or_else(|_| panic!("shapes"));
    let table = snapshot.demand_typed_bodies().unwrap_or_else(|_| panic!("typed bodies"));
    let program = snapshot.program().expect("the program");
    let rows_of = |name: &str| rows_of(table, program, &src, name);
    let row = |at: &str, cast: &str, policy| (at.to_string(), cast.to_string(), policy);
    assert_eq!(
        rows_of("first"),
        [row("", "Int(s.o)", None), row("S {}", "OrderId(1)", None), row("S {}", "Session(70)", Some(Discharge::Clamp))],
        "the literal's evaluation records the defaults' casts"
    );
    assert_eq!(
        rows_of("main"),
        [
            row("", "Int(s.s)", None),
            row("", "Int(t.o)", None),
            row("", "OrderId(2)", None),
            row("S { o: OrderId(2) }", "Session(70)", Some(Discharge::Clamp)),
            row("S {}", "OrderId(1)", None),
            row("S {}", "Session(70)", Some(Discharge::Clamp)),
        ],
        "each literal leaving a field has the row of its own evaluation; one writing the field has none"
    );
    let default_cast = sites_of(program, &src, hale_syntax::sites::SiteKind::Call)
        .into_iter()
        .find(|(_, at)| at == "OrderId(1)")
        .expect("the default's cast")
        .0;
    assert!(table.conversion(&ConversionSite::Cast(default_cast)).is_none(), "a default's cast has no evaluation-less row");
}

/// U2 (review 2): a default has a row per evaluation, and what
/// discharges its narrowing is written in the default, the same in each:
/// a bare one is refused once, at the cast, however often it is
/// evaluated.
#[test]
fn a_bare_narrowing_in_a_default_evaluated_twice_is_refused_once() {
    let src = format!(
        "{DECLS}type S {{ s: Session = Session(70); }}\n\
         fn take(s: Session = Session(71)) -> Int {{ return Int(s); }}\n\
         fn main() {{\n    let a = S {{}};\n    let b = S {{}};\n    println(take() + take() + Int(a.s) + Int(b.s));\n}}\n"
    );
    let program = parse_source(&src).expect("parses");
    let found: Vec<String> =
        check_program(&program).iter().filter(|d| d.is_error()).map(|d| d.span.slice(&src).to_string()).collect();
    assert_eq!(found, ["Session(70)", "Session(71)"]);
    // The law itself, before the check's findings are finished.
    let snapshot =
        Snapshot::from_program(program, Vec::new(), Config::check(true, false)).unwrap_or_else(|_| panic!("shapes"));
    let table = snapshot.demand_typed_bodies().unwrap_or_else(|_| panic!("typed bodies"));
    let mut judged: Vec<String> = hale_types::bare_fallible::bare_fallible_calls(table)
        .iter()
        .map(|d| d.span.slice(&src).to_string())
        .collect();
    judged.sort();
    assert_eq!(judged, ["Session(70)", "Session(71)"]);
}

/// U2 (review 2): a default's name means what each evaluation's scope
/// says. Where a local shadows the type, the cast is a call of the
/// local, and that evaluation has no row: lowering, reading the row by
/// the evaluation it lowers, lowers the call. The other evaluation of
/// the same default keeps its conversion.
#[test]
fn a_defaults_evaluation_where_a_local_shadows_the_type_has_no_row() {
    let src = format!(
        "{DECLS}type S {{ o: OrderId = OrderId(1); }}\n\
         fn bump(n: Int) -> OrderId {{ return OrderId(n + 10); }}\n\
         fn main() {{\n    let a = S {{}};\n    {{\n        let OrderId = bump;\n        let b = S {{}};\n        println(Int(b.o));\n    }}\n    println(Int(a.o));\n}}\n"
    );
    let program = parse_source(&src).expect("parses");
    let errors: Vec<String> =
        check_program(&program).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
    assert!(errors.is_empty(), "{errors:#?}");
    let snapshot =
        Snapshot::from_program(program, Vec::new(), Config::check(true, false)).unwrap_or_else(|_| panic!("shapes"));
    let table = snapshot.demand_typed_bodies().unwrap_or_else(|_| panic!("typed bodies"));
    let program = snapshot.program().expect("the program");
    let literals: Vec<u32> =
        sites_of(program, &src, hale_syntax::sites::SiteKind::StructLiteral).into_iter().map(|(id, _)| id).collect();
    let [a, b] = literals[..] else { panic!("two literals: {literals:?}") };
    let call = sites_of(program, &src, hale_syntax::sites::SiteKind::Call)
        .into_iter()
        .find(|(_, at)| at == "OrderId(1)")
        .expect("the default's cast")
        .0;
    let row = table
        .conversion(&ConversionSite::DefaultCast { path: vec![a], call })
        .expect("`a`'s evaluation is a conversion");
    assert_eq!((row.kind, row.target.as_str()), (ConversionKind::Total, "OrderId"));
    assert!(
        table.conversion(&ConversionSite::DefaultCast { path: vec![b], call }).is_none(),
        "`b`'s evaluation calls the local `OrderId`"
    );
    assert!(table.conversion(&ConversionSite::Cast(call)).is_none());
}

/// U2 (review 3): a default evaluated inside another default is keyed by
/// the whole evaluation path, not the outermost evaluation alone. One
/// `Outer {}` evaluates `Inner`'s default twice, once in each of its
/// fields' defaults; `b`'s scope shadows the type, so its path has no
/// row, while `a`'s keeps the conversion. A parameter default left by a
/// call in a field default is keyed the same way, the call one step of
/// the path.
#[test]
fn a_nested_defaults_evaluations_are_keyed_by_their_paths() {
    let src = format!(
        "{DECLS}fn bump(n: Int) -> OrderId {{ return OrderId(n + 10); }}\n\
         type Inner {{ n: OrderId = OrderId(1); }}\n\
         fn take(n: OrderId = OrderId(1)) -> OrderId {{ return n; }}\n\
         type Outer {{\n    a: Inner = Inner {{}};\n    b: Inner = {{ let OrderId = bump; Inner {{}} }};\n    \
         c: OrderId = take();\n    d: OrderId = {{ let OrderId = bump; take() }};\n}}\n\
         fn main() {{\n    let o = Outer {{}};\n    println(Int(o.a.n) + Int(o.b.n) + Int(o.c) + Int(o.d));\n}}\n"
    );
    let program = parse_source(&src).expect("parses");
    let errors: Vec<String> =
        check_program(&program).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
    assert!(errors.is_empty(), "{errors:#?}");
    let snapshot =
        Snapshot::from_program(program, Vec::new(), Config::check(true, false)).unwrap_or_else(|_| panic!("shapes"));
    let table = snapshot.demand_typed_bodies().unwrap_or_else(|_| panic!("typed bodies"));
    let program = snapshot.program().expect("the program");
    let row = |at: &str, cast: &str| (at.to_string(), cast.to_string(), None);
    assert_eq!(
        rows_of(table, program, &src, "main"),
        [
            row("", "Int(o.a.n)"),
            row("", "Int(o.b.n)"),
            row("", "Int(o.c)"),
            row("", "Int(o.d)"),
            row("Outer {} / Inner {}", "OrderId(1)"),
            row("Outer {} / take()", "OrderId(1)"),
        ],
        "`a`'s and `c`'s paths hold the conversion; `b`'s and `d`'s, where a local shadows the type, hold none"
    );
    // By key: the two nested evaluations under one `Outer {}`.
    let literals: Vec<u32> =
        sites_of(program, &src, hale_syntax::sites::SiteKind::StructLiteral).into_iter().map(|(id, _)| id).collect();
    let [inner_a, inner_b, outer] = literals[..] else { panic!("three literals: {literals:?}") };
    let call = sites_of(program, &src, hale_syntax::sites::SiteKind::Call)
        .into_iter()
        .find(|(_, at)| at == "OrderId(1)")
        .expect("`Inner`'s default cast")
        .0;
    let a = table.conversion(&ConversionSite::DefaultCast { path: vec![outer, inner_a], call });
    assert_eq!(a.map(|r| (r.kind, r.target.as_str())), Some((ConversionKind::Total, "OrderId")));
    assert!(
        table.conversion(&ConversionSite::DefaultCast { path: vec![outer, inner_b], call }).is_none(),
        "`b`'s evaluation calls the local `OrderId`"
    );
    assert!(table.conversion(&ConversionSite::DefaultCast { path: vec![outer], call }).is_none());
}

#[test]
fn a_policy_word_is_read_as_a_policy_and_a_parenthesized_one_as_a_value() {
    // `clamp` is a local here: in parentheses it is the substitute, an
    // `Int`, which does not narrow into `Session` implicitly.
    one(
        "    let n = 70;\n    let clamp = 5;\n    let s = Session(n) or (clamp);\n",
        "clamp",
        "`Int` does not narrow to `Session` implicitly: write `Session(…) or …`, which says what becomes of a \
         value outside `0..64`",
    );
    clean("    let n = 70;\n    let clamp = 5;\n    let s = Session(n) or clamp;\n");
}

#[test]
fn a_conversion_across_families_goes_through_int() {
    one(
        "    let o: OrderId = 1;\n    let q = SeqNo(o);\n",
        "SeqNo(o)",
        "`SeqNo` and `OrderId` are distinct identities; a conversion between them goes through `Int`: \
         `SeqNo(Int(…))`",
    );
    one(
        "    let b: Byte = 1;\n    let o = OrderId(b);\n",
        "OrderId(b)",
        "`OrderId` and `Byte` are distinct types; a conversion between them goes through `Int`: `OrderId(Int(…))`",
    );
    clean("    let b: Byte = 1;\n    let o = OrderId(Int(b));\n    let q = SeqNo(Int(o));\n");
}

#[test]
fn range_error_is_a_builtin_type_where_a_range_is_declared() {
    let row = hale_types::builtin_types::builtin_type("RangeError").expect("a builtin type");
    let fields: Vec<&str> = row.fields.iter().map(|(n, _)| *n).collect();
    assert_eq!(fields, ["kind", "value", "low", "high"]);
    // A handler reads its fields.
    clean("    let n = 70;\n    let s = Session(n) or sink(err);\n");
    let read = format!(
        "{DECLS}fn report(e: RangeError) -> Session {{\n    println(e.kind + \" \" + e.value + \" \" + e.low + \" \" + e.high);\n    return 0;\n}}\n\
         fn main() {{\n    println(1);\n}}\n"
    );
    let errors: Vec<String> = check_program(&parse_source(&read).expect("parses"))
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message)
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
    // Injected where a type declares a range, as `BusUnmatchedKey` is
    // where a topic fails: a program with none has no such symbol.
    let injected = |src: &str| {
        let program = parse_source(src).expect("parses");
        let bundle = hale_types::Bundle::new(BTreeMap::from([(String::new(), &program)]));
        build_top_scope(&bundle).0.lookup("RangeError").is_some()
    };
    assert!(injected("type Byte = Int { range: 0..256; }\nfn main() { println(1); }\n"));
    assert!(!injected("type OrderId = distinct Int;\nfn main() { println(1); }\n"));
}
