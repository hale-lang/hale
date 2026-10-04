//! F.40 phase 3, C7: the laws that replaced lowering's backstops hold
//! at the harness too.
//!
//! `build_executable_with_options` builds through a snapshot whose
//! lowering is not gated on the check (`Config::harness`). Lowering
//! used to keep a spanless refusal of its own for each rule below,
//! because a harness build reached it unchecked; the refusals are
//! deleted, and the harness's lowering view demands the laws instead
//! (`hale_types::lowering_laws`). So a harness build of a program a law
//! refuses is refused before lowering, with the law's wording, and
//! lowering judges none of them.

use hale_codegen::build_executable_with_options;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// The harness's refusal of `src`, rendered.
fn refusal(tag: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(tag);
    let err = build_executable_with_options(&program, &bin, &[], &build_opts::options())
        .expect_err("a program a lowering law refuses must not build");
    let _ = std::fs::remove_file(&bin);
    err.to_string()
}

/// Rule 6: a pinned locus that accepts children.
#[test]
fn rule_6_a_pinned_locus_that_accepts_is_refused_by_the_law() {
    let msg = refusal(
        "hale_c7_rule6_accept",
        "locus Child { run() { } }\n\
         locus Coord { accept(c: Child) { } run() { } }\n\
         main locus App {\n\
             params { w: Coord = Coord { }; }\n\
             placement { w: pinned; }\n\
         }\n\
         fn main() { App { }; }\n",
    );
    assert!(
        msg.contains("placement entry `w`: `Coord` is placed `pinned` but declares `accept()`")
            && msg.contains("(rule 6)"),
        "expected the rule 6 law's refusal, got: {msg}"
    );
}

/// Rule 6: a pinned locus with a dissolve closure (the default epoch).
#[test]
fn rule_6_a_pinned_locus_with_a_dissolve_closure_is_refused_by_the_law() {
    let msg = refusal(
        "hale_c7_rule6_closure",
        "locus Worker {\n\
             params { n: Int = 0; }\n\
             closure settled { self.n ~~ self.n within 0; }\n\
             run() { }\n\
         }\n\
         main locus App {\n\
             params { w: Worker = Worker { }; }\n\
             placement { w: pinned; }\n\
         }\n\
         fn main() { App { }; }\n",
    );
    assert!(
        msg.contains("placement entry `w`") && msg.contains("dissolve is the default") && msg.contains("(rule 6)"),
        "expected the rule 6 law's refusal, got: {msg}"
    );
}

/// Rule 6: an adapter inline in `bindings { }`, which no placement entry
/// names, runs pinned too.
#[test]
fn rule_6_an_adapter_binding_that_accepts_is_refused_by_the_law() {
    let msg = refusal(
        "hale_c7_rule6_adapter",
        "type Tick { n: Int; }\n\
         topic Beat { payload: Tick; subject: \"beat\"; }\n\
         locus Child { run() { } }\n\
         locus Sink { accept(c: Child) { } fn send(subject: String, bytes: Bytes) { } }\n\
         locus Pub { bus { publish Beat; } run() { Beat <- Tick { n: 1 }; } }\n\
         main locus App {\n\
             params { p: Pub = Pub { }; }\n\
             bindings { Beat: Sink { }; }\n\
         }\n\
         fn main() { App { }; }\n",
    );
    assert!(
        msg.contains("adapter binding for topic `Beat`: `Sink` runs on its own pinned thread") && msg.contains("(rule 6)"),
        "expected the rule 6 law's refusal at the binding entry, got: {msg}"
    );
}

/// The cross-pool spawn (`Driver` on pool `workers`, `World` a singleton
/// on main that accepts `Ship`), with `body` as `Driver.run()`'s. Spelled
/// without a raw string so the corpus does not harvest it.
fn crosspool_src(body: &str) -> String {
    [
        "locus Ship { params { hull: Int = 0; } }\n",
        "locus Holder { params { s: Ship = Ship { hull: 1 }; } }\n",
        "locus Driver { run() { ",
        body,
        " } }\n",
        "main locus World {\n",
        "    params { driver: Driver = Driver { }; }\n",
        "    placement { driver: cooperative(pool = workers); }\n",
        "    accept(s: Ship) { }\n",
        "    run() { }\n",
        "}\n",
        "fn main() { World { }; }\n",
    ]
    .concat()
}

fn build_err(tag: &str, src: &str) -> hale_codegen::CodegenError {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(tag);
    let err = build_executable_with_options(&program, &bin, &[], &build_opts::options())
        .expect_err("the program must not build");
    let _ = std::fs::remove_file(&bin);
    err
}

const FIRE_AND_FORGET: &str = "cross-pool spawn `Ship{ }` is fire-and-forget";

/// The cross-pool spawn law: a value use in the enclosing locus's own
/// body is refused before lowering. Two value uses tell the law from a
/// lowering refusal: the law judges every literal and the harness joins
/// its refusals, where lowering stops at the first.
#[test]
fn a_cross_pool_spawn_used_as_a_value_is_refused_by_the_law() {
    let err = build_err(
        "hale_c7_xpool_value",
        &crosspool_src("let a = Ship { hull: 7 }; let b = Ship { hull: 8 };"),
    );
    let msg = err.to_string();
    assert_eq!(msg.matches(FIRE_AND_FORGET).count(), 2, "expected the law's two refusals, got: {msg}");
}

/// The residue lowering keeps: `Driver` spawns `Ship` itself (a bare
/// statement, legal), so the plan holds (Driver, Ship); `Holder`'s
/// params default builds a `Ship` too, expanded under `Driver`'s self,
/// which no row relates to the literal. Lowering's own refusal is still
/// its only evaluator (the check passes it: hale-types'
/// `ownership_graph.rs`).
#[test]
fn a_cross_pool_spawn_in_another_locus_default_is_refused_by_lowering_alone() {
    let err = build_err("hale_c7_xpool_default", &crosspool_src("Ship { hull: 7 }; Holder { };"));
    let msg = err.to_string();
    assert!(
        msg.starts_with("unsupported in codegen v0: cross-pool spawn `Ship{ }` is fire-and-forget"),
        "expected lowering's own refusal, got: {msg}"
    );
}

/// Two aliased value uses must be rejected by the shared law before
/// lowering gets to the first one, with the resolved child's plan.
#[test]
fn aliased_cross_pool_births_are_refused_together_by_the_law() {
    let src = format!("type Vessel = Ship;\n{}", crosspool_src("let a = Vessel { hull: 7 }; let b = Vessel { hull: 8 };"));
    let err = build_err("hale_c3_xpool_alias", &src);
    let msg = err.to_string();
    assert_eq!(msg.matches(FIRE_AND_FORGET).count(), 2, "expected the law's two refusals, got: {msg}");
}
