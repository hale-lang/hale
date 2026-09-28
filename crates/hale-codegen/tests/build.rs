//! The `build` integration-test binary: 11 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "build_hello.rs"]
mod build_hello;
#[path = "closed_world_nested_struct.rs"]
mod closed_world_nested_struct;
#[path = "concurrent_same_output.rs"]
mod concurrent_same_output;
#[path = "library_shape_2026_05_16.rs"]
mod library_shape_2026_05_16;
#[path = "library_shape_http_json.rs"]
mod library_shape_http_json;
#[path = "module_decls.rs"]
mod module_decls;
#[path = "multi_file_build.rs"]
mod multi_file_build;
#[path = "qualified_locus_type_coop_pool.rs"]
mod qualified_locus_type_coop_pool;
#[path = "top_level_const.rs"]
mod top_level_const;
#[path = "type_record_fn_pointer_field.rs"]
mod type_record_fn_pointer_field;
#[path = "violate_build.rs"]
mod violate_build;
