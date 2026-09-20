//! GH #747: a `const` inside a locus body is rejected at its
//! declaration.
//!
//! `const` is a top-level declaration. The parser accepts one as a
//! locus member and the checker used to typecheck its value, so
//!
//! ```hale
//! locus Holder {
//!     const limit: Int = 7;
//!     params { n: Int = 3; }
//!     fn read() -> Int { return self.n + Holder::limit; }
//! }
//! ```
//!
//! passed `hale check` ("ok: 1 file(s) typechecked") and then failed
//! in the backend with `Unsupported("locus `Holder` member kind not
//! yet lowered to codegen")` — a program the gate accepted could not
//! be built, and the message named no line. The read spelling made no
//! difference: `Holder::limit` typed as `Unknown` (a two-segment path
//! that is not an enum variant), and a bare `limit` was reported at
//! the USE, as an unknown identifier, which is a confusing thing to
//! read when a `const limit` sits three lines above.
//!
//! The member is now refused at the `const` keyword, naming the two
//! things that do work: a top-level `const` (in scope inside every
//! locus of the seed) or a `params` field with a default.

use hale_syntax::parse_source;
use hale_types::{check_bundle_opts_whole_program, check_program, Bundle};
use std::collections::BTreeMap;

/// The GH #747 locus-member-const diagnostics raised for `src`, as
/// (message, the source text the span covers).
fn member_const_diags(src: &str) -> Vec<(String, String)> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog)
        .into_iter()
        .filter(|d| d.message.contains("not a locus member"))
        .map(|d| (d.message.clone(), d.span.slice(src).to_string()))
        .collect()
}

const ISSUE_REPRODUCER: &str = r#"
locus Holder {
    const limit: Int = 7;
    params { n: Int = 3; }
    fn read() -> Int { return self.n + Holder::limit; }
}
fn main() { let h = Holder { }; println(h.read()); }
"#;

#[test]
fn the_reproducer_is_rejected_at_the_const_keyword() {
    let hits = member_const_diags(ISSUE_REPRODUCER);
    assert_eq!(hits.len(), 1, "exactly one diagnostic: {hits:?}");
    let (msg, at) = &hits[0];
    // At the declaration's first token — the one line that has to
    // change — not at the `Holder::limit` that reads it.
    assert_eq!(at, "const", "span covers the `const` keyword: {msg}");
    assert!(
        msg.starts_with("`const limit` is declared inside locus `Holder`:"),
        "names what was declared, and where: {msg}"
    );
    assert!(
        msg.contains("`const` is a top-level declaration"),
        "states the rule: {msg}"
    );
    assert!(
        msg.contains("Move it above the locus")
            && msg.contains("params field with a default"),
        "offers both working spellings: {msg}"
    );
}

/// The bare spelling — `limit` instead of `Holder::limit` — is the
/// same declaration and gets the same diagnostic at the same place.
/// (A whole-program check adds GH #721's unknown-identifier report at
/// the USE, which is correct: nothing binds `limit` until the const
/// moves out.)
#[test]
fn the_bare_read_spelling_is_rejected_at_the_declaration_too() {
    let src = r#"
locus Holder {
    const limit: Int = 7;
    params { n: Int = 3; }
    fn read() -> Int { return self.n + limit; }
}
fn main() { let h = Holder { }; println(h.read()); }
"#;
    let hits = member_const_diags(src);
    assert_eq!(hits.len(), 1, "exactly one diagnostic: {hits:?}");
    assert_eq!(hits[0].1, "const", "span covers the `const` keyword");
}

/// The build path (`hale build` / `hale run` / `hale test`) runs the
/// whole-program check, so the member never reaches codegen — which is
/// the point: check and build agree now.
#[test]
fn the_build_path_check_refuses_it_as_well() {
    let prog = parse_source(ISSUE_REPRODUCER).expect("parse failed");
    let mut programs: BTreeMap<String, &hale_syntax::ast::Program> =
        BTreeMap::new();
    programs.insert("main.hl".to_string(), &prog);
    let diags = check_bundle_opts_whole_program(&Bundle::new(programs), false);
    assert!(
        diags
            .iter()
            .any(|d| d.is_error() && d.message.contains("not a locus member")),
        "the whole-program check reports it: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

/// A `main locus` is a locus: its body may not declare one either.
#[test]
fn a_main_locus_may_not_declare_one() {
    let src = r#"
main locus App {
    const limit: Int = 7;
    params { n: Int = 3; }
    run() { println(self.n); }
}
"#;
    let hits = member_const_diags(src);
    assert_eq!(hits.len(), 1, "exactly one diagnostic: {hits:?}");
    assert!(
        hits[0].0.starts_with("`const limit` is declared inside locus `App`:"),
        "names the locus: {}",
        hits[0].0
    );
}

/// Neither workaround is refused: a top-level `const` read from a
/// locus method, and a params field with the same default, both check
/// clean. (Both also run and print 10 — see the issue.)
#[test]
fn the_two_working_spellings_check_clean() {
    let top_level = r#"
const LIMIT: Int = 7;
locus Holder {
    params { n: Int = 3; }
    fn read() -> Int { return self.n + LIMIT; }
}
fn main() { let h = Holder { }; println(h.read()); }
"#;
    let as_a_param = r#"
locus Holder {
    params { n: Int = 3; limit: Int = 7; }
    fn read() -> Int { return self.n + self.limit; }
}
fn main() { let h = Holder { }; println(h.read()); }
"#;
    for src in [top_level, as_a_param] {
        let prog = parse_source(src).expect("parse failed");
        let diags = check_program(&prog);
        assert!(
            diags.is_empty(),
            "checks clean: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }
}
