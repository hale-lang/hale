//! GH #265 frontier items: cross-actor causality, supervision
//! coverage, secret taint, inferred manifest, symbolic cost.

fn diags_for(src: &str) -> Vec<String> {
    let program = hale_syntax::parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

/// The headline: causality reaches PAST the call graph, through a
/// declared bus edge, into what a subscriber does. No call-graph
/// analysis can see this.
#[test]
fn causes_follows_bus_edges_into_subscribers() {
    let src = r#"
        type Order { n: Int; }
        topic Orders { payload: Order; subject: "orders"; }
        locus Audit {
            bus { subscribe Orders as on_order; }
            fn on_order(o: Order) {
                std::io::fs::write_file("/tmp/audit", "x") or discard;
            }
        }
        locus Api {
            bus { publish Orders; }
            @effects(causes: {publish}) fn handle(n: Int) {
                Orders <- Order { n: n };
            }
        }
        fn main() { Audit { }; Api { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        ds.iter().any(|m| m.contains("causal set violated")
            && m.contains("syscall")),
        "publishing to a subject whose subscriber writes a file CAUSES a \
         syscall — the declaration omits it: {:?}",
        ds
    );
    assert!(
        ds.iter().any(|m| m.contains("subject `Orders`")
            && m.contains("Audit::on_order")),
        "the diagnostic must name the causal path: {:?}",
        ds
    );
}

#[test]
fn causes_accepts_a_complete_declaration() {
    let src = r#"
        type Order { n: Int; }
        topic Orders { payload: Order; subject: "orders"; }
        locus Audit {
            bus { subscribe Orders as on_order; }
            fn on_order(o: Order) {
                std::io::fs::write_file("/tmp/audit", "x") or discard;
            }
        }
        locus Api {
            bus { publish Orders; }
            @effects(causes: {publish, syscall, alloc, block})
            fn handle(n: Int) {
                Orders <- Order { n: n };
            }
        }
        fn main() { Audit { }; Api { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("causal set violated")),
        "a complete causal declaration must pass: {:?}",
        ds
    );
}

/// Supervision coverage: a locus with children and no failure policy
/// anywhere above it has nowhere for a failure to go.
#[test]
fn supervised_flags_an_uncovered_subtree() {
    let src = r#"
        locus Leaf { params { n: Int = 0; } }
        locus Mid { params { leaf: Leaf = Leaf { }; } }
        @supervised
        main locus App {
            params { mid: Mid = Mid { }; }
            run() { }
        }
        fn main() { App { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        ds.iter().any(|m| m.contains("@supervised` violated")),
        "a subtree with no failure policy must be reported: {:?}",
        ds
    );
}

#[test]
fn supervised_satisfied_by_a_root_policy() {
    let src = r#"
        locus Leaf { params { n: Int = 0; } }
        locus Mid { params { leaf: Leaf = Leaf { }; } }
        @supervised
        main locus App {
            params { mid: Mid = Mid { }; }
            on_failure(e: Violation) { }
            run() { }
        }
        fn main() { App { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("@supervised")),
        "a root policy covers the whole subtree: {:?}",
        ds
    );
}

/// Coarse secret taint: a `@secret` param must not reach a publish
/// or a log sink.
#[test]
fn secret_must_not_reach_a_log_sink() {
    let src = r#"
        fn auth(@secret token: String, user: String) {
            println("authenticating ", user);
            println("token=", token);
        }
        fn main() { auth("s3cr3t", "riley"); }
    "#;
    let ds = diags_for(src);
    assert!(
        ds.iter().any(|m| m.contains("`@secret` value reaches a log")),
        "a secret in a log line must be flagged: {:?}",
        ds
    );
}

#[test]
fn secret_must_not_reach_the_bus() {
    let src = r#"
        type Msg { s: String; }
        topic T { payload: Msg; subject: "t"; }
        locus P {
            bus { publish T; }
            fn leak(@secret key: String) {
                T <- Msg { s: key };
            }
        }
        fn main() { P { }; }
    "#;
    let ds = diags_for(src);
    assert!(
        ds.iter().any(|m| m.contains("`@secret` value reaches a bus")),
        "a secret on the bus must be flagged: {:?}",
        ds
    );
}

#[test]
fn non_secret_params_are_unaffected() {
    let src = r#"
        fn auth(token: String, user: String) {
            println("token=", token, " user=", user);
        }
        fn main() { auth("t", "u"); }
    "#;
    let ds = diags_for(src);
    assert!(
        !ds.iter().any(|m| m.contains("@secret")),
        "without @secret nothing is tainted: {:?}",
        ds
    );
}

/// The manifest can carry INFERRED sets (it's a report, not a type).
#[test]
fn manifest_and_cost_are_available() {
    use hale_types::alloc_summary::{self, FnKey};
    use hale_types::frontier;
    let src = r#"
        fn helper(n: Int) -> Int {
            std::io::fs::write_file("/tmp/x", "y") or discard;
            return n;
        }
        fn caller(n: Int) -> Int {
            let mut i = 0;
            while i < n { i = i + 1; }
            return helper(i);
        }
        fn main() { println(caller(2)); }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let summary = alloc_summary::summarize_programs(&[&program]);
    let ffi = std::collections::BTreeSet::new();
    let eff = frontier::infer_effects(
        &summary,
        &FnKey::free_fn("caller"),
        &ffi,
    );
    let names = frontier::render_effects(eff);
    assert!(
        names.contains(&"syscall".to_string()),
        "inference must propagate the callee's syscall: {:?}",
        names
    );
    let cost = frontier::cost_expression(&summary, &FnKey::free_fn("caller"));
    assert!(
        cost.contains("O(n^1)"),
        "a single loop yields a linear structural cost: {}",
        cost
    );
}
