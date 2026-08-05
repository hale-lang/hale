//! GH #409: constitutions — one claimset shared across entrypoints.
//!
//! `claims { }` is only legal inside `main locus`, and for a good
//! reason: claims are closed-world statements and `main` is the only
//! place a world IS closed. But that constrains *evaluation*, not
//! *authoring* — so a law that should hold for twenty entrypoints had
//! to be copy-pasted into twenty main loci, where the copy somebody
//! forgets fails open silently.
//!
//! A constitution is authored once, outside any main; each entrypoint
//! adopts it; and every clause is still evaluated in that
//! entrypoint's own closed world. One text, N evaluations, N worlds —
//! the soundness argument is unchanged.

use hale_syntax::parse_source;

fn diags(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse failed");
    hale_types::check_program(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message)
        .collect()
}

/// Shared scaffolding: two domains and a topic to quantify over.
const WORLD: &str = r#"
type Msg { v: Int; }
topic Settled { payload: Msg; subject: "app.settled"; }
locus Billing {
    params { n: Int = 0; }
    bus { publish Settled; }
    fn go() { let m = Msg { v: 1 }; Settled <- m; }
}
locus Research { params { n: Int = 0; } fn look() -> Int { return self.n; } }
group billing = { Billing };
group research = { Research };
"#;

fn program(body: &str) -> String {
    format!("{WORLD}\n{body}\nfn main() {{ App {{ }}; }}")
}

#[test]
fn an_adopted_clause_is_evaluated_in_the_adopting_main() {
    let src = program(
        r#"
constitution Core {
    tenant_iso: forbid reaches(billing, research);
}
main locus App {
    params { b: Billing = Billing { }; r: Research = Research { }; }
    claims { adopt Core; }
}
"#,
    );
    assert!(
        diags(&src).is_empty(),
        "the clause holds in this world: {:#?}",
        diags(&src)
    );
}

/// `extends` is transitive: adopting the derived one brings the base.
#[test]
fn extends_pulls_in_the_base_clauses() {
    let src = program(
        r#"
constitution Core { tenant_iso: forbid reaches(billing, research); }
constitution Dev extends Core { quiet: count subscribers(topic Settled) == 0; }
main locus App {
    params { b: Billing = Billing { }; r: Research = Research { }; }
    claims { adopt Dev; }
}
"#,
    );
    assert!(diags(&src).is_empty(), "{:#?}", diags(&src));

    // …and a base clause that FAILS here must fail through adoption,
    // otherwise adoption would be decorative.
    let broken = src.replace(
        "tenant_iso: forbid reaches(billing, research);",
        "impossible: count publishers(topic Settled) == 7;",
    );
    let ds = diags(&broken);
    assert!(
        ds.iter().any(|m| m.contains("claim `impossible` violated")),
        "an adopted clause must gate the build: {:#?}",
        ds
    );
}

/// The rule the whole design rests on. Two constitutions declaring
/// one name is an error, which is what makes weakening
/// *unexpressible* rather than merely discouraged: a derived
/// constitution cannot replace an inherited clause, so a stricter
/// variant has to be a separate named claim that coexists with it.
#[test]
fn a_derived_constitution_cannot_override_an_inherited_clause() {
    let src = program(
        r#"
constitution A { rule: count publishers(topic Settled) == 1; }
constitution B extends A { rule: count publishers(topic Settled) == 9; }
main locus App {
    params { b: Billing = Billing { }; r: Research = Research { }; }
    claims { adopt B; }
}
"#,
    );
    let ds = diags(&src);
    let hit = ds
        .iter()
        .find(|m| m.contains("declared by two constitutions"))
        .unwrap_or_else(|| panic!("expected a collision: {:#?}", ds));
    assert!(
        hit.contains("`A`") && hit.contains("`B`"),
        "both origins must be named: {}",
        hit
    );
}

/// Nor can a main quietly shadow an adopted clause with a local one.
#[test]
fn a_local_claim_cannot_shadow_an_adopted_one() {
    let src = program(
        r#"
constitution A { rule: count publishers(topic Settled) == 1; }
main locus App {
    params { b: Billing = Billing { }; r: Research = Research { }; }
    claims { adopt A; rule: count publishers(topic Settled) == 9; }
}
"#,
    );
    let ds = diags(&src);
    assert!(
        ds.iter().any(|m| m.contains("cannot replace an adopted one")),
        "expected the shadowing error: {:#?}",
        ds
    );
}

/// A diamond contributes the shared base exactly once. Dedup is by
/// ORIGIN, not by claim name — deduping by name would swallow the
/// genuine two-origin collision the test above relies on.
#[test]
fn a_diamond_contributes_the_shared_base_once() {
    let src = program(
        r#"
constitution Base { shared: count publishers(topic Settled) == 1; }
constitution L extends Base { l: count subscribers(topic Settled) == 0; }
constitution R extends Base { r: forbid reaches(billing, research); }
constitution Both extends L, R { }
main locus App {
    params { b: Billing = Billing { }; r: Research = Research { }; }
    claims { adopt Both; }
}
"#,
    );
    assert!(
        diags(&src).is_empty(),
        "`Base` arriving twice must not read as a collision: {:#?}",
        diags(&src)
    );
}

#[test]
fn a_cycle_in_extends_is_an_error() {
    let src = program(
        r#"
constitution A extends B { one: count publishers(topic Settled) == 1; }
constitution B extends A { two: count subscribers(topic Settled) == 0; }
main locus App {
    params { b: Billing = Billing { }; r: Research = Research { }; }
    claims { adopt A; }
}
"#,
    );
    assert!(
        diags(&src).iter().any(|m| m.contains("extends itself")),
        "{:#?}",
        diags(&src)
    );
}

#[test]
fn an_unknown_constitution_is_an_error() {
    let src = program(
        r#"
constitution Core { one: count publishers(topic Settled) == 1; }
main locus App {
    params { b: Billing = Billing { }; r: Research = Research { }; }
    claims { adopt Coer; }
}
"#,
    );
    let ds = diags(&src);
    let hit = ds
        .iter()
        .find(|m| m.contains("unknown constitution"))
        .unwrap_or_else(|| panic!("{:#?}", ds));
    assert!(
        hit.contains("Did you mean `Core`?"),
        "a near miss should be named: {}",
        hit
    );
}

#[test]
fn two_constitutions_may_not_share_a_name() {
    let src = program(
        r#"
constitution Core { one: count publishers(topic Settled) == 1; }
constitution Core { two: count subscribers(topic Settled) == 0; }
main locus App {
    params { b: Billing = Billing { }; r: Research = Research { }; }
    claims { adopt Core; }
}
"#,
    );
    assert!(
        diags(&src).iter().any(|m| m.contains("declared twice")),
        "{:#?}",
        diags(&src)
    );
}

/// Adoption is the closing world's act: it is what fixes which world
/// the clauses are evaluated against. A library seed declares a
/// constitution — that is how one is shared — but does not adopt.
#[test]
fn a_library_tier_block_may_not_adopt() {
    let src = format!(
        "{WORLD}\nconstitution A {{ one: count publishers(topic Settled) == 1; }}\nclaims {{ adopt A; }}\n"
    );
    let ds = diags(&src);
    assert!(
        ds.iter()
            .any(|m| m.contains("only legal in a `main locus`")),
        "{:#?}",
        ds
    );
}

/// `adopt` inside a constitution is a parse error — claimsets compose
/// with `extends`, and allowing both spellings would give two
/// different composition mechanisms with different rules.
#[test]
fn a_constitution_may_not_adopt() {
    let src = program(
        r#"
constitution A { one: count publishers(topic Settled) == 1; }
constitution B { adopt A; }
main locus App {
    params { b: Billing = Billing { }; r: Research = Research { }; }
    claims { adopt B; }
}
"#,
    );
    let err = parse_source(&src).expect_err("must not parse");
    assert!(
        err.iter().any(|d| d.message.contains("`extends`")),
        "the error should point at the right mechanism: {:?}",
        err
    );
}

// =====================================================================
// PR #415 review findings
// =====================================================================

/// Review finding 1. Diamond duplication is resolved by
/// constitution-level traversal, so a repeated (origin, claim name)
/// can only be two clauses declared under one name. The expansion
/// took a `Some(_) => {}` branch and dropped the second — the build
/// passed while a law the author wrote went unchecked, which is the
/// exact failure mode this feature exists to prevent.
#[test]
fn a_constitution_may_not_declare_one_claim_name_twice() {
    let src = program(
        r#"
constitution A {
    rule: count publishers(topic Settled) == 1;
    rule: count publishers(topic Settled) == 9;
}
main locus App {
    params { b: Billing = Billing { }; r: Research = Research { }; }
    claims { adopt A; }
}
"#,
    );
    let ds = diags(&src);
    let hit = ds
        .iter()
        .find(|m| m.contains("declares claim `rule` twice"))
        .unwrap_or_else(|| panic!("expected a duplicate error: {:#?}", ds));
    assert!(hit.contains("`A`"), "name the constitution: {}", hit);
}

/// The second clause would have FAILED, so keeping only the first was
/// not merely lossy — it changed the verdict.
#[test]
fn the_dropped_duplicate_would_have_changed_the_verdict() {
    let src = program(
        r#"
constitution A {
    rule: count publishers(topic Settled) == 1;
    rule: count publishers(topic Settled) == 9;
}
main locus App {
    params { b: Billing = Billing { }; r: Research = Research { }; }
    claims { adopt A; }
}
"#,
    );
    assert!(
        !diags(&src).is_empty(),
        "a program whose second clause cannot hold must not pass"
    );
}

/// Review finding 3: identity is the normalized closure, not the
/// display name. Two constitutions with the same NAME and different
/// clauses must not share a digest; the same closure reached two ways
/// must.
#[test]
fn constitution_identity_follows_the_closure_not_the_name() {
    use hale_types::bus_graph::build_bus_graph;
    use hale_types::claims::claims_report_with_identities;

    fn identities(src: &str) -> Vec<(String, String)> {
        let prog = parse_source(src).expect("parse");
        let mut programs = std::collections::BTreeMap::new();
        programs.insert("app.hl".to_string(), &prog);
        let bundle = hale_types::Bundle::new(programs);
        let (top, _) = hale_types::resolve::build_top_scope(&bundle);
        let graph = build_bus_graph(&bundle, &top);
        let progs: Vec<&hale_syntax::ast::Program> =
            bundle.programs.values().copied().collect();
        let (_, _, ids) =
            claims_report_with_identities(&progs, &graph, &[]);
        ids.into_iter().map(|i| (i.name, i.digest)).collect()
    }

    let base = program(
        r#"
constitution Core { r: count publishers(topic Settled) == 1; }
main locus App {
    params { b: Billing = Billing { }; r: Research = Research { }; }
    claims { adopt Core; }
}
"#,
    );
    // Same name, one extra clause: a different claimset entirely.
    let wider = base.replace(
        "constitution Core { r: count publishers(topic Settled) == 1; }",
        "constitution Core { r: count publishers(topic Settled) == 1; \
         s: count subscribers(topic Settled) == 0; }",
    );

    let a = identities(&base);
    let b = identities(&wider);
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].0, "Core");
    assert_eq!(b[0].0, "Core", "the display name is unchanged");
    assert_ne!(
        a[0].1, b[0].1,
        "…but two different claimsets must not share an identity, or a \
         deployment binding the bare name proves nothing"
    );

    // Identity is stable for the same closure.
    assert_eq!(identities(&base)[0].1, a[0].1);
}

/// An inherited clause changing must change the derived digest —
/// otherwise a base edit would slip past an identity comparison.
#[test]
fn a_changed_base_clause_changes_the_derived_digest() {
    use hale_types::bus_graph::build_bus_graph;
    use hale_types::claims::claims_report_with_identities;

    fn digest_of(src: &str, want: &str) -> String {
        let prog = parse_source(src).expect("parse");
        let mut programs = std::collections::BTreeMap::new();
        programs.insert("app.hl".to_string(), &prog);
        let bundle = hale_types::Bundle::new(programs);
        let (top, _) = hale_types::resolve::build_top_scope(&bundle);
        let graph = build_bus_graph(&bundle, &top);
        let progs: Vec<&hale_syntax::ast::Program> =
            bundle.programs.values().copied().collect();
        let (_, _, ids) =
            claims_report_with_identities(&progs, &graph, &[]);
        ids.into_iter()
            .find(|i| i.name == want)
            .unwrap_or_else(|| panic!("no identity for {}", want))
            .digest
    }

    let src = program(
        r#"
constitution Core { r: count publishers(topic Settled) == 1; }
constitution Dev extends Core { d: count subscribers(topic Settled) == 0; }
main locus App {
    params { b: Billing = Billing { }; r: Research = Research { }; }
    claims { adopt Dev; }
}
"#,
    );
    let changed = src.replace(
        "r: count publishers(topic Settled) == 1;",
        "r: count publishers(topic Settled) == 2;",
    );
    assert_ne!(
        digest_of(&src, "Dev"),
        digest_of(&changed, "Dev"),
        "`Dev`'s own clauses did not change, but its CLOSURE did"
    );
}
