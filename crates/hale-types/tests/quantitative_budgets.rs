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

/// A repeated dimension used to overwrite silently: writing
/// `@budget(alloc_per_call = 0, alloc_per_call = 5)` enforced **5**.
/// The author asked for a zero-alloc certificate and got a ceiling
/// of five, with nothing said. Whichever way precedence fell would
/// be a guess — the annotation is ambiguous, so it is rejected.
#[test]
fn a_repeated_budget_dimension_is_rejected() {
    for src in [
        "@budget(alloc_per_call = 0, alloc_per_call = 5)\nfn f() -> Int { return 1; }\nfn main() { println(f()); }",
        "@budget(stack_bytes = 16, stack_bytes = 32)\nfn f() -> Int { return 1; }\nfn main() { println(f()); }",
    ] {
        let err = hale_syntax::parse_source(src)
            .err()
            .unwrap_or_else(|| panic!("expected a parse error for:\n{}", src));
        let msg = err
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(
            msg.contains("given twice"),
            "expected a duplicate-dimension diagnostic, got: {}",
            msg
        );
    }
}

/// …but distinct dimensions in one clause are the documented shape
/// and must keep working.
#[test]
fn distinct_budget_dimensions_still_compose() {
    let src = "@budget(alloc_per_call = 0, stack_bytes = 4096, block_points = 0)\n\
               fn f() -> Int { return 1; }\nfn main() { println(f()); }";
    assert!(
        hale_syntax::parse_source(src).is_ok(),
        "a multi-dimension budget is the canonical hot-path certificate"
    );
}

/// `@budget(alloc_per_call = 0)` must count string concatenation.
///
/// It didn't. A fn doing `"x" + a + "y"` performs **34 heap
/// allocations** (measured with `std::diag::heap_alloc_count`) and
/// passed a zero-allocation certificate clean. That is a fail-open in
/// a contract: worse than no certificate, because it reads as proof.
///
/// Detection is deliberately narrow — an operand must be provably a
/// String (a literal, or a name whose DECLARED type is String).
/// Flagging every `i + 1` would be the cry-wolf failure the alloc
/// pass exists to avoid, which is why this was originally deferred.
#[test]
fn budget_counts_string_concatenation() {
    let src = "@budget(alloc_per_call = 0)\n\
               fn build(a: String) -> Int { let s = \"x\" + a + \"y\"; return len(s); }\n\
               fn main() { println(build(\"q\")); }";
    let program = hale_syntax::parse_source(src).expect("parse");
    let msgs: Vec<String> = hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect();
    assert!(
        msgs.iter().any(|m| m.contains("budget exceeded")),
        "concat must count against the ceiling: {:?}",
        msgs
    );
    // The kind is named on the SECONDARY diagnostic that points at the
    // site, not the primary verdict — same shape as every other alloc
    // kind.
    assert!(
        msgs.iter().any(|m| m.contains("string concatenation")),
        "some diagnostic should name the site kind: {:?}",
        msgs
    );
}

/// The control that makes the above safe: integer arithmetic is NOT
/// an allocation. If this ever fails, every arithmetic-bearing fn
/// with a budget starts failing and the annotation becomes unusable.
#[test]
fn integer_arithmetic_is_not_an_allocation() {
    let src = "@budget(alloc_per_call = 0)\n\
               fn add(a: Int, b: Int) -> Int { let n = a + b + 1; return n; }\n\
               fn main() { println(add(1, 2)); }";
    let program = hale_syntax::parse_source(src).expect("parse");
    let msgs: Vec<String> = hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect();
    assert!(
        !msgs.iter().any(|m| m.contains("budget exceeded")),
        "`i + 1` must never be mistaken for concatenation: {:?}",
        msgs
    );
}
