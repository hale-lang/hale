//! #392 thread 4 — interface-dispatch edges.
//!
//! A method call on an interface-typed value used to resolve
//! `Unresolved` with `recv_ty: Some(<interface>)` and was silently
//! dropped by every walker — the one receiver shape left where a
//! real call contributed nothing to any judgment (the #382 root fix
//! closed the `recv_ty: None` class). The closed world makes the
//! implementor set enumerable, so the summarizer now fans the one
//! written call out to an edge per conforming locus; an interface
//! nothing conforms to fails closed like an untyped receiver.
//!
//! Canary + control per judgment form, per the #382 doctrine: a
//! checker that cannot fail proves nothing.

use hale_syntax::parse_source;

fn diags(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

// =====================================================================
// Effects: `@effects(none: …)` must see through dispatch
// =====================================================================

/// CANARY — the audit's router shape (`route.handler.handle(ctx)`):
/// a carrier reached through an interface-typed STRUCT FIELD must
/// violate the certificate. Before the fan-out this passed silently.
#[test]
fn a_carrier_behind_an_interface_typed_field_fires() {
    let src = r#"
        effect money;
        interface Notifier { fn send(n: Int) -> Int; }
        locus Email {
            @effects(is: {money})
            fn send(n: Int) -> Int { return n; }
        }
        type Route { handler: Notifier; }
        locus A {
            @effects(none: {money})
            fn go(n: Int) -> Int {
                let r = Route { handler: Email { } };
                return r.handler.send(n);
            }
        }
        fn main() { println(A { }.go(1)); }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("effect assertion violated")),
        "a carrier behind an interface-typed field must violate: {:?}",
        ds
    );
}

/// CANARY — the stdlib server shape: dispatch through an
/// interface-typed fn PARAMETER reaches every conformer, so one
/// carrying implementor among several is found.
#[test]
fn a_carrier_among_several_conformers_fires_through_a_param() {
    let src = r#"
        effect money;
        interface Notifier { fn send(n: Int) -> Int; }
        locus Log {
            fn send(n: Int) -> Int { return n; }
        }
        locus Email {
            @effects(is: {money})
            fn send(n: Int) -> Int { return n; }
        }
        locus A {
            @effects(none: {money})
            fn go(h: Notifier, n: Int) -> Int { return h.send(n); }
        }
        fn main() { println(A { }.go(Log { }, 1)); }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("effect assertion violated")),
        "dispatch must reach every conformer, including the carrier: {:?}",
        ds
    );
}

/// CONTROL — clean conformers keep the certificate: fanning out must
/// not turn every dispatch into a violation.
#[test]
fn a_dispatch_to_clean_conformers_still_certifies() {
    let src = r#"
        effect money;
        interface Notifier { fn send(n: Int) -> Int; }
        locus Log { fn send(n: Int) -> Int { return n; } }
        type Route { handler: Notifier; }
        locus A {
            @effects(none: {money})
            fn go(n: Int) -> Int {
                let r = Route { handler: Log { } };
                return r.handler.send(n);
            }
        }
        fn main() { println(A { }.go(1)); }
    "#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("effect assertion violated")),
        "clean conformers must still certify: {:?}",
        ds
    );
}

/// An interface NO locus conforms to has no values in this closed
/// world — an interface value only arises by coercing a conformer —
/// so its call sites are DEAD and a certificate over them holds
/// vacuously. (The everyday instance: the stdlib router's
/// `m.before(cur)` over an empty middleware list. Failing closed
/// here would refuse every certificate through the router.)
#[test]
fn an_uninhabited_interface_call_is_dead_code() {
    let src = r#"
        effect money;
        interface Ghost { fn vanish(n: Int) -> Int; }
        locus A {
            @effects(none: {money})
            fn go(g: Ghost, n: Int) -> Int { return g.vanish(n); }
        }
        fn main() { A { }; }
    "#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("effect assertion violated")),
        "a call no value can ever reach must certify vacuously: {:?}",
        ds
    );
}

// =====================================================================
// Claims: `forbid reaches` composes through dispatch
// =====================================================================

/// CANARY — a bundle claim sees the dispatch edge and produces a
/// witness naming the conforming implementor.
#[test]
fn forbid_reaches_finds_a_path_through_dispatch() {
    let src = r#"
        interface Notifier { fn send(n: Int) -> Int; }
        locus Email { fn send(n: Int) -> Int { return n; } }
        type Route { handler: Notifier; }
        locus A {
            fn go(n: Int) -> Int {
                let r = Route { handler: Email { } };
                return r.handler.send(n);
            }
        }
        group a_side = { A };
        group sinks = { Email };
        main locus App {
            params { a: A = A { }; }
            claims { iso: forbid reaches(a_side, sinks); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    let hit = ds
        .iter()
        .find(|m| m.contains("claim `iso` violated"))
        .unwrap_or_else(|| {
            panic!("dispatch must be a reachability edge: {:?}", ds)
        });
    assert!(
        hit.contains("Email"),
        "the witness must name the implementor: {}",
        hit
    );
}

/// CONTROL — dispatch fan-out adds edges only to CONFORMERS: a locus
/// with the same method name but different arity is not fanned to,
/// so a claim against it holds.
#[test]
fn forbid_reaches_holds_against_a_non_conformer() {
    let src = r#"
        interface Notifier { fn send(n: Int) -> Int; }
        locus Email { fn send(n: Int) -> Int { return n; } }
        locus Audit { fn send(a: Int, b: Int) -> Int { return a + b; } }
        type Route { handler: Notifier; }
        locus A {
            fn go(n: Int) -> Int {
                let r = Route { handler: Email { } };
                return r.handler.send(n);
            }
        }
        group a_side = { A };
        group sinks = { Audit };
        main locus App {
            params { a: A = A { }; }
            claims { iso: forbid reaches(a_side, sinks); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("claim `iso` violated")),
        "a non-conformer must not receive a dispatch edge: {:?}",
        ds
    );
}

/// The claim-side control for the same rule: a dead dispatch site is
/// not an unresolvable edge, so a claim whose source group contains
/// it still certifies (nothing can flow through it).
#[test]
fn a_claim_over_an_uninhabited_interface_call_still_certifies() {
    let src = r#"
        interface Ghost { fn vanish(n: Int) -> Int; }
        locus B { fn work(n: Int) -> Int { return n; } }
        locus A {
            fn go(g: Ghost, n: Int) -> Int { return g.vanish(n); }
        }
        group a_side = { A };
        group b_side = { B };
        main locus App {
            params { a: A = A { }; }
            claims { iso: forbid reaches(a_side, b_side); }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("violated")
            || m.contains("cannot be certified")),
        "a dead dispatch site must not refuse certification: {:?}",
        ds
    );
}

// =====================================================================
// The stdlib router — the shape that motivated the thread
// =====================================================================

/// CANARY — end-to-end through `std::http::Router`: the certificate
/// walk enters `Router::dispatch` -> `__http_run_chain` and the
/// route-entry dispatch (`e.handler.handle(cur)`) fans out to the
/// user's carrying handler. Before #392 the whole chain was
/// invisible past the interface hop.
#[test]
fn a_certificate_sees_a_carrier_through_the_stdlib_router() {
    let src = r#"
        effect money;
        locus Hello {
            @effects(is: {money})
            fn handle(ctx: std::http::Context) -> std::http::Response {
                return std::http::Response {
                    status: 200,
                    content_type: "text/plain",
                    body: "hi"
                };
            }
        }
        locus Gate {
            @effects(none: {money})
            fn probe(r: std::http::Router, req: std::http::Request) -> Int {
                let resp = r.dispatch(req);
                return resp.status;
            }
        }
        fn main() {
            let r = std::http::Router { };
            r.get("/", Hello { });
            let req = std::http::Request {
                method: "GET", path: "/", body: ""
            };
            println(Gate { }.probe(r, req));
        }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("effect assertion violated")),
        "the router chain must expose the handler's effect: {:?}",
        ds
    );
}

/// CONTROL — the same chain with a clean handler certifies: the
/// empty middleware list (`Middleware` has no conformer here) is
/// dead, not fail-closed, and clean fan-out targets add nothing.
#[test]
fn a_clean_handler_through_the_stdlib_router_still_certifies() {
    let src = r#"
        effect money;
        locus Hello {
            fn handle(ctx: std::http::Context) -> std::http::Response {
                return std::http::Response {
                    status: 200,
                    content_type: "text/plain",
                    body: "hi"
                };
            }
        }
        locus Gate {
            @effects(none: {money})
            fn probe(r: std::http::Router, req: std::http::Request) -> Int {
                let resp = r.dispatch(req);
                return resp.status;
            }
        }
        fn main() {
            let r = std::http::Router { };
            r.get("/", Hello { });
            let req = std::http::Request {
                method: "GET", path: "/", body: ""
            };
            println(Gate { }.probe(r, req));
        }
    "#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("effect assertion violated")),
        "a clean handler through the router must certify: {:?}",
        ds
    );
}

// =====================================================================
// `bound` (and every counting judgment): alternatives take the MAX
// =====================================================================

/// CANARY of the SEMANTICS — one dispatch invokes ONE conforming
/// target, so two one-carrier alternatives bound at 1, not 2. A sum
/// over the fan-out would fail this claim on phantom calls.
#[test]
fn dispatch_alternatives_bound_by_their_max_not_their_sum() {
    let src = r#"
        effect llm;
        interface Model { fn ask(p: Int) -> Int; }
        locus Fast {
            @effects(is: {llm})
            fn ask(p: Int) -> Int { return p; }
        }
        locus Deep {
            @effects(is: {llm})
            fn ask(p: Int) -> Int { return p; }
        }
        type Slot { m: Model; }
        locus Planner {
            fn plan(n: Int) -> Int {
                let s = Slot { m: Fast { } };
                return s.m.ask(n);
            }
        }
        group planners = { Planner };
        main locus App {
            params { p: Planner = Planner { }; }
            claims { one_call: bound llm <= 1 on paths from planners; }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("claim `one_call` violated")),
        "one dispatch is one call — the bound must take the max over \
         alternatives, not their sum: {:?}",
        ds
    );
}

/// CANARY — the max is still counted: an alternative whose body
/// carries TWO sites exceeds a bound of one through dispatch.
#[test]
fn a_heavy_alternative_still_violates_the_bound() {
    let src = r#"
        effect llm;
        @effects(is: {llm})
        fn model_call(p: Int) -> Int { return p; }
        interface Model { fn ask(p: Int) -> Int; }
        locus Fast { fn ask(p: Int) -> Int { return model_call(p); } }
        locus Deep {
            fn ask(p: Int) -> Int {
                return model_call(p) + model_call(p);
            }
        }
        type Slot { m: Model; }
        locus Planner {
            fn plan(n: Int) -> Int {
                let s = Slot { m: Fast { } };
                return s.m.ask(n);
            }
        }
        group planners = { Planner };
        main locus App {
            params { p: Planner = Planner { }; }
            claims { one_call: bound llm <= 1 on paths from planners; }
        }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("claim `one_call` violated")),
        "the heaviest alternative (2 sites) must exceed the bound: {:?}",
        ds
    );
}

// =====================================================================
// `@budget(alloc_per_call)`: the guard the #382 fix left dead
// =====================================================================

/// CANARY — an untyped-receiver method call must refuse a zero-alloc
/// budget. The #382 root fix wrote this message but only widened the
/// `indirect` guard in the OTHER walkers; the budget's own guard
/// still let a wrapper certify.
#[test]
fn an_untyped_receiver_call_refuses_an_alloc_budget() {
    let src = r#"
type P { a: Int; }
locus Maker {
    fn make(n: Int) -> P { return P { a: n }; }
}

@budget(alloc_per_call = 0)
fn zero(n: Int) -> Int {
    let xs = [Maker { }];
    let p = xs[0].make(n);
    return p.a;
}

fn main() { }
"#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("budget")
            && m.contains("receiver the compiler cannot type")),
        "an untyped-receiver call must fail the budget closed: {:?}",
        ds
    );
}

/// CANARY — dispatch to an allocating conformer counts against the
/// budget (the fan-out resolves the edge, the walk enters the body).
#[test]
fn a_dispatch_to_an_allocating_conformer_counts() {
    let src = r#"
type P { a: Int; }
interface Maker { fn make(n: Int) -> P; }
locus Boxed {
    fn make(n: Int) -> P { return P { a: n }; }
}

@budget(alloc_per_call = 0)
fn zero(m: Maker, n: Int) -> Int {
    let p = m.make(n);
    return p.a;
}

fn main() { }
"#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("budget")),
        "an allocating conformer must count against the budget: {:?}",
        ds
    );
}

/// CONTROL — dispatch alternatives budget at their max: two
/// one-alloc conformers satisfy `alloc_per_call = 1`.
#[test]
fn budget_counts_dispatch_alternatives_at_their_max() {
    let src = r#"
type P { a: Int; }
interface Maker { fn make(n: Int) -> P; }
locus Boxed {
    fn make(n: Int) -> P { return P { a: n }; }
}
locus Tagged {
    fn make(n: Int) -> P { return P { a: n + 1 }; }
}

@budget(alloc_per_call = 1)
fn one(m: Maker, n: Int) -> Int {
    let p = m.make(n);
    return p.a;
}

fn main() { }
"#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("budget")),
        "two one-alloc alternatives must satisfy a budget of 1: {:?}",
        ds
    );
}
