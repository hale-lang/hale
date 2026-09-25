//! GH #1030 — a refused TCP connect fails at once; the wait is asked for.
//!
//! `lotus_tcp_connect` retried a refused connect for ~1 s (200 x 5 ms),
//! the shape for a peer starting at the same moment and the wrong
//! default for a client: a dial to a store that was down cost a second,
//! and the pq driver's eight dials made one read ~9 s. `connect` now
//! makes one attempt; `connect_wait(host, port, wait)` keeps the retry
//! for the caller that is racing its peer's `listen()`.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn run(name: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(name);
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{name}: {:?}; stdout: {stdout}; stderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

#[test]
fn a_refused_connect_answers_at_once_and_connect_wait_waits() {
    let port = harness::free_port();
    let src = format!(
        r#"
fn main() {{
    let mut kind = "";
    let t0 = std::time::monotonic();
    let fd = std::io::tcp::connect("127.0.0.1", {port}) or {{ kind = err.kind; - 1 }};
    let fast = std::time::monotonic() - t0;
    println("connect fd=" + to_string(fd) + " kind=" + kind + " fast=" + to_string(fast < 100ms));

    kind = "";
    let t1 = std::time::monotonic();
    let fd2 = std::io::tcp::connect_wait("127.0.0.1", {port}, 150ms) or {{ kind = err.kind; - 1 }};
    let waited = std::time::monotonic() - t1;
    println("wait fd=" + to_string(fd2) + " kind=" + kind + " waited=" + to_string(waited >= 150ms && waited < 1s));
}}
"#
    );
    let out = run("hale_tcp_connect_refused", &src);
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        [
            "connect fd=-1 kind=connection_refused fast=true",
            "wait fd=-1 kind=connection_refused waited=true",
        ],
    );
}

#[test]
fn connect_wait_reaches_a_listener_that_comes_up_during_the_wait() {
    let port = harness::free_port();
    let src = format!(
        r#"
locus Late {{
    run() {{
        std::time::sleep(100ms);
        let l = std::io::tcp::listen_socket("127.0.0.1", {port}) or - 1;
        let c = std::io::tcp::accept_one(l) or - 1;
        let _c = std::io::tcp::close_fd(c);
        let _l = std::io::tcp::close_fd(l);
    }}
}}
main locus App {{
    params {{ late: Late = Late {{ }}; }}
    placement {{ late: pinned; }}
    run() {{
        let fd = std::io::tcp::connect_wait("127.0.0.1", {port}, 3s) or - 1;
        println("connected=" + to_string(fd >= 0));
        let _f = std::io::tcp::close_fd(fd);
    }}
}}
fn main() {{ App {{ }}; }}
"#
    );
    let out = run("hale_tcp_connect_wait_late", &src);
    assert_eq!(out.lines().collect::<Vec<_>>(), ["connected=true"]);
}
