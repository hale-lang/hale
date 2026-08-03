//! `@form(set)` (#353 item 5).
//!
//! `spec/decisions.md` specified this and deferred it with a trigger:
//! "a `@form(set)` would be a hashmap-without-value variant (the cell
//! IS the key). Not part of FORM-4; revisit if a workload needs it."
//! #353 is that workload, so this is scheduled work whose condition
//! fired rather than a new design.
//!
//! It reuses the hashmap slot and the entire `lotus_hashmap_*`
//! runtime — including the sync disciplines, so `@form(set, sync =
//! striped)` works for free. Only the synthesized surface differs:
//! `insert` / `contains` / `remove` / `len` / `is_empty` instead of
//! `set` / `get` / `has`.
//!
//! The point of the separate surface is keeping the VALUE off the
//! call site. Membership through a hashmap means writing `get(k) or
//! false` everywhere, which is the value plumbing leaking back out at
//! every use.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!(
        "hale_set_{}_{}",
        name,
        std::process::id()
    ));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

const S: &str = "type Item { key: String; }\n\
                 @form(set)\n\
                 locus Seen { capacity { pool items of Item indexed_by key; } }\n";

/// The defining property: inserting the same key twice does not grow
/// the set. If this fails it is a map with a set's method names.
#[test]
fn insert_deduplicates() {
    let src = format!(
        "{S}fn main() {{
            let s = Seen {{ }};
            s.insert(Item {{ key: \"a\" }});
            s.insert(Item {{ key: \"b\" }});
            s.insert(Item {{ key: \"a\" }});
            println(\"len=\", s.len());
        }}"
    );
    let (out, st) = run("dedup", &src);
    assert!(st.success(), "non-zero: {:?}", st);
    assert!(out.contains("len=2"), "a,b,a is two members: {:?}", out);
}

/// `contains` answers Bool directly. Membership through the hashmap
/// surface would be `get(k) or false` at every call site — the value
/// plumbing leaking back out, which is what this form exists to stop.
#[test]
fn contains_answers_membership_without_a_fallible() {
    let src = format!(
        "{S}fn main() {{
            let s = Seen {{ }};
            s.insert(Item {{ key: \"a\" }});
            println(\"a=\", s.contains(\"a\"));
            println(\"z=\", s.contains(\"z\"));
        }}"
    );
    let (out, st) = run("contains", &src);
    assert!(st.success(), "non-zero: {:?}", st);
    assert!(out.contains("a=true"), "got: {:?}", out);
    assert!(out.contains("z=false"), "got: {:?}", out);
}

#[test]
fn remove_takes_a_member_out() {
    let src = format!(
        "{S}fn main() {{
            let s = Seen {{ }};
            s.insert(Item {{ key: \"a\" }});
            s.insert(Item {{ key: \"b\" }});
            s.remove(\"a\") or {{ }};
            println(\"len=\", s.len(), \" a=\", s.contains(\"a\"));
        }}"
    );
    let (out, st) = run("remove", &src);
    assert!(st.success(), "non-zero: {:?}", st);
    assert!(out.contains("len=1"), "got: {:?}", out);
    assert!(out.contains("a=false"), "got: {:?}", out);
}

/// An empty set is empty — the case a hashmap-backed implementation
/// gets wrong if `len` reads the slot count rather than the live
/// count.
#[test]
fn an_empty_set_is_empty() {
    let src = format!(
        "{S}fn main() {{
            let s = Seen {{ }};
            println(\"len=\", s.len(), \" empty=\", s.is_empty());
        }}"
    );
    let (out, st) = run("empty", &src);
    assert!(st.success(), "non-zero: {:?}", st);
    assert!(out.contains("len=0"), "got: {:?}", out);
    assert!(out.contains("empty=true"), "got: {:?}", out);
}
