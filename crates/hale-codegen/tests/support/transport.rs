//! Shared readiness handshake for two-process unix-transport tests.
//!
//! Both `transport_counters` and `binding_stream_framing` spawn a
//! LISTEN-role subscriber and then immediately run a CONNECT-role
//! publisher. The runtime retries a refused connect for ~1s
//! (200 × 5ms, `lotus_tcp_create`), which is ample on a quiet
//! machine — and NOT ample on a CI runner executing 16 test
//! binaries in parallel, where a freshly-spawned process can take
//! longer than that just to reach its `bind()`. When the budget
//! expires the publish is lost, the subscriber never sees its 2
//! deliveries, its wait loop hits the cap and `exit(3)`s — which
//! skips the counters dump, so the failure surfaced as the
//! misleading "no counters dump line from subscriber" rather than
//! "the exchange never happened".
//!
//! The fix is a deterministic handshake instead of a race against a
//! timeout: wait for the listener's socket FILE to exist before
//! launching the publisher. `bind()` is what creates it, and the
//! runtime's listener setup binds + listens back-to-back in one
//! function (#227 hoisted it out of the reader thread), so the file
//! appearing means the listener is up.
//!
//! Deliberately NOT a probe connect: these tests assert on `rearms`,
//! and every accepted-then-closed peer increments it — a probe
//! connection would corrupt the very counter under test.
#![allow(dead_code)]

use std::time::{Duration, Instant};

/// Block until `path` exists (the listener bound), or `timeout`
/// elapses. Returns whether it appeared — callers assert on it so a
/// genuinely dead subscriber fails loudly rather than as a downstream
/// symptom.
pub fn wait_for_listener(path: &str, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if std::path::Path::new(path).exists() {
            // The file appears at bind(); listen() follows
            // immediately in the same C function. One short settle
            // covers that instruction-level window without racing.
            std::thread::sleep(Duration::from_millis(20));
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// Turn the subscriber's exit status into a diagnosis. Exit code 3 is
/// the in-Hale wait-loop cap ("never saw the expected deliveries"),
/// which is a different failure from "ran but printed nothing".
pub fn describe_sub_exit(status: &std::process::ExitStatus) -> Option<String> {
    match status.code() {
        Some(3) => Some(
            "the subscriber timed out waiting for its deliveries \
             (exit 3) — the publisher's connect never landed, so no \
             counters were dumped. This is the transport handshake \
             failing, not a counters bug."
                .to_string(),
        ),
        _ => None,
    }
}
