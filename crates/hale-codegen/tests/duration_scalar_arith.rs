//! Crumb batch-3 item 5 — Duration scalar arithmetic.
//!
//! A runtime-computed delay (`ms * 1ms` where ms is an Int at an
//! FFI boundary — a JS setTimeout's millis) had no direct
//! expression: `Int * Duration` was "binary op: incompatible
//! operand types", forcing an O(ms/100) tiered sleep loop.
//! Duration is i64 nanoseconds internally, so `Int * Duration`
//! (either order) and `Duration / Int` are plain integer ops.
//! `Duration * Duration` stays rejected (ns² has no meaning) —
//! now with a real diagnostic instead of the codegen catch-all.
//! Since U4 (GH #1076) the algebra of the stdlib's `Duration`, a
//! quantity like any, says all of this, and lowering emits the result
//! its operator row names.

#[path = "../../hale-types/tests/support/entries.rs"]
mod entries;
use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

#[test]
fn int_times_duration_scales_the_interval() {
    let src = r#"
        fn sleep_ms(ms: Int) {
            std::time::sleep(ms * 1ms);
        }
        fn main() {
            let t0 = std::time::monotonic();
            sleep_ms(50);
            // reversed operand order + scalar divide; a literal divisor
            // is a narrowing whose `or` says what becomes of the rest (D2)
            std::time::sleep(1ms * 10);
            std::time::sleep(100ms / 10 or floor);
            let dt = std::time::monotonic() - t0;
            // 50 + 10 + 10 = 70ms of computed sleeps; scheduling
            // jitter only ever adds. An order-of-magnitude bound
            // catches the failure modes (0ns from a dropped
            // multiply, ns-instead-of-ms scale confusion) without
            // being timing-flaky.
            if dt >= 70ms {
                if dt < 700ms {
                    println("scaled sleeps ok");
                }
            }
        }
    "#;
    let bin = harness::unique_bin(&format!("hale_dur_scalar_{}", std::process::id()));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("scaled sleeps ok"),
        "stdout: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn duration_times_duration_is_rejected_with_a_pointer() {
    let src = r#"
        fn main() {
            let d = 2ms * 3ms;
            std::time::sleep(d);
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let diags = entries::check_program(&program);
    // U4: the algebra's refusal, `Duration` being the stdlib's quantity.
    assert!(
        diags.iter().any(|d| d.message
            == "`Duration` * `Duration`: a product of two quantities is a quantity only when one is dimensionless \
                (a ratio); neither is"),
        "expected the Duration×Duration diagnostic; got {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

/// U4 (GH #1076): `Duration` and `Time` are the stdlib's declarations
/// and keep their representation class. A `Duration` prints its
/// nanoseconds as it always did (decision 8), a `Time` its instant; a
/// time literal of every unit of the stdlib's catalogue is that count of
/// nanoseconds; and the algebra's results lower as the class they are
/// (`Int * Duration` a `Duration`, `Time - Time` a `Duration`, `Time +
/// Duration` a `Time`), an operator's row saying so.
#[test]
fn the_time_types_print_and_compute_as_they_always_did() {
    let src = r#"
        fn main() {
            let t = `2026-05-08T12:00:00Z`;
            println(1500ms);
            println(t + 90s);
            println((t + 2min) - t);
            println(3 * 1h);
            println(1day / 4 or floor);
            println(to_string(250us) + " " + 7ns);
            let n = 4;
            println(n * 1ms + 1s);
            println(1day == 24h && 1h == 60min && 1min == 60s);
        }
    "#;
    let bin = harness::unique_bin(&format!("hale_dur_print_{}", std::process::id()));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let lines = [
        "1500000000ns",
        "2026-05-08T12:01:30Z",
        "120000000000ns",
        "10800000000000ns",
        "21600000000000ns",
        "250000ns 7ns",
        "1004000000ns",
        "true",
    ];
    assert_eq!(String::from_utf8_lossy(&out.stdout), lines.map(|l| format!("{l}\n")).concat());
}

/// U4's second correction (D2, decision 5): `Duration ÷ Duration` is
/// the `Int` quotient of the two counts; a literal divisor is a
/// narrowing its `or` rounds (`floor` toward minus infinity, `ceil`
/// toward plus); a runtime divisor is the integer division it always
/// was.
#[test]
fn a_duration_quotient_is_an_int_and_a_literal_divisor_rounds() {
    let src = r#"
        fn main() {
            let d = 1500ms;
            let e = 1ms;
            let n: Int = d / e;
            println(n);
            println(1h / 1min);
            let timeout = 7ns;
            println(timeout / 2 or floor);
            println(timeout / 2 or ceil);
            println((0ns - timeout) / 2 or floor);
            let k = 2;
            println(timeout / k);
        }
    "#;
    let bin = harness::unique_bin(&format!("hale_dur_quot_{}", std::process::id()));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let lines = ["1500", "60", "3ns", "4ns", "-4ns", "3ns"];
    assert_eq!(String::from_utf8_lossy(&out.stdout), lines.map(|l| format!("{l}\n")).concat());
}
