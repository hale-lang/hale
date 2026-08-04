//! GH #382 — edge cases and cross-feature combinations the
//! per-phase suites don't reach: relation restrictions in both
//! directions, modifier stacking, overlap degeneracies, bus-hop
//! composition for sinks and bounds, grant-form boundaries, and
//! family × contract interactions.

use hale_syntax::parse_source;

fn diags(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

// =====================================================================
// forbid: via { bus }, overlaps, masks, composition
// =====================================================================

/// `via { bus }` excludes call edges — the dual of the phase-1
/// `via { calls }` test: a call-only path must not violate a
/// bus-only claim, and a direct bus edge must.
#[test]
fn via_bus_ignores_a_call_path_and_catches_a_bus_edge() {
    let call_path = r#"
        locus B { fn work(n: Int) -> Int { return n * 2; } }
        locus A {
            params { b: B = B { }; }
            fn go(n: Int) -> Int { return self.b.work(n); }
        }
        group a_side = { A };
        group b_side = { B };
        main locus App {
            params { a: A = A { }; }
            claims { iso: forbid reaches(a_side, b_side) via { bus }; }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(call_path);
    assert!(
        !ds.iter().any(|m| m.contains("violated")),
        "a call-only path must not violate a bus-only claim: {:?}",
        ds
    );
    let bus_path = r#"
        type M { n: Int; }
        topic T { payload: M; }
        locus A {
            bus { publish T; }
            fn go(n: Int) { T <- M { n: n }; }
        }
        locus B {
            params { t: Int = 0; }
            bus { subscribe T as on_m; }
            fn on_m(m: M) { self.t = self.t + m.n; }
        }
        group a_side = { A };
        group b_side = { B };
        main locus App {
            params { a: A = A { }; b: B = B { }; }
            claims { iso: forbid reaches(a_side, b_side) via { bus }; }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(bus_path);
    assert!(
        ds.iter().any(|m| m.contains("claim `iso` violated")),
        "a direct bus edge must violate a bus-only claim: {:?}",
        ds
    );
}

/// A decl in BOTH groups is a zero-length path — a real boundary
/// confusion `forbid` surfaces rather than skips.
#[test]
fn overlapping_src_and_dst_is_a_zero_length_violation() {
    let src = r#"
        locus X { fn go() { } }
        group a_side = { X };
        group b_side = { X };
        main locus App {
            params { x: X = X { }; }
            claims { iso: forbid reaches(a_side, b_side); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    let hit = ds
        .iter()
        .find(|m| m.contains("claim `iso` violated"))
        .unwrap_or_else(|| {
            panic!("a shared decl must be a zero-length violation: {:?}", ds)
        });
    assert!(
        hit.contains("X::"),
        "the witness must name the shared decl: {}",
        hit
    );
}

/// `avoiding` overlapping an endpoint is an ERROR, not a silently
/// weaker claim — masking the target holds vacuously, masking a
/// source drops roots.
#[test]
fn a_mask_overlapping_an_endpoint_is_an_error() {
    let src = r#"
        locus B { fn work(n: Int) -> Int { return n; } }
        locus A {
            params { b: B = B { }; }
            fn go(n: Int) -> Int { return self.b.work(n); }
        }
        group a_side = { A };
        group b_side = { B };
        group gate = { B };
        main locus App {
            params { a: A = A { }; }
            claims { g: forbid reaches(a_side, b_side) avoiding gate; }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("`avoiding gate` overlaps")),
        "a mask covering the target must be rejected: {:?}",
        ds
    );
}

/// The effects sink composes through a bus hop: publish -> handler
/// -> carrier.
#[test]
fn an_effects_sink_is_reached_through_the_bus() {
    let src = r#"
        effect money;
        @effects(is: {money})
        fn charge(cents: Int) -> Int { return cents; }
        type Cmd { cents: Int; }
        topic Charge { payload: Cmd; }
        locus Api {
            bus { publish Charge; }
            fn handle(n: Int) { Charge <- Cmd { cents: n }; }
        }
        locus Worker {
            params { done: Int = 0; }
            bus { subscribe Charge as on_cmd; }
            fn on_cmd(c: Cmd) { self.done = self.done + charge(c.cents); }
        }
        group api = { Api };
        main locus App {
            params { a: Api = Api { }; w: Worker = Worker { }; }
            claims { no_spend: forbid reaches(api, effects(money)); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    let hit = ds
        .iter()
        .find(|m| m.contains("claim `no_spend` violated"))
        .unwrap_or_else(|| {
            panic!("the carrier one bus hop away must violate: {:?}", ds)
        });
    assert!(
        hit.contains("Charge") && hit.contains("Worker::on_cmd"),
        "the witness must show the bus hop: {}",
        hit
    );
}

/// The modifiers stack: `via { calls } during birth avoiding gate`
/// parses and evaluates together (the gated birth-call path holds;
/// the bypass violates).
#[test]
fn modifiers_stack_on_one_claim() {
    let gated = r#"
        locus B { fn work(n: Int) -> Int { return n * 2; } }
        locus Gate {
            params { b: B = B { }; }
            fn check(n: Int) -> Int { return self.b.work(n); }
        }
        locus A {
            params { g: Gate = Gate { }; done: Int = 0; }
            birth() { self.done = self.g.check(1); }
        }
        group a_side = { A };
        group b_side = { B };
        group gate = { Gate };
        main locus App {
            params { a: A = A { }; }
            claims {
                q: forbid reaches(a_side, b_side)
                       via { calls } during birth avoiding gate;
            }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(gated);
    assert!(
        !ds.iter().any(|m| m.contains("violated")),
        "the gated birth path must hold: {:?}",
        ds
    );
    // The bypass: A holds B directly and dodges the gate in birth.
    let bypass = gated.replace(
        "params { g: Gate = Gate { }; done: Int = 0; }\n            birth() { self.done = self.g.check(1); }",
        "params { g: Gate = Gate { }; b: B = B { }; done: Int = 0; }\n            birth() { self.done = self.b.work(1); }",
    );
    assert_ne!(gated, bypass, "the bypass rewrite must apply");
    let ds = diags(&bypass);
    assert!(
        ds.iter().any(|m| m.contains("claim `q` violated")),
        "a birth-phase bypass must violate: {:?}",
        ds
    );
}

/// `during` accepts any method name, not only lifecycle hooks.
#[test]
fn during_works_over_an_ordinary_method() {
    let src = r#"
        type M { n: Int; }
        topic T { payload: M; }
        locus A {
            bus { publish T; }
            fn tick(n: Int) { T <- M { n: n }; }
            fn quiet(n: Int) -> Int { return n; }
        }
        locus B {
            params { t: Int = 0; }
            bus { subscribe T as on_m; }
            fn on_m(m: M) { self.t = self.t + m.n; }
        }
        group a_side = { A };
        group b_side = { B };
        main locus App {
            params { a: A = A { }; b: B = B { }; }
            claims { q: forbid reaches(a_side, b_side) during tick; }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("claim `q` violated")),
        "a method-phase edge must violate its phase claim: {:?}",
        ds
    );
}

/// An explicitly `may_be_empty` group as a forbid SOURCE holds — no
/// roots, no paths, by declared intent.
#[test]
fn an_opted_out_empty_source_holds() {
    let src = r#"
        locus B { fn work() { } }
        group probes = { } may_be_empty;
        group b_side = { B };
        main locus App {
            params { b: B = B { }; }
            claims { p: forbid reaches(probes, b_side); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("violated") || m.contains("vacuously")),
        "an opted-out empty source must hold silently: {:?}",
        ds
    );
}

/// Multiple `claims { }` blocks in one main locus all evaluate.
#[test]
fn multiple_claims_blocks_all_evaluate() {
    let src = r#"
        type M { n: Int; }
        topic T { payload: M; }
        locus A {
            bus { publish T; }
            fn go(n: Int) { T <- M { n: n }; }
        }
        locus B {
            params { t: Int = 0; }
            bus { subscribe T as on_m; }
            fn on_m(m: M) { self.t = self.t + m.n; }
        }
        group a_side = { A };
        group b_side = { B };
        main locus App {
            params { a: A = A { }; b: B = B { }; }
            claims { wired: require subscribes(some b_side, topic T); }
            claims { iso: forbid reaches(a_side, b_side); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("claim `iso` violated")),
        "the second block's claim must evaluate: {:?}",
        ds
    );
    assert!(
        !ds.iter().any(|m| m.contains("claim `wired`")),
        "the first block's claim must hold: {:?}",
        ds
    );
}

// =====================================================================
// only edges: direction, ungrantable subjects
// =====================================================================

/// `only edges A -> B` constrains A→B flow only — a B→A edge is not
/// its concern.
#[test]
fn only_edges_is_directional() {
    let src = r#"
        type M { n: Int; }
        topic T { payload: M; }
        locus B {
            bus { publish T; }
            fn go(n: Int) { T <- M { n: n }; }
        }
        locus A {
            params { t: Int = 0; }
            bus { subscribe T as on_m; }
            fn on_m(m: M) { self.t = self.t + m.n; }
        }
        group a_side = { A };
        group b_side = { B };
        main locus App {
            params { a: A = A { }; b: B = B { }; }
            claims { gate: only edges a_side -> b_side { }; }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("violated")),
        "a reverse-direction edge must not violate A -> B: {:?}",
        ds
    );
}

/// A wildcard-subscribed literal subject reaching INTO the target
/// group is an edge — and it can never be granted (grants name
/// declared topics), so it fails closed.
#[test]
fn a_wildcard_edge_into_the_target_fails_closed() {
    let src = r#"
        locus A {
            bus { publish "log.app" of type Int; }
            fn go(n: Int) { "log.app" <- n; }
        }
        locus B {
            params { t: Int = 0; }
            bus { subscribe "log.**" as on_log of type Int; }
            fn on_log(n: Int) { self.t = self.t + n; }
        }
        group a_side = { A };
        group b_side = { B };
        main locus App {
            params { a: A = A { }; b: B = B { }; }
            claims { gate: only edges a_side -> b_side { }; }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("claim `gate` violated")
            && m.contains("log.app")),
        "a wildcard-covered edge into the target must be un-granted: {:?}",
        ds
    );
}

// =====================================================================
// bound: bus hops, diamonds
// =====================================================================

/// `bound` composes through a bus hop, and two carrier calls behind
/// the hop are two sites.
#[test]
fn bound_counts_sites_across_a_bus_hop() {
    let src = r#"
        effect llm;
        @effects(is: {llm})
        fn model_call(p: Int) -> Int { return p; }
        type Cmd { n: Int; }
        topic Plan { payload: Cmd; }
        locus Planner {
            bus { publish Plan; }
            fn kick(n: Int) { Plan <- Cmd { n: n }; }
        }
        locus Worker {
            params { done: Int = 0; }
            bus { subscribe Plan as on_plan; }
            fn on_plan(c: Cmd) {
                self.done = model_call(c.n) + model_call(c.n);
            }
        }
        group planners = { Planner };
        main locus App {
            params { p: Planner = Planner { }; w: Worker = Worker { }; }
            claims { one: bound llm <= 1 on paths from planners; }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("claim `one` violated")
            && m.contains("carries 2")),
        "two carrier sites one bus hop away must count: {:?}",
        ds
    );
}

/// The diamond: f calls g and h, each reaching the carrier once —
/// the per-call total is TWO (a call-tree sum, `@budget`'s
/// semantics), not a longest-path one.
#[test]
fn bound_sums_across_a_diamond() {
    let base = r#"
        effect llm;
        @effects(is: {llm})
        fn model_call(p: Int) -> Int { return p; }
        fn left(n: Int) -> Int { return model_call(n); }
        fn right(n: Int) -> Int { return model_call(n); }
        locus Planner {
            fn plan(n: Int) -> Int { return left(n) + right(n); }
        }
        group planners = { Planner };
        main locus App {
            params { p: Planner = Planner { }; }
            claims { one: bound llm <= LIMIT on paths from planners; }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(&base.replace("LIMIT", "1"));
    assert!(
        ds.iter().any(|m| m.contains("claim `one` violated")
            && m.contains("carries 2")),
        "a diamond must sum to two sites: {:?}",
        ds
    );
    let ds = diags(&base.replace("LIMIT", "2"));
    assert!(
        !ds.iter().any(|m| m.contains("violated")),
        "two sites within a limit of two must hold: {:?}",
        ds
    );
}

// =====================================================================
// require / count: remaining operators and error paths
// =====================================================================

/// `count` with `>=` and `<=`.
#[test]
fn count_ge_and_le_operators() {
    let base = r#"
        type M { n: Int; }
        topic T { payload: M; }
        locus P {
            bus { publish T; }
            fn go(n: Int) { T <- M { n: n }; }
        }
        locus S1 {
            params { t: Int = 0; }
            bus { subscribe T as on_m; }
            fn on_m(m: M) { self.t = self.t + m.n; }
        }
        locus S2 {
            params { t: Int = 0; }
            bus { subscribe T as on_m; }
            fn on_m(m: M) { self.t = self.t + m.n; }
        }
        main locus App {
            params { p: P = P { }; a: S1 = S1 { }; b: S2 = S2 { }; }
            claims { CLAIM }
        }
        fn main() { App { }; }
    "#;
    // >= 3 with two subscribers: violated.
    let ds = diags(&base.replace(
        "CLAIM",
        "c: count subscribers(topic T) >= 3;",
    ));
    assert!(
        ds.iter().any(|m| m.contains("claim `c` violated")
            && m.contains("counted 2")),
        ">= must fail under the floor: {:?}",
        ds
    );
    // <= 1 with two subscribers: violated, naming both.
    let ds = diags(&base.replace(
        "CLAIM",
        "c: count subscribers(topic T) <= 1;",
    ));
    assert!(
        ds.iter().any(|m| m.contains("claim `c` violated")
            && m.contains("S1")
            && m.contains("S2")),
        "<= must fail over the ceiling and name the loci: {:?}",
        ds
    );
    // >= 2 with two subscribers: holds.
    let ds = diags(&base.replace(
        "CLAIM",
        "c: count subscribers(topic T) >= 2;",
    ));
    assert!(
        !ds.iter().any(|m| m.contains("violated")),
        ">= at the floor must hold: {:?}",
        ds
    );
}

/// An unresolved qualified topic in `require` is an error.
#[test]
fn an_unresolved_qualified_topic_in_require_is_an_error() {
    let src = r#"
        locus A { fn go() { } }
        group a_side = { A };
        main locus App {
            params { a: A = A { }; }
            claims { w: require subscribes(some a_side, topic nosuch::T); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("does not resolve")),
        "a dangling qualified topic must error: {:?}",
        ds
    );
}

/// `cover` over an alias that names no imported topics is an error
/// (empty coverage domain = vacuity), single-seed included.
#[test]
fn cover_over_an_unknown_alias_is_an_error() {
    let src = r#"
        locus A { fn go() { } }
        group a_side = { A };
        main locus App {
            params { a: A = A { }; }
            claims { c: cover topic in seed(t): subscribed_by(some a_side); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("names no import alias")),
        "an unknown seed alias must error: {:?}",
        ds
    );
}

// =====================================================================
// families × contracts
// =====================================================================

const FAMILY: &str = r#"
    domain wing = { delta, gamma };
    effect knowledge(wing);
    @effects(is: {knowledge(delta)})
    fn read_delta(k: Int) -> Int { return k; }
    @effects(is: {knowledge(gamma)})
    fn read_gamma(k: Int) -> Int { return k; }
"#;

/// The #354 inheritance the reduction buys: an `only:` naming one
/// index REJECTS the other via the complement computed from the
/// live universe.
#[test]
fn an_only_contract_over_one_index_rejects_the_other() {
    let ds = diags(&format!(
        "{FAMILY}@effects(only: {{knowledge(delta), alloc}})\n\
         fn f(n: Int) -> Int {{ return read_gamma(n); }}\n\
         fn main() {{ println(f(5)); }}"
    ));
    assert!(
        ds.iter().any(|m| m.contains("closed effect contract violated")),
        "the complement must reject the unlisted index: {:?}",
        ds
    );
    let ds = diags(&format!(
        "{FAMILY}@effects(only: {{knowledge(delta), alloc}})\n\
         fn f(n: Int) -> Int {{ return read_delta(n); }}\n\
         fn main() {{ println(f(5)); }}"
    ));
    assert!(
        !ds.iter().any(|m| m.contains("closed effect contract violated")),
        "the listed index must pass its own contract: {:?}",
        ds
    );
}

/// A composed class may contain an instantiation.
#[test]
fn a_composed_class_may_contain_an_instantiation() {
    let ds = diags(&format!(
        "{FAMILY}effect sensitive = {{ knowledge(delta) }};\n\
         @effects(none: {{sensitive}})\n\
         fn f(n: Int) -> Int {{ return read_delta(n); }}\n\
         fn main() {{ println(f(5)); }}"
    ));
    assert!(
        ds.iter().any(|m| m.contains("effect assertion violated")),
        "a composed class over an instantiation must fire: {:?}",
        ds
    );
}

/// `effects(knowledge(*))` as a claim sink covers every index.
#[test]
fn the_star_family_works_as_a_claim_sink() {
    let src = format!(
        "{FAMILY}\
         locus Quote {{ fn handle(n: Int) -> Int {{ return read_gamma(n); }} }}\n\
         group quote_api = {{ Quote }};\n\
         main locus App {{\n\
             params {{ q: Quote = Quote {{ }}; }}\n\
             claims {{ iso: forbid reaches(quote_api, effects(knowledge(*))); }}\n\
         }}\n\
         fn main() {{ App {{ }}; }}"
    );
    let ds = diags(&src);
    assert!(
        ds.iter().any(|m| m.contains("claim `iso` violated")),
        "the star sink must cover every index: {:?}",
        ds
    );
}

/// Projection vacuity: a group of fn-less loci passes the
/// decl-grain guard but projects to no executable vertices — a
/// claim over it proves nothing and must refuse, at either
/// endpoint.
#[test]
fn a_projection_empty_group_is_an_error_at_either_endpoint() {
    let base = r#"
        locus Store { params { cap: Int = 8; } }
        locus A { fn go(n: Int) -> Int { return n; } }
        group data = { Store };
        group a_side = { A };
        main locus App {
            params { s: Store = Store { }; a: A = A { }; }
            claims { CLAIM }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(&base.replace(
        "CLAIM",
        "iso: forbid reaches(data, a_side);",
    ));
    assert!(
        ds.iter().any(|m| m.contains("projects to no executable")
            && m.contains("source")),
        "a fn-less source must refuse: {:?}",
        ds
    );
    let ds = diags(&base.replace(
        "CLAIM",
        "iso: forbid reaches(a_side, data);",
    ));
    assert!(
        ds.iter().any(|m| m.contains("projects to no executable")
            && m.contains("target")),
        "a fn-less target must refuse: {:?}",
        ds
    );
    let ds = diags(&base.replace(
        "CLAIM",
        "one: bound llm <= 1 on paths from data;",
    ));
    // (`llm` is undeclared here — both errors are acceptable
    // evidence that the claim refused; assert the projection one
    // fires when the class exists.)
    let ds2 = diags(
        &base
            .replace("locus Store", "effect llm;\n        locus Store")
            .replace(
                "CLAIM",
                "one: bound llm <= 1 on paths from data;",
            ),
    );
    assert!(
        ds2.iter().any(|m| m.contains("projects to no executable")),
        "a fn-less bound source must refuse: {:?} (first run: {:?})",
        ds2,
        ds
    );
}

// =====================================================================
// The unresolved-callee backstop (#382 soundness audit)
// =====================================================================
//
// Four receiver shapes land in the call graph as `Unresolved` with
// no receiver type — a struct-literal receiver, a chained field, a
// call result, a branch value — and a walk that ignored them
// certified a `forbid` while the forbidden path executed at
// runtime. The backstop fails closed when such a call's NAME
// matches a method of the claim's target set.

fn backstop_fixture(go_body: &str) -> String {
    format!(
        r#"
        locus B {{ fn work(n: Int) -> Int {{ return n * 2; }} }}
        locus Mid {{ params {{ inner: B = B {{ }}; }} }}
        fn make_b() -> B {{ return B {{ }}; }}
        locus A {{
            params {{ mid: Mid = Mid {{ }}; }}
            fn go(n: Int) -> Int {{ {go_body} }}
        }}
        group a_side = {{ A }};
        group b_side = {{ B }};
        main locus App {{
            params {{ a: A = A {{ }}; }}
            claims {{ iso: forbid reaches(a_side, b_side); }}
        }}
        fn main() {{ App {{ }}; }}
    "#
    )
}

fn assert_fails_closed(body: &str, shape: &str) {
    let ds = diags(&backstop_fixture(body));
    assert!(
        ds.iter().any(|m| m.contains("claim `iso` cannot be certified")
            && m.contains("receiver the compiler cannot type")),
        "{} must fail closed, not certify: {:?}",
        shape,
        ds
    );
}

/// AUDIT SHAPE 3: struct-literal receiver.
#[test]
fn a_literal_receiver_call_fails_closed() {
    assert_fails_closed("return B { }.work(n);", "a literal receiver");
}

/// AUDIT SHAPE 5: chained field receiver.
#[test]
fn a_chained_field_receiver_call_fails_closed() {
    assert_fails_closed(
        "return self.mid.inner.work(n);",
        "a chained field receiver",
    );
}

/// AUDIT SHAPE 6: call-result receiver.
#[test]
fn a_call_result_receiver_call_fails_closed() {
    assert_fails_closed(
        "let b = make_b(); return b.work(n);",
        "a call-result receiver",
    );
}

/// AUDIT SHAPE 8: branch-valued receiver.
#[test]
fn a_branch_valued_receiver_call_fails_closed() {
    assert_fails_closed(
        "let b = if n > 0 { B { } } else { B { } }; return b.work(n);",
        "a branch-valued receiver",
    );
}

/// The backstop's own control: the same unresolved shape with a
/// name the target set does NOT declare stays certifiable — the
/// backstop keys on the target's method names, not on every
/// unresolved edge.
#[test]
fn an_unresolved_call_not_matching_the_target_certifies() {
    let src = r#"
        locus B { fn work(n: Int) -> Int { return n * 2; } }
        locus C { fn other(n: Int) -> Int { return n + 1; } }
        locus A {
            fn go(n: Int) -> Int { return C { }.other(n); }
        }
        group a_side = { A };
        group b_side = { B };
        main locus App {
            params { a: A = A { }; }
            claims { iso: forbid reaches(a_side, b_side); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("cannot be certified")),
        "an unresolved name outside the target set must not trip \
         the backstop: {:?}",
        ds
    );
}

/// The backstop covers `only edges` and `bound` too.
#[test]
fn the_backstop_covers_only_edges_and_bound() {
    let only = backstop_fixture("return B { }.work(n);").replace(
        "iso: forbid reaches(a_side, b_side);",
        "gate: only edges a_side -> b_side { };",
    );
    let ds = diags(&only);
    assert!(
        ds.iter().any(|m| m.contains("claim `gate` cannot be certified")),
        "only edges must fail closed on the untyped receiver: {:?}",
        ds
    );
    let bound = r#"
        effect llm;
        locus Model {
            @effects(is: {llm})
            fn ask(p: Int) -> Int { return p; }
        }
        locus Planner {
            fn plan(n: Int) -> Int { return Model { }.ask(n); }
        }
        group planners = { Planner };
        main locus App {
            params { p: Planner = Planner { }; }
            claims { one: bound llm <= 5 on paths from planners; }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(bound);
    assert!(
        ds.iter().any(|m| m.contains("claim `one` violated")
            && m.contains("unbounded")),
        "bound must treat an untyped carrier-named call as \
         unbounded: {:?}",
        ds
    );
}

/// Domain declaration guards: empty and duplicate domains are parse
/// errors.
#[test]
fn domain_decl_guards() {
    let errs = parse_source("domain wing = { };\nfn main() { }")
        .expect_err("must reject");
    assert!(
        errs.iter().any(|e| e.message.contains("has no members")),
        "got: {:?}",
        errs
    );
    let errs = parse_source(
        "domain wing = { delta };\ndomain wing = { gamma };\nfn main() { }",
    )
    .expect_err("must reject");
    assert!(
        errs.iter()
            .any(|e| e.message.contains("declared more than once")),
        "got: {:?}",
        errs
    );
}
