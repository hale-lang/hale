//! The `http` integration-test binary: 11 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "docs_server.rs"]
mod docs_server;
#[path = "http_client.rs"]
mod http_client;
#[path = "http_hello.rs"]
mod http_hello;
#[path = "http_request_headers.rs"]
mod http_request_headers;
#[path = "http_response_headers.rs"]
mod http_response_headers;
#[path = "http_route_matching.rs"]
mod http_route_matching;
#[path = "http_router.rs"]
mod http_router;
#[path = "http_server_bind_failure.rs"]
mod http_server_bind_failure;
#[path = "http_server_classic_pool_shutdown.rs"]
mod http_server_classic_pool_shutdown;
#[path = "http_split_write.rs"]
mod http_split_write;
#[path = "http_upgrade.rs"]
mod http_upgrade;
