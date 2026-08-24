//! GH #476 Change 5h — the model's per-call COST facts.
//!
//! `@budget` is the last law answered by engines of its own, and
//! those engines read an analysis the model did not carry:
//! allocation sites, frame sizes, blocking points. These controls
//! pin the facts themselves — that they exist, that they are
//! SITE-grained, and that the loop flag survives — before anything
//! judges over them. A per-call budget is a statement about one
//! invocation, so a site collapsed into a per-function total has
//! already lost the thing that decides the verdict.

use std::collections::BTreeMap;

use hale_model::CostDimension;
use hale_types::model_builder::derive_application_model;
use hale_types::symbol::SourceFile;
use hale_types::Bundle;

fn model_of(src: &str) -> hale_model::ApplicationModel {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), &program);
    let mut b = Bundle::new(programs);
    b.sources = vec![SourceFile {
        id: 0,
        path: "app.hl".to_string(),
        digest: "0".to_string(),
        base: 0,
        len: src.len() as u32,
    }];
    let b = b;
    derive_application_model(&b)
}

fn sites(
    m: &hale_model::ApplicationModel,
    f: &str,
    dim: CostDimension,
) -> Vec<hale_model::CostSite> {
    let id = m
        .entities
        .functions
        .iter()
        .position(|x| x.display == f)
        .unwrap_or_else(|| panic!("no function `{}`", f));
    m.relations
        .costs
        .iter()
        .filter(|c| c.function.0 as usize == id && c.dimension == dim)
        .cloned()
        .collect()
}

const SRC: &str = r#"
type T { n: Int = 0; }
fn straight() -> T { return T { n: 1 }; }
fn twice() -> Int { let a = T { n: 1 }; let b = T { n: 2 }; return a.n + b.n; }
fn looped() -> Int {
    let total = 0;
    for i in 0..4 { let t = T { n: i }; total = total + t.n; }
    return total;
}
fn quiet(v: Int) -> Int { return v + 1; }
main locus App {
    params { n: Int = 0; }
    run() { self.n = straight().n + twice() + looped() + quiet(1); }
}
fn main() { App { }; }
"#;

#[test]
fn allocation_sites_are_recorded_per_site() {
    let m = model_of(SRC);
    assert_eq!(sites(&m, "straight", CostDimension::Alloc).len(), 1);
    assert_eq!(
        sites(&m, "twice", CostDimension::Alloc).len(),
        2,
        "two literals, two sites — not one row carrying a total"
    );
    assert!(
        sites(&m, "quiet", CostDimension::Alloc).is_empty(),
        "a fn that allocates nothing contributes no site"
    );
}

#[test]
fn the_loop_flag_survives_into_the_model() {
    // The whole reason the facts are site-grained: this is what
    // turns a finite per-call count into an unbounded one, and a
    // per-function total would have erased it.
    let m = model_of(SRC);
    let looped = sites(&m, "looped", CostDimension::Alloc);
    assert_eq!(looped.len(), 1);
    assert!(
        looped[0].in_loop,
        "the allocation is loop-nested: {:?}",
        looped
    );
    let straight = sites(&m, "straight", CostDimension::Alloc);
    assert!(!straight[0].in_loop);
}

#[test]
fn every_analyzed_function_is_charged_a_frame() {
    let m = model_of(SRC);
    for name in ["straight", "twice", "looped", "quiet"] {
        let frames = sites(&m, name, CostDimension::FrameBytes);
        assert_eq!(
            frames.len(),
            1,
            "`{}` must carry exactly one frame estimate",
            name
        );
        assert!(
            frames[0].amount >= 32,
            "the estimate includes call overhead: {:?}",
            frames[0]
        );
        assert!(
            !frames[0].in_loop,
            "a frame is charged once per call — the same frame is \
             reused across iterations"
        );
    }
}

#[test]
fn a_wider_frame_costs_more_than_a_narrow_one() {
    // The estimate must actually vary with the declared shape, or
    // `stack_bytes` would be judging a constant.
    let m = model_of(SRC);
    let twice = sites(&m, "twice", CostDimension::FrameBytes)[0].amount;
    let quiet = sites(&m, "quiet", CostDimension::FrameBytes)[0].amount;
    assert!(
        twice > quiet,
        "two locals must cost more than none: {} vs {}",
        twice,
        quiet
    );
}

#[test]
fn cost_sites_are_sorted_canonically() {
    let m = model_of(SRC);
    let mut sorted = m.relations.costs.clone();
    sorted.sort_by(|a, b| {
        (a.function.0, a.dimension, a.provenance.0)
            .cmp(&(b.function.0, b.dimension, b.provenance.0))
    });
    assert_eq!(
        m.relations.costs, sorted,
        "the table is part of the model's identity — it must come \
         out in canonical order, not summary-walk order"
    );
}

/// Review pin (round 2): `Block` was in the cost vocabulary and
/// nothing emitted it, while `exact_costs` positively claimed the
/// account was complete for every analyzed function.
#[test]
fn a_blocking_call_is_charged_a_block_site() {
    let src = r#"
fn waits() { std::time::sleep(1ms); }
fn quiet(v: Int) -> Int { return v + 1; }
main locus App {
    params { n: Int = 0; }
    run() { waits(); self.n = quiet(1); }
}
fn main() { App { }; }
"#;
    let m = model_of(src);
    let blocks = sites(&m, "waits", CostDimension::Block);
    assert_eq!(
        blocks.len(),
        1,
        "one blocking call, one block site: {:?}",
        blocks
    );
    assert!(!blocks[0].in_loop);
    assert!(
        sites(&m, "quiet", CostDimension::Block).is_empty(),
        "a pure fn blocks nowhere"
    );
}

#[test]
fn a_loop_nested_blocking_call_keeps_its_loop_flag() {
    let src = r#"
fn pump() {
    let mut n = 0;
    while n < 4 { std::time::sleep(1ms); n = n + 1; }
}
main locus App {
    params { n: Int = 0; }
    run() { pump(); }
}
fn main() { App { }; }
"#;
    let m = model_of(src);
    let blocks = sites(&m, "pump", CostDimension::Block);
    assert_eq!(blocks.len(), 1);
    assert!(
        blocks[0].in_loop,
        "a per-call bound cannot survive the loop: {:?}",
        blocks
    );
}

#[test]
fn an_unfollowable_call_withdraws_the_cost_account() {
    // The positive capability must not outlive the knowledge: an
    // indirect call may reach an allocation or a blocking call, and
    // the model cannot see past it.
    let src = r#"
fn apply(f: fn(Int) -> Int, v: Int) -> Int { return f(v); }
main locus App {
    params { n: Int = 0; }
    fn dbl(v: Int) -> Int { return v * 2; }
    run() { self.n = apply(self.dbl, 1); }
}
fn main() { App { }; }
"#;
    let m = model_of(src);
    assert!(
        !m.capabilities.exact_costs,
        "an indirect call hides COSTS, so the account is degraded"
    );
}
