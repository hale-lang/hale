//! The `imports` integration-test binary: 13 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "binding_imported_main.rs"]
mod binding_imported_main;
#[path = "diamond_import.rs"]
mod diamond_import;
#[path = "import_alias_must_be_declared.rs"]
mod import_alias_must_be_declared;
#[path = "import_alias_scoped_per_seed.rs"]
mod import_alias_scoped_per_seed;
#[path = "import_alias_vs_free_fn.rs"]
mod import_alias_vs_free_fn;
#[path = "import_library_key.rs"]
mod import_library_key;
#[path = "import_library_names.rs"]
mod import_library_names;
#[path = "import_parse_error_gates.rs"]
mod import_parse_error_gates;
#[path = "import_qualified_topic.rs"]
mod import_qualified_topic;
#[path = "imported_literal_fields.rs"]
mod imported_literal_fields;
#[path = "imported_main_claims.rs"]
mod imported_main_claims;
#[path = "imported_type_annotation.rs"]
mod imported_type_annotation;
#[path = "two_hop_qualified_literal.rs"]
mod two_hop_qualified_literal;
