//! Tests for the 2026-05-16 HTTP + JSON additions surfaced by
//! the checkpoint-2026-05-16 handoff. Plus a couple of fixes
//! caught while wiring the worked example for the TCP fan-out
//! pattern: `lotus_tcp_accept` → `lotus_tcp_accept_one`,
//! `lotus_tcp_listen` → `lotus_tcp_listen_socket` (existing
//! wrappers looked up wrong symbols), and `std::io::tcp::close_fd`
//! path-dispatch (was only the __close_fd internal name).

use std::process::Command;

use aperio_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_libshape_http_{}_{}", name, std::process::id()));
    build_executable(&program, &bin).expect("build");
    bin
}

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let bin = build(name, src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

// ===== JSON ============================================================

#[test]
fn json_escape_string_handles_control_bytes_and_quotes() {
    let src = r#"
        fn main() {
            let s = "a\"b\\c\nd\te";
            println(std::json::escape_string(s));
        }
    "#;
    let (stdout, status) = build_and_run("json_esc", src);
    assert!(status.success(), "non-zero: {:?}", status);
    // Escaped: a\"b\\c\nd\te (each backslash literal in output)
    assert!(stdout.contains("a\\\"b\\\\c\\nd\\te"), "got: {:?}", stdout);
}

#[test]
fn json_escape_then_unescape_round_trips() {
    let src = r#"
        fn main() {
            let original = "say \"hi\"\nthere\\";
            let escaped = std::json::escape_string(original);
            let back = std::json::unescape_string(escaped);
            if back == original { println("eq"); } else { println("ne"); }
        }
    "#;
    let (stdout, status) = build_and_run("json_round", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("eq"), "got: {:?}", stdout);
}

#[test]
fn json_find_string_field_reads_value() {
    let src = r#"
        fn main() {
            let body = "{ \"id\": 42, \"title\": \"hello\" }";
            println(std::json::find_string_field(body, "title"));
        }
    "#;
    let (stdout, status) = build_and_run("json_str_field", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("hello"), "got: {:?}", stdout);
}

#[test]
fn json_find_int_field_reads_value() {
    let src = r#"
        fn main() {
            let body = "{ \"id\": 42, \"title\": \"x\" }";
            let id = std::json::find_int_field(body, "id");
            println("id=", id);
        }
    "#;
    let (stdout, status) = build_and_run("json_int_field", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("id=42"), "got: {:?}", stdout);
}

#[test]
fn json_find_bool_field_returns_true_or_false() {
    let src = r#"
        fn main() {
            let body = "{ \"done\": true, \"pinned\": false }";
            println("d=", std::json::find_bool_field(body, "done"));
            println("p=", std::json::find_bool_field(body, "pinned"));
        }
    "#;
    let (stdout, status) = build_and_run("json_bool_field", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("d=true"), "got: {:?}", stdout);
    assert!(stdout.contains("p=false"), "got: {:?}", stdout);
}

#[test]
fn json_find_field_returns_empty_for_missing() {
    let src = r#"
        fn main() {
            let body = "{ \"a\": 1 }";
            let v = std::json::find_string_field(body, "missing");
            println("len=", len(v));
        }
    "#;
    let (stdout, status) = build_and_run("json_missing", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("len=0"), "got: {:?}", stdout);
}

#[test]
fn json_array_iter_walks_object_elements() {
    let src = r#"
        fn main() {
            let arr = "[{\"n\":1},{\"n\":10},{\"n\":100}]";
            let mut it = std::json::array_first(arr);
            let mut sum = 0;
            while !it.done {
                sum = sum + std::json::find_int_field(it.element, "n");
                it = std::json::array_next(it);
            }
            println("sum=", sum);
        }
    "#;
    let (stdout, status) = build_and_run("json_arr_iter", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("sum=111"), "got: {:?}", stdout);
}

#[test]
fn json_array_iter_walks_string_elements() {
    let src = r#"
        fn main() {
            let arr = "[\"a\", \"b\", \"c\"]";
            let mut it = std::json::array_first(arr);
            let mut n = 0;
            while !it.done {
                n = n + 1;
                it = std::json::array_next(it);
            }
            println("count=", n);
        }
    "#;
    let (stdout, status) = build_and_run("json_arr_strs", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("count=3"), "got: {:?}", stdout);
}

#[test]
fn json_array_iter_empty_array_is_done_immediately() {
    let src = r#"
        fn main() {
            let it = std::json::array_first("[]");
            if it.done { println("empty"); } else { println("non-empty"); }
        }
    "#;
    let (stdout, status) = build_and_run("json_arr_empty", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("empty"), "got: {:?}", stdout);
}

// ===== HTTP Server =====================================================

#[test]
fn http_server_compiles_with_handler_and_max_accepts() {
    // Compile-only — exercising the Listener-backed Server
    // shape against a real socket would need a port + curl
    // round-trip which is racy under the workspace test
    // harness. The smoke test for end-to-end behavior lived in
    // the brief-example verification during this session.
    let src = r#"
        fn dispatch(req: std::http::Request) -> std::http::Response {
            if req.method == "GET" && req.path == "/health" {
                return std::http::Response {
                    status: 200, content_type: "text/plain", body: "ok"
                };
            }
            return std::http::Response {
                status: 404, content_type: "text/plain", body: "nf"
            };
        }
        fn main() {
            std::http::Server { port: 18181, max_accepts: 1, handler: dispatch };
        }
    "#;
    let bin = build("http_server_compile", src);
    let _ = std::fs::remove_file(&bin);
}

// ===== TCP wrappers (regression for the typo'd C-fn lookups) ===========

#[test]
fn tcp_fanout_pattern_compiles() {
    // Repro for the lotus_tcp_accept / lotus_tcp_listen
    // panics that surfaced when the fallible accept_one /
    // listen_socket wrappers were exercised by the
    // brief-example TCP fan-out pattern. Plus regression for
    // std::io::tcp::close_fd path-dispatch (was missing — only
    // the __close_fd internal name worked).
    let src = r#"
        @form(vec)
        locus FdSet { capacity { heap items of Int; } }

        fn main() {
            let listen_fd = std::io::tcp::listen_socket("127.0.0.1", 19199) or raise;
            let set = FdSet { };
            // Don't actually accept (no client) — just verify
            // the wiring + close_fd path dispatch.
            std::io::tcp::close_fd(listen_fd);
            let _ = set;
        }
    "#;
    let bin = build("tcp_fanout_compile", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
}
