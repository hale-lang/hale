//! GH #1565: an exposure is a param of the locus that holds it, its literal
//! the serve call, placed by `placement { }`.
//!
//! These build programs that serve that way and drive them over real
//! sockets:
//!
//! * the main locus's own exposures, over `http::Rpc` (placed on a pool by
//!   its row) and `unix::Rpc` (unplaced), each answering a call;
//! * server loci below the main locus, one placed by main on an `async_io`
//!   pool and one a level deeper, each answering, its description stating
//!   the port its listener was given (the address handed down the literals
//!   that build it);
//! * `stop()` and `wait()` on the param, and a main locus with no `run()`
//!   serving until SIGTERM;
//! * a server whose listener cannot bind failing the program's birth.
//!
//! Where each listener runs is the placement table's, asserted in
//! `crates/hale-types/tests/api_serve_check.rs`.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/http_rpc.rs"]
mod http_rpc;

use http_rpc::*;

/// The declarations every program here shares: a surface of one member and
/// the sources an HTTP exposure reads.
const COMMON: &str = r#"
type Ping { n: Int; }
type Pong { n: Int; by: String; }

api Echo { rpc Echoer::echo; }

locus Tokens {
    fn principal(token: String) -> std::api::Principal {
        if token == "t-alice" { return std::api::Principal { mode: "bearer", name: "alice" }; }
        return std::api::Principal { mode: "bearer", name: "" };
    }
    fn refused() -> String { return "no such token"; }
}

locus Echoer {
    params { base: Int = 0; seen: Int = 0; }
    fn echo(p: Ping, ctx: std::api::Context) -> Pong {
        self.seen = self.seen + 1;
        return Pong { n: self.base + p.n, by: ctx.via };
    }
}

fn written(path: String) -> Bool {
    return (std::io::fs::read_file(path) or "") != "";
}
"#;

/// The main locus's own exposures: `web` over HTTP on `$BIND`, its listener
/// placed on `io` by its row, and `sock` over the Unix socket `$SOCK`,
/// unplaced. `run()` stops both when `$TRIGGER` appears, then waits on the
/// stopped exposure, which returns at once.
const MAIN_HELD: &str = r#"
main locus App {
    params {
        tokens: Tokens = Tokens { };
        echoer: Echoer = Echoer { base: 100 };
        web: api::Exposure = api::serve(Echo, http::Rpc { bind: std::env::var("BIND"), codec: json, principals: self.tokens }, as: "web", receivers: { Echoer: self.echoer }, bound: 8, on_full: refuse);
        sock: api::Exposure = api::serve(Echo, unix::Rpc { path: std::env::var("SOCK") }, as: "sock", receivers: { Echoer: self.echoer }, bound: 8, on_full: refuse);
    }
    placement {
        echoer: cooperative(pool = work) where async_io;
        web: cooperative(pool = io) where async_io;
    }
    run() {
        while !self.draining && !written(std::env::var("TRIGGER")) { std::time::sleep(10ms); }
        self.web.stop();
        self.sock.stop();
        self.web.wait();
        println("stopped, waited, served " + to_string(self.echoer.seen));
    }
}

fn main() { App { }; }
"#;

/// Two server loci: `AdminServer` over HTTP on `$BIND`, held by main and
/// placed on `io`; `DeepServer` over HTTP on `$BIND2`, a level deeper (a
/// param of `Wrapper`, which main places on `io2`), its bind a param the
/// wrapper's literal gives it. Each holds its own receiver and sources.
const SERVERS: &str = r#"
locus AdminServer {
    params {
        tokens: Tokens = Tokens { };
        echoer: Echoer = Echoer { base: 1000 };
        api: api::Exposure = api::serve(Echo, http::Rpc { bind: std::env::var("BIND"), codec: json, principals: self.tokens }, as: "admin", receivers: { Echoer: self.echoer }, bound: 8, on_full: refuse);
    }
}

locus DeepServer {
    params {
        bind: String = "";
        tokens: Tokens = Tokens { };
        echoer: Echoer = Echoer { base: 2000 };
        api: api::Exposure = api::serve(Echo, http::Rpc { bind: self.bind, codec: json, principals: self.tokens }, as: "deep", receivers: { Echoer: self.echoer }, bound: 8, on_full: refuse);
    }
}

locus Wrapper {
    params { inner: DeepServer = DeepServer { bind: std::env::var("BIND2") }; }
}

main locus App {
    params {
        admin: AdminServer = AdminServer { };
        deep: Wrapper = Wrapper { };
    }
    placement {
        admin: cooperative(pool = io) where async_io;
        deep: cooperative(pool = io2) where async_io;
    }
    run() {
        while !self.draining && !written(std::env::var("TRIGGER")) { std::time::sleep(10ms); }
        println("stopped");
    }
}

fn main() { App { }; }
"#;

/// A main locus that writes no `run()`: it serves until SIGTERM.
const NO_RUN: &str = r#"
main locus App {
    params {
        tokens: Tokens = Tokens { };
        echoer: Echoer = Echoer { };
        web: api::Exposure = api::serve(Echo, http::Rpc { bind: std::env::var("BIND"), codec: json, principals: self.tokens }, as: "web", receivers: { Echoer: self.echoer }, bound: 8, on_full: refuse);
    }
    placement { web: cooperative(pool = io) where async_io; }
}

fn main() { App { }; }
"#;

/// `run()` parks on the exposure: `wait()` returns on the drain.
const WAITS: &str = r#"
main locus App {
    params {
        tokens: Tokens = Tokens { };
        echoer: Echoer = Echoer { };
        web: api::Exposure = api::serve(Echo, http::Rpc { bind: std::env::var("BIND"), codec: json, principals: self.tokens }, as: "web", receivers: { Echoer: self.echoer }, bound: 8, on_full: refuse);
    }
    run() {
        self.web.wait();
        println("wait returned");
    }
}

fn main() { App { }; }
"#;

fn built(name: &str, body: &str) -> PathBuf {
    let bin = harness::unique_bin(name);
    build_opts::build_source(&format!("{COMMON}{body}"), &bin, &build_opts::options()).unwrap_or_else(|e| panic!("build {name}: {e:?}"));
    bin
}

fn main_held() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| built("api_exposure_main_held", MAIN_HELD)).clone()
}

fn servers() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| built("api_exposure_servers", SERVERS)).clone()
}

fn echo(to: impl Reach, n: i64) -> Response {
    request(
        to,
        "POST",
        "/call/Echoer::echo",
        &[(String::from("Authorization"), String::from("Bearer t-alice")), (String::from("Content-Type"), String::from("application/json"))],
        Some(&format!("{{\"n\":{n}}}")),
    )
}

fn sigterm(pid: u32) {
    let sent = std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).status().expect("kill");
    assert!(sent.success(), "sent SIGTERM");
}

#[test]
fn the_main_locus_serves_its_exposure_params_over_http_and_unix() {
    let server = Server::start(&main_held(), &[]);
    server.ready();
    let got = echo(server.port, 7);
    assert_eq!(got.status, 200, "{}", got.body);
    assert_eq!(got.body, "{\"n\":107,\"by\":\"http\"}");
    wait_accepting_unix(&server.sock());
    let line = unix_ask(&server.sock(), "{\"call\": \"Echoer::echo\", \"payload\": {\"n\": 8}, \"id\": 1}");
    assert!(line.contains("\"value\":{\"n\":108,\"by\":\"unix\"}"), "{line}");
    let port = server.port;
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
    // stop() then wait(): the wait returns once the exposure has stopped
    assert!(done.stdout.contains("stopped, waited, served 2"), "{}", done.stdout);
    assert!(closed(port), "the listener is released");
}

#[test]
fn server_loci_below_main_serve_where_main_places_them() {
    let server = Server::start(&servers(), &[]);
    server.ready();
    let admin = echo(server.port, 1);
    assert_eq!(admin.status, 200, "{}", admin.body);
    assert_eq!(admin.body, "{\"n\":1001,\"by\":\"http\"}");
    let deep_at = server.port2();
    wait_listening(deep_at);
    let deep = echo(deep_at, 2);
    assert_eq!(deep.status, 200, "{}", deep.body);
    assert_eq!(deep.body, "{\"n\":2002,\"by\":\"http\"}");
    // each describes the address its listener was given, though the
    // listener is the main locus's: the bound port is handed down
    let auth = [(String::from("Authorization"), String::from("Bearer t-alice"))];
    let described = request(server.port, "GET", "/.description", &auth, None);
    assert_eq!(described.status, 200, "{}", described.body);
    assert!(described.body.contains(&format!("\"address\":\"127.0.0.1:{}\"", server.port)), "{}", described.body);
    let described = request(deep_at, "GET", "/.description", &auth, None);
    assert!(described.body.contains(&format!("\"address\":\"{deep_at}\"")), "{}", described.body);
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
}

#[test]
fn a_server_whose_listener_cannot_bind_fails_the_birth() {
    let held = std::net::TcpListener::bind("127.0.0.1:0").expect("hold a port");
    let port = held.local_addr().unwrap().port();
    let mut server = Server::start(&servers(), &[("BIND", &format!("127.0.0.1:{port}"))]);
    let done = server.wait(Duration::from_secs(30));
    assert_eq!(done.status.code(), Some(2), "the birth fails: {:?}\n{}\n{}", done.status, done.stdout, done.stderr);
    assert!(done.stderr.contains("api: http::Rpc could not listen on 127.0.0.1:"), "{}", done.stderr);
    drop(held);
}

#[test]
fn a_main_locus_with_no_run_serves_until_sigterm() {
    let bin = built("api_exposure_no_run", NO_RUN);
    let mut server = Server::start(&bin, &[]);
    server.ready();
    assert_eq!(echo(server.port, 3).status, 200);
    std::thread::sleep(Duration::from_millis(400));
    assert!(!server.exited(), "a main with no run() is still serving");
    assert_eq!(echo(server.port, 4).status, 200);
    sigterm(server.pid());
    let done = server.wait(Duration::from_secs(30));
    assert!(done.status.success(), "the drain ends it cleanly: {:?}\n{}", done.status, done.stderr);
}

#[test]
fn wait_on_the_param_returns_on_the_drain() {
    let bin = built("api_exposure_waits", WAITS);
    let mut server = Server::start(&bin, &[]);
    server.ready();
    assert_eq!(echo(server.port, 5).status, 200);
    sigterm(server.pid());
    let done = server.wait(Duration::from_secs(30));
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
    assert!(done.stdout.contains("wait returned"), "{}", done.stdout);
}

/// A server whose address is its own param, which a construction of the
/// main locus may override, at one level or two: the listener (the main
/// locus's) reads the address off the server as that construction built it,
/// and the exposure describes the port the kernel gave it. The defaults
/// listen on `127.0.0.3`; `Server::start` waits for `127.0.0.1`, `$BIND`.
const OVERRIDDEN: &str = r#"
locus Server {
    params {
        bind: String = "127.0.0.3:0";
        tokens: Tokens = Tokens { };
        echoer: Echoer = Echoer { base: 10 };
        web: api::Exposure = api::serve(Echo, http::Rpc { bind: self.bind, codec: json, principals: self.tokens }, as: "web", receivers: { Echoer: self.echoer }, bound: 8, on_full: refuse);
    }
}

locus Inner {
    params {
        bind: String = "127.0.0.3:0";
        tokens: Tokens = Tokens { };
        echoer: Echoer = Echoer { base: 20 };
        api: api::Exposure = api::serve(Echo, http::Rpc { bind: self.bind, codec: json, principals: self.tokens }, as: "inner", receivers: { Echoer: self.echoer }, bound: 8, on_full: refuse);
    }
}

locus Wrapper {
    params { inner: Inner = Inner { }; }
}

main locus App {
    params {
        server: Server = Server { };
        deep: Wrapper = Wrapper { };
    }
    placement {
        server: cooperative(pool = io) where async_io;
        deep: cooperative(pool = io2) where async_io;
    }
    run() {
        while !self.draining && !written(std::env::var("TRIGGER")) { std::time::sleep(10ms); }
    }
}
"#;

fn overridden(name: &str, main_fn: &str) -> PathBuf {
    built(name, &format!("{OVERRIDDEN}\n{main_fn}\n"))
}

/// `(status, address)` of the description at `to`.
fn described(to: impl Reach) -> String {
    let got = request(to, "GET", "/.description", &[(String::from("Authorization"), String::from("Bearer t-alice"))], None);
    assert_eq!(got.status, 200, "{}", got.body);
    got.body
}

#[test]
fn a_construction_override_of_a_servers_address_is_the_address_its_listener_binds() {
    // one level: `App { server: Server { bind: … } }`
    let bin = overridden("api_exposure_override_one", "fn main() { App { server: Server { bind: std::env::var(\"BIND\") } }; }");
    let server = Server::start(&bin, &[]);
    server.ready();
    let got = echo(server.port, 1);
    assert_eq!(got.body, "{\"n\":11,\"by\":\"http\"}", "{}", got.body);
    assert!(described(server.port).contains(&format!("\"address\":\"127.0.0.1:{}\"", server.port)));
    assert!(server.finish().status.success());

    // two levels: `App { deep: Wrapper { inner: Inner { bind: … } } }`
    let bin = overridden("api_exposure_override_two", "fn main() { App { deep: Wrapper { inner: Inner { bind: std::env::var(\"BIND\") } } }; }");
    let server = Server::start(&bin, &[]);
    server.ready();
    let got = echo(server.port, 2);
    assert_eq!(got.body, "{\"n\":22,\"by\":\"http\"}", "{}", got.body);
    assert!(described(server.port).contains(&format!("\"address\":\"127.0.0.1:{}\"", server.port)));
    assert!(server.finish().status.success());
}

#[test]
fn a_server_rebuilt_by_a_construction_keeps_the_bound_address() {
    // `App { server: Server { } }` replaces the default literal: the listener
    // still binds the server's address and the exposure still learns the port
    let src = OVERRIDDEN.replace("bind: String = \"127.0.0.3:0\";\n        tokens: Tokens = Tokens { };\n        echoer: Echoer = Echoer { base: 10 };", "bind: String = std::env::var(\"BIND\");\n        tokens: Tokens = Tokens { };\n        echoer: Echoer = Echoer { base: 10 };");
    assert_ne!(src, OVERRIDDEN);
    let bin = built("api_exposure_rebuilt", &format!("{src}\nfn main() {{ App {{ server: Server {{ }} }}; }}\n"));
    let server = Server::start(&bin, &[]);
    server.ready();
    assert_eq!(echo(server.port, 3).body, "{\"n\":13,\"by\":\"http\"}");
    assert!(described(server.port).contains(&format!("\"address\":\"127.0.0.1:{}\"", server.port)));
    assert!(server.finish().status.success());
}

/// A server another seed declares, binding its receiver explicitly (`receivers:
/// { Worker: self.worker }`), over the Unix socket and HTTP: the binding names
/// the library's own `Worker`, which the application's rows see by its
/// mangled name.
const SERVER_LIB: &str = r#"
type Req { n: Int; }
type Res { n: Int; }
locus Worker {
    params { base: Int = 300; }
    fn ask(r: Req) -> Res { return Res { n: self.base + r.n }; }
}
api Public { rpc Worker::ask; }
locus Tokens {
    fn principal(token: String) -> std::api::Principal {
        if token == "t-alice" { return std::api::Principal { mode: "bearer", name: "alice" }; }
        return std::api::Principal { mode: "bearer", name: "" };
    }
    fn refused() -> String { return "no such token"; }
}
locus Server {
    params {
        tokens: Tokens = Tokens { };
        worker: Worker = Worker { };
        spare: Worker = Worker { base: 900 };
        sock: api::Exposure = api::serve(Public, unix::Rpc { path: std::env::var("SOCK") }, as: "sock", receivers: { Worker: self.worker }, bound: 8, on_full: refuse);
        web: api::Exposure = api::serve(Public, http::Rpc { bind: std::env::var("BIND"), codec: json, principals: self.tokens }, as: "web", receivers: { Worker: self.worker }, bound: 8, on_full: refuse);
    }
}
"#;

const SERVES_LIB: &str = r#"
import "../lib" as lib;
main locus App {
    params { server: lib::Server = lib::Server { }; }
    placement { server: cooperative(pool = io) where async_io; }
}
fn main() { App { }; }
"#;

#[test]
fn an_imported_server_binds_its_receiver_explicitly() {
    let dir = harness::unique_dir("api_exposure_imported");
    std::fs::create_dir_all(dir.join("app")).expect("the app dir");
    std::fs::create_dir_all(dir.join("lib")).expect("the lib dir");
    std::fs::write(dir.join("lib/server.hl"), SERVER_LIB).expect("write the library");
    let entry = dir.join("app/main.hl");
    std::fs::write(&entry, SERVES_LIB).expect("write the app");
    let bin = harness::unique_bin("api_exposure_imported");
    build_opts::build_seed_dir(&entry, &bin, &build_opts::options()).expect("an explicit binding in an imported server builds");
    let mut server = Server::start(&bin, &[]);
    server.ready();
    let got = request(
        server.port,
        "POST",
        "/call/Worker::ask",
        &[(String::from("Authorization"), String::from("Bearer t-alice")), (String::from("Content-Type"), String::from("application/json"))],
        Some("{\"n\":4}"),
    );
    assert_eq!(got.status, 200, "{}", got.body);
    assert_eq!(got.body, "{\"n\":304}", "the bound receiver answers, not the spare");
    wait_accepting_unix(&server.sock());
    let line = unix_ask(&server.sock(), "{\"call\": \"Worker::ask\", \"payload\": {\"n\": 5}, \"id\": 1}");
    assert!(line.contains("\"value\":{\"n\":305}"), "{line}");
    sigterm(server.pid());
    let done = server.wait(Duration::from_secs(30));
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
}
