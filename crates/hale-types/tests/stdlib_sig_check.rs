//! Typecheck M3 stage 2 (2026-07-02): stdlib signature enforcement
//! — arity, arg types, and REAL return types (killing the Unknown
//! passthrough for tabled fns). Fallible rows return Ty::Fallible,
//! so `or` substitutes check against the true success type.

#[path = "support/entries.rs"]
mod entries;
use hale_syntax::parse_source;
use entries::check_program;

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

/// GH #1076 (U4): `Duration` is a quantity, so an `Int` where a stdlib
/// fn takes one is refused by the quantity rule, at the argument, with
/// the unit that makes a count one.
#[test]
fn duration_param_rejects_int() {
    let src = r#"
        fn main() {
            std::time::sleep(100);
        }
    "#;
    let prog = parse_source(src).expect("parse");
    let found: Vec<(String, String)> = check_program(&prog)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| (d.span.slice(src).to_string(), d.message))
        .collect();
    assert_eq!(
        found,
        [(
            "100".to_string(),
            "`Int` is not `Duration`: a count becomes a quantity by a unit (`n * 1ns`)".to_string()
        )]
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

/// The seven cursor primitives are signed with the count and types their
/// helper reads (F.40 phase 4, S6's ruling). The helper ignores arguments
/// past the ones it reads, so lowering built each extra-argument call
/// below; the check refuses it now. One per group the helper lowers
/// together.
#[test]
fn std_io_mirror_cursor_calls_are_checked() {
    refused(
        "    let h = std::io::mirror::__new(4096);\n    let c = std::io::mirror::__commit(h, 1, 2);\n    println(c);",
        &[("`std::io::mirror::__commit` takes 2 arguments, got 3", "std::io::mirror::__commit")],
    );
    refused(
        "    let h = std::io::mirror::__new(4096);\n    let f = std::io::mirror::__free(h, 0);\n    println(f);",
        &[("`std::io::mirror::__free` takes 1 argument, got 2", "std::io::mirror::__free")],
    );
    refused(
        "    let h = std::io::mirror::__new(4096);\n    let w = std::io::mirror::__writable(h, 64);\n    std::bytes::write_i8(w, 0, 1) or raise;",
        &[("`std::io::mirror::__writable` takes 1 argument, got 2", "std::io::mirror::__writable")],
    );
    refused(
        "    let h = std::io::mirror::__new(4096);\n    let n = std::io::mirror::__capacity(h, 1);\n    println(n);",
        &[("`std::io::mirror::__capacity` takes 1 argument, got 2", "std::io::mirror::__capacity")],
    );
    refused(
        "    let n = std::io::mirror::__consume(\"h\", 1);\n    println(n);",
        &[("`std::io::mirror::__consume` argument 1: expected `Int`, got `String`", "\"h\"")],
    );
    // The values are what the helper returns: a window is a BytesMut.
    refused(
        "    let h = std::io::mirror::__new(4096);\n    let r: BytesMut = std::io::mirror::__readable(h);\n    let n: Int = \
         std::io::mirror::__len(h);\n    std::bytes::write_i8(r, 0, n) or raise;",
        &[],
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

/// `__finish` and `__snapshot` are `(Int) -> Bytes`, signed after S7
/// made the stdlib call fixture print their lengths instead of the
/// Bytes, which do not print.
#[test]
fn std_bytes_builder_finish_and_snapshot_are_checked() {
    refused(
        "    let f = std::bytes::builder::__finish(1, 2);\n    println(len(f));",
        &[("`std::bytes::builder::__finish` takes 1 argument, got 2", "std::bytes::builder::__finish")],
    );
    refused(
        "    let s = std::bytes::builder::__snapshot(\"h\");\n    println(len(s));",
        &[("`std::bytes::builder::__snapshot` argument 1: expected `Int`, got `String`", "\"h\"")],
    );
    refused(
        "    let s = std::bytes::builder::__snapshot(1) or b\"\";\n    println(len(s));",
        &[(
            "`std::bytes::builder::__snapshot` is not fallible (it returns `Bytes`); drop the `or` clause",
            "std::bytes::builder::__snapshot(1)",
        )],
    );
    refused(
        "    let f = std::bytes::builder::__finish(1);\n    println(f);",
        &[(
            "`println` cannot render a value of type `Bytes` — `Bytes` is binary — choose a rendering (hex, \
             length, or a text decode)",
            "f",
        )],
    );
    // The value is the Bytes it always was.
    refused("    let f: Bytes = std::bytes::builder::__finish(1);\n    println(len(f));", &[]);
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
fn std_crypto_calls_are_checked() {
    refused(
        "    let k = std::bytes::from_string(\"k\");\n    let ok = std::crypto::ecdsa_p256_verify(k, k);\n    println(ok);",
        &[("`std::crypto::ecdsa_p256_verify` takes 3 arguments, got 2", "std::crypto::ecdsa_p256_verify")],
    );
    refused(
        "    let k = std::bytes::from_string(\"k\");\n    let ok = std::crypto::ecdsa_p256_verify(k, \"message\", k);\n    println(ok);",
        &[("`std::crypto::ecdsa_p256_verify` argument 2: expected `Bytes`, got `String`", "\"message\"")],
    );
    refused(
        "    let k = std::bytes::from_string(\"k\");\n    let ok = std::crypto::ecdsa_p256_verify(k, k, k) or false;\n    println(ok);",
        &[(
            "`std::crypto::ecdsa_p256_verify` is not fallible (it returns `Bool`); drop the `or` clause",
            "std::crypto::ecdsa_p256_verify(k, k, k)",
        )],
    );
}

#[test]
fn std_decimal_calls_are_checked() {
    refused(
        "    let d = std::str::parse_decimal(\"1.5\") or raise;\n    let s = std::decimal::format(d);\n    println(s);",
        &[("`std::decimal::format` takes 2 arguments, got 1", "std::decimal::format")],
    );
    refused(
        "    let s = std::decimal::format(1.5, 2);\n    println(s);",
        &[("`std::decimal::format` argument 1: expected `Decimal`, got `Float`", "1.5")],
    );
    refused(
        "    let d = std::str::parse_decimal(\"1.5\") or raise;\n    let s = std::decimal::format(d, 2) or \"\";\n    println(s);",
        &[(
            "`std::decimal::format` is not fallible (it returns `String`); drop the `or` clause",
            "std::decimal::format(d, 2)",
        )],
    );
}

#[test]
fn std_io_file_calls_are_checked() {
    refused(
        "    let s = std::io::file::__read_line(3, 80);\n    println(s);",
        &[("`std::io::file::__read_line` takes 1 argument, got 2", "std::io::file::__read_line")],
    );
    refused(
        "    let e = std::io::file::__at_eof(\"/tmp/x\");\n    println(e);",
        &[("`std::io::file::__at_eof` argument 1: expected `Int`, got `String`", "\"/tmp/x\"")],
    );
    refused(
        "    let r = std::io::file::__close(3) or 0;\n    println(r);",
        &[(
            "`std::io::file::__close` is not fallible (it returns `Int`); drop the `or` clause",
            "std::io::file::__close(3)",
        )],
    );
}

#[test]
fn std_io_tcp_calls_are_checked() {
    refused(
        "    let fd = std::io::tcp::__connect(\"127.0.0.1\");\n    println(fd);",
        &[("`std::io::tcp::__connect` takes 2 arguments, got 1", "std::io::tcp::__connect")],
    );
    refused(
        "    let n = std::io::tcp::__send(3, std::bytes::from_string(\"x\"));\n    println(n);",
        &[(
            "`std::io::tcp::__send` argument 2: expected `String`, got `Bytes`",
            "std::bytes::from_string(\"x\")",
        )],
    );
    refused(
        "    let e = std::io::tcp::__last_io_status() or 0;\n    println(e);",
        &[(
            "`std::io::tcp::__last_io_status` is not fallible (it returns `Int`); drop the `or` clause",
            "std::io::tcp::__last_io_status()",
        )],
    );
}

#[test]
fn std_io_tls_calls_are_checked() {
    refused(
        "    let ns = std::io::tls::last_recv_kernel_ns(3);\n    println(ns);",
        &[("`std::io::tls::last_recv_kernel_ns` takes 0 arguments, got 1", "std::io::tls::last_recv_kernel_ns")],
    );
    refused(
        "    let ns = std::io::tls::last_recv_user_ns() or 0;\n    println(ns);",
        &[(
            "`std::io::tls::last_recv_user_ns` is not fallible (it returns `Int`); drop the `or` clause",
            "std::io::tls::last_recv_user_ns()",
        )],
    );
}

#[test]
fn std_io_udp_calls_are_checked() {
    refused(
        "    let r = std::io::udp::__close();\n    println(r);",
        &[("`std::io::udp::__close` takes 1 argument, got 0", "std::io::udp::__close")],
    );
    refused(
        "    let r = std::io::udp::__close(3) or 0;\n    println(r);",
        &[(
            "`std::io::udp::__close` is not fallible (it returns `Int`); drop the `or` clause",
            "std::io::udp::__close(3)",
        )],
    );
    // `send`'s message was `Any` in its row while its helper takes only a
    // String (or a view of one): a Bytes message is refused by the check.
    refused(
        "    std::io::udp::send(3, \"127.0.0.1\", 9, std::bytes::from_string(\"x\")) or raise;",
        &[(
            "`std::io::udp::send` argument 4: expected `String`, got `Bytes`",
            "std::bytes::from_string(\"x\")",
        )],
    );
}

#[test]
fn std_shm_calls_are_checked() {
    refused(
        "    let s = std::shm::last_record_seq(0);\n    println(s);",
        &[("`std::shm::last_record_seq` takes 0 arguments, got 1", "std::shm::last_record_seq")],
    );
    refused(
        "    let s = std::shm::last_record_seq() or 0;\n    println(s);",
        &[(
            "`std::shm::last_record_seq` is not fallible (it returns `Int`); drop the `or` clause",
            "std::shm::last_record_seq()",
        )],
    );
}

#[test]
fn std_term_calls_are_checked() {
    refused(
        "    std::term::__raw_enable(0);",
        &[("`std::term::__raw_enable` takes 0 arguments, got 1", "std::term::__raw_enable")],
    );
    refused(
        "    let p = std::term::__size_packed() or 0;\n    println(p);",
        &[(
            "`std::term::__size_packed` is not fallible (it returns `Int`); drop the `or` clause",
            "std::term::__size_packed()",
        )],
    );
}

#[test]
fn std_ts_calls_are_checked() {
    refused(
        "    let c = std::ts::node_child(1);\n    println(c);",
        &[("`std::ts::node_child` takes 2 arguments, got 1", "std::ts::node_child")],
    );
    refused(
        "    let t = std::ts::parse_go(42);\n    println(t);",
        &[("`std::ts::parse_go` argument 1: expected `String`, got `Int`", "42")],
    );
    refused(
        "    let k = std::ts::node_kind(1) or \"\";\n    println(k);",
        &[(
            "`std::ts::node_kind` is not fallible (it returns `String`); drop the `or` clause",
            "std::ts::node_kind(1)",
        )],
    );
}

#[test]
fn std_str_calls_are_checked() {
    refused(
        "    let b = std::str::builder_new(16);\n    println(std::str::builder_len(b));",
        &[("`std::str::builder_new` takes 0 arguments, got 1", "std::str::builder_new")],
    );
    refused(
        "    let b = std::str::builder_new();\n    std::str::builder_append(b, 7);",
        &[("`std::str::builder_append` argument 2: expected `String`, got `Int`", "7")],
    );
    refused(
        "    let b = std::str::builder_new();\n    let n = std::str::builder_len(b) or 0;\n    println(n);",
        &[(
            "`std::str::builder_len` is not fallible (it returns `Int`); drop the `or` clause",
            "std::str::builder_len(b)",
        )],
    );
}

#[test]
fn std_json_calls_are_checked() {
    let json = "    let j = \"{\\\"a\\\": 1}\";\n    let it = std::json::object_first(j);\n";
    refused(
        &format!("{json}    let n = std::json::obj_value_int(it);\n    println(n);"),
        &[("`std::json::obj_value_int` takes 2 arguments, got 1", "std::json::obj_value_int")],
    );
    // The iterator and the text swapped: the iterator's type is checked.
    refused(
        &format!("{json}    let n = std::json::obj_value_int(j, it);\n    println(n);"),
        &[
            ("`std::json::obj_value_int` argument 1: expected `std::json::ObjectIterSpan`, got `String`", "j"),
            ("`std::json::obj_value_int` argument 2: expected `String`, got `std::json::ObjectIterSpan`", "it"),
        ],
    );
    refused(
        "    let p = std::json::next_non_ws(\"  x\", 0, 3) or 0;\n    println(p);",
        &[(
            "`std::json::next_non_ws` is not fallible (it returns `Int`); drop the `or` clause",
            "std::json::next_non_ws(\"  x\", 0, 3)",
        )],
    );
}

#[test]
fn std_test_calls_are_checked() {
    refused(
        "    std::test::assert(1 == 1);",
        &[("`std::test::assert` takes 2 arguments, got 1", "std::test::assert")],
    );
    refused(
        "    std::test::assert_eq_int(\"1\", 1, \"one\");",
        &[("`std::test::assert_eq_int` argument 1: expected `Int`, got `String`", "\"1\"")],
    );
    refused(
        "    std::test::assert_eq_str(\"a\", \"a\", \"same\") or discard;",
        &[(
            "`std::test::assert_eq_str` is not fallible (it returns `()`); drop the `or` clause",
            "std::test::assert_eq_str(\"a\", \"a\", \"same\")",
        )],
    );
}

#[test]
fn std_text_calls_are_checked() {
    refused(
        "    let h = std::text::md_to_html(\"# a\", true);\n    println(h);",
        &[("`std::text::md_to_html` takes 1 argument, got 2", "std::text::md_to_html")],
    );
    refused(
        "    let h = std::text::md_to_html(std::bytes::from_string(\"# a\"));\n    println(h);",
        &[(
            "`std::text::md_to_html` argument 1: expected `String`, got `Bytes`",
            "std::bytes::from_string(\"# a\")",
        )],
    );
    refused(
        "    let h = std::text::md_to_html(\"# a\") or \"\";\n    println(h);",
        &[(
            "`std::text::md_to_html` is not fallible (it returns `String`); drop the `or` clause",
            "std::text::md_to_html(\"# a\")",
        )],
    );
}

#[test]
fn std_http_calls_are_checked() {
    let req = "    let r = std::http::parse_request(\"GET / HTTP/1.1\\r\\n\\r\\n\");\n";
    refused(
        &format!("{req}    let h = std::http::header(r);\n    println(h);"),
        &[("`std::http::header` takes 2 arguments, got 1", "std::http::header")],
    );
    refused(
        &format!("{req}    let h = std::http::header(r, 7);\n    println(h);"),
        &[("`std::http::header` argument 2: expected `String`, got `Int`", "7")],
    );
    refused(
        &format!("{req}    let h = std::http::header(r, \"Host\") or \"\";\n    println(h);"),
        &[(
            "`std::http::header` is not fallible (it returns `String`); drop the `or` clause",
            "std::http::header(r, \"Host\")",
        )],
    );
    // The response writer's two handles are typed: a Request is not a
    // Response.
    refused(
        &format!("{req}    std::http::write_response(r, r);"),
        &[
            ("`std::http::write_response` argument 1: expected `std::io::tcp::Stream`, got `std::http::Request`", "r"),
            ("`std::http::write_response` argument 2: expected `std::http::Response`, got `std::http::Request`", "r"),
        ],
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
