//! A locus born in a param-field child's method bubbles up to the
//! ancestor that accepts it.
//!
//! `Ship` is born in `Fleet.spawn`, and `Fleet` does not accept it;
//! `World` does, and `Fleet` is `World`'s param default. The ownership
//! plan stitches the ship to `World` through the `__owner_for_Ship`
//! field the birth chain threads down, and the bubble site loads that
//! field from `Fleet`'s own struct. The docs pass found the program
//! segfaulting after `before` (exit 139): the field was `null`, because
//! a param child is born while its holder is still constructing, and
//! the threading took its parent from `current_self` — the enclosing
//! METHOD's locus, or nothing at all in `main`.
//!
//! Two things were wrong, and both are pinned here:
//!  1. The parent a field child threads from is the locus whose FIELD it
//!     is, not `current_self`.
//!  2. That threading is written before the holder's own params-init
//!     loop, not after it. A grandchild (`Squad`, a param default of
//!     `Fleet`) forwards `Fleet`'s own field, which the loop had not
//!     yet been written — the second shape below read it uninitialised.
//!
//! Every program runs under AddressSanitizer with the chunk pool off and
//! heap values (each ship carries a String built at runtime); a stale or
//! null owner would fault there, and each ship must be accepted once
//! and dissolved once.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;

const SHIP: &str = r#"
locus Ship {
    params { name: String = ""; hull: Int = 0; }
    dissolve() { println("dissolve ", self.name); }
}
"#;

fn run_asan(tag: &str, src: &str) -> Vec<String> {
    let bin = harness::unique_bin(tag);
    harness::build_source_asan(src, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .env("ASAN_OPTIONS", "detect_leaks=0")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{tag}: {:?}\n{stderr}", out.status);
    assert!(!stderr.contains("AddressSanitizer"), "{tag}: {stderr}");
    String::from_utf8_lossy(&out.stdout).lines().map(String::from).collect()
}

#[test]
fn a_ship_born_in_a_param_childs_method_is_accepted_by_the_ancestor() {
    let src = format!(
        r#"{SHIP}
locus Fleet {{
    fn spawn(k: Int) {{ Ship {{ name: "fl-" + to_string(k) + "-zzzzzzzzzzzzzzzzzzzz", hull: k }}; }}
}}
locus World {{
    params {{ fleet: Fleet = Fleet {{ }}; }}
    accept(s: Ship) {{ println("accepted ", s.name); }}
    fn go() {{
        println("before");
        self.fleet.spawn(1);
        self.fleet.spawn(2);
        println("after");
    }}
}}
fn main() {{ let w = World {{ }}; w.go(); println("end"); }}
"#
    );
    let lines = run_asan("bubble_param_child", &src);
    let z = "-zzzzzzzzzzzzzzzzzzzz";
    assert_eq!(
        lines,
        [
            "before".to_string(),
            format!("accepted fl-1{z}"),
            format!("accepted fl-2{z}"),
            "after".to_string(),
            "end".to_string(),
            format!("dissolve fl-1{z}"),
            format!("dissolve fl-2{z}"),
        ]
    );
}

#[test]
fn the_threading_reaches_a_grandchild_and_an_override() {
    let src = format!(
        r#"{SHIP}
locus Squad {{
    fn spawn(k: Int) {{ Ship {{ name: "sq-" + to_string(k) + "-zzzzzzzzzzzzzzzzzzzz", hull: k }}; }}
}}
locus Fleet {{
    params {{ squad: Squad = Squad {{ }}; }}
    fn spawn(k: Int) {{
        Ship {{ name: "fl-" + to_string(k) + "-zzzzzzzzzzzzzzzzzzzz", hull: k }};
        self.squad.spawn(k + 100);
    }}
}}
locus World {{
    params {{ fleet: Fleet = Fleet {{ }}; }}
    accept(s: Ship) {{ println("accepted ", s.name); }}
    fn go() {{ self.fleet.spawn(1); self.fleet.spawn(2); }}
}}
fn main() {{
    let w = World {{ }};
    w.go();
    let v = World {{ fleet: Fleet {{ }} }};
    v.go();
    println("end");
}}
"#
    );
    let lines = run_asan("bubble_param_grandchild", &src);
    let z = "-zzzzzzzzzzzzzzzzzzzz";
    let accepted = |l: &String| l.starts_with("accepted ");
    let dissolved = |l: &String| l.starts_with("dissolve ");
    assert_eq!(lines.iter().filter(|l| accepted(l)).count(), 8, "two worlds x four ships: {lines:?}");
    assert_eq!(lines.iter().filter(|l| dissolved(l)).count(), 8, "each ship dissolves once: {lines:?}");
    for want in [format!("accepted fl-1{z}"), format!("accepted sq-101{z}"), format!("accepted fl-2{z}"), format!("accepted sq-102{z}")] {
        assert_eq!(lines.iter().filter(|l| **l == want).count(), 2, "{want} in each world: {lines:?}");
    }
    let end = lines.iter().position(|l| l == "end").expect("end");
    assert!(lines[end + 1..].iter().all(dissolved), "the ships are reclaimed by their world's teardown, after main's last line: {lines:?}");
}
