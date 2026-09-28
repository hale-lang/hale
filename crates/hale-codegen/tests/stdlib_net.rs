//! The `stdlib_net` integration-test binary: 8 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "std_secret_vault.rs"]
mod std_secret_vault;
#[path = "stdlib_cli.rs"]
mod stdlib_cli;
#[path = "stdlib_http_request.rs"]
mod stdlib_http_request;
#[path = "stdlib_http_response.rs"]
mod stdlib_http_response;
#[path = "stdlib_listener_multi.rs"]
mod stdlib_listener_multi;
#[path = "stdlib_stdin.rs"]
mod stdlib_stdin;
#[path = "stdlib_stream.rs"]
mod stdlib_stream;
#[path = "stdlib_ts.rs"]
mod stdlib_ts;
