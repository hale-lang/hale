//! The `bus_transport` integration-test binary: 5 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "api_hub.rs"]
mod api_hub;
#[path = "api_hub_desk.rs"]
mod api_hub_desk;
#[path = "api_hub_streams.rs"]
mod api_hub_streams;
#[path = "api_hub_udp.rs"]
mod api_hub_udp;
#[path = "api_http.rs"]
mod api_http;
#[path = "api_http_witness.rs"]
mod api_http_witness;
#[path = "api_mcp.rs"]
mod api_mcp;
#[path = "api_runtime.rs"]
mod api_runtime;
#[path = "api_serve_build.rs"]
mod api_serve_build;
#[path = "api_unix.rs"]
mod api_unix;
#[path = "api_unix_lifecycle.rs"]
mod api_unix_lifecycle;
#[path = "api_witness_whole.rs"]
mod api_witness_whole;
#[path = "io_h2_server.rs"]
mod io_h2_server;
#[path = "io_h2_session.rs"]
mod io_h2_session;
#[path = "mirror_ring.rs"]
mod mirror_ring;
#[path = "spsc_ring.rs"]
mod spsc_ring;
#[path = "transport.rs"]
mod transport;
#[path = "transport_counters.rs"]
mod transport_counters;
#[path = "transport_tcp.rs"]
mod transport_tcp;
