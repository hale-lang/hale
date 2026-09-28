//! The `cross_seed` integration-test binary: 10 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "cross_locus_from_method.rs"]
mod cross_locus_from_method;
#[path = "cross_locus_return_chain.rs"]
mod cross_locus_return_chain;
#[path = "cross_seed_imports.rs"]
mod cross_seed_imports;
#[path = "cross_seed_locus_arg.rs"]
mod cross_seed_locus_arg;
#[path = "imported_mark.rs"]
mod imported_mark;
#[path = "ws1_cross_seed_bus_decimal.rs"]
mod ws1_cross_seed_bus_decimal;
#[path = "ws1_cross_seed_locus_reassign.rs"]
mod ws1_cross_seed_locus_reassign;
#[path = "ws3_int_float_conversion.rs"]
mod ws3_int_float_conversion;
#[path = "ws3_nested_if_tail.rs"]
mod ws3_nested_if_tail;
#[path = "ws3_topic_cross_file.rs"]
mod ws3_topic_cross_file;
