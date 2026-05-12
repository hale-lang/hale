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
        ["std", "str", "replace"] => Some(Value::Builtin(BuiltinRef {
            name: "std::str::replace",
            func: Rc::new(std_str_replace),
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
        _ => None,
    }
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
    let mut a = handle.borrow_mut();
    if let Some(Value::Bytes(buf)) = a.get_mut(0) {
        buf.extend_from_slice(chunk.as_bytes());
        Ok(Value::Unit)
    } else {
        Err("std::str::builder_append: corrupt builder handle".to_string())
    }
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
            // Match codegen contract: empty / non-numeric / partial
            // tail returns 0.0 rather than an error, so callers can
            // gate on can_parse_float and use a defaulting shape.
            Err(_) => Ok(Value::Float(0.0)),
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
