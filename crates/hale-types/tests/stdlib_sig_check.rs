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

// ── Tranche 2 (io namespaces) + dual-mode semantics ──

#[test]
fn tranche2_io_fs_checks_fire() {
    let m = msgs(
        r#"
        fn main() {
            let sz = std::io::fs::file_size(42) or 0;
            let r = std::io::fs::read_file("/x") or 0;
            std::io::fs::mkdir("/tmp/x", "extra") or raise;
            println(sz, r);
        }
    "#,
    );
    assert!(
        m.iter().any(|s| s.contains("file_size")
            && s.contains("expected `String`, got `Int`")),
        "got: {:?}",
        m
    );
    assert!(
        m.iter().any(|s| s.contains("does not match success type")
            && s.contains("String")),
        "got: {:?}",
        m
    );
    assert!(
        m.iter()
            .any(|s| s.contains("mkdir") && s.contains("takes 1")),
        "got: {:?}",
        m
    );
}

#[test]
fn bare_fallible_calls_stay_legal_dual_mode() {
    // Stdlib fallible path-calls are dual-mode at codegen: the bare
    // (no `or`) legacy form returns a direct value (read_file → the
    // String, write_file → an Int status). Bare calls must not be
    // flagged, and their returns stay permissive.
    let m = msgs(
        r#"
        fn main() {
            let payload = std::io::fs::read_file("/etc/hostname");
            let r: Int = std::io::fs::write_file("/tmp/x", payload);
            println(r);
        }
    "#,
    );
    let errs: Vec<&String> = m
        .iter()
        .filter(|s| {
            s.contains("error not addressed")
                || s.contains("argument")
                || s.contains("expected")
        })
        .collect();
    assert!(errs.is_empty(), "got: {:?}", errs);
}

#[test]
fn statement_position_or_discards_value_type() {
    // `call() or handler(err);` in statement position discards the
    // value — a Bool-returning handler over a Unit-success call is
    // fine (pond / downstream apps production pattern).
    let m = msgs(
        r#"
        fn boolish(e: Int) -> Bool {
            return e > 0;
        }
        fn main() {
            std::io::fs::write_file("/tmp/x", "y") or boolish(1);
        }
    "#,
    );
    let errs: Vec<&String> = m
        .iter()
        .filter(|s| s.contains("does not match"))
        .collect();
    assert!(errs.is_empty(), "got: {:?}", errs);
}

#[test]
fn value_position_or_still_checks_fallback() {
    // Same shapes in VALUE position still check.
    let m = msgs(
        r#"
        fn main() {
            let x = std::io::fs::file_size("/x") or "zero";
            println(x);
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
