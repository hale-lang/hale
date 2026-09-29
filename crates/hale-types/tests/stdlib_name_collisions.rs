//! A top-level name the stdlib declares is declared once (spec/semantics.md
//! § "Declarations inside `module { }`"). The stdlib's loci, types and fns
//! are merged into every program under their internal names, so a second
//! declaration of one —
//! a user seed spelling it, or a second stdlib file declaring it — met the
//! first only in the merged program, and the build panicked ("method
//! declared in pass A2" when the HTTP server's per-connection locus reused
//! the client's `__StdHttpConn`). Both shapes are refused by the checker,
//! located, naming both declarations.

use std::collections::BTreeMap;

use hale_syntax::error::SpanOrigin;
use hale_syntax::parse_source;
use hale_types::check_program;
use hale_types::stdlib_names::name_diags;

/// Two loci of one name in the stdlib: refused at the second, pointing at
/// the first, both in the stdlib's coordinates.
#[test]
fn two_stdlib_loci_sharing_a_name_are_refused() {
    let std_src = "locus __StdConn {\n    params { fd: Int = 0; }\n}\nlocus __StdConn {\n    params { handle: Int = 0; }\n    birth() { }\n}\n";
    let std_prog = parse_source(std_src).expect("parse");
    let diags = name_diags(&std_prog, &BTreeMap::new());
    assert_eq!(diags.len(), 1, "one refusal: {:?}", diags.iter().map(|d| &d.message).collect::<Vec<_>>());
    let d = &diags[0];
    assert!(d.is_error(), "an error: {}", d.message);
    assert!(d.message.contains("the stdlib declares `__StdConn` twice"), "{}", d.message);
    assert_eq!(d.origin, SpanOrigin::Stdlib, "located in the stdlib");
    let second = std_src.rfind("__StdConn").unwrap();
    let first = std_src.find("__StdConn").unwrap();
    assert_eq!(d.span.start.as_usize(), second, "at the second declaration");
    assert!(
        d.related.iter().any(|r| r.span.start.as_usize() == first && r.origin == SpanOrigin::Stdlib),
        "the note names the first: {:?}",
        d.related
    );
}

/// A user locus spelling a bundled stdlib locus's internal name: refused
/// at the user's declaration, with a note at the stdlib's.
#[test]
fn a_program_locus_named_like_a_stdlib_locus_is_refused() {
    let src = "locus __StdHttpConn {\n    params { n: Int = 0; }\n}\nfn main() { __StdHttpConn { }; }\n";
    let prog = parse_source(src).expect("parse");
    let diags: Vec<_> = check_program(&prog)
        .into_iter()
        .filter(|d| d.message.contains("is the stdlib's internal name for its locus `__StdHttpConn`"))
        .collect();
    assert_eq!(diags.len(), 1, "one refusal");
    let d = &diags[0];
    assert!(d.is_error(), "an error: {}", d.message);
    assert_eq!(d.origin, SpanOrigin::Seed, "located in the program");
    assert_eq!(d.span.start.as_usize(), src.find("__StdHttpConn").unwrap(), "at the program's declaration");
    assert!(
        d.related.iter().any(|r| r.origin == SpanOrigin::Stdlib && r.label.contains("stdlib")),
        "the note names the stdlib's declaration: {:?}",
        d.related
    );
}

/// The stdlib's own seed, checked as a program, is the same declaration
/// and not a second one; and a program's own names are untouched.
#[test]
fn the_stdlibs_own_declaration_and_ordinary_names_are_clean() {
    let std_src = "locus __StdConn {\n    params { fd: Int = 0; }\n}\n";
    let std_prog = parse_source(std_src).expect("parse");
    // the same text again, at other positions
    let same = parse_source(&format!("\n\n{std_src}")).expect("parse");
    let mine = parse_source("locus Conn {\n    params { fd: Int = 0; }\n}\n").expect("parse");
    let mut programs = BTreeMap::new();
    programs.insert("std_seed.hl".to_string(), &same);
    programs.insert("app.hl".to_string(), &mine);
    let diags = name_diags(&std_prog, &programs);
    assert!(diags.is_empty(), "{:?}", diags.iter().map(|d| &d.message).collect::<Vec<_>>());
}

/// One namespace, as the program's own duplicate rule has: a fn may not
/// take a stdlib locus's name either, and a declaration nested in a
/// `module { }` is a top-level declaration.
#[test]
fn a_fn_or_a_module_nested_decl_reusing_a_stdlib_name_is_refused() {
    for (what, src) in [
        ("a fn", "fn __StdHttpConn() -> Int { return 1; }\nfn main() { print(__StdHttpConn()); }\n"),
        ("a module-nested locus", "module m {\n    locus __StdHttpConn { params { n: Int = 0; } }\n}\nfn main() { print(1); }\n"),
    ] {
        let prog = parse_source(src).expect("parse");
        let refused = check_program(&prog)
            .into_iter()
            .any(|d| d.is_error() && d.message.contains("is the stdlib's internal name for its locus `__StdHttpConn`"));
        assert!(refused, "{what} is refused");
    }
}

/// The invariant behind the rule's stdlib half: the stdlib that ships
/// declares each name once. A duplicate would fail every program's check
/// with a diagnostic located in the stdlib, so it is caught here first.
#[test]
fn the_bundled_stdlib_declares_each_name_once() {
    let std_prog = hale_types::stdlib_bodies::program().expect("the bundled stdlib parses");
    let diags = name_diags(std_prog, &BTreeMap::new());
    assert!(diags.is_empty(), "{:?}", diags.iter().map(|d| &d.message).collect::<Vec<_>>());
}
