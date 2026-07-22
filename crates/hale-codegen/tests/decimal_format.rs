//! GH #230 item 2: std::decimal::format(d, places) — fixed
//! fraction places, round half-up; the explicit surface for
//! money-style display. Default printing still trims (declared
//! precision isn't stored in the scale-9 repr — the resolved
//! design call on the issue).

use std::process::Command;

use hale_codegen::build_executable;

#[test]
fn format_renders_fixed_places_with_half_up_rounding() {
    let src = r#"
        fn main() {
            let price = 12.50d;
            println(std::decimal::format(price, 2));
            println(std::decimal::format(price, 0));
            println(std::decimal::format(1.005d, 2));
            println(std::decimal::format(0.0d - 3.14159d, 3));
            println(price);
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("hale_test_decimal_format");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        vec!["12.50", "13", "1.01", "-3.142", "12.5"],
        "full stdout: {:?}",
        stdout
    );
}
