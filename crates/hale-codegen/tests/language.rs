//! The `language` integration-test binary: 8 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "cond_match.rs"]
mod cond_match;
#[path = "fn_nonalloc_add.rs"]
mod fn_nonalloc_add;
#[path = "fn_ptr.rs"]
mod fn_ptr;
#[path = "if_expression.rs"]
mod if_expression;
#[path = "match_expression.rs"]
mod match_expression;
#[path = "phase_b_fixes.rs"]
mod phase_b_fixes;
#[path = "view_coerce_e1.rs"]
mod view_coerce_e1;
#[path = "view_storage_e2.rs"]
mod view_storage_e2;
