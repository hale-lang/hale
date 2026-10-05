//! The `text_strings` integration-test binary: 9 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "case_fold_and_nested_fstring.rs"]
mod case_fold_and_nested_fstring;
#[path = "fstring_interpolation.rs"]
mod fstring_interpolation;
#[path = "printable_rule.rs"]
mod printable_rule;
#[path = "str_repeat_and_pad.rs"]
mod str_repeat_and_pad;
#[path = "str_split_into.rs"]
mod str_split_into;
#[path = "str_trim_and_replace.rs"]
mod str_trim_and_replace;
#[path = "string_builder.rs"]
mod string_builder;
#[path = "string_escapes.rs"]
mod string_escapes;
#[path = "wildcard_match_parity.rs"]
mod wildcard_match_parity;
