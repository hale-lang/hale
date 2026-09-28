//! The `check_cli` integration-test binary: 12 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "bounded_annotation.rs"]
mod bounded_annotation;
#[path = "check_arg_parsing.rs"]
mod check_arg_parsing;
#[path = "check_borrow_lifetime.rs"]
mod check_borrow_lifetime;
#[path = "check_flows.rs"]
mod check_flows;
#[path = "check_single_file_scope.rs"]
mod check_single_file_scope;
#[path = "check_strict_fallible.rs"]
mod check_strict_fallible;
#[path = "check_unbound_callee.rs"]
mod check_unbound_callee;
#[path = "check_unknown_identifier.rs"]
mod check_unknown_identifier;
#[path = "sibling_file_topic_check.rs"]
mod sibling_file_topic_check;
#[path = "source_map.rs"]
mod source_map;
#[path = "unbounded_alloc_opt_in.rs"]
mod unbounded_alloc_opt_in;
#[path = "workspace_check.rs"]
mod workspace_check;
