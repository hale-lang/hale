//! The `tcp_io` integration-test binary: 12 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "accept_shutdown_silent.rs"]
mod accept_shutdown_silent;
#[path = "io_demo.rs"]
mod io_demo;
#[path = "io_error_fallible.rs"]
mod io_error_fallible;
#[path = "io_tcp_recv_timeout.rs"]
mod io_tcp_recv_timeout;
#[path = "io_tls.rs"]
mod io_tls;
#[path = "tcp_connect_refused.rs"]
mod tcp_connect_refused;
#[path = "tcp_dns_connect.rs"]
mod tcp_dns_connect;
#[path = "tcp_listener_exclusive_bind.rs"]
mod tcp_listener_exclusive_bind;
#[path = "tcp_raw_fd_freefns.rs"]
mod tcp_raw_fd_freefns;
#[path = "tcp_recv_bytes.rs"]
mod tcp_recv_bytes;
#[path = "tcp_set_nodelay.rs"]
mod tcp_set_nodelay;
#[path = "tls_fast_io.rs"]
mod tls_fast_io;
