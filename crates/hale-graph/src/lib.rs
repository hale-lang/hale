//! # The graph core and its registry (F.40, phase 0)
//!
//! F.40 reorganizes the implementation of existing Hale semantics
//! into authoritative, typed, witnessed results consumed through one
//! pipeline. Its acceptance question:
//!
//! > Can a contributor make a correct change to one semantic family
//! > by understanding its contract, producer, relevant consumers and
//! > focused tests, without reconstructing the rest of the compiler?
//!
//! Phase 0 ships the **registry** ([`registry`]): every semantic
//! family the compiler derives, with its contract — what it answers,
//! what it reads, its authoritative producer, the legacy producers
//! still permitted while it migrates, its consumers, invariants,
//! missing-data policy and focused tests — plus the spec's numbered
//! rules with their evaluators, and the frozen list of Debug-string
//! scans. The registry is data: `spec/registry.md` is rendered from
//! it and checked byte for byte, and the guard tests under `tests/`
//! fail the build when the tree grows a derivation the registry does
//! not name.
//!
//! Three states, so the inventory can be exact without banning what
//! exists today:
//!
//! - **Reserved** — a future family or consumer; creates no
//!   production demand and computes no table.
//! - **Migrating** — the canonical producer is under development;
//!   the registry carries the exact inventory of permitted legacy
//!   producers, each with its removal condition.
//! - **Canonical** — one production producer; consumers cannot
//!   reconstruct its meaning.
//!
//! Phase 1 adds the generic mechanics the families share: snapshot
//! identity ([`ids::SiteId`], `(seed, index)`, minted once after
//! desugar), typed tables with provenance ([`provenance`], the
//! model's store moved down so `hale-model` is rebuilt on this
//! crate), shared query support and the shadow facility
//! ([`shadow`]). Row meanings never move here: `hale-graph` depends
//! on nothing and knows no family by anything but its name.
//!
//! The parent of this work is GH #476, which made every verification
//! consumer read one typed model and deleted the second evaluator.
//! Its canaries are the registry's first entries.

#![forbid(unsafe_code)]

#[macro_use]
pub mod ids;
pub mod identity;
pub mod provenance;
pub mod registry;
pub mod shadow;

pub use registry::{
    families, family, render_markdown, rules, Consumer, DebugScan, Family, Kind, Layer, Legacy,
    Missing, Rule, ScanVerdict, Seam, Site, State, DEBUG_SCANS, FAMILIES, RULES,
    SHADOW_CALL_SITES,
};
