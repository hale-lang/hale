//! 2026-05-30 wakeable park: a coro parked at program shutdown.
//!
//! When the program exits with an async_io coro still PARKED (a
//! listener in accept() that no client ever hit, a recv blocked on
//! a quiet socket), shutdown must wake it cooperatively so it
//! unwinds through its normal run() exit. Two prior failure modes
//! this guards against:
//!   - HANG: shutdown_all only broadcast the cell condvar, which
//!     never wakes a worker blocked in epoll_wait(-1) → join hung.
//!   - CRASH: once woken, the coro resumed AFTER the main locus's
//!     dissolve freed its arena → use-after-free in the work it did
//!     on resume (e.g. the Stream it builds per accepted fd).
//!
//! The fix: a per-pool wake eventfd (shutdown unblocks epoll_wait),
//! a cancel flag that makes park_on_fd return -1 so the parking
//! primitive yields and run() unwinds, and quiescing the pool
//! BEFORE the main locus dissolves its pooled children. This test
//! just asserts the program EXITS (cleanly, promptly) — that single
//! observation catches both the hang and the crash.

use std::process::Command;
use std::time::{Duration, Instant};

use hale_codegen::build_executable;

#[test]
fn program_with_coro_parked_at_shutdown_exits_cleanly() {
    // A lone async_io listener that no client ever connects to, so
    // its accept() coro is still parked when main returns. Pre-fix
    // this either hung the join forever or segfaulted on resume.
    let src = r#"
        fn noop(s: std::io::tcp::Stream) { }

        main locus App {
            params {
                srv: std::io::tcp::Listener = std::io::tcp::Listener {
                    host: "127.0.0.1",
                    port: 0,
                    max_accepts: 1,
                    on_connection: noop,
                };
            }
            placement { srv: cooperative(pool = io) where async_io; }
            run() {
                std::time::sleep(150ms);   // let srv park in accept()
            }
        }
        fn main() { App { }; }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("hale_test_async_io_shutdown_parked");
    build_executable(&program, &bin).expect("build");

    let mut child = Command::new(&bin).spawn().expect("spawn");
    let deadline = Instant::now() + Duration::from_secs(8);
    let status = loop {
        if let Some(s) = child.try_wait().expect("try_wait") {
            break Some(s);
        }
        if Instant::now() >= deadline {
            // Still running well past the 150ms sleep → the shutdown
            // join hung on the parked coro. Kill and fail.
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let _ = std::fs::remove_file(&bin);

    let status = status.expect(
        "program hung at shutdown with a coro still parked in accept() \
         — wakeable-park did not wake the worker",
    );
    assert!(
        status.success(),
        "program crashed at shutdown with a parked coro (exit {:?}) — \
         a parked coro likely resumed after its arena was freed",
        status,
    );
}
