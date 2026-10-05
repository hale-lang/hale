//! The unit dialect's checks (GH #1076).
//!
//! Step U1 parses the dialect's declarations (`unit`, the scalar body of
//! a `type`) and its quantity literal, and checks none of them yet: the
//! checker refuses each with the one located error below. The rows and
//! the declaration laws replace this function.

use hale_syntax::{Diag, Span};

/// The one error a unit-dialect declaration or quantity literal gets
/// until its rows and laws land: `what` names it (`unit \`ms\``).
pub(crate) fn not_yet_checked(span: Span, what: &str) -> Diag {
    Diag::ty(
        span,
        format!(
            "{what}: the unit dialect's declarations are parsed and not yet \
             checked (GH #1076)"
        ),
    )
}
