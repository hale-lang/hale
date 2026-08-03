//! `hale check` must reject an unknown `std::` namespace (#353 item 9).
//!
//! `unknown_fn_error` resolved the namespace first and returned early
//! when it was untabled — "fine or not our business". So a call into a
//! namespace that does not exist passed the checker and was caught only
//! by codegen:
//!
//! ```text
//! std::totally::fake()   check: PASS      build: caught
//! ```
//!
//! Which meant a typo'd or imagined stdlib call was invisible to
//! `hale check`, to the CI check gate, and to the LSP — the editor
//! would positively confirm that made-up code was valid. That
//! undermines the promise the whole toolchain rests on: that
//! structured checking is authoritative.
//!
//! Found by falling into it. An earlier survey of missing stdlib
//! features reported regex, sort and sets as PRESENT, because `check`
//! accepted calls into all three.

use std::process::Command;

fn check(src: &str, tag: &str) -> String {
    let dir = std::env::temp_dir()
        .join(format!("hale-ns-{}-{}", std::process::id(), tag));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let f = dir.join("main.hl");
    std::fs::write(&f, src).expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&f)
        .output()
        .expect("run hale check");
    let _ = std::fs::remove_dir_all(&dir);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn an_unknown_namespace_is_rejected_by_check() {
    let out = check("fn main() { println(std::totally::fake()); }", "unknown");
    assert!(
        out.contains("unknown stdlib namespace"),
        "an entirely made-up `std::` namespace must not pass check:\n{}",
        out
    );
}

#[test]
fn a_near_miss_namespace_offers_the_real_one() {
    let out = check("fn main() { println(std::tmie::now()); }", "nearmiss");
    assert!(
        out.contains("did you mean `std::time`"),
        "a transposition should point at the namespace meant:\n{}",
        out
    );
}

/// The guard must not fire on real namespaces, or every program breaks.
#[test]
fn a_real_namespace_still_passes() {
    let out = check("fn main() { println(std::math::sqrt(4.0)); }", "real");
    assert!(
        out.contains("typechecked"),
        "`std::math::sqrt` is real and must pass:\n{}",
        out
    );
}
