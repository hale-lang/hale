//! GH #877 — an unresolvable BARE type name in an annotation is a
//! located error under the whole-program check.
//!
//! The reported shape is a signature:
//!
//! ```text
//! fn helper() -> int { return 99; }
//! ```
//!
//! `int` is the lowercase spelling of `Int` and names nothing.
//! `resolve_type_expr` maps an unknown single-segment name to
//! `Ty::Unknown`, which is permissive everywhere, so `hale check`
//! answered `ok: 1 file(s) typechecked` and the program then died in
//! codegen with `unknown type name 'int' in signature` — late, from
//! another layer, and with no source location. It is the
//! check/build divergence class `corpus_check_build_agreement`
//! ratchets, and the fixture that carried the ratchet row for it
//! (`crates/hale-cli/tests/fixtures/hale-test-mixed/notes.hl`) said
//! in its own first line that it was "valid Hale".
//!
//! The rule is the bare-IDENTIFIER rule (GH #721) applied to type
//! position, with the same whole-vs-partial line: a bare name in a
//! whole program can only be declared by the bundle, so an
//! unresolvable one is a typo; one file of a multi-file seed may
//! legitimately name a type a sibling file declares, so checked alone
//! it keeps the permissive reading. QUALIFIED names are untouched —
//! `lib::Thing` resolves only when the bundle carries the build's
//! import renames (GH #803 / #833), and a tool holding one seed
//! without its imports must not squiggle it.
//!
//! ## The programs below are deliberately PLAIN string literals
//!
//! `hale-corpus` scrapes `r#"…"#` literals out of test sources and
//! feeds every one that looks like a program to the corpus-wide
//! properties — including `corpus_check_build_agreement`, which
//! typechecks each one and then builds it, and the topology baseline,
//! which stamps an artifact identity for each. A negative fixture for
//! this rule has no business in either, so writing these as raw
//! strings would harvest the rule's own counterexamples into the
//! gates the rule is judged by. Do not "tidy" them into raw strings.

use std::collections::BTreeMap;

use hale_syntax::error::DiagKind;
use hale_syntax::parse_source;
use hale_types::{check_bundle_opts_whole_program, check_program, Bundle};

/// The GH #877 diagnostics for `src` under the WHOLE-program check —
/// what `hale check <dir>`, `hale build`, `hale run`, `hale test` and
/// `hale lsp` all run — as (message, the source text the span
/// covers).
fn unknown_type_diags(src: &str) -> Vec<(String, String)> {
    let prog = parse_source(src).expect("parse failed");
    let mut programs: BTreeMap<String, &hale_syntax::ast::Program> =
        BTreeMap::new();
    programs.insert("main.hl".to_string(), &prog);
    check_bundle_opts_whole_program(&Bundle::new(programs), false)
        .into_iter()
        .filter(|d| d.is_error() && d.message.starts_with("unknown type `"))
        .map(|d| (d.message.clone(), d.span.slice(src).to_string()))
        .collect()
}

/// Every error the whole-program check raises, for the "this program
/// is clean" controls — a rule that traded the missing diagnostic for
/// a different wrong one would still pass a filtered assertion.
fn all_errors(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    let mut programs: BTreeMap<String, &hale_syntax::ast::Program> =
        BTreeMap::new();
    programs.insert("main.hl".to_string(), &prog);
    check_bundle_opts_whole_program(&Bundle::new(programs), false)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message)
        .collect()
}

/// The issue's own program, and the four other annotation positions
/// the same typo reaches.
const FIVE_POSITIONS: &str = "fn helper() -> int {\n\
     \x20   return 99;\n\
     }\n\
     fn take(x: Strng) -> Int {\n\
     \x20   return 1;\n\
     }\n\
     type Row {\n\
     \x20   id: Idnt;\n\
     }\n\
     locus Holder {\n\
     \x20   params { seen: Cnt; }\n\
     }\n\
     fn main() {\n\
     \x20   let n: flot = 1;\n\
     \x20   println(n);\n\
     }\n";

/// A fn return, a fn param, a struct field, a locus `params` field
/// and a `let` ascription: one diagnostic each, each one at the NAME
/// rather than at the declaration or the file.
#[test]
fn every_annotation_position_is_reported_at_the_name() {
    let hits = unknown_type_diags(FIVE_POSITIONS);
    let at: Vec<&str> = hits.iter().map(|(_, s)| s.as_str()).collect();
    assert_eq!(
        at,
        vec!["int", "Strng", "Idnt", "Cnt", "flot"],
        "one diagnostic per position, spanning exactly the name: {hits:?}"
    );
    for (msg, _) in &hits {
        assert!(
            msg.contains(
                "no type, enum, locus, interface or alias with that name \
                 is declared"
            ),
            "the message states the rule: {msg}"
        );
    }
}

/// The did-you-mean, in the shape GH #721 / #722 established. The
/// primitives are offered first: `int` for `Int` is the whole point
/// of the issue, and a program's own declarations are never one edit
/// from a primitive's spelling by accident.
#[test]
fn the_nearest_spelling_is_named() {
    let hits = unknown_type_diags(FIVE_POSITIONS);
    let msgs: Vec<&str> = hits.iter().map(|(m, _)| m.as_str()).collect();
    assert!(msgs[0].ends_with("did you mean `Int`?"), "{}", msgs[0]);
    assert!(msgs[1].ends_with("did you mean `String`?"), "{}", msgs[1]);
    assert!(msgs[4].ends_with("did you mean `Float`?"), "{}", msgs[4]);
    // A user declaration is a candidate too, not only a primitive.
    let own = unknown_type_diags(
        "type Reading { v: Int = 0; }\n\
         fn read() -> Readng { return Reading { }; }\n\
         fn main() { println(1); }\n",
    );
    assert_eq!(own.len(), 1, "{own:?}");
    assert!(own[0].0.ends_with("did you mean `Reading`?"), "{:?}", own[0]);
}

/// A name nothing resembles gets the rule without a guess, rather
/// than a suggestion that cannot be taken.
#[test]
fn a_name_nothing_resembles_gets_no_guess() {
    let hits = unknown_type_diags(
        "fn f() -> Quaternion { return 1; }\n\
         fn main() { println(1); }\n",
    );
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(
        !hits[0].0.contains("did you mean"),
        "no candidate within a short edit distance: {:?}",
        hits[0]
    );
}

/// The machine-readable record `hale check --json` prints is built
/// from the diagnostic's kind, its error-ness and its span, so a gate
/// reading the JSON sees a located error rather than an empty list.
#[test]
fn the_diagnostic_is_a_located_type_error() {
    let prog = parse_source(FIVE_POSITIONS).expect("parse failed");
    let mut programs: BTreeMap<String, &hale_syntax::ast::Program> =
        BTreeMap::new();
    programs.insert("main.hl".to_string(), &prog);
    let d = check_bundle_opts_whole_program(&Bundle::new(programs), false)
        .into_iter()
        .find(|d| d.message.starts_with("unknown type `int`"))
        .expect("the reproducer's diagnostic");
    assert_eq!(d.kind, DiagKind::Type, "`type error`, not a warning");
    assert!(d.is_error(), "it fails the check: {}", d.message);
    // The span is the name's own, which is what the renderers turn
    // into `line:col` — line 1, columns 16..19 of the reproducer.
    assert_eq!(d.span.slice(FIVE_POSITIONS), "int");
    assert_eq!(
        FIVE_POSITIONS[..d.span.start.as_usize()].lines().count(),
        1,
        "on the signature's own line"
    );
}

/// One file of a multi-file seed keeps the permissive reading: the
/// type may be declared by a sibling this check was not handed.
/// `check_bundle_opts` is that caller — `hale check <file>`, a
/// harness's partial program, a styleguide snippet.
///
/// `check_program` was that caller too until GH #911 B1. It is not one
/// any more: one `Program` has no sibling it could be missing, so it
/// holds the whole-program rules and a caller that means "a fragment"
/// says so by asking for the partial check. Both readings are pinned
/// here, because the one thing that must not happen is for them to
/// become the same reading by accident.
#[test]
fn one_file_checked_alone_stays_permissive() {
    let prog = parse_source(FIVE_POSITIONS).expect("parse failed");
    let mut programs: BTreeMap<String, &hale_syntax::ast::Program> =
        BTreeMap::new();
    programs.insert("main.hl".to_string(), &prog);
    let errors: Vec<String> =
        hale_types::check_bundle_opts(&Bundle::new(programs), false)
            .into_iter()
            .filter(|d| d.is_error())
            .map(|d| d.message)
            .collect();
    assert!(
        errors.is_empty(),
        "a partial program keeps the Unknown tolerance: {errors:?}"
    );

    // And the whole-program entry refuses the same bytes, at the name.
    let refused: Vec<String> = check_program(&prog)
        .into_iter()
        .filter(|d| d.is_error() && d.message.starts_with("unknown type `"))
        .map(|d| d.message)
        .collect();
    assert!(
        !refused.is_empty(),
        "`check_program` holds the whole-program rules (GH #911 B1) — a \
         bare type name nothing declares is an error there"
    );
}

/// The sibling that supplies the name makes the seed clean — the
/// rule is about the whole program, not about one file's contents.
#[test]
fn a_sibling_file_declaring_the_type_is_enough() {
    let a = parse_source("fn take(r: Row) -> Int { return r.id; }\n")
        .expect("parse failed");
    let b = parse_source(
        "type Row { id: Int = 0; }\nfn main() { println(take(Row { })); }\n",
    )
    .expect("parse failed");
    let mut programs: BTreeMap<String, &hale_syntax::ast::Program> =
        BTreeMap::new();
    programs.insert("a.hl".to_string(), &a);
    programs.insert("b.hl".to_string(), &b);
    let errors: Vec<String> =
        check_bundle_opts_whole_program(&Bundle::new(programs), false)
            .into_iter()
            .filter(|d| d.is_error())
            .map(|d| d.message)
            .collect();
    assert!(errors.is_empty(), "{errors:?}");
}

/// A generic parameter names no declaration by design. It resolves to
/// `Ty::Unknown` exactly like a typo does, so the rule has to know
/// the parameters in scope — in the signature, in the body's `let`
/// ascriptions, and in a generic type's fields.
#[test]
fn a_generic_parameter_is_not_a_typo() {
    let errors = all_errors(
        "type Pair<A, B> {\n\
         \x20   left: A;\n\
         \x20   right: B;\n\
         }\n\
         fn id<T>(x: T) -> T {\n\
         \x20   let same: T = x;\n\
         \x20   return same;\n\
         }\n\
         fn main() { println(id(1)); }\n",
    );
    assert!(errors.is_empty(), "{errors:?}");
}

/// A generic LOCUS carries its parameters for every annotation its
/// members write — a method signature, a `params` field, a capacity
/// slot — and a generic method of one adds to them rather than
/// replacing them.
#[test]
fn a_generic_locus_carries_its_parameters_to_its_members() {
    let errors = all_errors(
        "locus Cache<K, V> {\n\
         \x20   params { cap: Int = 4; }\n\
         \x20   capacity { pool cells of V; }\n\
         \x20   fn put(k: K, v: V) -> Int {\n\
         \x20       let held: V = v;\n\
         \x20       println(k, held);\n\
         \x20       return self.cap;\n\
         \x20   }\n\
         }\n\
         fn main() { println(1); }\n",
    );
    assert!(errors.is_empty(), "{errors:?}");
}

/// The names that resolve to something other than a plain user
/// declaration, in one program: an interface (registered as its own
/// symbol, not in the type table the resolver consults), an enum, an
/// alias and its target, a locus, a generic instantiation, and the
/// compiler-synthesized payload types — `IoError` and friends, which
/// the resolver injects, and `ClosureViolation`, which it does not
/// inject anywhere and which has always resolved to `Ty::Unknown`.
#[test]
fn the_names_that_are_not_declarations_are_still_not_typos() {
    let errors = all_errors(
        "interface Sink { fn emit(n: Int); }\n\
         type Color = enum { Red, Green };\n\
         type Count = Int;\n\
         type Box<T> { value: T; }\n\
         locus Child {\n\
         \x20   params { n: Int = 0; }\n\
         }\n\
         locus Parent {\n\
         \x20   params { n: Int = 0; }\n\
         \x20   accept(c: Child) { }\n\
         \x20   on_failure(c: Child, err: ClosureViolation) { }\n\
         }\n\
         fn emit_to(s: Sink, c: Color, n: Count) -> Int { return n; }\n\
         fn boxed() -> Box<Int> { return Box_Int { value: 1 }; }\n\
         fn read(p: String) -> String fallible(IoError) {\n\
         \x20   return std::io::fs::read_file(p) or fail err;\n\
         }\n\
         fn num(s: String) -> Int fallible(ParseError) {\n\
         \x20   return std::str::parse_int(s) or fail err;\n\
         }\n\
         fn main() { println(1); }\n",
    );
    assert!(errors.is_empty(), "{errors:?}");
}

/// A QUALIFIED name keeps GH #803's rules: it is not this rule's
/// business, in any position, including the `let` ascription #833
/// resolves through the bundle's rename table.
#[test]
fn a_qualified_name_is_left_alone() {
    let errors = all_errors(
        "fn main() {\n\
         \x20   let s: std::text::StringSink = std::text::StringSink { };\n\
         \x20   let t: nowhere::Thing = 1;\n\
         \x20   println(1);\n\
         }\n",
    );
    assert!(
        !errors.iter().any(|e| e.starts_with("unknown type `")),
        "a path is #803's frontier, not this rule's: {errors:?}"
    );
}

/// The compound type expressions carry annotations of their own, and
/// a typo inside one is the same typo.
#[test]
fn nested_annotations_are_walked() {
    let hits = unknown_type_diags(
        "fn f(rows: [Rw; 4], pairs: (Int, Strin), fill: bounded[Byt; 8]) {\n\
         \x20   println(1);\n\
         }\n\
         fn main() { println(1); }\n",
    );
    let at: Vec<&str> = hits.iter().map(|(_, s)| s.as_str()).collect();
    assert_eq!(at, vec!["Rw", "Strin", "Byt"], "{hits:?}");
}

/// The rule fires once per annotation, not once per use: the name
/// resolves to `Unknown` afterwards, which is assignable in both
/// directions, so nothing downstream reports a second message about
/// the same mistake.
#[test]
fn one_typo_is_one_diagnostic() {
    let errors = all_errors(
        "fn helper() -> int {\n\
         \x20   return 99;\n\
         }\n\
         fn main() { println(helper()); }\n",
    );
    assert_eq!(errors.len(), 1, "no cascade: {errors:?}");
}
