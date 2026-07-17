//! std::http::Router (promoted from pond/router, 2026-07-17).
//!
//! Two layers: dispatch-level tests drive `Router.dispatch(req)`
//! directly (routing, `:name` captures, query params, middleware
//! onion, stateful handlers, the 404 default, method matching) with
//! no sockets; the wire test mounts the Router as a Server handler
//! and asserts over a real TCP round-trip that the interface
//! satisfaction (`Router` IS a `std::http::Handler`) holds through
//! the vtable.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::thread;
use std::time::Duration;

use hale_codegen::build_executable;

fn pick_free_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    probe.local_addr().expect("local_addr").port()
}

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_http_router_{}_{}", name, std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

#[test]
fn dispatch_routes_captures_middleware_and_404() {
    let src = r#"
        locus Hello {
            fn handle(ctx: std::http::Context) -> std::http::Response {
                let who = std::http::path_param(ctx.params, "name");
                let greet = std::http::query_param(ctx.params, "greet");
                let g = if len(greet) > 0 { greet } else { "hi" };
                return std::http::Response { status: 200, body: g + " " + who };
            }
        }
        locus Count {
            params { hits: Int = 0; }
            fn handle(ctx: std::http::Context) -> std::http::Response {
                self.hits = self.hits + 1;
                return std::http::Response { status: 200, body: "hits=" + self.hits };
            }
        }
        locus Stamp {
            fn before(ctx: std::http::Context) -> std::http::Context {
                return ctx;
            }
            fn after(ctx: std::http::Context, resp: std::http::Response) -> std::http::Response {
                return std::http::Response {
                    status: resp.status,
                    content_type: resp.content_type,
                    headers: "X-Stamp: yes",
                    body: resp.body
                };
            }
        }
        fn main() {
            let r = std::http::Router { };
            r.add("GET", "/hello/:name", Hello { });
            r.add("get", "/count", Count { });
            r.use(Stamp { });

            let r1 = r.dispatch(std::http::Request {
                method: "GET", path: "/hello/world?greet=yo",
                version: "HTTP/1.1", headers: "", body: ""
            });
            println("r1 ", r1.status, " [", r1.body, "] hdr=", r1.headers);
            let r2 = r.dispatch(std::http::Request {
                method: "GET", path: "/count",
                version: "HTTP/1.1", headers: "", body: ""
            });
            let r3 = r.dispatch(std::http::Request {
                method: "GET", path: "/count",
                version: "HTTP/1.1", headers: "", body: ""
            });
            println("r2 [", r2.body, "] r3 [", r3.body, "]");
            let r4 = r.dispatch(std::http::Request {
                method: "GET", path: "/nope",
                version: "HTTP/1.1", headers: "", body: ""
            });
            let r5 = r.dispatch(std::http::Request {
                method: "POST", path: "/hello/x",
                version: "HTTP/1.1", headers: "", body: ""
            });
            println("r4 ", r4.status, " r5 ", r5.status);
        }
    "#;
    let (out, status) = build_and_run("dispatch", src);
    assert!(status.success(), "exit: {:?}\n{}", status, out);
    // :name capture + query param + middleware header stamp.
    assert!(out.contains("r1 200 [yo world] hdr=X-Stamp: yes"), "got:\n{}", out);
    // Stateful handler accumulates across dispatches; register-time
    // method uppercasing ("get" == "GET").
    assert!(out.contains("r2 [hits=1] r3 [hits=2]"), "got:\n{}", out);
    // Unmatched path AND method mismatch both hit the 404 default.
    assert!(out.contains("r4 404 r5 404"), "got:\n{}", out);
}

#[test]
fn router_serves_through_server_over_tcp() {
    let port = pick_free_port();
    let src = format!(
        r#"
        locus Hello {{
            fn handle(ctx: std::http::Context) -> std::http::Response {{
                let who = std::http::path_param(ctx.params, "name");
                return std::http::Response {{ status: 200, body: "hi " + who }};
            }}
        }}
        fn build_router() -> std::http::Router {{
            let r = std::http::Router {{ }};
            r.add("GET", "/hello/:name", Hello {{ }});
            return r;
        }}
        fn main() {{
            std::http::Server {{
                port: {port}, max_accepts: 2, ready_signal: "READY",
                handler: build_router()
            }};
        }}
    "#
    );
    let program = hale_syntax::parse_source(&src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_http_router_wire_{}", std::process::id()));
    build_executable(&program, &bin).expect("build");
    let mut child = Command::new(&bin)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn server");

    // Wait for READY (bounded).
    let mut ready = false;
    for _ in 0..50 {
        thread::sleep(Duration::from_millis(100));
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            ready = true;
            break;
        }
    }
    assert!(ready, "server never started listening");

    let fetch = |path: &str| -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        write!(s, "GET {} HTTP/1.1\r\nHost: t\r\n\r\n", path).expect("send");
        let mut buf = String::new();
        s.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let _ = s.read_to_string(&mut buf);
        buf
    };
    // The probe connect above consumed one accept only if it sent a
    // request; it sent nothing and closed — the Server's conn
    // handler serves what it has (no complete header -> close), so
    // budget an extra accept isn't needed: max_accepts counts
    // accepts, and the probe consumed one. Use the remaining one.
    let r1 = fetch("/hello/wire");
    assert!(r1.starts_with("HTTP/1.1 200"), "got:\n{}", r1);
    assert!(r1.contains("hi wire"), "got:\n{}", r1);

    let _ = child.wait();
    let _ = std::fs::remove_file(&bin);
}
