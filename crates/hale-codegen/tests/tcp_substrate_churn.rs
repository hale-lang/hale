//! Churning connections through a TCP substrate listener leaves nothing on
//! its long-lived consumer.
//!
//! A listener relays each connection as an `Accepted { listener, route, fd,
//! seq }` handle over the bus; a consumer on the listener's own thread (a
//! `std::http::Server` on an async_io pool, an `Acceptor` and its
//! `Connections` on one pool, a `std::io::tcp::Listener`) used to receive
//! it deserialized into its locus arena, so the handle's two Strings — the
//! listener's name and the route — stayed there: ~34 B a connection for a
//! `127.0.0.1:<port>` name, for the life of the server. A delivery's payload
//! now lives in a region of the delivery's own, on every thread
//! (`bus_same_thread_payload_reclaim.rs`).
//!
//! The oracle is the consumer's arena, to the byte (the residency dump's
//! `used=`, `LOTUS_ARENA_RESIDENCY=1`): after a warm-up and after many more
//! connections, the program its own client.

#![cfg(target_os = "linux")]

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;

const WARM: usize = 300;
const MORE: usize = 3000;

// The client: a connection, a request, the answer read to its close.
const CLIENT: &str = r#"
    fn one(host: String, port: Int) {
        let fd = std::io::tcp::connect(host, port) or - 1;
        if fd < 0 {
            println("connect failed");
            return;
        }
        let s = std::io::tcp::Stream { conn_fd: fd };
        s.send("GET / HTTP/1.0\r\nHost: x\r\n\r\n") or discard;
        let mut n = 0;
        while n < 100 {
            let chunk = s.recv(4096) or "";
            if len(chunk) == 0 {
                break;
            }
            n = n + 1;
        }
    }
    fn churn(host: String, port: Int, n: Int) {
        let mut i = 0;
        while i < n {
            one(host, port);
            i = i + 1;
        }
        std::time::sleep(100ms);
        println("mark");
        let _ = std::process::dump_arena_residency();
    }
    locus Hello {
        fn handle(req: std::http::Request) -> std::http::Response {
            return std::http::Response { status: 200, body: "ok" };
        }
    }
    fn echo(s: std::io::tcp::Stream) {
        let _ = s.recv(4096) or "";
        s.send("HTTP/1.0 200 OK\r\n\r\nok") or discard;
    }
    main locus App {
        params {
            host: String = std::env::arg(1);
            port: Int = std::str::parse_int(std::env::arg(2)) or 0;
            SERVER
        }
        placement { PLACEMENT }
        run() {
            std::time::sleep(200ms);
            churn(self.host, self.port, WARM);
            churn(self.host, self.port, MORE);
            std::process::exit(0);
        }
    }
    fn main() { App { }; }
"#;

/// The consumers' (`__StdIoTcpConnections`) `used=` bytes in each dump.
fn consumer_used(stderr: &str) -> Vec<usize> {
    stderr
        .split("[arena_residency dump]")
        .skip(1)
        .map(|d| {
            d.lines()
                .filter(|l| l.contains("label=__StdIoTcpConnections]"))
                .filter_map(|l| l.rsplit("used=").next()?.trim().parse::<usize>().ok())
                .sum()
        })
        .collect()
}

fn churn_case(tag: &str, server: &str, placement: &str) {
    let src = CLIENT
        .replace("SERVER", server)
        .replace("PLACEMENT", placement)
        .replace("WARM", &WARM.to_string())
        .replace("MORE", &MORE.to_string());
    let bin = harness::unique_bin(tag);
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    // the program is its own client, so it is handed the address it
    // serves on: one the test holds, so nothing else takes it meanwhile
    let at = child::Reserved::new();
    let out = Command::new(&bin)
        .arg(at.host())
        .arg(at.port().to_string())
        .env("LOTUS_ARENA_RESIDENCY", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{tag}: {:?}\n{stdout}\n{stderr}", out.status);
    assert!(!stdout.contains("connect failed"), "{tag}: every connection was accepted:\n{stdout}");
    let used = consumer_used(&stderr);
    assert!(used.len() >= 2, "{tag}: a dump after the warm-up and one after the rest:\n{stderr}");
    let grew = used[1].saturating_sub(used[0]);
    // ~34 B a connection was the residue (100 KB over these 3000); flat is
    // nothing, and anything near a byte a connection is the handle again
    assert!(
        grew < MORE / 4,
        "{tag}: the consumer's arena grew {grew} B over {MORE} connections ({} -> {})",
        used[0],
        used[1]
    );
}

#[test]
fn an_async_io_servers_consumer_stays_flat_under_churn() {
    churn_case(
        "tcp_churn_server",
        "web: std::http::Server = std::http::Server { host: self.host, port: self.port, handler: Hello { } };",
        "web: cooperative(pool = web) where async_io;",
    );
}

#[test]
fn a_consumer_on_its_acceptors_pool_stays_flat_under_churn() {
    churn_case(
        "tcp_churn_acceptor",
        "c: std::io::tcp::Connections = std::io::tcp::Connections { listener: \"web\", handler: std::http::Responder { handler: Hello { } } };\n\
         a: std::io::tcp::Acceptor = std::io::tcp::Acceptor { host: self.host, port: self.port, name: \"web\" };",
        "c: cooperative(pool = web) where async_io; a: cooperative(pool = web) where async_io;",
    );
}

#[test]
fn an_async_io_listeners_consumer_stays_flat_under_churn() {
    churn_case(
        "tcp_churn_listener",
        "l: std::io::tcp::Listener = std::io::tcp::Listener { host: self.host, port: self.port, max_accepts: -1, on_connection: echo };",
        "l: cooperative(pool = web) where async_io;",
    );
}
