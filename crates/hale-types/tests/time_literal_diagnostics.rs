//! GH #607 — what the checker says about `Time`: a malformed literal
//! is the author's error, and the arithmetic an instant admits is
//! exactly a shift by a `Duration` and a difference of two instants.

use hale_syntax::parse_source;

fn errors(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program).into_iter().filter(|d| d.is_error()).map(|d| d.message).collect()
}

#[test]
fn a_malformed_time_literal_is_refused_at_check_time() {
    let e = errors("fn main() { let t = `2026-05-08T12:00:00+01:00`; }");
    assert!(e.iter().any(|m| m.contains("not an ISO-8601 UTC instant")), "{e:?}");
    let e = errors("fn main() { let t = `not a time`; }");
    assert!(e.iter().any(|m| m.contains("not an ISO-8601 UTC instant")), "{e:?}");
    assert!(errors("fn main() { let t = `2026-05-08T12:00:00.5Z`; let u = `1969-12-31T23:59:59`; }").is_empty());
}

#[test]
fn time_arithmetic_is_a_shift_or_a_difference() {
    let ok = "fn main() { let t = `2026-05-08T12:00:00Z`; let d = (t + 5s) - t; let e = d + 1s; let u = 5s + t; let v = u - 1h; let b = t < u && t == t; }";
    assert!(errors(ok).is_empty(), "{:?}", errors(ok));
    for bad in ["let x = t * 2;", "let y = t + t;", "let z = t / 2;", "let w = 5s - t;"] {
        let e = errors(&format!("fn main() {{ let t = `2026-05-08T12:00:00Z`; {bad} }}"));
        assert!(e.iter().any(|m| m.contains("`Time` arithmetic")), "{bad}: {e:?}");
    }
}
