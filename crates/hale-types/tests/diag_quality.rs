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
            // plain assignability does not know about `serves`.
            "a locus serving the perspective the param is typed as",
            r#"perspective P { fn go(); }
               locus V1 : serves P { fn go() { } }
               locus L { params { p: P = V1 { }; } }
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
