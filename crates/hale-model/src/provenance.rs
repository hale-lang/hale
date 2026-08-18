//! Source-neutral provenance.
//!
//! This crate never sees the AST: `hale-types` maps compiler spans
//! into these records while deriving the model (Change 2). The law —
//! enforced by construction, since no row type makes its provenance
//! optional — is that **every** entity, relation, hole, label, and
//! weight answers "where did this fact come from": either a source
//! location or a *named* synthetic origin (facts the compiler
//! introduces with no single authored location, e.g. the implicit
//! main arrangement root).

use crate::ids::SourceId;

/// One origin record.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Provenance {
    /// An authored fact: a byte span in a source unit.
    Source {
        source: SourceId,
        /// Byte offsets `[start, end)` in the unit's content.
        span: (u32, u32),
    },
    /// A derived fact with no single authored location. The origin
    /// names the introducing rule so a witness can still say
    /// something true ("synthetic: main-arrangement root").
    Synthetic { origin: String },
    /// A span in an offset space OUTSIDE the recorded sources —
    /// stdlib bodies parse in their own space, and the evaluator's
    /// certificate diagnostics carry those offsets verbatim
    /// (GH #476 Change 5e). Preserved as-is so evidence rendering
    /// is byte-identical; never resolvable to a recorded source.
    ForeignSpan { span: (u32, u32) },
}

/// One source unit provenance points into. `path` is as-authored
/// (never absolutized — artifacts must not embed machine paths);
/// `digest` pins the content the spans index.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SourceUnit {
    pub path: String,
    pub digest: u64,
}

/// The provenance store: sources plus origin records, referenced by
/// dense IDs from every row in the model.
#[derive(Clone, Default, Debug)]
pub struct ProvenanceTable {
    pub sources: Vec<SourceUnit>,
    pub records: Vec<Provenance>,
}
