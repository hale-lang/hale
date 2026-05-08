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

/// Sleep `ns` nanoseconds on CLOCK_MONOTONIC, retrying on EINTR.
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
        let r = unsafe {
            libc::clock_nanosleep(
                libc::CLOCK_MONOTONIC,
                0,
                &req,
                &mut rem,
            )
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

pub fn resolve_path(segments: &[&str]) -> Option<Value> {
    match segments {
        ["time", "sleep"] => Some(Value::Builtin(BuiltinRef {
            name: "time::sleep",
            func: Rc::new(time_sleep),
        })),
        _ => None,
    }
}
