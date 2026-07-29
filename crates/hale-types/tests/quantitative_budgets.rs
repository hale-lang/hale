//! GH #265 step 5 — the quantitative layer.

fn diags_for(src: &str) -> Vec<String> {
    let program = hale_syntax::parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

/// Stack depth is a DAG longest path: the deepest CHAIN, not the sum
/// of all callees (a fn calling two 100-byte helpers uses one at a
/// time).
#[test]
fn stack_bytes_measures_the_deepest_chain_not_the_sum() {
    let src = r#"
        fn leaf(a: Int) -> Int { return a + 1; }
        fn left(a: Int) -> Int { return leaf(a); }
        fn right(a: Int) -> Int { return leaf(a); }
        @budget(stack_bytes = 4096) fn wide(a: Int) -> Int {
            return left(a) + right(a);
        }
        fn main() { println(wide(1)); }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("stack_bytes")),
        "a shallow diamond must fit a 4KB budget: {:?}",
        ds
    );
}

#[test]
fn stack_bytes_rejects_a_deep_chain_against_a_tiny_budget() {
    let src = r#"
        fn d(a: Int) -> Int { return a; }
        fn c(a: Int) -> Int { return d(a); }
        fn b(a: Int) -> Int { return c(a); }
        @budget(stack_bytes = 48) fn a_(a: Int) -> Int { return b(a); }
        fn main() { println(a_(1)); }
    "#;
    let ds = diags_for(src);
    assert!(
        ds.iter().any(|m| m.contains("stack_bytes")
            && m.contains("budget exceeded")),
        "a 4-deep chain must exceed a 48-byte budget: {:?}",
        ds
    );
}

/// Recursion makes the bound unbounded — which is why the issue
/// pairs this dimension with `@no_recursion`.
#[test]
fn stack_bytes_is_unbounded_under_recursion() {
    let src = r#"
        fn down(n: Int) -> Int {
            if n <= 0 { return 0; }
            return down(n - 1);
        }
        @budget(stack_bytes = 65536) fn entry() -> Int { return down(5); }
        fn main() { println(entry()); }
    "#;
    let ds = diags_for(src);
    assert!(
        ds.iter().any(|m| m.contains("stack_bytes")
            && m.contains("unbounded")),
        "recursion must report an unbounded stack: {:?}",
        ds
    );
}

/// `@budget(publish = 1)` IS the issue's `@replies` — exactly-once
/// reply per delivery, as a count.
#[test]
fn publish_budget_is_exactly_once_reply() {
    let src = r#"
        type Ev { n: Int; }
        topic Out { payload: Ev; subject: "out"; }
        locus H {
            bus { publish Out; }
            @budget(publish = 1) fn reply_once(n: Int) {
                Out <- Ev { n: n };
            }
            @budget(publish = 1) fn reply_twice(n: Int) {
                Out <- Ev { n: n };
                Out <- Ev { n: n + 1 };
            }
        }
        fn main() { H { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("H::reply_once")),
        "one publish must satisfy publish = 1: {:?}",
        ds
    );
    assert!(
        ds.iter().any(|m| m.contains("H::reply_twice")
            && m.contains("publish")),
        "two publishes must exceed publish = 1: {:?}",
        ds
    );
}

/// A publish inside a loop is unbounded per call.
#[test]
fn publish_in_a_loop_is_unbounded() {
    let src = r#"
        type Ev { n: Int; }
        topic Out { payload: Ev; subject: "out"; }
        locus H {
            bus { publish Out; }
            @budget(publish = 4) fn spray(n: Int) {
                let mut i = 0;
                while i < n {
                    Out <- Ev { n: i };
                    i = i + 1;
                }
            }
        }
        fn main() { H { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        ds.iter().any(|m| m.contains("publish") && m.contains("unbounded")),
        "a loop publish must saturate: {:?}",
        ds
    );
}

/// Fan-out counts transitive subscriber DELIVERIES — the
/// amplification property no per-fn count reveals.
#[test]
fn fanout_counts_subscriber_amplification() {
    let src = r#"
        type Ev { n: Int; }
        topic Out { payload: Ev; subject: "out"; }
        locus S1 { bus { subscribe Out as on_o; } fn on_o(e: Ev) { } }
        locus S2 { bus { subscribe Out as on_o; } fn on_o(e: Ev) { } }
        locus S3 { bus { subscribe Out as on_o; } fn on_o(e: Ev) { } }
        locus P {
            bus { publish Out; }
            @budget(fanout = 2) fn emit(n: Int) {
                Out <- Ev { n: n };
            }
        }
        fn main() { S1 { }; S2 { }; S3 { }; P { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        ds.iter().any(|m| m.contains("fanout")
            && m.contains("budget exceeded")),
        "one publish to a 3-subscriber subject must exceed fanout = 2: {:?}",
        ds
    );
}

/// `block_points` is the counted form of `@no_block` — `0` is the
/// assertion, `1` is "may wait once".
#[test]
fn block_points_bounds_waiting() {
    let src = r#"
        fn wait_once() { std::time::sleep(5ms); }
        @budget(block_points = 1) fn ok_once() { wait_once(); }
        @budget(block_points = 1) fn too_many() {
            wait_once();
            wait_once();
        }
        fn main() { ok_once(); too_many(); }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("`ok_once`")),
        "one blocking point must satisfy block_points = 1: {:?}",
        ds
    );
    assert!(
        ds.iter().any(|m| m.contains("`too_many`")
            && m.contains("block_points")),
        "two blocking points must exceed the budget: {:?}",
        ds
    );
}

/// Dimensions compose in one clause, with alloc_per_call.
#[test]
fn dimensions_compose_in_one_budget_clause() {
    let src = r#"
        fn pure_math(n: Int) -> Int { return n * 2; }
        @budget(alloc_per_call = 0, stack_bytes = 4096, block_points = 0)
        fn tick(n: Int) -> Int { return pure_math(n); }
        fn main() { println(tick(2)); }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("budget")),
        "a clean fn must satisfy all three dimensions: {:?}",
        ds
    );
}
