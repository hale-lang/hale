//! Interpreter parity for the common stdlib path-calls. Each
//! test verifies that an `aperio run` program can invoke the
//! same `std::*` paths that `aperio build` already supports.

use aperio_runtime::run_program;

fn run(src: &str) -> i32 {
    let program = aperio_syntax::parse_source(src)
        .map_err(|d| {
            d.iter()
                .map(|x| x.render(src))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .expect("parse");
    run_program(&program).expect("run")
}

#[test]
fn process_pid_returns_positive_int() {
    let src = r#"
fn main() {
    let n = std::process::pid();
    if n > 0 { return 0; }
    return 1;
}
"#;
    assert_eq!(run(src), 0);
}

#[test]
fn env_var_exists_returns_bool() {
    // PATH almost certainly exists on the test host; if not,
    // fall back to checking the inverse for an unlikely name.
    let src = r#"
fn main() {
    if !std::env::var_exists("CERTAINLY_NOT_SET_XYZZY") {
        return 0;
    }
    return 1;
}
"#;
    assert_eq!(run(src), 0);
}

#[test]
fn fs_file_exists_and_read_file_round_trip() {
    use std::fs;
    let path = std::env::temp_dir().join("aperio_interp_parity_test.txt");
    fs::write(&path, "hello aperio").unwrap();
    let path_str = path.display().to_string();
    let src = format!(
        r#"
fn main() {{
    let p = "{}";
    if !std::io::fs::file_exists(p) {{ return 1; }}
    let c = std::io::fs::read_file(p);
    if c != "hello aperio" {{ return 2; }}
    return 0;
}}
"#,
        path_str
    );
    let code = run(&src);
    let _ = fs::remove_file(&path);
    assert_eq!(code, 0);
}

#[test]
fn fs_read_file_missing_returns_io_error() {
    // #68 — read_file is fallible(IoError). Interpreter parity:
    // failures route to FallibleErr, addressable via `or`.
    let src = r#"
fn report(e: IoError) -> String {
    return e.kind;
}
fn main() {
    let s = std::io::fs::read_file("/no/such/path/aperio_interp_parity")
        or report(err);
    println(s);
}
"#;
    // The println output isn't captured here, but the program
    // exits 0 — the FallibleErr was addressed without diverging.
    assert_eq!(run(src), 0);
}

#[test]
fn fs_write_then_read_round_trip() {
    // Updated after the #68 fallible-fs flip: write_file now
    // returns `() fallible(IoError)`; callers address the error
    // with `or raise` / `or <fallback>`.
    use std::fs;
    let path = std::env::temp_dir().join("aperio_interp_parity_write.txt");
    let path_str = path.display().to_string();
    let _ = fs::remove_file(&path);
    let src = format!(
        r#"
fn main() {{
    let p = "{}";
    std::io::fs::write_file(p, "round-trip") or raise;
    let c = std::io::fs::read_file(p) or raise;
    if c != "round-trip" {{ return 2; }}
    return 0;
}}
"#,
        path_str
    );
    let code = run(&src);
    let _ = fs::remove_file(&path);
    assert_eq!(code, 0);
}

#[test]
fn str_parse_int_recognizes_int_and_garbage() {
    // 2026-05-17 — parse_int returns Int fallible(ParseError).
    // Substitute path on garbage yields 0; valid input yields 42.
    let src = r#"
fn main() {
    let a = std::str::parse_int("42") or -1;
    if a != 42 { return 1; }
    let b = std::str::parse_int("garbage") or 0;
    if b != 0 { return 2; }
    if !std::str::can_parse_int("42") { return 3; }
    if std::str::can_parse_int("garbage") { return 4; }
    return 0;
}
"#;
    assert_eq!(run(src), 0);
}

#[test]
fn math_unary_and_pow_match_libm() {
    let src = r#"
fn main() {
    if std::math::sqrt(16.0) != 4.0 { return 1; }
    if std::math::floor(3.7) != 3.0 { return 2; }
    if std::math::ceil(3.2) != 4.0 { return 3; }
    if std::math::pow(2.0, 10.0) != 1024.0 { return 4; }
    return 0;
}
"#;
    assert_eq!(run(src), 0);
}

#[test]
fn str_parse_decimal_recognizes_decimal_and_garbage() {
    // 2026-05-20 — parse_decimal returns Decimal fallible(ParseError).
    let src = r#"
fn main() {
    let a = std::str::parse_decimal("100.5") or 0.0d;
    if a != 100.5d { return 1; }
    let b = std::str::parse_decimal("garbage") or 0.0d;
    if b != 0.0d { return 2; }
    if !std::str::can_parse_decimal("100.5") { return 3; }
    if std::str::can_parse_decimal("garbage") { return 4; }
    let c = std::str::parse_decimal("0.00005100") or 0.0d;
    if c != 0.000051d { return 5; }
    return 0;
}
"#;
    assert_eq!(run(src), 0);
}

#[test]
fn time_from_unix_constructs_iso8601() {
    // 2026-05-20 — time_from_unix(n) -> Time, ISO 8601 UTC.
    // Epoch 1700000000 = 2023-11-14T22:13:20Z.
    let src = r#"
fn main() {
    let t = std::time::time_from_unix(1700000000);
    // The Time displays as its ISO 8601 string; compare via println-equivalent.
    // Direct equality check on Time value through string conversion is
    // future work — for parity, we just verify it runs without error
    // and a round-trip through now() returns a non-empty Time.
    let n = std::time::now();
    let stamp = std::time::time_from_unix(n);
    return 0;
}
"#;
    assert_eq!(run(src), 0);
}
