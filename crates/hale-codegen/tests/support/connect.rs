//! Connect to a TCP listener a test just spawned, once it answers.
//!
//! A test that spawns a Hale listener and then connects cannot know
//! when the child reaches `listen()`. A fixed sleep before one connect
//! is a guess that holds on a quiet machine and fails under load: the
//! parity job (libtest threads, every test of a binary in one process)
//! refused `http_response_headers`' connect after its 150 ms sleep.
//!
//! So connect in a loop until the port answers or the deadline passes.
//! A refused connect is not an accept, so the retries cost a listener
//! with `max_accepts: 1` nothing; the connection that succeeds is the
//! one handed back, never a probe that would use up the listener's
//! only accept. Its own file, not part of `harness.rs`, for the reason
//! `ports.rs` gives: a helper a binary does not call is a dead-code
//! warning in that binary.

use std::net::TcpStream;
use std::time::{Duration, Instant};

/// The first connection `127.0.0.1:<port>` accepts, retried every
/// 20 ms for up to ten seconds. Panics naming the port when the
/// listener never answers, so a dead child fails here and not as an
/// empty response further on.
///
/// The port was drawn from the ephemeral range, which is where a
/// connect's own local port comes from too, so a retry can land on
/// local == peer and open a connection to itself before the child
/// binds. That one is dropped and the loop goes on.
pub fn connect_when_listening(port: u16) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) if stream.local_addr().ok() != stream.peer_addr().ok() => return stream,
            Ok(_) => std::thread::sleep(Duration::from_millis(20)),
            Err(err) if Instant::now() >= deadline => {
                panic!("nothing listened on 127.0.0.1:{port} within 10s: {err}")
            }
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}
