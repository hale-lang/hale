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
    // A bare (no `or`) stdlib fallible call — once a legacy form that
    // returned a direct value (read_file → the String, write_file → an
    // Int status), gone from lowering since F.40 phase 4, S5. The checker
    // types the bare call permissively, so its uses report nothing; the
    // call itself is the `bare_fallible` law's error (GH #738), one per
    // call.
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

/// F.40 phase 4, S5 (a classified correction): `ecdsa_p256_sign` has one
/// mode. Its bare call answered an empty `Bytes` on a bad key and passed
/// the check; its row now says it can fail, so the bare call is the law's
/// error at the call, and an `or` checks against the `Bytes` it succeeds
/// with.
#[test]
fn ecdsa_p256_sign_is_fallible_and_its_bare_call_is_refused() {
    let call = "std::crypto::ecdsa_p256_sign(k, k)";
    let src = format!("fn main() {{\n    let k = std::bytes::from_string(\"key\");\n    let s = {call};\n    println(len(s));\n}}\n");
    let prog = parse_source(&src).expect("parse");
    let errors: Vec<_> = check_program(&prog).into_iter().filter(|d| d.is_error()).collect();
    assert_eq!(errors.len(), 1, "{:?}", errors.iter().map(|d| &d.message).collect::<Vec<_>>());
    assert!(
        errors[0].message.starts_with(
            "`std::crypto::ecdsa_p256_sign` can fail (CryptoError) and this call says nothing about it"
        ),
        "{}",
        errors[0].message
    );
    let at = src.find(call).unwrap() as u32;
    assert_eq!((errors[0].span.start.0, errors[0].span.end.0), (at, at + call.len() as u32));

    let substitute = msgs(
        "fn main() {\n    let k = std::bytes::from_string(\"key\");\n    let s = std::crypto::ecdsa_p256_sign(k, k) or b\"\";\n    println(len(s));\n}\n",
    );
    assert!(substitute.is_empty(), "got: {substitute:?}");
    let wrong = msgs(
        "fn main() {\n    let k = std::bytes::from_string(\"key\");\n    let s = std::crypto::ecdsa_p256_sign(k, k) or 0;\n    println(s);\n}\n",
    );
    assert!(wrong.iter().any(|m| m.contains("does not match success type") && m.contains("Bytes")), "got: {wrong:?}");
}

/// F.40 phase 4, S5 (a classified correction): `std::io::file::close`
/// does not exist. Its row was a signature and nothing else (no surface
/// entry, no lowering), so a call was an unknown function whose arity the
/// signature still checked; the row is gone, and a call is the unknown
/// function alone.
#[test]
fn io_file_close_is_an_unknown_function_and_nothing_else() {
    let m = msgs("fn main() {\n    let r = std::io::file::close(1, 2);\n    println(r);\n}\n");
    assert_eq!(
        m,
        vec![
            "unknown stdlib function `std::io::file::close` — did you mean `std::io::file::__close`?"
                .to_string()
        ]
    );
    assert!(hale_types::stdlib_surface::row(&["std", "io", "file", "close"]).is_none());
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

// F.40 phase 4, S6 (a classified correction): every dispatched stdlib
// function has a signature, the one its lowering helper enforced. Before,
// the rows below had none, so the check typed their calls `Unknown` and
// let any arguments through: a wrong count or type failed at build, in the
// helper's own words and without a location, and an `or` over one of them
// was refused by lowering. Each module's test pins, for one function, that
// the check now refuses a wrong argument count at the callee, a wrong
// argument type at the argument, and an `or` over a function that cannot
// fail at the call, in the check's own words. The base compiler checked
// every one of these programs clean (`hale check`, exit 0).

/// The error diagnostics of checking `body` as `main`'s body, each with
/// the text its span covers.
fn located_errors(body: &str) -> Vec<(String, String)> {
    let src = format!("fn main() {{\n{body}\n}}\n");
    let prog = parse_source(&src).expect("parse");
    check_program(&prog)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| (d.message, src[d.span.start.0 as usize..d.span.end.0 as usize].to_string()))
        .collect()
}

/// `body` checks with exactly the errors `want`, each `(message, the text
/// its span covers)`.
fn refused(body: &str, want: &[(&str, &str)]) {
    let got = located_errors(body);
    let want: Vec<(String, String)> = want.iter().map(|(m, at)| (m.to_string(), at.to_string())).collect();
    assert_eq!(got, want, "checking:\n{body}");
}

#[test]
fn std_io_sockopt_calls_are_checked() {
    refused(
        "    let n = std::io::sockopt::SO_RCVBUF(4);\n    println(n);",
        &[("`std::io::sockopt::SO_RCVBUF` takes 0 arguments, got 1", "std::io::sockopt::SO_RCVBUF")],
    );
    refused(
        "    let n = std::io::sockopt::SO_RCVBUF() or 0;\n    println(n);",
        &[(
            "`std::io::sockopt::SO_RCVBUF` is not fallible (it returns `Int`); drop the `or` clause",
            "std::io::sockopt::SO_RCVBUF()",
        )],
    );
    // The value is the Int it always was.
    refused("    let n: Int = std::io::sockopt::SO_RCVBUF();\n    println(n);", &[]);
}

#[test]
fn std_io_mirror_calls_are_checked() {
    refused(
        "    let h = std::io::mirror::__new(4096, 1);\n    println(h);",
        &[("`std::io::mirror::__new` takes 1 argument, got 2", "std::io::mirror::__new")],
    );
    refused(
        "    let n = std::io::mirror::__recv_into(0, \"fd\", 64);\n    println(n);",
        &[("`std::io::mirror::__recv_into` argument 2: expected `Int`, got `String`", "\"fd\"")],
    );
    refused(
        "    let h = std::io::mirror::__new(4096) or 0;\n    println(h);",
        &[(
            "`std::io::mirror::__new` is not fallible (it returns `Int`); drop the `or` clause",
            "std::io::mirror::__new(4096)",
        )],
    );
}

#[test]
fn std_bytes_calls_are_checked() {
    refused(
        "    let h = std::bytes::builder::__new();\n    println(h);",
        &[("`std::bytes::builder::__new` takes 1 argument, got 0", "std::bytes::builder::__new")],
    );
    refused(
        "    let h = std::bytes::builder::__new(64);\n    let ok = std::bytes::builder::__append_str(h, 7);\n    println(ok);",
        &[("`std::bytes::builder::__append_str` argument 2: expected `String`, got `Int`", "7")],
    );
    refused(
        "    let f = std::bytes::__is_alloc_fail(\"x\");\n    println(f);",
        &[("`std::bytes::__is_alloc_fail` argument 1: expected `Bytes`, got `String`", "\"x\"")],
    );
    refused(
        "    let n = std::bytes::builder::__len(0) or 0;\n    println(n);",
        &[(
            "`std::bytes::builder::__len` is not fallible (it returns `Int`); drop the `or` clause",
            "std::bytes::builder::__len(0)",
        )],
    );
}

#[test]
fn std_ring_calls_are_checked() {
    refused(
        "    std::ring::__spsc_note_drop(1, 2);",
        &[("`std::ring::__spsc_note_drop` takes 1 argument, got 2", "std::ring::__spsc_note_drop")],
    );
    refused(
        "    std::ring::__spsc_set_tag_b(1, true);",
        &[("`std::ring::__spsc_set_tag_b` argument 2: expected `Int`, got `Bool`", "true")],
    );
    refused(
        "    let n = std::ring::__spsc_read(1, 2, 3, 4, 5, 6, 7) or 0;\n    println(n);",
        &[(
            "`std::ring::__spsc_read` is not fallible (it returns `Int`); drop the `or` clause",
            "std::ring::__spsc_read(1, 2, 3, 4, 5, 6, 7)",
        )],
    );
}

#[test]
fn std_bus_calls_are_checked() {
    refused(
        "    std::bus::__local_dispatch(\"subject\");",
        &[("`std::bus::__local_dispatch` takes 2 arguments, got 1", "std::bus::__local_dispatch")],
    );
    refused(
        "    std::bus::__local_dispatch(7, std::bytes::from_string(\"x\"));",
        &[("`std::bus::__local_dispatch` argument 1: expected `String`, got `Int`", "7")],
    );
    refused(
        "    let r = std::bus::__local_dispatch(\"s\", std::bytes::from_string(\"x\")) or 0;\n    println(r);",
        &[(
            "`std::bus::__local_dispatch` is not fallible (it returns `Int`); drop the `or` clause",
            "std::bus::__local_dispatch(\"s\", std::bytes::from_string(\"x\"))",
        )],
    );
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
