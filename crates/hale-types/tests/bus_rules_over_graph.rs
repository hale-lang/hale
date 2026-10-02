//! F.40 phase 3, C4 (the second judgment migration): rules 9 and 10
//! and the dead-receiver rule (7) are judged over the bus graph
//! (spec/semantics.md rules 7, 9 and 10). Each test pins one class of
//! what the migration moved, by the diagnostic's wording and by the
//! graph's column the rule now reads:
//!
//! - a subject is compared under the canonical key, the wire subject:
//!   a topic published by its declared name and subscribed by its
//!   literal subject is one subject, and a literal that spells a
//!   topic's name rather than its wire is another subject;
//! - a subject the graph cannot resolve is a hole, never an orphan;
//! - the walk's bound, cross-seed and wildcard facts are columns of the
//!   graph's wire rows.

use std::collections::BTreeMap;

use hale_syntax::ast::Program;
use hale_syntax::parse_source;
use hale_types::bus_graph::{build_bus_graph, BusGraph};
use hale_types::resolve::build_top_scope;
use hale_types::{check_program, Bundle};

fn check(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

fn graph(src: &str) -> BusGraph {
    let prog = parse_source(src).expect("parse failed");
    let mut programs: BTreeMap<String, &Program> = BTreeMap::new();
    programs.insert(String::new(), &prog);
    let bundle = Bundle::new(programs);
    let (top, _) = build_top_scope(&bundle);
    build_bus_graph(&bundle, &top)
}

fn orphans(msgs: &[String]) -> Vec<&String> {
    msgs.iter()
        .filter(|m| {
            m.contains("has no subscriber")
                || m.contains("never published")
                || m.contains("neither published nor subscribed")
        })
        .collect()
}

// --- rule 9: the canonical key ------------------------------------

const NAME_AND_LITERAL: &str = r#"
type Tick { n: Int; }
topic Beat { payload: Tick; subject: "demo.beat"; }

locus P {
    bus { publish Beat; }
    birth() { Beat <- Tick { n: 1 }; }
}
locus S {
    bus { subscribe "demo.beat" as on_b of type Tick; }
    fn on_b(t: Tick) { }
}
main locus App {
    params { p: P = P { }; s: S = S { }; }
}
fn main() { App { }; }
"#;

/// Published by the declared name, subscribed by the literal wire
/// subject: one subject, both ends wired, no orphan.
#[test]
fn a_topic_published_by_name_and_subscribed_by_its_literal_subject_is_one_subject() {
    let msgs = check(NAME_AND_LITERAL);
    assert!(orphans(&msgs).is_empty(), "one subject, wired at both ends: {msgs:?}");
    let g = graph(NAME_AND_LITERAL);
    let row = g.wires.get("demo.beat").expect("the topic's wire is a row");
    assert!(row.published.is_some() && row.subscribed.is_some(), "{row:?}");
    assert!(!g.wires.contains_key("Beat"), "the topic's name is not a subject");
}

const SPELLS_THE_NAME: &str = r#"
type Tick { n: Int; }
topic Beat { payload: Tick; subject: "demo.beat"; }

locus P {
    bus { publish Beat; }
    birth() { Beat <- Tick { n: 1 }; }
}
locus S {
    bus { subscribe "Beat" as on_b of type Tick; }
    fn on_b(t: Tick) { }
}
main locus App {
    params { p: P = P { }; s: S = S { }; }
}
fn main() { App { }; }
"#;

/// A literal subject that spells the topic's NAME is the wire subject
/// `"Beat"`, which the topic's cells never reach (spec/model.md rule
/// 8). Before the migration rule 9 matched a literal by the topic's
/// name as well as its wire and said nothing; both ends are now
/// reported.
#[test]
fn a_literal_that_spells_a_topics_name_is_another_subject() {
    let msgs = check(SPELLS_THE_NAME);
    assert_eq!(
        orphans(&msgs),
        [
            "bus topic `Beat` is published but has no subscriber — the cells go nowhere. \
             Add a `subscribe` for it, bind it to a transport, or drop the publish.",
            "bus subject `\"Beat\"` is subscribed but never published — its handler can't \
             fire. Add a `publish`, bind it to a transport, or drop the subscription.",
        ],
        "{msgs:?}"
    );
}

// --- rule 9: holes --------------------------------------------------

const UNRESOLVED: &str = r#"
type Tick { n: Int; }
locus P {
    bus { publish Nowhere; }
    birth() { }
}
main locus App {
    params { p: P = P { }; }
}
fn main() { App { }; }
"#;

/// A topic name no declaration answers is an unresolved subject: the
/// resolver's error stands, the graph records the hole, and rule 9 does
/// not call it an orphan. Before the migration it also warned
/// "bus subject `"Nowhere"` is published but has no subscriber".
#[test]
fn an_unresolved_subject_is_a_hole_not_an_orphan() {
    let msgs = check(UNRESOLVED);
    assert!(
        msgs.iter().any(|m| m.contains("publish references unknown topic `Nowhere`")),
        "the resolver still reports the name: {msgs:?}"
    );
    assert!(orphans(&msgs).is_empty(), "an unresolved subject is not a proven orphan: {msgs:?}");
    let g = graph(UNRESOLVED);
    assert_eq!(g.holes.len(), 1, "{:?}", g.holes);
    assert_eq!((g.holes[0].written.as_str(), g.holes[0].locus.as_str(), g.holes[0].publish), ("Nowhere", "P", true));
    assert!(!g.wires.contains_key("Nowhere"), "a hole is not a subject");
}

// --- rule 9: the walk's facts are columns ---------------------------

const COLUMNS: &str = r#"
type Tick { n: Int; }
type Line { s: String = ""; }
topic Beat { payload: Tick; subject: "beat"; }
topic Feed { payload: Tick; subject: "feed"; }
topic Shared { payload: Tick; subject: "shared"; }

locus Producer {
    bus { publish Beat; publish "log.app" of type Line; publish Shared; }
    birth() { Beat <- Tick { n: 1 }; }
}
locus Logger {
    bus { subscribe "log.**" as on_log of type Line; subscribe other::Shared as on_shared; }
    fn on_log(l: Line) { }
    fn on_shared(t: Tick) { }
}
locus Gateway {
    fn send(subject: String, bytes: Bytes) { }
}
main locus App {
    params { p: Producer = Producer { }; l: Logger = Logger { }; }
    bindings { Beat: Gateway { }; }
}
fn main() { App { }; }
"#;

/// Each fact rule 9 reads is a column of the subject's row: `beat` is
/// bound, `log.app` is covered by a `**` subscription, `shared` may be
/// named by the unresolved cross-seed path `other::Shared` (a hole of
/// its own), and `feed` carries none of them.
#[test]
fn the_walks_bound_cross_seed_and_wildcard_facts_are_columns() {
    let g = graph(COLUMNS);
    let beat = &g.wires["beat"];
    assert!(beat.bound && !beat.cross_seed && !beat.subscribed_by_pattern, "{beat:?}");
    let log = &g.wires["log.app"];
    assert!(log.subscribed_by_pattern && !log.bound && log.published.is_some(), "{log:?}");
    let shared = &g.wires["shared"];
    assert!(shared.cross_seed && shared.published.is_some() && shared.subscribed.is_none(), "{shared:?}");
    let feed = &g.wires["feed"];
    assert_eq!(*feed, Default::default(), "an untouched topic is a row with no facts");
    assert!(!g.wires.contains_key("log.**"), "a pattern is a column, not a row");
    assert_eq!(g.holes.iter().map(|h| h.written.as_str()).collect::<Vec<_>>(), ["other::Shared"]);
    // Rule 9 reads them: the bound, covered and cross-seed subjects are
    // not orphans; the untouched topic is dead wiring.
    let msgs = check(COLUMNS);
    assert_eq!(
        orphans(&msgs),
        ["bus topic `Feed` is declared but neither published nor subscribed — it's dead wiring."],
        "{msgs:?}"
    );
}

// --- rule 10: edges by declaration ----------------------------------

fn cycles(msgs: &[String]) -> Vec<&String> {
    msgs.iter().filter(|m| m.contains("bus cycle")).collect()
}

const TWO_OF_ONE_NAME: &str = r#"
type Tick { n: Int; }
topic A { payload: Tick; subject: "a"; }
topic B { payload: Tick; subject: "b"; }

module left {
    locus W {
        bus { subscribe A as on_a; publish B; }
        fn on_a(x: Tick) { B <- Tick { n: 1 }; }
    }
}
module right {
    locus W {
        bus { subscribe B as on_b; publish A; }
        fn on_b(x: Tick) { A <- Tick { n: 1 }; }
    }
}

main locus App {
    params { l: left::W = left::W { }; r: right::W = right::W { }; }
}

fn main() { App { }; }
"#;

/// Two loci of one name (the duplicate is its own error) each wrote one
/// edge: `A → B` in `left::W`, `B → A` in `right::W`. The loop crosses
/// two declarations, so it is the cross-locus warning, naming both by
/// their module paths. Before the migration rule 10 merged the edges by
/// name and reported "locus `W` has a re-entrant synchronous bus cycle".
#[test]
fn two_loci_of_one_name_have_their_own_edges() {
    let msgs = check(TWO_OF_ONE_NAME);
    assert!(msgs.iter().any(|m| m.contains("duplicate top-level name `W`")), "{msgs:?}");
    assert_eq!(
        cycles(&msgs),
        ["bus cycle `A → B → A` across loci (left::W, right::W): a cell can re-trigger its \
          own publish, spinning the cooperative queue. Break the loop or add a terminating \
          condition."],
        "{msgs:?}"
    );
    let g = graph(TWO_OF_ONE_NAME);
    let by_decl: Vec<(Vec<String>, &str, &str)> = g
        .edges
        .iter()
        .map(|e| (g.decls[e.decl].modules.clone(), e.from.as_str(), e.to.as_str()))
        .collect();
    assert_eq!(
        by_decl,
        [(vec!["left".to_string()], "a", "b"), (vec!["right".to_string()], "b", "a")],
        "each edge is its own declaration's, between wire subjects"
    );
}

const NAME_SENT_LITERAL_SUBSCRIBED: &str = r#"
type Tick { n: Int; }
topic T { payload: Tick; subject: "t"; }

locus Echo {
    bus { subscribe "t" as on_t of type Tick; publish T; }
    fn on_t(x: Tick) { T <- Tick { n: 1 }; }
}
main locus App {
    params { e: Echo = Echo { }; }
}
fn main() { App { }; }
"#;

/// A handler subscribed to the literal `"t"` that sends `T` by name
/// sends to its own subject: one subject under the canonical key, so
/// the unconditional self-republish is the intra-locus error. Before
/// the migration the edge ran from `t` to `T`, two nodes, and rule 10
/// said nothing.
#[test]
fn a_send_by_name_meets_a_subscription_by_literal_subject() {
    let msgs = check(NAME_SENT_LITERAL_SUBSCRIBED);
    assert!(
        msgs.iter().any(|m| m
            == "locus `Echo` has a re-entrant synchronous bus cycle `t → t`: each publish onto \
                a topic the locus also subscribes is a direct in-thread call (intra-locus \
                self-dispatch), so this recurses without bound and overflows the stack. Break \
                the cycle, or route one hop through a different pool (an async enqueue)."),
        "{msgs:?}"
    );
}

const UNRESOLVED_CYCLE: &str = r#"
type Tick { n: Int; }
locus Echo {
    bus { subscribe Nowhere as on_n; publish Nowhere; }
    fn on_n(x: Tick) { Nowhere <- Tick { n: 1 }; }
}
main locus App {
    params { e: Echo = Echo { }; }
}
fn main() { App { }; }
"#;

/// A subject no topic answers forms no edge: the resolver reports the
/// name, and rule 10 does not build a cycle out of the spelling. Before
/// the migration it also reported "locus `Echo` has a re-entrant
/// synchronous bus cycle `Nowhere → Nowhere`".
#[test]
fn an_unresolved_subject_forms_no_edge() {
    let msgs = check(UNRESOLVED_CYCLE);
    assert!(msgs.iter().any(|m| m.contains("unknown topic `Nowhere`")), "{msgs:?}");
    assert!(!msgs.iter().any(|m| m.contains("re-entrant") || m.contains("bus cycle")), "{msgs:?}");
    assert!(graph(UNRESOLVED_CYCLE).edges.is_empty());
}
