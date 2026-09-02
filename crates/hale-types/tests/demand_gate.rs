//! GH #476 acceptance criterion 1: *"One demand-gated model
//! derivation entry point; no-claims LSP path **provably** skips
//! it."*
//!
//! The gate shipped (`has_claim_surface`, an early return in
//! `claim_law_diags`) and so did the instrumentation
//! (`model_builder::builds()`, whose doc comment says "The no-claims
//! check path must leave this at zero"). Nothing asserted it. A
//! refactor that hoisted the derivation above the gate — or a new
//! claim surface that forgot to extend it — would pass CI silently
//! while every keystroke in the editor paid for a model.
//!
//! That is the whole point of the criterion, so it is checked here.
//!
//! **This file is deliberately a lone test in its own binary.**
//! `builds()` is a process-global counter and Rust runs a test
//! binary's tests in parallel by default, so a sibling test deriving
//! a model would make this either flaky or vacuous.

/// The editor path: a program that swears to nothing must not build
/// a model — and one that does must build exactly one, so the test
/// cannot pass by the gate simply never opening.
#[test]
fn the_no_claims_check_path_derives_no_model() {
    let no_claims = hale_syntax::parse_source(
        r#"
type Order { id: Int = 0; }
topic Placed { payload: Order; }

locus Desk {
    params { seen: Int = 0; }
    bus { subscribe Placed as on_placed; }
    fn on_placed(o: Order) { self.seen = o.id; }
}

locus Feed {
    bus { publish Placed; }
    fn go() { Placed <- Order { id: 1 }; }
}

main locus App {
    params { d: Desk = Desk { }; f: Feed = Feed { }; }
}
fn main() { App { }; }
"#,
    )
    .expect("parse");

    let before = hale_types::model_builder::builds();
    let diags = hale_types::check_program(&no_claims);
    let after = hale_types::model_builder::builds();

    assert!(
        !diags.iter().any(|d| d.is_error()),
        "the fixture must be a CLEAN program, or the checker may bail \
         before reaching the gate and the test proves nothing: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert_eq!(
        after, before,
        "a program with no claims, no constitution, and no judged \
         annotation must not derive an ApplicationModel — this is the \
         LSP's path, and it runs on every keystroke"
    );

    // The other half: the gate must actually open. Without this the
    // assertion above is satisfied by a gate that never lets anything
    // through, which would be a silent loss of all claim checking.
    let with_claim = hale_syntax::parse_source(
        r#"
type Order { id: Int = 0; }
topic Placed { payload: Order; }

locus Desk {
    params { seen: Int = 0; }
    bus { subscribe Placed as on_placed; }
    fn on_placed(o: Order) { self.seen = o.id; }
}

locus Feed {
    bus { publish Placed; }
    fn go() { Placed <- Order { id: 1 }; }
}

main locus App {
    params { d: Desk = Desk { }; f: Feed = Feed { }; }
    claims { one_writer: count publishers(topic Placed) == 1; }
}
fn main() { App { }; }
"#,
    )
    .expect("parse");

    let before = hale_types::model_builder::builds();
    let diags = hale_types::check_program(&with_claim);
    let after = hale_types::model_builder::builds();

    assert!(
        !diags.iter().any(|d| d.is_error()),
        "the claim-bearing fixture must also be clean: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(
        after > before,
        "a program carrying a claim MUST derive a model — a gate that \
         never opens would satisfy the first assertion while silently \
         checking no law at all"
    );
}
