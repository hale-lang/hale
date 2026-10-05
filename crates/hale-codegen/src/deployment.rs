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
//! It is the lowering view of the placement table (F.40 phase 3, P1 5
//! of 6): `Cx::collect_main_placement` reads the snapshot's table
//! (`LoweringView::placement`) — the root rows a `placement { }` entry
//! decides, their domains (schedule class, pool, affinity, NUMA node,
//! `async_io`), the replica rows, and the adapters of the root's
//! `bindings { }` — and every consumer reads `self.deployment.<field>`.
//! The type sets are the rows' realized declarations, by the name
//! lowering keys on. Field semantics are unchanged from the Cx
//! originals (the doc comments moved with them).
//!
//! Not yet folded in (next #262 increments): the `bindings { }`
//! block's transports (emitted straight from the AST in the prelude).

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::ScheduleClass;

/// The whole-program deployment arrangement, read from the placement
/// table before any lowering runs.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DeploymentPlan {
    /// Per-params-field placement override: field name →
    /// schedule class (pinned with resolved core set, cooperative,
    /// ...). Absent fields keep the locus's own default class.
    pub main_placement_map: BTreeMap<String, ScheduleClass>,
    /// Topology arena-on-node: field name → resolved NUMA node for
    /// `pinned(node = ...)` / `pinned(l3 = ...)` entries; absent for
    /// every other placement (arena stays unbound).
    pub main_placement_node: BTreeMap<String, i64>,
    /// Topology Phase 1c (replicas): field name → the core each replica
    /// of a `pinned(..., replicas = K)` field binds to, for `K > 1`: one
    /// per replica row, in replica order (`None` = OS-scheduled). Replica
    /// 0 takes the field's own class in `main_placement_map`; the params
    /// init emits the other `K - 1` instances. `K <= 1` is an ordinary
    /// single instance and has no entry.
    pub main_placement_replicas: BTreeMap<String, Vec<Option<i64>>>,
    /// Locus TYPE names of the pinned anchors (a root field placed
    /// `pinned`, an adapter of the root's `bindings { }`), as realized —
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
    /// Locus TYPE names placed on a named cooperative pool, as realized.
    /// The `__coop_pool_run_<L>` wrapper synthesis covers every
    /// run-bearing locus, so nothing reads it to decide a wrapper.
    pub coop_pool_locus_types: BTreeSet<String>,
    /// Locus TYPE names of the pinned anchors whose nested tree holds a
    /// subscriber (`LoweringView::route_anchors`, from the placement
    /// table): each gets a mailbox even when it subscribes to nothing
    /// itself, so its descendants' subscriptions route to its thread
    /// (U-6).
    pub route_anchor_types: BTreeSet<String>,
}
