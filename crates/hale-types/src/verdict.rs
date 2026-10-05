//! One outcome vocabulary for every law the compiler evaluates.
//!
//! Bundle claims and fn-grained certificates (`@effects`, `@budget`,
//! `@phase_effects`) are the same kind of statement at different
//! granularity — #392 §8 already reports the second as the claim form
//! it is pointwise sugar for. They should not disagree about how to
//! spell an outcome, and before this type they did: claim rows
//! carried three states while lowered rows carried a bool.
//!
//! The four states are distinguished by **what the reader should do
//! about them**, which is the only distinction worth encoding:
//!
//! | verdict | meaning | repair |
//! |---|---|---|
//! | `holds` | proved | nothing |
//! | `violated` | disproved — a counterexample exists | fix the program (the witness says where) |
//! | `uncertified` | not provable — the graph has unknowns | resolve the unknown edge, or accept that this law cannot be checked here |
//! | `invalid` | the statement itself is malformed | fix the claim (an unknown group member, an undeclared effect class) |
//!
//! `violated` and `uncertified` were previously one value, because
//! **unknown ⇒ violation** — an indirect call fails closed rather
//! than certifying an absence it cannot see. That rule is unchanged
//! and both still fail the build. What changes is that the artifact
//! now records which happened, because the repairs are different and
//! because composing models across binaries needs the distinction: a
//! propagated unknown must make an exact-cardinality claim
//! `uncertified`, not report it as disproved when nothing disproved
//! it.

/// The outcome of evaluating one law.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Proved.
    Holds,
    /// Disproved — a counterexample exists.
    Violated,
    /// Well-formed but not provable here: the graph carries an
    /// unknown (an indirect call, an untypeable receiver, a computed
    /// subject) that fails closed.
    Uncertified,
    /// The statement is malformed — an unknown group member, an
    /// undeclared effect class. Evaluation never ran.
    Invalid,
}

impl Verdict {
    /// The wire spelling, as it appears in the topology artifact.
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Holds => "holds",
            Verdict::Violated => "violated",
            Verdict::Uncertified => "uncertified",
            Verdict::Invalid => "invalid",
        }
    }

    /// Did this law pass?
    ///
    /// Only `Holds` does. `Uncertified` deliberately does not: a law
    /// that could not be checked has not been satisfied, and treating
    /// "we could not tell" as success is the fail-open this whole
    /// vocabulary exists to prevent.
    pub fn passed(self) -> bool {
        matches!(self, Verdict::Holds)
    }
}

/// The model's spelling of the same four states, which its verdict
/// aggregation (`hale_model::certificate_row_verdict`,
/// `hale_model::document_verdict`) is written over.
impl From<Verdict> for hale_model::VerdictIr {
    fn from(v: Verdict) -> Self {
        match v {
            Verdict::Holds => hale_model::VerdictIr::Holds,
            Verdict::Violated => hale_model::VerdictIr::Violated,
            Verdict::Uncertified => hale_model::VerdictIr::Uncertified,
            Verdict::Invalid => hale_model::VerdictIr::Invalid,
        }
    }
}

impl From<hale_model::VerdictIr> for Verdict {
    fn from(v: hale_model::VerdictIr) -> Self {
        match v {
            hale_model::VerdictIr::Holds => Verdict::Holds,
            hale_model::VerdictIr::Violated => Verdict::Violated,
            hale_model::VerdictIr::Uncertified => Verdict::Uncertified,
            hale_model::VerdictIr::Invalid => Verdict::Invalid,
        }
    }
}
