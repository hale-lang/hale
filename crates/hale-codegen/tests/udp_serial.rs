//! The `udp_serial` integration-test binary: 4 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "bus_udp_transport.rs"]
mod bus_udp_transport;
#[path = "udp_multicast.rs"]
mod udp_multicast;
#[path = "udp_p4_source_and_timeout.rs"]
mod udp_p4_source_and_timeout;
#[path = "udp_primitives.rs"]
mod udp_primitives;
