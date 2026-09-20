//! GH #829: a stdlib call's ARGUMENT TYPES are checked where the
//! row declares one.
//!
//! `std::io::tcp::recv_into(0, 0, 64)` typechecked. It cannot run:
//! `buf` is a `std::bytes::BytesBuilder` locus whose internal handle
//! the lowering GEPs out, and `lower_recv_into_common` refuses
//! anything else — late, from codegen, with no span:
//!
//! ```text
//! codegen error: unsupported in codegen v0: std::io::tcp::recv_into:
//! buf must be std::bytes::BytesBuilder (constructed via
//! `std::bytes::BytesBuilder { initial_cap: ... }`), got Int
//! ```
//!
//! The checker HAS an argument-type check (`SigTy::accepts`, run per
//! argument at every tabled path-call). What it did not have was a
//! row that said anything: the `recv_into` family declared its
//! buffer `Any`, which types as `Ty::Unknown` and accepts every
//! value in the language. The four rows now name the type, and this
//! file pins both directions.
//!
//! The permissiveness is deliberate and stays: `Unknown` on either
//! side of the comparison is accepted, because an absent or
//! deliberately-polymorphic row must never invent a false error on
//! valid code (a single file the checker cannot see all of is the
//! normal case, not an exception). `Any` rows — the `@form(vec)`
//! targets of `split_into` / `join` / `tokenize_words_into` — are
//! pinned permissive below so tightening one is a deliberate act.

use hale_syntax::parse_source;

/// Errors only, with spans, so a pin can assert the diagnostic is
/// LOCATED rather than a spanless whole-program gripe.
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

/// Wrap a body in a whole program.
///
/// Every program in this file is built from PLAIN string literals,
/// never `r#"…"#`: the corpus harvester scrapes raw literals out of
/// test sources, and the wrong-typed programs here are written to be
/// refused. Do not "tidy" them into raw strings.
fn prog(body: &str) -> String {
    format!("fn main() {{\n{body}\n}}\n")
}

/// The rows GH #829 typed: every stdlib fn whose lowering is
/// `lower_recv_into_common`, which accepts exactly one codegen type
/// in `buf` and refuses the rest.
const RECV_INTO_FAMILY: &[&str] = &[
    "std::io::tcp::recv_into",
    "std::io::tcp::recv_stamped_into",
    "std::io::tls::recv_into",
    "std::io::tls::recv_stamped_into",
    "std::io::udp::recv_into",
];

// ---------------------------------------------------------------
// the bug: a wrong-typed argument was accepted
// ---------------------------------------------------------------

#[test]
fn an_int_where_a_builder_is_declared_is_a_located_error() {
    for path in RECV_INTO_FAMILY {
        let src = prog(&format!("let n = {path}(0, 0, 64);"));
        let diags = errors(&src);
        let hit = diags
            .iter()
            .find(|(m, _)| m.contains(&format!("`{path}` argument 2")));
        let (msg, span) = match hit {
            Some(h) => h,
            None => panic!(
                "`{path}(0, 0, 64)` must be refused at argument 2 — \
                 codegen refuses it, so accepting it here tells the \
                 author their working program is broken, late, from \
                 another layer; got {diags:?}"
            ),
        };
        assert!(
            msg.contains("expected `std::bytes::BytesBuilder`")
                && msg.contains("got `Int`"),
            "the diagnostic must name both types in the spelling the \
             author writes: {msg}"
        );
        assert!(
            !msg.contains("__Std"),
            "a message naming the mangled type points at a symbol that \
             appears nowhere in the author's source: {msg}"
        );
        assert!(
            span.1 > span.0,
            "the diagnostic must be located at the argument, got an \
             empty span {span:?} for {msg}"
        );
    }
}

#[test]
fn a_string_where_a_builder_is_declared_is_refused_too() {
    // Not just the `0` in the issue: any concrete non-builder value.
    let src = prog("let n = std::io::tcp::recv_into(0, \"buf\", 64);");
    assert!(
        messages(&src).iter().any(|m| {
            m.contains("`std::io::tcp::recv_into` argument 2")
                && m.contains("got `String`")
        }),
        "a String buffer must be refused: {:?}",
        messages(&src)
    );
}

// ---------------------------------------------------------------
// the risk of the fix: the correct shapes still check
// ---------------------------------------------------------------

#[test]
fn a_real_builder_checks_clean() {
    for path in RECV_INTO_FAMILY {
        let src = prog(&format!(
            "let b = std::bytes::BytesBuilder {{ initial_cap: 64 }};\n\
             let n = {path}(0, b, 64);"
        ));
        assert!(
            messages(&src).is_empty(),
            "the documented `{path}` shape must still check clean: {:?}",
            messages(&src)
        );
    }
}

#[test]
fn a_builder_reached_through_a_field_or_a_parameter_checks_clean() {
    // The two shapes the corpus actually writes: a reused builder
    // held as a locus param, and one threaded through a helper fn.
    // Both must unify with the row's mangled name, or the fix would
    // be a false-error machine on every real recv loop.
    let src = "\
fn pump(fd: Int, buf: std::bytes::BytesBuilder) -> Int {
    return std::io::tcp::recv_into(fd, buf, 64);
}

locus Gateway {
    params { buf: std::bytes::BytesBuilder = std::bytes::BytesBuilder { initial_cap: 64 }; }
    run() {
        let n = std::io::udp::recv_into(0, self.buf, 64);
        let m = pump(0, self.buf);
    }
}

main locus App {
    params { gw: Gateway = Gateway { }; }
}

fn main() { App { }; }
";
    assert!(
        messages(src).is_empty(),
        "a builder held in a param and passed to a helper must check \
         clean: {:?}",
        messages(src)
    );
}

// ---------------------------------------------------------------
// the permissiveness that stays
// ---------------------------------------------------------------

#[test]
fn an_unknown_typed_argument_stays_permissive() {
    // A BARE (no `or`) fallible stdlib call types `Unknown` on
    // purpose — the legacy direct form's return differs per fn and
    // the table does not model it. A value the checker cannot see
    // the type of must not be refused: that is the rule that keeps
    // an incomplete table from inventing errors on valid code.
    let src = prog(
        "let opaque = std::io::fs::read_file(\"f\");\n\
         let n = std::io::tcp::recv_into(0, opaque, 64);",
    );
    assert!(
        !messages(&src)
            .iter()
            .any(|m| m.contains("`std::io::tcp::recv_into` argument 2")),
        "an Unknown-typed argument must stay permissive: {:?}",
        messages(&src)
    );
}

#[test]
fn an_any_row_stays_permissive() {
    // `split_into`'s third argument is a user `@form(vec)` locus —
    // a type the table cannot name, so the row says `Any` and every
    // value passes. Tightening one of these is a deliberate act with
    // its own evidence, not a side effect of #829.
    let src = prog("std::str::split_into(\"a,b\", \",\", 0);");
    assert!(
        !messages(&src)
            .iter()
            .any(|m| m.contains("`std::str::split_into` argument 3")),
        "an `Any` row must stay permissive: {:?}",
        messages(&src)
    );
}

#[test]
fn arity_is_still_checked_alongside_the_types() {
    // The arity check and the type check are the same loop; one
    // must not shadow the other.
    let src = prog("let n = std::io::tcp::recv_into(0);");
    assert!(
        messages(&src)
            .iter()
            .any(|m| m.contains("`std::io::tcp::recv_into` takes 3 arguments, got 1")),
        "expected the arity error: {:?}",
        messages(&src)
    );
}
