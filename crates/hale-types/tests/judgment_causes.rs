//! GH #476 Change 5f — `@effects(causes: …)` over the canonical
//! model, held against the evaluator over the corpus.
//!
//! The first version of this differential compared rendered
//! diagnostics only, never verdicts, and permitted ANY model-only
//! output as "the documented stdlib divergence" — which would have
//! blessed a false violation from an unrelated bug (review round 1).
//! It is strict now:
//!
//!   * VERDICTS are compared, not just messages;
//!   * equal-strength verdicts require byte-equal diagnostics;
//!   * the model may only be STRICTER (holds < uncertified <
//!     violated), never weaker — a weakening is always a fail-open;
//!   * and a strengthening must be explained by a premise the test
//!     computes from the program itself, not by "the model printed
//!     more".
//!
//! The premises, each a place the evaluator is blind and the model
//! is not: an unclassified body reachable as a subscriber, a
//! completeness hole, a publish inside a stdlib interior, a delivery
//! whose two ends spell the same wire differently, and a second bus
//! hop.

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

fn render(d: &hale_syntax::Diag) -> String {
    format!(
        "[{}..{}] {}",
        d.span.start.as_usize(),
        d.span.end.as_usize(),
        d.message
    )
}

/// holds < uncertified < violated; invalid is its own answer and
/// must match exactly.
fn strength(v: Verdict) -> u8 {
    match v {
        Verdict::Holds => 0,
        Verdict::Uncertified => 1,
        Verdict::Violated => 2,
        Verdict::Invalid => 3,
    }
}

/// Why the model may legitimately be stricter than the evaluator on
/// this program. Computed from the model, never from the output.
fn divergence_premises(
    model: &hale_model::ApplicationModel,
) -> Vec<&'static str> {
    let e = &model.entities;
    let r = &model.relations;
    let mut out = Vec::new();
    // The evaluator infers effects from the user-only summary, so a
    // subscriber whose class arrives through a stdlib call reads as
    // pure to it.
    if r.subscribes.iter().any(|s| {
        !e.functions[s.handler.index()].effects.is_empty()
    }) {
        out.push("subscriber-effects");
    }
    // Unknown downstream behaviour: the model refuses to certify,
    // the evaluator counts it as nothing.
    if e.functions.iter().any(|f| {
        f.effects.iter().any(|c| c == "unclassified")
    }) {
        out.push("unclassified-subscriber");
    }
    if !model.holes.is_empty() {
        out.push("completeness-hole");
    }
    if !model.analyses.stdlib_absorption.is_empty() {
        out.push("stdlib-interior");
    }
    // A publish and a subscription on ONE wire whose written forms
    // differ — the join the evaluator does by text misses it.
    if r.publishes.iter().any(|p| {
        r.subscribes.iter().any(|s| {
            s.subject == p.subject
                && s.declared_topic != p.declared_topic
        })
    }) {
        out.push("wire-vs-text-join");
    }
    // A subscriber that publishes: a second hop the evaluator's
    // one-level walk never takes.
    if r.subscribes
        .iter()
        .any(|s| r.publishes.iter().any(|p| p.function == s.handler))
    {
        out.push("second-hop");
    }
    out
}

#[test]
fn causes_judgment_matches_the_evaluator_over_the_corpus() {
    let mut with_rows = 0usize;
    let mut strengthened = 0usize;
    let mut bad: Vec<String> = Vec::new();
    for p in
        hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok())
    {
        let Ok(program) = hale_syntax::parse_source(&p.source) else {
            continue;
        };
        let bundle = bundle_of(&p.source, &program);
        if hale_types::check_bundle_opts(&bundle, false)
            .iter()
            .any(|d| {
                d.is_error()
                    && d.kind != hale_syntax::error::DiagKind::Claim
            })
        {
            continue;
        }
        let model = derive_application_model(&bundle);
        let table =
            hale_types::claim_lowering::lower_claims(&bundle, &model);
        if !table.rows.iter().any(|r| {
            matches!(r.law, hale_model::ClaimIr::EffectCauses { .. })
        }) {
            continue;
        }
        with_rows += 1;

        let programs_v: Vec<&hale_syntax::ast::Program> = vec![&program];
        let (top, _) = hale_types::resolve::build_top_scope(&bundle);
        let graph = hale_types::bus_graph::build_bus_graph(&bundle, &top);
        let old_diags: Vec<String> =
            hale_types::frontier::causes_diags(&programs_v, &graph)
                .iter()
                .map(render)
                .collect();
        let bases: Vec<u32> =
            bundle.sources.iter().map(|f| f.base).collect();
        let judged =
            hale_types::judgment::judge_causes(&table, &model, &bases);
        let new_diags: Vec<String> = judged
            .iter()
            .flat_map(|j| j.diags.iter())
            .map(render)
            .collect();

        // The evaluator has no verdict channel: a diagnostic is a
        // violation, silence is a pass. Compare at that grain.
        let old_v = if old_diags.is_empty() {
            Verdict::Holds
        } else {
            Verdict::Violated
        };
        // How each side expresses "I don't know". The evaluator
        // SATURATES: the unclassified bit unions in as every class
        // at once, so its message asserts `can transitively cause
        // syscall, block, time, entropy, env, secret_use` — a
        // definite claim about classes nobody observed, built on the
        // absence of information. The model answers `Uncertified`,
        // which is the vocabulary this epic settled on for exactly
        // that evidence (Change 6: uncertified is a state distinct
        // from violated). Neither certifies; they differ in what
        // they are willing to assert.
        //
        // Read off the MODEL, not the message: an unclassified body
        // sits on a delivered-to path.
        let premises_here = divergence_premises(&model);
        let evaluator_guessed =
            premises_here.contains(&"unclassified-subscriber");
        for j in &judged {
            let weaker = strength(j.verdict) < strength(old_v);
            let honest_uncertified = evaluator_guessed
                && j.verdict == Verdict::Uncertified;
            if weaker && !honest_uncertified {
                bad.push(format!(
                    "{}: model is WEAKER ({:?} vs evaluator {:?}) — a \
                     fail-open",
                    p.origin, j.verdict, old_v
                ));
            }
        }
        if evaluator_guessed
            && judged.iter().all(|j| j.verdict == Verdict::Uncertified)
        {
            strengthened += 1;
            continue;
        }
        let model_v = judged
            .iter()
            .map(|j| j.verdict)
            .max_by_key(|v| strength(*v))
            .unwrap_or(Verdict::Holds);
        if strength(model_v) == strength(old_v) {
            if old_diags != new_diags {
                bad.push(format!(
                    "{}: same verdict, different diagnostics:\n  \
                     evaluator: {:?}\n  model:     {:?}",
                    p.origin, old_diags, new_diags
                ));
            }
            continue;
        }
        // Stricter: allowed only with a premise, and the premise is
        // read off the model.
        let premises = premises_here;
        if premises.is_empty() {
            bad.push(format!(
                "{}: model is stricter ({:?} vs {:?}) with NO premise \
                 that explains it:\n  evaluator: {:?}\n  model:     {:?}",
                p.origin, model_v, old_v, old_diags, new_diags
            ));
            continue;
        }
        strengthened += 1;
    }
    assert!(
        with_rows > 0,
        "the corpus must exercise `causes:` — the differential \
         would pass vacuously"
    );
    assert!(
        bad.is_empty(),
        "{} programs disagree:\n{}",
        bad.len(),
        bad.join("\n")
    );
    eprintln!(
        "causes differential: {} programs, {} strengthened",
        with_rows, strengthened
    );
}

// ===================== negative controls =========================
//
// Each drops or perturbs ONE relation the engine claims to read and
// requires the verdict to move. A judgment that ignores a relation
// cannot fail these.

fn judge(src: &str) -> (Verdict, String) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let model = derive_application_model(&bundle);
    let table = hale_types::claim_lowering::lower_claims(&bundle, &model);
    let bases: Vec<u32> = bundle.sources.iter().map(|f| f.base).collect();
    let judged = hale_types::judgment::judge_causes(&table, &model, &bases);
    let j = judged.first().expect("a causes row was judged");
    (
        j.verdict,
        j.diags.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join(" | "),
    )
}

/// The reviewer's fixture: an unclassified subscriber is UNKNOWN
/// behaviour, not absent behaviour.
#[test]
fn an_unclassified_subscriber_is_uncertified_not_holds() {
    let (v, _) = judge(
        r#"
type Msg { n: Int = 0; }
topic T { payload: Msg; subject: "t"; }
fn does_syscall(x: Int) -> Int { println("side effect"); return x; }
fn apply(f: fn (Int) -> Int, x: Int) -> Int { return f(x); }
locus Sink {
    params { n: Int = 0; }
    bus { subscribe T as on_t; }
    fn on_t(m: Msg) { self.n = apply(does_syscall, m.n); }
}
locus Source {
    bus { publish T; }
    @effects(causes: { publish, alloc })
    fn fire() { T <- Msg { n: 1 }; }
}
main locus App {
    params { s: Sink = Sink { }; p: Source = Source { }; }
    run() { self.p.fire(); }
}
fn main() { App { }; }
"#,
    );
    assert_eq!(
        v,
        Verdict::Uncertified,
        "an indirect call the analysis cannot follow makes the \
         causal set a lower bound, not an answer"
    );
}

/// The reviewer's second fixture: publisher and subscriber name ONE
/// wire in two spellings. Delivery is real, so the downstream
/// syscall is caused.
#[test]
fn a_wire_identity_join_finds_the_delivery() {
    let (v, msg) = judge(
        r#"
type Msg { n: Int = 0; }
topic Orders { payload: Msg; subject: "orders"; }
locus Sink {
    params { n: Int = 0; }
    bus { subscribe "orders" as on_order of type Msg; }
    fn on_order(m: Msg) { println("received"); }
}
locus Source {
    bus { publish Orders; }
    @effects(causes: { publish, alloc })
    fn fire() { Orders <- Msg { n: 1 }; }
}
main locus App {
    params { s: Sink = Sink { }; p: Source = Source { }; }
    run() { self.p.fire(); }
}
fn main() { App { }; }
"#,
    );
    assert_eq!(
        v,
        Verdict::Violated,
        "a topic-name publish and a literal subscribe on the same \
         wire deliver; comparing rendered text certified it away"
    );
    assert!(
        msg.contains("syscall"),
        "the downstream syscall is the excess: {}",
        msg
    );
}

/// The reviewer's third fixture: a class occurring BOTH locally and
/// downstream is still caused downstream. Set subtraction erased it.
#[test]
fn a_local_occurrence_does_not_erase_the_downstream_one() {
    let (v, msg) = judge(
        r#"
type Msg { n: Int = 0; }
topic T { payload: Msg; subject: "t"; }
locus Sink {
    params { n: Int = 0; }
    bus { subscribe T as on_t; }
    fn on_t(m: Msg) { println("downstream syscall"); }
}
locus Source {
    bus { publish T; }
    @effects(causes: { publish, alloc })
    fn fire() { println("local syscall"); T <- Msg { n: 1 }; }
}
main locus App {
    params { s: Sink = Sink { }; p: Source = Source { }; }
    run() { self.p.fire(); }
}
fn main() { App { }; }
"#,
    );
    assert_eq!(
        v,
        Verdict::Violated,
        "the downstream syscall is caused whether or not the same \
         class also occurs locally"
    );
    assert!(msg.contains("syscall"), "{}", msg);
}

/// `ffi` IS the syscall bit. A declaration naming it must satisfy an
/// actual set that spells the same bit `syscall` — the false
/// rejection the old differential would have blessed.
#[test]
fn ffi_declares_the_syscall_bit() {
    let (v, msg) = judge(
        r#"
type Msg { n: Int = 0; }
topic T { payload: Msg; subject: "t"; }
locus Sink {
    params { n: Int = 0; }
    bus { subscribe T as on_t; }
    fn on_t(m: Msg) { println("io"); }
}
locus Source {
    bus { publish T; }
    @effects(causes: { ffi, publish, alloc })
    fn fire() { T <- Msg { n: 1 }; }
}
main locus App {
    params { s: Sink = Sink { }; p: Source = Source { }; }
    run() { self.p.fire(); }
}
fn main() { App { }; }
"#,
    );
    assert_eq!(
        v,
        Verdict::Holds,
        "`causes: {{ffi}}` covers a downstream syscall: {}",
        msg
    );
}

/// Canonical rendering: built-ins in the language's fixed order,
/// then user classes in DECLARATION order — not the lexical order a
/// set iterates in.
#[test]
fn excess_classes_render_in_canonical_order() {
    let (v, msg) = judge(
        r#"
effect zeta;
effect alpha;
type Msg { n: Int = 0; }
topic T { payload: Msg; subject: "t"; }
@effects(is: { zeta })
fn z(v: Int) -> Int { return v; }
@effects(is: { alpha })
fn a(v: Int) -> Int { return v; }
locus Sink {
    params { n: Int = 0; }
    bus { subscribe T as on_t; }
    fn on_t(m: Msg) { self.n = z(m.n) + a(m.n); println("io"); }
}
locus Source {
    bus { publish T; }
    @effects(causes: { publish, alloc })
    fn fire() { T <- Msg { n: 1 }; }
}
main locus App {
    params { s: Sink = Sink { }; p: Source = Source { }; }
    run() { self.p.fire(); }
}
fn main() { App { }; }
"#,
    );
    assert_eq!(v, Verdict::Violated);
    let at = |c: &str| msg.find(c).unwrap_or(usize::MAX);
    assert!(
        at("syscall") < at("zeta"),
        "built-ins render before user classes: {}",
        msg
    );
    assert!(
        at("zeta") < at("alpha"),
        "user classes render in DECLARATION order (zeta declared \
         first), not lexically: {}",
        msg
    );
}

/// The engine reads the CALLS relation: a publish reached only
/// through a call is still the root's doing.
#[test]
fn a_publish_behind_a_call_is_still_caused() {
    let (v, _) = judge(
        r#"
type Msg { n: Int = 0; }
topic T { payload: Msg; subject: "t"; }
locus Sink {
    params { n: Int = 0; }
    bus { subscribe T as on_t; }
    fn on_t(m: Msg) { println("io"); }
}
locus Source {
    bus { publish T; }
    fn emit() { T <- Msg { n: 1 }; }
    @effects(causes: { publish, alloc })
    fn fire() { self.emit(); }
}
main locus App {
    params { s: Sink = Sink { }; p: Source = Source { }; }
    run() { self.p.fire(); }
}
fn main() { App { }; }
"#,
    );
    assert_eq!(
        v,
        Verdict::Violated,
        "the causal walk follows calls to find publish sites"
    );
}

/// …and it follows a SECOND hop: a handler that publishes carries
/// the root's causality onward.
#[test]
fn a_second_bus_hop_is_caused_too() {
    let (v, _) = judge(
        r#"
type Msg { n: Int = 0; }
topic First { payload: Msg; subject: "first"; }
topic Second { payload: Msg; subject: "second"; }
locus Relay {
    params { n: Int = 0; }
    bus { subscribe First as on_first; publish Second; }
    fn on_first(m: Msg) { Second <- m; }
}
locus Deep {
    params { n: Int = 0; }
    bus { subscribe Second as on_second; }
    fn on_second(m: Msg) { println("io"); }
}
locus Source {
    bus { publish First; }
    @effects(causes: { publish, alloc })
    fn fire() { First <- Msg { n: 1 }; }
}
main locus App {
    params { r: Relay = Relay { }; d: Deep = Deep { }; p: Source = Source { }; }
    run() { self.p.fire(); }
}
fn main() { App { }; }
"#,
    );
    assert_eq!(
        v,
        Verdict::Violated,
        "`causes` asks what this fn can cause ANYWHERE — the walk \
         does not stop at the first delivery"
    );
}

/// A wildcard subscription covers the concrete subject published.
#[test]
fn a_wildcard_subscriber_receives_the_publish() {
    let (v, _) = judge(
        r#"
type Msg { n: Int = 0; }
topic Created { payload: Msg; subject: "orders.created"; }
locus Audit {
    params { n: Int = 0; }
    bus { subscribe "orders.**" as on_any of type Msg; }
    fn on_any(m: Msg) { println("io"); }
}
locus Source {
    bus { publish Created; }
    @effects(causes: { publish, alloc })
    fn fire() { Created <- Msg { n: 1 }; }
}
main locus App {
    params { a: Audit = Audit { }; p: Source = Source { }; }
    run() { self.p.fire(); }
}
fn main() { App { }; }
"#,
    );
    assert_eq!(
        v,
        Verdict::Violated,
        "`orders.**` covers `orders.created`"
    );
}

/// The control that keeps the rest honest: a genuinely pure
/// downstream still HOLDS. Without it, an engine that answered
/// `violated` unconditionally would pass every test above.
#[test]
fn a_pure_downstream_holds() {
    let (v, msg) = judge(
        r#"
type Msg { n: Int = 0; }
topic T { payload: Msg; subject: "t"; }
locus Sink {
    params { n: Int = 0; }
    bus { subscribe T as on_t; }
    fn on_t(m: Msg) { self.n = m.n + 1; }
}
locus Source {
    bus { publish T; }
    @effects(causes: { publish, alloc })
    fn fire() { T <- Msg { n: 1 }; }
}
main locus App {
    params { s: Sink = Sink { }; p: Source = Source { }; }
    run() { self.p.fire(); }
}
fn main() { App { }; }
"#,
    );
    assert_eq!(v, Verdict::Holds, "nothing undeclared is caused: {}", msg);
}
