//! Source-neutral provenance: the graph core's store (F.40 phase 1.1a,
//! `hale_graph::provenance`), re-exported so every row of the model
//! keeps its origin record under the model's own paths.
//!
//! This crate never sees the AST: `hale-types` maps compiler spans
//! into these records while deriving the model (Change 2). The law —
//! enforced by construction, since no row type makes its provenance
//! optional — is that **every** entity, relation, hole, label, and
//! weight answers "where did this fact come from": either a source
//! location or a *named* synthetic origin.

pub use hale_graph::provenance::{Provenance, ProvenanceTable, SourceUnit};
