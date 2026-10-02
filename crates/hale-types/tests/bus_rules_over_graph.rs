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
import "../other" as other;

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
    fn on_shared(t: other::Tick) { }
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
/// named by the cross-seed path `other::Shared` (a hole of its own),
/// and `feed` carries none of them.
///
/// `other::Shared` is a real imported topic: the seed imported as
/// `other` declares it, and the workspace of both seeds checks clean
/// and builds (`hale-cli`'s `unresolved_qualified.rs` runs this program
/// beside that seed). The bundle here holds this seed alone, as an
/// editor's per-directory bundle does, so the import is one the bundle
/// never resolved (GH #724): the checker keeps the path a hole without
/// a word, and the graph records it and reads its leaf as the
/// cross-seed fact. A path that names no declaration in a whole program
/// is an error instead (`a_qualified_subject_naming_no_declaration_is_refused`).
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
    // The program checks clean: the hole is the unresolved import's.
    let prog = parse_source(COLUMNS).expect("parse failed");
    let errors: Vec<String> =
        check_program(&prog).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect();
    assert!(errors.is_empty(), "{errors:?}");
    // Rule 9 reads them: the bound, covered and cross-seed subjects are
    // not orphans; the untouched topic is dead wiring.
    let msgs = check(COLUMNS);
    assert_eq!(
        orphans(&msgs),
        ["bus topic `Feed` is declared but neither published nor subscribed — it's dead wiring."],
        "{msgs:?}"
    );
}

/// In a whole program a qualified subject that names no declaration is
/// the resolver's "unknown topic", located at the path, in both verbs:
/// the build would otherwise lower a subject with no topic and no
/// payload and refuse it without a span. Here no seed is imported as
/// `other`; `hale-cli`'s `unresolved_qualified.rs` pins the library
/// that lacks the name.
#[test]
fn a_qualified_subject_naming_no_declaration_is_refused() {
    let src = COLUMNS.replace("import \"../other\" as other;\n", "");
    let msgs = check(&src);
    assert!(
        msgs.iter().any(|m| m
            == "subscribe references unknown topic `other::Shared` (`other` is not an import \
                of this seed, so no `topic Shared` declaration is in scope)"),
        "{msgs:?}"
    );
    let msgs = check(&src.replace("subscribe other::Shared as on_shared;", "").replace(
        "publish Shared;",
        "publish other::Shared;",
    ));
    assert!(
        msgs.iter().any(|m| m
            == "publish references unknown topic `other::Shared` (`other` is not an import \
                of this seed, so no `topic Shared` declaration is in scope)"),
        "{msgs:?}"
    );
}

// --- rule 7: the dead receiver reads the declaration's row ----------

/// A non-main cooperative gateway whose `run()` blocks, with the given
/// `bus { }` members.
fn gateway(bus: &str) -> String {
    format!(
        r#"
type Tick {{ n: Int; }}
topic T {{ payload: Tick; subject: "t"; }}

locus Gateway {{
    bus {{ {bus} }}
    fn on_t(t: Tick) {{ }}
    fn on_other(t: Tick) {{ }}
    run() {{
        let b = std::bytes::BytesBuilder {{ initial_cap: 64 }};
        let n = std::io::tls::recv_into(0, b, 64);
    }}
}}

main locus App {{
    params {{ gw: Gateway = Gateway {{ }}; }}
    placement {{ gw: cooperative(pool = ws); }}
}}

fn main() {{ App {{ }}; }}
"#
    )
}

fn dead_receiver(msgs: &[String]) -> Vec<&String> {
    msgs.iter().filter(|m| m.contains("subscribes to bus topics")).collect()
}

/// Published by the declared name, subscribed by the literal subject:
/// one subject under the canonical key, so the subscription is a
/// self-publish and the error lists only the other subject's handler.
/// The rule said the same before the migration (its own key was the
/// wire); the graph's split by `BusSubject::canonical()` would have
/// listed `on_t` too.
#[test]
fn the_dead_receiver_lists_the_handlers_of_subjects_it_does_not_publish() {
    let msgs = check(&gateway(
        r#"publish T; subscribe "t" as on_t of type Tick; subscribe "other" as on_other of type Tick;"#,
    ));
    assert_eq!(
        dead_receiver(&msgs),
        ["locus `Gateway` (field `gw`) subscribes to bus topics (on_other) but its `run()` makes \
          the blocking call `std::io::tls::recv_into` while placed `cooperative(pool = ws)`. The \
          blocking call monopolizes the pool's thread, so the dispatch that would deliver those \
          cells never runs — the handlers can't fire. (An event-driven subscriber that yields — \
          handlers plus a `time::sleep` loop, or `where async_io` — receives fine; the problem \
          is the blocking call, not the placement.) Use `pinned` (its own thread + a mailbox \
          drained at sleep/yield), or keep `run()` non-blocking."],
        "{msgs:?}"
    );
    // With the self-published subscription alone there is no receive
    // to starve: the rule does not fire.
    let msgs = check(&gateway(r#"publish T; subscribe "t" as on_t of type Tick;"#));
    assert!(dead_receiver(&msgs).is_empty(), "{msgs:?}");
}

/// A subscription the graph cannot resolve (a qualified path through an
/// import the bundle never resolved) is compared as written: it is not
/// a self-publish of anything the locus publishes, so its handler is
/// listed.
#[test]
fn an_unresolved_subscription_is_compared_as_written() {
    let src = format!(
        "import \"../other\" as other;\n{}",
        gateway(r#"publish T; subscribe other::T as on_other;"#)
    );
    let msgs = check(&src);
    assert!(!msgs.iter().any(|m| m.contains("unknown topic")), "{msgs:?}");
    let dead = dead_receiver(&msgs);
    assert_eq!(dead.len(), 1, "{msgs:?}");
    assert!(dead[0].starts_with("locus `Gateway` (field `gw`) subscribes to bus topics (on_other) "), "{dead:?}");
}

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
/// the unconditional self-republish is a cycle. Before the migration
/// the edge ran from `t` to `T`, two nodes, and rule 10 said nothing.
/// It is not the synchronous error: the intra-locus rewrite makes a
/// direct call only of a send to a topic subscribed by its name, so
/// this send is queued (the relation holds no row for it), and the
/// cycle is the queue's warning. The same locus subscribing `T` by
/// name is rewritten, and is the error.
#[test]
fn a_send_by_name_meets_a_subscription_by_literal_subject() {
    let msgs = check(NAME_SENT_LITERAL_SUBSCRIBED);
    assert_eq!(
        cycles(&msgs),
        ["bus cycle `t → t` in locus `Echo`: a cell can re-trigger its own publish, spinning \
          the cooperative queue. Break the loop or add a terminating condition."],
        "{msgs:?}"
    );
    assert!(!msgs.iter().any(|m| m.contains("re-entrant")), "{msgs:?}");

    let by_name = NAME_SENT_LITERAL_SUBSCRIBED.replace("subscribe \"t\" as on_t of type Tick", "subscribe T as on_t");
    let msgs = check(&by_name);
    assert!(
        msgs.iter().any(|m| m
            == "locus `Echo` has a re-entrant synchronous bus cycle `T → T`: each publish onto \
                a topic the locus also subscribes is a direct in-thread call (intra-locus \
                self-dispatch), so this recurses without bound and overflows the stack. Break \
                the cycle, or route one hop through a different pool (an async enqueue)."),
        "{msgs:?}"
    );
}

/// The check of a bundle of parsed programs no entry minted
/// (`Bundle::new` + `check_bundle`), as a library caller makes one.
fn check_unminted(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    let bundle = Bundle::new(BTreeMap::from([("main.hl".to_string(), &prog)]));
    hale_types::check_bundle(&bundle).into_iter().map(|d| d.message).collect()
}

const SELF_RECURSION_BY_NAME: &str = r#"
type Tick { n: Int; }
topic T { payload: Tick; subject: "t"; }
locus Echo {
    bus { publish T; subscribe T as on_t; }
    fn on_t(t: Tick) { T <- Tick { n: t.n + 1 }; }
    run() { T <- Tick { n: 0 }; }
}
main locus App { params { e: Echo = Echo { }; } }
fn main() { App { }; }
"#;

/// A locus subscribing its own topic by name is rewritten to a direct
/// call, so the self-republish is the re-entrant error on the bundle
/// entry too, as it is through `check_program` and the snapshot. That
/// entry once built the bus graph over the parsed programs, whose sends
/// had no ids, and the relation over a copy it numbered on its own: the
/// join matched nothing and the recursion read as the queue's warning.
/// The bundle is now numbered once, and both are derived from it.
#[test]
fn a_bundle_no_entry_minted_joins_the_graph_to_the_rewrite() {
    let error = "locus `Echo` has a re-entrant synchronous bus cycle `T → T`: each publish onto \
                 a topic the locus also subscribes is a direct in-thread call (intra-locus \
                 self-dispatch), so this recurses without bound and overflows the stack. Break \
                 the cycle, or route one hop through a different pool (an async enqueue).";
    for msgs in [check(SELF_RECURSION_BY_NAME), check_unminted(SELF_RECURSION_BY_NAME)] {
        assert_eq!(cycles(&msgs), [error], "{msgs:?}");
        assert!(!msgs.iter().any(|m| m.starts_with("internal:")), "{msgs:?}");
    }

    // The send the rewrite leaves on the queue is still the warning on
    // the same entry.
    let msgs = check_unminted(NAME_SENT_LITERAL_SUBSCRIBED);
    assert_eq!(
        cycles(&msgs),
        ["bus cycle `t → t` in locus `Echo`: a cell can re-trigger its own publish, spinning \
          the cooperative queue. Break the loop or add a terminating condition."],
        "{msgs:?}"
    );
    assert!(!msgs.iter().any(|m| m.contains("re-entrant") || m.starts_with("internal:")), "{msgs:?}");
}

/// Rule 10's join of a send to the intra-locus rewrite's relation needs
/// the send's id, and a relation holding no row for a send is what a
/// queued send looks like. Handed the inputs the bundle entry used to
/// build (the graph over the parsed programs, the relation over a copy
/// numbered on its own), the check refuses the cycle as an internal
/// failure naming the send rather than judging it queued.
#[test]
fn an_unnumbered_send_is_refused_at_the_join() {
    use hale_types::check::{check_bundle_scoped, CheckInputs};

    let prog = parse_source(SELF_RECURSION_BY_NAME).expect("parse failed");
    let bundle = Bundle::new(BTreeMap::from([(String::new(), &prog)]));
    let (top, _) = build_top_scope(&bundle);
    let handlers = hale_types::handler_routing::handler_rows(&[&prog], &[], &bundle.snapshot);
    let alloc_summary = std::sync::Arc::new(hale_types::alloc_summary::derive_alloc_summary(&bundle));
    let rows = std::cell::OnceCell::new();
    let effects = || {
        Some(rows.get_or_init(|| {
            hale_types::effect_rows::derive_effect_rows(&bundle, &top, alloc_summary.clone())
        }))
    };
    let entry = hale_types::entry::entry_row(&bundle);
    let placement = hale_types::placement::derive_placement(&bundle, &top, &entry);
    let forms = hale_types::form_rows::form_rows(&bundle, &top, &placement, true);
    let bus = build_bus_graph(&bundle, &top);
    let intra_locus = hale_types::resolved::rewrite_intra_locus(&prog).intra_locus;
    assert!(!intra_locus.is_empty(), "the rewrite makes the self-send a direct call");
    let inputs = CheckInputs {
        top: &top,
        handlers: &handlers,
        effects: &effects,
        entry: &entry,
        alloc_summary: &alloc_summary,
        forms: &forms,
        bus: &bus,
        intra_locus: &intra_locus,
        placement: &placement,
    };
    let diags = check_bundle_scoped(&bundle, &inputs, false, false, false);
    let cycles: Vec<(bool, &str)> = diags
        .iter()
        .filter(|d| d.message.contains("bus cycle"))
        .map(|d| (d.is_error(), d.message.as_str()))
        .collect();
    assert_eq!(
        cycles,
        [(
            true,
            "internal: the send to `t` in handler `on_t` of locus `Echo` carries no identity, so \
             the bus cycle `T → T` cannot be joined to the intra-locus rewrite's relation to tell \
             a direct call from a queued send. The check was handed a bundle whose programs were \
             never numbered."
        )]
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
