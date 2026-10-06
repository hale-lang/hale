//! GH #1076, step U4: `Time` and `Duration` are declarations.
//!
//! The stdlib's seed (`crates/hale-stdlib/hl/time.hl`) declares the time
//! catalogue (`ns us ms s min h day`), `type Duration = quantity Int in
//! ns;` and `type Time = point Duration;`. Their rows are the stdlib's
//! universe's, beside a program's; a literal's suffix is a unit of that
//! catalogue; the algebra types the two. Pinned here: the rows and the
//! universe key; the literal counts; and the representation class the two
//! keep, contract by contract, at the values the hand-written primitives
//! had (the shape tags `u` and `t`, the topic shape string and its hash,
//! key eligibility and flat payloads, the FFI classes). Printing and the
//! wire's lowering are pinned where they are built
//! (`hale-codegen/tests/duration_scalar_arith.rs`).

use hale_syntax::ast::{PrimType, TopDecl};
use hale_syntax::parse_source;
use hale_types::capability::FfiTypeClass;
use hale_types::placement::SiteUniverse;
use hale_types::ty::Ty;
use hale_types::unit_graph::{Denom, Ratio};
use hale_types::units::{stdlib_literal, stdlib_rows, ScalarKindRow};

#[path = "support/entries.rs"]
mod entries;
use entries::check_program;

fn ratio(n: u64, d: u64) -> Ratio {
    Ratio::new(n.into(), d.into()).unwrap()
}

/// The stdlib's rows: seven units and six equations of one component,
/// `Duration` its quantity counted in `ns`, `Time` a point over it with
/// no origin; every row of the stdlib's universe, each declaration the
/// primitive a type position reads by its name.
#[test]
fn the_time_catalogue_is_the_stdlibs_rows() {
    let rows = stdlib_rows();
    let names: Vec<&str> = rows.units.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(names, ["ns", "us", "ms", "s", "min", "h", "day"]);
    assert_eq!(rows.equations.len(), 6);
    assert!(rows.units.iter().all(|u| u.site.universe == SiteUniverse::StdlibAnalysis));
    assert!(rows.units.iter().all(|u| u.component == rows.units[0].component), "one component");
    let catalogue = rows.catalogue.as_ref().expect("the catalogue closes");
    let one = |name: &str| Denom { unit: rows.units[rows.unit_named(name).unwrap()].site, multiple: Ratio::one() };
    for (unit, ns) in [
        ("us", 1_000u64),
        ("ms", 1_000_000),
        ("s", 1_000_000_000),
        ("min", 60_000_000_000),
        ("h", 3_600_000_000_000),
        ("day", 86_400_000_000_000),
    ] {
        assert_eq!(catalogue.factor(&one(unit), &one("ns")), Some(ratio(ns, 1)), "{unit}");
    }
    let names: Vec<(&str, ScalarKindRow, Option<PrimType>)> =
        rows.scalars.iter().map(|s| (s.name.as_str(), s.kind, s.primitive)).collect();
    assert_eq!(
        names,
        [("Duration", ScalarKindRow::Quantity, Some(PrimType::Duration)), ("Time", ScalarKindRow::Point, Some(PrimType::Time))]
    );
    let duration = &rows.scalars[0];
    assert!(duration.principal);
    assert_eq!(duration.denomination.as_ref(), Some(&one("ns")));
    assert_eq!(rows.scalars[1].origin, None, "a `Time` counts from the epoch the runtime defines");
    assert_eq!(duration.ty(), Ty::Prim(PrimType::Duration));
    assert_eq!(rows.scalars[1].ty(), Ty::Prim(PrimType::Time));
}

/// A time literal is a quantity literal of the stdlib's unit, a whole
/// count of `Duration`'s nanoseconds: the constant the lexer used to
/// compute is the catalogue's. `m` and `d` are no time units (`min`,
/// `day`), and a unit no catalogue declares has no count.
#[test]
fn a_time_literal_is_its_count_of_nanoseconds() {
    let duration = Ty::Prim(PrimType::Duration);
    for (value, unit, ns) in [
        (500, "ms", 500_000_000i64),
        (1_000, "ns", 1_000),
        (7, "us", 7_000),
        (3, "s", 3_000_000_000),
        (2, "min", 120_000_000_000),
        (5, "h", 18_000_000_000_000),
        (1, "day", 86_400_000_000_000),
    ] {
        assert_eq!(stdlib_literal(value, unit), Some((ns, duration.clone())), "{value}{unit}");
    }
    assert_eq!(stdlib_literal(5, "m"), None);
    assert_eq!(stdlib_literal(3, "d"), None);
    assert_eq!(stdlib_literal(3, "cent"), None);
}

/// The universe key: a program's units are rows of the program's
/// universe beside the stdlib's, the catalogue one graph of disjoint
/// components, joined only where a program writes an equation against a
/// stdlib unit.
#[test]
fn a_programs_units_are_beside_the_stdlibs_and_join_by_an_equation() {
    let program = parse_source("unit cent;\nunit USD = 100 cent;\nunit tick = 10 ms;\nfn main() { }\n").expect("parses");
    assert!(check_program(&program).iter().all(|d| !d.is_error()));
    let snapshot =
        hale_frontend::snapshot::Snapshot::from_program(program, Vec::new(), hale_frontend::snapshot::Config::check(true, false))
            .unwrap_or_else(|_| panic!("shapes"));
    let rows = snapshot.demand_units().unwrap_or_else(|_| panic!("rows"));
    let unit = |name: &str| &rows.units[rows.unit_named(name).unwrap()];
    assert_eq!(unit("ms").site.universe, SiteUniverse::StdlibAnalysis);
    assert_eq!(unit("cent").site.universe, SiteUniverse::User);
    // The stdlib's first unit and the program's first share a `SiteId`
    // number in their own universes; the key keeps them apart.
    assert_ne!(unit("ns").site, unit("cent").site);
    assert_ne!(unit("cent").component, unit("ns").component, "two components");
    assert_eq!(unit("tick").component, unit("ns").component, "`tick` joins the time component");
}

/// A topic whose payload has a `Duration` and a `Time` field: its shape
/// string tags them `u` and `t`, as the primitives always were, so its
/// hash is the one an observer built before U4 computes; and the two are
/// routing keys, flat payload fields and FFI classes of their own.
const WIRE: &str = r#"type Tick { id: Int; wait: Duration; at: Time; }
topic Ticked { payload: Tick; subject: "t.ticks"; keyed_by wait; }
locus Clock {
    bus { publish Ticked; }
    run() {
        Ticked <- Tick { id: 1, wait: 1500ms, at: std::time::current() };
    }
}
fn main() {
    let c = Clock { };
    println(1500ms);
}
"#;

#[test]
fn the_two_keep_their_representation_class() {
    let program = parse_source(WIRE).expect("parses");
    let errors: Vec<String> = check_program(&program).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(hale_types::topic_identity::canonical_type_shape(&program.items, "Tick"), "id:i;wait:u;at:t");
    let topic = program
        .items
        .iter()
        .find_map(|i| match i {
            TopDecl::Topic(t) => Some(t.clone()),
            _ => None,
        })
        .expect("a topic");
    let shape = hale_types::topic_identity::canonical_topic_shape(&program.items, &topic);
    assert_eq!(shape, "id:i;wait:u;at:t");
    // FNV-1a/64 of `t.ticks:id:i;wait:u;at:t`, the observer protocol's
    // shape hash (`lotus_obs.c::obs_fnv`).
    assert_eq!(hale_types::topic_identity::topic_shape_hash("t.ticks", &shape), TICK_SHAPE_HASH);
    let bundle = hale_types::Bundle::new(std::collections::BTreeMap::from([(String::new(), &program)]));
    let (top, _) = hale_types::resolve::build_top_scope(&bundle);
    for p in [PrimType::Duration, PrimType::Time] {
        let t = Ty::Prim(p);
        assert!(hale_types::ty::is_key_eligible(&t, &top), "{p:?} is a routing key");
        assert!(hale_types::ty::is_flat_shapeable(&t, &top), "{p:?} is a flat payload field");
    }
    assert!(hale_types::ty::is_flat_shapeable(&Ty::Named("Tick".into()), &top), "a flat payload");
    assert_eq!(FfiTypeClass::of(&Ty::Prim(PrimType::Duration)), FfiTypeClass::Duration);
    assert_eq!(FfiTypeClass::of(&Ty::Prim(PrimType::Time)), FfiTypeClass::Time);
    // An `@ffi` fn takes and returns a `Duration` as the class it is,
    // `Int`-sized nanoseconds on the C ABI.
    let ffi = "@ffi(\"c\") fn f(d: Duration) -> Duration;\nfn main() { println(f(5ms)); }\n";
    let errors: Vec<String> =
        check_program(&parse_source(ffi).expect("parses")).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

const TICK_SHAPE_HASH: u64 = 4251888922411353198;
