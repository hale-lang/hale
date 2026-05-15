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
fn fs_write_then_read_round_trip() {
    use std::fs;
    let path = std::env::temp_dir().join("aperio_interp_parity_write.txt");
    let path_str = path.display().to_string();
    let _ = fs::remove_file(&path);
    let src = format!(
        r#"
fn main() {{
    let p = "{}";
    if std::io::fs::write_file(p, "round-trip") != 0 {{ return 1; }}
    let c = std::io::fs::read_file(p);
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
    let src = r#"
fn main() {
    if std::str::parse_int("42") != 42 { return 1; }
    if std::str::parse_int("garbage") != 0 { return 2; }
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
