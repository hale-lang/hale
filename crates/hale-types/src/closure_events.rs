//! The `closures` family's recovery events (F.40 phase 4, W3).
//!
//! A closure may say which recovery events its accumulators survive
//! (`persists_through(...)`) or reset on (`resets_on(...)`). The parser
//! types each name once ([`RecoveryEvents`]): the event it is when the
//! name is in the closed alphabet (`restart`, `restart_in_place`,
//! `quarantine`), the name as written either way. [`closure_event_rows`]
//! makes one row per such clause; the laws here judge the rows, each a
//! registered rule of `spec/verification.md`'s structural table, and
//! [`closure_event_laws`] is the one entry the check runs for the clause
//! rows alone. Whether a recovery reaches a locus reads what the checker
//! typed (the typed bodies' `recoveries`), so [`unreached_event_laws`]
//! runs over the whole typed-body table, beside the `bare_fallible` law.
//! Lowering reads the same typed list (`ClosureDecl::persists_through`),
//! so what the laws accept is what runs.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{
    flat_decls, ClosureClause, ClosureDecl, LocusDecl, LocusMember, RecoveryEvent, RecoveryEvents, RecoveryOp, TopDecl,
};
use hale_syntax::{Diag, SpanOrigin};

use crate::entry::EntryRow;
use crate::handler_routing::{ChildRef, HandlerRouting, HandlerRow};
use crate::law::{Law, RuleId, Severity, Violation, WitnessStep};
use crate::placement::{SiteRef, SiteUniverse};
use crate::ty::Ty;
use crate::typed_bodies::{accumulator_sites, RecoveryRow, TypedBodies};
use crate::Bundle;

/// A name outside the alphabet.
const ALPHABET: RuleId = RuleId::registered("verification/structural", "recovery-event-alphabet");
/// `dissolve` in `persists_through(...)`.
const DISSOLVE: RuleId = RuleId::registered("verification/structural", "persist-through-dissolve");
/// An event in both clauses of one closure.
const CONTRADICTION: RuleId = RuleId::registered("verification/structural", "contradicting-recovery-clauses");
/// An event the closed world never applies to the locus.
const UNREACHED: RuleId = RuleId::registered("verification/structural", "unreached-recovery-event");
/// `persists_through(...)` on a closure that accumulates nothing.
const NOTHING_KEPT: RuleId = RuleId::registered("verification/structural", "persistence-without-accumulator");

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

/// What the reach law reads beside the clauses: the handler rows (who
/// applies which recovery to a child of which type), the typed bodies
/// (each recovery statement outside a handler, with the locus the
/// checker typed its receiver as), the entry row (whether the world is
/// closed), and the bundle's declarations (whose generic params a
/// handler's child may be).
struct Reach<'r, 'b> {
    clauses: &'r ClosureEventRows<'b>,
    handlers: &'r HandlerRouting,
    typed: &'r TypedBodies,
    entry: &'r EntryRow,
    bundle: &'r Bundle<'b>,
}

/// The recovery-event laws over `bundle`'s clauses alone, as
/// diagnostics: every one but the reach law ([`unreached_event_laws`]).
pub fn closure_event_laws(bundle: &Bundle<'_>) -> Vec<Diag> {
    let rows = closure_event_rows(bundle);
    let mut diags = Law { rule: ALPHABET, eval: outside_the_alphabet }.diags(&rows);
    diags.extend(Law { rule: DISSOLVE, eval: persists_through_dissolve }.diags(&rows));
    diags.extend(Law { rule: CONTRADICTION, eval: in_both_clauses }.diags(&rows));
    diags.extend(Law { rule: NOTHING_KEPT, eval: nothing_to_keep }.diags(&rows));
    diags
}

/// The reach law over `bundle`'s clauses, the handler rows and the whole
/// typed-body table `typed` (a typing that reused a declaration holds no
/// record of its bodies, so the law reads the table, never the check's
/// partial record).
pub fn unreached_event_laws(
    bundle: &Bundle<'_>,
    handlers: &HandlerRouting,
    entry: &EntryRow,
    typed: &TypedBodies,
) -> Vec<Diag> {
    let rows = closure_event_rows(bundle);
    let reach = Reach { clauses: &rows, handlers, typed, entry, bundle };
    Law { rule: UNREACHED, eval: unreached_events }.diags(&reach)
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

/// `resets_on(...)` states the default (an accumulator resets on every
/// recovery event its closure does not persist through), so an event a
/// closure names in both clauses contradicts itself. An error at each
/// `resets_on` name whose event the closure also persists through, the
/// first time the closure's `resets_on` clauses name it; the witness is
/// where `persists_through` names it.
fn in_both_clauses(rows: &ClosureEventRows<'_>, out: &mut Vec<Violation>) {
    for (i, row) in rows.rows.iter().enumerate().filter(|(_, r)| r.clause == Clause::ResetsOn) {
        let same_closure = |r: &&ClauseRow<'_>| std::ptr::eq(r.closure, row.closure);
        let persisted = rows
            .rows
            .iter()
            .filter(same_closure)
            .filter(|r| r.clause == Clause::PersistsThrough)
            .flat_map(|r| r.events.names.iter());
        let earlier_resets: Vec<RecoveryEvent> = rows.rows[..i]
            .iter()
            .filter(same_closure)
            .filter(|r| r.clause == Clause::ResetsOn)
            .flat_map(|r| r.events.events())
            .collect();
        let mut seen = earlier_resets;
        for n in &row.events.names {
            let Some(event) = n.event else { continue };
            if seen.contains(&event) {
                continue;
            }
            seen.push(event);
            let Some(kept) = persisted.clone().find(|p| p.event == Some(event)) else { continue };
            out.push(Violation {
                rule: CONTRADICTION,
                severity: Severity::Error,
                span: n.name.span,
                message: format!(
                    "closure `{}`: `{}` is in both `persists_through(...)` and `resets_on(...)`, which \
                     contradict each other: its accumulators either survive `{}` or reset on it",
                    row.closure.name.name,
                    event.name(),
                    event.name(),
                ),
                witness: vec![WitnessStep {
                    span: kept.name.span,
                    origin: SpanOrigin::Seed,
                    note: format!("`persists_through` names `{}` here", event.name()),
                }],
            });
        }
    }
}

/// The recovery event a statement applies: its own, for the three in the
/// alphabet (`reorganize` and `bubble` apply none to the child).
fn event_of(op: RecoveryOp) -> Option<RecoveryEvent> {
    match op {
        RecoveryOp::Restart => Some(RecoveryEvent::Restart),
        RecoveryOp::RestartInPlace => Some(RecoveryEvent::RestartInPlace),
        RecoveryOp::Quarantine => Some(RecoveryEvent::Quarantine),
        RecoveryOp::Reorganize | RecoveryOp::Bubble => None,
    }
}

/// The events a handler row applies to its child: its ops', and
/// `quarantine` when a restart states a `for` bound (a spent bound
/// quarantines).
fn handler_events(row: &HandlerRow) -> BTreeSet<RecoveryEvent> {
    let mut out: BTreeSet<RecoveryEvent> = row.ops.iter().filter_map(|op| event_of(*op)).collect();
    if row.bounds.iter().any(|b| matches!(b.op, RecoveryOp::Restart | RecoveryOp::RestartInPlace)) {
        out.insert(RecoveryEvent::Quarantine);
    }
    out
}

/// The events a recovery statement outside the handlers applies.
fn statement_events(row: &RecoveryRow) -> BTreeSet<RecoveryEvent> {
    let mut out: BTreeSet<RecoveryEvent> = event_of(row.op).into_iter().collect();
    if row.bounded && matches!(row.op, RecoveryOp::Restart | RecoveryOp::RestartInPlace) {
        out.insert(RecoveryEvent::Quarantine);
    }
    out
}

/// The events, as the witness says them.
fn spelled(events: &BTreeSet<RecoveryEvent>) -> String {
    let names: Vec<String> = events.iter().map(|e| format!("`{}`", e.name())).collect();
    match names.len() {
        0 => "no recovery event".to_string(),
        1 => names[0].clone(),
        n => format!("{} and {}", names[..n - 1].join(", "), names[n - 1]),
    }
}

/// Whether a child, as a row names it, is `locus`: by the declaration
/// the child resolves to when the row has it (a monomorph's is its
/// template's), else by name.
fn is_locus(child: &ChildRef, child_decl: Option<SiteRef>, locus: &LocusDecl, site: Option<SiteRef>) -> bool {
    match (child_decl, site) {
        (Some(c), Some(l)) => c == l,
        _ => matches!(child, ChildRef::Locus(n) if *n == locus.name.name),
    }
}

/// Whether a recovery statement's typed receiver is `locus`: by the
/// declaration the checker typed it as (a monomorph's template), else,
/// in a bundle no mint numbered, by the name the type has.
fn receives(row: &RecoveryRow, locus: &LocusDecl) -> bool {
    let Some(child) = row.child else { return false };
    if locus.id.is_none() {
        return matches!(&row.receiver, Ty::Named(n) if *n == locus.name.name);
    }
    child.universe == SiteUniverse::User && child.decl.0 == locus.id.0
}

/// An event a closure names that no recovery in the closed world applies
/// to its locus, for a locus of the program's own seed: a warning at the
/// name, whose witness is every handler and recovery statement that
/// names the locus with the events it applies (or the locus, when none
/// does). A spent `restart(c) for N` bound is `quarantine`. A recovery
/// statement outside the handlers names the locus the checker typed its
/// receiver as, whatever the receiver is (a param, a local, a field of
/// another value, a call's result); a generic body's, as each of its
/// specializations types it.
///
/// Not judged when the world is not closed (the entry row has no entry,
/// as rule 9 asks: a library checked alone has no parents), for an
/// imported locus, or for an event some recovery applies to a child no
/// row names: a generic supervisor's handler's child (its parent's type
/// parameter), or a statement whose receiver the checker typed as no
/// locus (a value of a type parameter in a body no specialization was
/// walked for).
fn unreached_events(reach: &Reach<'_, '_>, out: &mut Vec<Violation>) {
    if reach.entry.entry().is_none() {
        return;
    }
    let generics: BTreeMap<&str, Vec<&str>> = reach
        .bundle
        .programs
        .values()
        .flat_map(|p| flat_decls(&p.items))
        .filter_map(|d| match d {
            TopDecl::Locus(l) => Some((l.name.name.as_str(), l.generics.iter().map(|g| g.name.name.as_str()).collect())),
            _ => None,
        })
        .collect();
    let is_type_param = |parent: Option<&str>, child: &ChildRef| match (parent, child) {
        (Some(p), ChildRef::External(n)) => generics.get(p).is_some_and(|gs| gs.contains(&n.as_str())),
        _ => false,
    };
    // The events some recovery applies to a child the rows cannot name.
    let mut unnamed: BTreeSet<RecoveryEvent> = BTreeSet::new();
    for row in reach.handlers.rows() {
        if is_type_param(Some(&row.parent), &row.child) {
            unnamed.extend(handler_events(row));
        }
    }
    // A generic body's statement is named by its specializations' rows:
    // the template's walk types a value of a type parameter as nothing.
    let specialized: BTreeSet<(u32, u32, u32)> = reach
        .typed
        .recoveries()
        .filter(|(_, r)| !r.specialization.is_empty())
        .map(|(body, r)| (body.0, r.statement.start.0, r.statement.end.0))
        .collect();
    for (body, row) in reach.typed.recoveries() {
        let theirs = row.specialization.is_empty()
            && specialized.contains(&(body.0, row.statement.start.0, row.statement.end.0));
        if row.child.is_none() && !theirs {
            unnamed.extend(statement_events(row));
        }
    }
    for row in reach.clauses.rows.iter().filter(|r| !r.locus.imported) {
        let locus = row.locus;
        let site = reach.bundle.snapshot.site_id(locus.id).map(SiteRef::user);
        let handlers: Vec<&HandlerRow> =
            reach.handlers.rows().iter().filter(|h| is_locus(&h.child, h.child_decl, locus, site)).collect();
        // One statement once, however many specializations name the locus.
        let mut statements: Vec<&RecoveryRow> = Vec::new();
        for (_, s) in reach.typed.recoveries().filter(|(_, s)| receives(s, locus)) {
            if !statements.iter().any(|seen| seen.statement == s.statement && seen.op == s.op) {
                statements.push(s);
            }
        }
        let applied: BTreeSet<RecoveryEvent> = handlers
            .iter()
            .map(|h| handler_events(h))
            .chain(statements.iter().map(|s| statement_events(s)))
            .flatten()
            .collect();
        let mut judged: Vec<RecoveryEvent> = Vec::new();
        for n in &row.events.names {
            let Some(event) = n.event else { continue };
            if judged.contains(&event) || applied.contains(&event) || unnamed.contains(&event) {
                continue;
            }
            judged.push(event);
            let l = locus.name.name.as_str();
            let mut witness: Vec<WitnessStep> = handlers
                .iter()
                .map(|h| WitnessStep {
                    span: h.span,
                    origin: SpanOrigin::Seed,
                    note: format!("`{}` handles a failing `{l}` here and applies {}", h.parent, spelled(&handler_events(h))),
                })
                .collect();
            witness.extend(statements.iter().map(|s| WitnessStep {
                span: s.statement,
                origin: SpanOrigin::Seed,
                note: format!(
                    "{} applies {} to a `{l}` here",
                    s.parent_name.as_deref().map(|p| format!("`{p}`")).unwrap_or_else(|| "a free fn".to_string()),
                    spelled(&statement_events(s)),
                ),
            }));
            if witness.is_empty() {
                witness.push(WitnessStep {
                    span: locus.name.span,
                    origin: SpanOrigin::Seed,
                    note: format!(
                        "`{l}` is declared here; no `on_failure` in the program handles one, and no recovery \
                         statement is applied to one"
                    ),
                });
            }
            out.push(Violation {
                rule: UNREACHED,
                severity: Severity::Warning,
                span: n.name.span,
                message: format!(
                    "closure `{}`: no recovery in this program applies `{}` to a `{l}`, so `{}({})` never \
                     takes effect",
                    row.closure.name.name,
                    event.name(),
                    row.clause.keyword(),
                    event.name(),
                ),
                witness,
            });
        }
    }
}

/// `persists_through(...)` on a closure whose assertion accumulates
/// nothing (no `sum`, `count` or `mean`) keeps nothing: a warning at the
/// clause, for a locus of the program's own seed. The witness is the
/// assertion, when the closure has one.
fn nothing_to_keep(rows: &ClosureEventRows<'_>, out: &mut Vec<Violation>) {
    for row in rows.rows.iter().filter(|r| r.clause == Clause::PersistsThrough && !r.locus.imported) {
        let assertion = row.closure.assertion.as_ref();
        if assertion.is_some_and(|a| !accumulator_sites(a).is_empty()) {
            continue;
        }
        out.push(Violation {
            rule: NOTHING_KEPT,
            severity: Severity::Warning,
            span: row.events.span,
            message: format!(
                "closure `{}`: `persists_through(...)` keeps a closure's accumulators through a recovery, and \
                 this closure has none, so the clause keeps nothing",
                row.closure.name.name,
            ),
            witness: assertion
                .map(|a| WitnessStep {
                    span: a.span,
                    origin: SpanOrigin::Seed,
                    note: "the assertion accumulates nothing: no `sum`, `count` or `mean`".to_string(),
                })
                .into_iter()
                .collect(),
        });
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
