//! A Float's text is the shortest that reads back as the identical double
//! (`lotus_fmt_float_shortest`: Burger & Dybvig's free-format algorithm over
//! a small bignum, ties to the even digit). The formatter is the runtime's
//! own, independent of libc's printf precision, so it is checked here
//! against Rust's shortest round-trip formatting of f64, over the edges and
//! a seeded sample of bit patterns: every text must read back as its double,
//! and must be exactly Rust's digits laid out as `%.17g` lays them out
//! (fixed point for a decimal exponent from -4 to 16, else `d.ddde±XX`).

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// The text a Float must have: Rust's shortest digits, laid out as the
/// runtime lays them.
fn expected(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x < 0.0 { "-inf".into() } else { "inf".into() };
    }
    let sign = if x.is_sign_negative() { "-" } else { "" };
    if x == 0.0 {
        return format!("{sign}0");
    }
    let (digits, x10) = shortest_digits(x.abs());
    let n = digits.len() as i32;
    let body = if (-4..17).contains(&x10) {
        if x10 < 0 {
            format!("0.{}{}", "0".repeat((-x10 - 1) as usize), digits)
        } else {
            let mut s: String = (0..=x10).map(|i| digits.as_bytes().get(i as usize).map_or('0', |b| *b as char)).collect();
            if n > x10 + 1 {
                s.push('.');
                s.push_str(&digits[(x10 + 1) as usize..]);
            }
            s
        }
    } else {
        let frac = if n > 1 { format!(".{}", &digits[1..]) } else { String::new() };
        format!("{}{frac}e{}{:02}", &digits[..1], if x10 < 0 { '-' } else { '+' }, x10.abs())
    };
    format!("{sign}{body}")
}

/// Rust's shortest round-trip digits of `x` and the decimal exponent of
/// the first, but a tie (the double exactly halfway between the two
/// shortest candidates) to the even digit, as the shortest-repr reference
/// (David Gay's, Python's `repr`) breaks it; Rust's formatter rounds a
/// tie up.
fn shortest_digits(x: f64) -> (String, i32) {
    let split = |e: &str| {
        let (mant, exp) = e.split_once('e').expect("an exponent");
        (mant.chars().filter(|c| *c != '.').collect::<String>(), exp.parse::<i32>().expect("an exponent"))
    };
    let (digits, x10) = split(&format!("{:e}", x));
    let n = digits.len();
    // the exact value, to n + 20 significant digits: a tie is the n
    // digits below, then a 5, then zeros
    let (exact, ex10) = split(&format!("{:.*e}", n + 19, x));
    let tie = ex10 == x10 && exact.as_bytes()[n] == b'5' && exact[n + 1..].bytes().all(|b| b == b'0');
    if !tie || n < 2 {
        return (digits, x10);
    }
    let mut d: Vec<u8> = exact[..n].bytes().collect();
    if (d[n - 1] - b'0') % 2 == 1 {
        let mut i = n;
        loop {
            i -= 1;
            if d[i] == b'9' {
                d[i] = b'0';
                if i == 0 {
                    d.insert(0, b'1');
                    let s = String::from_utf8(d).unwrap();
                    return (s.trim_end_matches('0').to_string(), x10 + 1);
                }
            } else {
                d[i] += 1;
                break;
            }
        }
    }
    let s = String::from_utf8(d).unwrap();
    (s.trim_end_matches('0').to_string(), x10)
}

/// The edges, and a seeded sample of 300k: random bit patterns,
/// decimal-looking values, and exact ties at the 17th digit.
fn values() -> Vec<f64> {
    let mut v = vec![
        0.1 + 0.2, 99.999995, 100.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 1e-300, 1e300, 0.0, -0.0, 100.0, 1234567.0, 1e16, 1e17, 1e21,
        1e22, 1e23, 1e-4, 1e-5, 0.1, 0.15, 4.35, 2.5, -2.5, 9007199254740993.0, 123456789012345678.0, f64::MAX, f64::MIN_POSITIVE,
        f64::MIN_POSITIVE / 2.0, 5e-324, 2.2250738585072009e-308, 1.7976931348623157e308, f64::EPSILON, 1.0 + f64::EPSILON,
        1059438285926254.25, 26363981746409.3125, 0.30000000000000004, 1e-7, 123e-20,
    ];
    for p in -323..=308 {
        v.push(format!("1e{p}").parse().unwrap());
        v.push(format!("9.999999999999999e{p}").parse().unwrap());
    }
    // a seeded xorshift64*: the same sample every run
    let mut s: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = || {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        s.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    for _ in 0..200_000 {
        let x = f64::from_bits(next());
        if !x.is_nan() {
            v.push(x);
        }
    }
    for _ in 0..70_000 {
        let r = next();
        v.push((r % 2_000_000_000) as f64 / 10f64.powi((r >> 40) as i32 % 9) - 1e6);
    }
    for _ in 0..30_000 {
        // an integer in [2^49, 2^50) plus a quarter: its 17th digit is a tie
        let n = (1u64 << 49) + next() % (1u64 << 49);
        v.push(n as f64 + if next() & 1 == 0 { 0.25 } else { 0.75 });
    }
    v
}

fn run(src: &str, tag: &str, input: &[f64]) -> Vec<String> {
    let bin = harness::unique_bin(tag);
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let data = bin.with_extension("f64");
    let bytes: Vec<u8> = input.iter().flat_map(|x| x.to_le_bytes()).collect();
    std::fs::write(&data, bytes).expect("write the sample");
    let out = Command::new(&bin).arg(&data).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&data);
    assert!(out.status.success(), "{:?}: {}", out.status, String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect()
}

const PRINT: &str = r#"
fn main() {
    let b = std::io::fs::read_bytes(std::env::arg(1)) or std::bytes::from_string("");
    let n = len(b) / 8;
    let mut i = 0;
    while i < n {
        let x = std::bytes::read_f64_le(b, i * 8) or 0.0;
        println(std::str::__float_text(x));
        i = i + 1;
    }
}
"#;

fn check(texts: &[String], input: &[f64], what: &str) {
    assert_eq!(texts.len(), input.len(), "{what}: one line a value");
    let mut bad = Vec::new();
    for (t, x) in texts.iter().zip(input) {
        let back: f64 = t.parse().unwrap_or(f64::NAN);
        let same = back.to_bits() == x.to_bits() || (x.is_nan() && back.is_nan());
        if !same || *t != expected(*x) {
            bad.push(format!("{:#018x}: got {t}, want {} (reads back: {same})", x.to_bits(), expected(*x)));
        }
    }
    assert!(bad.is_empty(), "{what}: {} of {} differ, first: {:?}", bad.len(), input.len(), &bad[..bad.len().min(10)]);
}

#[test]
fn a_float_text_reads_back_as_the_same_double() {
    let mut input = values();
    input.extend([f64::NAN, f64::INFINITY, f64::NEG_INFINITY]);
    let texts = run(PRINT, "float_text", &input);
    check(&texts, &input, "std::str::__float_text");
}

const PRINTED: &str = r#"
fn main() {
    let b = std::io::fs::read_bytes(std::env::arg(1)) or std::bytes::from_string("");
    let n = len(b) / 8;
    let mut i = 0;
    while i < n {
        let x = std::bytes::read_f64_le(b, i * 8) or 0.0;
        println(to_string(x));
        println(x);
        println(f"{x}");
        let back = std::str::parse_float(to_string(x)) or 0.0;
        println(std::str::__float_text(back));
        i = i + 1;
    }
}
"#;

/// `to_string(Float)`, `println` of a Float and its interpolation are the
/// same text, and `parse_float(to_string(x))` is `x` again.
#[test]
fn to_string_println_and_interpolation_read_back() {
    let mut input: Vec<f64> = values().into_iter().take(20_000).collect();
    input.extend([f64::INFINITY, f64::NEG_INFINITY]);
    let lines = run(PRINTED, "float_printed", &input);
    assert_eq!(lines.len(), input.len() * 4, "four lines a value");
    for (k, what) in ["to_string", "println", "interpolation", "parse_float(to_string(x))"].iter().enumerate() {
        let texts: Vec<String> = lines.iter().skip(k).step_by(4).cloned().collect();
        check(&texts, &input, what);
    }
}

const READ: &str = r#"
fn main() {
    let text = std::io::fs::read_file(std::env::arg(1)) or "";
    let n = len(text);
    let mut at = 0;
    while at < n {
        let mut end = std::str::index_of_from(text, "\n", at);
        if end < 0 {
            end = n;
        }
        let line = text[at..end];
        if std::str::can_parse_float(line) {
            let v = std::str::parse_float(line) or 0.0;
            println(std::str::__float_text(v));
        } else {
            println("refused");
        }
        at = end + 1;
    }
}
"#;

/// Decimal texts no formatter wrote: random digits (1 to 40 of them, so
/// past the 17 a double needs and past the fast path's 15), a random point
/// and exponent across the whole range and past it, the exact ties a
/// shorter text can't hit (2^53 + 1 and its kin), and the edges. Each with
/// what Rust's correctly rounded `parse::<f64>` makes of it.
fn decimal_texts() -> Vec<String> {
    let mut v: Vec<String> = [
        "5e-324", "4.9406564584124654e-324", "2.4703282292062328e-324", "2.4703282292062327e-324",
        "2.2250738585072014e-308", "2.2250738585072011e-308", "2.2250738585072009e-308", "1.7976931348623157e308",
        "1.7976931348623158e308", "1.7976931348623159e308", "1.5e-310", "9007199254740993", "9007199254740995",
        "18014398509481987", "1e23", "8.98846567431158e307", "0.1", "0.30000000000000004", "1e-400", "1e400", "0e-999",
        "0.0000", "000123.4500", ".5", "5.", "-2.5e-3", "+7", "1e-7", "123456789012345678901234567890e-30",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        s.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    for _ in 0..100_000 {
        let n = 1 + (next() % 40) as usize;
        let mut d: String = (0..n).map(|_| char::from(b'0' + (next() % 10) as u8)).collect();
        if next() % 3 == 0 {
            let at = (next() % (n as u64 + 1)) as usize;
            d.insert(at, '.');
            if d == "." {
                d = "0.5".into();
            }
        }
        let e = (next() % 700) as i64 - 360;
        v.push(format!("{d}e{e}"));
    }
    v
}

/// `parse_float` reads a decimal as the nearest double, ties to even, on
/// every target: the runtime's own exact reader, not libc's strtod (the
/// wasm shim's built 10^n by multiplication, so `5e-324` read as 0). A text
/// past the largest double, or a nonzero one that rounds to zero, is
/// refused.
#[test]
fn parse_float_reads_a_decimal_as_the_nearest_double() {
    let texts = decimal_texts();
    let bin = harness::unique_bin("float_read");
    build_opts::build_source(READ, &bin, &build_opts::options()).expect("build");
    let data = bin.with_extension("txt");
    std::fs::write(&data, texts.join("\n")).expect("write the texts");
    let out = Command::new(&bin).arg(&data).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&data);
    assert!(out.status.success(), "{:?}: {}", out.status, String::from_utf8_lossy(&out.stderr));
    let lines: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect();
    assert_eq!(lines.len(), texts.len(), "one line a text");
    let mut bad = Vec::new();
    for (t, got) in texts.iter().zip(&lines) {
        let r: f64 = t.parse().expect("Rust reads every text here");
        let nonzero = t.split(['e', 'E']).next().unwrap().chars().any(|c| c.is_ascii_digit() && c != '0');
        let want = if r.is_infinite() || (r == 0.0 && nonzero) { "refused".to_string() } else { expected(r) };
        if *got != want {
            bad.push(format!("{t}: got {got}, want {want}"));
        }
    }
    assert!(bad.is_empty(), "{} of {} differ, first: {:?}", bad.len(), texts.len(), &bad[..bad.len().min(10)]);
}
