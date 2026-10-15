//! GH #1566 — F.31's runtime backstop.
//!
//! The checker refuses a store into, or a method call on, a locus that
//! runs on another pool wherever its walk follows the handle. What it
//! does not follow — here, a handler the runtime calls through a
//! function pointer, which runs on the server's pool — is enforced at
//! run time: the compiled check compares the receiver's domain with the
//! running code's and, on a mismatch, reports the site and exits 1
//! before the store touches the other pool's arena.
//!
//! `Routes { w: self.a }` is built in the server's params on the `io`
//! worker, so the checker places it on `io` with the `W` it holds (a held
//! instance runs in its holder's domain): `let w = self.w; w.name = …` is
//! a same-pool store to it, and `hale check` accepts the program. At run
//! time `w` is `App.a`, built on `main`, and the handler runs on `io`.

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
    let placement = if same_pool {
        "placement { srv: cooperative(pool = io); a: cooperative(pool = io); }"
    } else {
        "placement { srv: cooperative(pool = io); }"
    };
    format!(
        r#"locus W {{
    params {{ name: String = ""; }}
}}
locus Routes {{
    params {{ w: W; }}
    fn handle(req: std::http::Request) -> std::http::Response {{
        let w = self.w;
        w.name = std::str::upper("from the server's pool");
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

/// Build `src` (with a line table, so the report is located), serve one
/// request, and wait for the program to end: its exit status, stdout,
/// stderr, and the response's status line.
fn serve_one(src: &str, name: &str, asan: bool) -> (std::process::ExitStatus, String, String, String) {
    let bin = harness::unique_bin(name);
    let mut options = build_opts::options();
    options.debug = Some(hale_codegen::DebugSources {
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
        stderr.contains(&format!("main.hl:8:9: {REPORT}")),
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
fn the_backstop_is_asan_clean() {
    for same_pool in [false, true] {
        let src = program(same_pool);
        let (status, _, stderr, _) = serve_one(&src, "hale_1566_backstop_asan", true);
        assert!(!stderr.contains("AddressSanitizer"), "same_pool={same_pool}: {stderr}");
        assert_eq!(status.code(), Some(if same_pool { 0 } else { 1 }), "same_pool={same_pool}: {stderr}");
    }
}
