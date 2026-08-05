//! #392 thread 2 — library-tier claims, the in-crate half: the
//! tier split (a closing seed may not write the top-level form) and
//! standalone-library evaluation. The cross-seed travel-and-recheck
//! path lives in `hale-cli/tests/xseed_library_claims.rs`.

use hale_syntax::parse_source;

fn diags(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

/// A seed that closes the world (declares `main locus`) states its
/// law inside main — the top-level form is the library tier and is
/// rejected here. This is the enforcement surface for "a dependency
/// may not brick downstream builds with world-claims": the tier is
/// syntactic, and a library can only name what it can see.
#[test]
fn a_closing_seed_may_not_use_the_top_level_form() {
    let src = r#"
        locus B { fn work(n: Int) -> Int { return n; } }
        group b_side = { B };
        claims {
            iso: require subscribes(some b_side, topic T);
        }
        type M { n: Int; }
        topic T { payload: M; }
        main locus App { params { b: B = B { }; } }
        fn main() { App { }; }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("LIBRARY tier")
            && m.contains("state world law inside it")),
        "a closing seed's top-level block must be rejected: {:?}",
        ds
    );
}

/// A library checked standalone (no main anywhere) evaluates its
/// top-level claims over its own world — canary and control.
#[test]
fn a_standalone_library_evaluates_its_own_claims() {
    let violated = r#"
        type M { n: Int; }
        topic T { payload: M; }
        locus A {
            params { s: Int = 0; }
            bus { subscribe T as on_m; }
            fn on_m(m: M) { self.s = self.s + m.n; }
        }
        locus B {
            params { s: Int = 0; }
            bus { subscribe T as on_m; }
            fn on_m(m: M) { self.s = self.s + m.n; }
        }
        group subs = { A, B };
        claims {
            single: count subscribers(topic T) <= 1;
        }
    "#;
    let ds = diags(violated);
    assert!(
        ds.iter().any(|m| m.contains("claim `single` violated")),
        "two subscribers must violate the library's own claim: {:?}",
        ds
    );
    let control = r#"
        type M { n: Int; }
        topic T { payload: M; }
        locus A {
            params { s: Int = 0; }
            bus { subscribe T as on_m; }
            fn on_m(m: M) { self.s = self.s + m.n; }
        }
        group subs = { A };
        claims {
            single: count subscribers(topic T) <= 1;
        }
    "#;
    let ds = diags(control);
    assert!(
        !ds.iter().any(|m| m.contains("violated")),
        "one subscriber satisfies the library's claim: {:?}",
        ds
    );
}
