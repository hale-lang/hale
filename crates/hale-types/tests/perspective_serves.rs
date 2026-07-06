//! Phase 2a (perspectives) — `serves` conformance typecheck.
//!
//! A `locus L : serves P` must provide every contract method P
//! declares (matching arity + param + return types), the
//! perspective analog of interface structural satisfaction.

use hale_syntax::parse_source;
use hale_types::check_program;

fn check(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

#[test]
fn conforming_impl_clean() {
    let src = r#"
perspective Router {
    fn route(code: Int) -> Int;
    fn health() -> Int;
}
locus RouterV1 : serves Router {
    fn route(code: Int) -> Int { return code + 1; }
    fn health() -> Int { return 1; }
}
main locus App {
    params { r: perspective(Router) = RouterV1 { }; }
    run() { }
}
fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().all(|m| !m.contains("serves") && !m.contains("contract")),
        "expected conforming impl to typecheck clean, got: {:?}",
        msgs
    );
}

#[test]
fn missing_contract_method_rejected() {
    let src = r#"
perspective Router {
    fn route(code: Int) -> Int;
    fn health() -> Int;
}
locus RouterV1 : serves Router {
    fn route(code: Int) -> Int { return code; }
}
main locus App { params { r: RouterV1 = RouterV1 { }; } run() { } }
fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m|
            m.contains("missing contract method") && m.contains("health")),
        "expected missing-method diagnostic, got: {:?}",
        msgs
    );
}

#[test]
fn wrong_return_type_rejected() {
    let src = r#"
perspective Router { fn route(code: Int) -> Int; }
locus RouterV1 : serves Router {
    fn route(code: Int) -> Bool { return true; }
}
main locus App { params { r: RouterV1 = RouterV1 { }; } run() { } }
fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m|
            m.contains("route") && m.contains("requires") && m.contains("Int")),
        "expected wrong-return diagnostic, got: {:?}",
        msgs
    );
}

#[test]
fn wrong_arity_rejected() {
    let src = r#"
perspective Router { fn route(code: Int) -> Int; }
locus RouterV1 : serves Router {
    fn route(code: Int, extra: Int) -> Int { return code; }
}
main locus App { params { r: RouterV1 = RouterV1 { }; } run() { } }
fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("route") && m.contains("arg")),
        "expected arity diagnostic, got: {:?}",
        msgs
    );
}

#[test]
fn serves_unknown_perspective_rejected() {
    let src = r#"
locus RouterV1 : serves Nonexistent {
    fn route(code: Int) -> Int { return code; }
}
main locus App { params { r: RouterV1 = RouterV1 { }; } run() { } }
fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m|
            m.contains("serves unknown perspective") && m.contains("Nonexistent")),
        "expected unknown-perspective diagnostic, got: {:?}",
        msgs
    );
}

#[test]
fn serves_non_perspective_rejected() {
    // `serves` must name a perspective, not a locus/type.
    let src = r#"
type NotAPerspective { n: Int; }
locus RouterV1 : serves NotAPerspective {
    fn route(code: Int) -> Int { return code; }
}
main locus App { params { r: RouterV1 = RouterV1 { }; } run() { } }
fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m|
            m.contains("not a perspective contract")),
        "expected non-perspective diagnostic, got: {:?}",
        msgs
    );
}

// === Phase 2b: reperspective typecheck ========================

#[test]
fn reperspective_clean() {
    let src = r#"
perspective Router { fn route(c: Int) -> Int; }
locus RouterV1 : serves Router { fn route(c: Int) -> Int { return c + 1; } }
locus RouterV2 : serves Router { fn route(c: Int) -> Int { return c + 2; } }
locus Gateway {
    params { router: perspective(Router) = RouterV1 { }; }
    run() { reperspective self.router as RouterV2; }
}
main locus App { params { gw: Gateway = Gateway { }; } run() { } }
fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().all(|m| !m.contains("reperspective")),
        "expected clean reperspective, got: {:?}",
        msgs
    );
}

#[test]
fn reperspective_impl_not_serving_rejected() {
    let src = r#"
perspective Router { fn route(c: Int) -> Int; }
locus RouterV1 : serves Router { fn route(c: Int) -> Int { return c; } }
locus NotARouter { fn route(c: Int) -> Int { return c; } }
locus Gateway {
    params { router: perspective(Router) = RouterV1 { }; }
    run() { reperspective self.router as NotARouter; }
}
main locus App { params { gw: Gateway = Gateway { }; } run() { } }
fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("does not") && m.contains("serve")),
        "expected does-not-serve diagnostic, got: {:?}",
        msgs
    );
}

#[test]
fn reperspective_non_perspective_field_rejected() {
    let src = r#"
locus Plain { fn f() -> Int { return 1; } }
locus Gateway {
    params { p: Plain = Plain { }; }
    run() { reperspective self.p as Plain; }
}
main locus App { params { gw: Gateway = Gateway { }; } run() { } }
fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("not a `perspective")),
        "expected not-a-perspective-handle diagnostic, got: {:?}",
        msgs
    );
}

// === Phase 2c: bus surface in the contract ====================

#[test]
fn bus_perspective_conforming_clean() {
    let src = r#"
type Order { id: Int; }
perspective OrderRouter {
    fn health() -> Int;
    bus { subscribe "orders" as on_order of type Order; }
}
locus RouterV1 : serves OrderRouter {
    fn health() -> Int { return 1; }
    bus { subscribe "orders" as on_order of type Order; }
    fn on_order(o: Order) { }
}
main locus App { params { r: perspective(OrderRouter) = RouterV1 { }; } run() { } }
fn main() { App { }; }
"#;
    let msgs = check(src);
    // The only diagnostic is the orphan-topic warning (the demo has
    // no publisher for "orders"); no conformance error should fire.
    assert!(
        msgs.iter().all(|m| !m.contains("does not subscribe")
            && !m.contains("does not publish")
            && !m.contains("missing contract method")),
        "expected no conformance error on a conforming bus-perspective, got: {:?}",
        msgs
    );
}

#[test]
fn bus_perspective_missing_subscription_rejected() {
    let src = r#"
type Order { id: Int; }
perspective OrderRouter {
    fn health() -> Int;
    bus { subscribe "orders" as on_order of type Order; }
}
locus RouterV1 : serves OrderRouter {
    fn health() -> Int { return 1; }
}
main locus App { params { r: RouterV1 = RouterV1 { }; } run() { } }
fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("does not subscribe") && m.contains("orders")),
        "expected missing-subscription diagnostic, got: {:?}",
        msgs
    );
}

#[test]
fn reperspective_bus_perspective_gated() {
    let src = r#"
type Order { id: Int; }
perspective OrderRouter {
    fn health() -> Int;
    bus { subscribe "orders" as on_order of type Order; }
}
locus RouterV1 : serves OrderRouter {
    fn health() -> Int { return 1; }
    bus { subscribe "orders" as on_order of type Order; }
    fn on_order(o: Order) { }
}
locus RouterV2 : serves OrderRouter {
    fn health() -> Int { return 2; }
    bus { subscribe "orders" as on_order of type Order; }
    fn on_order(o: Order) { }
}
locus Gateway {
    params { r: perspective(OrderRouter) = RouterV1 { }; }
    run() { reperspective self.r as RouterV2; }
}
main locus App { params { gw: Gateway = Gateway { }; } run() { } }
fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("bus surface") && m.contains("supported yet")),
        "expected bus-perspective swap gate, got: {:?}",
        msgs
    );
}

// === Phase 3: state-preserving swap footprint check ===========

#[test]
fn reperspective_matching_footprint_clean() {
    let src = r#"
perspective Counter { fn get() -> Int; }
locus CounterV1 : serves Counter { params { n: Int = 0; } fn get() -> Int { return self.n; } }
locus CounterV2 : serves Counter { params { n: Int = 0; } fn get() -> Int { return self.n; } }
locus Service {
    params { c: perspective(Counter) = CounterV1 { }; }
    run() { reperspective self.c as CounterV2; }
}
main locus App { params { s: Service = Service { }; } run() { } }
fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().all(|m| !m.contains("footprint")),
        "expected matching-footprint swap clean, got: {:?}",
        msgs
    );
}

#[test]
fn reperspective_footprint_change_rejected() {
    // CounterV2 adds a field — a state-preserving swap can't
    // reinterpret V1's bytes; this is the (deferred) migrate case.
    let src = r#"
perspective Counter { fn get() -> Int; }
locus CounterV1 : serves Counter { params { n: Int = 0; } fn get() -> Int { return self.n; } }
locus CounterV2 : serves Counter { params { n: Int = 0; extra: Int = 5; } fn get() -> Int { return self.n + self.extra; } }
locus Service {
    params { c: perspective(Counter) = CounterV1 { }; }
    run() { reperspective self.c as CounterV2; }
}
main locus App { params { s: Service = Service { }; } run() { } }
fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("different footprint") && m.contains("migrate")),
        "expected footprint-change diagnostic, got: {:?}",
        msgs
    );
}

#[test]
fn reperspective_footprint_type_change_rejected() {
    // Same field name, different type — also a footprint change.
    let src = r#"
perspective Counter { fn get() -> Int; }
locus CounterV1 : serves Counter { params { n: Int = 0; } fn get() -> Int { return self.n; } }
locus CounterV2 : serves Counter { params { n: Bool = true; } fn get() -> Int { return 0; } }
locus Service {
    params { c: perspective(Counter) = CounterV1 { }; }
    run() { reperspective self.c as CounterV2; }
}
main locus App { params { s: Service = Service { }; } run() { } }
fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("different footprint")),
        "expected footprint-type-change diagnostic, got: {:?}",
        msgs
    );
}
