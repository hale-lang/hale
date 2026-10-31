//! F.40 phase 3, C1, a classified correction: an explicit `sync = none`
//! takes no lock and is not one.
//!
//! Three readers ask whether a `@form` map synchronizes, as one
//! question: the effects certificate engine (`@no_block` refuses a call
//! that can take a lock), the model's `depends` law (a held form is an
//! input channel outside the bus graph) and the instance-aliasing rule
//! (a field behind a discipline is not unsynchronized state). They
//! asked "was a `sync =` argument written", so a `sync = none` map
//! counted as a lock: a `@no_block` method reading it and an
//! `@effects(depends: …)` locus holding it were refused, and one
//! instance holding it, shared across two pools, got no warning. They
//! now ask `safe_for_cross_domain_access` alone, as the cross-pool
//! check does. Each test pins one program under `sync = none`, a map
//! inference leaves unsynchronized (unconfigured, one pool: its answers
//! do not move) and `sync = serialized` (the control that still
//! synchronizes).

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hale_sync_none_readers_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn check(root: &Path, sync: &str, program: &str) -> (bool, String) {
    let app = root.join("app.hl");
    std::fs::write(&app, program.replace("@form(hashmap)", &format!("@form(hashmap{sync})"))).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&app)
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    (
        out.status.success(),
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
    )
}

/// A `@no_block` method reading a map.
const NO_BLOCK_READS: &str = "\
type Entry { k: Int; v: Int; }

@form(hashmap)
locus Registry {
    capacity { pool entries of Entry indexed_by k; }
}

locus Reader {
    params { reg: Registry = Registry { }; }
    @no_block fn size() -> Int { return self.reg.len(); }
}

main locus App {
    params { r: Reader = Reader { }; }
    run() { let n = self.r.size(); }
}

fn main() { App { }; }
";

const LOCK_WAIT: &str = "acquiring its lock can wait on another thread";

#[test]
fn a_no_block_method_may_read_a_sync_none_map() {
    let root = scratch("no_block_none");
    let (ok, out) = check(&root, ", sync = none", NO_BLOCK_READS);
    assert!(ok && !out.contains(LOCK_WAIT), "a sync = none map takes no lock: {out}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_no_block_method_still_may_read_a_map_inference_left_unsynchronized() {
    let root = scratch("no_block_plain");
    let (ok, out) = check(&root, "", NO_BLOCK_READS);
    assert!(ok && !out.contains(LOCK_WAIT), "{out}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_no_block_method_is_still_refused_a_serialized_map() {
    let root = scratch("no_block_serialized");
    let (ok, out) = check(&root, ", sync = serialized", NO_BLOCK_READS);
    assert!(!ok && out.contains(LOCK_WAIT), "{out}");
    let _ = std::fs::remove_dir_all(&root);
}

/// A locus declaring `depends:` and holding a map.
const DEPENDS_HOLDS: &str = "\
type Act { mag: Float; pos: Int; }
type Entry { k: Int; v: Int; }
topic Recalled { payload: Act; subject: \"recalled\"; }

@form(hashmap)
locus Registry {
    capacity { pool entries of Entry indexed_by k; }
}

@effects(depends: {Recalled})
locus Carry {
    bus { subscribe Recalled as on_recalled; }
    params { reg: Registry = Registry { }; recalled: Float = 0.0; }
    fn on_recalled(a: Act) { self.recalled = a.mag; }
}

main locus App {
    params { c: Carry = Carry { }; }
}

fn main() { App { }; }
";

const SHARED_STATE: &str = "shared state another pool can write";

#[test]
fn a_depends_locus_may_hold_a_sync_none_map() {
    let root = scratch("depends_none");
    let (ok, out) = check(&root, ", sync = none", DEPENDS_HOLDS);
    assert!(ok && !out.contains(SHARED_STATE), "a sync = none map is not shared state: {out}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_depends_locus_still_may_hold_a_map_inference_left_unsynchronized() {
    let root = scratch("depends_plain");
    let (ok, out) = check(&root, "", DEPENDS_HOLDS);
    assert!(ok && !out.contains(SHARED_STATE), "{out}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_depends_locus_is_still_refused_a_serialized_map() {
    let root = scratch("depends_serialized");
    let (ok, out) = check(&root, ", sync = serialized", DEPENDS_HOLDS);
    assert!(!ok && out.contains(SHARED_STATE), "{out}");
    let _ = std::fs::remove_dir_all(&root);
}

/// One instance holding a map, reached by two main-locus fields placed
/// on different pools.
const ALIASED_HOLDS: &str = "\
type Entry { k: Int; v: Int; }

@form(hashmap)
locus Registry {
    capacity { pool entries of Entry indexed_by k; }
}

locus Shared { params { reg: Registry = Registry { }; } }
locus A { params { s: Shared = Shared { }; } run() { } }
locus B { params { s: Shared = Shared { }; } run() { } }

main locus App {
    params { sh: Shared = Shared { };
             a: A = A { s: self.sh };
             b: B = B { s: self.sh }; }
    placement { a: pinned(core = 0); b: pinned(core = 1); }
}

fn main() { App { }; }
";

/// The map itself, reached by two main-locus fields placed on different
/// pools.
const ALIASED_MAP: &str = "\
type Entry { k: Int; v: Int; }

@form(hashmap)
locus Registry {
    capacity { pool entries of Entry indexed_by k; }
}

locus A { params { reg: Registry = Registry { }; } run() { } }
locus B { params { reg: Registry = Registry { }; } run() { } }

main locus App {
    params { reg: Registry = Registry { };
             a: A = A { reg: self.reg };
             b: B = B { reg: self.reg }; }
    placement { a: pinned(core = 0); b: pinned(core = 1); }
}

fn main() { App { }; }
";

const ALIAS: &str = "shares one instance across pools";

/// GH #1566: one instance two pools reach is refused where it is handed
/// over, whatever map it holds: `Shared` itself is unsynchronized.
#[test]
fn sharing_an_instance_holding_a_map_across_pools_is_refused() {
    for (tag, sync) in [("none", ", sync = none"), ("plain", ""), ("serialized", ", sync = serialized")] {
        let root = scratch(&format!("alias_{tag}"));
        let (ok, out) = check(&root, sync, ALIASED_HOLDS);
        assert!(!ok && out.contains(ALIAS) && out.contains("`Shared` (`self.sh`)"), "{tag}: {out}");
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// A map whose form synchronizes access from another domain may be
/// shared across pools; one left unsynchronized may not.
#[test]
fn sharing_a_serialized_map_across_pools_is_accepted() {
    let root = scratch("alias_map_serialized");
    let (ok, out) = check(&root, ", sync = serialized", ALIASED_MAP);
    assert!(ok && !out.contains(ALIAS) && !out.contains("unsynchronized"), "{out}");
    let _ = std::fs::remove_dir_all(&root);
    for (tag, sync) in [("none", ", sync = none"), ("plain", "")] {
        let root = scratch(&format!("alias_map_{tag}"));
        let (ok, out) = check(&root, sync, ALIASED_MAP);
        assert!(!ok && out.contains(ALIAS), "{tag}: {out}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
