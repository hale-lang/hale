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
fn bare_fallible_calls_are_the_law_s_errors_and_type_permissively() {
    // Stdlib fallible path-calls are dual-mode at codegen: the bare
    // (no `or`) legacy form returns a direct value (read_file → the
    // String, write_file → an Int status). The checker types the bare
    // call permissively, so its uses report nothing; the call itself is
    // the `bare_fallible` law's error (GH #738), one per call.
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
        .filter(|s| s.contains("argument") || s.contains("expected"))
        .collect();
    assert!(errs.is_empty(), "got: {:?}", errs);
    let bare: Vec<&String> = m.iter().filter(|s| s.contains("says nothing about it")).collect();
    assert_eq!(bare.len(), 2, "got: {:?}", m);
    assert!(bare[0].starts_with("`std::io::fs::read_file` can fail (IoError)"), "got: {:?}", bare);
    assert!(bare[1].starts_with("`std::io::fs::write_file` can fail (IoError)"), "got: {:?}", bare);
}

/// F.40 phase 4, S5 (a classified correction): the functions lowering
/// lowered only under an `or` and whose rows said nothing about it — the
/// tcp and tls setters, the `File`, udp and process primitives — say
/// they can fail, so a bare call is the bare-fallible law's error, at the
/// call, from the check. Lowering refused most of them without a span and
/// answered "not implemented" for the rest. One per module.
#[test]
fn a_function_lowering_treats_as_fallible_is_refused_bare_by_the_check() {
    let calls = [
        ("std::io::file::__seek(3, 0)", "std::io::file::__seek"),
        ("std::io::tcp::set_recv_timeout(3, 5ms)", "std::io::tcp::set_recv_timeout"),
        ("std::io::tls::set_nodelay(3, true)", "std::io::tls::set_nodelay"),
        ("std::io::udp::__send(3, \"127.0.0.1\", 9, \"x\")", "std::io::udp::__send"),
        ("std::process::__kill_escalate(3)", "std::process::__kill_escalate"),
    ];
    for (call, path) in calls {
        let src = format!("fn main() {{\n    {call};\n}}\n");
        let prog = parse_source(&src).expect("parse");
        let errors: Vec<_> = check_program(&prog).into_iter().filter(|d| d.is_error()).collect();
        assert_eq!(errors.len(), 1, "{call}: {:?}", errors.iter().map(|d| &d.message).collect::<Vec<_>>());
        assert_eq!(
            errors[0].message,
            format!(
                "`{path}` can fail (IoError) and this call says nothing about it: write \
                 `or raise` to hand the failure to the caller, `or <fallback>` for a value \
                 to use instead, `or handler(err)` to deal with it here, or `or discard` \
                 when losing it is the intent. A bare call to a fallible entry point is an \
                 error since v0.22.0 (GH #738)."
            )
        );
        let at = src.find(call).expect("the call") as u32;
        assert_eq!(
            (errors[0].span.start.0, errors[0].span.end.0),
            (at, at + call.len() as u32),
            "{call}: the error is the call's"
        );
    }
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
