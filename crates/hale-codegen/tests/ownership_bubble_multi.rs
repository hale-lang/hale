//! Interest-based ownership, artifact #2b — NON-singleton owner
//! threading (same-tower).
//!
//! #2 bubbled a child to a *singleton* ancestor by folding the owner
//! pointer through a global constant. #2b generalizes to an owner with
//! MULTIPLE instances: the pointer is threaded down the birth chain via
//! hidden per-locus `__owner_for_<I>` fields, so two owner instances each
//! collect the children born in THEIR OWN subtree — never crossing.
//!
//! The prize is **instance isolation**: a global singleton pointer cannot
//! distinguish two `World`s; a threaded per-instance pointer can. These
//! tests exercise:
//!   * instance isolation — two plain (non-singleton) `World`s, each with
//!     its own intermediary `Yard` (which does NOT accept `Ship`), each
//!     spawning distinct-valued Ships; each World collects ONLY its own;
//!   * threading depth — a 2-intermediary chain `World -> A -> B -> Ship`
//!     still stitches to the right World;
//!   * null / transient — a `Ship` born outside any World's subtree stays
//!     transient (no owner, not collected, no crash);
//!   * the disable flag — `LOTUS_NO_OWNERSHIP_BUBBLE=1` empties the plan
//!     AND the forwarding fields, so both Worlds collect nothing.
//!
//! Run under `LOTUS_ASAN=1 --include-ignored` to prove each World
//! reclaims its own Ships exactly once (no leak, no UAF, no double-free
//! across the two Worlds).

use std::process::Command;

use hale_codegen::build_executable;

fn build_named(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_ownership_bubble_multi_{}", name));
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

/// Two plain `World`s (NOT a `main locus` — instantiated twice under the
/// `Root`), each with its own `Yard` that does NOT accept `Ship`. Each
/// Yard spawns two Ships whose hulls key off the owner's `tag`, so the
/// per-World sums are disjoint. Instance isolation ⇒ world 100 collects
/// {101,102} (total 203) and world 200 collects {201,202} (total 403);
/// neither sees the other's Ships.
const ISOLATION_SRC: &str = r#"
    locus Ship {
        params { hull: Int = 0; }
        contract { expose hull: Int; }
    }
    locus Yard {
        params { base: Int = 0; }
        run() {
            Ship { hull: self.base + 1 };
            Ship { hull: self.base + 2 };
        }
    }
    locus World {
        params { tag: Int = 0; }
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
            Yard { base: self.tag };
            println("w", self.tag, " count=", self.harmonic(), " total=", self.bulk());
        }
    }
    main locus Root {
        run() {
            World { tag: 100 };
            World { tag: 200 };
        }
    }
    fn main() { Root { }; }
"#;


// Shares the cross-process bubble lock discipline with the sibling
// bubble suites (see ownership_bubble.rs for the why): concurrent
// bubble-program binaries starve each other's threaded delivery.
struct BubbleLock(std::path::PathBuf);
impl Drop for BubbleLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn bubble_lock() -> BubbleLock {
    let path = std::env::temp_dir().join("hale_bubble_suite.lock");
    let start = std::time::Instant::now();
    loop {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return BubbleLock(path),
            Err(_) => {
                if let Ok(md) = std::fs::metadata(&path) {
                    if let Ok(age) = md.modified().and_then(|m| {
                        std::time::SystemTime::now()
                            .duration_since(m)
                            .map_err(std::io::Error::other)
                    }) {
                        if age.as_secs() > 120 {
                            let _ = std::fs::remove_file(&path);
                            continue;
                        }
                    }
                }
                assert!(
                    start.elapsed().as_secs() < 300,
                    "bubble lock timed out"
                );
                std::thread::sleep(
                    std::time::Duration::from_millis(100),
                );
            }
        }
    }
}

#[test]
fn two_worlds_collect_only_their_own_ships() {
    let _lock = bubble_lock();
    let bin = build_named("isolation", ISOLATION_SRC);
    let stdout = run(&bin);
    // THE prize: each World collects exactly its own two Ships, summed
    // disjointly. A global singleton pointer would give one World all
    // four (or the wrong sums); threading gives each its own subtree.
    assert!(
        stdout.contains("w100 count=2 total=203"),
        "world 100 must collect ONLY its own Ships (101+102=203); got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("w200 count=2 total=403"),
        "world 200 must collect ONLY its own Ships (201+202=403); got: {:?}",
        stdout
    );
}

#[test]
fn disable_flag_reverts_both_worlds_to_transient() {
    let _lock = bubble_lock();
    // Control arm: gate #2b off. The Ships stay transient (dissolved at
    // Yard.run()'s scope exit), so BOTH Worlds collect nothing — proving
    // the flag empties the non-singleton plan + the forwarding fields and
    // the emit is inert without it.
    std::env::set_var("LOTUS_NO_OWNERSHIP_BUBBLE", "1");
    let bin = build_named("isolation_off", ISOLATION_SRC);
    std::env::remove_var("LOTUS_NO_OWNERSHIP_BUBBLE");
    let stdout = run(&bin);
    assert!(
        stdout.contains("w100 count=0 total=0")
            && stdout.contains("w200 count=0 total=0"),
        "with #2b disabled both Worlds collect nothing; got: {:?}",
        stdout
    );
}

#[test]
fn threading_depth_two_intermediaries() {
    let _lock = bubble_lock();
    // World -> A -> B -> Ship{}. Neither A nor B accepts Ship; the owner
    // pointer is forwarded through BOTH intermediaries' `__owner_for_Ship`
    // fields and still stitches to the correct World instance.
    let src = r#"
        locus Ship {
            params { hull: Int = 0; }
            contract { expose hull: Int; }
        }
        locus Bhold {
            params { base: Int = 0; }
            run() { Ship { hull: self.base + 1 }; }
        }
        locus Ahold {
            params { base: Int = 0; }
            run() { Bhold { base: self.base }; }
        }
        locus World {
            params { tag: Int = 0; }
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
                Ahold { base: self.tag };
                println("w", self.tag, " count=", self.harmonic(), " total=", self.bulk());
            }
        }
        main locus Root {
            run() {
                World { tag: 10 };
                World { tag: 20 };
            }
        }
        fn main() { Root { }; }
    "#;
    let bin = build_named("depth", src);
    let stdout = run(&bin);
    assert!(
        stdout.contains("w10 count=1 total=11"),
        "world 10: 2-deep chain must stitch its Ship (11); got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("w20 count=1 total=21"),
        "world 20: 2-deep chain must stitch its Ship (21); got: {:?}",
        stdout
    );
}

#[test]
fn ship_outside_any_world_stays_transient() {
    let _lock = bubble_lock();
    // A `Depot` born directly under `Root` spawns a Ship with a sentinel
    // hull (999). Depot has no World ancestor → the site resolves to
    // Orphan → it is NOT in the plan, carries no `__owner_for_Ship`, and
    // the Ship stays transient (not collected). The one real `World`
    // still collects its own Yard's Ships. No crash, no cross-collection.
    let src = r#"
        locus Ship {
            params { hull: Int = 0; }
            contract { expose hull: Int; }
        }
        locus Yard {
            params { base: Int = 0; }
            run() { Ship { hull: self.base + 1 }; }
        }
        locus Depot {
            run() { Ship { hull: 999 }; }
        }
        locus World {
            params { tag: Int = 0; }
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
                Yard { base: self.tag };
                println("w", self.tag, " count=", self.harmonic(), " total=", self.bulk());
            }
        }
        main locus Root {
            run() {
                World { tag: 50 };
                Depot { };
                println("done");
            }
        }
        fn main() { Root { }; }
    "#;
    let bin = build_named("transient", src);
    let stdout = run(&bin);
    // The World collected only its own Ship (51); the Depot's sentinel
    // Ship (999) is NOT collected anywhere.
    assert!(
        stdout.contains("w50 count=1 total=51"),
        "world 50 must collect only its own Ship (51); got: {:?}",
        stdout
    );
    assert!(
        !stdout.contains("999"),
        "the transient Depot Ship (999) must not be collected; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("done"),
        "program must run to completion (no crash on the transient Ship); \
         got: {:?}",
        stdout
    );
}
