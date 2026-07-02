//! Typecheck M3 stage 2 (2026-07-02): stdlib signature enforcement
//! — arity, arg types, and REAL return types (killing the Unknown
//! passthrough for tabled fns). Fallible rows return Ty::Fallible,
//! so `or` substitutes check against the true success type.

use hale_syntax::parse_source;
use hale_types::check_program;

fn msgs(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse");
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

#[test]
fn arg_type_mismatch_is_caught() {
    let m = msgs(
        r#"
        fn main() {
            let a = std::math::sqrt("four");
            println(a);
        }
    "#,
    );
    assert!(
        m.iter().any(|s| s.contains("std::math::sqrt")
            && s.contains("expected `Float`, got `String`")),
        "got: {:?}",
        m
    );
}

#[test]
fn arity_mismatch_is_caught() {
    let m = msgs(
        r#"
        fn main() {
            let b = std::math::pow(2.0);
            println(b);
        }
    "#,
    );
    assert!(
        m.iter().any(
            |s| s.contains("std::math::pow") && s.contains("takes 2")
        ),
        "got: {:?}",
        m
    );
}

#[test]
fn fallible_substitute_checked_against_success_type() {
    let m = msgs(
        r#"
        fn main() {
            let c = std::str::parse_int("42") or "";
            println(c);
        }
    "#,
    );
    assert!(
        m.iter().any(|s| s.contains("does not match success type")
            && s.contains("Int")),
        "got: {:?}",
        m
    );
}

#[test]
fn duration_param_rejects_int() {
    let m = msgs(
        r#"
        fn main() {
            std::time::sleep(100);
        }
    "#,
    );
    assert!(
        m.iter().any(|s| s.contains("std::time::sleep")
            && s.contains("expected `Duration`")),
        "got: {:?}",
        m
    );
}

#[test]
fn lowering_coercions_stay_legal() {
    // Int coerces to Float for math fns (sitofp in the lowering);
    // valid fallible use passes; StringView-free plain calls pass.
    let m = msgs(
        r#"
        fn main() {
            let a = std::math::sqrt(4);
            let b = std::str::parse_int("42") or 0;
            let c = std::env::var("HOME");
            let d = std::bytes::from_string("x");
            let e = std::crypto::crc32(d);
            println(a, b, c, e);
        }
    "#,
    );
    let sig_errs: Vec<&String> = m
        .iter()
        .filter(|s| {
            s.contains("argument")
                || s.contains("takes ")
                || s.contains("success type")
        })
        .collect();
    assert!(sig_errs.is_empty(), "got: {:?}", sig_errs);
}

#[test]
fn untabled_fns_keep_permissive_returns() {
    // io::fs is names-only (tranche 2): return stays Unknown, so
    // any `or` substitute type is accepted, as before.
    let m = msgs(
        r#"
        fn main() {
            let r = std::io::fs::read_file("/dev/null") or "";
            println(r);
        }
    "#,
    );
    let sig_errs: Vec<&String> = m
        .iter()
        .filter(|s| s.contains("argument") || s.contains("success type"))
        .collect();
    assert!(sig_errs.is_empty(), "got: {:?}", sig_errs);
}
