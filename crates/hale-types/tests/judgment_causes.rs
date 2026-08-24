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

/// Why the model may legitimately differ from the evaluator ON THIS
/// ROW — read from the row's own traversal witness, never from
/// facts elsewhere in the program. A hole on an unrelated topic, an
/// unclassified function nothing delivers to, a stdlib interior
/// another law crosses: none of them excuse a different answer
/// here.
fn row_premises(
    model: &hale_model::ApplicationModel,
    w: &hale_types::judgment::CausesWitness,
) -> Vec<&'static str> {
    let mut out = Vec::new();
    if !w.unknown_handlers.is_empty() {
        out.push("unclassified-handler-on-path");
    }
    if !w.incomplete_endpoints.is_empty() {
        out.push("incomplete-endpoint-on-path");
    }
    if !w.incomplete_discovery.is_empty() {
        out.push("incomplete-discovery-on-path");
    }
    if w.crossed_stdlib_interior {
        out.push("stdlib-interior-on-path");
    }
    if w.multi_hop {
        out.push("second-hop");
    }
    // The evaluator infers effects from the user-only summary, so a
    // handler this row reaches whose classes come through a stdlib
    // call reads as pure to it.
    if w.reached_handlers.iter().any(|h| {
        !model.entities.functions[h.index()]
            .effect_lower_bound
            .is_empty()
    }) {
        out.push("reached-handler-effects");
    }
    out
}

#[test]
fn causes_judgment_matches_the_evaluator_over_the_corpus() {
    let mut rows_compared = 0usize;
    let mut strengthened = 0usize;
    let mut unenumerated = 0usize;
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

        let programs_v: Vec<&hale_syntax::ast::Program> = vec![&program];
        let (top, _) = hale_types::resolve::build_top_scope(&bundle);
        let graph = hale_types::bus_graph::build_bus_graph(&bundle, &top);
        // PER-ASSERTION outcomes. A function may carry several
        // `causes:` clauses and every one of them anchors its
        // diagnostic at the same fn-name span, so a span is not an
        // identity — joining on it lets one clause's diagnostic
        // stand in as another clause's answer (review round 4).
        let old: Vec<hale_types::frontier::CausesReport> =
            hale_types::frontier::causes_reports(&programs_v, &graph);
        let bases: Vec<u32> =
            bundle.sources.iter().map(|f| f.base).collect();
        let judged = hale_types::judgment::judge_causes_witnessed(
            &table, &model, &bases,
        );

        // ONE LAW TO ONE LAW. Both sides anchor a causes diagnostic
        // at the annotated fn's name span, which is also the row's
        // provenance — so the rows and the evaluator's outcomes join
        // on span. A program-wide "any diagnostic means violated"
        // compares row B against row A's answer.
        for (j, w) in &judged {
            let row = table
                .rows
                .iter()
                .find(|r| r.ordinal == j.ordinal)
                .expect("judged row exists");
            // (function, assertion ordinal) — the key both sides
            // can produce. The lowering emits one row per `causes:`
            // clause in source order, which is the order the
            // evaluator enumerates them in.
            let hale_model::ClaimIr::EffectCauses { at, .. } = &row.law
            else {
                continue;
            };
            let fn_display = at
                .0
                .map(|f| {
                    model.entities.functions[f.index()].display.clone()
                })
                .unwrap_or_else(|| at.1.display.clone());
            let assertion_ordinal = table
                .rows
                .iter()
                .filter(|r| {
                    matches!(
                        &r.law,
                        hale_model::ClaimIr::EffectCauses { at: a, .. }
                            if a.0 == at.0
                    )
                })
                .position(|r| r.ordinal == row.ordinal)
                .expect("this row is one of its fn's causes rows");
            let mine: Vec<&hale_syntax::Diag> = old
                .iter()
                .filter(|r| {
                    r.function == fn_display
                        && r.ordinal == assertion_ordinal
                })
                .filter_map(|r| r.diag.as_ref())
                .collect();
            let matched = old.iter().any(|r| {
                r.function == fn_display
                    && r.ordinal == assertion_ordinal
            });
            if !matched {
                // No oracle for this row. The evaluator's root walk
                // is non-recursive over modules, so a module-scoped
                // `causes:` annotation is never enumerated by it —
                // the documented Change-6 rule that unmigrated rows
                // bridge to the old engines ONLY where the old walk
                // demonstrably saw them. Nothing to compare against;
                // `a_module_scoped_row_has_no_evaluator_outcome`
                // pins that this is the reason.
                unenumerated += 1;
                continue;
            }
            let old_v = if mine.is_empty() {
                Verdict::Holds
            } else {
                Verdict::Violated
            };
            rows_compared += 1;
            let premises = row_premises(&model, w);
            // The evaluator SATURATES on unknown: it unions the
            // unclassified marker as every class at once and asserts
            // them. `Uncertified` is this epic's vocabulary for that
            // same evidence, so it is not a weakening — but only
            // when an unknown handler is actually on THIS row's
            // path.
            let saturating = premises
                .contains(&"unclassified-handler-on-path");
            let mine_text: String = mine
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join(" ");
            let new_text: String = j
                .diags
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join(" ");

            if strength(j.verdict) < strength(old_v) {
                if !(saturating && j.verdict == Verdict::Uncertified) {
                    bad.push(format!(
                        "{} ordinal {}: model is WEAKER ({:?} vs {:?}) \
                         — a fail-open",
                        p.origin, j.ordinal, j.verdict, old_v
                    ));
                }
                strengthened += 1;
                continue;
            }
            if strength(j.verdict) > strength(old_v) {
                if premises.is_empty() {
                    bad.push(format!(
                        "{} ordinal {}: model is stricter ({:?} vs \
                         {:?}) with NO premise on this row's path\n  \
                         evaluator: {}\n  model: {}",
                        p.origin, j.ordinal, j.verdict, old_v,
                        mine_text, new_text
                    ));
                }
                strengthened += 1;
                continue;
            }
            // Equal strength: byte-equal messages, except that a
            // saturated evaluator message is compared as a SUPERSET
            // of the model's class list.
            if saturating {
                let invented: Vec<&str> = [
                    "syscall", "block", "publish", "time", "entropy",
                    "env", "alloc", "secret_use",
                ]
                .into_iter()
                .filter(|c| {
                    new_text.contains(c) && !mine_text.contains(c)
                })
                .collect();
                if !invented.is_empty() {
                    bad.push(format!(
                        "{} ordinal {}: the model names classes the \
                         evaluator did not: {:?}",
                        p.origin, j.ordinal, invented
                    ));
                }
                continue;
            }
            let old_rendered: Vec<String> =
                mine.iter().map(|d| render(d)).collect();
            let new_rendered: Vec<String> =
                j.diags.iter().map(render).collect();
            if old_rendered != new_rendered {
                bad.push(format!(
                    "{} ordinal {}: same verdict, different \
                     diagnostics:\n  evaluator: {:?}\n  model: {:?}",
                    p.origin, j.ordinal, old_rendered, new_rendered
                ));
            }
        }
    }
    assert!(
        rows_compared > 0,
        "the corpus must exercise `causes:` — the differential \
         would pass vacuously"
    );
    assert!(
        bad.is_empty(),
        "{} rows disagree:\n{}",
        bad.len(),
        bad.join("\n")
    );
    eprintln!(
        "causes differential: {} rows compared, {} with a \
         documented divergence, {} the evaluator never enumerated",
        rows_compared, strengthened, unenumerated
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

/// Review round 2, blocker 1: a KNOWN excess beats uncertainty.
///
/// The handler definitely performs a syscall AND makes an indirect
/// call. The declaration does not cover syscall, so the law is
/// already violated whatever the indirect call turns out to do —
/// monotone, and the reason the model must keep a lower bound
/// rather than a rendered `unclassified` token that discards it.
#[test]
fn a_known_excess_beats_uncertainty() {
    let (v, msg) = judge(
        r#"
type Msg { n: Int = 0; }
topic T { payload: Msg; subject: "t"; }
fn id(x: Int) -> Int { return x; }
fn apply(f: fn (Int) -> Int, x: Int) -> Int { return f(x); }
locus Sink {
    params { n: Int = 0; }
    bus { subscribe T as on_t; }
    fn on_t(m: Msg) {
        println("definite syscall");
        self.n = apply(id, m.n);
    }
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
        Verdict::Violated,
        "a known syscall already exceeds the declaration: {}",
        msg
    );
    assert!(msg.contains("syscall"), "{}", msg);
}

/// …and the anti-control: the SAME indirect call with nothing known
/// beyond the declaration stays `Uncertified`. Without this, an
/// engine that answered `Violated` on any uncertainty would pass
/// the test above.
#[test]
fn uncertainty_without_a_known_excess_is_uncertified() {
    let (v, _) = judge(
        r#"
type Msg { n: Int = 0; }
topic T { payload: Msg; subject: "t"; }
fn id(x: Int) -> Int { return x; }
fn apply(f: fn (Int) -> Int, x: Int) -> Int { return f(x); }
locus Sink {
    params { n: Int = 0; }
    bus { subscribe T as on_t; }
    fn on_t(m: Msg) { self.n = apply(id, m.n); }
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
    assert_eq!(v, Verdict::Uncertified);
}

/// Review round 2, blocker 2: uncertainty is RELEVANCE-SCOPED. An
/// unrelated adapter-bound topic is a hole somewhere else in the
/// application; nothing on this causal closure reaches it, so it
/// must not turn a local `Holds` into `Uncertified`.
#[test]
fn an_unrelated_binding_does_not_poison_the_law() {
    let (v, _) = judge(
        r#"
type Msg { n: Int = 0; }
topic T { payload: Msg; subject: "t"; }
topic Unrelated { payload: Msg; subject: "unrelated"; }
locus MyAdapter {
    params { n: Int = 0; }
    fn send(subject: String, bytes: Bytes) { self.n = self.n + 1; }
}
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
    bindings { Unrelated: MyAdapter { }; }
    run() { self.p.fire(); }
}
fn main() { App { }; }
"#,
    );
    assert_eq!(
        v,
        Verdict::Holds,
        "a hole on an unrelated topic says nothing about this \
         causal closure"
    );
}

/// Keyed delivery widens when the produced key is not statically
/// known. The builder records this publish's domain as
/// `AnyOfType(Int)` — it does not read `1` out of the literal — so
/// the edge is possible and the law is violated. The conservative
/// direction, and the reviewer's "unknown domain still creates the
/// possible edge" control.
#[test]
fn an_unknown_key_domain_widens_conservatively() {
    let (v, _) = judge(
        r#"
type Msg { shard: Int = 0; }
topic T { payload: Msg; subject: "t"; keyed_by shard; }
locus Sink {
    params { n: Int = 0; }
    bus { subscribe T as on_t where key == 2; }
    fn on_t(m: Msg) { println("io"); }
}
locus Source {
    bus { publish T; }
    @effects(causes: { publish, alloc })
    fn fire() { T <- Msg { shard: 1 }; }
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
        "an unknown produced key cannot rule the delivery out"
    );
}

/// …and the disjointness rule itself, pinned where it lives. A
/// lawful model CAN carry an exact domain (`validate` accepts it as
/// a first-class fact); the builder simply does not infer one from
/// a literal today, so the rule is exercised on the query directly
/// rather than through a program that cannot reach the state.
#[test]
fn may_deliver_decides_exact_key_domains() {
    use hale_model::keys::{KeyDomain, KeyPredicate, KeyValue};
    let e = hale_model::Entities {
        subjects: vec![hale_model::Subject {
            pattern: "t".to_string(),
            exact: true,
            provenance: hale_model::ProvenanceId(0),
        }],
        ..Default::default()
    };
    let sid = hale_model::SubjectId(0);
    let publish = |domain: Option<KeyDomain>| hale_model::Publish {
        function: hale_model::FunctionId(0),
        subject: sid,
        declared_topic: None,
        payload: hale_model::PayloadContractId(0),
        site: 0,
        in_loop: false,
        key_domain: domain,
        disposition: hale_model::keys::PublishDisposition::Default,
        provenance: hale_model::ProvenanceId(0),
    };
    let sub = |pred: KeyPredicate| hale_model::Subscribe {
        subject: sid,
        declared_topic: None,
        payload: hale_model::PayloadContractId(0),
        handler: hale_model::FunctionId(0),
        site: 0,
        key_predicate: pred,
        capacity: hale_model::keys::Capacity::Unbounded,
        shed: hale_model::keys::ShedPolicy::None,
        provenance: hale_model::ProvenanceId(0),
    };
    let exact_one =
        Some(KeyDomain::Exact(vec![KeyValue::Int(1)]));
    assert!(
        !hale_types::judgment::may_deliver(
            &e,
            &publish(exact_one.clone()),
            &sub(KeyPredicate::EqLiteral(KeyValue::Int(2)))
        ),
        "an exact key set of {{1}} never reaches a `key == 2` filter"
    );
    assert!(hale_types::judgment::may_deliver(
        &e,
        &publish(exact_one.clone()),
        &sub(KeyPredicate::EqLiteral(KeyValue::Int(1)))
    ));
    // Ranges decide the same way…
    assert!(!hale_types::judgment::may_deliver(
        &e,
        &publish(Some(KeyDomain::IntRange { min: 0, max: 3 })),
        &sub(KeyPredicate::EqLiteral(KeyValue::Int(9)))
    ));
    // …and everything unknown widens: an unknown domain, an
    // unkeyed subscription, a replica filter, an unknown filter.
    assert!(hale_types::judgment::may_deliver(
        &e,
        &publish(Some(KeyDomain::Unknown)),
        &sub(KeyPredicate::EqLiteral(KeyValue::Int(2)))
    ));
    assert!(hale_types::judgment::may_deliver(
        &e,
        &publish(exact_one.clone()),
        &sub(KeyPredicate::Any)
    ));
    assert!(hale_types::judgment::may_deliver(
        &e,
        &publish(exact_one),
        &sub(KeyPredicate::EqReplica)
    ));
}

/// A matching literal filter delivers — the keyed path is not an
/// unconditional "keyed means no edge".
#[test]
fn a_matching_key_filter_creates_the_edge() {
    let (v, _) = judge(
        r#"
type Msg { shard: Int = 0; }
topic T { payload: Msg; subject: "t"; keyed_by shard; }
locus Sink {
    params { n: Int = 0; }
    bus { subscribe T as on_t where key == 1; }
    fn on_t(m: Msg) { println("io"); }
}
locus Source {
    bus { publish T; }
    @effects(causes: { publish, alloc })
    fn fire() { T <- Msg { shard: 1 }; }
}
main locus App {
    params { s: Sink = Sink { }; p: Source = Source { }; }
    run() { self.p.fire(); }
}
fn main() { App { }; }
"#,
    );
    assert_eq!(v, Verdict::Violated);
}

/// Review round 3, blocker 2: a binding on the PUBLISHED topic is
/// an opaque boundary. The publish leaves the program; whatever the
/// adapter or its peer does is not in this graph, so the law cannot
/// be certified — the mirror of the unrelated-binding control,
/// which must stay `Holds`.
#[test]
fn a_binding_on_the_published_topic_is_uncertified() {
    let (v, _) = judge(
        r#"
type Msg { n: Int = 0; }
topic T { payload: Msg; subject: "t"; }
locus MyAdapter {
    params { n: Int = 0; }
    fn send(subject: String, bytes: Bytes) { self.n = self.n + 1; }
}
locus Source {
    bus { publish T; }
    @effects(causes: { publish, alloc })
    fn fire() { T <- Msg { n: 1 }; }
}
main locus App {
    params { p: Source = Source { }; }
    bindings { T: MyAdapter { }; }
    run() { self.p.fire(); }
}
fn main() { App { }; }
"#,
    );
    assert_eq!(
        v,
        Verdict::Uncertified,
        "the publish crosses an opaque external boundary"
    );
}

/// Review round 3, blocker 1: publish SITES survive the delivery
/// query. Two sites on one subject with different exact key
/// domains — only one of which reaches the `key == 2` subscriber —
/// must not collapse before `may_deliver` runs.
///
/// The builder records `AnyOfType` for a literal send, so the exact
/// domains are set on the derived model (a lawful state `validate`
/// accepts) to reach the shape the engine must handle.
#[test]
fn publish_sites_survive_the_delivery_query() {
    use hale_model::keys::{KeyDomain, KeyValue};
    let src = r#"
type Msg { shard: Int = 0; }
topic T { payload: Msg; subject: "t"; keyed_by shard; }
locus Sink {
    params { n: Int = 0; }
    bus { subscribe T as on_t where key == 2; }
    fn on_t(m: Msg) { println("io"); }
}
locus Source {
    bus { publish T; }
    @effects(causes: { publish, alloc })
    fn fire() {
        T <- Msg { shard: 1 };
        T <- Msg { shard: 2 };
    }
}
main locus App {
    params { s: Sink = Sink { }; p: Source = Source { }; }
    run() { self.p.fire(); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    // Two sites, distinct exact domains: site 0 produces key 1
    // (never delivered), site 1 produces key 2 (delivered).
    let mut sites: Vec<usize> = model
        .relations
        .publishes
        .iter()
        .enumerate()
        .filter(|(_, p)| p.key_domain.is_some())
        .map(|(i, _)| i)
        .collect();
    sites.sort();
    assert_eq!(
        sites.len(),
        2,
        "fixture premise: two keyed publish sites"
    );
    model.relations.publishes[sites[0]].key_domain =
        Some(KeyDomain::Exact(vec![KeyValue::Int(1)]));
    model.relations.publishes[sites[1]].key_domain =
        Some(KeyDomain::Exact(vec![KeyValue::Int(2)]));
    model.validate().expect("still a lawful model");

    let table = hale_types::claim_lowering::lower_claims(&bundle, &model);
    let bases: Vec<u32> = bundle.sources.iter().map(|f| f.base).collect();
    let judged =
        hale_types::judgment::judge_causes(&table, &model, &bases);
    assert_eq!(
        judged.first().expect("a row").verdict,
        Verdict::Violated,
        "the site that delivers was collapsed away before the \
         delivery query ran"
    );
}

/// The evaluator's root walk does not recurse into modules, so a
/// module-scoped `causes:` annotation produces no outcome from it at
/// all — the documented rule that unmigrated rows bridge to the old
/// engines only where the old walk demonstrably enumerated them.
///
/// This is why the corpus differential SKIPS such rows rather than
/// treating a missing outcome as `holds`: comparing against silence
/// that means "never looked" would let anything through.
#[test]
fn a_module_scoped_row_has_no_evaluator_outcome() {
    let src = r#"
effect money;
module billing {
    @effects(causes: { money })
    fn poke(v: Int) -> Int { return v; }
}
main locus App {
    params { n: Int = 0; }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let (top, _) = hale_types::resolve::build_top_scope(&bundle);
    let graph = hale_types::bus_graph::build_bus_graph(&bundle, &top);
    let reports = hale_types::frontier::causes_reports(
        &vec![&program],
        &graph,
    );
    assert!(
        reports.is_empty(),
        "fixture premise: the evaluator never enumerates a \
         module-scoped annotation: {:?}",
        reports.iter().map(|r| &r.function).collect::<Vec<_>>()
    );
    // …while the model lowers the row and judges it.
    let model = derive_application_model(&bundle);
    let table = hale_types::claim_lowering::lower_claims(&bundle, &model);
    let bases: Vec<u32> = bundle.sources.iter().map(|f| f.base).collect();
    assert_eq!(
        hale_types::judgment::judge_causes(&table, &model, &bases).len(),
        1,
        "the model judges what the evaluator could not see"
    );
}

/// Review round 4, blocker 1: an ordinary typed outbound binding
/// takes the causal closure out of the application. The transport
/// is fully modeled — no hole — but the peer's behaviour is not
/// here, so the law cannot be certified.
#[test]
fn a_connect_binding_leaves_the_application() {
    let (v, _) = judge(
        r#"
type Msg { n: Int = 0; }
topic T { payload: Msg; subject: "t"; }
locus Source {
    bus { publish T; }
    @effects(causes: { publish, alloc })
    fn fire() { T <- Msg { n: 1 }; }
}
main locus App {
    params { p: Source = Source { }; }
    bindings { T: unix("/tmp/t.sock", role: connect); }
    run() { self.p.fire(); }
}
fn main() { App { }; }
"#,
    );
    assert_eq!(
        v,
        Verdict::Uncertified,
        "a connect-role send is handed to a peer this model does \
         not contain"
    );
}

/// Review round 4, blocker 2: two `causes:` clauses on ONE function
/// share a span, so a span cannot be the join key. One holds and one
/// is violated; each model row must be matched to its OWN evaluator
/// outcome.
#[test]
fn two_clauses_on_one_fn_are_judged_separately() {
    let src = r#"
type Msg { n: Int = 0; }
topic T { payload: Msg; subject: "t"; }
locus Sink {
    params { n: Int = 0; }
    bus { subscribe T as on_t; }
    fn on_t(m: Msg) { println("io"); }
}
locus Source {
    bus { publish T; }
    @effects(causes: { syscall, publish, alloc }, causes: { publish })
    fn fire() { T <- Msg { n: 1 }; }
}
main locus App {
    params { s: Sink = Sink { }; p: Source = Source { }; }
    run() { self.p.fire(); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let (top, _) = hale_types::resolve::build_top_scope(&bundle);
    let graph = hale_types::bus_graph::build_bus_graph(&bundle, &top);
    let reports =
        hale_types::frontier::causes_reports(&vec![&program], &graph);
    assert_eq!(
        reports.len(),
        2,
        "fixture premise: two assertions on one fn: {:?}",
        reports.iter().map(|r| (&r.function, r.ordinal)).collect::<Vec<_>>()
    );
    // Same span on both — which is exactly why the ordinal is the
    // identity.
    let spans: Vec<_> = reports
        .iter()
        .filter_map(|r| r.diag.as_ref().map(|d| d.span))
        .collect();
    assert!(spans.windows(2).all(|w| w[0] == w[1]));
    // The first clause covers syscall and holds; the second does not.
    assert!(reports[0].diag.is_none(), "clause 0 holds");
    assert!(reports[1].diag.is_some(), "clause 1 is violated");

    let model = derive_application_model(&bundle);
    let table = hale_types::claim_lowering::lower_claims(&bundle, &model);
    let bases: Vec<u32> = bundle.sources.iter().map(|f| f.base).collect();
    let judged =
        hale_types::judgment::judge_causes(&table, &model, &bases);
    assert_eq!(judged.len(), 2, "the model lowers both clauses");
    assert_eq!(judged[0].verdict, Verdict::Holds);
    assert_eq!(judged[1].verdict, Verdict::Violated);
}

/// Review round 5: the route query joins on the WIRE, not on the
/// syntactic declaration link. A literal `"t" <- …` send carries
/// `declared_topic: None` even though its text is the bound topic's
/// wire subject, and after lowering the runtime cannot tell the two
/// spellings apart — the send reaches the binding installed for
/// that wire.
#[test]
fn a_literal_send_into_a_connect_bound_wire_leaves_the_application() {
    let (v, _) = judge(
        r#"
type Msg { n: Int = 0; }
topic T { payload: Msg; subject: "t"; }
locus Source {
    bus { publish "t" of type Msg; }
    @effects(causes: { publish, alloc })
    fn fire() { "t" <- Msg { n: 1 }; }
}
main locus App {
    params { p: Source = Source { }; }
    bindings { T: unix("/tmp/t.sock", role: connect); }
    run() { self.p.fire(); }
}
fn main() { App { }; }
"#,
    );
    assert_eq!(
        v,
        Verdict::Uncertified,
        "a literal send on the bound wire reaches the peer just as \
         a topic-spelled one does"
    );
}

/// …and the same for an opaque adapter boundary, whose hole is
/// anchored at the TOPIC while the publish names the wire.
#[test]
fn a_literal_send_into_an_adapter_bound_wire_is_uncertified() {
    let (v, _) = judge(
        r#"
type Msg { n: Int = 0; }
topic T { payload: Msg; subject: "t"; }
locus MyAdapter {
    params { n: Int = 0; }
    fn send(subject: String, bytes: Bytes) { self.n = self.n + 1; }
}
locus Source {
    bus { publish "t" of type Msg; }
    @effects(causes: { publish, alloc })
    fn fire() { "t" <- Msg { n: 1 }; }
}
main locus App {
    params { p: Source = Source { }; }
    bindings { T: MyAdapter { }; }
    run() { self.p.fire(); }
}
fn main() { App { }; }
"#,
    );
    assert_eq!(
        v,
        Verdict::Uncertified,
        "the ExternalOpaque hole is anchored at the topic; the \
         publish names the wire; they are one address"
    );
}

/// Review pin (round 3): an `Uncertified` verdict is never SILENT.
///
/// `claim_law_diags` appends diagnostics, never verdicts. A row that
/// could not be certified and carried no diagnostic therefore
/// compiled clean while the artifact marked the document
/// `law_failed` — the exact check/artifact disagreement this epic
/// removes — and left the row with no evidence, which admission
/// then refused.
#[test]
fn an_uncertified_causes_row_explains_itself() {
    let src = r#"
type T { n: Int = 0; }
topic Settled { payload: T; subject: "settled"; }
locus Ledger {
    bus { subscribe Settled as on_settled; }
    params { n: Int = 0; }
    fn on_settled(t: T) { self.n = apply(bump, t.n); }
}
fn bump(v: Int) -> Int { return v + 1; }
// An INDIRECT call: its target is chosen by the caller, so what the
// handler does is not knowable here.
fn apply(f: fn(Int) -> Int, v: Int) -> Int { return f(v); }
main locus App {
    params { l: Ledger = Ledger { }; }
    bus { publish Settled; }
    @effects(causes: { publish })
    fn fire() { Settled <- T { n: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let model = derive_application_model(&bundle);
    let table = hale_types::claim_lowering::lower_claims(&bundle, &model);
    let judged =
        hale_types::judgment::judge_causes(&table, &model, &[0]);
    let row = judged
        .iter()
        .find(|j| j.verdict != Verdict::Holds)
        .unwrap_or_else(|| {
            panic!(
                "expected a non-holds causes row, got {:?}",
                judged.iter().map(|j| j.verdict).collect::<Vec<_>>()
            )
        });
    assert!(
        !row.diags.is_empty(),
        "a non-holds row must say why — silence is what let check \
         and the artifact disagree"
    );

    // …and the check path carries it, so the program is not clean.
    let check = hale_types::check_bundle_opts(&bundle, false);
    assert!(
        check.iter().any(|d| d.message.contains("causal set")),
        "check reports the law: {:?}",
        check.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}
