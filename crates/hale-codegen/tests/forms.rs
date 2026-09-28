//! The `forms` integration-test binary: 13 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "form_hashmap_codegen.rs"]
mod form_hashmap_codegen;
#[path = "form_hashmap_lockfree.rs"]
mod form_hashmap_lockfree;
#[path = "form_hashmap_serialized.rs"]
mod form_hashmap_serialized;
#[path = "form_hashmap_synced_retire.rs"]
mod form_hashmap_synced_retire;
#[path = "form_iteration.rs"]
mod form_iteration;
#[path = "form_lru_cache_codegen.rs"]
mod form_lru_cache_codegen;
#[path = "form_ring_buffer_codegen.rs"]
mod form_ring_buffer_codegen;
#[path = "form_set.rs"]
mod form_set;
#[path = "form_vec_bce.rs"]
mod form_vec_bce;
#[path = "form_vec_codegen.rs"]
mod form_vec_codegen;
#[path = "form_vec_inline.rs"]
mod form_vec_inline;
#[path = "form_vec_pop_retire.rs"]
mod form_vec_pop_retire;
#[path = "form_vec_set_retire.rs"]
mod form_vec_set_retire;
