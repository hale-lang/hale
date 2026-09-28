//! The `collections` integration-test binary: 17 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "array_repeat.rs"]
mod array_repeat;
#[path = "array_repeat_stack.rs"]
mod array_repeat_stack;
#[path = "bounded_collections.rs"]
mod bounded_collections;
#[path = "bounded_receiver_dispatch.rs"]
mod bounded_receiver_dispatch;
#[path = "chain_array_sources.rs"]
mod chain_array_sources;
#[path = "children_count_sugar.rs"]
mod children_count_sugar;
#[path = "children_growable.rs"]
mod children_growable;
#[path = "element_chains.rs"]
mod element_chains;
#[path = "f22_as_parent_for_surface.rs"]
mod f22_as_parent_for_surface;
#[path = "f22_capacity_acceptance.rs"]
mod f22_capacity_acceptance;
#[path = "f22_capacity_dispatch.rs"]
mod f22_capacity_dispatch;
#[path = "f22_capacity_smoke.rs"]
mod f22_capacity_smoke;
#[path = "f22_cell_field_io.rs"]
mod f22_cell_field_io;
#[path = "hashmap.rs"]
mod hashmap;
#[path = "hashmap_cell_alias.rs"]
mod hashmap_cell_alias;
#[path = "recpool.rs"]
mod recpool;
#[path = "recpool_codegen.rs"]
mod recpool_codegen;
