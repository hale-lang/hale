//! The `closures` family's recovery events (F.40 phase 4, W3).
//!
//! A closure may say which recovery events its accumulators survive
//! (`persists_through(...)`) or reset on (`resets_on(...)`). The parser
//! types each name once ([`RecoveryEvents`]): the event it is when the
//! name is in the closed alphabet (`restart`, `restart_in_place`,
//! `quarantine`), the name as written either way. [`closure_event_rows`]
//! makes one row per such clause; the laws here judge the rows, each a
//! registered rule of `spec/verification.md`'s structural table, and
//! [`closure_event_laws`] is the one entry the check runs. Lowering reads
//! the same typed list (`ClosureDecl::persists_through`), so what the
//! laws accept is what runs.

use hale_syntax::ast::{flat_decls, ClosureClause, ClosureDecl, LocusDecl, LocusMember, RecoveryEvent, RecoveryEvents, TopDecl};
use hale_syntax::Diag;

use crate::law::{Law, RuleId, Severity, Violation};
use crate::Bundle;

/// A name outside the alphabet.
const ALPHABET: RuleId = RuleId::registered("verification/structural", "recovery-event-alphabet");
/// `dissolve` in `persists_through(...)`.
const DISSOLVE: RuleId = RuleId::registered("verification/structural", "persist-through-dissolve");

/// Which clause a row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clause {
    PersistsThrough,
    ResetsOn,
}

impl Clause {
    /// The clause's keyword.
    pub fn keyword(self) -> &'static str {
        match self {
            Clause::PersistsThrough => "persists_through",
            Clause::ResetsOn => "resets_on",
        }
    }
}

/// One `persists_through(...)` or `resets_on(...)` clause of a closure.
#[derive(Debug, Clone, Copy)]
pub struct ClauseRow<'b> {
    /// The locus that declares the closure.
    pub locus: &'b LocusDecl,
    pub closure: &'b ClosureDecl,
    pub clause: Clause,
    /// The clause's list, typed by the parser, and its span.
    pub events: &'b RecoveryEvents,
}

/// Every recovery-event clause of a bundle's loci.
#[derive(Debug, Clone, Default)]
pub struct ClosureEventRows<'b> {
    pub rows: Vec<ClauseRow<'b>>,
}

/// The `closures` family's clause rows: every `persists_through(...)` and
/// `resets_on(...)` clause of every locus the bundle's programs declare
/// (module-nested ones included), in program order, then declaration
/// order, then clause order.
pub fn closure_event_rows<'b>(bundle: &Bundle<'b>) -> ClosureEventRows<'b> {
    let mut rows = Vec::new();
    for program in bundle.programs.values().copied() {
        for item in flat_decls(&program.items) {
            let TopDecl::Locus(locus) = item else { continue };
            for member in &locus.members {
                let LocusMember::Closure(closure) = member else { continue };
                for c in &closure.clauses {
                    let (clause, events) = match c {
                        ClosureClause::PersistsThrough(events) => (Clause::PersistsThrough, events),
                        ClosureClause::ResetsOn(events) => (Clause::ResetsOn, events),
                        ClosureClause::Epoch(_) | ClosureClause::ResetsPerEpoch(_) | ClosureClause::Captures(_) => {
                            continue
                        }
                    };
                    rows.push(ClauseRow { locus, closure, clause, events });
                }
            }
        }
    }
    ClosureEventRows { rows }
}

/// Every recovery-event law over `bundle`'s clauses, as diagnostics.
pub fn closure_event_laws(bundle: &Bundle<'_>) -> Vec<Diag> {
    let rows = closure_event_rows(bundle);
    let mut diags = Law { rule: ALPHABET, eval: outside_the_alphabet }.diags(&rows);
    diags.extend(Law { rule: DISSOLVE, eval: persists_through_dissolve }.diags(&rows));
    diags
}

/// The alphabet as a message names it.
fn alphabet() -> String {
    let names: Vec<String> = RecoveryEvent::ALL.iter().map(|e| format!("`{}`", e.name())).collect();
    format!("{} or {}", names[..names.len() - 1].join(", "), names[names.len() - 1])
}

/// A name outside the alphabet is an error at the name, naming the
/// alphabet; a name one edit away from an event suggests it. `dissolve`
/// in `persists_through(...)` is [`persists_through_dissolve`]'s.
fn outside_the_alphabet(rows: &ClosureEventRows<'_>, out: &mut Vec<Violation>) {
    for row in &rows.rows {
        for n in row.events.names.iter().filter(|n| n.event.is_none()) {
            let written = n.name.name.as_str();
            if row.clause == Clause::PersistsThrough && written == "dissolve" {
                continue;
            }
            let suggestion = RecoveryEvent::ALL
                .into_iter()
                .find(|e| one_edit_apart(written, e.name()))
                .map(|e| format!("; did you mean `{}`?", e.name()))
                .unwrap_or_default();
            out.push(Violation {
                rule: ALPHABET,
                severity: Severity::Error,
                span: n.name.span,
                message: format!(
                    "closure `{}`: `{}` is not a recovery event: `{}(...)` names {}{}",
                    row.closure.name.name,
                    written,
                    row.clause.keyword(),
                    alphabet(),
                    suggestion,
                ),
                witness: Vec::new(),
            });
        }
    }
}

/// `persists_through(dissolve)` can mean nothing: an accumulator does not
/// outlive its locus's dissolve. An error at the name.
fn persists_through_dissolve(rows: &ClosureEventRows<'_>, out: &mut Vec<Violation>) {
    for row in rows.rows.iter().filter(|r| r.clause == Clause::PersistsThrough) {
        for n in row.events.names.iter().filter(|n| n.name.name == "dissolve") {
            out.push(Violation {
                rule: DISSOLVE,
                severity: Severity::Error,
                span: n.name.span,
                message: format!(
                    "closure `{}`: an accumulator does not outlive its locus's dissolve, so \
                     `persists_through(dissolve)` can mean nothing: `persists_through(...)` names {}",
                    row.closure.name.name,
                    alphabet(),
                ),
                witness: Vec::new(),
            });
        }
    }
}

/// Whether `a` becomes `b` by one insertion, deletion, substitution or
/// swap of two adjacent characters.
fn one_edit_apart(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a == b {
        return false;
    }
    let (short, long) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };
    match long.len() - short.len() {
        0 => {
            let differ: Vec<usize> = (0..a.len()).filter(|&i| a[i] != b[i]).collect();
            match differ[..] {
                [_] => true,
                [i, j] => j == i + 1 && a[i] == b[j] && a[j] == b[i],
                _ => false,
            }
        }
        1 => {
            let i = (0..short.len()).find(|&i| short[i] != long[i]).unwrap_or(short.len());
            short[i..] == long[i + 1..]
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::one_edit_apart;

    #[test]
    fn one_edit_is_an_insertion_a_deletion_a_substitution_or_a_swap() {
        assert!(one_edit_apart("quarantin", "quarantine"));
        assert!(one_edit_apart("quarantinee", "quarantine"));
        assert!(one_edit_apart("restarr", "restart"));
        assert!(one_edit_apart("rsetart", "restart"));
        assert!(!one_edit_apart("restart", "restart"));
        assert!(!one_edit_apart("dissolve", "restart"));
        assert!(!one_edit_apart("rstrt", "restart"));
        assert!(!one_edit_apart("restart_in_plcae_", "restart_in_place"));
    }
}
