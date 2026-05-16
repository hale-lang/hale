//! Interpreter parity for the 2026-05-16 library-shape additions.
//! Programs use std::process::exit(1) to fail loudly so a passing
//! run (exit 0) actually demonstrates the assertion.

use aperio_runtime::run_program;

fn run(src: &str) -> i32 {
    let program = aperio_syntax::parse_source(src)
        .map_err(|d| d.iter().map(|x| x.render(src)).collect::<Vec<_>>().join("\n"))
        .expect("parse");
    run_program(&program).expect("run")
}

#[test]
fn bump_init_then_increment_interp() {
    let src = r#"
        type WC { word: String; count: Int; }
        @form(hashmap)
        locus CM { capacity { pool entries of WC indexed_by word; } }
        fn main() {
            let m = CM { };
            m.bump("the");
            m.bump("the");
            m.bump("the");
            let e = m.get("the") or raise;
            if e.count != 3 { std::process::exit(1); }
            if m.len() != 1 { std::process::exit(1); }
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn key_at_entry_at_iterate_interp() {
    let src = r#"
        type E { k: String; v: Int; }
        @form(hashmap)
        locus M { capacity { pool entries of E indexed_by k; } }
        fn main() {
            let m = M { };
            m.bump("a");
            m.bump("b");
            m.bump("b");
            if m.len() != 2 { std::process::exit(1); }
            let mut i = 0;
            let mut total = 0;
            while i < m.len() {
                let e = m.entry_at(i) or raise;
                total = total + e.v;
                i = i + 1;
            }
            if total != 3 { std::process::exit(1); }
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn text_predicates_interp() {
    let src = r#"
        fn main() {
            if !std::text::is_alpha(65) { std::process::exit(1); }
            if std::text::is_alpha(48) { std::process::exit(1); }
            if !std::text::is_digit(48) { std::process::exit(1); }
            if !std::text::is_word_char(95) { std::process::exit(1); }
            if !std::text::is_word_char(39) { std::process::exit(1); }
            if !std::text::is_whitespace(32) { std::process::exit(1); }
            if std::text::is_whitespace(65) { std::process::exit(1); }
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn tokenize_words_into_interp() {
    let src = r#"
        @form(vec) locus WV { capacity { heap items of String; } }
        fn main() {
            let words = WV { };
            std::text::tokenize_words_into("Hi there, friend.", words);
            if words.len() != 3 { std::process::exit(1); }
            let w0 = words.get(0) or "";
            if w0 != "hi" { std::process::exit(1); }
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn or_discard_interp() {
    let src = r#"
        fn main() {
            std::io::fs::mkdir("/tmp/aperio_interp_discard_t") or discard;
            std::io::fs::mkdir("/tmp/aperio_interp_discard_t") or discard;
            println("ok");
        }
    "#;
    assert_eq!(run(src), 0);
    let _ = std::fs::remove_dir_all("/tmp/aperio_interp_discard_t");
}

#[test]
fn env_arg_or_default_interp() {
    let src = r#"
        fn main() {
            let v = std::env::arg_or(99, "fallback");
            if v != "fallback" { std::process::exit(1); }
        }
    "#;
    assert_eq!(run(src), 0);
}
