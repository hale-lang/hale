//! The `lifecycle_flow` integration-test binary: 28 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "birth_spine_ir.rs"]
mod birth_spine_ir;
#[path = "cascade_model.rs"]
mod cascade_model;
#[path = "drain_grace_names_the_wait.rs"]
mod drain_grace_names_the_wait;
#[path = "failure_delivery_domain.rs"]
mod failure_delivery_domain;
#[path = "frame_flush_ir.rs"]
mod frame_flush_ir;
#[path = "generic_monomorph_agreement.rs"]
mod generic_monomorph_agreement;
#[path = "generics.rs"]
mod generics;
#[path = "gh730_interface_identity.rs"]
mod gh730_interface_identity;
#[path = "harness_lowering_laws.rs"]
mod harness_lowering_laws;
#[path = "interface_dispatch.rs"]
mod interface_dispatch;
#[path = "interface_in_composites.rs"]
mod interface_in_composites;
#[path = "interface_in_form_vec.rs"]
mod interface_in_form_vec;
#[path = "interface_return.rs"]
mod interface_return;
#[path = "lifecycle_fixtures.rs"]
mod lifecycle_fixtures;
#[path = "missing_rows.rs"]
mod missing_rows;
#[path = "nested_long_lived_child.rs"]
mod nested_long_lived_child;
#[path = "nested_long_running_child.rs"]
mod nested_long_running_child;
#[path = "on_failure_dispatch_by_child_type.rs"]
mod on_failure_dispatch_by_child_type;
#[path = "or_block_disposer.rs"]
mod or_block_disposer;
#[path = "or_fallible_handler.rs"]
mod or_fallible_handler;
#[path = "or_wait_loss_window.rs"]
mod or_wait_loss_window;
#[path = "placement_factory_default.rs"]
mod placement_factory_default;
#[path = "placement_where_async_io.rs"]
mod placement_where_async_io;
#[path = "pool_affinity.rs"]
mod pool_affinity;
#[path = "typed_body_rows.rs"]
mod typed_body_rows;
#[path = "reclaim_cancel_ir.rs"]
mod reclaim_cancel_ir;
#[path = "reclaim_spine_ir.rs"]
mod reclaim_spine_ir;
#[path = "restart_spine_ir.rs"]
mod restart_spine_ir;
