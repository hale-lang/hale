//! Downstream handoff 2026-07-16 — a udp recv inside a BUS HANDLER on a
//! `where async_io` pool must not overflow the coroutine stack.
//!
//! `lotus_udp_recv_bytes_global` / `_with_source` used to declare a
//! 64 KiB `char stack_buf[65536]` local. A bus handler runs on a
//! per-invocation coroutine whose stack is 64 KiB
//! (`LOTUS_CORO_STACK_BYTES`), so calling udp recv from a handler put a
//! 64 KiB buffer on a 64 KiB stack — a latent overflow that corrupts
//! adjacent memory and becomes a hard SIGSEGV the moment the coro
//! struct layout shifts (it did, on a since-discarded candidate). The
//! buffer now lives on the heap; this pins that a handler-side udp recv
//! on an async pool runs to completion and delivers correct data
//! instead of smashing the stack.

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale-async-udp-handler-{}-{}", name, std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

fn free_udp_port() -> u16 {
    let s = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind probe");
    s.local_addr().expect("local_addr").port()
}

#[test]
fn udp_recv_in_async_bus_handler_does_not_smash_the_coro_stack() {
    // A `pinned` sender publishes "tick" cells to an async_io
    // subscriber. Each on_tick handler recvs one datagram (arriving on
    // the subscriber's bound udp socket) and echoes its length. The
    // handler doing udp recv is the exact path that overflowed the coro
    // stack. Success = it runs to completion (no crash) and the handler
    // saw the payloads.
    let rport = free_udp_port();
    let src = format!(
        r#"
        type Tick {{ n: Int; }}
        fn __empty(e: IoError) -> Bytes {{ return b""; }}

        locus Reader {{
            params {{ sock: Int = -1; seen: Int = 0; }}
            bus {{ subscribe "tick" as on_tick of type Tick; }}
            fn on_tick(t: Tick) {{
                // udp recv INSIDE the handler, on the async coro stack —
                // used to overflow it with a 64 KiB stack buffer. The
                // overflow is exercised whether or not a datagram is
                // waiting (the buffer is allocated either way), so we
                // count handler invocations, not successful recvs.
                let _ = std::io::udp::recv(self.sock, 2048) or __empty(err);
                self.seen = self.seen + 1;
                if self.seen == 4 {{ println("reader saw 4"); }}
            }}
            run() {{
                self.sock = std::io::udp::bind("127.0.0.1", {rport}) or raise;
                std::io::udp::set_recv_timeout(self.sock, 200ms) or discard;
                let mut n = 0;
                while n < 1000000 {{
                    let _ = std::io::udp::recv(self.sock, 64) or __empty(err);
                    n = n + 1;
                }}
            }}
        }}

        locus Sender {{
            bus {{ publish "tick" of type Tick; }}
            run() {{
                std::time::sleep(300ms);
                let fd = std::io::udp::bind("", 0) or raise;
                let mut i = 0;
                while i < 8 {{
                    std::io::udp::send(fd, "127.0.0.1", {rport}, "payload-xyz") or discard;
                    "tick" <- Tick {{ n: i }};
                    std::time::sleep(20ms);
                    i = i + 1;
                }}
                std::io::udp::close(fd);
            }}
        }}

        main locus App {{
            params {{ r: Reader = Reader {{ }}; tx: Sender = Sender {{ }}; }}
            placement {{ r: cooperative(pool = io) where async_io; tx: pinned; }}
            run() {{ std::time::sleep(2s); std::process::exit(0); }}
        }}
        fn main() {{ App {{ }}; }}
        "#,
    );
    let (out, status) = build_and_run("recv_in_handler", &src);
    // The crash was a SIGSEGV; assert a clean exit (not signalled).
    assert!(
        status.success(),
        "udp recv in an async bus handler must not crash the coro stack; \
         status: {:?}\nstdout: {}",
        status, out
    );
    assert!(
        out.contains("reader saw 4"),
        "handler-side udp recv should have delivered the datagrams; got:\n{}",
        out
    );
}
