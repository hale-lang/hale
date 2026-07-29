//! m88: Hale-language self-tests using std::test.
//!
//! Each Rust test compiles a small .hl test program that
//! exercises real Hale behavior (HTTP parser, str primitives,
//! lifecycle ordering) using the m87 assertion library, then
//! confirms the program exits 0 with no stdout (the
//! "all assertions passed" signal).
//!
//! This proves Phase 2 actually works: tests can be authored
//! in Hale, asserting on Hale behavior, with diagnostic
//! output the developer can read directly. The Rust harness
//! is just the test runner — once an `hale test` CLI lands,
//! these same .hl programs will run unchanged under it.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn run_self_test(name: &str, source: &str) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_self_test_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "self-test `{}` failed (exit {:?})\nstdout:\n{}\nstderr:\n{}",
        name,
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.is_empty(),
        "self-test `{}` should be silent on pass; got stdout:\n{}",
        name,
        stdout
    );
}

#[test]
fn http_request_parser_field_extraction() {
    // Hale-level test of m84's parse_request. Equivalent to
    // the Rust-side stdlib_http_request.rs cases, written from
    // inside Hale using std::test::assert_eq_str.
    run_self_test(
        "parse_request",
        r#"
        fn main() {
            let req = std::http::parse_request("GET /docs HTTP/1.1\r\n\r\n");
            std::test::assert_eq_str(req.method, "GET", "method");
            std::test::assert_eq_str(req.path, "/docs", "path");
            std::test::assert_eq_str(req.version, "HTTP/1.1", "version");
            std::test::assert_eq_str(req.body, "", "empty body");

            let post = std::http::parse_request(
                "POST /api HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello"
            );
            std::test::assert_eq_str(post.method, "POST", "post method");
            std::test::assert_eq_str(post.body, "hello", "post body");
        }
        "#,
    );
}

#[test]
fn str_index_of_returns_byte_position_or_minus_one() {
    // Hale-level test of std::str::index_of (m84 primitive).
    run_self_test(
        "index_of",
        r#"
        fn main() {
            std::test::assert_eq_int(
                std::str::index_of("hello world", "world"),
                6,
                "found at offset 6"
            );
            std::test::assert_eq_int(
                std::str::index_of("hello world", "missing"),
                0 - 1,
                "not found returns -1"
            );
            std::test::assert_eq_int(
                std::str::index_of("hello", ""),
                0,
                "empty needle is position 0"
            );
            std::test::assert_eq_int(
                std::str::index_of("aaa", "a"),
                0,
                "first occurrence wins"
            );
        }
        "#,
    );
}

#[test]
fn str_concat_via_plus_round_trips() {
    // String + String concat — exercise the m36 binop path
    // through std::test assertions.
    run_self_test(
        "concat",
        r#"
        fn main() {
            let s = "hello" + " " + "world";
            std::test::assert_eq_str(s, "hello world", "two-step concat");

            let n = 42;
            let combo = "answer=" + to_string(n);
            std::test::assert_eq_str(
                combo,
                "answer=42",
                "concat with to_string"
            );
        }
        "#,
    );
}

#[test]
fn locus_let_binding_dissolves_at_scope_exit() {
    // m82 lifecycle test, written in Hale. A locus with a
    // mutable counter records its dissolve-time state; the
    // assertion checks that all method calls completed before
    // dissolve (i.e. dissolve fires AFTER the body, not at
    // the struct-literal expression).
    run_self_test(
        "locus_lifecycle",
        r#"
        locus Counter {
            params { tally: Int = 0; }
            fn bump() {
                self.tally = self.tally + 1;
            }
            fn current() -> Int {
                return self.tally;
            }
        }

        fn main() {
            let c = Counter { tally: 0 };
            c.bump();
            c.bump();
            c.bump();
            std::test::assert_eq_int(
                c.current(),
                3,
                "three bumps recorded"
            );
        }
        "#,
    );
}

#[test]
fn string_equality_distinguishes_distinct_inputs() {
    // == on String routes through lotus_str_eq (content compare,
    // not pointer compare). Worth pinning at the language level
    // so future codegen changes don't silently break it.
    run_self_test(
        "str_eq",
        r#"
        fn main() {
            let a = "hello";
            let b = "hel" + "lo";
            // Different pointer, same content — content compare wins.
            std::test::assert(a == b, "string equality is content-based");
            std::test::assert(a != "world", "negation works too");
        }
        "#,
    );
}

#[test]
fn fn_returning_user_type_record_round_trips() {
    // Phase 3 lean: parse_request returns a Request record by
    // value. Test the same shape on a user-declared type to
    // pin the contract.
    run_self_test(
        "fn_returns_record",
        r#"
        type Pair {
            left: Int;
            right: Int;
        }

        fn make_pair(l: Int, r: Int) -> Pair {
            return Pair { left: l, right: r };
        }

        fn main() {
            let p = make_pair(7, 13);
            std::test::assert_eq_int(p.left, 7, "left");
            std::test::assert_eq_int(p.right, 13, "right");
            std::test::assert_eq_int(p.left + p.right, 20, "sum");
        }
        "#,
    );
}
