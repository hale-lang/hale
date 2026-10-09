//! GH #1327 §1 — `cover keys(topic T [in LO..=HI]): delivered_to(exactly_one G)`:
//! every permitted key of a keyed topic reaches exactly one
//! registration of a named group.
//!
//! Discipline (soundness law 4): the claim ships with the canaries
//! that MUST fail — a gap, an overlap, an unknown filter — beside the
//! programs that must hold, including the audit subscriber that
//! shares the topic but sits outside the group.

#[path = "support/entries.rs"]
mod entries;
use hale_syntax::parse_source;

fn diags(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    entries::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

fn claim_diags(src: &str) -> Vec<String> {
    diags(src)
        .into_iter()
        .filter(|m| m.contains("claim `route`"))
        .collect()
}

/// Four replicated workers shard `Orders` by `where key == replica`.
/// `EXTRA_LOCI` / `EXTRA_PARAMS` / `GROUP` / `RANGE` are the knobs.
const SHARDED: &str = r#"
type Order { shard: Int = 0; qty: Int = 0; }
topic Orders { payload: Order; subject: "orders"; keyed_by shard; }

locus Worker {
    params { seen: Int = 0; }
    bus { subscribe Orders as on_order where key == replica; }
    fn on_order(o: Order) { self.seen = self.seen + o.qty; }
}

EXTRA_LOCI

group workers = { GROUP };

main locus App {
    params { w: Worker = Worker { }; EXTRA_PARAMS }
    placement { w: pinned(cores = 0..4, replicas = 4); }
    bus { publish Orders; }
    claims {
        route: cover keys(topic Orders RANGE): delivered_to(exactly_one workers);
    }
    run() { Orders <- Order { shard: 1, qty: 2 }; }
}
fn main() { App { }; }
"#;

fn sharded(extra_loci: &str, extra_params: &str, group: &str, range: &str) -> String {
    SHARDED
        .replace("EXTRA_LOCI", extra_loci)
        .replace("EXTRA_PARAMS", extra_params)
        .replace("GROUP", group)
        .replace("RANGE", range)
}

#[test]
fn every_key_has_one_recipient_holds() {
    let src = sharded("", "", "Worker", "in 0..=3");
    assert_eq!(claim_diags(&src), Vec::<String>::new(), "{:?}", diags(&src));
    assert!(diags(&src).is_empty(), "{:?}", diags(&src));
}

#[test]
fn a_gap_names_the_uncovered_interval() {
    let src = sharded("", "", "Worker", "in 0..=5");
    let ds = claim_diags(&src);
    assert_eq!(ds.len(), 1, "{:?}", ds);
    assert!(ds[0].contains("violated"), "{}", ds[0]);
    assert!(ds[0].contains("uncovered"), "{}", ds[0]);
    assert!(ds[0].contains("keys 4..=5"), "{}", ds[0]);
    assert!(ds[0].contains("no registration of `workers`"), "{}", ds[0]);
}

#[test]
fn a_single_uncovered_key_is_named_as_a_key() {
    let src = sharded("", "", "Worker", "in 0..=4");
    let ds = claim_diags(&src);
    assert_eq!(ds.len(), 1, "{:?}", ds);
    assert!(ds[0].contains("key 4 "), "{}", ds[0]);
}

#[test]
fn the_unbounded_int_domain_names_both_unbounded_gaps() {
    // No stated interval: the permitted keys are whatever the publish
    // site can produce — every `Int`. Four replicas cover 0..=3 only.
    let src = sharded("", "", "Worker", "");
    let ds = claim_diags(&src);
    assert_eq!(ds.len(), 1, "{:?}", ds);
    assert!(
        ds[0].contains("keys -9223372036854775808..=-1, 4..=9223372036854775807"),
        "{}",
        ds[0]
    );
}

/// Two loci both claim key 0.
const OVERLAP: &str = r#"
type Order { shard: Int = 0; qty: Int = 0; }
topic Orders { payload: Order; subject: "orders"; keyed_by shard; }

locus Alpha {
    params { seen: Int = 0; }
    bus { subscribe Orders as on_alpha where key == 0; }
    fn on_alpha(o: Order) { self.seen = self.seen + o.qty; }
}
locus Beta {
    params { seen: Int = 0; }
    bus { subscribe Orders as on_beta where key == 0; }
    fn on_beta(o: Order) { self.seen = self.seen + o.qty; }
}
locus Gamma {
    params { seen: Int = 0; }
    bus { subscribe Orders as on_gamma where key == 1; }
    fn on_gamma(o: Order) { self.seen = self.seen + o.qty; }
}

group workers = { Alpha, Beta, Gamma };

main locus App {
    params { a: Alpha = Alpha { }; b: Beta = Beta { }; g: Gamma = Gamma { }; }
    bus { publish Orders; }
    claims {
        route: cover keys(topic Orders in 0..=1): delivered_to(exactly_one workers);
    }
    run() { Orders <- Order { shard: 0, qty: 2 }; }
}
fn main() { App { }; }
"#;

#[test]
fn an_overlap_names_both_registrations() {
    let ds = claim_diags(OVERLAP);
    assert_eq!(ds.len(), 1, "{:?}", ds);
    assert!(ds[0].contains("violated"), "{}", ds[0]);
    assert!(ds[0].contains("overlapping"), "{}", ds[0]);
    assert!(ds[0].contains("2 registrations of `workers` receive key 0"), "{}", ds[0]);
    assert!(ds[0].contains("Alpha::on_alpha"), "{}", ds[0]);
    assert!(ds[0].contains("Beta::on_beta"), "{}", ds[0]);
    assert!(!ds[0].contains("Gamma::on_gamma"), "{}", ds[0]);
    assert!(!ds[0].contains("uncovered"), "{}", ds[0]);
}

const AUDIT: &str = "
locus Audit {
    params { n: Int = 0; }
    bus { subscribe Orders as on_any; }
    fn on_any(o: Order) { self.n = self.n + 1; }
}
";

#[test]
fn an_audit_subscriber_outside_the_group_does_not_break_uniqueness() {
    let src = sharded(AUDIT, "audit: Audit = Audit { };", "Worker", "in 0..=3");
    assert_eq!(claim_diags(&src), Vec::<String>::new(), "{:?}", diags(&src));
}

#[test]
fn the_same_audit_subscriber_inside_the_group_is_an_overlap() {
    let src = sharded(AUDIT, "audit: Audit = Audit { };", "Worker, Audit", "in 0..=3");
    let ds = claim_diags(&src);
    assert_eq!(ds.len(), 1, "{:?}", ds);
    assert!(ds[0].contains("Audit::on_any"), "{}", ds[0]);
    assert!(ds[0].contains("Worker::on_order"), "{}", ds[0]);
}

#[test]
fn two_instances_of_one_literal_subscriber_are_two_registrations() {
    let src = r#"
type Order { shard: Int = 0; qty: Int = 0; }
topic Orders { payload: Order; subject: "orders"; keyed_by shard; }
locus Worker {
    params { seen: Int = 0; }
    bus { subscribe Orders as on_order where key == 7; }
    fn on_order(o: Order) { self.seen = self.seen + o.qty; }
}
group workers = { Worker };
main locus App {
    params { a: Worker = Worker { }; b: Worker = Worker { }; }
    bus { publish Orders; }
    claims {
        route: cover keys(topic Orders in 7..=7): delivered_to(exactly_one workers);
    }
    run() { Orders <- Order { shard: 7, qty: 2 }; }
}
fn main() { App { }; }
"#;
    let ds = claim_diags(src);
    assert_eq!(ds.len(), 1, "{:?}", ds);
    assert!(ds[0].contains("App.a"), "{}", ds[0]);
    assert!(ds[0].contains("App.b"), "{}", ds[0]);
}

#[test]
fn an_unknown_filter_is_uncertified_never_holds() {
    let src = r#"
type Order { shard: Int = 0; qty: Int = 0; }
topic Orders { payload: Order; subject: "orders"; keyed_by shard; }
locus Worker {
    params { id: Int = 0; seen: Int = 0; }
    bus { subscribe Orders as on_order where key == self.id; }
    fn on_order(o: Order) { self.seen = self.seen + o.qty; }
}
group workers = { Worker };
main locus App {
    params { w: Worker = Worker { id: 0 }; }
    bus { publish Orders; }
    claims {
        route: cover keys(topic Orders in 0..=0): delivered_to(exactly_one workers);
    }
    run() { Orders <- Order { shard: 0, qty: 2 }; }
}
fn main() { App { }; }
"#;
    let ds = claim_diags(src);
    assert_eq!(ds.len(), 1, "{:?}", ds);
    assert!(ds[0].contains("cannot be certified"), "{}", ds[0]);
    assert!(ds[0].contains("not statically known"), "{}", ds[0]);
    assert!(ds[0].contains("Worker::on_order"), "{}", ds[0]);
}

#[test]
fn an_unknown_filter_outside_the_group_still_withdraws_the_answer() {
    // A fallback hears only what no filter, in ANY group, matched; an
    // outside filter nobody can evaluate may absorb any key.
    let src = sharded(
        "locus Odd {
    params { id: Int = 0; }
    bus { subscribe Orders as on_odd where key == self.id; }
    fn on_odd(o: Order) { self.id = o.qty; }
}",
        "odd: Odd = Odd { id: 9 };",
        "Worker",
        "in 0..=3",
    );
    let ds = claim_diags(&src);
    assert_eq!(ds.len(), 1, "{:?}", ds);
    assert!(ds[0].contains("cannot be certified"), "{}", ds[0]);
    assert!(ds[0].contains("Odd::on_odd"), "{}", ds[0]);
}

#[test]
fn a_fallback_in_the_group_covers_every_other_key() {
    let src = r#"
type Order { shard: Int = 0; qty: Int = 0; }
topic Orders { payload: Order; subject: "orders"; keyed_by shard; on_unmatched: fallback; }
locus Even {
    params { seen: Int = 0; }
    bus { subscribe Orders as on_even where key == 0; }
    fn on_even(o: Order) { self.seen = self.seen + o.qty; }
}
locus Rest {
    params { seen: Int = 0; }
    bus { subscribe Orders as on_rest where key == _; }
    fn on_rest(o: Order) { self.seen = self.seen + o.qty; }
}
group workers = { Even, Rest };
main locus App {
    params { e: Even = Even { }; r: Rest = Rest { }; }
    bus { publish Orders; }
    claims {
        route: cover keys(topic Orders): delivered_to(exactly_one workers);
    }
    run() { Orders <- Order { shard: 0, qty: 2 }; }
}
fn main() { App { }; }
"#;
    assert_eq!(claim_diags(src), Vec::<String>::new(), "{:?}", diags(src));
}

#[test]
fn a_fallback_outside_the_group_leaves_the_rest_uncovered() {
    let src = r#"
type Order { shard: Int = 0; qty: Int = 0; }
topic Orders { payload: Order; subject: "orders"; keyed_by shard; on_unmatched: fallback; }
locus Even {
    params { seen: Int = 0; }
    bus { subscribe Orders as on_even where key == 0; }
    fn on_even(o: Order) { self.seen = self.seen + o.qty; }
}
locus Rest {
    params { seen: Int = 0; }
    bus { subscribe Orders as on_rest where key == _; }
    fn on_rest(o: Order) { self.seen = self.seen + o.qty; }
}
group workers = { Even };
main locus App {
    params { e: Even = Even { }; r: Rest = Rest { }; }
    bus { publish Orders; }
    claims {
        route: cover keys(topic Orders in 0..=2): delivered_to(exactly_one workers);
    }
    run() { Orders <- Order { shard: 0, qty: 2 }; }
}
fn main() { App { }; }
"#;
    let ds = claim_diags(src);
    assert_eq!(ds.len(), 1, "{:?}", ds);
    assert!(ds[0].contains("keys 1..=2"), "{}", ds[0]);
    assert!(ds[0].contains("`Rest::on_rest`"), "{}", ds[0]);
}

#[test]
fn an_unkeyed_topic_is_invalid() {
    let src = r#"
type Order { shard: Int = 0; }
topic Orders { payload: Order; subject: "orders"; }
locus Worker {
    params { seen: Int = 0; }
    bus { subscribe Orders as on_order; }
    fn on_order(o: Order) { self.seen = o.shard; }
}
group workers = { Worker };
main locus App {
    params { w: Worker = Worker { }; }
    bus { publish Orders; }
    claims {
        route: cover keys(topic Orders in 0..=0): delivered_to(exactly_one workers);
    }
    run() { Orders <- Order { shard: 0 }; }
}
fn main() { App { }; }
"#;
    let ds = claim_diags(src);
    assert_eq!(ds.len(), 1, "{:?}", ds);
    assert!(ds[0].contains("is malformed"), "{}", ds[0]);
    assert!(ds[0].contains("is not keyed"), "{}", ds[0]);
}

#[test]
fn an_interval_on_a_non_int_key_is_invalid() {
    let src = r#"
type Order { tag: String = ""; }
topic Orders { payload: Order; subject: "orders"; keyed_by tag; }
locus Worker {
    params { seen: Int = 0; }
    bus { subscribe Orders as on_order where key == "a"; }
    fn on_order(o: Order) { self.seen = self.seen + 1; }
}
group workers = { Worker };
main locus App {
    params { w: Worker = Worker { }; }
    bus { publish Orders; }
    claims {
        route: cover keys(topic Orders in 0..=3): delivered_to(exactly_one workers);
    }
    run() { Orders <- Order { tag: "a" }; }
}
fn main() { App { }; }
"#;
    let ds = claim_diags(src);
    assert_eq!(ds.len(), 1, "{:?}", diags(src));
    assert!(ds[0].contains("is malformed"), "{}", ds[0]);
    assert!(ds[0].contains("integer keys"), "{}", ds[0]);
}

#[test]
fn a_group_with_no_locus_members_is_invalid() {
    let src = sharded("", "", "Worker", "in 0..=3")
        .replace("group workers = { Worker };", "group workers = { };");
    let ds = claim_diags(&src);
    assert_eq!(ds.len(), 1, "{:?}", diags(&src));
    assert!(ds[0].contains("is malformed"), "{}", ds[0]);
    assert!(ds[0].contains("no locus members"), "{}", ds[0]);
}

#[test]
fn no_stated_interval_and_no_publisher_is_uncertified() {
    let src = sharded("", "", "Worker", "in 0..=3").replace("bus { publish Orders; }", "");
    let src = src.replace("run() { Orders <- Order { shard: 1, qty: 2 }; }", "run() { }");
    let none = src.replace(" in 0..=3", "");
    let ds = claim_diags(&none);
    assert_eq!(ds.len(), 1, "{:?}", ds);
    assert!(ds[0].contains("cannot be certified"), "{}", ds[0]);
    assert!(ds[0].contains("no publish site"), "{}", ds[0]);
}

#[test]
fn an_empty_interval_is_a_parse_error() {
    let src = sharded("", "", "Worker", "in 3..=0");
    let err = parse_source(&src).expect_err("an empty interval must not parse");
    assert!(format!("{:?}", err).contains("is empty"), "{:?}", err);
}

#[test]
fn a_negative_bound_parses() {
    let src = sharded("", "", "Worker", "in -2..=3");
    let ds = claim_diags(&src);
    assert_eq!(ds.len(), 1, "{:?}", ds);
    assert!(ds[0].contains("keys -2..=-1"), "{}", ds[0]);
}

#[test]
fn a_subscriber_born_outside_the_arrangement_is_uncertified() {
    // A listed instance population is a lower bound once the locus
    // can also be born at runtime: the claim cannot count recipients.
    let src = sharded("", "", "Worker", "in 0..=3").replace(
        "run() { Orders <- Order { shard: 1, qty: 2 }; }",
        "run() { Orders <- Order { shard: 1, qty: 2 }; Worker { }; }",
    );
    let ds = claim_diags(&src);
    assert_eq!(ds.len(), 1, "{:?}", diags(&src));
    assert!(ds[0].contains("cannot be certified"), "{}", ds[0]);
    assert!(ds[0].contains("population"), "{}", ds[0]);
}
