//! GH #756: a `type` inside a locus body is rejected at its
//! declaration.
//!
//! The sibling of the member `const` (GH #747). `type` is a top-level
//! declaration; the parser accepts one as a locus member and the
//! checker ignored it, so
//!
//! ```hale
//! locus Holder {
//!     type Pair { a: Int = 0; b: Int = 0; }
//!     params { n: Int = 3; }
//!     fn read() -> Int { return self.n; }
//! }
//! ```
//!
//! passed `hale check` and then failed in the backend with
//! `unsupported in codegen v0: locus `Holder` member kind not yet
//! lowered to codegen` — a program the gate accepted that could not be
//! built, refused by a message that named no line. Nothing could USE
//! the declaration either: the resolver registers no member type, so
//! the name was invisible everywhere, including inside the locus that
//! declared it.
//!
//! The member is now refused at the `type` keyword, and the working
//! spelling — a top-level `type`, in scope inside every locus of the
//! seed — is named in the message.

use hale_syntax::parse_source;
use hale_types::{check_bundle_opts_whole_program, check_program, Bundle};
use std::collections::BTreeMap;

/// The GH #756 locus-member-type diagnostics raised for `src`, as
/// (message, the source text the span covers).
fn member_type_diags(src: &str) -> Vec<(String, String)> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog)
        .into_iter()
        .filter(|d| d.message.contains("not a locus member"))
        .map(|d| (d.message.clone(), d.span.slice(src).to_string()))
        .collect()
}

const ISSUE_REPRODUCER: &str = r#"
locus Holder {
    type Pair { a: Int = 0; b: Int = 0; }
    params { n: Int = 3; }
    fn read() -> Int { return self.n; }
}
fn main() { let h = Holder { }; println(h.read()); }
"#;

#[test]
fn the_reproducer_is_rejected_at_the_type_keyword() {
    let hits = member_type_diags(ISSUE_REPRODUCER);
    assert_eq!(hits.len(), 1, "exactly one diagnostic: {hits:?}");
    let (msg, at) = &hits[0];
    // At the declaration's first token — the one line that has to
    // change.
    assert_eq!(at, "type", "span covers the `type` keyword: {msg}");
    assert!(
        msg.starts_with("`type Pair` is declared inside locus `Holder`:"),
        "names what was declared, and where: {msg}"
    );
    assert!(
        msg.contains("`type` is a top-level declaration"),
        "states the rule: {msg}"
    );
    assert!(
        msg.contains("Move it above the locus"),
        "says what to do: {msg}"
    );
}

/// Both `type` forms a program can write are the same declaration to
/// the parser, and `TypeDecl::span` starts at the keyword in each — so
/// both are refused in the same place. (The third form the AST carries,
/// the alias `type X = Int;`, does not parse ANYWHERE today, at top
/// level either — a pre-existing gap unrelated to this rule.)
#[test]
fn every_type_form_is_refused_at_the_keyword() {
    for body in ["{ a: Int = 0; }", "= enum { Red, Green };"] {
        let src = format!(
            "locus Holder {{\n\
             \x20   type Thing {body}\n\
             \x20   params {{ n: Int = 3; }}\n\
             \x20   fn read() -> Int {{ return self.n; }}\n\
             }}\n\
             fn main() {{ Holder {{ }}; }}\n"
        );
        let hits = member_type_diags(&src);
        assert_eq!(hits.len(), 1, "`type Thing {body}` refused: {hits:?}");
        assert_eq!(hits[0].1, "type", "span covers the `type` keyword");
        assert!(
            hits[0].0.starts_with("`type Thing` is declared inside locus"),
            "names what was declared: {}",
            hits[0].0
        );
    }
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
    type Pair { a: Int = 0; b: Int = 0; }
    params { n: Int = 3; }
    run() { println(self.n); }
}
"#;
    let hits = member_type_diags(src);
    assert_eq!(hits.len(), 1, "exactly one diagnostic: {hits:?}");
    assert!(
        hits[0].0.starts_with("`type Pair` is declared inside locus `App`:"),
        "names the locus: {}",
        hits[0].0
    );
}

/// The working spelling is not refused: the same `type` moved above
/// the locus checks clean and is in scope inside the locus's own
/// method, which is what the diagnostic tells the author to do. (It
/// also runs and prints 10 — see the issue.)
#[test]
fn a_top_level_type_read_from_a_locus_method_checks_clean() {
    let src = r#"
type Pair { a: Int = 0; b: Int = 0; }
locus Holder {
    params { n: Int = 3; }
    fn read() -> Int {
        let p = Pair { a: self.n, b: 7 };
        return p.a + p.b;
    }
}
fn main() { let h = Holder { }; println(h.read()); }
"#;
    let prog = parse_source(src).expect("parse failed");
    let diags = check_program(&prog);
    assert!(
        diags.is_empty(),
        "checks clean: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}
