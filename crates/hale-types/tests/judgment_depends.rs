//! GH #476 Change 5g — `@effects(depends: {…})` over the canonical
//! model (RFC #330).
//!
//! The backward dual of `causes:`, and these controls are mostly
//! about the places the two must AGREE. `causes:` asks what a
//! publish reaches; `depends:` asks what can reach a subscription.
//! If those two walks disagree about whether a publish and a
//! subscription meet, one of them is wrong — so both call the same
//! `model_query` joins, and the cases below are the ones where the
//! evaluator's name comparison and the model's typed identity gave
//! different answers.

use std::collections::BTreeMap;

use hale_types::model_builder::derive_application_model;
use hale_types::symbol::SourceFile;
use hale_types::verdict::Verdict;
use hale_types::Bundle;

fn bundle_of<'a>(
    src: &str,
    program: &'a hale_syntax::ast::Program,
) -> Bundle<'a> {
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), program);
    let mut b = Bundle::new(programs);
    b.sources = vec![SourceFile {
        id: 0,
        path: "app.hl".to_string(),
        digest: "0".to_string(),
        base: 0,
        len: src.len() as u32,
    }];
    b
}

/// The row verdict plus its diagnostics, which is what every
/// control below asserts on.
fn judge(src: &str) -> (Verdict, Vec<String>) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let model = derive_application_model(&bundle);
    let table = hale_types::claim_lowering::lower_claims(&bundle, &model);
    let bases: Vec<u32> = bundle.sources.iter().map(|f| f.base).collect();
    let judged =
        hale_types::judgment::judge_depends(&table, &model, &bases);
    assert_eq!(
        judged.len(),
        1,
        "expected exactly one depends row, got {}",
        judged.len()
    );
    let j = &judged[0];
    (
        j.verdict,
        j.diags.iter().map(|d| d.message.clone()).collect(),
    )
}

const BASE: &str = r#"
type Act { mag: Float; pos: Int; }
topic SumLookup { payload: Act; subject: "sum.lookup"; }
topic Recalled  { payload: Act; subject: "recalled"; }

locus Relay {
    bus { subscribe SumLookup as on_sum; publish Recalled; }
    fn on_sum(a: Act) { Recalled <- Act { mag: a.mag, pos: a.pos }; }
}
@effects(depends: {DECLARED})
locus Carry {
    bus { subscribe Recalled as on_recalled; }
    params { recalled: Float = 0.0; }
    fn on_recalled(a: Act) { self.recalled = a.mag; }
}
locus Compute {
    bus { publish SumLookup; }
    fn go() { SumLookup <- Act { mag: 7.0, pos: 1 }; }
}
main locus App {
    params {
        r: Relay = Relay { }; c: Carry = Carry { };
        k: Compute = Compute { };
    }
}
fn main() { App { }; }
"#;

fn with(declared: &str) -> String {
    BASE.replace("DECLARED", declared)
}

#[test]
fn the_full_backward_closure_holds() {
    let (v, ds) = judge(&with("Recalled, SumLookup"));
    assert_eq!(v, Verdict::Holds, "{:?}", ds);
}

#[test]
fn a_laundered_second_hop_is_a_violation() {
    // `Carry` names only what it directly subscribes. The whole
    // point of the law: `SumLookup` reaches it through `Relay`.
    let (v, ds) = judge(&with("Recalled"));
    assert_eq!(v, Verdict::Violated);
    assert!(
        ds.iter().any(|m| m.contains("SumLookup")
            && m.contains("-> `Relay` ->")),
        "the diagnostic must name the laundering path: {:?}",
        ds
    );
}

#[test]
fn the_declaration_may_name_the_wire_subject() {
    // `"recalled"` IS topic `Recalled` — one endpoint, two
    // spellings. The evaluator compared names and called this an
    // omission; the model joins on `SubjectId`.
    let (v, ds) = judge(&with("\"recalled\", \"sum.lookup\""));
    assert_eq!(
        v,
        Verdict::Holds,
        "wire subject and topic name address the same endpoint: {:?}",
        ds
    );
}

#[test]
fn an_operand_naming_nothing_is_invalid_not_violated() {
    // The evaluator matched by NAME, so a typo covered nothing and
    // every reached subject came back as an omission — a violation
    // report about the subjects, when the defect is the typo.
    let (v, ds) = judge(&with("Recaled, SumLookup"));
    assert_eq!(v, Verdict::Invalid);
    assert!(
        ds.iter().any(|m| m.contains("is invalid")
            && m.contains("Recaled")),
        "the diagnostic must name the unresolved entry: {:?}",
        ds
    );
    assert!(
        !ds.iter().any(|m| m.contains("violated")),
        "an invalid law is not a violated one: {:?}",
        ds
    );
}

#[test]
fn a_sync_form_param_refuses_the_whole_claim() {
    // #340. Another pool writes the form, this locus reads it, and
    // NO bus edge records the transfer — so the message graph, all
    // this walk can see, cannot support a completeness claim.
    let src = r#"
type Act { mag: Float; pos: Int; }
topic Recalled { payload: Act; subject: "recalled"; }
@form(hashmap, sync = lockfree)
locus Shared {
    params { n: Int = 0; }
    fn get() -> Int { return self.n; }
}
@effects(depends: {Recalled})
locus Carry {
    bus { subscribe Recalled as on_recalled; }
    params { shared: Shared = Shared { }; recalled: Float = 0.0; }
    fn on_recalled(a: Act) { self.recalled = a.mag; }
}
main locus App {
    params { c: Carry = Carry { }; }
}
fn main() { App { }; }
"#;
    let (v, ds) = judge(src);
    assert_eq!(v, Verdict::Violated);
    assert!(
        ds.iter().any(|m| m.contains("sync") && m.contains("shared")),
        "the diagnostic must name the param and the discipline: {:?}",
        ds
    );
}

#[test]
fn an_ordinary_child_locus_is_not_a_sync_form() {
    // The premise of the control above: it is the FORM's `sync`
    // argument that matters, not merely holding another locus.
    let src = r#"
type Act { mag: Float; pos: Int; }
topic Recalled { payload: Act; subject: "recalled"; }
locus Plain {
    params { n: Int = 0; }
    fn get() -> Int { return self.n; }
}
@effects(depends: {Recalled})
locus Carry {
    bus { subscribe Recalled as on_recalled; }
    params { plain: Plain = Plain { }; recalled: Float = 0.0; }
    fn on_recalled(a: Act) { self.recalled = a.mag; }
}
main locus App {
    params { c: Carry = Carry { }; }
}
fn main() { App { }; }
"#;
    let (v, ds) = judge(src);
    assert_eq!(v, Verdict::Holds, "{:?}", ds);
}

#[test]
fn an_inbound_route_makes_the_upstream_uncertified() {
    // A `listen` binding accepts from a peer this application does
    // not model. Nothing local publishes `Recalled`, so a walk
    // reading only local publishes would report a clean `holds` —
    // the fail-open the direction-aware endpoint query exists to
    // close.
    let src = r#"
type Act { mag: Float; pos: Int; }
topic Recalled { payload: Act; subject: "recalled"; }
@effects(depends: {Recalled})
locus Carry {
    bus { subscribe Recalled as on_recalled; }
    params { recalled: Float = 0.0; }
    fn on_recalled(a: Act) { self.recalled = a.mag; }
}
main locus App {
    params { c: Carry = Carry { }; }
    bindings { Recalled: unix("/tmp/hale-depends-in.sock", role: listen); }
}
fn main() { App { }; }
"#;
    let (v, ds) = judge(src);
    assert_eq!(
        v,
        Verdict::Uncertified,
        "a peer can publish into this subject: {:?}",
        ds
    );
}

#[test]
fn an_outbound_route_does_not_taint_the_backward_walk() {
    // The dual of the control above, and the reason the query takes
    // a DIRECTION: a `connect` route is where messages LEAVE. It
    // says nothing about what can reach this locus.
    let src = r#"
type Act { mag: Float; pos: Int; }
topic Recalled { payload: Act; subject: "recalled"; }
topic Outbound { payload: Act; subject: "outbound"; }
locus Src {
    bus { publish Recalled; }
    fn go() { Recalled <- Act { mag: 1.0, pos: 1 }; }
}
@effects(depends: {Recalled})
locus Carry {
    bus { subscribe Recalled as on_recalled; publish Outbound; }
    params { recalled: Float = 0.0; }
    fn on_recalled(a: Act) {
        self.recalled = a.mag;
        Outbound <- Act { mag: a.mag, pos: a.pos };
    }
}
main locus App {
    params { s: Src = Src { }; c: Carry = Carry { }; }
    bindings { Outbound: unix("/tmp/hale-depends-out.sock", role: connect); }
}
fn main() { App { }; }
"#;
    let (v, ds) = judge(src);
    assert_eq!(v, Verdict::Holds, "{:?}", ds);
}

#[test]
fn a_free_function_publisher_still_counts_as_an_upstream() {
    // Ownership is how the walk hops from a subject to a locus's
    // own inputs; a publisher owned by no locus has no inputs to
    // walk on to, but its subject reaches here all the same.
    let src = r#"
type Act { mag: Float; pos: Int; }
topic Recalled { payload: Act; subject: "recalled"; }
fn shout() { Recalled <- Act { mag: 1.0, pos: 1 }; }
@effects(depends: {})
locus Carry {
    bus { subscribe Recalled as on_recalled; }
    params { recalled: Float = 0.0; }
    fn on_recalled(a: Act) { self.recalled = a.mag; }
}
main locus App {
    params { c: Carry = Carry { }; }
    run() { shout(); }
}
fn main() { App { }; }
"#;
    let (v, ds) = judge(src);
    assert_eq!(v, Verdict::Violated);
    assert!(
        ds.iter().any(|m| m.contains("Recalled")
            || m.contains("recalled")),
        "{:?}",
        ds
    );
}

#[test]
fn a_pure_publisher_depends_on_nothing() {
    let src = r#"
type Act { mag: Float; pos: Int; }
topic Recalled { payload: Act; subject: "recalled"; }
@effects(depends: {})
locus Src {
    bus { publish Recalled; }
    fn go() { Recalled <- Act { mag: 1.0, pos: 1 }; }
}
main locus App {
    params { s: Src = Src { }; }
    run() { self.s.go(); }
}
fn main() { App { }; }
"#;
    let (v, ds) = judge(src);
    assert_eq!(v, Verdict::Holds, "{:?}", ds);
}
