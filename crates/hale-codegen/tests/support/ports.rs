//! A free TCP port for a test that must hand one to a child process.
//!
//! Its own file, not part of `harness.rs`: every test binary includes the
//! harness, and a helper the binary does not call is a dead-code warning
//! in that binary. The tests that need a port include this one too, with
//! `#[path = "support/ports.rs"] mod ports;`.

/// A TCP port nothing else holds, obtained by binding `:0` and
/// letting the kernel choose.
///
/// The alternative in the suite today is a hand-maintained registry
/// of high port numbers (the 57xxx / 47xxx blocks) spread across 159
/// files — a uniqueness invariant kept in a person's head, and
/// already violated: `9876` appears six times.
///
/// Note the inherent race: the listener is closed before the port is
/// handed back, so it is *free*, not *reserved*. That is still
/// strictly better than a fixed number, because the kernel does not
/// hand out a port already bound by a concurrent test.
/// NOTE: this does not RESERVE the port. It binds an ephemeral port,
/// reads the number, and drops the listener — so between the return
/// and whoever binds it next there is a window in which a parallel
/// test can take it. The window cannot be closed: holding the port is
/// exactly what would stop the caller (often a child process) from
/// binding it.
///
/// A test that must actually bind the port should therefore retry the
/// acquire-and-bind pair as a unit rather than trusting one draw —
/// see `tcp_listener_exclusive_bind.rs`, which flaked in CI on
/// 2026-08-03 for exactly this reason.
pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind an ephemeral port")
        .local_addr()
        .expect("read back the bound addr")
        .port()
}
