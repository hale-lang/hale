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

use hale_codegen::build_executable_with_options;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn pick_free_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    probe.local_addr().expect("local_addr").port()
}

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_http_router_{}_{}", name, std::process::id()));
    build_executable_with_options(&program, &bin, &[], &build_opts::options()).expect("build");
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
        // GH #1048: the router keeps its handler as a borrow, so the
        // handler is a field of the locus that owns the router
        locus Routes {{
            params {{
                hello: Hello = Hello {{ }};
                router: std::http::Router = std::http::Router {{ }};
            }}
            birth() {{ self.router.add("GET", "/hello/:name", self.hello); }}
            fn handle(req: std::http::Request) -> std::http::Response {{
                return self.router.dispatch(req);
            }}
        }}
        fn main() {{
            std::http::Server {{
                port: {port}, max_accepts: 2, ready_signal: "READY",
                handler: Routes {{ }}
            }};
        }}
    "#
    );
    let program = hale_syntax::parse_source(&src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_http_router_wire_{}", std::process::id()));
    build_executable_with_options(&program, &bin, &[], &build_opts::options()).expect("build");
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

/// Downstream request (2026-08-11): a route can be a bare fn —
/// `router.add_fn("GET", "/x", handler_fn)` — no handler locus.
/// The fn pointer lives in the route entry itself (an adapter
/// locus instantiated inside the register method would dissolve at
/// method exit, out from under the entry), so fn routes and locus
/// routes share one list and one precedence order.
#[test]
fn add_fn_registers_bare_fn_routes() {
    let src = r#"
        fn greet(ctx: std::http::Context) -> std::http::Response {
            let name = std::http::path_param(ctx.params, "name");
            return std::http::Response { status: 200, body: "hi " + name };
        }
        fn ping(ctx: std::http::Context) -> std::http::Response {
            return std::http::Response { status: 200, body: "pong" };
        }
        locus Classic {
            fn handle(ctx: std::http::Context) -> std::http::Response {
                return std::http::Response { status: 200, body: "classic" };
            }
        }
        fn req(path: String) -> std::http::Request {
            return std::http::Request {
                method: "GET", path: path, version: "HTTP/1.1",
                headers: "", body: "", conn_fd: -1
            };
        }
        fn main() {
            let router = std::http::Router { };
            router.add_fn("GET", "/hello/:name", greet);
            router.add_fn("GET", "/ping", ping);
            router.add("GET", "/classic", Classic { });

            let r1 = router.dispatch(req("/hello/ada"));
            println("r1=", to_string(r1.status), " ", r1.body);
            println("r2=", router.dispatch(req("/ping")).body);
            println("r3=", router.dispatch(req("/classic")).body);
            println("r4=", to_string(router.dispatch(req("/nope")).status));
        }
    "#;
    let (out, st) = build_and_run("add_fn", src);
    assert!(st.success(), "non-zero: {:?}\n{}", st, out);
    assert!(out.contains("r1=200 hi ada"), "fn route with capture: {}", out);
    assert!(out.contains("r2=pong"), "plain fn route: {}", out);
    assert!(out.contains("r3=classic"), "locus routes coexist: {}", out);
    assert!(out.contains("r4=404"), "404 default intact: {}", out);
}

// ---- GH #1048: the handler outlives the router that keeps it ---------
//
// `Router.add` keeps its handler as a borrow, so the checker refuses a
// handler literal from a frame the router outlives (that half is
// `hale-cli`'s `check_borrow_lifetime`). These are the shapes it lets
// through. The oracle that would see GH #1048's defect is the OUTPUT:
// the handler's `String` param is built on the heap (a literal's static
// bytes would hide it) and read back through the router after the
// building frame is gone, so a reclaimed handler prints garbage. ASan
// and residency are hygiene here, not the detector: the reclaimed
// handler's bytes stay in memory the process still maps, so neither
// reports the refused shapes either.

/// Run `src` twice: under `LOTUS_ARENA_RESIDENCY=1`, and as an ASan
/// build with chunk recycling off. Returns the first run's stdout.
fn run_under_oracles(name: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_http_router_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).env("LOTUS_ARENA_RESIDENCY", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "{name}: exit {:?}\n{stdout}\n{stderr}", out.status);
    assert!(
        stderr.contains("[arena_residency dump] 0 live arenas"),
        "{name}: an arena outlived the program:\n{stderr}"
    );
    let asan = harness::unique_bin(&format!("hale_http_router_{}_asan", name));
    harness::build_asan(&program, &asan);
    let out = Command::new(&asan)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .env("ASAN_OPTIONS", "detect_leaks=0")
        .output()
        .expect("run the asan build");
    let _ = std::fs::remove_file(&asan);
    let asan_err = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        out.status.success() && !asan_err.contains("ERROR: AddressSanitizer"),
        "{name}: the sanitizer reported:\n{asan_err}"
    );
    stdout
}

const ECHO: &str = r#"
    locus Echo {
        params { s: String = ""; }
        fn handle(ctx: std::http::Context) -> std::http::Response {
            return std::http::Response { status: 200, body: "[" + self.s + "]" };
        }
    }
"#;

#[test]
fn a_handler_field_of_the_routers_owner_outlives_the_building_frame() {
    // the issue's third row, the sound way: the owner holds the handler
    // as a field and fills the router in `birth()`; the owner is built
    // in one fn and dispatched through from another
    let src = format!(
        r#"{ECHO}
        locus Api {{
            params {{ dir: String = ""; echo: Echo = Echo {{ }}; router: std::http::Router = std::http::Router {{ }}; }}
            birth() {{
                self.echo.s = self.dir + "/canned";
                self.router.add("GET", "/x", self.echo);
            }}
            fn handle(req: std::http::Request) -> std::http::Response {{ return self.router.dispatch(req); }}
        }}
        fn serve(a: Api) -> String {{
            let noise = std::str::repeat("z", 64);
            return a.handle(std::http::Request {{ method: "GET", path: "/x" }}).body + " " + to_string(len(noise));
        }}
        fn main() {{
            let a = Api {{ dir: std::str::upper("some") + "/dir" }};
            println(serve(a));
        }}
        "#
    );
    let out = run_under_oracles("owner_field", &src);
    assert!(out.contains("[SOME/dir/canned] 64"), "{out}");
}

#[test]
fn a_router_built_and_used_in_one_frame_reads_its_handler() {
    let src = format!(
        r#"{ECHO}
        fn main() {{
            let r = std::http::Router {{ }};
            r.add("GET", "/x", Echo {{ s: std::str::upper("d") + "/canned" }});
            let h = Echo {{ s: std::str::upper("e") + "/let" }};
            r.add("GET", "/y", h);
            let noise = std::str::repeat("z", 64);
            println(r.dispatch(std::http::Request {{ method: "GET", path: "/x" }}).body,
                r.dispatch(std::http::Request {{ method: "GET", path: "/y" }}).body, " ", len(noise));
        }}
        "#
    );
    let out = run_under_oracles("one_frame", &src);
    assert!(out.contains("[D/canned][E/let] 64"), "{out}");
}
