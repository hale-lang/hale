//! GH #1566 — F.31's runtime backstop.
//!
//! The checker refuses a store into, or a method call on, a locus that
//! runs on another pool wherever its walk follows the handle. What it
//! does not follow — here, a call through a fn value — is enforced at
//! run time: the compiled check compares the receiver's domain with the
//! running code's and, on a mismatch, reports the site and exits 1
//! before the store touches the other pool's arena.
//!
//! `Routes { w: self.a }` is built in the server's params on the `io`
//! worker; its handler runs there and hands `self.w` — `App.a`, built on
//! `main` — to `touch` through a fn value, which the checker's walk does
//! not see into, so `hale check` accepts the program. At run time the
//! store in `touch` is made on `io` into a `W` that runs on `main`.
//! The same holds through an interface: `poke(n: Namer)` calls
//! `n.rename(..)` on the locus behind the fat pointer, checked by the
//! `__domain` offset its vtable carries.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;

/// `same_pool`: `App.a` is placed on `io` with the server, so the store
/// the handler makes is its own pool's.
fn program(same_pool: bool) -> String {
    program_with(
        same_pool,
        "fn touch(w: W) {
    w.name = std::str::upper(\"from the server's pool\");
}",
        "let f: fn(W) = touch;
        f(self.w);",
    )
}

/// The handler reaches `App.a` through an interface-typed parameter: a
/// call through an interface is checked on the locus behind it.
fn iface_program(same_pool: bool) -> String {
    program_with(
        same_pool,
        "interface Namer {
    fn rename(s: String);
}
fn poke(n: Namer) { n.rename(std::str::upper(\"from the server's pool\")); }
fn touch(w: W) { poke(w); }",
        "let f: fn(W) = touch;
        f(self.w);",
    )
}

fn program_with(same_pool: bool, extra: &str, body: &str) -> String {
    let placement = if same_pool {
        "placement { srv: cooperative(pool = io); a: cooperative(pool = io); }"
    } else {
        "placement { srv: cooperative(pool = io); }"
    };
    format!(
        r#"locus W {{
    params {{ name: String = ""; }}
    fn rename(s: String) {{ self.name = s; }}
}}
{extra}
locus Routes {{
    params {{ w: W; }}
    fn handle(req: std::http::Request) -> std::http::Response {{
        {body}
        return std::http::Response {{ status: 200, body: "ok" }};
    }}
}}
main locus App {{
    params {{
        a: W = W {{ }};
        srv: std::http::Server = std::http::Server {{ host: "127.0.0.1", port: 0, handler: Routes {{ w: self.a }} }};
    }}
    {placement}
    run() {{
        let mut i = 0;
        while i < 100 && !self.draining {{ std::time::sleep(50ms); i = i + 1; }}
        println("APP DONE");
    }}
}}
fn main() {{ App {{ }}; }}
"#
    )
}

/// Build `src` (with its source table, so the report is located), serve one
/// request, and wait for the program to end: its exit status, stdout,
/// stderr, and the response's status line.
fn serve_one(src: &str, name: &str, asan: bool) -> (std::process::ExitStatus, String, String, String) {
    let bin = harness::unique_bin(name);
    let mut options = build_opts::options();
    options.site_sources = Some(hale_codegen::DebugSources {
        files: vec![hale_codegen::DebugSourceFile {
            base: 0,
            len: src.len() as u32,
            path: std::path::PathBuf::from("main.hl"),
            text: src.to_string(),
        }],
    });
    options.asan = asan;
    build_opts::build_source(src, &bin, &options).expect("build");
    let mut cmd = Command::new(&bin);
    if asan {
        cmd.env("LOTUS_NO_CHUNK_POOL", "1");
    }
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("spawn");
    let port = child::tcp_port(&mut child);
    let t0 = Instant::now();
    let mut stream = loop {
        match std::net::TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => break s,
            Err(e) if t0.elapsed() > Duration::from_secs(10) => panic!("connect: {e}"),
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    stream.set_read_timeout(Some(Duration::from_secs(10))).expect("timeout");
    stream.write_all(b"GET / HTTP/1.0\r\nHost: x\r\n\r\n").expect("request");
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    let status_line = String::from_utf8_lossy(&response).lines().next().unwrap_or("").to_string();
    let out = child.wait_with_output().expect("wait");
    let _ = std::fs::remove_file(&bin);
    (
        out.status,
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        status_line,
    )
}

const REPORT: &str = "violation: cross-pool store into `W.name` from pool `io`; this `W` runs on pool `main`: \
                      reach it through a method on the bus";

#[test]
fn a_store_the_checker_does_not_follow_is_refused_at_run_time() {
    let src = program(false);
    let (status, stdout, stderr, _) = serve_one(&src, "hale_1566_backstop", false);
    assert_eq!(status.code(), Some(1), "a violation exits 1: {status:?}\nstdout: {stdout}\nstderr: {stderr}");
    assert!(
        stderr.contains(&format!("main.hl:6:5: {REPORT}")),
        "the report names the site, the store and both pools: {stderr}"
    );
    assert!(!stdout.contains("APP DONE"), "nothing runs after the violation: {stdout}");
}

#[test]
fn the_same_store_on_its_own_pool_runs() {
    let src = program(true);
    let (status, stdout, stderr, line) = serve_one(&src, "hale_1566_backstop_same", false);
    assert!(line.contains("200"), "the request is answered: {line:?}; stderr: {stderr}");
    assert!(status.success(), "exit 0: {status:?}\nstderr: {stderr}");
    assert!(stdout.contains("APP DONE"), "stdout: {stdout}");
    assert!(!stderr.contains("violation"), "stderr: {stderr}");
}

#[test]
fn a_call_through_an_interface_is_checked_on_the_locus_behind_it() {
    let src = iface_program(false);
    let (status, stdout, stderr, _) = serve_one(&src, "hale_1566_backstop_iface", false);
    assert_eq!(status.code(), Some(1), "a violation exits 1: {status:?}\nstdout: {stdout}\nstderr: {stderr}");
    assert!(
        stderr.contains(
            "main.hl:8:21: violation: cross-pool method call `Namer.rename` from pool `io`; this `Namer` runs on pool `main`"
        ),
        "the report names the site, the call and both pools: {stderr}"
    );
    let src = iface_program(true);
    let (status, stdout, stderr, line) = serve_one(&src, "hale_1566_backstop_iface_same", false);
    assert!(line.contains("200"), "the request is answered: {line:?}; stderr: {stderr}");
    assert!(status.success() && stdout.contains("APP DONE"), "exit 0: {status:?}\nstderr: {stderr}");
}

#[test]
fn the_backstop_is_asan_clean() {
    for same_pool in [false, true] {
        let src = program(same_pool);
        let (status, _, stderr, _) = serve_one(&src, "hale_1566_backstop_asan", true);
        assert!(!stderr.contains("AddressSanitizer"), "same_pool={same_pool}: {stderr}");
        assert_eq!(status.code(), Some(if same_pool { 0 } else { 1 }), "same_pool={same_pool}: {stderr}");
    }
}

/// An adapter a `bindings { }` entry builds runs its own thread, and the
/// runtime calls its `send` from every publishing thread through a
/// function pointer, by design: it and what its params build belong to no
/// one domain, so the backstop does not stop a `send` that writes through
/// what it holds (the trio worker's `NatsPublisher` reveals its
/// credential there, on `main`).
#[test]
fn an_adapters_send_on_the_publishing_thread_is_let_through() {
    let src = r#"
type Note { n: Int = 0; }
topic Ping { payload: Note; subject: "probe.ping"; }
locus Tally { params { last: String = ""; } }
locus Keep {
    params { tally: Tally = Tally { }; }
    fn send(subject: String, bytes: Bytes) {
        let t = self.tally;
        t.last = std::str::upper(subject);
    }
}
main locus App {
    bindings { Ping: Keep { }; }
    bus { publish Ping; }
    run() {
        Ping <- Note { n: 1 };
        Ping <- Note { n: 2 };
        println("sent");
    }
}
fn main() { App { }; }
"#;
    let bin = harness::unique_bin("hale_1566_backstop_adapter");
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{:?}\nstderr: {stderr}", out.status);
    assert!(String::from_utf8_lossy(&out.stdout).contains("sent"), "stderr: {stderr}");
    assert!(!stderr.contains("violation"), "stderr: {stderr}");
}

/// Down `self`, through a field a literal hands an instance built
/// elsewhere (`Routes { w: self.a }`): the checker refuses these (the
/// held field keeps its source's domain), and the backstop stops them
/// too — the harness builds without the check's gate, so the run shows
/// the second layer. A store, a method call, and a store through a
/// locus the code built that holds the handed-in instance.
#[test]
fn down_self_through_a_handed_in_field_is_checked() {
    let program = |body: &str| {
        program_with(
            false,
            "locus Holder {
    params { w: W; }
    fn poke(s: String) { self.w.name = s; }
}",
            body,
        )
    };
    for (body, what, at) in [
        ("self.w.name = std::str::upper(\"x\");", "store into `W.name`", "main.hl:12:9"),
        ("self.w.rename(std::str::upper(\"x\"));", "method call `W.rename`", "main.hl:12:9"),
        ("let h = Holder { w: self.w }; h.poke(std::str::upper(\"x\"));", "store into `W.name`", "main.hl:7:26"),
    ] {
        let (status, _, stderr, _) = serve_one(&program(body), "hale_1566_backstop_down_self", false);
        assert_eq!(status.code(), Some(1), "{body}: {stderr}");
        assert!(
            stderr.contains(&format!("{at}: violation: cross-pool {what} from pool `io`; this `W` runs on pool `main`")),
            "{body}: {stderr}"
        );
    }
}

fn build_run(src: &str, name: &str, asan: bool) -> std::process::Output {
    let bin = harness::unique_bin(name);
    let options = hale_codegen::BuildOptions { asan, ..build_opts::options() };
    build_opts::build_source(src, &bin, &options).expect("build");
    let mut cmd = Command::new(&bin);
    if asan {
        cmd.env("LOTUS_NO_CHUNK_POOL", "1").env("ASAN_OPTIONS", "detect_stack_use_after_return=1");
    }
    let out = cmd.output().expect("run");
    let _ = std::fs::remove_file(&bin);
    out
}

/// A pool-placed root's `drain()` / `dissolve()` run on main after its
/// pool joined, as the root's own work: a call on its child through a
/// helper there is the root's domain's, not a cross-pool call.
#[test]
fn a_pool_placed_roots_teardown_runs_as_its_own_domain() {
    let src = r#"
locus C {
    params { name: String = "c"; }
    fn bye() { println(f"C.bye {self.name}"); }
}
fn close(c: C) { c.bye(); }
locus P {
    params { c: C = C { name: "a" }; }
    run() { std::time::sleep(20ms); }
    drain() { let c = self.c; c.bye(); }
    dissolve() { close(self.c); }
}
main locus App {
    params { q: P = P { }; }
    placement { q: cooperative(pool = io); }
    run() { std::time::sleep(100ms); println("APP DONE"); }
}
fn main() { App { }; }
"#;
    let out = build_run(src, "hale_1566_backstop_teardown", false);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{:?}\nstderr: {stderr}", out.status);
    assert_eq!(stdout.matches("C.bye a").count(), 2, "stdout: {stdout}");
}

/// An `on_failure` handler reaches its failed child's subtree for its
/// duration: the child is stopped while its owner's handler runs.
#[test]
fn an_on_failure_handler_reaches_its_failed_childs_subtree() {
    let src = r#"
locus C {
    params { name: String = "c"; }
    fn hello() { println(f"C.hello {self.name}"); }
}
fn greet(c: C) { c.hello(); }
locus Q {
    params { c: C = C { name: "a" }; n: Int = 0; }
    closure fuse { captures: n; epoch inline; }
    run() { violate fuse; }
}
main locus App {
    params { q: Q = Q { }; }
    placement { q: cooperative(pool = io); }
    on_failure(q: Q, err: ClosureViolation) {
        q.c.hello();
        greet(q.c);
    }
    run() { std::time::sleep(200ms); println("APP DONE"); }
}
fn main() { App { }; }
"#;
    let out = build_run(src, "hale_1566_backstop_on_failure", false);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{:?}\nstderr: {stderr}", out.status);
    assert_eq!(stdout.matches("C.hello a").count(), 2, "stdout: {stdout}");
}

/// An array of loci a pool-placed root's params build lives as long as
/// the root: its elements are in the root's arena, not the init job's
/// frame (they were a dead stack slot by `dissolve()`, read on main
/// after the pool joined).
#[test]
fn a_pool_placed_roots_array_of_loci_outlives_its_init() {
    let src = r#"
locus C {
    params { name: String = "c"; }
    fn bye() { println(f"C.bye {self.name}"); }
}
locus P {
    params { cs: [C; 2] = [C { name: "a" }, C { name: "b" }]; }
    run() { std::time::sleep(20ms); }
    dissolve() { for c in self.cs { c.bye(); } }
}
main locus App {
    params { q: P = P { }; }
    placement { q: cooperative(pool = io); }
    run() { std::time::sleep(100ms); println("APP DONE"); }
}
fn main() { App { }; }
"#;
    for asan in [false, true] {
        let out = build_run(src, "hale_1566_pool_array_of_loci", asan);
        let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success(), "asan={asan}: {:?}\nstderr: {stderr}", out.status);
        assert!(stdout.contains("C.bye a\nC.bye b"), "asan={asan}: stdout: {stdout}");
        assert!(!stderr.contains("Sanitizer"), "asan={asan}: {stderr}");
    }
}

/// An instance a local hands to a holder on another pool is adopted:
/// it, and the children it owns, run where the holder does — `Web`,
/// built in `fn main` and handed to an io-placed server's handler, serves
/// from io, its own child included. A builder that kept the local and
/// reaches the instance from main afterwards is caught.
#[test]
fn an_instance_handed_over_runs_where_its_holder_does() {
    let program = |keep: &str| {
        format!(
            r#"locus Inner {{ params {{ s: String = ""; }} }}
locus Web {{
    params {{ name: String = "w"; inner: Inner = Inner {{ }}; }}
    fn serves() -> Bool {{
        let i = self.inner;
        i.s = std::str::upper("on the server's pool");
        return true;
    }}
}}
locus Api {{
    params {{ web: Web; }}
    fn handle(req: std::http::Request) -> std::http::Response {{
        let w = self.web;
        let ok = w.serves();
        return std::http::Response {{ status: 200, body: "ok" }};
    }}
}}
main locus App {{
    params {{ srv: std::http::Server; }}
    placement {{ srv: cooperative(pool = io); }}
    run() {{
        let mut i = 0;
        while i < 20 && !self.draining {{ std::time::sleep(50ms); i = i + 1; }}
        println("APP DONE");
    }}
}}
fn main() {{
    let web = Web {{ }};
    App {{ srv: std::http::Server {{ host: "127.0.0.1", port: 0, handler: Api {{ web: web }} }} }};
    {keep}
}}
"#
        )
    };
    let (status, stdout, stderr, line) = serve_one(&program(""), "hale_1566_adopt", false);
    assert!(line.contains("200"), "the request is answered: {line:?}; stderr: {stderr}");
    assert!(status.success() && stdout.contains("APP DONE"), "{status:?}\nstderr: {stderr}");
    let keep = "web.name = std::str::upper(\"from main\");";
    let (status, _, stderr, _) = serve_one(&program(keep), "hale_1566_adopt_alias", false);
    assert_eq!(status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("main.hl:30:5: violation: cross-pool store into `Web.name` from pool `main`; this `Web` runs on pool `io`"),
        "{stderr}"
    );
}

/// An instance a locus already holds, handed to a holder on another pool
/// by a path the checker does not follow (a builder called through a fn
/// value): the holder shares it, its domain stays its owner's, and the
/// holder's store down `self` through that field is caught where it is
/// made — whichever builder spelling hands it over. The same hand-over
/// within one pool runs.
#[test]
fn an_alias_handed_across_pools_through_a_fn_value_is_caught() {
    let program = |builder: &str, same_pool: bool| {
        let placement = if same_pool {
            "placement { srv: cooperative(pool = io); a: cooperative(pool = io); }"
        } else {
            "placement { srv: cooperative(pool = io); }"
        };
        format!(
            r#"locus W {{ params {{ name: String = ""; }} }}
locus Routes {{
    params {{ w: W; }}
    fn handle(req: std::http::Request) -> std::http::Response {{
        self.w.name = std::str::upper("from the server's pool");
        return std::http::Response {{ status: 200, body: "ok" }};
    }}
}}
fn mk(w: W) -> Routes {{ return Routes {{ w: w }}; }}
fn mk2(w: W) -> Routes {{ let r = Routes {{ w: w }}; return r; }}
fn mk3(w: W) -> Routes {{ return Routes {{ w: {{ let x = w; x }} }}; }}
fn via(f: fn(W) -> Routes, w: W) -> Routes {{ return f(w); }}
main locus App {{
    params {{
        a: W = W {{ }};
        srv: std::http::Server = std::http::Server {{ host: "127.0.0.1", port: 0, handler: via({builder}, self.a) }};
    }}
    {placement}
    run() {{
        let mut i = 0;
        while i < 40 && !self.draining {{ std::time::sleep(50ms); i = i + 1; }}
        println("APP DONE");
    }}
}}
fn main() {{ App {{ }}; }}
"#
        )
    };
    for builder in ["mk", "mk2", "mk3"] {
        let (status, stdout, stderr, _) = serve_one(&program(builder, false), "hale_1566_alias_fn_value", false);
        assert_eq!(status.code(), Some(1), "{builder}: {status:?}\nstdout: {stdout}\nstderr: {stderr}");
        assert!(stderr.contains(&format!("main.hl:5:9: {REPORT}")), "{builder}: {stderr}");
        let (status, stdout, stderr, line) = serve_one(&program(builder, true), "hale_1566_alias_fn_value_same", false);
        assert!(line.contains("200"), "{builder}: the request is answered: {line:?}; stderr: {stderr}");
        assert!(status.success() && stdout.contains("APP DONE"), "{builder}: {status:?}\nstderr: {stderr}");
    }
}

/// An `on_failure` handler reaches its failed child's domain for the
/// child and what it owns only: a store through a fn value into the
/// child's own child is let through, one into another instance running
/// on that pool is a violation.
#[test]
fn an_on_failure_handler_reaches_only_its_failed_childs_subtree() {
    let program = |target: &str| {
        format!(
            r#"locus C {{ params {{ name: String = "c"; }} }}
fn touch(c: C) {{ c.name = std::str::upper("from the handler"); }}
locus Q {{
    params {{ c: C = C {{ name: "a" }}; n: Int = 0; }}
    closure fuse {{ captures: n; epoch inline; }}
    run() {{ violate fuse; }}
}}
locus R {{
    params {{ c: C = C {{ name: "r" }}; }}
    run() {{ while !self.draining {{ std::time::sleep(10ms); }} }}
}}
main locus App {{
    params {{ q: Q = Q {{ }}; r: R = R {{ }}; }}
    placement {{ q: cooperative(pool = io); r: cooperative(pool = io); }}
    on_failure(q: Q, err: ClosureViolation) {{
        let f: fn(C) = touch;
        f({target});
        println("HANDLED");
    }}
    run() {{ std::time::sleep(200ms); println("APP DONE"); }}
}}
fn main() {{ App {{ }}; }}
"#
        )
    };
    let out = build_run(&program("q.c"), "hale_1566_on_failure_own", false);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && stdout.contains("HANDLED"), "{:?}\nstdout: {stdout}\nstderr: {stderr}", out.status);
    let out = build_run(&program("self.r.c"), "hale_1566_on_failure_other", false);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.status.code(), Some(1), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stderr.contains("violation: cross-pool store into `C.name` from pool `main`; this `C` runs on pool `io`"), "{stderr}");
    assert!(!stdout.contains("HANDLED"), "{stdout}");
}

/// The hand-over the DNA head makes: an instance `fn main` builds goes
/// through a chain of helpers' parameters into the literal of an
/// io-placed server's handler. Nothing held it before, so it moves to
/// io with what it owns and serves from there.
#[test]
fn a_fresh_instance_handed_through_parameters_serves_from_its_holders_pool() {
    let src = r#"locus Inner { params { s: String = ""; } }
locus Web {
    params { name: String = "w"; inner: Inner = Inner { }; }
    fn serves() -> Bool {
        self.name = std::str::upper("on the server's pool");
        let i = self.inner;
        i.s = std::str::upper("on the server's pool");
        return true;
    }
}
locus Api {
    params { web: Web; }
    fn handle(req: std::http::Request) -> std::http::Response {
        let ok = self.web.serves();
        return std::http::Response { status: 200, body: "ok" };
    }
}
main locus App {
    params { srv: std::http::Server; }
    placement { srv: cooperative(pool = io); }
    run() {
        let mut i = 0;
        while i < 20 && !self.draining { std::time::sleep(50ms); i = i + 1; }
        println("APP DONE");
    }
}
fn up(web: Web) { App { srv: std::http::Server { host: "127.0.0.1", port: 0, handler: Api { web: web } } }; }
fn serve(web: Web) { up(web); }
fn main() {
    let web = Web { };
    let loaded = web.serves();
    serve(web);
}
"#;
    let (status, stdout, stderr, line) = serve_one(src, "hale_1566_adopt_params", false);
    assert!(line.contains("200"), "the request is answered: {line:?}; stderr: {stderr}");
    assert!(status.success() && stdout.contains("APP DONE"), "{status:?}\nstderr: {stderr}");
}

/// A violation the backstop finds inside the stdlib (here, the router
/// calling a route handler that runs on main, from the server's pool) has
/// no location of its own: it is reported at the program's call into the
/// stdlib it is under.
#[test]
fn a_violation_inside_the_stdlib_names_the_programs_call() {
    let src = r#"locus H {
    params { note: String = ""; }
    fn handle(ctx: std::http::Context) -> std::http::Response {
        self.note = std::str::upper("hit");
        return std::http::Response { status: 200, body: "h" };
    }
}
locus Routes {
    params { r: std::http::Router = std::http::Router { }; h: H; ready: Bool = false; }
    fn handle(req: std::http::Request) -> std::http::Response {
        if !self.ready { self.r.add("GET", "/", self.h); self.ready = true; }
        return self.r.dispatch(req);
    }
}
main locus App {
    params {
        h: H = H { };
        srv: std::http::Server = std::http::Server { host: "127.0.0.1", port: 0, handler: Routes { h: self.h } };
    }
    placement { srv: cooperative(pool = io); }
    run() {
        let mut i = 0;
        while i < 40 && !self.draining { std::time::sleep(50ms); i = i + 1; }
        println("APP DONE");
    }
}
fn main() { App { }; }
"#;
    let (status, _, stderr, _) = serve_one(src, "hale_1566_stdlib_site", false);
    assert_eq!(status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("main.hl:12:16: violation: cross-pool method call `StdHttpRouteHandler.handle` from pool `io` (inside the stdlib, under the call here); this `StdHttpRouteHandler` runs on pool `main`"),
        "{stderr}"
    );
}

/// The DNA API's shape (the face's Knowledge reads, CI on 6b70fc90c): a
/// provider `fn main`'s helper builds — holding a store that owns a
/// connection — is handed to an io-placed server's handler through a
/// field of interface type. The provider moves to io with what it owns,
/// so a free fn the store hands its connection to (`dial(self.conn)`)
/// fills it on io, its own pool, and no violation is reported.
#[test]
fn a_provider_handed_through_an_interface_field_moves_with_what_it_owns() {
    let src = r#"locus Conn { params { host: String = ""; } }
fn dial(c: Conn, host: String) { c.host = host; }
locus Store {
    params { conn: Conn = Conn { }; }
    fn open() -> Bool { dial(self.conn, std::str::upper("db")); return true; }
}
interface Provider { fn read() -> String; }
locus Local {
    params { root: String = "."; store: Store = Store { }; }
    fn read() -> String { let ok = self.store.open(); return self.root; }
}
locus Api {
    params { knowledge: Provider; }
    fn handle(req: std::http::Request) -> std::http::Response {
        return std::http::Response { status: 200, body: self.knowledge.read() };
    }
}
main locus App {
    params { srv: std::http::Server; }
    placement { srv: cooperative(pool = io); }
    run() {
        let mut i = 0;
        while i < 20 && !self.draining { std::time::sleep(50ms); i = i + 1; }
        println("APP DONE");
    }
}
fn up(knowledge: Provider) { App { srv: std::http::Server { host: "127.0.0.1", port: 0, handler: Api { knowledge: knowledge } } }; }
fn main() {
    let knowledge = Local { root: "r" };
    up(knowledge);
}
"#;
    let (status, stdout, stderr, line) = serve_one(src, "hale_1566_adopt_iface", false);
    assert!(line.contains("200"), "the request is answered: {line:?}; stderr: {stderr}");
    assert!(status.success() && stdout.contains("APP DONE"), "{status:?}\nstderr: {stderr}");
    assert!(!stderr.contains("violation"), "{stderr}");
}

/// A graph composed from locals and handed over whole (the DNA heads
/// build their command providers this way): the provider holds a service
/// through an interface, the service a store of its own. The provider's
/// first holder moves it, and it moves what it holds first in turn, so
/// the service answers on the server's pool too.
#[test]
fn a_composed_graph_moves_with_its_first_holder() {
    let src = r#"locus Conn { params { host: String = ""; } }
fn dial(c: Conn, host: String) { c.host = host; }
interface Service { fn supported() -> Bool; }
locus Native {
    params { conn: Conn = Conn { }; }
    fn supported() -> Bool { dial(self.conn, std::str::upper("db")); return true; }
}
locus Provider {
    params { service: Service; }
    fn read() -> String { if self.service.supported() { return "yes"; } return "no"; }
}
locus Api {
    params { provider: Provider; }
    fn handle(req: std::http::Request) -> std::http::Response {
        return std::http::Response { status: 200, body: self.provider.read() };
    }
}
main locus App {
    params { srv: std::http::Server; }
    placement { srv: cooperative(pool = io); }
    run() {
        let mut i = 0;
        while i < 20 && !self.draining { std::time::sleep(50ms); i = i + 1; }
        println("APP DONE");
    }
}
fn up(provider: Provider) { App { srv: std::http::Server { host: "127.0.0.1", port: 0, handler: Api { provider: provider } } }; }
fn main() {
    let native = Native { };
    let provider = Provider { service: native };
    up(provider);
}
"#;
    let (status, stdout, stderr, line) = serve_one(src, "hale_1566_adopt_graph", false);
    assert!(line.contains("200"), "the request is answered: {line:?}; stderr: {stderr}");
    assert!(status.success() && stdout.contains("APP DONE"), "{status:?}\nstderr: {stderr}");
    assert!(!stderr.contains("violation"), "{stderr}");
}

/// A locus literal handed to a holder through a call's argument — a fn
/// that hands it back (`id(W { })`), a builder that keeps it in the
/// instance it returns (`mk(W { })`, and with the instance bound first)
/// — is owned where the call's value is: the holder's, torn down with
/// it, never a temporary of the frame that built it. It was: the frame
/// reclaimed it as soon as the server's params were built, and the
/// handler then wrote into a freed instance whose `__domain` was whatever
/// the memory held. Plain and sanitized, on a classic and an async_io
/// pool.
#[test]
fn an_argument_the_holder_keeps_is_owned_by_the_holder() {
    let program = |handler: &str, async_io: bool| {
        let placement = if async_io { "srv: cooperative(pool = io) where async_io;" } else { "srv: cooperative(pool = io);" };
        format!(
            r#"locus W {{
    params {{ name: String = "w"; }}
    dissolve() {{ println(f"W.dissolve {{self.name}}"); }}
}}
locus Routes {{
    params {{ w: W; }}
    fn handle(req: std::http::Request) -> std::http::Response {{
        self.w.name = std::str::upper("from the server's pool");
        return std::http::Response {{ status: 200, body: "ok" }};
    }}
}}
fn id(w: W) -> W {{ return w; }}
fn mk(w: W) -> Routes {{ return Routes {{ w: w }}; }}
fn mk2(w: W) -> Routes {{ let r = Routes {{ w: w }}; return r; }}
main locus App {{
    params {{
        srv: std::http::Server = std::http::Server {{ host: "127.0.0.1", port: 0, handler: {handler} }};
    }}
    placement {{ {placement} }}
    run() {{
        let mut i = 0;
        while i < 20 && !self.draining {{ std::time::sleep(50ms); i = i + 1; }}
        println("APP DONE");
    }}
}}
fn main() {{ App {{ }}; }}
"#
        )
    };
    for handler in ["Routes { w: id(W { }) }", "mk(W { })", "mk2(W { })"] {
        for async_io in [false, true] {
            for asan in [false, true] {
                let what = format!("{handler}, async_io={async_io}, asan={asan}");
                let (status, stdout, stderr, line) = serve_one(&program(handler, async_io), "hale_1566_kept_argument", asan);
                assert!(line.contains("200"), "{what}: the request is answered: {line:?}; stderr: {stderr}");
                assert!(status.success() && stdout.contains("APP DONE"), "{what}: {status:?}\nstderr: {stderr}");
                assert!(!stderr.contains("violation") && !stderr.contains("Sanitizer"), "{what}: {stderr}");
                // torn down once, with its holder, after the program's run
                let done = stdout.find("APP DONE").expect("checked");
                assert_eq!(stdout.matches("W.dissolve").count(), 1, "{what}: {stdout}");
                assert!(stdout.find("W.dissolve").expect("counted") > done, "{what}: {stdout}");
            }
        }
    }
}
