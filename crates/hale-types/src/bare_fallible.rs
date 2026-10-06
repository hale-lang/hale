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

use crate::typed_bodies::{CalleeKind, ConversionKind, ConversionRow, ConversionSite, FallibleCall, Handling, SiteKind, TypedBodies};

/// Every fallible call `table` holds that nothing handles, as errors;
/// then every narrowing nothing discharges (GH #1076, U2), from the
/// `conversions` column, once per conversion: a conversion in a default
/// has a row per evaluation (its `ConversionSite::path`), and what
/// discharges it is written in the default, the same in each. A quantity's narrowing
/// (U3) is judged through the same filter; two different judgments at
/// one span (a division by a literal that also flows into a narrower
/// denomination) both stand.
pub fn bare_fallible_calls(table: &TypedBodies) -> Vec<Diag> {
    let mut judged = std::collections::BTreeSet::new();
    let narrowings = table
        .conversion_sites()
        .filter_map(judge_narrowing)
        .filter(|d| judged.insert((d.span.start.as_usize(), d.span.end.as_usize(), d.message.clone())));
    table.fallible_calls().filter_map(judge).chain(narrowings).collect()
}

/// A narrowing (`Session(n)`) is fallible like a call: a value outside
/// the range has to become something, and the program says what. A
/// conversion between denominations that divides (U3) has a remainder
/// to say something about, at the site (`or floor`) or by the target
/// type's `round:`.
fn judge_narrowing((site, row): (ConversionSite, &ConversionRow)) -> Option<Diag> {
    if row.kind != ConversionKind::Narrowing || row.policy.is_some() {
        return None;
    }
    if let Some(scale) = &row.scale {
        let divisor = crate::unit_quantities::grouped(scale.factor.denominator());
        let declared = !row.target.contains(" in ");
        let policy = if declared { format!(", or give `{}` a `round:` policy", row.target) } else { String::new() };
        let say = match site.kind {
            // An implicit conversion has no `or` of its own.
            SiteKind::Value { .. } => format!(
                "say what happens to the remainder: convert explicitly (`.in(u) or floor`, `{}(…) or half_even`, \
                 `or <value>`, `or raise`){policy}",
                row.target
            ),
            _ => format!("say what happens to the remainder: `or floor`, `or <value>`, `or raise`{policy}"),
        };
        let what = match site.kind {
            SiteKind::Divide { .. } => format!("`{}` divided by {divisor} leaves a remainder", row.target),
            _ => format!("`{}` from `{}` divides by {divisor}", row.target, row.from.display()),
        };
        return Some(Diag::ty(row.span, format!("{what}: {say}")));
    }
    let range = row.range.map(|(lo, hi)| format!("`{lo}..{hi}`")).unwrap_or_default();
    Some(Diag::ty(
        row.span,
        format!(
            "`{t}(…)` narrows `{from}` into `{t}`'s range {range} and this conversion says nothing \
             about a value outside it: write `or <fallback>` for a value to use instead, `or clamp` \
             for the nearest bound, `or wrap` to wrap around the range, `or handler(err)` to deal \
             with the `RangeError` here, or `or raise` to hand it to the caller",
            t = row.target,
            from = row.from.display(),
        ),
    ))
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
