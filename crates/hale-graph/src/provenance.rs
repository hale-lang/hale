//! Source-neutral provenance (F.40 phase 1.1a; the model's, moved
//! down).
//!
//! This crate never sees the AST: the producers map compiler spans
//! into these records while deriving rows. The law, enforced by
//! construction since no row type makes its provenance optional, is
//! that every entity, relation, hole, label and weight answers
//! "where did this fact come from": either a source location or a
//! *named* synthetic origin (a fact the compiler introduces with no
//! single authored location, e.g. the implicit main arrangement
//! root).

table_id!(
    /// A source-neutral origin record in the [`ProvenanceTable`].
    ProvenanceId
);
table_id!(
    /// A source unit (path + content digest) provenance points into.
    SourceId
);

/// One origin record.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Provenance {
    /// An authored fact: a byte span in a source unit.
    Source {
        /// The unit the span indexes.
        source: SourceId,
        /// Byte offsets `[start, end)` in the unit's content.
        span: (u32, u32),
    },
    /// A derived fact with no single authored location. The origin
    /// names the introducing rule so a witness can still say
    /// something true ("synthetic: main-arrangement root").
    Synthetic {
        /// The introducing rule.
        origin: String,
    },
    /// A span in an offset space OUTSIDE the recorded sources —
    /// stdlib bodies parse in their own space, and the evaluator's
    /// certificate diagnostics carry those offsets verbatim
    /// (GH #476 Change 5e). Preserved as-is so evidence rendering
    /// is byte-identical; never resolvable to a recorded source.
    ForeignSpan {
        /// Byte offsets in the foreign space.
        span: (u32, u32),
    },
}

/// One source unit provenance points into. `path` is as-authored
/// (never absolutized — artifacts must not embed machine paths);
/// `digest` pins the content the spans index — kept as the
/// producer's exact string (round 2: parsing it numerically lost
/// non-canonical digests and made the artifact's `sources`
/// section unprojectable from the model).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SourceUnit {
    /// The file's path as the bundle saw it.
    pub path: String,
    /// Content digest, so a span can be shown to belong to the text it
    /// was taken from.
    pub digest: String,
}

/// The provenance store: sources plus origin records, referenced by
/// dense IDs from every row.
#[derive(Clone, Default, Debug)]
pub struct ProvenanceTable {
    /// Every source file spans may point into. A `SourceId` is an index
    /// here.
    pub sources: Vec<SourceUnit>,
    /// Every span. A `ProvenanceId` is an index here, which is why rows
    /// carry an id rather than a span.
    pub records: Vec<Provenance>,
}
