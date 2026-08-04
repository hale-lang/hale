//! GH #382 phase 1 — claims: named bundle-level sentences over the
//! program graph.
//!
//! The motivating shape is multi-tenant isolation: "no path from
//! domain A to domain B" as ONE declaration with a name a contract
//! can cite, instead of per-fn `@effects` contracts scattered across
//! every position with completeness by hope.
//!
//! Discipline (soundness law 4): every judgment form ships with a
//! canary whose claim MUST fail — a checker that cannot fail proves
//! nothing.

use hale_syntax::parse_source;

fn diags(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

/// Two wings, isolated except through `CLAIMS`-controlled law. The
/// `delta` wing handles tasks; the `gamma` wing does research. A
/// shared metrics topic is the boundary-crossing temptation.
const TWO_WINGS: &str = r#"
    type Task { id: Int; }
    type Metric { n: Int; }
    topic Tasks   { payload: Task; }
    topic Metrics { payload: Metric; }

    locus DeltaTriage {
        bus { subscribe Tasks as on_task; PUBLISH }
        params { seen: Int = 0; }
        fn on_task(t: Task) {
            self.seen = self.seen + 1;
            BODY
        }
    }

    locus GammaResearch {
        bus { subscribe Metrics as on_metric; }
        params { total: Int = 0; }
        fn on_metric(m: Metric) { self.total = self.total + m.n; }
    }

    group delta_wing = { DeltaTriage };
    group gamma_wing = { GammaResearch };

    locus Gateway {
        bus { publish Tasks; }
        fn intake(id: Int) { Tasks <- Task { id: id }; }
    }

    main locus Org {
        params {
            gw: Gateway = Gateway { };
            triage: DeltaTriage = DeltaTriage { };
            research: GammaResearch = GammaResearch { };
        }
        claims {
            CLAIMS
        }
    }
    fn main() { Org { }; }
"#;

fn two_wings(publish: &str, body: &str, claims: &str) -> String {
    TWO_WINGS
        .replace("PUBLISH", publish)
        .replace("BODY", body)
        .replace("CLAIMS", claims)
}

// ===================== the canary (negative control) =============

/// A bus path from delta to gamma violates the isolation claim, and
/// the witness names the path. This is the negative control: the
/// claim MUST fail here or the checker proves nothing.
#[test]
fn a_bus_path_across_the_boundary_is_a_violation() {
    let ds = diags(&two_wings(
        "publish Metrics;",
        "Metrics <- Metric { n: 1 };",
        "iso_dg: forbid reaches(delta_wing, gamma_wing);",
    ));
    let hit = ds
        .iter()
        .find(|m| m.contains("claim `iso_dg` violated"))
        .unwrap_or_else(|| {
            panic!("the boundary-crossing publish must violate: {:?}", ds)
        });
    assert!(
        hit.contains("DeltaTriage::on_task")
            && hit.contains("Metrics")
            && hit.contains("GammaResearch::on_metric"),
        "the witness must name the full path: {}",
        hit
    );
}

/// The control for the control: with no cross-wing edge, the same
/// claim certifies.
#[test]
fn isolated_wings_certify() {
    let ds = diags(&two_wings(
        "",
        "",
        "iso_dg: forbid reaches(delta_wing, gamma_wing);",
    ));
    assert!(
        !ds.iter().any(|m| m.contains("violated")),
        "no cross-wing edge exists, the claim must hold: {:?}",
        ds
    );
}

/// A call path (no bus) also violates — `reaches` composes both
/// relations by default.
#[test]
fn a_call_path_across_the_boundary_is_a_violation() {
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
            claims { iso: forbid reaches(a_side, b_side); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    let hit = ds
        .iter()
        .find(|m| m.contains("claim `iso` violated"))
        .unwrap_or_else(|| {
            panic!("a handle-method call must violate: {:?}", ds)
        });
    assert!(
        hit.contains("A::go") && hit.contains("B::work"),
        "the witness must name both ends: {}",
        hit
    );
}

// ===================== via restriction ===========================

/// `via { calls }` excludes bus edges: the bus path that violates
/// the default claim does not violate the calls-only claim.
#[test]
fn via_calls_ignores_a_bus_path() {
    let ds = diags(&two_wings(
        "publish Metrics;",
        "Metrics <- Metric { n: 1 };",
        "iso_calls: forbid reaches(delta_wing, gamma_wing) via { calls };",
    ));
    assert!(
        !ds.iter().any(|m| m.contains("violated")),
        "a bus-only path must not violate a calls-only claim: {:?}",
        ds
    );
}

// ===================== effects(...) sink =========================

/// `forbid reaches(G, effects(C))` — the data-plane form: a group
/// must not reach any declared carrier of a user class.
#[test]
fn reaching_a_user_class_carrier_violates_an_effects_sink() {
    let src = r#"
        effect money;
        @effects(is: {money})
        fn charge(cents: Int) -> Int { return cents; }
        fn quote_helper(n: Int) -> Int { return charge(n); }
        locus Quote { fn handle(n: Int) -> Int { return quote_helper(n); } }
        group quote_api = { Quote };
        main locus App {
            params { q: Quote = Quote { }; }
            claims { read_only: forbid reaches(quote_api, effects(money)); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    let hit = ds
        .iter()
        .find(|m| m.contains("claim `read_only` violated"))
        .unwrap_or_else(|| {
            panic!("reaching the money carrier must violate: {:?}", ds)
        });
    assert!(
        hit.contains("charge"),
        "the witness must end at the carrier: {}",
        hit
    );
}

/// The control: a group that never reaches the carrier certifies.
#[test]
fn avoiding_the_carrier_certifies_the_effects_sink() {
    let src = r#"
        effect money;
        @effects(is: {money})
        fn charge(cents: Int) -> Int { return cents; }
        fn calc(n: Int) -> Int { return n * 2; }
        locus Quote { fn handle(n: Int) -> Int { return calc(n); } }
        group quote_api = { Quote };
        main locus App {
            params { q: Quote = Quote { }; }
            claims { read_only: forbid reaches(quote_api, effects(money)); }
        }
        fn main() { App { }; charge(1); }
    "#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("violated")),
        "a path avoiding the carrier must certify: {:?}",
        ds
    );
}

// ===================== vocabulary guards =========================

/// Unknown group member = error, never an empty set (the misspelt-
/// effect-class lesson at the group layer), with a did-you-mean.
#[test]
fn an_unknown_group_member_is_an_error() {
    let src = r#"
        locus DeltaTriage { fn go() { } }
        group delta_wing = { DeltaTriag };
        main locus App {
            params { d: DeltaTriage = DeltaTriage { }; }
            claims { }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    let hit = ds
        .iter()
        .find(|m| m.contains("names no declared locus or fn"))
        .unwrap_or_else(|| {
            panic!("an unknown member must be an error: {:?}", ds)
        });
    assert!(
        hit.contains("Did you mean `DeltaTriage`?"),
        "close names deserve a hint: {}",
        hit
    );
}

/// A qualified member that matches no import is an error — in a
/// single-seed bundle every `a::b` member is one.
#[test]
fn an_unresolved_qualified_member_is_an_error() {
    let src = r#"
        locus A { fn go() { } }
        group g = { nosuch::Thing };
        main locus App {
            params { a: A = A { }; }
            claims { }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("does not resolve")),
        "a qualified member with no matching import must error: {:?}",
        ds
    );
}

/// An empty group is a vacuity error unless it opts out.
#[test]
fn an_empty_group_is_a_vacuity_error() {
    let src = r#"
        group probes = { };
        main locus App { claims { } }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("holds vacuously")
            || m.contains("resolves to no declarations")),
        "an empty group without may_be_empty must error: {:?}",
        ds
    );
}

/// `may_be_empty` is the explicit opt-out.
#[test]
fn may_be_empty_silences_the_vacuity_guard() {
    let src = r#"
        group probes = { } may_be_empty;
        main locus App { claims { } }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("resolves to no declarations")),
        "may_be_empty must silence the vacuity error: {:?}",
        ds
    );
}

/// A claim naming an undeclared group is an error with a hint.
#[test]
fn an_unknown_group_in_a_claim_is_an_error() {
    let src = r#"
        locus A { fn go() { } }
        group a_side = { A };
        main locus App {
            params { a: A = A { }; }
            claims { iso: forbid reaches(a_side, b_side); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("names group `b_side`")
            && m.contains("never declared")),
        "an undeclared group must be an error: {:?}",
        ds
    );
}

/// An undeclared effect class in `effects(...)` is an error — the
/// `only:`-list lesson, not repeated here.
#[test]
fn an_undeclared_effect_class_in_a_claim_is_an_error() {
    let src = r#"
        effect money;
        locus A { fn go() { } }
        group a_side = { A };
        main locus App {
            params { a: A = A { }; }
            claims { m: forbid reaches(a_side, effects(monye)); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    let hit = ds
        .iter()
        .find(|m| m.contains("names effect class `monye`"))
        .unwrap_or_else(|| {
            panic!("an undeclared class must be an error: {:?}", ds)
        });
    assert!(
        hit.contains("Did you mean `money`?"),
        "close names deserve a hint: {}",
        hit
    );
}

/// Duplicate claim names are an error — the name is the
/// contract-of-record.
#[test]
fn duplicate_claim_names_are_an_error() {
    let src = r#"
        locus A { fn go() { } }
        group a_side = { A };
        group b_side = { A };
        main locus App {
            params { a: A = A { }; }
            claims {
                iso: forbid reaches(a_side, b_side) via { bus };
                iso: forbid reaches(b_side, a_side) via { bus };
            }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("declared more than once")),
        "duplicate claim names must error: {:?}",
        ds
    );
}

// ===================== surface placement =========================

/// `claims { }` outside `main locus` is a parse error: main is the
/// closed-world gate, so bundle-wide claims cannot be evaluated
/// anywhere earlier.
#[test]
fn claims_outside_main_is_a_parse_error() {
    let src = r#"
        locus NotMain {
            claims { }
        }
        fn main() { }
    "#;
    let errs = parse_source(src).expect_err("must reject");
    assert!(
        errs.iter()
            .any(|e| e.message.contains("only valid inside `main locus`")),
        "got: {:?}",
        errs
    );
}

/// The glob is trailing-only, mirroring the `**` subject rule.
#[test]
fn an_infix_glob_is_a_parse_error() {
    let src = r#"
        group g = { a::*::b };
        fn main() { }
    "#;
    let errs = parse_source(src).expect_err("must reject");
    assert!(
        errs.iter().any(|e| e.message.contains("trailing-only")),
        "got: {:?}",
        errs
    );
}

// ===================== fail-closed unknowns ======================

/// An indirect call (fn-typed param) on a path from a forbid source
/// cannot be certified — unknown ⇒ violation, exactly as
/// `@no_syscall` treats the same shape (#353).
#[test]
fn an_indirect_call_on_the_path_fails_closed() {
    let src = r#"
        locus A { fn go(f: fn(Int) -> Int, n: Int) -> Int { return f(n); } }
        locus B { fn work(n: Int) -> Int { return n; } }
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
        ds.iter().any(|m| m.contains("cannot be certified")
            && m.contains("function-typed parameter")),
        "an indirect call must fail closed: {:?}",
        ds
    );
}
