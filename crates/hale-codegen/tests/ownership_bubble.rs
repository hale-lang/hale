//! Interest-based ownership, artifact #2 — singleton-owner, same-tower
//! bubbling.
//!
//! A locus `I` instantiated deep in a tree — inside an intermediary `B`
//! that does NOT itself `accept(I)` — stitches to the nearest accepting
//! ancestor `A` *when* `A` is a provably-unique instance (a `main locus`
//! / `@export` locus, `OwnerKind::SingletonConst`) on the same OS thread
//! (`EdgeClass::SameTower`). The bubbled child is allocated in `A`'s
//! arena, `A.accept(A, I)` fires, and `I` is appended to `A.__children[]`
//! so `A`'s dissolve cascade reclaims it exactly once.
//!
//! These tests exercise:
//!   * the bubble itself — `World` (a `main locus`) collects Ships born
//!     inside an intermediary `Yard` that does not accept Ship;
//!   * the SelfOwned control — `World` births Ship *directly* and still
//!     accepts it (today's path, unchanged);
//!   * inertness of the non-singleton-ancestor case — a plain (non-main)
//!     `Fleet` accepting Ship via an intermediary stays transient (that's
//!     artifacts #2b/#3, not this one);
//!   * the disable flag — `LOTUS_NO_OWNERSHIP_BUBBLE=1` empties the plan,
//!     so the bubble program falls back to transient (the differential
//!     control arm).
//!
//! Run under `LOTUS_ASAN=1` to prove the bubbled child is reclaimed
//! exactly once (no leak, no use-after-free): `build_executable` reads
//! the flag at codegen time, so the emitted binary is ASan-instrumented
//! and any leak/UAF fails `run`'s success assertion.

use std::process::Command;

use hale_codegen::build_executable;

fn build_named(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_ownership_bubble_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
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

/// The intermediary `Yard` births two Ships but declares no `accept`.
/// They must bubble to the `main locus World`, which accepts Ship — so
/// `World.__children` sees both, and reading `child.hull` back through
/// them round-trips the values (7 + 35 = 42).
const BUBBLE_SRC: &str = r#"
    locus Ship {
        params { hull: Int = 0; }
        contract { expose hull: Int; }
    }
    locus Yard {
        run() {
            Ship { hull: 7 };
            Ship { hull: 35 };
        }
    }
    main locus World {
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
            Yard { };
            println("count=", self.harmonic());
            println("total=", self.bulk());
        }
    }
    fn main() { World { }; }
"#;

#[test]
fn world_collects_bubbled_ship() {
    let bin = build_named("collect", BUBBLE_SRC);
    let stdout = run(&bin);
    // Both Ships bubbled up to World's __children.
    assert!(
        stdout.contains("count=2"),
        "expected World to collect both bubbled Ships; got: {:?}",
        stdout
    );
    // Values round-trip through the bubbled children (7 + 35).
    assert!(
        stdout.contains("total=42"),
        "expected bubbled Ship values to round-trip (7+35=42); got: {:?}",
        stdout
    );
}

#[test]
fn disable_flag_reverts_to_transient() {
    // Same program, but the bubble is gated off. The Ships stay
    // transient (dissolved at Yard.run()'s scope exit), so World
    // collects nothing — proving the flag empties the ownership plan and
    // the emit is inert without it. Env is process-global; the crate's
    // tests run serial (`--test-threads=1`), and we scope the var to the
    // OFF build only.
    std::env::set_var("LOTUS_NO_OWNERSHIP_BUBBLE", "1");
    let bin = build_named("disabled", BUBBLE_SRC);
    std::env::remove_var("LOTUS_NO_OWNERSHIP_BUBBLE");
    let stdout = run(&bin);
    assert!(
        stdout.contains("count=0"),
        "expected no children with bubbling disabled; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("total=0"),
        "expected empty projection with bubbling disabled; got: {:?}",
        stdout
    );
}

#[test]
fn self_owned_control_still_works() {
    // Control: the main locus births Ship DIRECTLY and accepts it — the
    // SelfOwned path, which this artifact must leave byte-identical.
    let src = r#"
        locus Ship {
            params { hull: Int = 0; }
            contract { expose hull: Int; }
        }
        main locus World {
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
                Ship { hull: 7 };
                Ship { hull: 35 };
                println("count=", self.harmonic());
                println("total=", self.bulk());
            }
        }
        fn main() { World { }; }
    "#;
    let bin = build_named("selfowned", src);
    let stdout = run(&bin);
    assert!(
        stdout.contains("count=2") && stdout.contains("total=42"),
        "SelfOwned direct-accept must still collect both Ships; got: {:?}",
        stdout
    );
}

#[test]
fn non_singleton_ancestor_now_bubbles_via_threading() {
    // A plain (non-`main`) `Fleet` accepts Ship but reaches it only
    // through an intermediary `Yard`. The site resolves to
    // `Ancestor(Fleet)` with `OwnerKind::Ancestor` (NOT SingletonConst).
    // Under #2 this was inert (Fleet's pointer is not a constant); as of
    // #2b the pointer is THREADED through `Yard.__owner_for_Ship`, so
    // Fleet now collects both bubbled Ships. (Was
    // `non_singleton_ancestor_stays_transient` in the #2-only world —
    // #2b is exactly the artifact that makes this case work. Instance
    // isolation across MULTIPLE Fleets is proved in
    // `ownership_bubble_multi.rs`.)
    let src = r#"
        locus Ship {
            params { hull: Int = 0; }
            contract { expose hull: Int; }
        }
        locus Yard {
            run() { Ship { hull: 7 }; Ship { hull: 35 }; }
        }
        locus Fleet {
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
                Yard { };
                println("fleet_count=", self.harmonic());
                println("fleet_total=", self.bulk());
            }
        }
        main locus World {
            run() { Fleet { }; }
        }
        fn main() { World { }; }
    "#;
    let bin = build_named("nonsingleton", src);
    let stdout = run(&bin);
    assert!(
        stdout.contains("fleet_count=2") && stdout.contains("fleet_total=42"),
        "a non-singleton accepting ancestor must bubble via #2b threading \
         (collects both Ships, 7+35=42); got: {:?}",
        stdout
    );
}
