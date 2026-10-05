//! An instance's header is initialized before anything can reach it
//! (F.40 phase 3, a classified correction).
//!
//! A locus literal used to build its params first and initialize its
//! own header after them: the children list (`__children`,
//! `__child_count`, `__child_cap`), the restart and failure flags, the
//! recpool fields, the parent and owner pointers. A params default
//! whose child runs inside the literal can already reach the instance —
//! through `@__owner_singleton_<L>` (published before the params) or
//! through the pointer its parent threads down — so a child bubbling to
//! the half-built owner pushed itself onto a children list the literal
//! had not written yet. On a stack instance that read whatever the stack
//! held (SIGSEGV on hosts whose stack was not zero there); and when it
//! did not crash, the literal then reset the list, so the children
//! accepted during the params were forgotten and never torn down.
//!
//! Each program below fails on the parent on every host: the stack
//! programs by the children they lose (and the dirtied one by the
//! garbage it pushes into), the heap-allocated owners under
//! AddressSanitizer with the chunk pool off, whose allocator fills new
//! memory with 0xbe — a half-built owner's header reads that, never zero.

use std::process::{Command, Output};

#[path = "support/harness.rs"]
mod harness;

/// ASan fills a fresh allocation's first `max_malloc_fill_size` bytes
/// with 0xbe; an arena chunk is larger than the default 4 KiB, so fill
/// all of it.
const ASAN_OPTIONS: &str = "detect_leaks=0:max_malloc_fill_size=1048576";

const SHIP: &str = r#"
locus Ship {
    params { name: String = ""; }
    dissolve() { println("dissolve " + self.name); }
}
"#;

fn run(tag: &str, src: &str, asan: bool) -> Output {
    let bin = harness::unique_bin(tag);
    if asan {
        harness::build_source_asan(src, &bin);
    } else {
        harness_build::build_source(src, &bin, &harness_build::options()).expect("build");
    }
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .env("ASAN_OPTIONS", ASAN_OPTIONS)
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    out
}

#[path = "support/build.rs"]
mod harness_build;

fn lines_of(tag: &str, out: &Output) -> Vec<String> {
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{tag}: {:?}\nstdout: {}\nstderr: {stderr}", out.status, String::from_utf8_lossy(&out.stdout));
    assert!(!stderr.contains("AddressSanitizer"), "{tag}: {stderr}");
    String::from_utf8_lossy(&out.stdout).lines().map(String::from).collect()
}

/// The program the defect was found with: `Driver`, `World`'s param
/// default, spawns two `Ship`s in its `run()`, which runs inside
/// `World`'s literal; both bubble to `World` through its owner singleton.
fn world(main_body: &str, extra: &str) -> String {
    format!(
        r#"{SHIP}
fn take(s: Ship = Ship {{ name: std::str::upper("taken") }}) {{ println("took " + s.name); }}
locus Driver {{ run() {{ Ship {{ name: std::str::upper("spawned") }}; take(); }} }}
main locus World {{
    params {{ driver: Driver = Driver {{ }}; }}
    accept(s: Ship) {{ println("accepted " + s.name); }}
    run() {{ println("world run"); }}
}}
{extra}
fn main() {{ {main_body} }}
"#
    )
}

const WORLD_LINES: [&str; 7] = [
    "accepted SPAWNED",
    "accepted TAKEN",
    "took TAKEN",
    "world run",
    "dissolve SPAWNED",
    "dissolve TAKEN",
    "end",
];

#[test]
fn children_accepted_during_the_params_are_kept_and_torn_down() {
    let src = world(r#"World { }; println("end");"#, "");
    for asan in [false, true] {
        let tag = if asan { "header_world_asan" } else { "header_world" };
        let lines = lines_of(tag, &run(tag, &src, asan));
        assert_eq!(lines, WORLD_LINES, "{tag}: both ships stay accepted and dissolve with World");
    }
}

/// The same literal built in a frame whose stack a previous call left
/// full of non-zero bytes: `dirty()` holds a large locus struct of `-1`s
/// at the depth `build()`'s `World` takes next.
#[test]
fn a_dirty_stack_under_the_literal_is_never_read() {
    let fields: String = (0..48).map(|i| format!("f{i}: Int = 0; ")).collect();
    let inits: Vec<String> = (0..48).map(|i| format!("f{i}: 0 - 1")).collect();
    let extra = format!(
        r#"
locus Junk {{ params {{ {fields} }} }}
fn dirty() -> Int {{ let j = Junk {{ {inits} }}; return j.f0 + j.f47; }}
fn build() {{ World {{ }}; }}
"#,
        inits = inits.join(", ")
    );
    let src = world(r#"println("dirty " + to_string(dirty())); build(); println("end");"#, &extra);
    let lines = lines_of("header_dirty", &run("header_dirty", &src, false));
    let mut want = vec!["dirty -2".to_string()];
    want.extend(WORLD_LINES.iter().map(|s| s.to_string()));
    assert_eq!(lines, want);
}

/// A non-main owner in its owner's arena: `Mid` is `Outer`'s param
/// child, and its own param child `Spawner` runs inside `Mid`'s literal
/// and spawns two `Ship`s that bubble to `Mid` through the
/// `__owner_for_Ship` pointer `Mid` threads down (not a singleton).
fn mid(placement: &str) -> String {
    format!(
        r#"{SHIP}
locus Spawner {{
    run() {{
        Ship {{ name: std::str::upper("one") }};
        Ship {{ name: std::str::upper("two") }};
    }}
}}
locus Mid {{
    params {{ n: Int = 0; sp: Spawner = Spawner {{ }}; }}
    accept(s: Ship) {{ self.n = self.n + 1; println("accepted " + s.name); }}
    run() {{ println("mid run " + to_string(self.n)); }}
}}
main locus Outer {{
    params {{ mid: Mid = Mid {{ }}; }}
    {placement}
    run() {{ println("outer run"); }}
}}
fn main() {{ Outer {{ }}; println("end"); }}
"#
    )
}

fn assert_mid(tag: &str, lines: &[String]) {
    for want in ["accepted ONE", "accepted TWO", "mid run 2", "dissolve ONE", "dissolve TWO", "end"] {
        assert_eq!(lines.iter().filter(|l| *l == want).count(), 1, "{tag}: `{want}` once: {lines:?}");
    }
    let end = lines.iter().position(|l| l == "end").unwrap();
    let dissolved = lines.iter().position(|l| l.starts_with("dissolve ")).unwrap();
    assert!(dissolved < end, "{tag}: the ships go with Mid, at Outer's teardown: {lines:?}");
}

#[test]
fn an_owner_in_its_owners_arena_is_initialized_before_its_params() {
    let lines = lines_of("header_mid_arena", &run("header_mid_arena", &mid(""), true));
    assert_mid("arena", &lines);
}

#[test]
fn a_pool_placed_owner_is_initialized_before_its_params() {
    let src = mid("placement { mid: cooperative(pool = io, cores = 0..=1); }");
    let lines = lines_of("header_mid_pool", &run("header_mid_pool", &src, true));
    assert_mid("pool", &lines);
}

/// A pinned locus accepts no children (rule 6), so its params' child
/// reaches it the other way: it fails in its `run()`, inside the pinned
/// locus's init on its thread, and the held failure is delivered to the
/// pinned owner's `on_failure` when its params settle — still inside
/// the init. The handler reads its own header (`draining`), which must
/// be the zero the literal stored.
#[test]
fn a_pinned_owner_is_initialized_before_its_params() {
    let src = r#"
locus Kid {
    params { tag: String = ""; }
    closure fuse { captures: tag; epoch inline; }
    run() { violate fuse; }
    dissolve() { println("dissolve " + self.tag); }
}
locus Post {
    params { kid: Kid = Kid { tag: std::str::upper("kid") }; }
    on_failure(c: Kid, err: ClosureViolation) {
        println("held draining " + to_string(self.draining));
    }
    run() { println("post draining " + to_string(self.draining)); }
}
main locus App {
    params { p: Post = Post { }; }
    placement { p: pinned; }
}
fn main() { App { }; println("end"); }
"#;
    let lines = lines_of("header_pinned", &run("header_pinned", src, true));
    assert_eq!(lines, ["held draining false", "post draining false", "dissolve KID", "end"]);
}
