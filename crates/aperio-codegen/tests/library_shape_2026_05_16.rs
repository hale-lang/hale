//! Tests for the 2026-05-16 library-shape additions surfaced by
//! the wordfreq-corpus library-dev handoff. Each test maps to
//! one reinvention pattern the agents were paying for:
//!
//! - `@form(hashmap).key_at` / `.entry_at` — iterate without
//!   a parallel keys vec.
//! - `@form(hashmap).bump(k)` — collapse the has/get/set
//!   increment-or-init dance.
//! - `std::text::is_*` byte predicates + tokenize_words_into —
//!   replace the hand-rolled byte walk every wordfreq program
//!   reinvented.
//! - `or discard` — sugar for swallowing a Unit-success error.
//! - `std::env::arg_or` — collapse the args-with-default
//!   3-line ceremony to a 1-line call.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, src: &str, argv: &[&str]) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_libshape_{}_{}", name, std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).args(argv).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn hashmap_bump_init_then_increment() {
    let src = r#"
        type WC { word: String; count: Int; }

        @form(hashmap)
        locus CountMap {
            capacity { pool entries of WC indexed_by word; }
        }

        fn main() {
            let m = CountMap { };
            m.bump("the");
            m.bump("quick");
            m.bump("the");
            m.bump("brown");
            m.bump("the");
            let e = m.get("the") or raise;
            println("the=", e.count);
            let q = m.get("quick") or raise;
            println("quick=", q.count);
            println("len=", m.len());
        }
    "#;
    let (stdout, status) = build_and_run("bump_inc", src, &[]);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("the=3"), "got: {:?}", stdout);
    assert!(stdout.contains("quick=1"), "got: {:?}", stdout);
    assert!(stdout.contains("len=3"), "got: {:?}", stdout);
}

#[test]
fn hashmap_bump_rejects_three_field_cells() {
    let src = r#"
        type Triple { k: String; a: Int; b: Int; }
        @form(hashmap)
        locus M { capacity { pool entries of Triple indexed_by k; } }
        fn main() {
            let m = M { };
            m.bump("x");
        }
    "#;
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_libshape_bump_bad_{}", std::process::id()));
    let err = aperio_codegen::build_executable(&program, &bin).expect_err("should reject");
    let _ = std::fs::remove_file(&bin);
    let msg = format!("{:?}", err);
    assert!(msg.contains("exactly two fields"), "got: {}", msg);
    assert!(msg.contains("has/get/set"), "got: {}", msg);
}

#[test]
fn hashmap_key_at_and_entry_at_round_trip() {
    let src = r#"
        type WC { word: String; count: Int; }
        @form(hashmap)
        locus CM { capacity { pool entries of WC indexed_by word; } }

        fn main() {
            let m = CM { };
            m.bump("alpha");
            m.bump("beta");
            m.bump("alpha");
            let n = m.len();
            println("n=", n);
            let mut i = 0;
            let mut total = 0;
            while i < n {
                let k = m.key_at(i) or raise;
                let e = m.entry_at(i) or raise;
                total = total + e.count;
                println(k, "=", e.count);
                i = i + 1;
            }
            println("total=", total);
        }
    "#;
    let (stdout, status) = build_and_run("hm_iter", src, &[]);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("n=2"), "got: {:?}", stdout);
    assert!(stdout.contains("total=3"), "got: {:?}", stdout);
}

#[test]
fn hashmap_key_at_out_of_bounds_returns_index_error() {
    let src = r#"
        type E { k: String; v: Int; }
        @form(hashmap)
        locus M { capacity { pool entries of E indexed_by k; } }

        fn handle(e: IndexError) -> String { return "oob"; }

        fn main() {
            let m = M { };
            m.set(E { k: "hi", v: 1 });
            let k = m.key_at(5) or handle(err);
            println("k=", k);
        }
    "#;
    let (stdout, status) = build_and_run("hm_oob", src, &[]);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("k=oob"), "got: {:?}", stdout);
}

#[test]
fn text_predicates_classify_bytes() {
    let src = r#"
        fn main() {
            let s = std::bytes::from_string("Hi 9!");
            let mut i = 0;
            while i < 5 {
                let c = std::bytes::at(s, i) or 0;
                if std::text::is_alpha(c) { print("A"); }
                else if std::text::is_digit(c) { print("D"); }
                else if std::text::is_whitespace(c) { print("S"); }
                else { print("?"); }
                i = i + 1;
            }
            println("");
        }
    "#;
    let (stdout, status) = build_and_run("text_preds", src, &[]);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("AASD?"), "got: {:?}", stdout);
}

#[test]
fn text_tokenize_words_into_round_trip() {
    let src = r#"
        @form(vec)
        locus WV { capacity { heap items of String; } }
        fn main() {
            let words = WV { };
            std::text::tokenize_words_into("Hello, world! It's 2026.", words);
            println("count=", words.len());
            let mut i = 0;
            while i < words.len() {
                let w = words.get(i) or "";
                println(w);
                i = i + 1;
            }
        }
    "#;
    let (stdout, status) = build_and_run("tokenize", src, &[]);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("count=4"), "got: {:?}", stdout);
    assert!(stdout.contains("hello"), "got: {:?}", stdout);
    assert!(stdout.contains("world"), "got: {:?}", stdout);
    assert!(stdout.contains("it's"), "got: {:?}", stdout);
    assert!(stdout.contains("2026"), "got: {:?}", stdout);
}

#[test]
fn or_discard_swallows_unit_error() {
    let src = r#"
        fn main() {
            std::io::fs::write_file("/tmp/aperio_test_discard.txt", "ok") or discard;
            std::io::fs::mkdir("/tmp/aperio_test_discard_dir") or discard;
            println("ok");
        }
    "#;
    let (stdout, status) = build_and_run("discard_unit", src, &[]);
    let _ = std::fs::remove_file("/tmp/aperio_test_discard.txt");
    let _ = std::fs::remove_dir_all("/tmp/aperio_test_discard_dir");
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("ok"), "got: {:?}", stdout);
}

#[test]
fn or_discard_rejects_value_bearing_call() {
    let src = r#"
        fn main() {
            let s = std::io::fs::read_file("/no/such/path") or discard;
            println(s);
        }
    "#;
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_discard_bad_{}", std::process::id()));
    let err = aperio_codegen::build_executable(&program, &bin).expect_err("should reject");
    let _ = std::fs::remove_file(&bin);
    let msg = format!("{:?}", err);
    assert!(msg.contains("discard"), "got: {}", msg);
    assert!(msg.contains("Unit"), "got: {}", msg);
}

#[test]
fn env_arg_or_present_returns_arg() {
    let src = r#"
        fn main() {
            let path = std::env::arg_or(1, "/default");
            println("path=", path);
        }
    "#;
    let (stdout, status) = build_and_run("arg_or_present", src, &["/some/path"]);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("path=/some/path"), "got: {:?}", stdout);
}

#[test]
fn env_arg_or_absent_returns_default() {
    let src = r#"
        fn main() {
            let port = std::env::arg_or(1, "8080");
            println("port=", port);
        }
    "#;
    let (stdout, status) = build_and_run("arg_or_absent", src, &[]);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("port=8080"), "got: {:?}", stdout);
}
