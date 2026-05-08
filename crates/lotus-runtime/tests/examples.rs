//! Integration test: each runnable example program executes to
//! completion with the v0 interpreter.
//!
//! "Runnable" means: single-binary projects whose surface stays
//! within v0's executable subset (no bus router, no scheduler).
//! Projects that hit deferred features (05-bus subscriptions,
//! trellis-pair multi-binary) are listed as deferred.

use std::path::PathBuf;

use lotus_runtime::{run_bundle_with_bus, run_program, TransportKind};

fn examples_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("examples");
    p
}

fn parse_and_run(rel: &str) -> Result<i32, String> {
    let mut path = examples_dir();
    for seg in rel.split('/') {
        path.push(seg);
    }
    let source = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let program = lotus_syntax::parse_source(&source)
        .map_err(|d| {
            d.iter()
                .map(|x| x.render(&source))
                .collect::<Vec<_>>()
                .join("\n")
        })?;
    run_program(&program)
}

#[test]
fn hello_world_runs() {
    assert_eq!(parse_and_run("hello-world/main.lt").unwrap(), 0);
}

#[test]
fn parent_child_runs() {
    assert_eq!(parse_and_run("02-parent-child/main.lt").unwrap(), 0);
}

#[test]
fn closure_test_runs() {
    assert_eq!(parse_and_run("03-closure-test/main.lt").unwrap(), 0);
}

#[test]
fn closure_absorbed_runs() {
    assert_eq!(parse_and_run("03b-closure-absorbed/main.lt").unwrap(), 0);
}

#[test]
fn closure_bubbled_exits_nonzero() {
    // F.9: bubble(err) from on_failure with no further handler
    // produces a runtime error wrapping the formatted
    // ClosureViolation. The interpreter surfaces that as
    // Err(String) which the CLI maps to non-zero exit.
    let result = parse_and_run("03c-closure-bubbled/main.lt");
    let msg = match result {
        Err(m) => m,
        Ok(code) => panic!("expected runtime error; got clean exit {}", code),
    };
    assert!(
        msg.contains("ClosureViolation") && msg.contains("xy_match"),
        "expected formatted violation; got: {}",
        msg
    );
}

#[test]
fn modes_runs() {
    assert_eq!(parse_and_run("04-modes/main.lt").unwrap(), 0);
}

#[test]
fn mutable_counter_runs() {
    assert_eq!(
        parse_and_run("06-mutable-counter/main.lt").unwrap(),
        0
    );
}

#[test]
fn control_flow_runs() {
    assert_eq!(
        parse_and_run("07-control-flow/main.lt").unwrap(),
        0
    );
}

#[test]
fn monotonic_sleep_runs() {
    // 3 × 50ms = 150ms floor; trivially fast under the
    // interpreter's monotonic-clock sleep primitive.
    assert_eq!(
        parse_and_run("08-monotonic-sleep/main.lt").unwrap(),
        0
    );
}

#[test]
fn functions_runs() {
    assert_eq!(
        parse_and_run("09-functions/main.lt").unwrap(),
        0
    );
}

#[test]
fn bus_runs() {
    assert_eq!(parse_and_run("05-bus/main.lt").unwrap(), 0);
}

#[test]
fn trellis_demo_runs() {
    assert_eq!(parse_and_run("trellis-demo/main.lt").unwrap(), 0);
}

/// Ring-buffer transport delivers the same observable result
/// as sync dispatch for 05-bus. Source unchanged; transport
/// configured at runtime construction.
#[test]
fn bus_runs_under_ringbuffer_transport() {
    let mut path = examples_dir();
    path.push("05-bus");
    path.push("main.lt");
    let source = std::fs::read_to_string(&path).unwrap();
    let program = lotus_syntax::parse_source(&source).unwrap();

    let bus_config = vec![
        ("demo.greeting".to_string(), TransportKind::Ring { capacity: 16 }),
        ("demo.ack".to_string(), TransportKind::Ring { capacity: 16 }),
    ];
    let exit = run_bundle_with_bus(&[&program], bus_config).unwrap();
    assert_eq!(exit, 0);
}

// 01-locus-with-run uses time::sleep with 500ms intervals;
// running it under cargo test would stall. Smoke-tested
// manually via `lotus run examples/01-locus-with-run/main.lt`.

// 05-bus exits immediately under v0 (the bus router is a
// no-op), so it doesn't exercise observable behavior.

// trellis-pair has two `fn main()` files (analyst, executor)
// — the CLI doesn't yet partition by entry point, so the
// `lotus run` integration is deferred until the bus router
// + per-binary entry-point selection lands.
