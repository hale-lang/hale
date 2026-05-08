//! Integration test: each runnable example program executes to
//! completion with the v0 interpreter.
//!
//! "Runnable" means: single-binary projects whose surface stays
//! within v0's executable subset (no bus router, no scheduler).
//! Projects that hit deferred features (05-bus subscriptions,
//! trellis-pair multi-binary) are listed as deferred.

use std::path::PathBuf;

use lotus_runtime::run_program;

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
fn modes_runs() {
    assert_eq!(parse_and_run("04-modes/main.lt").unwrap(), 0);
}

#[test]
fn bus_runs() {
    assert_eq!(parse_and_run("05-bus/main.lt").unwrap(), 0);
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
