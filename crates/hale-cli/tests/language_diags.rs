//! The `language_diags` integration-test binary: 12 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "diag_reporting.rs"]
mod diag_reporting;
#[path = "entry_point_placement.rs"]
mod entry_point_placement;
#[path = "env_matrix.rs"]
mod env_matrix;
#[path = "message_spacing.rs"]
mod message_spacing;
#[path = "p3_namespace_method_refused.rs"]
mod p3_namespace_method_refused;
#[path = "qualified_type_forward_ref.rs"]
mod qualified_type_forward_ref;
#[path = "regex_engine.rs"]
mod regex_engine;
#[path = "str_predicates.rs"]
mod str_predicates;
#[path = "str_utf8.rs"]
mod str_utf8;
#[path = "time_iso8601.rs"]
mod time_iso8601;
#[path = "unknown_stdlib_namespace.rs"]
mod unknown_stdlib_namespace;
#[path = "unresolved_qualified.rs"]
mod unresolved_qualified;
