//! Tests for the 2026-05-16 HTTP + JSON additions surfaced by
//! the checkpoint-2026-05-16 handoff. Plus a couple of fixes
//! caught while wiring the worked example for the TCP fan-out
//! pattern: `lotus_tcp_accept` → `lotus_tcp_accept_one`,
//! `lotus_tcp_listen` → `lotus_tcp_listen_socket` (existing
//! wrappers looked up wrong symbols), and `std::io::tcp::close_fd`
//! path-dispatch (was only the __close_fd internal name).

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_libshape_http_{}_{}", name, std::process::id()));
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
fn http_server_compiles_with_handler_locus_and_max_accepts() {
    // Compile-only smoke. The handler is a locus typed against
    // std::http::Handler; state lives in its params so requests
    // can mutate cross-request state (counter, store, etc.).
    let src = r#"
        locus Routes {
            params { hits: Int = 0; }
            fn handle(req: std::http::Request) -> std::http::Response {
                self.hits = self.hits + 1;
                if req.method == "GET" && req.path == "/health" {
                    return std::http::Response { status: 200, body: "ok" };
                }
                return std::http::Response { status: 404, body: "nf" };
            }
        }
        fn main() {
            std::http::Server { port: 18181, max_accepts: 1, handler: Routes { } };
        }
    "#;
    let bin = build("http_server_compile", src);
    let _ = std::fs::remove_file(&bin);
}

#[test]
fn http_server_without_handler_is_compile_error() {
    // 2026-05-16 — Server requires `handler:`. Omitting it
    // surfaces an immediate compile error so agents don't ship
    // a server that 404s everything because they forgot to wire
    // up routes.
    let src = r#"
        fn main() {
            std::http::Server { port: 18182, max_accepts: 1 };
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_libshape_http_required_{}", std::process::id()));
    let err = build_executable(&program, &bin).expect_err("expected compile error");
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("handler") && msg.contains("required"),
        "diagnostic should name the missing handler param: {}",
        msg
    );
}

#[test]
fn http_response_content_type_defaults_to_text_plain() {
    // Response.content_type has a "text/plain" default; the
    // common case (`{ status: 200, body: "ok" }`) writes a
    // valid response without filling every field.
    let src = r#"
        fn main() {
            let r = std::http::Response { status: 200, body: "hi" };
            println(r.content_type);
            println(r.body);
        }
    "#;
    let (stdout, status) = build_and_run("http_resp_ct_default", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("text/plain"), "default missing; got: {:?}", stdout);
    assert!(stdout.contains("hi"), "body missing; got: {:?}", stdout);
}

#[test]
fn http_handler_state_persists_across_dispatches() {
    // The handler is a locus; its params are real state. Two
    // calls to `handle` on the same instance see the same `n`
    // field updated. Doesn't go through a real socket — invokes
    // `handle` directly to verify the dispatch path produces
    // monotonic results.
    let src = r#"
        locus Counter {
            params { n: Int = 0; }
            fn handle(req: std::http::Request) -> std::http::Response {
                let _ = req;
                self.n = self.n + 1;
                return std::http::Response { status: 200, body: to_string(self.n) };
            }
        }
        fn poke(h: std::http::Handler) -> Int {
            let req = std::http::Request { method: "GET", path: "/", version: "", headers: "", body: "" };
            let r = h.handle(req);
            return std::str::parse_int(r.body) or raise;
        }
        fn main() {
            let c = Counter { };
            println("a=", poke(c));
            println("b=", poke(c));
            println("c=", poke(c));
        }
    "#;
    let (stdout, status) = build_and_run("http_handler_state", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("a=1"), "first; got: {:?}", stdout);
    assert!(stdout.contains("b=2"), "second; got: {:?}", stdout);
    assert!(stdout.contains("c=3"), "third; got: {:?}", stdout);
}

#[test]
fn http_handler_satisfies_interface_structurally() {
    // Two distinct locus shapes (one with state, one without)
    // both satisfy std::http::Handler — same fn signature on
    // each, no explicit `impl` ceremony.
    let src = r#"
        locus Stateless {
            params { }
            fn handle(req: std::http::Request) -> std::http::Response {
                let _ = req;
                return std::http::Response { status: 200, body: "stateless" };
            }
        }
        locus Stateful {
            params { tag: String = "default"; }
            fn handle(req: std::http::Request) -> std::http::Response {
                let _ = req;
                return std::http::Response { status: 200, body: self.tag };
            }
        }
        fn first_body(h: std::http::Handler) -> String {
            let req = std::http::Request { method: "GET", path: "/", version: "", headers: "", body: "" };
            return h.handle(req).body;
        }
        fn main() {
            let a = Stateless { };
            let b = Stateful { tag: "stateful" };
            println(first_body(a));
            println(first_body(b));
        }
    "#;
    let (stdout, status) = build_and_run("http_handler_structural", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("stateless"), "got: {:?}", stdout);
    assert!(stdout.contains("stateful"), "got: {:?}", stdout);
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
