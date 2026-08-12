//! R3 (2026-07-29) — the deployment plan, reified.
//!
//! The committed arrangement of the system — which main-locus param
//! fields are pinned where, which live on which cooperative pool,
//! which pools are async_io, which NUMA node an arena binds to —
//! used to exist only as seven loose fields on `Cx`, populated by
//! `collect_main_placement` and consumed ad hoc by the prelude
//! emission and instantiation lowering. Issue #262 (deployment
//! elaboration / meta-scheduling) is premised on this arrangement
//! being a *value*: constructible by an upstream phase, verifiable,
//! submittable, diffable. This struct is that value's seed — one
//! place holding the whole plan, `Debug`-renderable today,
//! serializable when #262 needs it.
//!
//! Field semantics are unchanged from the Cx originals (the doc
//! comments moved with them); `Cx::collect_main_placement` still
//! populates it (path resolution needs `Cx`), and every consumer
//! reads `self.deployment.<field>`.
//!
//! Not yet folded in (next #262 increments): the `bindings { }`
//! block (emitted straight from the AST in the prelude), the
//! `topology { }` domains (resolved inside collect), and replica
//! counts (expanded inline at instantiation).

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::ScheduleClass;

/// The whole-program deployment arrangement, collected from the main
/// locus's `placement { }` / `topology { }` declarations before any
/// lowering runs.
#[derive(Debug, Default, Clone)]
pub struct DeploymentPlan {
    /// The main locus's type name (None when the program has no main
    /// locus — plain `fn main` programs).
    pub main_locus_name: Option<String>,
    /// Per-params-field placement override: field name →
    /// schedule class (pinned with resolved core set, cooperative,
    /// ...). Absent fields keep the locus's own default class.
    pub main_placement_map: BTreeMap<String, ScheduleClass>,
    /// Topology arena-on-node: field name → resolved NUMA node for
    /// `pinned(node = ...)` / `pinned(l3 = ...)` entries; absent for
    /// every other placement (arena stays unbound).
    pub main_placement_node: BTreeMap<String, i64>,
    /// Locus TYPE names that appear in any pinned placement entry —
    /// consumed by struct-shape decisions (pinned loci get a
    /// thread-id slot).
    pub pinned_locus_types: BTreeSet<String>,
    /// Field name → named cooperative pool, for
    /// `cooperative(pool = X)` entries. Drives pool registration in
    /// the prelude and the per-field pool override at instantiation.
    pub main_cooperative_pools: BTreeMap<String, String>,
    /// Pool affinity (2026-08-12): pool name -> the resolved core
    /// set its worker thread binds to (`cooperative(pool = X,
    /// cores/node/l3 = …)`). Resolved against `topology { }` in
    /// the placement pre-pass; conflicting declarations across
    /// entries were rejected at typecheck.
    pub coop_pool_affinity: BTreeMap<String, Vec<i64>>,
    /// Pool names declared `where async_io` (green-I/O scheduling).
    pub async_io_pools: BTreeSet<String>,
    /// Locus TYPE names placed on a named cooperative pool —
    /// consumed by the `__coop_pool_run_<L>` wrapper synthesis.
    pub coop_pool_locus_types: BTreeSet<String>,
}
