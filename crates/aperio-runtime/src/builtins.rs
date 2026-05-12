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

use crate::value::{BuiltinRef, Value};

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
        _ => None,
    }
}
