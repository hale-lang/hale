//! Where the three tiers meet: constitutions and effects on
//! components that are then composed into a fleet.
//!
//! Each tier has its own tests. This file covers what happens when
//! law authored at one altitude rides into another — the questions
//! nobody asks until a deployment answers them wrong.
//!
//! The load-bearing result here is a NEGATIVE one, recorded
//! deliberately: two components can adopt constitutions that share a
//! name and differ in content, and the fleet composes clean without
//! recording either. See
//! `a_shared_constitution_name_across_components_is_not_checked`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root(tag: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("hale_fct_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn write(root: &Path, rel: &str, src: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(&p, src).expect("write");
}

fn hale(args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .output()
        .expect("run hale");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

const TOPICS: &str = r#"
type Intent { id: Int; }
topic OrderIntent { payload: Intent; subject: "svc.order.intent"; }
"#;

/// Emitter adopting a constitution named `Core`.
fn emitter(clause: &str) -> String {
    format!(
        r#"
import "../lib" as t;
constitution Core {{
    {clause}
}}
locus Emit {{
    params {{ n: Int = 0; }}
    bus {{ publish t::OrderIntent; }}
    fn go() {{ let i = t::Intent {{ id: 1 }}; t::OrderIntent <- i; }}
}}
main locus AApp {{
    params {{ e: Emit = Emit {{ }}; }}
    claims {{ adopt Core; }}
}}
fn main() {{ AApp {{ }}; }}
"#
    )
}

/// Receiver adopting a constitution ALSO named `Core`.
fn receiver(clause: &str) -> String {
    format!(
        r#"
import "../lib" as t;
constitution Core {{
    {clause}
}}
locus Recv {{
    params {{ n: Int = 0; }}
    bus {{ subscribe t::OrderIntent as on_i; }}
    fn on_i(i: t::Intent) {{ self.n = i.id; }}
}}
main locus BApp {{
    params {{ r: Recv = Recv {{ }}; }}
    claims {{ adopt Core; }}
}}
fn main() {{ BApp {{ }}; }}
"#
    )
}

const PLAN: &str = r#"{
  "schema": "1.0", "name": "prod",
  "instances": [
    {"id": "a-0", "artifact": "artifacts/a.json", "labels": ["emit"]},
    {"id": "b-0", "artifact": "artifacts/b.json", "labels": ["recv"]}],
  "routes": [
    {"id": "r", "transport": "unix",
     "publishers":  [{"instance": "a-0", "topic": "t::OrderIntent"}],
     "subscribers": [{"instance": "b-0", "topic": "t::OrderIntent"}]}]
}"#;

/// Build both components and return (root, per-component artifact).
fn two_components(
    tag: &str,
    a_clause: &str,
    b_clause: &str,
) -> (PathBuf, serde_json::Value, serde_json::Value) {
    let r = root(tag);
    write(&r, "lib/topics.hl", TOPICS);
    write(&r, "a/main.hl", &emitter(a_clause));
    write(&r, "b/main.hl", &receiver(b_clause));
    write(&r, "hale.toml", "[deps]\n");
    write(&r, "plan.json", PLAN);
    for app in ["a", "b"] {
        let dst = r.join(format!("artifacts/{}.json", app));
        std::fs::create_dir_all(dst.parent().expect("parent")).expect("mkdir");
        let (out, code) = hale(&[
            "check",
            r.join(app).to_str().expect("utf8"),
            &format!("--dump-topology={}", dst.display()),
        ]);
        assert_eq!(code, 0, "component `{}` must check clean: {}", app, out);
    }
    let read = |n: &str| -> serde_json::Value {
        serde_json::from_str(
            &std::fs::read_to_string(r.join(format!("artifacts/{}.json", n)))
                .expect("artifact"),
        )
        .expect("artifact parses")
    };
    let (a, b) = (read("a"), read("b"));
    (r, a, b)
}

/// Two constitutions with the same NAME and different clauses get
/// different digests. This is the property the whole design rests on
/// — within a seed it is enforced; the next test shows the fleet does
/// not look at it.
#[test]
fn same_named_constitutions_with_different_clauses_differ_in_digest() {
    let (r, a, b) = two_components(
        "digests",
        "one_pub: count publishers(topic t::OrderIntent) == 1;",
        "any_subs: count subscribers(topic t::OrderIntent) >= 0;",
    );
    let dig = |v: &serde_json::Value| -> String {
        v["evaluation"]["closure"][0]["digest"]
            .as_str()
            .unwrap_or("")
            .to_string()
    };
    let name = |v: &serde_json::Value| -> String {
        v["evaluation"]["closure"][0]["name"]
            .as_str()
            .unwrap_or("")
            .to_string()
    };
    assert_eq!(name(&a), "Core", "both adopt a constitution called Core");
    assert_eq!(name(&b), "Core");
    assert_ne!(
        dig(&a),
        dig(&b),
        "different clauses under one name must not share an identity"
    );
    let _ = std::fs::remove_dir_all(&r);
}

/// **Known gap, pinned deliberately.**
///
/// Two components each record adopting `Core`, with different
/// closures. The fleet composes clean, and its artifact records only
/// `{id, artifact}` per component — so nothing anywhere says which
/// law each component was actually held to, and a reader of the
/// fleet artifact would reasonably conclude they share one.
///
/// This is NOT obviously a bug to fix by erroring: independently
/// built artifacts may legitimately each define a local `Core`, and
/// refusing that would be wrong. The gap is that the fleet artifact
/// does not carry the closures forward, so the arrangement cannot be
/// audited after the fact. The analogous check DOES exist one tier
/// down — `env_matrix::one_environment_may_not_mean_two_different_claimsets`.
///
/// This test asserts today's behavior. If composition later grows to
/// record or reconcile closures, this is the test to change, and the
/// change is deliberate rather than accidental.
#[test]
fn a_shared_constitution_name_across_components_is_not_checked() {
    let (r, _, _) = two_components(
        "namecollision",
        "one_pub: count publishers(topic t::OrderIntent) == 1;",
        "any_subs: count subscribers(topic t::OrderIntent) >= 0;",
    );
    let plan = r.join("plan.json");
    let (out, code) = hale(&["fleet", "check", plan.to_str().expect("utf8")]);
    assert_eq!(
        code, 0,
        "today two different `Core`s compose without complaint: {out}"
    );

    let dump = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["fleet", "dump", plan.to_str().expect("utf8")])
        .output()
        .expect("run hale");
    let v: serde_json::Value =
        serde_json::from_slice(&dump.stdout).expect("fleet artifact parses");
    let components = v["components"].to_string();
    assert!(
        !components.contains("digest"),
        "if the fleet artifact starts carrying component constitution \
         digests, this gap is closed — update this test: {components}"
    );
    let _ = std::fs::remove_dir_all(&r);
}

/// A component whose own adopted clause FAILS must never enter a
/// fleet. The constitution is the component's law; the fleet is
/// entitled to assume every component was certified under its own.
#[test]
fn a_component_failing_its_adopted_clause_cannot_enter_a_fleet() {
    let r = root("failedclause");
    write(&r, "lib/topics.hl", TOPICS);
    // `a` publishes OrderIntent, so demanding zero publishers fails.
    write(
        &r,
        "a/main.hl",
        &emitter("none: count publishers(topic t::OrderIntent) == 0;"),
    );
    write(
        &r,
        "b/main.hl",
        &receiver("any_subs: count subscribers(topic t::OrderIntent) >= 0;"),
    );
    write(&r, "hale.toml", "[deps]\n");
    write(&r, "plan.json", PLAN);

    for app in ["a", "b"] {
        let dst = r.join(format!("artifacts/{}.json", app));
        std::fs::create_dir_all(dst.parent().expect("parent")).expect("mkdir");
        let _ = hale(&[
            "check",
            r.join(app).to_str().expect("utf8"),
            &format!("--dump-topology={}", dst.display()),
        ]);
    }
    let (out, code) = hale(&[
        "fleet",
        "check",
        r.join("plan.json").to_str().expect("utf8"),
    ]);
    assert_eq!(
        code, 1,
        "a component whose constitution clause is violated must be \
         refused by the fleet, not silently composed: {out}"
    );
    let _ = std::fs::remove_dir_all(&r);
}

/// An effect class named by a constitution clause but never declared
/// is an error in the adopting seed, and the diagnostic says how to
/// fix it. Silence here would mean the clause quietly quantifies over
/// nothing and always holds.
#[test]
fn a_constitution_naming_an_undeclared_effect_class_is_refused() {
    let r = root("undeclared_effect");
    write(&r, "hale.toml", "[deps]\n");
    write(
        &r,
        "app/main.hl",
        r#"
locus Billing { params { n: Int = 0; } fn go() -> Int { return self.n; } }
group billing = { Billing };
constitution Core {
    no_llm: forbid reaches(billing, effects(llm));
}
main locus App {
    params { b: Billing = Billing { }; }
    claims { adopt Core; }
}
fn main() { App { }; }
"#,
    );
    let (out, code) =
        hale(&["check", r.join("app").to_str().expect("utf8")]);
    assert_eq!(code, 1, "an undeclared effect class must fail: {out}");
    assert!(
        out.contains("never declared"),
        "the diagnostic should name the missing declaration: {out}"
    );

    // Declaring the class makes the same clause evaluable — and it
    // holds, because nothing reaches it. The check is not merely
    // rejecting the word `effects`.
    write(
        &r,
        "app/main.hl",
        r#"
effect llm;
locus Billing { params { n: Int = 0; } fn go() -> Int { return self.n; } }
group billing = { Billing };
constitution Core {
    no_llm: forbid reaches(billing, effects(llm));
}
main locus App {
    params { b: Billing = Billing { }; }
    claims { adopt Core; }
}
fn main() { App { }; }
"#,
    );
    let (out, code) =
        hale(&["check", r.join("app").to_str().expect("utf8")]);
    assert_eq!(code, 0, "declared class, nothing reaches it: {out}");
    let _ = std::fs::remove_dir_all(&r);
}

/// The fleet claim vocabulary has no `effects(...)` target, and the
/// carve-out is structural rather than a runtime check: effect class
/// names are seed-local, so they have no meaning across artifacts.
/// `deny_unknown_fields` makes an attempt to smuggle one in a hard
/// error instead of an ignored field.
#[test]
fn a_fleet_claim_cannot_name_an_effect_class() {
    let (r, _, _) = two_components(
        "noeffects",
        "one_pub: count publishers(topic t::OrderIntent) == 1;",
        "any_subs: count subscribers(topic t::OrderIntent) >= 0;",
    );
    let plan = r#"{
      "schema": "1.0", "name": "prod",
      "instances": [
        {"id": "a-0", "artifact": "artifacts/a.json", "labels": ["emit"]},
        {"id": "b-0", "artifact": "artifacts/b.json", "labels": ["recv"]}],
      "groups": {"emit": {"labels": ["emit"]}},
      "claims": [
        {"name": "no_llm",
         "forbid_reaches": {"from": "emit", "to": "effects(llm)"},
         "effects": ["llm"]}]
    }"#;
    write(&r, "bad.plan.json", plan);
    let (out, code) = hale(&[
        "fleet",
        "check",
        r.join("bad.plan.json").to_str().expect("utf8"),
    ]);
    assert_eq!(code, 1, "an unknown claim field must be refused: {out}");

    // And with the stray field removed, `effects(llm)` as a group
    // name is an unknown group — not a silently-empty one that makes
    // the claim hold.
    let plan = plan.replace(",\n         \"effects\": [\"llm\"]", "");
    write(&r, "bad2.plan.json", &plan);
    let (out, code) = hale(&[
        "fleet",
        "check",
        r.join("bad2.plan.json").to_str().expect("utf8"),
    ]);
    assert_eq!(
        code, 1,
        "`effects(llm)` is not a declared fleet group and must not \
         resolve to the empty set: {out}"
    );
    let _ = std::fs::remove_dir_all(&r);
}
