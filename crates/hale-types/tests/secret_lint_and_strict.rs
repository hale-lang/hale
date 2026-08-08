//! GH #436: `@secret` is a lint by default and fails closed under
//! `--strict-secret`.
//!
//! The defect: `@secret` was reported as a certificate while being a
//! local identifier walker over a fragment of one fn body. It walked
//! `then` branches but not `else`, and had no notion of aliasing, so
//! moving a publish across a branch or renaming through one `let`
//! made the finding *disappear* rather than surface as uncertified.
//!
//! Two things are pinned here, and the split between them is the
//! whole point. The default pass keeps exactly the reach it always
//! had — widening a lint in place newly fails programs that compile
//! today, which is a userspace break even when every new finding is a
//! real bug. The widened, fail-closed walk is opt-in.

use hale_syntax::parse_source;

fn diags(src: &str) -> Vec<hale_syntax::error::Diag> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
}

fn strict(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::frontier::secret_taint_strict(&[&program])
        .into_iter()
        .map(|d| d.message)
        .collect()
}

/// One publish of a `@secret` param, in the shape named by `body`.
fn program(body: &str) -> String {
    format!(
        "
        type Msg {{ v: String; }}
        topic Out {{ payload: Msg; subject: \"app.out\"; }}
        locus Sender {{
            params {{ n: Int = 0; }}
            bus {{ publish Out; }}
            fn f(@secret token: String, flag: Bool) {{ {body} }}
        }}
        locus Sink {{
            params {{ n: Int = 0; }}
            bus {{ subscribe Out as on_out; }}
            fn on_out(m: Msg) {{ self.n = len(m.v); }}
        }}
        main locus App {{
            params {{ s: Sender = Sender {{ }}; k: Sink = Sink {{ }}; }}
        }}
        fn main() {{ App {{ }}; }}
        "
    )
}

const DIRECT: &str = "Out <- Msg { v: token };";
const IN_ELSE: &str =
    "if flag { print(\"x\"); } else { Out <- Msg { v: token }; }";
const VIA_ALIAS: &str = "let alias = token; Out <- Msg { v: alias };";

#[test]
fn the_default_pass_is_a_warning_not_an_error() {
    let ds = diags(&program(DIRECT));
    assert!(
        ds.iter().any(|d| !d.is_error()
            && d.message.contains("`@secret` value reaches a bus publish")),
        "the direct leak must still be reported: {:?}",
        ds.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    assert!(
        !ds.iter().any(|d| d.is_error()),
        "a lint must not fail the build: {:?}",
        ds.iter().filter(|d| d.is_error()).collect::<Vec<_>>()
    );
}

#[test]
fn the_default_pass_keeps_its_old_reach_exactly() {
    // Deliberate, not an oversight: widening the default would break
    // programs that compile today. Both of these are real leaks and
    // both stay silent until `--strict-secret`.
    for body in [IN_ELSE, VIA_ALIAS] {
        let ds = diags(&program(body));
        assert!(
            !ds.iter().any(|d| d.message.contains("`@secret`")),
            "default pass unexpectedly widened on {body:?}: {:?}",
            ds.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }
}

#[test]
fn strict_closes_the_else_hole() {
    let ms = strict(&program(IN_ELSE));
    assert!(
        ms.iter().any(|m| m.contains("reaches a bus publish")),
        "the else-branch publish must be caught: {ms:?}"
    );
}

#[test]
fn strict_closes_the_alias_hole() {
    let ms = strict(&program(VIA_ALIAS));
    assert!(
        ms.iter().any(|m| m.contains("reaches a bus publish")),
        "the aliased publish must be caught: {ms:?}"
    );
}

#[test]
fn strict_still_catches_the_direct_case() {
    // A widened walker that lost the case the narrow one had would be
    // a regression the other two tests cannot see.
    let ms = strict(&program(DIRECT));
    assert!(
        ms.iter().any(|m| m.contains("reaches a bus publish")),
        "{ms:?}"
    );
}

#[test]
fn strict_reports_uncertified_rather_than_passing_silently() {
    // The property that separates this from the lint: a secret
    // reaching something the walker cannot follow is uncertified, not
    // absent. `helper` is a fn whose body this pass does not enter.
    let src = "
        fn helper(s: String) -> Int { return len(s); }
        locus L {
            params { n: Int = 0; }
            fn f(@secret token: String) { self.n = helper(token); }
        }
        main locus App { params { l: L = L { }; } }
        fn main() { App { }; }
    ";
    let ms = strict(src);
    assert!(
        ms.iter().any(|m| m.contains("uncertified")),
        "an unfollowed call must be uncertified, got {ms:?}"
    );
}

#[test]
fn strict_is_silent_on_a_program_with_no_secret_params() {
    let src = "
        locus L {
            params { n: Int = 0; }
            fn f(x: String) { print(\"{x}\"); }
        }
        main locus App { params { l: L = L { }; } }
        fn main() { App { }; }
    ";
    assert!(strict(src).is_empty(), "{:?}", strict(src));
}
