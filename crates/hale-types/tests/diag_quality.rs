//! GH #241: diagnostic-quality checks — user errors that
//! previously escaped to spanless codegen internal errors now
//! die at check phase, and typo diags carry did-you-mean hints.

use hale_syntax::parse_source;
use hale_types::symbol::Bundle;

fn diags(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    let mut programs: std::collections::BTreeMap<
        String,
        &hale_syntax::ast::Program,
    > = std::collections::BTreeMap::new();
    programs.insert("test.hl".to_string(), &program);
    let bundle = Bundle::new(programs);
    let (scope, mut ds) = hale_types::resolve::build_top_scope(&bundle);
    ds.extend(hale_types::check::check_bundle(&bundle, &scope, true));
    ds.iter().map(|d| d.message.clone()).collect()
}

#[test]
fn printing_a_struct_renders_it() {
    // GH #241 made this an error with a span, replacing a spanless
    // codegen crash. GH #469 made it legal instead: the reason it
    // was an error was that nothing rendered records, and now
    // something does. What #241 was actually protecting — that the
    // failure arrives at check phase with a span rather than as a
    // backend internal error — is preserved by the LOCUS case
    // below, which is refused permanently rather than pending an
    // implementation.
    let src = r#"
        type P { x: Int = 0; }
        fn main() {
            let p = P { x: 1 };
            println("p=", p);
        }
    "#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("cannot render")),
        "a struct of printable fields renders; got: {:?}",
        ds
    );
}

#[test]
fn printing_a_locus_is_a_check_error() {
    // The exclusion that stays. A locus is flow, not shape, and
    // rendering one would read back the `params` a `@sealed` locus
    // exists to confine (GH #436).
    let src = r#"
        locus Svc { params { n: Int = 1; } }
        main locus App { params { s: Svc = Svc { }; } }
        fn main() { let a = App { }; println("s=", a.s); }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("cannot render") && m.contains("`Svc`")),
        "expected printable diag; got: {:?}",
        ds
    );
}

#[test]
fn printing_a_struct_with_an_unprintable_field_is_a_check_error() {
    // Recursion means one bad field anywhere makes the whole
    // record unprintable — the check still has a span and still
    // names the type the author wrote.
    let src = r#"
        type P { blob: Bytes; }
        fn main() {
            let p = P { blob: b"x" };
            println("p=", p);
        }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("cannot render") && m.contains("`P`")),
        "expected printable diag; got: {:?}",
        ds
    );
}

#[test]
fn string_plus_struct_renders_it() {
    // The `String + printable` coercion and the printable set are
    // one rule, so this moved with GH #469 for the same reason.
    let src = r#"
        type P { x: Int = 0; }
        fn main() {
            let p = P { x: 1 };
            let s = "v: " + p;
            println(s);
        }
    "#;
    let ds = diags(src);
    assert!(ds.is_empty(), "String + struct now renders; got: {:?}", ds);
}

#[test]
fn string_plus_locus_is_still_a_check_error() {
    let src = r#"
        locus Svc { params { n: Int = 1; } }
        main locus App { params { s: Svc = Svc { }; } }
        fn main() { let a = App { }; println("v: " + a.s); }
    "#;
    let ds = diags(src);
    assert!(
        !ds.is_empty(),
        "expected a diag for String + locus; got none"
    );
}

#[test]
fn printing_an_enum_stays_legal() {
    let src = r#"
        type Light = enum { Red, Green };
        fn main() {
            let l = Light::Red;
            println("light: ", l);
        }
    "#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("cannot render")),
        "enum printing must stay legal; got: {:?}",
        ds
    );
}

#[test]
fn abs_on_string_is_a_check_error() {
    let src = r#"
        fn main() {
            println(abs("hi"));
        }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("`abs` takes numeric")),
        "expected numeric-builtin diag; got: {:?}",
        ds
    );
}

#[test]
fn field_typo_gets_did_you_mean() {
    let src = r#"
        type Order { quantity: Int = 0; }
        fn main() {
            let o = Order { quantity: 2 };
            println(o.quantty);
        }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("did you mean `quantity`")),
        "expected did-you-mean; got: {:?}",
        ds
    );
}

#[test]
fn a_fixed_array_and_a_bounded_are_chain_sources() {
    // These two used to fail with "no field `get`": chains desugar
    // to a loop fetching each element through the source's `get`,
    // and the type-level collections had no such accessor, so they
    // could not anchor a chain at all. They answer `get` now, so the
    // programs below are ordinary (downstream handoff).
    //
    // Kept as a DIAGNOSTIC test rather than only a codegen one: the
    // failure mode was a confusing message on a reasonable program,
    // and the fix is that there is no message.
    for src in [
        r#"
            locus L {
                params { arr: [Int; 4] = [1, 2, 3, 4]; }
                fn c() -> Int { return self.arr.filter(it > 2).count(); }
            }
            fn main() { }
        "#,
        r#"
            fn take(b: bounded[Int; 8]) -> Int {
                return b.filter(it > 2).count();
            }
            fn main() { }
        "#,
    ] {
        let ds = diags(src);
        assert!(
            ds.is_empty(),
            "a chain over a type-level collection should check clean; \
             got: {:?}",
            ds
        );
    }
}

#[test]
fn a_real_field_typo_still_gets_did_you_mean_not_the_chain_hint() {
    // The chain hint must not swallow the ordinary did-you-mean path:
    // `.get` is the chain accessor, but a plain field typo on a struct
    // is not a chain and keeps its own hint.
    let src = r#"
        type Order { quantity: Int = 0; }
        fn main() {
            let o = Order { quantity: 2 };
            println(o.quantty);
        }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("did you mean `quantity`")
            && !m.contains("element chain")),
        "did-you-mean must be unaffected by the chain hint; got: {:?}",
        ds
    );
}

/// Locus param defaults are typechecked.
///
/// They were not. `check_locus_member` skipped the `Params` block
/// with a comment claiming defaults "are checked against declared
/// types implicitly when the param is referenced" — they weren't, so
/// a mistyped or unresolvable default passed `hale check` and failed
/// in codegen, which is a check/build divergence on one of the most
/// common things in a Hale program.
///
/// Found while writing an HTTP example: `handler: u` referencing a
/// sibling param checked clean and then failed to build with
/// "unknown identifier `u`" and no location.
#[test]
fn a_param_default_is_typechecked() {
    // A bare name resolves to top-level CONST scope, not to sibling
    // params — verified by giving a const and a param the same name
    // and observing the const win. So this is a genuine unknown
    // identifier, and the diagnostic should say the spelling that
    // works rather than leaving the reader to guess.
    let ds = diags(
        r#"
        locus I { params { n: Int = 0; } }
        locus L { params { a: I = I { }; b: I = a; } }
        fn main() { let l = L { }; }
    "#,
    );
    assert!(
        ds.iter().any(|d| d.contains("unknown identifier `a`")
            && d.contains("self.a")),
        "should name the sibling and the working spelling: {:?}",
        ds
    );

    // Declared-vs-default type mismatch.
    let ds = diags(
        r#"
        locus L { params { n: Int = "nope"; } }
        fn main() { let l = L { }; }
    "#,
    );
    assert!(
        ds.iter()
            .any(|d| d.contains("declared `Int`") && d.contains("`String`")),
        "{:?}",
        ds
    );
}

/// The shapes that must keep working — each of these is a way a
/// default legitimately reaches a value, and a naive check rejects
/// at least one of them.
#[test]
fn legitimate_param_defaults_are_not_rejected() {
    for (what, src) in [
        (
            "self.<sibling>",
            r#"locus L { params { n: Int = 1; m: Int = self.n; } }
               fn main() { let l = L { }; }"#,
        ),
        (
            "a top-level const",
            r#"const B: Int = 7;
               locus L { params { n: Int = B; } }
               fn main() { let l = L { }; }"#,
        ),
        (
            "an opaque multi-segment stdlib handle",
            r#"locus L {
                 params { s: std::io::tcp::Stream = std::io::tcp::Stream { }; }
               }
               fn main() { let l = L { }; }"#,
        ),
        (
            // The corpus's perspective fixtures do exactly this, and
            // plain assignability does not know about `serves`. The
            // holder's type is the slot type, `perspective(P)`: a bare
            // `P` is no type a value has, and the check refuses it.
            "a locus serving the perspective the param is typed as",
            r#"perspective P { fn go(); }
               locus V1 : serves P { fn go() { } }
               locus L { params { p: perspective(P) = V1 { }; } }
               fn main() { let l = L { }; }"#,
        ),
    ] {
        let ds = diags(src);
        let param_errs: Vec<&String> =
            ds.iter().filter(|d| d.contains("param `")).collect();
        assert!(
            param_errs.is_empty(),
            "{} must be accepted: {:?}",
            what,
            param_errs
        );
    }
}

#[test]
fn a_bounded_place_is_assigned_whole_from_its_own_type_only() {
    // A bounded field copies another of its own type whole (the
    // behaviour is pinned in tests/hale/bounded_whole_assign_test.hl);
    // any other value is refused at the assignment, once, naming the
    // place's type.
    let ok = diags(
        r#"
        type A { xs: bounded[String; 4]; }
        fn main() {
            let mut a = A { };
            let b = A { };
            a.xs = b.xs;
        }
    "#,
    );
    assert!(ok.is_empty(), "a same-typed whole assignment checks clean; got: {ok:?}");
    let ds = diags(
        r#"
        type A { xs: bounded[String; 4]; }
        type B { xs: bounded[String; 8]; }
        fn main() {
            let mut a = A { };
            let b = B { };
            a.xs = b.xs;
            a.xs = ["a"];
        }
    "#,
    );
    assert_eq!(ds.len(), 2, "one refusal per assignment; got: {ds:?}");
    assert!(ds[0].contains("not assignable from `bounded[String; 8]`"), "got: {ds:?}");
    assert!(
        ds[1].contains("a `bounded[String; 4]` is assigned whole only from another `bounded[String; 4]`"),
        "got: {ds:?}"
    );
    // A bounded local views the place it was bound from: assigned whole,
    // its elements would be anchored in the frame, not where the place
    // lives. Refused, naming the place to assign.
    let ds = diags(
        r#"
        type R { xs: bounded[String; 2]; }
        locus L {
            params { xs: bounded[String; 2]; }
            fn put(r: R) {
                let mut dest = self.xs;
                dest = r.xs;
            }
        }
        fn main() { }
    "#,
    );
    assert_eq!(ds.len(), 1, "one refusal; got: {ds:?}");
    assert!(ds[0].contains("`dest` is a `bounded[String; 2]` local, a view of the place"), "got: {ds:?}");
}

#[test]
fn a_route_params_copy_taken_before_the_match_is_warned_at_its_capture_read() {
    // `is_route` writes its `:name` captures into `c.params`; a copy
    // taken before the match never sees them, so `path_param` on it is
    // "" for every route. Warned at the read, naming the working spelling.
    let ds = diags(
        r#"
        fn route(req: std::http::Request) -> String {
            let c = std::http::build_context(req);
            let p = c.params;
            if std::http::is_route(c, "GET", "/m/:model") {
                return std::http::path_param(p, "model");
            }
            return "";
        }
        fn main() { }
    "#,
    );
    let warned: Vec<&String> = ds.iter().filter(|m| m.contains("is a copy of `c.params`")).collect();
    assert_eq!(warned.len(), 1, "one warning at the read; got: {ds:?}");
    assert!(warned[0].contains("`std::http::path_param(c.params, …)`"), "got: {ds:?}");

    // The shapes that read what the match wrote, or need no capture,
    // are not warned: the context's own params after the match, a copy
    // taken after the match, a query read through the early copy.
    let clean = diags(
        r#"
        fn route(req: std::http::Request) -> String {
            let c = std::http::build_context(req);
            let early = c.params;
            if std::http::is_route(c, "GET", "/a/:id") {
                return std::http::path_param(c.params, "id") + std::http::query_param(early, "q");
            }
            if std::http::is_route(c, "GET", "/b/:id") {
                let late = c.params;
                return std::http::path_param(late, "id");
            }
            return "";
        }
        fn main() { }
    "#,
    );
    assert!(!clean.iter().any(|m| m.contains("is a copy of")), "got: {clean:?}");
}

/// An assignment target's field path is resolved as a read's is: a field
/// the owner does not declare is refused with the read path's diagnostic,
/// one level down a child or several. It used to check clean and fail
/// only at the build ("no field `record` on `self.j`").
#[test]
fn an_assignment_target_names_only_declared_fields() {
    let refused = |body: &str, field: &str, owner: &str| {
        let src = format!(
            r#"
            type Pair {{ a: String = ""; b: Int = 0; }}
            type Rec {{ p: Pair = Pair {{ }}; ps: [Pair; 2] = [Pair {{ }}, Pair {{ }}]; }}
            locus Inner {{ params {{ repo: String = "."; rec: Rec = Rec {{ }}; }} }}
            locus Outer {{
                params {{ j: Inner = Inner {{ }}; rec: Rec = Rec {{ }}; }}
                birth() {{ {body} }}
            }}
            fn main() {{ let o = Outer {{ }}; println(o.j.repo); }}
        "#
        );
        let ds = diags(&src);
        let want = format!("no field `{field}` on `{owner}`");
        assert!(ds.iter().any(|m| m.starts_with(&want)), "`{body}`: want {want}; got {ds:?}");
    };
    // the issue's two forms
    refused(r#"self.j.missing = "x";"#, "missing", "Inner");
    refused(r#"self.j.record.repo = "x";"#, "record", "Inner");
    // each shape of a place
    refused(r#"self.nope = 1;"#, "nope", "Outer");
    refused(r#"self.rec.p.c = 1;"#, "c", "Pair");
    refused(r#"self.rec.ps[0].c = 1;"#, "c", "Pair");
    refused(r#"let mut r = Rec { }; r.q = 1;"#, "q", "Rec");
    refused(r#"let h = Inner { }; h.repoo = "x";"#, "repoo", "Inner");
    // a near name is offered
    let ds = diags(
        r#"
        locus Inner { params { repo: String = "."; } }
        locus Outer { params { j: Inner = Inner { }; } birth() { self.j.rpeo = "x"; } }
        fn main() { }
    "#,
    );
    assert!(ds.iter().any(|m| m.contains("no field `rpeo` on `Inner` — did you mean `repo`?")), "{ds:?}");

    // the value is held to the field's type down the path
    let ds = diags(
        r#"
        type Pair { a: String = ""; b: Int = 0; }
        locus Inner { params { repo: String = "."; p: Pair = Pair { }; } }
        locus Outer { params { j: Inner = Inner { }; } birth() { self.j.p.a = 5; } }
        fn main() { }
    "#,
    );
    assert!(ds.iter().any(|m| m.contains("target type `String` not assignable from `Int`")), "{ds:?}");

    // every shape, declared, checks clean
    let ds = diags(
        r#"
        type Pair { a: String = ""; b: Int = 0; }
        type Rec { p: Pair = Pair { }; ps: [Pair; 2] = [Pair { }, Pair { }]; }
        locus Inner { params { repo: String = "."; rec: Rec = Rec { }; } }
        locus Outer {
            params { j: Inner = Inner { }; rec: Rec = Rec { }; n: Int = 0; }
            birth() {
                self.n = 1;
                self.j.repo = "x";
                self.j.rec.p.a = "y";
                self.rec.p.b = 2;
                self.rec.ps[1].a = "z";
                let mut r = Rec { };
                r.p.a = "w";
                r.ps[0].b = 3;
                let h = Inner { };
                h.repo = "v";
            }
        }
        fn main() { }
    "#,
    );
    assert!(ds.is_empty(), "{ds:?}");
}

/// `==` on an enum holds only when every payload of every variant has its
/// own `==`, through nested enums; a struct payload is refused at the
/// check, naming the path to it (it failed at the build, "match on
/// TypeRef not supported").
#[test]
fn enum_equality_needs_every_payload_to_compare() {
    let ds = diags(
        r#"
        type P { a: Int; }
        type Inner = enum { X(P), Y };
        type E = enum { A(Int), B(Inner) };
        type F = enum { A(P), B };
        type G = enum { A(Int, String), B(Bytes) };
        type H = enum { A(Int, String), B };
        fn main() {
            let e = E::A(1) == E::A(1);
            let f = F::B != F::B;
            let g = G::A(1, "") == G::A(1, "");
            let h = H::A(1, "") == H::B;
        }
    "#,
    );
    assert!(ds.iter().any(|m| m.starts_with("`==` does not apply to `E`: its variant `B` carries `Inner`, whose variant `X` carries a struct (`P`)")), "{ds:?}");
    assert!(ds.iter().any(|m| m.starts_with("`!=` does not apply to `F`: its variant `A` carries a struct (`P`)")), "{ds:?}");
    assert!(ds.iter().any(|m| m.starts_with("`==` does not apply to `G`: its variant `B` carries a byte string (`Bytes`)")), "{ds:?}");
    assert_eq!(ds.len(), 3, "`H` compares: {ds:?}");
}

/// Review of #1616: a generic enum's monomorph (`Value<P>`, `Value_P`) is
/// judged by its substituted payloads, as an enum carrying it is: a struct
/// argument is refused at the check (it failed at the build), and an `Int`
/// or a comparable enum argument compares.
#[test]
fn a_generic_enums_equality_is_its_arguments() {
    let ds = diags(
        r#"
        type P { n: Int; }
        type Inner = enum { A, B(Int) };
        type Value<T> = enum { Some(T), None };
        type Outer = enum { Has(Value<P>), Empty };
        fn same(a: Value<P>, b: Value<P>) -> Bool { return a == b; }
        fn outer(a: Outer, b: Outer) -> Bool { return a == b; }
        fn ints(a: Value<Int>, b: Value<Int>) -> Bool { return a == b; }
        fn inners(a: Value<Inner>, b: Value<Inner>) -> Bool { return a != b; }
        fn main() { }
    "#,
    );
    assert!(ds.iter().any(|m| m.starts_with("`==` does not apply to `Value_P`: its variant `Some` carries a struct (`P`)")), "{ds:?}");
    assert!(
        ds.iter().any(|m| m.starts_with("`==` does not apply to `Outer`: its variant `Has` carries `Value_P`, whose variant `Some` carries a struct (`P`)")),
        "{ds:?}"
    );
    assert_eq!(ds.len(), 2, "`Value<Int>` and `Value<Inner>` compare: {ds:?}");
}


/// GH #1608 follow-up: a pointer-shaped element stored into a bounded place
/// under a parameter is kept in the frame that calls (a free fn) or in the
/// member's scratch (a locus member). A free fn's call that hands it a
/// place of a locus is refused at the argument, and so is a member doing
/// it at all; a frame-local place, a copy of a record, a scalar element and
/// a free fn's own body are sound.
#[test]
fn an_element_stored_through_a_parameters_place_is_refused_where_it_cannot_last() {
    let ds = diags(
        r#"
        type Row { xs: bounded[String; 4]; n: bounded[Int; 4]; }
        fn fill(r: Row, k: Int) { push(r.xs, "x" + to_string(k)) or raise; }
        fn relay(r: Row, k: Int) { fill(r, k); }
        fn ints(r: Row) { push(r.n, 1) or raise; }
        locus L {
            params { row: Row = Row { }; }
            fn put(k: Int) {
                fill(self.row, k);
                relay(self.row, k);
                ints(self.row);
                let mut mine = Row { };
                fill(mine, k);
                let copy = self.row;
                fill(copy, k);
                self.row = mine;
            }
            fn through(r: Row, k: Int) {
                set(r.xs, 0, "m" + to_string(k)) or raise;
            }
        }
        fn main() { }
    "#,
    );
    let calls: Vec<&String> = ds.iter().filter(|m| m.contains("stores pointer-shaped elements")).collect();
    assert_eq!(calls.len(), 2, "fill(self.row) and relay(self.row); got: {ds:?}");
    assert!(calls.iter().all(|m| m.contains("this locus's arena")), "{ds:?}");
    assert!(calls[0].starts_with("`fill`") && calls[1].starts_with("`relay`"), "{ds:?}");
    let member: Vec<&String> = ds.iter().filter(|m| m.contains("under a parameter of a locus member")).collect();
    assert_eq!(member.len(), 1, "the member's own set; got: {ds:?}");
    assert!(member[0].starts_with("`set` of a `String` element"), "{ds:?}");
    assert_eq!(ds.len(), 3, "nothing else: {ds:?}");
}

/// The stdlib's `std::http::is_route` writes its captures into
/// `ctx.params`: a context a locus holds is refused as its argument, and a
/// frame's own is not.
#[test]
fn is_route_on_a_context_a_locus_holds_is_refused() {
    let ds = diags(
        r#"
        locus Api {
            params { last: std::http::Context = std::http::build_context(std::http::Request { method: "GET", path: "/", body: "" }); }
            fn handle(req: std::http::Request) -> std::http::Response {
                let ctx = std::http::build_context(req);
                if std::http::is_route(ctx, "GET", "/u/:id") {
                    return std::http::Response { status: 200, body: std::http::path_param(ctx.params, "id") };
                }
                if std::http::is_route(self.last, "GET", "/v/:id") {
                    return std::http::Response { status: 200, body: "" };
                }
                return std::http::Response { status: 404, body: "" };
            }
        }
        fn main() { }
    "#,
    );
    let hits: Vec<&String> = ds.iter().filter(|m| m.contains("stores pointer-shaped elements")).collect();
    assert_eq!(hits.len(), 1, "{ds:?}");
    assert!(hits[0].starts_with("`std::http::is_route`"), "{ds:?}");
}

/// Review of #1612: an alias of the element or of the bounded type is the
/// same place (refused); a writer reached through a function value is the
/// writer (refused), and one the check cannot name is refused when handed
/// such a place; an owned record copy is no view (accepted).
#[test]
fn a_parameter_place_writer_is_found_through_aliases_and_fn_values() {
    let refused = |src: &str, why: &str| {
        let ds = diags(src);
        assert!(ds.iter().any(|m| m.contains("pointer-shaped elements")), "{why}: {ds:?}");
    };
    refused(
        r#"
        type Text = String;
        type Row { xs: bounded[Text; 4]; }
        fn fill(r: Row) { push(r.xs, "fresh" + to_string(1)) or raise; }
        locus L { params { row: Row = Row { }; } fn put() { fill(self.row); } }
        fn main() { }
    "#,
        "an alias of the element",
    );
    refused(
        r#"
        type Items = bounded[String; 4];
        type Row { xs: Items; }
        fn fill(r: Row) { push(r.xs, "fresh" + to_string(1)) or raise; }
        locus L { params { row: Row = Row { }; } fn put() { fill(self.row); } }
        fn main() { }
    "#,
        "an alias of the bounded type",
    );
    refused(
        r#"
        type Row { xs: bounded[String; 4]; }
        fn fill(r: Row) { push(r.xs, "fresh" + to_string(1)) or raise; }
        locus L { params { row: Row = Row { }; } fn put() { let f = fill; f(self.row); } }
        fn main() { }
    "#,
        "a writer through a function value",
    );
    refused(
        r#"
        type Row { xs: bounded[String; 4]; }
        fn apply(g: fn (Row) -> (), r: Row) { g(r); }
        locus L { params { row: Row = Row { }; } fn put() { apply(fill_one, self.row); } }
        fn fill_one(r: Row) { push(r.xs, "x") or raise; }
        fn main() { }
    "#,
        "a call through a fn the check cannot name",
    );
    let ds = diags(
        r#"
        type Row { xs: bounded[String; 4]; }
        fn fill(r: Row) { let copy = r; push(copy.xs, "fresh" + to_string(1)) or raise; }
        locus L { params { row: Row = Row { }; } fn put() { fill(self.row); } }
        fn main() { }
    "#,
    );
    assert!(!ds.iter().any(|m| m.contains("pointer-shaped elements")), "an owned copy is no view: {ds:?}");
}

/// `diags` over a program the snapshot minted, as the toolchain checks
/// it: each use names its declaration (`Snapshot::binding_of`).
fn minted_diags(src: &str) -> Vec<String> {
    let mut program = parse_source(src).expect("parse");
    hale_types::desugar_sequence::desugar_before_check(
        &mut [&mut program],
        &hale_types::desugar_sequence::Sequence { import_renames: &[], default_surface: "" },
    );
    let ids = hale_types::snapshot::mint([("test.hl", &mut program)], &[]);
    let mut programs = std::collections::BTreeMap::new();
    programs.insert("test.hl".to_string(), &program);
    let mut bundle = Bundle::new(programs);
    bundle.snapshot = ids;
    let (scope, mut ds) = hale_types::resolve::build_top_scope(&bundle);
    ds.extend(hale_types::check::check_bundle(&bundle, &scope, true));
    ds.iter().map(|d| d.message.clone()).collect()
}

/// Second review of #1612: a fn value's target follows the binding a call
/// names. Assigned (`f = fill;`), the binding names no one fn and the call
/// is refused for any argument holding such a place; an inner `let f`
/// names nothing after its block, so the outer writer is still refused;
/// a tuple `let` binds each name to its element. A shadowing owned copy
/// (`let r = r;`) is not the parameter, so writing its places is no write
/// through it, and a member's `let r = Row { }` shadowing its parameter is
/// a frame-local place.
#[test]
fn a_parameter_place_writer_follows_bindings_not_spellings() {
    const PRE: &str = r#"
        type Row { xs: bounded[String; 4]; }
        fn fill(r: Row) { push(r.xs, "fresh" + to_string(1)) or raise; }
        fn noop(r: Row) { }
        fn gfill<T>(r: Row, value: T) { push(r.xs, "fresh" + to_string(1)) or raise; }
        fn gapply<T>(g: fn (T) -> (), x: T) { g(x); }
        fn pass_on(r: Row) { gapply(fill, r); }
        fn gid<T>(x: T) { }
        fn tfill<T>(r: T) { push(r.xs, "fresh" + to_string(1)) or raise; }
    "#;
    let refused = |body: &str, why: &str| {
        let src = format!("{PRE} locus L {{ params {{ row: Row = Row {{ }}; }} fn put() {{ {body} }} }}\nfn main() {{ }}");
        let ds = minted_diags(&src);
        assert!(ds.iter().any(|m| m.contains("pointer-shaped elements")), "{why}: {ds:?}");
    };
    let accepted = |src: &str, why: &str| {
        let ds = minted_diags(&format!("{PRE}{src}\nfn main() {{ }}"));
        assert!(!ds.iter().any(|m| m.contains("pointer-shaped elements")), "{why}: {ds:?}");
    };
    refused("let mut f = noop; f = fill; f(self.row);", "a binding assigned a writer");
    refused("let mut f = fill; f(self.row); f = noop;", "a binding assigned after the call");
    refused(
        "let f = fill; if true { let f = noop; f(self.row); } f(self.row);",
        "the outer writer after an inner binding's block",
    );
    refused("let (f, n) = (fill, 0); f(self.row);", "a tuple binder");
    accepted(
        "locus L { params { row: Row = Row { }; } fn put() { let (f, n) = (noop, 0); f(self.row); } }",
        "a tuple binder of a non-writer",
    );
    accepted(
        "locus L { params { row: Row = Row { }; } fn put() { let f = fill; if true { let f = noop; f(self.row); } } }",
        "the inner non-writer",
    );
    accepted(
        r#"fn copy_fill(r: Row) { let r = r; push(r.xs, "fresh" + to_string(1)) or raise; }
        locus L { params { row: Row = Row { }; } fn put() { copy_fill(self.row); } }"#,
        "a shadowing owned copy",
    );
    accepted(
        "locus L { fn put(r: Row) { let r = Row { }; fill(r); } }",
        "a member's fresh local shadowing its parameter",
    );
    // a generic writer is a writer (third review): its parameter of a
    // concrete record type is written through, and one of a type
    // parameter may be such a record
    refused("gfill(self.row, 1);", "a generic writer");
    // a field path through a type parameter, which the template cannot
    // resolve, may be a bounded place (fourth review)
    refused("tfill(self.row);", "a field path through a type parameter");
    refused("pass_on(self.row);", "a generic fn handing a type parameter to a fn value");
    accepted(
        "locus L { params { row: Row = Row { }; } fn put() { gid(self.row); } }",
        "a generic fn that writes nothing",
    );
    let ds = minted_diags(&format!(
        "{PRE} locus L {{ fn put(r: Row) {{ fill(r); }} }}\nfn main() {{ }}"
    ));
    assert!(
        ds.iter().any(|m| m.contains("pointer-shaped elements") && m.contains("not this member's")),
        "the member's parameter itself is still refused: {ds:?}"
    );
}
