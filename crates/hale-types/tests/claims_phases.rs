//! GH #382 phases 2–5 — the claim verbs beyond `forbid reaches`.
//!
//! Discipline (soundness law 4): every judgment form ships with a
//! canary whose claim MUST fail, and a control that certifies. A
//! checker that cannot fail proves nothing; a checker that cannot
//! pass proves the wrong thing.

use hale_syntax::parse_source;

fn diags(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

// =====================================================================
// Phase 2 — `only edges` grant enumeration
// =====================================================================

/// Two wings and one crossing topic; the grant list decides.
fn grant_fixture(claims: &str) -> String {
    format!(
        r#"
        type Metric {{ n: Int; }}
        topic Metrics {{ payload: Metric; }}

        locus DeltaTriage {{
            params {{ seen: Int = 0; }}
            bus {{ publish Metrics; }}
            fn on_task(n: Int) {{
                self.seen = self.seen + 1;
                Metrics <- Metric {{ n: n }};
            }}
        }}
        locus GammaResearch {{
            params {{ total: Int = 0; }}
            bus {{ subscribe Metrics as on_metric; }}
            fn on_metric(m: Metric) {{ self.total = self.total + m.n; }}
        }}
        group delta_wing = {{ DeltaTriage }};
        group gamma_wing = {{ GammaResearch }};
        main locus Org {{
            params {{
                t: DeltaTriage = DeltaTriage {{ }};
                r: GammaResearch = GammaResearch {{ }};
            }}
            claims {{
                {claims}
            }}
        }}
        fn main() {{ Org {{ }}; }}
    "#
    )
}

/// CANARY: an un-granted bus edge is a violation naming the edge and
/// the granted list.
#[test]
fn an_ungranted_bus_edge_is_a_violation() {
    let ds = diags(&grant_fixture(
        "gate: only edges delta_wing -> gamma_wing { };",
    ));
    let hit = ds
        .iter()
        .find(|m| m.contains("claim `gate` violated"))
        .unwrap_or_else(|| {
            panic!("the un-granted edge must violate: {:?}", ds)
        });
    assert!(
        hit.contains("un-granted edge")
            && hit.contains("Metrics")
            && hit.contains("GammaResearch"),
        "the violation must name the edge: {}",
        hit
    );
}

/// The control: granting the edge (publish spelling) certifies.
#[test]
fn a_granted_edge_certifies() {
    let ds = diags(&grant_fixture(
        "gate: only edges delta_wing -> gamma_wing { publish Metrics; };",
    ));
    assert!(
        !ds.iter().any(|m| m.contains("violated")),
        "the granted edge must pass: {:?}",
        ds
    );
}

/// `subscribe T` admits the same edge — the verb names which end's
/// declaration is the reviewable line.
#[test]
fn the_subscribe_spelling_grants_the_same_edge() {
    let ds = diags(&grant_fixture(
        "gate: only edges delta_wing -> gamma_wing { subscribe Metrics; };",
    ));
    assert!(
        !ds.iter().any(|m| m.contains("violated")),
        "the subscribe-spelled grant must pass: {:?}",
        ds
    );
}

/// A direct call across the boundary is never grantable.
#[test]
fn a_cross_boundary_call_is_always_ungranted() {
    let src = r#"
        locus B { fn work(n: Int) -> Int { return n * 2; } }
        locus A {
            params { b: B = B { }; }
            fn go(n: Int) -> Int { return self.b.work(n); }
        }
        group a_side = { A };
        group b_side = { B };
        main locus App {
            params { a: A = A { }; }
            claims { gate: only edges a_side -> b_side { }; }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("claim `gate` violated")
            && m.contains("call edges are not grantable")),
        "a boundary call must be un-granted: {:?}",
        ds
    );
}

/// A third-party subscriber OUTSIDE the target group is not an
/// A->B edge — the log-sink shape needs no grant.
#[test]
fn a_third_party_sink_is_not_a_boundary_edge() {
    let src = r#"
        type Metric { n: Int; }
        topic Metrics { payload: Metric; }
        locus A {
            bus { publish Metrics; }
            fn go(n: Int) { Metrics <- Metric { n: n }; }
        }
        locus Sink {
            params { total: Int = 0; }
            bus { subscribe Metrics as on_m; }
            fn on_m(m: Metric) { self.total = self.total + m.n; }
        }
        locus B { fn quiet() { } }
        group a_side = { A };
        group b_side = { B };
        main locus App {
            params { a: A = A { }; s: Sink = Sink { }; b: B = B { }; }
            claims { gate: only edges a_side -> b_side { }; }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("violated")),
        "a sink outside the target group is not an A->B edge: {:?}",
        ds
    );
}

/// An unknown topic in the grant list is an error with a hint.
#[test]
fn an_unknown_grant_topic_is_an_error() {
    let ds = diags(&grant_fixture(
        "gate: only edges delta_wing -> gamma_wing { publish Metrix; };",
    ));
    let hit = ds
        .iter()
        .find(|m| m.contains("names topic `Metrix`"))
        .unwrap_or_else(|| {
            panic!("an unknown grant topic must error: {:?}", ds)
        });
    assert!(
        hit.contains("Did you mean `Metrics`?"),
        "close names deserve a hint: {}",
        hit
    );
}

// =====================================================================
// Phase 3 — indexed effect families + `@budget(<user class> = N)`
// =====================================================================

const FAMILY: &str = r#"
    domain wing = { delta, gamma };
    effect knowledge(wing);

    @effects(is: {knowledge(delta)})
    fn read_delta(k: Int) -> Int { return k; }
    @effects(is: {knowledge(gamma)})
    fn read_gamma(k: Int) -> Int { return k; }
    fn calc(n: Int) -> Int { return n * 2; }
"#;

/// An instantiation behaves as an ordinary class: `none:` over it
/// fires on a reachable carrier.
#[test]
fn a_family_instantiation_propagates_like_a_class() {
    let ds = diags(&format!(
        "{FAMILY}@effects(none: {{knowledge(delta)}})\n\
         fn f(n: Int) -> Int {{ return read_delta(n); }}\n\
         fn main() {{ println(f(5)); }}"
    ));
    assert!(
        ds.iter().any(|m| m.contains("effect assertion violated")),
        "reaching a delta carrier must violate: {:?}",
        ds
    );
}

/// The control: the OTHER index does not fire — instantiations are
/// distinct classes.
#[test]
fn distinct_indices_do_not_alias() {
    let ds = diags(&format!(
        "{FAMILY}@effects(none: {{knowledge(delta)}})\n\
         fn f(n: Int) -> Int {{ return read_gamma(n); }}\n\
         fn main() {{ println(f(5)); }}"
    ));
    assert!(
        !ds.iter().any(|m| m.contains("effect assertion violated")),
        "a gamma carrier must not violate a delta contract: {:?}",
        ds
    );
}

/// `knowledge(*)` is the auto-populated composed class: forbidding
/// it catches every index.
#[test]
fn the_star_class_covers_every_index() {
    let ds = diags(&format!(
        "{FAMILY}@effects(none: {{knowledge(*)}})\n\
         fn f(n: Int) -> Int {{ return read_gamma(n); }}\n\
         fn main() {{ println(f(5)); }}"
    ));
    assert!(
        ds.iter().any(|m| m.contains("effect assertion violated")),
        "the star class must cover gamma: {:?}",
        ds
    );
}

/// A misspelt index interns a name nothing declared — the existing
/// undeclared-class error fires, with a did-you-mean.
#[test]
fn a_misspelt_index_is_an_undeclared_class_error() {
    let ds = diags(&format!(
        "{FAMILY}@effects(none: {{knowledge(delt)}})\n\
         fn f(n: Int) -> Int {{ return calc(n); }}\n\
         fn main() {{ println(f(5)); }}"
    ));
    let hit = ds
        .iter()
        .find(|m| m.contains("knowledge(delt)"))
        .unwrap_or_else(|| {
            panic!("a misspelt index must be undeclared: {:?}", ds)
        });
    assert!(
        hit.contains("Did you mean `knowledge(delta)`?"),
        "close instantiations deserve a hint: {}",
        hit
    );
}

/// A family over a domain not declared in the same file is a parse
/// error — index domains are closed and source-declared.
#[test]
fn a_family_without_its_domain_is_a_parse_error() {
    let errs = parse_source(
        "effect knowledge(wing);\nfn main() { }",
    )
    .expect_err("must reject");
    assert!(
        errs.iter().any(|e| e.message.contains("not declared in this file")),
        "got: {:?}",
        errs
    );
}

/// The data-plane claim: `effects(knowledge(delta))` as a reaches
/// target.
#[test]
fn a_family_instantiation_works_as_a_claim_sink() {
    let src = format!(
        "{FAMILY}\
         locus Quote {{ fn handle(n: Int) -> Int {{ return read_delta(n); }} }}\n\
         group quote_api = {{ Quote }};\n\
         main locus App {{\n\
             params {{ q: Quote = Quote {{ }}; }}\n\
             claims {{ iso: forbid reaches(quote_api, effects(knowledge(delta))); }}\n\
         }}\n\
         fn main() {{ App {{ }}; }}"
    );
    let ds = diags(&src);
    assert!(
        ds.iter().any(|m| m.contains("claim `iso` violated")),
        "reaching the delta store must violate: {:?}",
        ds
    );
}

/// `@budget(<user class> = N)`: the canary (two carrier calls,
/// limit one) and the control (limit two).
#[test]
fn a_user_class_budget_counts_carrier_calls() {
    let base = "effect llm;\n\
        @effects(is: {llm})\n\
        fn model_call(p: Int) -> Int { return p; }\n\
        @budget(llm = LIMIT)\n\
        fn plan(n: Int) -> Int { return model_call(n) + model_call(n); }\n\
        fn main() { println(plan(5)); }";
    let ds = diags(&base.replace("LIMIT", "1"));
    let hit = ds
        .iter()
        .find(|m| m.contains("budget exceeded"))
        .unwrap_or_else(|| {
            panic!("two carrier calls must exceed a limit of one: {:?}", ds)
        });
    assert!(
        hit.contains("llm") && hit.contains("measures 2"),
        "the diagnostic must name the class and the count: {}",
        hit
    );
    let ds = diags(&base.replace("LIMIT", "2"));
    assert!(
        !ds.iter().any(|m| m.contains("budget exceeded")),
        "two carrier calls within a limit of two must pass: {:?}",
        ds
    );
}

/// A carrier call inside a loop is unbounded per call.
#[test]
fn a_looped_carrier_call_is_unbounded() {
    let src = "effect llm;\n\
        @effects(is: {llm})\n\
        fn model_call(p: Int) -> Int { return p; }\n\
        @budget(llm = 5)\n\
        fn plan(n: Int) -> Int {\n\
            let mut acc = 0;\n\
            for i in 0..n { acc = acc + model_call(i); }\n\
            return acc;\n\
        }\n\
        fn main() { println(plan(5)); }";
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("unbounded")),
        "a loop-nested carrier must be unbounded: {:?}",
        ds
    );
}

/// An undeclared class as a budget key is an error with a hint.
#[test]
fn an_undeclared_budget_class_is_an_error() {
    let src = "effect llm;\n\
        @budget(lln = 1)\n\
        fn plan(n: Int) -> Int { return n; }\n\
        fn main() { println(plan(5)); }";
    let ds = diags(src);
    let hit = ds
        .iter()
        .find(|m| m.contains("budgets effect class `lln`"))
        .unwrap_or_else(|| {
            panic!("an undeclared budget class must error: {:?}", ds)
        });
    assert!(
        hit.contains("Did you mean `llm`?"),
        "close names deserve a hint: {}",
        hit
    );
}

// =====================================================================
// Phase 4 — `bound` claims
// =====================================================================

fn bound_fixture(body: &str, claims: &str) -> String {
    format!(
        r#"
        effect llm;
        @effects(is: {{llm}})
        fn model_call(p: Int) -> Int {{ return p; }}
        locus Planner {{
            fn plan(n: Int) -> Int {{ {body} }}
        }}
        group planners = {{ Planner }};
        main locus App {{
            params {{ p: Planner = Planner {{ }}; }}
            claims {{ {claims} }}
        }}
        fn main() {{ App {{ }}; }}
    "#
    )
}

/// CANARY: two sequential carrier calls exceed a bound of one, and
/// the witness names the heaviest path.
#[test]
fn a_second_carrier_site_on_the_path_violates_the_bound() {
    let ds = diags(&bound_fixture(
        "return model_call(n) + model_call(n);",
        "one_call: bound llm <= 1 on paths from planners;",
    ));
    let hit = ds
        .iter()
        .find(|m| m.contains("claim `one_call` violated"))
        .unwrap_or_else(|| {
            panic!("two carrier sites must exceed a bound of one: {:?}", ds)
        });
    assert!(
        hit.contains("carries 2") && hit.contains("Planner::plan"),
        "the witness must carry the count and the path: {}",
        hit
    );
}

/// The control: one carrier call within the bound certifies.
#[test]
fn a_single_carrier_site_within_the_bound_certifies() {
    let ds = diags(&bound_fixture(
        "return model_call(n);",
        "one_call: bound llm <= 1 on paths from planners;",
    ));
    assert!(
        !ds.iter().any(|m| m.contains("violated")),
        "one carrier site within the bound must pass: {:?}",
        ds
    );
}

/// A carrier inside a loop is unbounded — no finite bound certifies.
#[test]
fn a_looped_carrier_is_unbounded_for_the_bound() {
    let ds = diags(&bound_fixture(
        "let mut acc = 0; for i in 0..n { acc = acc + model_call(i); } return acc;",
        "one_call: bound llm <= 100 on paths from planners;",
    ));
    assert!(
        ds.iter().any(|m| m.contains("claim `one_call` violated")
            && m.contains("unbounded")),
        "a loop-nested carrier must be unbounded: {:?}",
        ds
    );
}

/// `bound` takes user classes only; the counted built-ins keep their
/// `@budget` spellings. (`alloc` parses as an ident — the hard-
/// keyword built-ins like `publish` never even reach validation.)
#[test]
fn a_builtin_class_in_bound_is_an_error() {
    let ds = diags(&bound_fixture(
        "return n;",
        "p: bound alloc <= 1 on paths from planners;",
    ));
    assert!(
        ds.iter().any(|m| m.contains("user-declared effect class")),
        "a built-in in `bound` must error: {:?}",
        ds
    );
}

// =====================================================================
// Phase 5 — require / count / during / avoiding
// =====================================================================

fn wired_fixture(delta_bus: &str, claims: &str) -> String {
    format!(
        r#"
        type Task {{ id: Int; }}
        topic Tasks {{ payload: Task; }}
        locus DeltaTriage {{
            params {{ seen: Int = 0; }}
            {delta_bus}
        }}
        group delta_wing = {{ DeltaTriage }};
        main locus Org {{
            params {{ t: DeltaTriage = DeltaTriage {{ }}; }}
            claims {{ {claims} }}
        }}
        fn main() {{ Org {{ }}; }}
    "#
    )
}

/// `require subscribes` holds when a member subscribes…
#[test]
fn require_subscribes_holds_when_wired() {
    let ds = diags(&wired_fixture(
        "bus { subscribe Tasks as on_task; }\n\
         fn on_task(t: Task) { self.seen = self.seen + 1; }",
        "wired: require subscribes(some delta_wing, topic Tasks);",
    ));
    assert!(
        !ds.iter().any(|m| m.contains("violated")),
        "a wired subscription must satisfy require: {:?}",
        ds
    );
}

/// …and the CANARY: nothing subscribing is a violation.
#[test]
fn require_subscribes_fails_when_unwired() {
    let ds = diags(&wired_fixture(
        "fn idle(t: Task) { self.seen = self.seen + 1; }",
        "wired: require subscribes(some delta_wing, topic Tasks);",
    ));
    assert!(
        ds.iter().any(|m| m.contains("claim `wired` violated")
            && m.contains("no member of `delta_wing` subscribes")),
        "an unwired require must violate: {:?}",
        ds
    );
}

/// The publishes dual.
#[test]
fn require_publishes_both_ways() {
    let wired = diags(&wired_fixture(
        "bus { publish Tasks; }\n\
         fn go(n: Int) { Tasks <- Task { id: n }; }",
        "w: require publishes(some delta_wing, topic Tasks);",
    ));
    assert!(
        !wired.iter().any(|m| m.contains("violated")),
        "a declared publisher must satisfy require: {:?}",
        wired
    );
    let unwired = diags(&wired_fixture(
        "fn idle(t: Task) { self.seen = self.seen + 1; }",
        "w: require publishes(some delta_wing, topic Tasks);",
    ));
    assert!(
        unwired.iter().any(|m| m.contains("claim `w` violated")),
        "no publisher must violate: {:?}",
        unwired
    );
}

/// `count publishers == 1` — the single-writer invariant. CANARY:
/// two publishers violate, naming both.
#[test]
fn count_enforces_single_writer() {
    let two = r#"
        type Task { id: Int; }
        topic Tasks { payload: Task; }
        locus P1 {
            bus { publish Tasks; }
            fn go(n: Int) { Tasks <- Task { id: n }; }
        }
        locus P2 {
            bus { publish Tasks; }
            fn go(n: Int) { Tasks <- Task { id: n }; }
        }
        locus Sub {
            params { seen: Int = 0; }
            bus { subscribe Tasks as on_t; }
            fn on_t(t: Task) { self.seen = self.seen + 1; }
        }
        main locus Org {
            params { a: P1 = P1 { }; b: P2 = P2 { }; s: Sub = Sub { }; }
            claims { sw: count publishers(topic Tasks) == 1; }
        }
        fn main() { Org { }; }
    "#;
    let ds = diags(two);
    let hit = ds
        .iter()
        .find(|m| m.contains("claim `sw` violated"))
        .unwrap_or_else(|| {
            panic!("two publishers must violate ==1: {:?}", ds)
        });
    assert!(
        hit.contains("counted 2") && hit.contains("P1") && hit.contains("P2"),
        "the violation must name both writers: {}",
        hit
    );
    // The control: removing one publisher satisfies the count.
    let one = two
        .replace(
            "locus P2 {\n            bus { publish Tasks; }\n            fn go(n: Int) { Tasks <- Task { id: n }; }\n        }",
            "locus P2 { fn quiet() { } }",
        )
        .replace("b: P2 = P2 { };", "b: P2 = P2 { };");
    let ds = diags(&one);
    assert!(
        !ds.iter().any(|m| m.contains("claim `sw` violated")),
        "one publisher must satisfy ==1: {:?}",
        ds
    );
}

fn during_fixture(a_body: &str, claims: &str) -> String {
    format!(
        r#"
        type Metric {{ n: Int; }}
        topic Metrics {{ payload: Metric; }}
        locus A {{
            params {{ seen: Int = 0; }}
            bus {{ publish Metrics; }}
            {a_body}
        }}
        locus B {{
            params {{ total: Int = 0; }}
            bus {{ subscribe Metrics as on_m; }}
            fn on_m(m: Metric) {{ self.total = self.total + m.n; }}
        }}
        group a_side = {{ A }};
        group b_side = {{ B }};
        main locus Org {{
            params {{ a: A = A {{ }}; b: B = B {{ }}; }}
            claims {{ {claims} }}
        }}
        fn main() {{ Org {{ }}; }}
    "#
    )
}

/// `during birth` — CANARY: a birth-phase publish crossing the
/// boundary violates.
#[test]
fn during_birth_catches_a_birth_publish() {
    let ds = diags(&during_fixture(
        "birth() { Metrics <- Metric { n: 1 }; }\n\
         fn tick(n: Int) { self.seen = self.seen + n; }",
        "quiet_boot: forbid reaches(a_side, b_side) during birth;",
    ));
    assert!(
        ds.iter().any(|m| m.contains("claim `quiet_boot` violated")),
        "a birth publish must violate the birth-phase claim: {:?}",
        ds
    );
}

/// The control: the same edge from an ordinary method does not touch
/// the birth-phase claim.
#[test]
fn during_birth_ignores_a_run_phase_edge() {
    let ds = diags(&during_fixture(
        "birth() { self.seen = 0; }\n\
         fn tick(n: Int) { Metrics <- Metric { n: n }; }",
        "quiet_boot: forbid reaches(a_side, b_side) during birth;",
    ));
    assert!(
        !ds.iter().any(|m| m.contains("violated")),
        "a run-phase edge must not violate the birth-phase claim: {:?}",
        ds
    );
}

/// A phase that names nothing in the group is an error, not a
/// vacuously-holding claim.
#[test]
fn a_phase_naming_nothing_is_an_error() {
    let ds = diags(&during_fixture(
        "fn tick(n: Int) { self.seen = self.seen + n; }",
        "q: forbid reaches(a_side, b_side) during drain;",
    ));
    assert!(
        ds.iter().any(|m| m.contains("names nothing in group")),
        "an empty phase filter must error: {:?}",
        ds
    );
}

/// `avoiding` — the interposition form. The gate carries every path:
/// the claim holds. A bypass edge appears: violated.
#[test]
fn avoiding_proves_interposition() {
    let gated = r#"
        locus B { fn work(n: Int) -> Int { return n * 2; } }
        locus Gate {
            params { b: B = B { }; }
            fn check(n: Int) -> Int { return self.b.work(n); }
        }
        locus A {
            params { g: Gate = Gate { }; }
            fn go(n: Int) -> Int { return self.g.check(n); }
        }
        group a_side = { A };
        group b_side = { B };
        group gate = { Gate };
        main locus App {
            params { a: A = A { }; }
            claims { gated: forbid reaches(a_side, b_side) avoiding gate; }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(gated);
    assert!(
        !ds.iter().any(|m| m.contains("violated")),
        "every path passes the gate — the claim must hold: {:?}",
        ds
    );
    // CANARY: a direct bypass edge dodges the gate.
    let bypass = gated.replace(
        "params { g: Gate = Gate { }; }\n            fn go(n: Int) -> Int { return self.g.check(n); }",
        "params { g: Gate = Gate { }; b: B = B { }; }\n            fn go(n: Int) -> Int { return self.b.work(n); }",
    );
    let ds = diags(&bypass);
    assert!(
        ds.iter().any(|m| m.contains("claim `gated` violated")),
        "a bypass edge must violate the interposition claim: {:?}",
        ds
    );
}
