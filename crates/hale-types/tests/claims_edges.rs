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
// Receiver typing (#382 soundness audit — the root fix)
// =====================================================================
//
// Four receiver shapes — a struct-literal receiver, a chained
// field, a call result, a branch value — used to land as untyped
// unresolved edges the walk silently dropped: a claim certified
// while the forbidden path executed at runtime. The summarizer now
// TYPES those receivers, so each shape resolves to a real edge and
// the claim reports a real witness. The fail-closed backstop
// remains for what stays genuinely untypeable (an index result).

fn typing_fixture(go_body: &str) -> String {
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

fn assert_real_witness(body: &str, shape: &str) {
    let ds = diags(&typing_fixture(body));
    let hit = ds
        .iter()
        .find(|m| m.contains("claim `iso` violated"))
        .unwrap_or_else(|| {
            panic!("{} must resolve to a real violation: {:?}", shape, ds)
        });
    assert!(
        hit.contains("B::work"),
        "{} must produce a witness reaching the target: {}",
        shape,
        hit
    );
    assert!(
        !hit.contains("cannot be certified"),
        "{} must be a resolved edge, not a fail-closed one: {}",
        shape,
        hit
    );
}

/// AUDIT SHAPE 3: struct-literal receiver — resolved.
#[test]
fn a_literal_receiver_resolves_to_a_real_witness() {
    assert_real_witness("return B { }.work(n);", "a literal receiver");
}

/// AUDIT SHAPE 5: chained field receiver — resolved through the
/// per-locus field maps.
#[test]
fn a_chained_field_receiver_resolves_to_a_real_witness() {
    assert_real_witness(
        "return self.mid.inner.work(n);",
        "a chained field receiver",
    );
}

/// AUDIT SHAPE 6: call-result receiver — resolved through the
/// free-fn return-type map.
#[test]
fn a_call_result_receiver_resolves_to_a_real_witness() {
    assert_real_witness(
        "let b = make_b(); return b.work(n);",
        "a call-result receiver",
    );
}

/// AUDIT SHAPE 8: branch-valued receiver — resolved when every
/// branch types to the same locus.
#[test]
fn a_branch_valued_receiver_resolves_to_a_real_witness() {
    assert_real_witness(
        "let b = if n > 0 { B { } } else { B { } }; return b.work(n);",
        "a branch-valued receiver",
    );
}

/// The follow-up review's wrapper counterexample — now a RESOLVED
/// two-hop witness, not a fail-closed refusal.
#[test]
fn a_wrapper_through_an_untyped_receiver_resolves_end_to_end() {
    let src = r#"
        locus B { fn work(n: Int) -> Int { return n * 2; } }
        locus Bridge {
            params { b: B = B { }; }
            fn hop(n: Int) -> Int { return self.b.work(n); }
        }
        locus A {
            fn go(n: Int) -> Int { return Bridge { }.hop(n); }
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
    let hit = ds
        .iter()
        .find(|m| m.contains("claim `iso` violated"))
        .unwrap_or_else(|| {
            panic!("the wrapper must resolve to a violation: {:?}", ds)
        });
    assert!(
        hit.contains("Bridge::hop") && hit.contains("B::work"),
        "the witness must carry the full wrapper path: {}",
        hit
    );
}

/// The resolution CONTROL: a resolved edge to a locus OUTSIDE the
/// target group certifies — typing the receiver means no more
/// blanket fail-closed on these shapes.
#[test]
fn a_resolved_receiver_outside_the_target_certifies() {
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
        !ds.iter().any(|m| m.contains("violated")
            || m.contains("cannot be certified")),
        "a typed literal receiver outside the target must certify: {:?}",
        ds
    );
}

/// Wrapper reaching an effect carrier / bound carrier — resolved
/// with real verdicts (the follow-up review's other two
/// counterexamples).
#[test]
fn wrapper_effect_and_bound_carriers_resolve() {
    let effects = r#"
        effect money;
        @effects(is: {money})
        fn charge(n: Int) -> Int { return n; }
        locus Bridge {
            fn hop(n: Int) -> Int { return charge(n); }
        }
        locus A {
            fn go(n: Int) -> Int { return Bridge { }.hop(n); }
        }
        group a_side = { A };
        main locus App {
            params { a: A = A { }; }
            claims { no_spend: forbid reaches(a_side, effects(money)); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(effects);
    assert!(
        ds.iter().any(|m| m.contains("claim `no_spend` violated")),
        "the wrapper to the carrier must be a real violation: {:?}",
        ds
    );
    let bound = r#"
        effect llm;
        @effects(is: {llm})
        fn ask(n: Int) -> Int { return n; }
        locus Bridge {
            fn hop(n: Int) -> Int { return ask(n); }
        }
        locus Planner {
            fn plan(n: Int) -> Int { return Bridge { }.hop(n); }
        }
        group planners = { Planner };
        main locus App {
            params { p: Planner = Planner { }; }
            claims { none: bound llm <= 0 on paths from planners; }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(bound);
    assert!(
        ds.iter().any(|m| m.contains("claim `none` violated")
            && m.contains("carries 1")),
        "the wrapper carrier must COUNT (one site, limit zero): {:?}",
        ds
    );
}

// =====================================================================
// The fail-closed backstop, for what stays genuinely untypeable
// =====================================================================

/// An INDEX-result receiver has no declared type at this layer —
/// the backstop still refuses to certify over it.
#[test]
fn an_index_receiver_still_fails_closed() {
    let src = r#"
        locus B { fn work(n: Int) -> Int { return n * 2; } }
        locus A {
            fn go(n: Int) -> Int {
                let xs = [B { }];
                return xs[0].work(n);
            }
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
        ds.iter().any(|m| m.contains("claim `iso` cannot be certified")
            && m.contains("receiver the compiler cannot type")),
        "an index receiver must fail closed: {:?}",
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

// =====================================================================
// Interface dispatch in the witness (downstream review, item 5)
// =====================================================================
//
// The compiler fans an interface call out to EVERY conforming locus,
// which is sound and deliberately conservative. But the witness used
// to render that hop as an ordinary direct call, so a reader looking
// at a line that constructs `Email` and a witness that names
// `Sms::send` concluded the checker was wrong.
//
// A correct proof that reads as a bug is expensive: people stop
// trusting the checker, or work around it. The fact was already in
// the model (`via_interface`); it just never reached the human.

const DISPATCH: &str = r#"
interface Notifier { fn send(msg: String); }

locus Email {
    params { n: Int = 0; }
    fn send(msg: String) { self.n = self.n + 1; }
}
locus Sms {
    params { n: Int = 0; }
    fn send(msg: String) { self.n = self.n + 1; }
}

locus A {
    params { n: Int = 0; }
    fn go() {
        let e = Email { };
        notify(e, "hi");
    }
}

fn notify(x: Notifier, m: String) { x.send(m); }

group callers = { A };
group texting = { Sms };

main locus App {
    params { a: A = A { }; e: Email = Email { }; s: Sms = Sms { }; }
    claims { no_sms: forbid reaches(callers, texting); }
}
fn main() { App { }; }
"#;

#[test]
fn the_witness_names_the_dispatch_not_a_direct_call() {
    let ds = diags(DISPATCH);
    let witness = ds
        .iter()
        .find(|m| m.contains("claim `no_sms` violated"))
        .unwrap_or_else(|| panic!("expected a violation: {:#?}", ds));
    assert!(
        witness.contains("-(dispatches Notifier.send)-> `Sms::send`"),
        "the interface hop must be rendered AS a dispatch — shown as \
         a plain `->` it reads as impossible, because the call site \
         constructs an `Email`:\n{}",
        witness
    );
}

/// And the "where to edit" diagnostic has to explain the fanout,
/// since that is the part the reader disbelieves.
#[test]
fn the_dispatch_site_explains_why_every_conformer_counts() {
    let ds = diags(DISPATCH);
    let site = ds
        .iter()
        .find(|m| m.contains("crossed by this dispatch"))
        .unwrap_or_else(|| panic!("expected a dispatch site: {:#?}", ds));
    assert!(
        site.contains("`Notifier`") && site.contains("EVERY"),
        "it must name the interface and say the call reaches every \
         conformer:\n{}",
        site
    );
    assert!(
        site.contains("Narrow the receiver") || site.contains("exclude"),
        "and name a repair:\n{}",
        site
    );
}

/// A direct call must be unaffected — no dispatch vocabulary leaks
/// into a witness that has no interface in it.
#[test]
fn a_direct_call_witness_is_unchanged() {
    const DIRECT: &str = r#"
locus Sink { params { n: Int = 0; } fn take() { self.n = 1; } }
locus Src { params { n: Int = 0; } fn go() { reach(); } }
fn reach() { let s = Sink { }; s.take(); }
group srcs = { Src };
group sinks = { Sink };
main locus App {
    params { s: Src = Src { }; k: Sink = Sink { }; }
    claims { iso: forbid reaches(srcs, sinks); }
}
fn main() { App { }; }
"#;
    let ds = diags(DIRECT);
    let witness = ds
        .iter()
        .find(|m| m.contains("claim `iso` violated"))
        .unwrap_or_else(|| panic!("expected a violation: {:#?}", ds));
    assert!(
        !witness.contains("dispatches"),
        "no interface is involved here:\n{}",
        witness
    );
    assert!(witness.contains(" -> "), "plain call arrow:\n{}", witness);
}

// =====================================================================
// Provenance for `only edges` and `bound` (downstream review, item 4)
// =====================================================================
//
// `forbid` says WHERE to edit: the crossing call, or the publish and
// the receiving subscription. `only edges` and `bound` anchored only
// at the claim line — and `only edges` is explicitly a reviewable
// boundary inventory, so making the reviewer hand-find the crossing
// defeats the point of it.

const BOUNDARY: &str = r#"
type Cmd { v: Int; }
topic Allowed { payload: Cmd; subject: "app.allowed"; }
topic Sneaky  { payload: Cmd; subject: "app.sneaky"; }

locus Ops {
    params { n: Int = 0; }
    bus { publish Allowed; publish Sneaky; }
    fn act() {
        let c = Cmd { v: 1 };
        Allowed <- c;
        Sneaky <- c;
        let k = Core { };
        k.poked();
    }
}
locus Core {
    params { n: Int = 0; }
    bus { subscribe Allowed as on_allowed; subscribe Sneaky as on_sneaky; }
    fn on_allowed(c: Cmd) { self.n = c.v; }
    fn on_sneaky(c: Cmd) { self.n = c.v; }
    fn poked() { self.n = 99; }
}
group ops = { Ops };
group core = { Core };
main locus App {
    params { o: Ops = Ops { }; c: Core = Core { }; }
    claims { boundary: only edges ops -> core { publish Allowed; }; }
}
fn main() { App { }; }
"#;

#[test]
fn only_edges_points_at_the_ungranted_publish_and_subscription() {
    let ds = diags(BOUNDARY);
    assert!(
        ds.iter().any(|m| m.contains("the un-granted publish happens here")),
        "the publish site must be named: {:#?}",
        ds
    );
    let recv = ds
        .iter()
        .find(|m| m.contains("received here"))
        .unwrap_or_else(|| panic!("the subscription must be named: {:#?}", ds));
    assert!(
        recv.contains("publish Sneaky;"),
        "and the repair must be the exact grant line to add: {}",
        recv
    );
}

#[test]
fn only_edges_points_at_the_ungrantable_call() {
    let ds = diags(BOUNDARY);
    let call = ds
        .iter()
        .find(|m| m.contains("this call crosses the boundary"))
        .unwrap_or_else(|| panic!("the call site must be named: {:#?}", ds));
    assert!(
        call.contains("cannot be granted"),
        "and say why a grant is not the fix here: {}",
        call
    );
}

/// `bound` knew which of four conditions made a count unbounded and
/// printed all four, at the claim line, nowhere near the construct.
fn bound_src(body: &str, extra: &str) -> String {
    format!(
        r#"
effect llm;
@effects(is: {{llm}})
fn model_call(p: Int) -> Int {{ return p; }}
{extra}
locus Planner {{
    params {{ n: Int = 0; }}
    fn go() {{ {body} }}
}}
group planners = {{ Planner }};
main locus App {{
    params {{ p: Planner = Planner {{ }}; }}
    claims {{ one: bound llm <= 1 on paths from planners; }}
}}
fn main() {{ App {{ }}; }}
"#
    )
}

#[test]
fn an_unbounded_bound_names_the_loop_and_points_at_the_carrier() {
    let src = bound_src(
        "let mut i = 0; while i < 10 { self.n = model_call(i); i = i + 1; }",
        "",
    );
    let ds = diags(&src);
    let primary = ds
        .iter()
        .find(|m| m.contains("claim `one` violated"))
        .unwrap_or_else(|| panic!("expected a violation: {:#?}", ds));
    assert!(
        primary.contains("inside a loop") && primary.contains("Planner::go"),
        "the primary must name THIS condition, not list four: {}",
        primary
    );
    assert!(
        !primary.contains("recursion cycle, loop-nested carrier"),
        "the four-way disjunction must be gone: {}",
        primary
    );
    assert!(
        ds.iter().any(|m| m.contains("this is the loop-nested carrier")),
        "and point at the carrier site: {:#?}",
        ds
    );
}

#[test]
fn an_unbounded_bound_names_recursion() {
    let src = bound_src(
        "self.n = recur(3);",
        "fn recur(n: Int) -> Int { if n <= 0 { return model_call(n); } return recur(n - 1); }",
    );
    let primary = diags(&src)
        .into_iter()
        .find(|m| m.contains("claim `one` violated"))
        .expect("expected a violation");
    assert!(
        primary.contains("reachable from itself")
            && primary.contains("`recur`"),
        "recursion must be named, with the fn: {}",
        primary
    );
}

#[test]
fn an_unbounded_bound_points_at_an_unfollowable_call() {
    let src = bound_src(
        "self.n = thru(model_call, 1);",
        "fn thru(f: fn(Int) -> Int, n: Int) -> Int { return f(n); }",
    );
    let ds = diags(&src);
    let primary = ds
        .iter()
        .find(|m| m.contains("claim `one` violated"))
        .unwrap_or_else(|| panic!("expected a violation: {:#?}", ds));
    assert!(
        primary.contains("cannot follow"),
        "the unfollowable call must be named: {}",
        primary
    );
    assert!(
        ds.iter().any(|m| m.contains("this is the call the walk cannot follow")),
        "and pointed at: {:#?}",
        ds
    );
}
