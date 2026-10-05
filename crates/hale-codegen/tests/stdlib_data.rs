//! The `stdlib_data` integration-test binary: 16 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "stdlib_bytes.rs"]
mod stdlib_bytes;
#[path = "stdlib_call_fixtures.rs"]
mod stdlib_call_fixtures;
#[path = "stdlib_dispatch_coverage.rs"]
mod stdlib_dispatch_coverage;
#[path = "stdlib_iter.rs"]
mod stdlib_iter;
#[path = "stdlib_json.rs"]
mod stdlib_json;
#[path = "stdlib_m79.rs"]
mod stdlib_m79;
#[path = "stdlib_markdown.rs"]
mod stdlib_markdown;
#[path = "stdlib_metrics.rs"]
mod stdlib_metrics;
#[path = "stdlib_name.rs"]
mod stdlib_name;
#[path = "stdlib_path.rs"]
mod stdlib_path;
#[path = "stdlib_registry_parity.rs"]
mod stdlib_registry_parity;
#[path = "stdlib_source.rs"]
mod stdlib_source;
#[path = "stdlib_str.rs"]
mod stdlib_str;
#[path = "stdlib_tagged.rs"]
mod stdlib_tagged;
#[path = "stdlib_test_assert.rs"]
mod stdlib_test_assert;
#[path = "stdlib_yaml.rs"]
mod stdlib_yaml;
