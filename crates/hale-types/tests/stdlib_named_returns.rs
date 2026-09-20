//! GH #771: struct-returning stdlib helpers carry their real type.
//!
//! A `std::json::string_field(...)` call used to type `Ty::Unknown`,
//! because the only rows `SIGS` could express named a primitive
//! return — `sig!`'s `$ret:ident` slot cannot spell the tuple
//! variant `SigTy::Named("__JsonString")`. Unknown is permissive by
//! design (an absent row must never produce a false error), so every
//! question asked THROUGH such a value went unanswered: `.knd`
//! instead of `.kind` was accepted, a one-argument call was accepted,
//! and a `JsonFieldRange` handed to a `String` parameter was
//! accepted. The values are all structs whose fields are read by
//! offset, so the fail-open ran all the way to a wrong load.
//!
//! Two directions, both pinned here:
//!
//!   - fail-open (the bug): the negative pins below make a wrong
//!     field, a wrong arity and a wrong argument type located
//!     errors that name the PUBLIC spelling of the type;
//!   - false errors (the risk of the fix): the positive pins keep
//!     the correct shapes — the ones `dna/` and `iris/` actually
//!     write — accepted. (`hale check` over both trees and the
//!     whole fixture corpus was byte-identical before and after
//!     this change; see the PR body.)
//!
//! Plus a typo guard: every `Named(...)` in `SIGS` must name a type
//! the stdlib really declares AND the one `PATH_RENAMES` maps the
//! user-facing path to. A mistyped name is not a compile error in
//! Rust — it is a nominal type nobody declared, which silently
//! reopens the permissive behavior this file exists to close.

use hale_syntax::parse_source;
use hale_types::stdlib_surface::{SigTy, SIGS};

/// Errors only, with their spans, so a pin can assert the
/// diagnostic is LOCATED and not a spanless whole-program gripe.
fn errors(src: &str) -> Vec<(String, (u32, u32))> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| (d.message, (d.span.start.0, d.span.end.0)))
        .collect()
}

fn messages(src: &str) -> Vec<String> {
    errors(src).into_iter().map(|(m, _)| m).collect()
}

/// Wrap a body in a `main` so the checker sees a whole program.
fn prog(body: &str) -> String {
    format!("main locus App {{\n fn main() {{\n{body}\n }}\n}}\n")
}

// ---------------------------------------------------------------
// the bug: a wrong field on each named result
// ---------------------------------------------------------------

/// One row per struct-returning helper family. `(call, wrong field,
/// public type name)` — the public spelling is what the diagnostic
/// must carry: the checker works in mangled names (`__JsonString`)
/// and a message naming one points at something that appears
/// nowhere in the user's source.
const WRONG_FIELD: &[(&str, &str, &str)] = &[
    (
        "std::json::string_field(\"{}\", \"a\")",
        "knd",
        "std::json::JsonString",
    ),
    (
        "std::json::find_field_range_in(\"{}\", \"a\", 0, 2)",
        "start_pos",
        "std::json::JsonFieldRange",
    ),
    (
        "std::json::array_first_span(\"[]\")",
        "finished",
        "std::json::ArrayIterSpan",
    ),
    (
        "std::json::object_first(\"{}\")",
        "key",
        "std::json::ObjectIterSpan",
    ),
    (
        "std::json::array_first(\"[]\")",
        "elem",
        "std::json::ArrayIter",
    ),
    (
        "std::http::parse_request(\"GET / HTTP/1.1\\r\\n\\r\\n\")",
        "verb",
        "std::http::Request",
    ),
];

#[test]
fn a_wrong_field_on_each_named_result_is_a_located_error() {
    for (call, field, ty) in WRONG_FIELD {
        let src = prog(&format!("let v = {call};\n let x = v.{field};"));
        let diags = errors(&src);
        let hit = diags.iter().find(|(m, _)| {
            m.contains(&format!("no field `{field}`"))
        });
        let (msg, span) = match hit {
            Some(h) => h,
            None => panic!(
                "`{call}` -> `.{field}` must be an error naming \
                 `{ty}`; got {diags:?}"
            ),
        };
        assert!(
            msg.contains(ty),
            "the diagnostic must name the PUBLIC type `{ty}`, not \
             the mangled one: {msg}"
        );
        assert!(
            span.1 > span.0,
            "the diagnostic must be located at the field access, \
             got an empty span {span:?} for {msg}"
        );
    }
}

#[test]
fn the_wrong_field_diagnostic_suggests_the_right_one() {
    // `knd` is one edit from `kind`, which is the whole point of
    // typing the result: the checker can now say what the author
    // meant. (Nothing else in this file depends on the hint, so a
    // future change to the suggestion heuristic fails here alone.)
    let src = prog(
        "let v = std::json::string_field(\"{}\", \"a\");\n \
         let x = v.knd;",
    );
    assert!(
        messages(&src).iter().any(|m| m.contains("did you mean `kind`")),
        "expected a did-you-mean for `kind`: {:?}",
        messages(&src)
    );
}

// ---------------------------------------------------------------
// the bug: arity and argument types through a named value
// ---------------------------------------------------------------

#[test]
fn arity_of_a_struct_returning_helper_is_checked() {
    // No row at all meant no arity check either — this compiled to
    // a call with a missing argument.
    let src = prog("let v = std::json::string_field(\"{}\");");
    assert!(
        messages(&src)
            .iter()
            .any(|m| m.contains("`std::json::string_field` takes 2")),
        "expected an arity error: {:?}",
        messages(&src)
    );
}

#[test]
fn a_named_value_in_the_wrong_argument_position_is_refused() {
    // The span-iterator helpers take (iter, json): swapping them
    // reads the iterator's fields off a String. Both directions are
    // now typed, so the checker catches it.
    let src = prog(
        "let it = std::json::array_first_span(\"[]\");\n \
         let n = std::json::array_next_span(\"[]\", it);",
    );
    let msgs = messages(&src);
    assert!(
        msgs.iter().any(|m| {
            m.contains("`std::json::array_next_span` argument 1")
                && m.contains("std::json::ArrayIterSpan")
        }),
        "expected argument 1 to be refused: {msgs:?}"
    );
}

#[test]
fn a_named_result_is_not_a_string() {
    // The result used to be Unknown, so it satisfied every
    // parameter. A `JsonString` is a struct — feeding it to a
    // String parameter is a real mistake.
    let src = prog(
        "let v = std::json::string_field(\"{}\", \"a\");\n \
         let n = std::json::find_int_field(v, \"a\");",
    );
    assert!(
        messages(&src)
            .iter()
            .any(|m| m.contains("`std::json::find_int_field` argument 1")),
        "expected the struct to be refused where a String is \
         wanted: {:?}",
        messages(&src)
    );
}

#[test]
fn a_declared_field_keeps_its_declared_type() {
    // `kind` is a String and `start` is an Int. With the result
    // typed, using one as the other is an error — the fields are
    // not just present, they carry their declared types.
    let src = prog(
        "let r = std::json::find_field_range_in(\"{}\", \"a\", 0, 2);\n \
         let n = std::json::find_int_field(r.start, \"a\");",
    );
    assert!(
        messages(&src)
            .iter()
            .any(|m| m.contains("`std::json::find_int_field` argument 1")),
        "an Int field used where a String is wanted must be an \
         error: {:?}",
        messages(&src)
    );
}

// ---------------------------------------------------------------
// the risk: the correct shapes still check
// ---------------------------------------------------------------

#[test]
fn the_real_walker_shapes_still_check_clean() {
    // The allocation-free object/array walk as `dna/core` and
    // `iris/inspect` write it: every helper in the new rows, used
    // correctly, in one program.
    let src = prog(
        "let doc = \"[{\\\"a\\\":1}]\";\n \
         let mut it = std::json::array_first_span(doc);\n \
         while !it.done {\n \
             let r = std::json::iter_find_field_range(it, doc, \"a\");\n \
             let s = std::json::iter_find_string_field_range(it, doc, \"a\");\n \
             if r.ok { println(to_string(r.start)); }\n \
             if s.ok { println(to_string(s.end_pos)); }\n \
             it = std::json::array_next_span(it, doc);\n \
         }\n \
         let mut ob = std::json::object_first(doc);\n \
         while !ob.done {\n \
             println(to_string(ob.key_start));\n \
             ob = std::json::object_next(ob, doc);\n \
         }\n \
         let mut ai = std::json::array_first(doc);\n \
         while !ai.done {\n \
             println(ai.element);\n \
             ai = std::json::array_next(ai);\n \
         }\n \
         let js = std::json::string_field(\"{}\", \"a\");\n \
         if js.kind == \"string\" { println(js.text); }\n \
         let fr = std::json::find_field_range_in(doc, \"a\", 0, 2);\n \
         println(to_string(fr.end_pos));",
    );
    assert!(
        messages(&src).is_empty(),
        "the correct walker must still check clean: {:?}",
        messages(&src)
    );
}

#[test]
fn the_parsed_request_shape_still_checks_clean() {
    let src = prog(
        "let req = std::http::parse_request(\"GET / HTTP/1.1\\r\\n\\r\\n\");\n \
         println(req.method);\n \
         println(req.path);",
    );
    assert!(
        messages(&src).is_empty(),
        "the documented parse_request shape must still check \
         clean: {:?}",
        messages(&src)
    );
}

// ---------------------------------------------------------------
// the typo guard
// ---------------------------------------------------------------

/// Every `SigTy::Named` name in the table, from either position.
fn named_in_sigs() -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    for sig in SIGS {
        if let SigTy::Named(n) = sig.ret {
            out.push((sig.display_path(), n));
        }
        for p in sig.params {
            if let SigTy::Named(n) = p {
                out.push((sig.display_path(), n));
            }
        }
    }
    out
}

#[test]
fn every_named_sig_type_is_declared_by_the_stdlib() {
    let program = parse_source(hale_stdlib::AP_SOURCE)
        .expect("the bundled stdlib source must parse");
    let mut declared = std::collections::BTreeSet::new();
    for item in &program.items {
        match item {
            hale_syntax::ast::TopDecl::Locus(l) => {
                declared.insert(l.name.name.clone());
            }
            hale_syntax::ast::TopDecl::Type(t) => {
                declared.insert(t.name.name.clone());
            }
            hale_syntax::ast::TopDecl::Interface(i) => {
                declared.insert(i.name.name.clone());
            }
            _ => {}
        }
    }
    let missing: Vec<String> = named_in_sigs()
        .into_iter()
        .filter(|(_, n)| !declared.contains(*n))
        .map(|(p, n)| format!("{p} -> {n}"))
        .collect();
    assert!(
        missing.is_empty(),
        "a `Named(..)` row naming a type the stdlib does not \
         declare types as a nominal nobody has — which is the \
         permissive behavior GH #771 closed, back again and \
         invisible: {missing:?}"
    );
}

#[test]
fn every_named_sig_type_is_the_rename_target_users_can_spell() {
    // The user writes `std::json::JsonString`; `resolve_type_expr`
    // maps it through `PATH_RENAMES` to `__JsonString`. If a row
    // names something that is not a rename TARGET, the two
    // spellings never meet and an annotated `let x:
    // std::json::JsonString = ...` would not unify with the call
    // it is assigned from.
    let targets: std::collections::BTreeSet<&str> = hale_stdlib::PATH_RENAMES
        .iter()
        .map(|(_, target)| *target)
        .collect();
    let orphans: Vec<String> = named_in_sigs()
        .into_iter()
        .filter(|(_, n)| !targets.contains(*n))
        .map(|(p, n)| format!("{p} -> {n}"))
        .collect();
    assert!(
        orphans.is_empty(),
        "`Named(..)` names with no PATH_RENAMES entry — users have \
         no way to spell these types, so the row cannot unify with \
         an annotation: {orphans:?}"
    );
}

#[test]
fn an_annotation_unifies_with_the_call_it_names() {
    // The end-to-end consequence of the guard above: the two
    // spellings really are one type.
    let src = prog(
        "let v: std::json::JsonString = \
         std::json::string_field(\"{}\", \"a\");\n \
         println(v.kind);",
    );
    assert!(
        messages(&src).is_empty(),
        "the annotated and inferred spellings must unify: {:?}",
        messages(&src)
    );
}
