//! Interest-based ownership, artifact #3 — cross-pool bubbling.
//!
//! A locus `I` instantiated inside a consumer `B` that runs on a
//! DIFFERENT pool/thread than its accepting ancestor `A` (a
//! `SingletonConst` — a `main locus`). Because arenas are per-thread,
//! `I` cannot be born on `B`'s thread; instead its params are marshaled
//! and a create cell is posted to `A`'s thread, where a synthesized
//! dispatcher births `I` in `A`'s arena and stitches it (`accept` +
//! `children_push`). Async fire-and-forget: only a bare `I { ... };`
//! statement is legal.
//!
//! These tests exercise:
//!   * the cross-pool bubble itself — a `Driver` placed on a cooperative
//!     worker pool spawns Ships that collect on the main-thread `World`;
//!   * value-round-trip after a quiesce (sleep) barrier;
//!   * the fire-and-forget rejection — `let s = Ship { };` at a
//!     cross-pool site is a compile error;
//!   * the disable flag — `LOTUS_NO_OWNERSHIP_BUBBLE=1` empties the plan
//!     so the Ships stay transient (the differential control arm).
//!
//! Run under `LOTUS_ASAN=1 --include-ignored` to prove the Ship is
//! reclaimed exactly once by World (no leak / UAF / double-free).

use std::process::Command;

use hale_codegen::build_executable;

fn build_named(name: &str, src: &str) -> Result<std::path::PathBuf, String> {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "hale_test_xpool_bubble_{}_{}",
        name,
        std::process::id()
    ));
    build_executable(&program, &bin).map_err(|e| format!("{:?}", e))?;
    Ok(bin)
}

fn run(bin: &std::path::PathBuf) -> String {
    let out = Command::new(bin).output().expect("run hale");
    let _ = std::fs::remove_file(bin);
    assert!(
        out.status.success(),
        "non-zero exit: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// `Driver` is placed on a cooperative worker pool; its run() spawns
// three Ships. Each `Ship { ... };` resolves to `Ancestor(World)` with
// `OwnerKind::SingletonConst` (World is the `main locus`) and
// `EdgeClass::CrossPool` (Driver on `workers`, World on the main
// thread) — the #3 site. The Ships are marshaled and posted to World's
// thread, where World collects them. World.run() sleeps to quiesce (the
// main-thread sleep drains the cooperative bus queue), then reports.
const XPOOL_SRC: &str = r#"
    locus Ship {
        params { hull: Int = 0; }
        contract { expose hull: Int; }
    }
    locus Driver {
        run() {
            Ship { hull: 7 };
            Ship { hull: 15 };
            Ship { hull: 20 };
        }
    }
    main locus World {
        params {
            driver: Driver = Driver { };
        }
        placement {
            driver: cooperative(pool = workers);
        }
        contract { consume hull: Int; }
        accept(s: Ship) { }
        mode harmonic() -> Int {
            let mut n: Int = 0;
            for child in self.children { n = n + 1; }
            return n;
        }
        mode bulk() -> Int {
            let mut t: Int = 0;
            for child in self.children { t = t + child.hull; }
            return t;
        }
        run() {
            std::time::sleep(300ms);
            println("count=", self.harmonic());
            println("total=", self.bulk());
        }
    }
    fn main() { World { }; }
"#;

#[test]
fn world_collects_crosspool_bubbled_ships() {
    let bin = build_named("collect", XPOOL_SRC).expect("build");
    let stdout = run(&bin);
    // All three Ships bubbled cross-pool to World's __children.
    assert!(
        stdout.contains("count=3"),
        "expected World to collect all three cross-pool Ships; got: {:?}",
        stdout
    );
    // Values round-trip through the marshaled payload (7+15+20=42).
    assert!(
        stdout.contains("total=42"),
        "expected cross-pool Ship values to round-trip (7+15+20=42); got: {:?}",
        stdout
    );
}

#[test]
fn disable_flag_reverts_to_transient() {
    // Same program, bubble gated off: the Ships stay transient
    // (dissolved at Driver.run()'s scope exit on the worker thread), so
    // World collects nothing.
    std::env::set_var("LOTUS_NO_OWNERSHIP_BUBBLE", "1");
    let bin = build_named("disabled", XPOOL_SRC).expect("build");
    std::env::remove_var("LOTUS_NO_OWNERSHIP_BUBBLE");
    let stdout = run(&bin);
    assert!(
        stdout.contains("count=0") && stdout.contains("total=0"),
        "expected no children with cross-pool bubbling disabled; got: {:?}",
        stdout
    );
}

#[test]
fn fire_and_forget_value_use_is_rejected() {
    // A cross-pool `Ship { }` used as a VALUE (let-binding) is a compile
    // error — the instance is born on World's thread and can't be used
    // on the consumer's.
    let src = r#"
        locus Ship {
            params { hull: Int = 0; }
            contract { expose hull: Int; }
        }
        locus Driver {
            run() {
                let s = Ship { hull: 7 };
            }
        }
        main locus World {
            params {
                driver: Driver = Driver { };
            }
            placement {
                driver: cooperative(pool = workers);
            }
            accept(s: Ship) { }
            run() { std::time::sleep(50ms); }
        }
        fn main() { World { }; }
    "#;
    let err = build_named("valueuse", src).expect_err(
        "a cross-pool value-use must fail to build",
    );
    assert!(
        err.contains("fire-and-forget"),
        "expected a fire-and-forget diagnostic; got: {}",
        err
    );
}
