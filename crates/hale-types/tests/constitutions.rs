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
