//! The `lifecycle_locus` integration-test binary: 21 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "di_entry_reset_is_universal.rs"]
mod di_entry_reset_is_universal;
#[path = "locus_fallible_return_multichild.rs"]
mod locus_fallible_return_multichild;
#[path = "locus_field_cascade.rs"]
mod locus_field_cascade;
#[path = "locus_field_reorder.rs"]
mod locus_field_reorder;
#[path = "locus_let_lifecycle.rs"]
mod locus_let_lifecycle;
#[path = "locus_member_fallible.rs"]
mod locus_member_fallible;
#[path = "locus_member_fallible_rejects.rs"]
mod locus_member_fallible_rejects;
#[path = "main_return_call_teardown.rs"]
mod main_return_call_teardown;
#[path = "main_return_dissolve_frame.rs"]
mod main_return_dissolve_frame;
#[path = "method_call_from_free_fn.rs"]
mod method_call_from_free_fn;
#[path = "params_default_scalar_self_ref.rs"]
mod params_default_scalar_self_ref;
#[path = "perspective_ctor_override_agreement.rs"]
mod perspective_ctor_override_agreement;
#[path = "pure_read_accessors.rs"]
mod pure_read_accessors;
#[path = "repr_field_accessors.rs"]
mod repr_field_accessors;
#[path = "restart_bound.rs"]
mod restart_bound;
#[path = "restart_in_place_params.rs"]
mod restart_in_place_params;
#[path = "self_containing_locus.rs"]
mod self_containing_locus;
#[path = "self_field_alias.rs"]
mod self_field_alias;
#[path = "self_field_index_assign.rs"]
mod self_field_index_assign;
#[path = "self_field_retire_alternation.rs"]
mod self_field_retire_alternation;
#[path = "sibling_field_forward_ref.rs"]
mod sibling_field_forward_ref;
