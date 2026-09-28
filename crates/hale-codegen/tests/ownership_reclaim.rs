//! The `ownership_reclaim` integration-test binary: 22 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "arena_oom_is_loud.rs"]
mod arena_oom_is_loud;
#[path = "birth_order_trap.rs"]
mod birth_order_trap;
#[path = "closure_resets_per_epoch.rs"]
mod closure_resets_per_epoch;
#[path = "deferred_slot_per_iteration.rs"]
mod deferred_slot_per_iteration;
#[path = "drain_elision.rs"]
mod drain_elision;
#[path = "failed_child_kept_by_owner.rs"]
mod failed_child_kept_by_owner;
#[path = "fallible_method_scratch_order.rs"]
mod fallible_method_scratch_order;
#[path = "framework_elision.rs"]
mod framework_elision;
#[path = "freefn_locus_rebind.rs"]
mod freefn_locus_rebind;
#[path = "held_failure_outlives_reclaim.rs"]
mod held_failure_outlives_reclaim;
#[path = "method_return_dissolve.rs"]
mod method_return_dissolve;
#[path = "method_scratch_elision.rs"]
mod method_scratch_elision;
#[path = "method_scratch_reclaim.rs"]
mod method_scratch_reclaim;
#[path = "noalias_self.rs"]
mod noalias_self;
#[path = "reclamation_spine.rs"]
mod reclamation_spine;
#[path = "release_reclaims_flow.rs"]
mod release_reclaims_flow;
#[path = "release_two_parents.rs"]
mod release_two_parents;
#[path = "resident_transfer.rs"]
mod resident_transfer;
#[path = "shadow_return_binding.rs"]
mod shadow_return_binding;
#[path = "sink_polymorphism.rs"]
mod sink_polymorphism;
#[path = "teardown_pinned_join_order.rs"]
mod teardown_pinned_join_order;
#[path = "terminate_reclaims_child.rs"]
mod terminate_reclaims_child;
#[path = "violate_child_state_read.rs"]
mod violate_child_state_read;
