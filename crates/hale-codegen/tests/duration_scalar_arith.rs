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

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

#[test]
fn int_times_duration_scales_the_interval() {
    let src = r#"
        fn sleep_ms(ms: Int) {
            std::time::sleep(ms * 1ms);
        }
        fn main() {
            let t0 = std::time::monotonic();
            sleep_ms(50);
            // reversed operand order + scalar divide
            std::time::sleep(1ms * 10);
            std::time::sleep(100ms / 10);
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
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_dur_scalar_{}", std::process::id()));
    build_executable(&program, &bin).expect("build");
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
    let diags = hale_types::check_program(&program);
    assert!(
        diags.iter().any(|d| d
            .message
            .contains("cannot be multiplied or divided by another")),
        "expected the Duration×Duration diagnostic; got {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}
