//! Builtin functions in scope without `import`.
//!
//! v0 cut: `print`, `println`, `time::sleep`. Future cuts add
//! the rest of the stdlib (collections, strings, math) — these
//! three are the minimum to make the example ladder run.
//!
//! ## Clock discipline
//!
//! Every timing primitive in lotus is grounded on **CLOCK_MONOTONIC**.
//! `time::sleep` calls `clock_nanosleep(CLOCK_MONOTONIC, 0, &req, &rem)`
//! directly with EINTR retry, rather than `std::thread::sleep`, so the
//! contract is in code rather than implementation detail of std.
//! `CLOCK_REALTIME` is reserved for `time::now()` (wall-clock
//! observation only); scheduling decisions never touch it.

use std::rc::Rc;

use crate::eval::{io_error_value, parse_error_value};
use crate::value::{BuiltinRef, Value};

/// Map a `std::io::Error` to the IoError shape the agents see.
/// Mirrors `lotus_io_error_kind`'s errno → kind taxonomy in the
/// C runtime so both runtimes report the same tags.
fn io_error_from_std(e: &std::io::Error, path: &str) -> Value {
    use std::io::ErrorKind as K;
    let kind = match e.kind() {
        K::NotFound => "not_found",
        K::PermissionDenied => "permission_denied",
        K::AlreadyExists => "already_exists",
        K::ConnectionRefused => "connection_refused",
        K::ConnectionReset => "connection_reset",
        K::ConnectionAborted => "connection_aborted",
        K::TimedOut => "timeout",
        K::Interrupted => "interrupted",
        K::WouldBlock => "would_block",
        K::InvalidInput | K::InvalidData => "invalid",
        K::WriteZero | K::UnexpectedEof => "io",
        _ => "io",
    };
    let errno = e.raw_os_error().unwrap_or(0) as i64;
    io_error_value(kind, errno, path)
}

pub fn install_builtins(env: &crate::env::Env) {
    env.define(
        "print",
        Value::Builtin(BuiltinRef {
            name: "print",
            func: Rc::new(builtin_print),
        }),
    );
    env.define(
        "println",
        Value::Builtin(BuiltinRef {
            name: "println",
            func: Rc::new(builtin_println),
        }),
    );
    env.define(
        "eprint",
        Value::Builtin(BuiltinRef {
            name: "eprint",
            func: Rc::new(builtin_eprint),
        }),
    );
    env.define(
        "eprintln",
        Value::Builtin(BuiltinRef {
            name: "eprintln",
            func: Rc::new(builtin_eprintln),
        }),
    );
    env.define(
        "len",
        Value::Builtin(BuiltinRef {
            name: "len",
            func: Rc::new(builtin_len),
        }),
    );
    env.define(
        "to_string",
        Value::Builtin(BuiltinRef {
            name: "to_string",
            func: Rc::new(builtin_to_string),
        }),
    );
    env.define(
        "min",
        Value::Builtin(BuiltinRef {
            name: "min",
            func: Rc::new(builtin_min),
        }),
    );
    env.define(
        "max",
        Value::Builtin(BuiltinRef {
            name: "max",
            func: Rc::new(builtin_max),
        }),
    );
    env.define(
        "abs",
        Value::Builtin(BuiltinRef {
            name: "abs",
            func: Rc::new(builtin_abs),
        }),
    );
    env.define(
        "starts_with",
        Value::Builtin(BuiltinRef {
            name: "starts_with",
            func: Rc::new(builtin_starts_with),
        }),
    );
    env.define(
        "contains",
        Value::Builtin(BuiltinRef {
            name: "contains",
            func: Rc::new(builtin_contains),
        }),
    );
    // v1.x-11: explicit Float → Int narrowing. Surface is
    // `Int(x)` — a constructor-shaped call. Float arg truncates
    // toward zero (matches LLVM fptosi semantics); Int arg is
    // the identity. Other types are rejected so silent narrowing
    // doesn't sneak in.
    env.define(
        "Int",
        Value::Builtin(BuiltinRef {
            name: "Int",
            func: Rc::new(builtin_int_cast),
        }),
    );
}

fn builtin_min(args: &[Value]) -> Result<Value, String> {
    binop_choose(args, "min", true)
}

fn builtin_max(args: &[Value]) -> Result<Value, String> {
    binop_choose(args, "max", false)
}

fn binop_choose(
    args: &[Value],
    name: &str,
    pick_smaller: bool,
) -> Result<Value, String> {
    if args.len() != 2 {
        return Err(format!(
            "`{}` expects exactly 2 arguments, got {}",
            name,
            args.len()
        ));
    }
    match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => {
            Ok(Value::Int(if pick_smaller { *a.min(b) } else { *a.max(b) }))
        }
        (Value::Float(a), Value::Float(b)) => {
            let chosen = if pick_smaller { a.min(*b) } else { a.max(*b) };
            Ok(Value::Float(chosen))
        }
        (Value::Duration(a), Value::Duration(b)) => Ok(Value::Duration(
            if pick_smaller { *a.min(b) } else { *a.max(b) },
        )),
        (Value::Decimal(a), Value::Decimal(b)) => {
            let ord = crate::value::DecimalVal::cmp(*a, *b);
            let pick_a = if pick_smaller {
                ord != std::cmp::Ordering::Greater
            } else {
                ord != std::cmp::Ordering::Less
            };
            Ok(Value::Decimal(if pick_a { *a } else { *b }))
        }
        (l, r) => Err(format!(
            "`{}` not supported for {} and {}",
            name,
            l.type_name(),
            r.type_name()
        )),
    }
}

fn builtin_abs(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "`abs` expects exactly 1 argument, got {}",
            args.len()
        ));
    }
    match &args[0] {
        Value::Int(n) => Ok(Value::Int(n.abs())),
        Value::Float(f) => Ok(Value::Float(f.abs())),
        Value::Duration(n) => Ok(Value::Duration(n.abs())),
        Value::Decimal(d) => {
            let abs = crate::value::DecimalVal {
                mantissa: d.mantissa.abs(),
                scale: d.scale,
            };
            Ok(Value::Decimal(abs))
        }
        other => Err(format!("`abs` not supported for {}", other.type_name())),
    }
}

fn builtin_starts_with(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err(format!(
            "`starts_with` expects exactly 2 arguments, got {}",
            args.len()
        ));
    }
    match (&args[0], &args[1]) {
        (Value::String(s), Value::String(p)) => {
            Ok(Value::Bool(s.starts_with(p.as_str())))
        }
        (l, r) => Err(format!(
            "`starts_with` expects two String args; got {} and {}",
            l.type_name(),
            r.type_name()
        )),
    }
}

fn builtin_contains(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err(format!(
            "`contains` expects exactly 2 arguments, got {}",
            args.len()
        ));
    }
    match (&args[0], &args[1]) {
        (Value::String(s), Value::String(sub)) => {
            Ok(Value::Bool(s.contains(sub.as_str())))
        }
        (l, r) => Err(format!(
            "`contains` expects two String args; got {} and {}",
            l.type_name(),
            r.type_name()
        )),
    }
}

fn builtin_int_cast(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "`Int` cast expects exactly 1 argument, got {}",
            args.len()
        ));
    }
    match &args[0] {
        Value::Int(n) => Ok(Value::Int(*n)),
        // Match LLVM fptosi semantics — truncate toward zero.
        // NaN / out-of-range f64 produces unspecified bits in
        // LLVM; we mirror by clamping to i64 via the `as` cast
        // saturation Rust applies for finite values and
        // returning 0 for NaN.
        Value::Float(f) => {
            if f.is_nan() {
                Ok(Value::Int(0))
            } else {
                Ok(Value::Int(*f as i64))
            }
        }
        other => Err(format!(
            "`Int(...)` cast not supported for {} (only Float → Int \
             narrowing and Int identity are supported in v1)",
            other.type_name()
        )),
    }
}

fn builtin_to_string(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "`to_string` expects exactly 1 argument, got {}",
            args.len()
        ));
    }
    // Output must mirror codegen's printf-%g / %lld / %lldns
    // formatting so the same flex app prints identically on
    // both paths. fmt_decimal handles the %g-equivalent shape;
    // Int / Bool / Duration / String are direct conversions.
    let s = match &args[0] {
        Value::Int(n) => n.to_string(),
        Value::Float(f) => crate::eval::fmt_float(*f),
        Value::Decimal(d) => d.display(),
        Value::Bool(b) => {
            if *b {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        Value::Duration(ns) => format!("{}ns", ns),
        Value::String(s) => s.clone(),
        Value::Time(s) => s.clone(),
        Value::EnumVariant { enum_name, variant_name, payload } => {
            if payload.is_empty() {
                format!("{}::{}", enum_name, variant_name)
            } else {
                let parts: Vec<String> =
                    payload.iter().map(|v| v.display()).collect();
                format!("{}::{}({})", enum_name, variant_name, parts.join(", "))
            }
        }
        Value::Struct { name, .. } => {
            return Err(format!(
                "to_string on `{}` (a user `type` record) isn't supported \
                 — Aperio has no auto-derived debug shape at v1. Either \
                 access a primitive field (e.g. `to_string(x.id)`) or \
                 render it explicitly via `std::json::Builder`",
                name
            ));
        }
        other => {
            return Err(format!(
                "`to_string` not supported for {}",
                other.type_name()
            ));
        }
    };
    Ok(Value::String(s))
}

fn builtin_len(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "`len` expects exactly 1 argument, got {}",
            args.len()
        ));
    }
    match &args[0] {
        Value::String(s) => Ok(Value::Int(s.len() as i64)),
        Value::Array(a) => Ok(Value::Int(a.borrow().len() as i64)),
        Value::Bytes(b) => Ok(Value::Int(b.len() as i64)),
        other => Err(format!(
            "`len` not supported for {}",
            other.type_name()
        )),
    }
}

fn builtin_print(args: &[Value]) -> Result<Value, String> {
    use std::io::Write;
    let body: String = args.iter().map(|v| v.display()).collect();
    let mut out = std::io::stdout().lock();
    out.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())?;
    Ok(Value::Unit)
}

fn builtin_println(args: &[Value]) -> Result<Value, String> {
    let body: String = args.iter().map(|v| v.display()).collect();
    println!("{}", body);
    Ok(Value::Unit)
}

fn builtin_eprint(args: &[Value]) -> Result<Value, String> {
    use std::io::Write;
    let body: String = args.iter().map(|v| v.display()).collect();
    let mut err = std::io::stderr().lock();
    err.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    err.flush().map_err(|e| e.to_string())?;
    Ok(Value::Unit)
}

fn builtin_eprintln(args: &[Value]) -> Result<Value, String> {
    let body: String = args.iter().map(|v| v.display()).collect();
    eprintln!("{}", body);
    Ok(Value::Unit)
}

/// `time::sleep(duration)` — block the thread for the given
/// duration on the **monotonic** clock. v0: the interpreter is
/// single-threaded so this is just a real OS sleep. Replace with
/// the cooperative scheduler in Phase 2.
///
/// Implementation: `clock_nanosleep(CLOCK_MONOTONIC, 0, &req, &rem)`
/// with EINTR retry. CLOCK_MONOTONIC means NTP / wall-clock
/// adjustments cannot warp the requested interval. The retry loop
/// resumes from `rem` so a delivered signal does not shorten the
/// total sleep.
pub fn time_sleep(args: &[Value]) -> Result<Value, String> {
    let ns = match args.first() {
        Some(Value::Duration(ns)) => *ns,
        Some(other) => {
            return Err(format!(
                "time::sleep expects a Duration, got {}",
                other.type_name()
            ));
        }
        None => return Err("time::sleep called with no arguments".to_string()),
    };
    if ns > 0 {
        monotonic_sleep_ns(ns);
    }
    Ok(Value::Unit)
}

/// Sleep `ns` nanoseconds, retrying on EINTR.
///
/// Linux: `clock_nanosleep(CLOCK_MONOTONIC, 0, ...)` for an
/// explicitly-monotonic relative sleep.
///
/// macOS / other non-Linux POSIX: `nanosleep(...)`. `clock_nanosleep`
/// isn't exposed by Apple's libc; `nanosleep` is the standard
/// relative-sleep primitive. Both functions take a relative
/// timespec when the absolute-flag is 0, so the call sequence is
/// equivalent — the clock-source distinction only matters for
/// TIMER_ABSTIME (absolute) sleeps, which we don't use.
fn monotonic_sleep_ns(ns: i64) {
    if ns <= 0 {
        return;
    }
    let mut req = libc::timespec {
        tv_sec: (ns / 1_000_000_000) as libc::time_t,
        tv_nsec: (ns % 1_000_000_000) as libc::c_long,
    };
    let mut rem = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    loop {
        #[cfg(target_os = "linux")]
        let r = unsafe {
            libc::clock_nanosleep(
                libc::CLOCK_MONOTONIC,
                0,
                &req,
                &mut rem,
            )
        };
        #[cfg(not(target_os = "linux"))]
        let r = {
            // nanosleep returns 0 on success or -1 with errno set.
            // Normalize to clock_nanosleep's shape (return errno
            // directly) so the EINTR check below works on both.
            // std::io::Error::last_os_error() reads errno
            // portably — `libc::__error()` works on macOS but not
            // on every non-Linux POSIX.
            let rc = unsafe { libc::nanosleep(&req, &mut rem) };
            if rc == 0 {
                0
            } else {
                std::io::Error::last_os_error()
                    .raw_os_error()
                    .unwrap_or(libc::EIO)
            }
        };
        if r == 0 {
            return;
        }
        if r == libc::EINTR {
            req = rem;
            continue;
        }
        // Any other return is best-effort: we exit rather than
        // looping forever on a permanent error.
        return;
    }
}

/// `time::monotonic()` — current value of the monotonic clock as
/// a Duration (i64 nanoseconds since an unspecified reference).
/// Only meaningful for elapsed-time differences; the absolute
/// value has no calendar interpretation. Pairs with the
/// `clock_nanosleep(CLOCK_MONOTONIC, ...)` discipline used by
/// `time::sleep` so all scheduling decisions sit on one clock.
///
/// `CLOCK_REALTIME` (wall-clock) and the corresponding
/// `time::now()` are reserved for observation only — they're not
/// suitable for scheduling because NTP slewing and leap seconds
/// can warp the value.
pub fn time_monotonic(args: &[Value]) -> Result<Value, String> {
    if !args.is_empty() {
        return Err(format!(
            "time::monotonic takes no arguments, got {}",
            args.len()
        ));
    }
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    let r = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    if r != 0 {
        return Err(format!(
            "clock_gettime(CLOCK_MONOTONIC) failed: errno {}",
            r
        ));
    }
    let ns = (ts.tv_sec as i64)
        .saturating_mul(1_000_000_000)
        .saturating_add(ts.tv_nsec as i64);
    Ok(Value::Duration(ns))
}

/// C7 (pond follow-up): `time::now()` — wall-clock seconds since
/// the Unix epoch as `Int`. Wraps `clock_gettime(CLOCK_REALTIME,
/// &ts)` and returns `ts.tv_sec`. Observation only — NTP slewing
/// and leap seconds can warp the value, so `time::monotonic`
/// stays the basis for scheduling. Pond surfaces this for
/// session-cookie expiries that must survive a process restart
/// (the monotonic origin resets at boot; the wall-clock origin
/// does not).
pub fn time_now(args: &[Value]) -> Result<Value, String> {
    if !args.is_empty() {
        return Err(format!(
            "time::now takes no arguments, got {}",
            args.len()
        ));
    }
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    let r = unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts) };
    if r != 0 {
        return Err(format!(
            "clock_gettime(CLOCK_REALTIME) failed: errno {}",
            r
        ));
    }
    Ok(Value::Int(ts.tv_sec as i64))
}

pub fn resolve_path(segments: &[&str]) -> Option<Value> {
    match segments {
        // Canonical `std::time::*` paths (matches the codegen
        // dispatcher's m79 std-aliases). The bare `time::*` form
        // is preserved below as a legacy alias for the pre-m79
        // examples; both route to the same builtin implementations.
        ["std", "time", "sleep"] | ["time", "sleep"] => Some(Value::Builtin(BuiltinRef {
            name: "time::sleep",
            func: Rc::new(time_sleep),
        })),
        ["std", "time", "monotonic"] | ["time", "monotonic"] => Some(Value::Builtin(BuiltinRef {
            name: "time::monotonic",
            func: Rc::new(time_monotonic),
        })),
        // C7 (pond follow-up): wall-clock seconds-since-epoch.
        // Mirrors the codegen-side `std::time::now` path-call
        // dispatch. No legacy bare-`time::now` alias because the
        // surface ships fresh under the canonical `std::*` prefix.
        ["std", "time", "now"] => Some(Value::Builtin(BuiltinRef {
            name: "time::now",
            func: Rc::new(time_now),
        })),
        // v1.x-16: parse_float / can_parse_float / base64::decode.
        // String-parsing primitives (interpreter parity with codegen).
        ["std", "str", "parse_float"] => Some(Value::Builtin(BuiltinRef {
            name: "std::str::parse_float",
            func: Rc::new(std_str_parse_float),
        })),
        ["std", "str", "can_parse_float"] => Some(Value::Builtin(BuiltinRef {
            name: "std::str::can_parse_float",
            func: Rc::new(std_str_can_parse_float),
        })),
        ["std", "text", "base64", "decode"] => Some(Value::Builtin(BuiltinRef {
            name: "std::text::base64::decode",
            func: Rc::new(std_text_base64_decode),
        })),
        // C3 (pond follow-up): SHA-256 + HMAC-SHA256. Interpreter
        // parity for codegen's lotus_crypto_sha256 /
        // lotus_crypto_hmac_sha256. Hand-rolled (no crypto deps in
        // aperio-runtime); same algorithm as the C side.
        ["std", "crypto", "sha256"] => Some(Value::Builtin(BuiltinRef {
            name: "std::crypto::sha256",
            func: Rc::new(std_crypto_sha256),
        })),
        ["std", "crypto", "hmac_sha256"] => Some(Value::Builtin(BuiltinRef {
            name: "std::crypto::hmac_sha256",
            func: Rc::new(std_crypto_hmac_sha256),
        })),
        // v1.x: ASCII case folding.
        ["std", "str", "lower"] => Some(Value::Builtin(BuiltinRef {
            name: "std::str::lower",
            func: Rc::new(std_str_lower),
        })),
        ["std", "str", "upper"] => Some(Value::Builtin(BuiltinRef {
            name: "std::str::upper",
            func: Rc::new(std_str_upper),
        })),
        ["std", "str", "trim"] => Some(Value::Builtin(BuiltinRef {
            name: "std::str::trim",
            func: Rc::new(std_str_trim),
        })),
        ["std", "str", "substring"] => Some(Value::Builtin(BuiltinRef {
            name: "std::str::substring",
            func: Rc::new(std_str_substring),
        })),
        ["std", "str", "replace"] => Some(Value::Builtin(BuiltinRef {
            name: "std::str::replace",
            func: Rc::new(std_str_replace),
        })),
        ["std", "str", "repeat"] => Some(Value::Builtin(BuiltinRef {
            name: "std::str::repeat",
            func: Rc::new(std_str_repeat),
        })),
        ["std", "str", "pad_left"] => Some(Value::Builtin(BuiltinRef {
            name: "std::str::pad_left",
            func: Rc::new(std_str_pad_left),
        })),
        ["std", "str", "pad_right"] => Some(Value::Builtin(BuiltinRef {
            name: "std::str::pad_right",
            func: Rc::new(std_str_pad_right),
        })),
        // v1.x-15: string-builder primitive. The interpreter
        // uses a Bytes-shaped carrier — the first 8 bytes of the
        // backing Vec<u8> are a sentinel `"_sb_v1__"` so attempts
        // to mis-use the handle as a real Bytes blob produce
        // recognizable garbage. Append / finish trust the
        // carrier shape; they're the only consumers.
        ["std", "str", "builder_new"] => Some(Value::Builtin(BuiltinRef {
            name: "std::str::builder_new",
            func: Rc::new(std_str_builder_new),
        })),
        ["std", "str", "builder_append"] => Some(Value::Builtin(BuiltinRef {
            name: "std::str::builder_append",
            func: Rc::new(std_str_builder_append),
        })),
        ["std", "str", "builder_len"] => Some(Value::Builtin(BuiltinRef {
            name: "std::str::builder_len",
            func: Rc::new(std_str_builder_len),
        })),
        ["std", "str", "builder_finish"] => Some(Value::Builtin(BuiltinRef {
            name: "std::str::builder_finish",
            func: Rc::new(std_str_builder_finish),
        })),
        // C10 (pond follow-up): binary-safe builder. Same shape as
        // the str-builder but the chunk arg is Bytes and finish
        // returns Bytes — embedded NULs survive end-to-end.
        ["std", "bytes", "builder_new"] => Some(Value::Builtin(BuiltinRef {
            name: "std::bytes::builder_new",
            func: Rc::new(std_bytes_builder_new),
        })),
        ["std", "bytes", "builder_append"] => Some(Value::Builtin(BuiltinRef {
            name: "std::bytes::builder_append",
            func: Rc::new(std_bytes_builder_append),
        })),
        ["std", "bytes", "builder_len"] => Some(Value::Builtin(BuiltinRef {
            name: "std::bytes::builder_len",
            func: Rc::new(std_bytes_builder_len),
        })),
        ["std", "bytes", "builder_finish"] => Some(Value::Builtin(BuiltinRef {
            name: "std::bytes::builder_finish",
            func: Rc::new(std_bytes_builder_finish),
        })),
        // v1.x: stdin line reader. Trailing newline stripped;
        // empty string returned on EOF / error. Programs that
        // need to distinguish EOF from an empty input line drive
        // the read through the sibling status getter.
        ["std", "io", "stdin", "read_line"] => Some(Value::Builtin(BuiltinRef {
            name: "std::io::stdin::read_line",
            func: Rc::new(std_io_stdin_read_line),
        })),
        ["std", "io", "stdin", "read_line_status"] => Some(Value::Builtin(BuiltinRef {
            name: "std::io::stdin::read_line_status",
            func: Rc::new(std_io_stdin_read_line_status),
        })),
        // env — argv + environment variables. Mirrors the
        // codegen lowering surface; argv comes from the
        // CLI-set thread-local (see `set_user_args`).
        ["std", "env", "args_count"] => Some(Value::Builtin(BuiltinRef {
            name: "std::env::args_count",
            func: Rc::new(std_env_args_count),
        })),
        ["std", "env", "arg"] => Some(Value::Builtin(BuiltinRef {
            name: "std::env::arg",
            func: Rc::new(std_env_arg),
        })),
        // 2026-05-16: collapse the 3-line "arg-or-default"
        // pattern every CLI-style program reinvents.
        ["std", "env", "arg_or"] => Some(Value::Builtin(BuiltinRef {
            name: "std::env::arg_or",
            func: Rc::new(std_env_arg_or),
        })),
        ["std", "env", "var"] => Some(Value::Builtin(BuiltinRef {
            name: "std::env::var",
            func: Rc::new(std_env_var),
        })),
        ["std", "env", "var_exists"] => Some(Value::Builtin(BuiltinRef {
            name: "std::env::var_exists",
            func: Rc::new(std_env_var_exists),
        })),
        // process.
        ["std", "process", "pid"] => Some(Value::Builtin(BuiltinRef {
            name: "std::process::pid",
            func: Rc::new(std_process_pid),
        })),
        // io::fs — one-shot file ops. POSIX-style: sentinel "" /
        // -1 / 0 on error; errno-style status via the *_status
        // siblings for the cases where it matters.
        ["std", "io", "fs", "read_file"] => Some(Value::Builtin(BuiltinRef {
            name: "std::io::fs::read_file",
            func: Rc::new(std_io_fs_read_file),
        })),
        ["std", "io", "fs", "write_file"] => Some(Value::Builtin(BuiltinRef {
            name: "std::io::fs::write_file",
            func: Rc::new(std_io_fs_write_file),
        })),
        ["std", "io", "fs", "write_file_append"] => Some(Value::Builtin(BuiltinRef {
            name: "std::io::fs::write_file_append",
            func: Rc::new(std_io_fs_write_file_append),
        })),
        ["std", "io", "fs", "file_exists"] => Some(Value::Builtin(BuiltinRef {
            name: "std::io::fs::file_exists",
            func: Rc::new(std_io_fs_file_exists),
        })),
        ["std", "io", "fs", "file_size"] => Some(Value::Builtin(BuiltinRef {
            name: "std::io::fs::file_size",
            func: Rc::new(std_io_fs_file_size),
        })),
        ["std", "io", "fs", "mkdir"] => Some(Value::Builtin(BuiltinRef {
            name: "std::io::fs::mkdir",
            func: Rc::new(std_io_fs_mkdir),
        })),
        // C9 (pond/logfmt + pond/agent/sandbox).
        ["std", "io", "fs", "rename"] => Some(Value::Builtin(BuiltinRef {
            name: "std::io::fs::rename",
            func: Rc::new(std_io_fs_rename),
        })),
        ["std", "io", "fs", "unlink"] => Some(Value::Builtin(BuiltinRef {
            name: "std::io::fs::unlink",
            func: Rc::new(std_io_fs_unlink),
        })),
        ["std", "io", "fs", "mktemp"] => Some(Value::Builtin(BuiltinRef {
            name: "std::io::fs::mktemp",
            func: Rc::new(std_io_fs_mktemp),
        })),
        // C4 (pond/crypto): CSPRNG bytes via getrandom(2) +
        // /dev/urandom fallback. Returns Bytes fallible(IoError).
        ["std", "os", "getrandom"] => Some(Value::Builtin(BuiltinRef {
            name: "std::os::getrandom",
            func: Rc::new(std_os_getrandom),
        })),
        // str — the parse_int family (used in CLI argument
        // parsing).
        ["std", "str", "parse_int"] => Some(Value::Builtin(BuiltinRef {
            name: "std::str::parse_int",
            func: Rc::new(std_str_parse_int),
        })),
        ["std", "str", "can_parse_int"] => Some(Value::Builtin(BuiltinRef {
            name: "std::str::can_parse_int",
            func: Rc::new(std_str_can_parse_int),
        })),
        // math — libm-shaped float primitives.
        ["std", "math", "sqrt"] => Some(Value::Builtin(BuiltinRef {
            name: "std::math::sqrt",
            func: Rc::new(std_math_sqrt),
        })),
        ["std", "math", "exp"] => Some(Value::Builtin(BuiltinRef {
            name: "std::math::exp",
            func: Rc::new(std_math_exp),
        })),
        ["std", "math", "log"] => Some(Value::Builtin(BuiltinRef {
            name: "std::math::log",
            func: Rc::new(std_math_log),
        })),
        ["std", "math", "floor"] => Some(Value::Builtin(BuiltinRef {
            name: "std::math::floor",
            func: Rc::new(std_math_floor),
        })),
        ["std", "math", "ceil"] => Some(Value::Builtin(BuiltinRef {
            name: "std::math::ceil",
            func: Rc::new(std_math_ceil),
        })),
        ["std", "math", "pow"] => Some(Value::Builtin(BuiltinRef {
            name: "std::math::pow",
            func: Rc::new(std_math_pow),
        })),
        // C8 (pond follow-up): IEEE 754 surface — `tanh`,
        // `nan()`, `inf()`, `is_nan(f)`. Parity with codegen
        // dispatch in `lower_stdlib_path_call_expr`. NaN /
        // inf return f64::NAN / f64::INFINITY; is_nan delegates
        // to f64::is_nan (which is the IEEE 754 `f != f` test).
        ["std", "math", "tanh"] => Some(Value::Builtin(BuiltinRef {
            name: "std::math::tanh",
            func: Rc::new(std_math_tanh),
        })),
        ["std", "math", "nan"] => Some(Value::Builtin(BuiltinRef {
            name: "std::math::nan",
            func: Rc::new(std_math_nan),
        })),
        ["std", "math", "inf"] => Some(Value::Builtin(BuiltinRef {
            name: "std::math::inf",
            func: Rc::new(std_math_inf),
        })),
        ["std", "math", "is_nan"] => Some(Value::Builtin(BuiltinRef {
            name: "std::math::is_nan",
            func: Rc::new(std_math_is_nan),
        })),
        // 2026-05-16: std::text byte-class predicates. Each
        // takes a single byte (Int) and returns Bool. Parity
        // with the inline LLVM lowering in codegen.
        ["std", "text", "is_alpha"] => Some(Value::Builtin(BuiltinRef {
            name: "std::text::is_alpha",
            func: Rc::new(|args| std_text_byte_pred(args, "is_alpha")),
        })),
        ["std", "text", "is_digit"] => Some(Value::Builtin(BuiltinRef {
            name: "std::text::is_digit",
            func: Rc::new(|args| std_text_byte_pred(args, "is_digit")),
        })),
        ["std", "text", "is_alnum"] => Some(Value::Builtin(BuiltinRef {
            name: "std::text::is_alnum",
            func: Rc::new(|args| std_text_byte_pred(args, "is_alnum")),
        })),
        ["std", "text", "is_whitespace"] => Some(Value::Builtin(BuiltinRef {
            name: "std::text::is_whitespace",
            func: Rc::new(|args| std_text_byte_pred(args, "is_whitespace")),
        })),
        ["std", "text", "is_word_char"] => Some(Value::Builtin(BuiltinRef {
            name: "std::text::is_word_char",
            func: Rc::new(|args| std_text_byte_pred(args, "is_word_char")),
        })),
        // 2026-05-16: word-tokenize String into a @form(vec) of
        // String. Parity with the C primitive
        // lotus_text_tokenize_words_into. Tokens are lowercased
        // (matches the wordfreq idiom every agent reaches for).
        ["std", "text", "tokenize_words_into"] => Some(Value::Builtin(BuiltinRef {
            name: "std::text::tokenize_words_into",
            func: Rc::new(std_text_tokenize_words_into),
        })),
        _ => None,
    }
}

/// 2026-05-16: interpreter parity for the word-tokenize
/// primitive. Argument shape: (String source, @form(vec) of
/// String target). Walks the source, pushes each lowercased
/// token into the target's items vec.
fn std_text_tokenize_words_into(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err(format!(
            "std::text::tokenize_words_into expects 2 args (source, \
             target vec), got {}",
            args.len()
        ));
    }
    let src = match &args[0] {
        Value::String(s) => s.clone(),
        other => return Err(format!(
            "std::text::tokenize_words_into: source arg must be String, \
             got {}",
            other.type_name()
        )),
    };
    let handle = match &args[1] {
        Value::Locus(h) => h.clone(),
        other => return Err(format!(
            "std::text::tokenize_words_into: target arg must be a \
             @form(vec) of String locus, got {}",
            other.type_name()
        )),
    };
    let is_form_vec = handle
        .decl
        .form
        .as_ref()
        .map(|f| f.name.name == "vec")
        .unwrap_or(false);
    if !is_form_vec {
        return Err(format!(
            "std::text::tokenize_words_into: target locus `{}` is not a \
             @form(vec)",
            handle.name
        ));
    }
    let items_rc = {
        let slots = handle.slots.borrow();
        slots
            .iter()
            .find_map(|(_, st)| match st {
                crate::value::SlotState::Vec { items } => Some(items.clone()),
                _ => None,
            })
            .ok_or_else(|| format!(
                "std::text::tokenize_words_into: target locus `{}` has no \
                 vec-state slot",
                handle.name
            ))?
    };
    let is_word = |c: u8| {
        (b'a'..=b'z').contains(&c)
            || (b'A'..=b'Z').contains(&c)
            || (b'0'..=b'9').contains(&c)
            || c == b'_'
            || c == b'\''
    };
    let bytes = src.as_bytes();
    let mut i = 0;
    let n = bytes.len();
    let mut items = items_rc.borrow_mut();
    while i < n {
        while i < n && !is_word(bytes[i]) {
            i += 1;
        }
        if i >= n { break; }
        let start = i;
        while i < n && is_word(bytes[i]) {
            i += 1;
        }
        let mut tok: Vec<u8> = bytes[start..i].to_vec();
        for c in tok.iter_mut() {
            if (b'A'..=b'Z').contains(c) {
                *c += 32;
            }
        }
        items.push(Value::String(String::from_utf8_lossy(&tok).to_string()));
    }
    Ok(Value::Unit)
}

/// 2026-05-16: shared body for std::text::is_X predicates.
/// Match the codegen-side inline IR exactly so `aperio run` and
/// `aperio build` agree on the boundary character set.
fn std_text_byte_pred(args: &[Value], which: &str) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::text::{} expects 1 arg, got {}",
            which,
            args.len()
        ));
    }
    let b = match &args[0] {
        Value::Int(n) => *n as i64,
        other => return Err(format!(
            "std::text::{}: arg must be Int (byte value), got {}",
            which,
            other.type_name()
        )),
    };
    let result = match which {
        "is_alpha" => (b'a' as i64..=b'z' as i64).contains(&b)
            || (b'A' as i64..=b'Z' as i64).contains(&b),
        "is_digit" => (b'0' as i64..=b'9' as i64).contains(&b),
        "is_alnum" => (b'a' as i64..=b'z' as i64).contains(&b)
            || (b'A' as i64..=b'Z' as i64).contains(&b)
            || (b'0' as i64..=b'9' as i64).contains(&b),
        "is_whitespace" => matches!(b as u8, b' ' | b'\t' | b'\n' | b'\r'),
        "is_word_char" => (b'a' as i64..=b'z' as i64).contains(&b)
            || (b'A' as i64..=b'Z' as i64).contains(&b)
            || (b'0' as i64..=b'9' as i64).contains(&b)
            || b == b'_' as i64
            || b == b'\'' as i64,
        _ => unreachable!(),
    };
    Ok(Value::Bool(result))
}

// === user-args plumbing =====================================

thread_local! {
    /// argv visible to the interpreted program. Set by the
    /// CLI before invoking run_bundle (so `aperio run script.ap
    /// foo bar` populates this with ["script.ap", "foo",
    /// "bar"]); empty by default for embedded interpreter use.
    /// Mirrors the lotus_env_init stash in the C runtime.
    static USER_ARGS: std::cell::RefCell<Vec<String>> =
        std::cell::RefCell::new(Vec::new());
}

pub fn set_user_args(args: Vec<String>) {
    USER_ARGS.with(|a| *a.borrow_mut() = args);
}

/// v1.x: ASCII case folding mirroring the C runtime's
/// lotus_str_lower / lotus_str_upper. Non-ASCII bytes pass
/// through unchanged.
fn std_str_lower(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::str::lower expects 1 arg, got {}",
            args.len()
        ));
    }
    match &args[0] {
        Value::String(s) => {
            let out: String = s.chars().map(|c| {
                if c.is_ascii_uppercase() { c.to_ascii_lowercase() } else { c }
            }).collect();
            Ok(Value::String(out))
        }
        other => Err(format!(
            "std::str::lower expects String, got {}",
            other.type_name()
        )),
    }
}

fn std_str_upper(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::str::upper expects 1 arg, got {}",
            args.len()
        ));
    }
    match &args[0] {
        Value::String(s) => {
            let out: String = s.chars().map(|c| {
                if c.is_ascii_lowercase() { c.to_ascii_uppercase() } else { c }
            }).collect();
            Ok(Value::String(out))
        }
        other => Err(format!(
            "std::str::upper expects String, got {}",
            other.type_name()
        )),
    }
}

fn std_str_trim(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::str::trim expects 1 arg, got {}",
            args.len()
        ));
    }
    match &args[0] {
        Value::String(s) => Ok(Value::String(
            s.trim_matches(|c: char| {
                c == ' ' || c == '\t' || c == '\r' || c == '\n'
            }).to_string(),
        )),
        other => Err(format!(
            "std::str::trim expects String, got {}",
            other.type_name()
        )),
    }
}

fn std_str_substring(args: &[Value]) -> Result<Value, String> {
    if args.len() != 3 {
        return Err(format!(
            "std::str::substring expects 3 args (s, lo, hi), got {}",
            args.len()
        ));
    }
    let (s, lo, hi) = match (&args[0], &args[1], &args[2]) {
        (Value::String(s), Value::Int(lo), Value::Int(hi)) => (s.as_bytes(), *lo, *hi),
        (a, b, c) => {
            return Err(format!(
                "std::str::substring expects (String, Int, Int), got ({}, {}, {})",
                a.type_name(),
                b.type_name(),
                c.type_name()
            ));
        }
    };
    let n = s.len() as i64;
    let lo = if lo < 0 { 0 } else { lo };
    let hi = if hi > n { n } else { hi };
    if lo >= hi {
        return Ok(Value::String(String::new()));
    }
    let slice = &s[lo as usize..hi as usize];
    Ok(Value::String(String::from_utf8_lossy(slice).into_owned()))
}

fn std_str_repeat(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err(format!(
            "std::str::repeat expects 2 args, got {}",
            args.len()
        ));
    }
    let (s, n) = match (&args[0], &args[1]) {
        (Value::String(s), Value::Int(n)) => (s, *n),
        (a, b) => {
            return Err(format!(
                "std::str::repeat expects (String, Int); got ({}, {})",
                a.type_name(), b.type_name()
            ));
        }
    };
    if n <= 0 {
        return Ok(Value::String(String::new()));
    }
    Ok(Value::String(s.repeat(n as usize)))
}

fn pad_helper(
    args: &[Value],
    name: &str,
    on_left: bool,
) -> Result<Value, String> {
    if args.len() != 3 {
        return Err(format!(
            "std::str::{} expects 3 args, got {}",
            name,
            args.len()
        ));
    }
    let (s, w, pad) = match (&args[0], &args[1], &args[2]) {
        (Value::String(s), Value::Int(w), Value::String(p)) => (s, *w, p),
        (a, b, c) => {
            return Err(format!(
                "std::str::{} expects (String, Int, String); got ({}, {}, {})",
                name,
                a.type_name(), b.type_name(), c.type_name()
            ));
        }
    };
    let sl = s.len() as i64;
    if sl >= w {
        return Ok(Value::String(s.clone()));
    }
    let ch = pad.chars().next().unwrap_or(' ');
    let pad_count = (w - sl) as usize;
    let padding: String = std::iter::repeat(ch).take(pad_count).collect();
    let out = if on_left {
        let mut t = padding;
        t.push_str(s);
        t
    } else {
        let mut t = s.clone();
        t.push_str(&padding);
        t
    };
    Ok(Value::String(out))
}

fn std_str_pad_left(args: &[Value]) -> Result<Value, String> {
    pad_helper(args, "pad_left", true)
}

fn std_str_pad_right(args: &[Value]) -> Result<Value, String> {
    pad_helper(args, "pad_right", false)
}

fn std_str_replace(args: &[Value]) -> Result<Value, String> {
    if args.len() != 3 {
        return Err(format!(
            "std::str::replace expects 3 args, got {}",
            args.len()
        ));
    }
    let (s, needle, rep) = match (&args[0], &args[1], &args[2]) {
        (Value::String(s), Value::String(n), Value::String(r)) => (s, n, r),
        (a, b, c) => {
            return Err(format!(
                "std::str::replace expects (String, String, String); got ({}, {}, {})",
                a.type_name(), b.type_name(), c.type_name()
            ));
        }
    };
    if needle.is_empty() {
        // No-op for empty needle (avoids infinite replace).
        return Ok(Value::String(s.clone()));
    }
    Ok(Value::String(s.replace(needle.as_str(), rep.as_str())))
}

/// v1.x-15: interpreter string builder. The handle is a
/// Value::Bytes wrapping a Vec<u8> that we mutate in place. Since
/// Value::Bytes carries an Rc<RefCell<...>>-free plain Vec, we use
/// the bigger Value::Array(Rc<RefCell<Vec<Value>>>) trick: store
/// the running buffer as a one-element Array containing a
/// Value::Bytes, so we have shared mutable access.
fn std_str_builder_new(args: &[Value]) -> Result<Value, String> {
    if !args.is_empty() {
        return Err(format!(
            "std::str::builder_new expects 0 args, got {}",
            args.len()
        ));
    }
    use std::cell::RefCell;
    use std::rc::Rc;
    let inner = Rc::new(RefCell::new(vec![Value::Bytes(Vec::new())]));
    Ok(Value::Array(inner))
}

fn std_str_builder_append(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err(format!(
            "std::str::builder_append expects 2 args, got {}",
            args.len()
        ));
    }
    let handle = match &args[0] {
        Value::Array(a) => a.clone(),
        other => {
            return Err(format!(
                "std::str::builder_append: handle must be a builder, got {}",
                other.type_name()
            ));
        }
    };
    let chunk = match &args[1] {
        Value::String(s) => s.clone(),
        other => {
            return Err(format!(
                "std::str::builder_append: s must be String, got {}",
                other.type_name()
            ));
        }
    };
    {
        let mut a = handle.borrow_mut();
        let buf = match a.get_mut(0) {
            Some(Value::Bytes(buf)) => buf,
            _ => return Err("std::str::builder_append: corrupt builder handle".to_string()),
        };
        buf.extend_from_slice(chunk.as_bytes());
    }
    // Return the builder handle so the call is usable in expression
    // position (e.g. `let b2 = builder_append(b, "x");` or fluent
    // chaining). Same shape the codegen path exposes.
    Ok(Value::Array(handle))
}

fn std_str_builder_len(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::str::builder_len expects 1 arg, got {}",
            args.len()
        ));
    }
    match &args[0] {
        Value::Array(a) => {
            let a = a.borrow();
            if let Some(Value::Bytes(buf)) = a.get(0) {
                Ok(Value::Int(buf.len() as i64))
            } else {
                Err("std::str::builder_len: corrupt builder handle".to_string())
            }
        }
        other => Err(format!(
            "std::str::builder_len: handle must be a builder, got {}",
            other.type_name()
        )),
    }
}

fn std_str_builder_finish(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::str::builder_finish expects 1 arg, got {}",
            args.len()
        ));
    }
    match &args[0] {
        Value::Array(a) => {
            let mut a = a.borrow_mut();
            if let Some(Value::Bytes(buf)) = a.get_mut(0) {
                let owned = std::mem::take(buf);
                let s = String::from_utf8_lossy(&owned).into_owned();
                Ok(Value::String(s))
            } else {
                Err("std::str::builder_finish: corrupt builder handle".to_string())
            }
        }
        other => Err(format!(
            "std::str::builder_finish: handle must be a builder, got {}",
            other.type_name()
        )),
    }
}

/// C10 (pond follow-up): interpreter parity for the binary-safe
/// builder. Same shape as std::str::builder_* but the chunk is
/// Bytes (binary-safe Vec<u8>) and finish returns Bytes, so
/// embedded NULs survive end-to-end. Handle representation
/// (one-element Array wrapping the accumulating Bytes buffer)
/// is shared with the str-builder — the only difference is
/// what append/finish do.
fn std_bytes_builder_new(args: &[Value]) -> Result<Value, String> {
    if !args.is_empty() {
        return Err(format!(
            "std::bytes::builder_new expects 0 args, got {}",
            args.len()
        ));
    }
    use std::cell::RefCell;
    use std::rc::Rc;
    let inner = Rc::new(RefCell::new(vec![Value::Bytes(Vec::new())]));
    Ok(Value::Array(inner))
}

fn std_bytes_builder_append(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err(format!(
            "std::bytes::builder_append expects 2 args, got {}",
            args.len()
        ));
    }
    let handle = match &args[0] {
        Value::Array(a) => a.clone(),
        other => {
            return Err(format!(
                "std::bytes::builder_append: handle must be a builder, got {}",
                other.type_name()
            ));
        }
    };
    let chunk = match &args[1] {
        Value::Bytes(b) => b.clone(),
        other => {
            return Err(format!(
                "std::bytes::builder_append: chunk must be Bytes, got {}",
                other.type_name()
            ));
        }
    };
    {
        let mut a = handle.borrow_mut();
        let buf = match a.get_mut(0) {
            Some(Value::Bytes(buf)) => buf,
            _ => {
                return Err(
                    "std::bytes::builder_append: corrupt builder handle"
                        .to_string(),
                )
            }
        };
        buf.extend_from_slice(&chunk);
    }
    Ok(Value::Array(handle))
}

fn std_bytes_builder_len(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::bytes::builder_len expects 1 arg, got {}",
            args.len()
        ));
    }
    match &args[0] {
        Value::Array(a) => {
            let a = a.borrow();
            if let Some(Value::Bytes(buf)) = a.get(0) {
                Ok(Value::Int(buf.len() as i64))
            } else {
                Err("std::bytes::builder_len: corrupt builder handle".to_string())
            }
        }
        other => Err(format!(
            "std::bytes::builder_len: handle must be a builder, got {}",
            other.type_name()
        )),
    }
}

fn std_bytes_builder_finish(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::bytes::builder_finish expects 1 arg, got {}",
            args.len()
        ));
    }
    match &args[0] {
        Value::Array(a) => {
            let mut a = a.borrow_mut();
            if let Some(Value::Bytes(buf)) = a.get_mut(0) {
                // No NUL trailer — Bytes is length-prefixed,
                // embedded NULs survive verbatim.
                let owned = std::mem::take(buf);
                Ok(Value::Bytes(owned))
            } else {
                Err("std::bytes::builder_finish: corrupt builder handle"
                    .to_string())
            }
        }
        other => Err(format!(
            "std::bytes::builder_finish: handle must be a builder, got {}",
            other.type_name()
        )),
    }
}

fn std_str_parse_float(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::str::parse_float expects 1 arg, got {}",
            args.len()
        ));
    }
    match &args[0] {
        Value::String(s) => match s.parse::<f64>() {
            Ok(v) => Ok(Value::Float(v)),
            // 2026-05-17 — fallible. Empty / non-numeric / partial
            // tail diverges via ParseError so the caller's `or`
            // clause sees the input.
            Err(_) => Ok(Value::FallibleErr(Box::new(parse_error_value(
                "parse_float", s,
            )))),
        },
        other => Err(format!(
            "std::str::parse_float expects String, got {}",
            other.type_name()
        )),
    }
}

fn std_str_can_parse_float(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::str::can_parse_float expects 1 arg, got {}",
            args.len()
        ));
    }
    match &args[0] {
        Value::String(s) => Ok(Value::Bool(
            !s.is_empty() && s.parse::<f64>().is_ok(),
        )),
        other => Err(format!(
            "std::str::can_parse_float expects String, got {}",
            other.type_name()
        )),
    }
}

fn std_text_base64_decode(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::text::base64::decode expects 1 arg, got {}",
            args.len()
        ));
    }
    let s = match &args[0] {
        Value::String(s) => s.clone(),
        other => {
            return Err(format!(
                "std::text::base64::decode expects String, got {}",
                other.type_name()
            ));
        }
    };
    Ok(Value::Bytes(base64_decode(&s)))
}

/// Standard-alphabet base64 decoder. Whitespace is ignored.
/// Non-alphabet, non-padding chars or wrong padding-aligned length
/// yields an empty Vec — the same "failure → empty" contract the
/// C runtime uses.
fn base64_decode(s: &str) -> Vec<u8> {
    fn decode_char(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut alpha_count = 0usize;
    let mut pad_count = 0usize;
    for &c in s.as_bytes() {
        if matches!(c, b' ' | b'\t' | b'\n' | b'\r') {
            continue;
        }
        if c == b'=' {
            pad_count += 1;
            continue;
        }
        if decode_char(c).is_none() {
            return Vec::new();
        }
        alpha_count += 1;
    }
    if (alpha_count + pad_count) % 4 != 0 || pad_count > 2 {
        return Vec::new();
    }
    let out_cap = (alpha_count + pad_count) / 4 * 3 - pad_count;
    let mut out = Vec::with_capacity(out_cap);
    let mut buf: u32 = 0;
    let mut bits = 0i32;
    for &c in s.as_bytes() {
        if matches!(c, b' ' | b'\t' | b'\n' | b'\r') {
            continue;
        }
        if c == b'=' {
            break;
        }
        let v = decode_char(c).unwrap();
        buf = (buf << 6) | (v as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    out
}

// C3 (pond follow-up): SHA-256 + HMAC-SHA256 interpreter parity.
// Hand-rolled per FIPS 180-4 / RFC 2104 — no crypto deps. Mirrors
// the C runtime's lotus_crypto_sha256 / lotus_crypto_hmac_sha256.

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5,
    0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
    0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
    0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
    0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
    0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn sha256_compute(msg: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let bit_len = (msg.len() as u64).wrapping_mul(8);
    let mut padded = msg.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7)
                ^ w[i - 15].rotate_right(18)
                ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17)
                ^ w[i - 2].rotate_right(19)
                ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let big_s1 =
                e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(big_s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let big_s0 =
                a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = big_s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn std_crypto_sha256(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::crypto::sha256 expects 1 arg, got {}",
            args.len()
        ));
    }
    let b = match &args[0] {
        Value::Bytes(b) => b.clone(),
        other => {
            return Err(format!(
                "std::crypto::sha256 expects Bytes, got {}",
                other.type_name()
            ));
        }
    };
    Ok(Value::Bytes(sha256_compute(&b).to_vec()))
}

fn std_crypto_hmac_sha256(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err(format!(
            "std::crypto::hmac_sha256 expects 2 args, got {}",
            args.len()
        ));
    }
    let key = match &args[0] {
        Value::Bytes(b) => b.clone(),
        other => {
            return Err(format!(
                "std::crypto::hmac_sha256: key must be Bytes, got {}",
                other.type_name()
            ));
        }
    };
    let msg = match &args[1] {
        Value::Bytes(b) => b.clone(),
        other => {
            return Err(format!(
                "std::crypto::hmac_sha256: msg must be Bytes, got {}",
                other.type_name()
            ));
        }
    };
    const B: usize = 64;
    let mut kprime = [0u8; B];
    if key.len() > B {
        let kh = sha256_compute(&key);
        kprime[..32].copy_from_slice(&kh);
    } else {
        kprime[..key.len()].copy_from_slice(&key);
    }
    let mut inner = Vec::with_capacity(B + msg.len());
    for i in 0..B {
        inner.push(kprime[i] ^ 0x36);
    }
    inner.extend_from_slice(&msg);
    let inner_hash = sha256_compute(&inner);
    let mut outer = Vec::with_capacity(B + 32);
    for i in 0..B {
        outer.push(kprime[i] ^ 0x5C);
    }
    outer.extend_from_slice(&inner_hash);
    Ok(Value::Bytes(sha256_compute(&outer).to_vec()))
}

// v1.x: stdin line reader. Mirrors lotus_stdin_read_line in
// the C runtime. Last-call status is stashed in a thread-local
// so the sibling getter can distinguish EOF from an empty
// input line.

thread_local! {
    static LAST_STDIN_STATUS: std::cell::Cell<i64> =
        std::cell::Cell::new(0);
}

fn std_io_stdin_read_line(args: &[Value]) -> Result<Value, String> {
    if !args.is_empty() {
        return Err(format!(
            "std::io::stdin::read_line takes 0 args, got {}",
            args.len()
        ));
    }
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(0) => {
            // EOF.
            LAST_STDIN_STATUS.with(|c| c.set(-1));
            Ok(Value::String(String::new()))
        }
        Ok(_) => {
            // Strip trailing \n (and optional \r before it).
            if line.ends_with('\n') {
                line.pop();
                if line.ends_with('\r') {
                    line.pop();
                }
            }
            LAST_STDIN_STATUS.with(|c| c.set(0));
            Ok(Value::String(line))
        }
        Err(_) => {
            LAST_STDIN_STATUS.with(|c| c.set(-2));
            Ok(Value::String(String::new()))
        }
    }
}

fn std_io_stdin_read_line_status(args: &[Value]) -> Result<Value, String> {
    if !args.is_empty() {
        return Err(format!(
            "std::io::stdin::read_line_status takes 0 args, got {}",
            args.len()
        ));
    }
    let s = LAST_STDIN_STATUS.with(|c| c.get());
    Ok(Value::Int(s))
}

// === env =====================================================

fn std_env_args_count(args: &[Value]) -> Result<Value, String> {
    if !args.is_empty() {
        return Err(format!(
            "std::env::args_count takes 0 args, got {}",
            args.len()
        ));
    }
    let n = USER_ARGS.with(|a| a.borrow().len()) as i64;
    Ok(Value::Int(n))
}

fn std_env_arg(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::env::arg takes 1 arg (index), got {}",
            args.len()
        ));
    }
    let i = match &args[0] {
        Value::Int(n) => *n,
        other => {
            return Err(format!(
                "std::env::arg: index must be Int, got {}",
                other.type_name()
            ))
        }
    };
    let v = USER_ARGS.with(|a| {
        let av = a.borrow();
        if i < 0 || (i as usize) >= av.len() {
            String::new()
        } else {
            av[i as usize].clone()
        }
    });
    Ok(Value::String(v))
}

/// 2026-05-16: `std::env::arg_or(idx, default)` — interp parity
/// for the codegen-side helper. Returns argv[idx] when present,
/// otherwise the default. Matches the (idx, default) shape
/// agents reach for.
fn std_env_arg_or(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err(format!(
            "std::env::arg_or takes 2 args (index, default), got {}",
            args.len()
        ));
    }
    let i = match &args[0] {
        Value::Int(n) => *n,
        other => return Err(format!(
            "std::env::arg_or: index must be Int, got {}",
            other.type_name()
        )),
    };
    let default = match &args[1] {
        Value::String(s) => s.clone(),
        other => return Err(format!(
            "std::env::arg_or: default must be String, got {}",
            other.type_name()
        )),
    };
    let v = USER_ARGS.with(|a| {
        let av = a.borrow();
        if i < 0 || (i as usize) >= av.len() {
            default
        } else {
            av[i as usize].clone()
        }
    });
    Ok(Value::String(v))
}

fn std_env_var(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::env::var takes 1 arg (name), got {}",
            args.len()
        ));
    }
    let name = match &args[0] {
        Value::String(s) => s,
        other => {
            return Err(format!(
                "std::env::var: name must be String, got {}",
                other.type_name()
            ))
        }
    };
    Ok(Value::String(std::env::var(name).unwrap_or_default()))
}

fn std_env_var_exists(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::env::var_exists takes 1 arg (name), got {}",
            args.len()
        ));
    }
    let name = match &args[0] {
        Value::String(s) => s,
        other => {
            return Err(format!(
                "std::env::var_exists: name must be String, got {}",
                other.type_name()
            ))
        }
    };
    // Codegen converts the i32 return to Bool at the var_exists
    // call site; mirror that here for interp/codegen parity.
    Ok(Value::Bool(std::env::var(name).is_ok()))
}

// === process =================================================

fn std_process_pid(args: &[Value]) -> Result<Value, String> {
    if !args.is_empty() {
        return Err(format!(
            "std::process::pid takes 0 args, got {}",
            args.len()
        ));
    }
    // SAFETY: getpid is async-signal-safe + always succeeds.
    let pid = unsafe { libc::getpid() } as i64;
    Ok(Value::Int(pid))
}

// === io::fs ==================================================

fn std_io_fs_read_file(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::io::fs::read_file takes 1 arg (path), got {}",
            args.len()
        ));
    }
    let path = match &args[0] {
        Value::String(s) => s,
        other => {
            return Err(format!(
                "std::io::fs::read_file: path must be String, got {}",
                other.type_name()
            ))
        }
    };
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Value::String(s)),
        Err(e) => Ok(Value::FallibleErr(Box::new(io_error_from_std(&e, path)))),
    }
}

fn std_io_fs_write_file(args: &[Value]) -> Result<Value, String> {
    fs_write(args, false, "write_file")
}

fn std_io_fs_write_file_append(args: &[Value]) -> Result<Value, String> {
    fs_write(args, true, "write_file_append")
}

fn fs_write(args: &[Value], append: bool, label: &str) -> Result<Value, String> {
    if args.len() != 2 {
        return Err(format!(
            "std::io::fs::{} takes 2 args (path, content), got {}",
            label,
            args.len()
        ));
    }
    let path = match &args[0] {
        Value::String(s) => s.clone(),
        other => {
            return Err(format!(
                "std::io::fs::{}: path must be String, got {}",
                label,
                other.type_name()
            ))
        }
    };
    let bytes: std::borrow::Cow<[u8]> = match &args[1] {
        Value::String(s) => std::borrow::Cow::Owned(s.as_bytes().to_vec()),
        Value::Bytes(b) => std::borrow::Cow::Borrowed(b),
        other => {
            return Err(format!(
                "std::io::fs::{}: content must be String or Bytes, got {}",
                label,
                other.type_name()
            ))
        }
    };
    match write_bytes_to_path(&path, &bytes, append) {
        Ok(()) => Ok(Value::Unit),
        Err(e) => Ok(Value::FallibleErr(Box::new(io_error_from_std(&e, &path)))),
    }
}

fn write_bytes_to_path(
    path: &str,
    bytes: &[u8],
    append: bool,
) -> Result<(), std::io::Error> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true);
    if append {
        opts.append(true);
    } else {
        opts.truncate(true);
    }
    let mut f = opts.open(path)?;
    f.write_all(bytes)?;
    Ok(())
}

fn std_io_fs_file_exists(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::io::fs::file_exists takes 1 arg (path), got {}",
            args.len()
        ));
    }
    let path = match &args[0] {
        Value::String(s) => s,
        other => {
            return Err(format!(
                "std::io::fs::file_exists: path must be String, got {}",
                other.type_name()
            ))
        }
    };
    Ok(Value::Bool(std::path::Path::new(path).exists()))
}

fn std_io_fs_file_size(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::io::fs::file_size takes 1 arg (path), got {}",
            args.len()
        ));
    }
    let path = match &args[0] {
        Value::String(s) => s,
        other => {
            return Err(format!(
                "std::io::fs::file_size: path must be String, got {}",
                other.type_name()
            ))
        }
    };
    match std::fs::metadata(path) {
        Ok(m) => Ok(Value::Int(m.len() as i64)),
        Err(e) => Ok(Value::FallibleErr(Box::new(io_error_from_std(&e, path)))),
    }
}

fn std_io_fs_mkdir(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::io::fs::mkdir takes 1 arg (path), got {}",
            args.len()
        ));
    }
    let path = match &args[0] {
        Value::String(s) => s,
        other => {
            return Err(format!(
                "std::io::fs::mkdir: path must be String, got {}",
                other.type_name()
            ))
        }
    };
    match std::fs::create_dir(path) {
        Ok(_) => Ok(Value::Unit),
        Err(e) => Ok(Value::FallibleErr(Box::new(io_error_from_std(&e, path)))),
    }
}

// C9 (pond/logfmt rotation): rename src → dst. Same diagnostic-
// path convention as codegen: the IoError.path field carries
// `dst` because the destination is the more diagnostic of the
// two on the common failure modes.
fn std_io_fs_rename(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err(format!(
            "std::io::fs::rename takes 2 args (src, dst), got {}",
            args.len()
        ));
    }
    let src = match &args[0] {
        Value::String(s) => s.clone(),
        other => {
            return Err(format!(
                "std::io::fs::rename: src must be String, got {}",
                other.type_name()
            ))
        }
    };
    let dst = match &args[1] {
        Value::String(s) => s.clone(),
        other => {
            return Err(format!(
                "std::io::fs::rename: dst must be String, got {}",
                other.type_name()
            ))
        }
    };
    match std::fs::rename(&src, &dst) {
        Ok(_) => Ok(Value::Unit),
        Err(e) => Ok(Value::FallibleErr(Box::new(io_error_from_std(&e, &dst)))),
    }
}

// C9 (pond/logfmt rotation): unlink path. POSIX unlink(2) on
// regular files / symlinks; EISDIR on a directory.
fn std_io_fs_unlink(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::io::fs::unlink takes 1 arg (path), got {}",
            args.len()
        ));
    }
    let path = match &args[0] {
        Value::String(s) => s,
        other => {
            return Err(format!(
                "std::io::fs::unlink: path must be String, got {}",
                other.type_name()
            ))
        }
    };
    match std::fs::remove_file(path) {
        Ok(_) => Ok(Value::Unit),
        Err(e) => Ok(Value::FallibleErr(Box::new(io_error_from_std(&e, path)))),
    }
}

// C9 (pond/agent/sandbox): race-free tempfile path allocator.
// Assembles prefix + "XXXXXX" + suffix, calls mkstemps(3) via
// libc, closes the fd, returns the path. Caller owns cleanup.
// IoError.path on failure is the assembled template.
fn std_io_fs_mktemp(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err(format!(
            "std::io::fs::mktemp takes 2 args (prefix, suffix), got {}",
            args.len()
        ));
    }
    let prefix = match &args[0] {
        Value::String(s) => s.as_str(),
        other => {
            return Err(format!(
                "std::io::fs::mktemp: prefix must be String, got {}",
                other.type_name()
            ))
        }
    };
    let suffix = match &args[1] {
        Value::String(s) => s.as_str(),
        other => {
            return Err(format!(
                "std::io::fs::mktemp: suffix must be String, got {}",
                other.type_name()
            ))
        }
    };
    let template = format!("{}XXXXXX{}", prefix, suffix);
    // mkstemps needs a writable C string with the XXXXXX template.
    let mut buf: Vec<u8> = template.as_bytes().to_vec();
    buf.push(0);
    let suffix_len = suffix.len() as libc::c_int;
    let fd = unsafe {
        libc::mkstemps(buf.as_mut_ptr() as *mut libc::c_char, suffix_len)
    };
    if fd < 0 {
        let e = std::io::Error::last_os_error();
        return Ok(Value::FallibleErr(Box::new(io_error_from_std(&e, &template))));
    }
    unsafe { libc::close(fd) };
    // mkstemps mutated buf in place — pull the path back out
    // (drop the trailing NUL).
    buf.pop();
    let path = match String::from_utf8(buf) {
        Ok(s) => s,
        Err(_) => {
            // Pathological — POSIX paths are bytes, the substituted
            // X's are alphanumeric, so as long as prefix+suffix are
            // valid UTF-8 the result is too. Map to invalid.
            let e = std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "mktemp produced non-UTF-8 path",
            );
            return Ok(Value::FallibleErr(Box::new(io_error_from_std(&e, &template))));
        }
    };
    Ok(Value::String(path))
}

// C4 (pond/crypto): cryptographically-strong random bytes.
// Mirrors the codegen C primitive `lotus_os_getrandom`:
//   - n <= 0       → empty Bytes, no error.
//   - n > 8192     → IoError(kind="invalid") (per-call cap).
//   - getrandom(2) syscall on Linux; falls back to /dev/urandom
//     on ENOSYS (older kernels) or non-Linux platforms.
const STD_OS_GETRANDOM_PER_CALL_MAX: i64 = 8192;
const STD_OS_GETRANDOM_LABEL: &str = "std::os::getrandom";

fn std_os_getrandom(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::os::getrandom takes 1 arg (n), got {}",
            args.len()
        ));
    }
    let n = match &args[0] {
        Value::Int(i) => *i,
        other => {
            return Err(format!(
                "std::os::getrandom: n must be Int, got {}",
                other.type_name()
            ))
        }
    };
    if n <= 0 {
        return Ok(Value::Bytes(Vec::new()));
    }
    if n > STD_OS_GETRANDOM_PER_CALL_MAX {
        let e = std::io::Error::from_raw_os_error(libc::EINVAL);
        return Ok(Value::FallibleErr(Box::new(io_error_from_std(
            &e,
            STD_OS_GETRANDOM_LABEL,
        ))));
    }
    let mut buf = vec![0u8; n as usize];

    // Step 1: try getrandom(2) directly via syscall. Loop on
    // EINTR + short reads. ENOSYS → fall through to urandom.
    #[cfg(target_os = "linux")]
    {
        let mut filled = 0usize;
        let mut syscall_unavailable = false;
        while filled < buf.len() {
            let r = unsafe {
                libc::syscall(
                    libc::SYS_getrandom,
                    buf[filled..].as_mut_ptr() as *mut libc::c_void,
                    (buf.len() - filled) as libc::size_t,
                    0u32 as libc::c_uint,
                )
            };
            if r >= 0 {
                filled += r as usize;
                continue;
            }
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            if err.raw_os_error() == Some(libc::ENOSYS) {
                // Kernel too old (pre-3.17). Reset and try urandom.
                syscall_unavailable = true;
                break;
            }
            return Ok(Value::FallibleErr(Box::new(io_error_from_std(
                &err,
                STD_OS_GETRANDOM_LABEL,
            ))));
        }
        if !syscall_unavailable {
            return Ok(Value::Bytes(buf));
        }
    }

    // Step 2: /dev/urandom fallback. Used on non-Linux platforms
    // and on Linux kernels too old to expose getrandom(2).
    use std::io::Read;
    let mut f = match std::fs::File::open("/dev/urandom") {
        Ok(f) => f,
        Err(e) => {
            return Ok(Value::FallibleErr(Box::new(io_error_from_std(
                &e,
                STD_OS_GETRANDOM_LABEL,
            ))))
        }
    };
    if let Err(e) = f.read_exact(&mut buf) {
        return Ok(Value::FallibleErr(Box::new(io_error_from_std(
            &e,
            STD_OS_GETRANDOM_LABEL,
        ))));
    }
    Ok(Value::Bytes(buf))
}

// === str parsing =============================================

fn std_str_parse_int(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::str::parse_int takes 1 arg, got {}",
            args.len()
        ));
    }
    let s = match &args[0] {
        Value::String(s) => s,
        other => {
            return Err(format!(
                "std::str::parse_int expects String, got {}",
                other.type_name()
            ))
        }
    };
    // 2026-05-17 — fallible. strtoll-ish parse: leading whitespace
    // + optional sign accepted; trailing garbage diverges via
    // ParseError so the caller's `or` clause sees the input.
    match s.trim().parse::<i64>() {
        Ok(n) => Ok(Value::Int(n)),
        Err(_) => Ok(Value::FallibleErr(Box::new(parse_error_value(
            "parse_int", s,
        )))),
    }
}

fn std_str_can_parse_int(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::str::can_parse_int takes 1 arg, got {}",
            args.len()
        ));
    }
    let s = match &args[0] {
        Value::String(s) => s,
        other => {
            return Err(format!(
                "std::str::can_parse_int expects String, got {}",
                other.type_name()
            ))
        }
    };
    Ok(Value::Bool(s.trim().parse::<i64>().is_ok()))
}

// === math ====================================================

fn float_arg(v: &Value, fn_name: &str) -> Result<f64, String> {
    match v {
        Value::Float(f) => Ok(*f),
        Value::Int(n) => Ok(*n as f64),
        other => Err(format!(
            "std::math::{} expects Float, got {}",
            fn_name,
            other.type_name()
        )),
    }
}

fn unary_math(args: &[Value], name: &str, f: fn(f64) -> f64) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::math::{} takes 1 arg, got {}",
            name,
            args.len()
        ));
    }
    Ok(Value::Float(f(float_arg(&args[0], name)?)))
}

fn std_math_sqrt(args: &[Value]) -> Result<Value, String> {
    unary_math(args, "sqrt", f64::sqrt)
}
fn std_math_exp(args: &[Value]) -> Result<Value, String> {
    unary_math(args, "exp", f64::exp)
}
fn std_math_log(args: &[Value]) -> Result<Value, String> {
    unary_math(args, "log", f64::ln)
}
fn std_math_floor(args: &[Value]) -> Result<Value, String> {
    unary_math(args, "floor", f64::floor)
}
fn std_math_ceil(args: &[Value]) -> Result<Value, String> {
    unary_math(args, "ceil", f64::ceil)
}

fn std_math_pow(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err(format!(
            "std::math::pow takes 2 args, got {}",
            args.len()
        ));
    }
    let b = float_arg(&args[0], "pow")?;
    let e = float_arg(&args[1], "pow")?;
    Ok(Value::Float(b.powf(e)))
}

// C8 (pond follow-up): IEEE 754 surface — interpreter parity for
// `std::math::{tanh, nan, is_nan, inf}`. Routes through Rust's
// f64 methods (tanh / NAN / INFINITY / is_nan) which match libm
// + the C runtime wrappers in lotus_arena.c.
fn std_math_tanh(args: &[Value]) -> Result<Value, String> {
    unary_math(args, "tanh", f64::tanh)
}

fn std_math_nan(args: &[Value]) -> Result<Value, String> {
    if !args.is_empty() {
        return Err(format!(
            "std::math::nan takes 0 args, got {}",
            args.len()
        ));
    }
    Ok(Value::Float(f64::NAN))
}

fn std_math_inf(args: &[Value]) -> Result<Value, String> {
    if !args.is_empty() {
        return Err(format!(
            "std::math::inf takes 0 args, got {}",
            args.len()
        ));
    }
    Ok(Value::Float(f64::INFINITY))
}

fn std_math_is_nan(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "std::math::is_nan takes 1 arg, got {}",
            args.len()
        ));
    }
    let f = float_arg(&args[0], "is_nan")?;
    Ok(Value::Bool(f.is_nan()))
}
