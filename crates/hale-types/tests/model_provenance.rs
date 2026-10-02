//! #392 thread 1 — the normalized model: witness provenance and the
//! phase relation.
//!
//! A violation used to name WHO (a path of names); with the model's
//! decl provenance it also says WHERE TO EDIT — the callsite that
//! introduces the crossing edge, the publish site and subscription
//! for a bus hop, and the destination's declaration — as secondary
//! diagnostics whose spans point at real source, following the
//! effect system's root + leaf precedent. Spans are emitted only
//! for bundle decls: stdlib bodies parse in their own offset space,
//! and pointing a bundle diagnostic at one would name the wrong
//! source line.

use hale_syntax::{parse_source, Diag};

fn all_diags(src: &str) -> Vec<Diag> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
}

/// The span of `needle`'s first occurrence in `src`.
fn at(src: &str, needle: &str) -> (usize, usize) {
    let s = src.find(needle).expect("needle present");
    (s, s + needle.len())
}

fn within(d: &Diag, (s, e): (usize, usize)) -> bool {
    d.span.start.as_usize() >= s && d.span.start.as_usize() < e
}

// =====================================================================
// Call-crossing provenance
// =====================================================================

#[test]
fn a_call_witness_points_at_the_crossing_call_and_the_destination() {
    let src = r#"
        locus B { fn work(n: Int) -> Int { return n * 2; } }
        locus A {
            params { b: B = B { }; }
            fn go(n: Int) -> Int { return self.b.work(n); }
        }
        group a_side = { A };
        group b_side = { B };
        main locus App {
            params { a: A = A { }; }
            claims { iso: forbid reaches(a_side, b_side); }
        }
        fn main() { App { }; }
    "#;
    let ds = all_diags(src);
    assert!(
        ds.iter().any(|d| d.message.contains("claim `iso` violated")),
        "the claim must be violated: {:?}",
        ds.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    let cross = ds
        .iter()
        .find(|d| d.message.contains("is crossed by this call"))
        .expect("a crossing-call secondary must be emitted");
    assert!(
        within(cross, at(src, "self.b.work(n)")),
        "the crossing secondary must point at the callsite, got span \
         {:?}",
        cross.span
    );
    let decl = ds
        .iter()
        .find(|d| {
            d.message.contains(
                "the forbidden destination `B` is declared here",
            )
        })
        .expect("a destination-decl secondary must be emitted");
    assert!(
        within(decl, at(src, "B {")),
        "the decl secondary must point at the destination's \
         declaration, got span {:?}",
        decl.span
    );
}

// =====================================================================
// Bus-crossing provenance
// =====================================================================

#[test]
fn a_bus_witness_points_at_the_publish_and_the_subscription() {
    let src = r#"
        type M { n: Int; }
        topic T { payload: M; }
        locus A {
            bus { publish T; }
            fn go(n: Int) { T <- M { n: n }; }
        }
        locus B {
            params { t: Int = 0; }
            bus { subscribe T as on_m; }
            fn on_m(m: M) { self.t = self.t + m.n; }
        }
        group a_side = { A };
        group b_side = { B };
        main locus App {
            params { a: A = A { }; b: B = B { }; }
            claims { iso: forbid reaches(a_side, b_side); }
        }
        fn main() { App { }; }
    "#;
    let ds = all_diags(src);
    let publ = ds
        .iter()
        .find(|d| {
            d.message.contains("the crossing publish happens here")
        })
        .expect("a publish-site secondary must be emitted");
    assert!(
        within(publ, at(src, "T <- M { n: n }")),
        "the publish secondary must point at the publish site, got \
         span {:?}",
        publ.span
    );
    let sub = ds
        .iter()
        .find(|d| {
            d.message.contains("delivered at this subscription")
        })
        .expect("a subscription secondary must be emitted");
    assert!(
        within(sub, at(src, "subscribe T as on_m")),
        "the subscription secondary must point at the subscription \
         decl, got span {:?}",
        sub.span
    );
}

// =====================================================================
// Origin gating — no span may point into a foreign offset space
// =====================================================================

/// A witness whose crossing edge lives in a STDLIB body must not
/// emit a callsite span (stdlib parses at offset 0 of its own
/// source; the offset would name the wrong line of the user file).
/// The destination decl (a bundle decl) still gets its span.
#[test]
fn a_stdlib_interior_crossing_emits_no_foreign_span() {
    let src = r#"
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
            fn probe(r: std::http::Router, req: std::http::Request) -> Int {
                let resp = r.dispatch(req);
                return resp.status;
            }
        }
        group gates = { Gate };
        group handlers = { Hello };
        main locus App {
            params { g: Gate = Gate { }; }
            claims { iso: forbid reaches(gates, handlers); }
        }
        fn main() {
            let r = std::http::Router { };
            r.add("GET", "/", Hello { });
            let req = std::http::Request {
                method: "GET", path: "/", body: ""
            };
            println(Gate { }.probe(r, req));
        }
    "#;
    let ds = all_diags(src);
    assert!(
        ds.iter().any(|d| d.message.contains("claim `iso` violated")),
        "the through-stdlib path must violate: {:?}",
        ds.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    for d in &ds {
        assert!(
            d.span.start.as_usize() < src.len(),
            "no diagnostic may carry a span outside the bundle \
             source (a stdlib offset misattributed): {:?} `{}`",
            d.span,
            d.message
        );
    }
    assert!(
        ds.iter().any(|d| d.message.contains(
            "the forbidden destination `Hello` is declared here"
        )),
        "the bundle-side destination decl still gets its span: {:?}",
        ds.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

// =====================================================================
// The phase relation
// =====================================================================

/// `during` evaluates against the model's phase relation: a
/// lifecycle hook is a phase, and restricting to it excludes paths
/// rooted in other phases.
#[test]
fn during_selects_the_hook_phase_via_the_relation() {
    let base = r#"
        locus B { fn work(n: Int) -> Int { return n * 2; } }
        locus A {
            params { b: B = B { }; n: Int = 0; }
            birth { self.n = self.b.work(1); }
            fn later(n: Int) -> Int { return n; }
        }
        group a_side = { A };
        group b_side = { B };
        main locus App {
            params { a: A = A { }; }
            claims { iso: forbid reaches(a_side, b_side) during PHASE; }
        }
        fn main() { App { }; }
    "#;
    // The birth hook reaches B — a `during birth` claim violates.
    let ds = all_diags(&base.replace("PHASE", "birth"));
    assert!(
        ds.iter().any(|d| d.message.contains("claim `iso` violated")),
        "the birth-phase path must violate: {:?}",
        ds.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // `later` does not reach B — restricting to that phase holds.
    let ds = all_diags(&base.replace("PHASE", "later"));
    assert!(
        !ds.iter().any(|d| d.message.contains("violated")),
        "a phase that never reaches B must hold: {:?}",
        ds.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

/// The derived model itself: hooks and modes are `hook` rows,
/// ordinary methods are `method` rows, and seed decls carry their
/// alias.
#[test]
fn the_model_classifies_phases_and_seed_origin() {
    let src = r#"
        locus W {
            params { n: Int = 0; }
            birth { self.n = 1; }
            fn step(k: Int) -> Int { return k; }
        }
        fn main() { W { }; }
    "#;
    let program = parse_source(src).expect("parse");
    let renames: Vec<(Vec<String>, String)> = vec![(
        vec!["dep".into(), "Helper".into()],
        "__lib_1_dep_Helper".into(),
    )];
    let model =
        hale_types::model::Model::derive(&[&program], &renames);
    let birth = model
        .phases
        .get(&hale_types::alloc_summary::FnKey::method(
            "W".to_string(),
            "birth".to_string(),
        ))
        .expect("birth phase row");
    assert!(birth.hook && birth.phase == "birth");
    let step = model
        .phases
        .get(&hale_types::alloc_summary::FnKey::method(
            "W".to_string(),
            "step".to_string(),
        ))
        .expect("method phase row");
    assert!(!step.hook && step.phase == "step");
    assert_eq!(
        model.seeds.get("dep").map(|s| s.len()),
        Some(1),
        "the seed sort carries the alias's members"
    );
}
