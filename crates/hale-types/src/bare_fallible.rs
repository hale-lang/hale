//! A fallible call is handled only by an `or`, and a call that says
//! nothing about its failure is an **error** (GH #738), user or stdlib
//! alike (F.40 phase 3, E4).
//!
//! One law over the typed-body table's `fallible_calls` column: the
//! checker records every call whose callee is fallible, with what
//! addresses it where it stands ([`Handling`]), and this reads the rows.
//! A call that is the operand of an `or` (`or raise`, `or <fallback>`,
//! `or handler(err)`, `or fail <payload>`, `or discard`) is handled.
//! Every other position — an argument, an operand, a `match` scrutinee,
//! a `let` initializer, a statement, a returned value — is the bare
//! call, refused by `hale check`, `verify`, `build`, `run` and `test`
//! alike, with no flag and no legacy mode. A `match` does not handle a
//! fallible call: lowering never lowered one there.
//!
//! The handler of an `or` takes the implicit `or raise` where lowering
//! supports it, which is a limitation to lift rather than a rule: a fn
//! or locus method the program declares ([`CalleeKind::Declared`]). Any
//! other fallible handler is refused with the nested spelling that
//! works, `or (h(err) or raise)`.

use hale_syntax::error::Diag;

use crate::typed_bodies::{CalleeKind, FallibleCall, Handling, TypedBodies};

/// Every fallible call `table` holds that nothing handles, as errors.
pub fn bare_fallible_calls(table: &TypedBodies) -> Vec<Diag> {
    table.fallible_calls().filter_map(judge).collect()
}

fn judge(row: &FallibleCall) -> Option<Diag> {
    match (row.handled, row.kind) {
        (Handling::Or, _) | (Handling::Handler(_), CalleeKind::Declared) => None,
        (Handling::Handler(or_span), CalleeKind::Stdlib) => Some(Diag::ty(
            or_span,
            format!(
                "`or {c}(...)`: a fallible stdlib call can't be the handler \
                 directly yet — write the nested form `or ({c}(...) or \
                 raise)` so its own failure has a path",
                c = row.callee
            ),
        )),
        (Handling::Handler(or_span), CalleeKind::Typed) => Some(Diag::ty(
            or_span,
            format!(
                "`or {c}(...)`: only a fallible fn or locus method the \
                 program declares can be the handler directly yet — write \
                 the nested form `or ({c}(...) or raise)` so its own failure \
                 has a path",
                c = row.callee
            ),
        )),
        (Handling::Bare, _) => Some(Diag::ty(
            row.span,
            format!(
                "`{}` can fail ({}) and this call says nothing \
                 about it: write `or raise` to hand the failure \
                 to the caller, `or <fallback>` for a value to \
                 use instead, `or handler(err)` to deal with it \
                 here, or `or discard` when losing it is the \
                 intent. A bare call to a fallible entry point \
                 is an error since v0.22.0 (GH #738).",
                row.callee,
                row.payload.display()
            ),
        )),
    }
}
