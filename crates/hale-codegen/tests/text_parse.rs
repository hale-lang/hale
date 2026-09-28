//! The `text_parse` integration-test binary: 6 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "json_from_json.rs"]
mod json_from_json;
#[path = "json_object_cursor.rs"]
mod json_object_cursor;
#[path = "json_range_helpers.rs"]
mod json_range_helpers;
#[path = "json_span_iter.rs"]
mod json_span_iter;
#[path = "parse_float_and_base64_decode.rs"]
mod parse_float_and_base64_decode;
#[path = "term_primitives.rs"]
mod term_primitives;
