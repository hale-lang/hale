//! The `ownership_owners` integration-test binary: 16 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "bubble_from_param_child.rs"]
mod bubble_from_param_child;
#[path = "caller_arena_publish_gate.rs"]
mod caller_arena_publish_gate;
#[path = "caller_arena_tls_unwind.rs"]
mod caller_arena_tls_unwind;
#[path = "conformance_routing_correction.rs"]
mod conformance_routing_correction;
#[path = "factory_field_ownership.rs"]
mod factory_field_ownership;
#[path = "factory_locus_reclaim.rs"]
mod factory_locus_reclaim;
#[path = "factory_returned_binding.rs"]
mod factory_returned_binding;
#[path = "fresh_temp_attribution.rs"]
mod fresh_temp_attribution;
#[path = "gh967_contract_field_borrow.rs"]
mod gh967_contract_field_borrow;
#[path = "owned_child_arena_reclaim.rs"]
mod owned_child_arena_reclaim;
#[path = "owner_table.rs"]
mod owner_table;
#[path = "ownership_bubble.rs"]
mod ownership_bubble;
#[path = "ownership_bubble_crosspool.rs"]
mod ownership_bubble_crosspool;
#[path = "ownership_bubble_mixed.rs"]
mod ownership_bubble_mixed;
#[path = "ownership_bubble_multi.rs"]
mod ownership_bubble_multi;
#[path = "unowned_literal_pair_leak.rs"]
mod unowned_literal_pair_leak;
#[path = "unowned_literal_positions.rs"]
mod unowned_literal_positions;
