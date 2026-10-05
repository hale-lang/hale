//! The sealed rule (GH #436) over the `param_accesses` rows (F.40 phase
//! 4, W4).
//!
//! A `@sealed` locus's `params` are reachable only from inside its own
//! methods. The checker records every access to a locus's params as a
//! row of the typed bodies ([`crate::typed_bodies::ParamAccess`]), sealed
//! or not, where it types the access; this law reads the rows and finds
//! the ones that break the rule: the receiver is sealed and the reader is
//! not the receiver. The rule is about the reader, not the receiver
//! syntax: `self.key` inside `Signer` and `self.s.key` in a parent both
//! have receiver type `Signer`, and only the second is read from outside.
//!
//! Only `params` are confined: a capacity slot or a method named on a
//! locus is no row. Sealing confines state, it does not make a locus
//! uncallable.

use hale_syntax::Diag;

use crate::law::{Law, RuleId, Severity, Violation};
use crate::resolve::TopScope;
use crate::symbol::TopSymbol;
use crate::typed_bodies::{AccessKind, ParamAccess};

/// `@sealed` confinement.
const CONFINEMENT: RuleId = RuleId::registered("verification/structural", "sealed-confinement");

/// What the law reads: access rows, and the scope that says which of
/// their receivers are sealed and what each one declares.
pub struct SealedAccessRows<'a> {
    pub top: &'a TopScope,
    pub rows: &'a [ParamAccess],
}

/// The law's findings over `rows`, each with the index of the row it
/// judged, in row order. The check places each where its walk first
/// reached the access.
pub fn sealed_access_law(top: &TopScope, rows: &[ParamAccess]) -> Vec<(usize, Diag)> {
    let law = Law { rule: CONFINEMENT, eval: outside_access };
    rows.iter()
        .enumerate()
        .flat_map(|(i, row)| {
            law.diags(&SealedAccessRows { top, rows: std::slice::from_ref(row) }).into_iter().map(move |d| (i, d))
        })
        .collect()
}

/// A row whose receiver is `@sealed`, read or written from outside the
/// receiver's own members: an error at the access, naming the methods to
/// call instead.
fn outside_access(rows: &SealedAccessRows<'_>, out: &mut Vec<Violation>) {
    for row in rows.rows {
        if row.from_inside() {
            continue;
        }
        let Some(TopSymbol::Locus(li)) = rows.top.symbols.get(&row.locus) else { continue };
        if !li.sealed {
            continue;
        }
        // Render the spelling the author wrote. A stdlib locus is
        // declared under a mangled name (`__StdSecretSigner`) that
        // appears nowhere in their program; they wrote
        // `std::secret::Signer`.
        let shown = shown_name(&li.name);
        let callable: Vec<&str> = li.methods.iter().map(|m| m.name.as_str()).collect();
        let hint = if callable.is_empty() {
            format!(
                "`{shown}` declares no methods, so its state is \
                 reachable only from inside it"
            )
        } else {
            format!("call one of its methods instead ({})", callable.join(", "))
        };
        let (verb, gerund) = match row.kind {
            AccessKind::Read => ("readable", "reads"),
            AccessKind::Write => ("writable", "writes"),
        };
        out.push(Violation {
            rule: CONFINEMENT,
            severity: Severity::Error,
            span: row.span,
            message: format!(
                "`{shown}` is `@sealed`: its `params` are {verb} only \
                 from inside its own methods, and `{shown}.{}` {gerund} \
                 one from outside — {hint}",
                row.param
            ),
            witness: Vec::new(),
        });
    }
}

/// A locus's name as the author spells it: a `std::` locus by its path,
/// any other by its declared name.
pub fn shown_name(locus: &str) -> String {
    hale_stdlib::PATH_RENAMES
        .iter()
        .find(|(_, m)| *m == locus)
        .map(|(p, _)| p.join("::"))
        .unwrap_or_else(|| locus.to_string())
}
