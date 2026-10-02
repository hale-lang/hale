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
        msg.contains("binding entry `Beat`: adapter `Sink` runs pinned") && msg.contains("(rule 6)"),
        "expected the rule 6 law's refusal at the binding entry, got: {msg}"
    );
}
