//! What a structural law finds (F.40 phase 4, W2).
//!
//! A structural law is a registered rule (`hale_graph::registry::RULES`)
//! and a function from family rows to its violations, run by the check
//! as [`crate::lowering_laws`] runs its laws: the check calls each one
//! explicitly; nothing here runs a law by itself. What the laws share is
//! the shape of a finding: the rule it breaks, where it points, its
//! wording, and the witness, the chain of places that makes it so. A
//! violation becomes a diagnostic in one place, [`Violation::into_diag`],
//! so a witness renders one way everywhere: each step a related location
//! with its note, which the CLI prints as a `note:` line and the editor
//! shows as a second location.

use hale_graph::registry::{Rule, RULES};
use hale_syntax::{Diag, Related, Span, SpanOrigin};

/// A registered rule, named as the registry names it: its list's key and
/// its number (a numbered list's) or slug (a table's), `<list>/<n>`.
///
/// Made only by [`RuleId::registered`], which refuses a rule the registry
/// does not hold; a `RuleId` in a `const` is checked when the crate
/// builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleId {
    id: &'static str,
}

impl RuleId {
    /// The rule `<list>/<n>` of the registry. Panics when the registry
    /// holds no such rule: evaluated in a `const`, that fails the build.
    pub const fn registered(list: &str, n: &str) -> RuleId {
        let mut i = 0;
        while i < RULES.len() {
            if names(RULES[i].id, list, n) {
                return RuleId { id: RULES[i].id };
            }
            i += 1;
        }
        panic!("no registered rule by that list and number (hale_graph::registry::RULES)")
    }

    /// The registry's id, `<list>/<n>`.
    pub fn id(self) -> &'static str {
        self.id
    }

    /// The registry's entry.
    pub fn rule(self) -> &'static Rule {
        RULES.iter().find(|r| r.id == self.id).expect("a RuleId names a registered rule")
    }
}

/// Whether `id` is `<list>/<n>`.
const fn names(id: &str, list: &str, n: &str) -> bool {
    let (id, list, n) = (id.as_bytes(), list.as_bytes(), n.as_bytes());
    if id.len() != list.len() + 1 + n.len() || id[list.len()] != b'/' {
        return false;
    }
    let mut i = 0;
    while i < list.len() {
        if id[i] != list[i] {
            return false;
        }
        i += 1;
    }
    let mut j = 0;
    while j < n.len() {
        if id[list.len() + 1 + j] != n[j] {
            return false;
        }
        j += 1;
    }
    true
}

/// Whether a violation fails the build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// One step of a witness: a place, and what it contributes to the finding.
#[derive(Debug, Clone, PartialEq)]
pub struct WitnessStep {
    pub span: Span,
    /// The space `span` is measured in: a step may stand in the stdlib
    /// (a `std::` locus's declaration) while the finding is the seed's.
    pub origin: SpanOrigin,
    pub note: String,
}

/// What a structural law found.
#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    pub rule: RuleId,
    pub severity: Severity,
    /// Where the diagnostic points, in the seed's space.
    pub span: Span,
    /// The rule's wording.
    pub message: String,
    /// The chain that makes it so, in reading order.
    pub witness: Vec<WitnessStep>,
}

impl Violation {
    /// An error of `rule` at `span`, with no witness yet.
    pub fn error(rule: RuleId, span: Span, message: impl Into<String>) -> Violation {
        Violation { rule, severity: Severity::Error, span, message: message.into(), witness: Vec::new() }
    }

    /// A warning of `rule` at `span`, with no witness yet.
    pub fn warning(rule: RuleId, span: Span, message: impl Into<String>) -> Violation {
        Violation { severity: Severity::Warning, ..Violation::error(rule, span, message) }
    }

    /// The violation with one more witness step, in the seed's space.
    pub fn step(mut self, span: Span, note: impl Into<String>) -> Violation {
        self.witness.push(WitnessStep { span, origin: SpanOrigin::Seed, note: note.into() });
        self
    }

    /// The diagnostic: the message unchanged, at the span, and each step
    /// of the witness a related location with its note, in order.
    pub fn into_diag(self) -> Diag {
        let mut diag = match self.severity {
            Severity::Error => Diag::ty(self.span, self.message),
            Severity::Warning => Diag::warn(self.span, self.message),
        };
        diag.related.extend(
            self.witness.into_iter().map(|s| Related { span: s.span, label: s.note, origin: s.origin }),
        );
        diag
    }
}

/// The diagnostics of violations found in one walk, in the order found: a
/// walk that judges several rules at once (the role rules) reports them
/// interleaved, as the walk reaches them, which a [`Law`] per rule would
/// reorder.
pub fn diags(found: Vec<Violation>) -> Vec<Diag> {
    found.into_iter().map(Violation::into_diag).collect()
}

/// A structural law: a registered rule, and the function from family rows
/// to its violations.
pub struct Law<I: ?Sized> {
    pub rule: RuleId,
    pub eval: fn(&I, &mut Vec<Violation>),
}

impl<I: ?Sized> Law<I> {
    /// The law's findings over `rows`, as diagnostics.
    pub fn diags(&self, rows: &I) -> Vec<Diag> {
        let mut found = Vec::new();
        (self.eval)(rows, &mut found);
        debug_assert!(found.iter().all(|v| v.rule == self.rule), "a law reports its own rule");
        found.into_iter().map(Violation::into_diag).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hale_syntax::DiagKind;

    const RULE_6: RuleId = RuleId::registered("semantics/placement", "6");

    fn violation(witness: Vec<WitnessStep>) -> Violation {
        Violation {
            rule: RULE_6,
            severity: Severity::Error,
            span: Span::new(10, 14),
            message: "placement entry `w`: it is so (rule 6)".to_string(),
            witness,
        }
    }

    fn step(at: usize, origin: SpanOrigin, note: &str) -> WitnessStep {
        WitnessStep { span: Span::new(at, at + 1), origin, note: note.to_string() }
    }

    #[test]
    fn a_violation_with_no_witness_is_the_diagnostic_it_was() {
        let v = violation(Vec::new());
        assert_eq!(v.clone().into_diag(), Diag::ty(v.span, v.message.clone()));
        let warning = Violation { severity: Severity::Warning, ..v.clone() };
        assert_eq!(warning.into_diag(), Diag::warn(v.span, v.message));
    }

    #[test]
    fn the_witness_renders_in_order_as_related_notes() {
        let d = violation(vec![
            step(20, SpanOrigin::Seed, "first"),
            step(5, SpanOrigin::Stdlib, "second"),
            step(40, SpanOrigin::Seed, "third"),
        ])
        .into_diag();
        assert_eq!(d.kind, DiagKind::Type);
        assert_eq!(d.message, "placement entry `w`: it is so (rule 6)");
        assert_eq!(d.span, Span::new(10, 14));
        let notes: Vec<(&str, u32, SpanOrigin)> =
            d.related.iter().map(|r| (r.label.as_str(), r.span.start.0, r.origin)).collect();
        assert_eq!(
            notes,
            [("first", 20, SpanOrigin::Seed), ("second", 5, SpanOrigin::Stdlib), ("third", 40, SpanOrigin::Seed)]
        );
        // The single-source renderer prints each step as a note, in order.
        let src = "x".repeat(64);
        let rendered = d.render(&src);
        let first = rendered.find("note: first").expect(&rendered);
        let second = rendered.find("note: second (in the standard library)").expect(&rendered);
        let third = rendered.find("note: third").expect(&rendered);
        assert!(first < second && second < third, "{rendered}");
    }

    #[test]
    fn the_constructors_build_the_violation_and_its_steps_in_order() {
        let v = Violation::error(RULE_6, Span::new(10, 14), "placement entry `w`: it is so (rule 6)")
            .step(Span::new(20, 21), "first")
            .step(Span::new(40, 41), "third");
        assert_eq!(
            v,
            violation(vec![step(20, SpanOrigin::Seed, "first"), step(40, SpanOrigin::Seed, "third")])
        );
        let w = Violation::warning(RULE_6, v.span, v.message.clone());
        assert_eq!(w.severity, Severity::Warning);
        assert!(w.witness.is_empty());
        let found = diags(vec![w.clone(), v.clone()]);
        assert_eq!(found, [w.into_diag(), v.into_diag()]);
    }

    #[test]
    fn a_rule_id_names_the_registry_entry() {
        assert_eq!(RULE_6.id(), "semantics/placement/6");
        assert_eq!(RULE_6.rule().title, "Locus-pinning compatibility.");
        let slots = RuleId::registered("semantics/slots", "1");
        assert_eq!(slots.id(), "semantics/slots/1");
        // A prefix of a number is not the number.
        assert_ne!(RuleId::registered("semantics/placement", "1").id(), "semantics/placement/17");
    }

    #[test]
    #[should_panic(expected = "no registered rule")]
    fn a_rule_id_for_an_unregistered_rule_fails() {
        RuleId::registered("semantics/placement", "999");
    }

    #[test]
    #[should_panic(expected = "no registered rule")]
    fn a_rule_id_for_an_unregistered_list_fails() {
        RuleId::registered("semantics/nowhere", "6");
    }

    #[test]
    fn a_law_renders_its_violations() {
        fn eval(n: &usize, out: &mut Vec<Violation>) {
            for i in 0..*n {
                out.push(Violation { span: Span::new(i, i + 1), ..violation(Vec::new()) });
            }
        }
        let law: Law<usize> = Law { rule: RULE_6, eval };
        let diags = law.diags(&2);
        assert_eq!(diags.iter().map(|d| d.span.start.0).collect::<Vec<_>>(), [0, 1]);
    }
}
