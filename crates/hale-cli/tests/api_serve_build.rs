//! GH #1417 (R2a): a build refuses a serve site over a transport this
//! compiler does not ship. The R0 witness serves `Public` over
//! `http::Rpc` and `Admin` over `unix::Rpc`; `hale check` admits it
//! (R1's description and the serve-site laws read it as written), and
//! `hale build` says what it cannot serve yet, instead of dropping the
//! sites (R2b: `unix::Rpc` is served, `http::Rpc` is not). A hub binding
//! is refused likewise (R5).

use std::path::PathBuf;
use std::process::Command;

fn witness() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("tests/api-contract/program.hl");
    p
}

#[test]
fn a_build_refuses_a_serve_site_over_a_transport_it_does_not_ship() {
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("hale_api_serve_build_{}_witness", std::process::id()));
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("build")
        .arg(witness())
        .arg("-o")
        .arg(&out_path)
        .env("HALE_SKIP_STALE_CHECK", "1")
        .output()
        .expect("run hale build");
    let _ = std::fs::remove_file(&out_path);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "the witness builds:\n{stderr}");
    // (R2b ships `unix::Rpc`; `http::Rpc` follows in R3)
    assert!(
        stderr.contains("`api::serve` over `http::Rpc`: this compiler serves a surface over"),
        "no refusal of `http::Rpc` in:\n{stderr}"
    );
    assert!(!stderr.contains("`api::serve` over `unix::Rpc`"), "`unix::Rpc` is served:\n{stderr}");
    assert!(stderr.contains("`Fills` is bound to the hub `self.hub`"), "the hub binding is refused too:\n{stderr}");
}

#[test]
fn a_check_admits_the_witness() {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(witness())
        .env("HALE_SKIP_STALE_CHECK", "1")
        .output()
        .expect("run hale check");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

/// `Server` is a plain locus holding or writing a `unix::Rpc` and serving
/// over it; `App` is the main locus that holds it.
fn unix_server(transport_param: &str, transport: &str) -> String {
    format!(
        r#"
api Public {{ rpc Echo::echo; }}
locus Echo {{ fn echo(n: Int) -> Int {{ return n; }} }}
locus Server {{
    params {{
        echo: Echo = Echo {{ }};
        {transport_param}
    }}
    run() {{
        let h = api::serve(Public, {transport}, as: "public", bound: 2, on_full: refuse);
        h.stop();
    }}
}}
main locus App {{ params {{ server: Server = Server {{ }}; }} }}
fn main() {{ App {{ }}; }}
"#
    )
}

/// Builds `src` (dumping the IR beside the output); (built, stderr, the IR).
fn build_source(name: &str, src: &str) -> (bool, String, String) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let stem = format!("hale_api_serve_build_{}_{}_{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed), name);
    let dir = std::env::temp_dir().join(&stem);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.hl");
    std::fs::write(&file, src).unwrap();
    let out_path = dir.join("out");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("build")
        .arg(&file)
        .arg("-o")
        .arg(&out_path)
        .env("HALE_SKIP_STALE_CHECK", "1")
        .env("LOTUS_DUMP_IR", "1")
        .output()
        .expect("run hale build");
    let ir = std::fs::read_to_string(dir.join("out.ll")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    (out.status.success(), String::from_utf8_lossy(&out.stderr).into_owned(), ir)
}

const NON_MAIN: &str = "`api::serve` over `unix::Rpc` in `Server`: a socket's listener runs on a pool of its own";

/// A Unix transport served from a locus that is not `main` has no listener
/// (only the main locus places the pool it needs): refused whether the
/// transport is written at the serve site or held as a param.
#[test]
fn a_unix_serve_outside_main_is_refused_written_or_held() {
    let written = unix_server("", "unix::Rpc { path: \"/tmp/hale_r2b_a.sock\" }");
    let (ok, err, _) = build_source("written", &written);
    assert!(!ok && err.contains(NON_MAIN), "written: {err}");

    let held = unix_server("rpc: std::api::unix::Rpc = std::api::unix::Rpc { path: \"/tmp/hale_r2b_b.sock\" };", "self.rpc");
    let (ok, err, _) = build_source("held", &held);
    assert!(!ok && err.contains(NON_MAIN), "held: {err}");

    // the declared type alone says it: the default is built elsewhere
    let typed = unix_server("rpc: std::api::unix::Rpc = make_rpc();", "self.rpc").replace(
        "api Public",
        "fn make_rpc() -> std::api::unix::Rpc { return std::api::unix::Rpc { path: \"/tmp/hale_r2b_c.sock\" }; }\napi Public",
    );
    let (ok, err, _) = build_source("typed", &typed);
    assert!(!ok && err.contains(NON_MAIN), "typed: {err}");
}

/// The main locus's serve over a `unix::Rpc` whose path reads a param the
/// locus declares: the listener is born after that param, so its default
/// (the copied path) reads an initialized `self.socket_path`.
fn main_serving(path: &str) -> String {
    format!(
        r#"
api Public {{ rpc Echo::echo; }}
locus Echo {{ fn echo(n: Int) -> Int {{ return n; }} }}
main locus App {{
    params {{
        socket_path: String = "/tmp/hale_r2b_d.sock";
        echo: Echo = Echo {{ }};
    }}
    run() {{
        let h = api::serve(Public, unix::Rpc {{ path: {path} }}, as: "public", bound: 2, on_full: refuse);
        h.stop();
    }}
}}
fn main() {{ App {{ }}; }}
"#
    )
}

#[test]
fn a_listener_path_may_read_a_param_declared_before_the_serve() {
    let (ok, err, read) = build_source("path_reads_param", &main_serving("self.socket_path"));
    assert!(ok, "the path reads a declared param: {err}");
    let (ok, err, literal) = build_source("path_literal", &main_serving("\"/tmp/hale_r2b_d.sock\""));
    assert!(ok, "{err}");
    assert!(literal.contains("__rpc_u_1"), "the control schedules the listener's birth");
    assert!(read.contains("__rpc_u_1"), "the listener's birth is scheduled when the path reads a param");
}
