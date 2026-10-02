//! F.40 phase 3, C7: the laws that replaced lowering's backstops are
//! judged at the production entry points, located.
//!
//! Lowering used to refuse these shapes itself, without a span, and
//! `hale check` accepted them. The laws (`hale_types::lowering_laws`)
//! run among the check's rules, and `hale build` checks before it
//! lowers, so both verbs refuse with the law's span and wording. The
//! harness's entry, which skips the check, is pinned in
//! `hale-codegen/tests/harness_lowering_laws.rs`.
//!
//! The programs are plain escaped string literals: `hale-corpus`
//! harvests `r#"…"#` literals out of test files into the corpus-wide
//! properties, and these are written to be refused.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Rule 6: an adapter inline in `bindings { }` runs pinned, and this
/// one accepts children. The binding entry is line 6, column 16.
const ADAPTER_ACCEPTS: &str = "type Tick { n: Int; }\n\
     topic Beat { payload: Tick; subject: \"beat\"; }\n\
     locus Child { run() { } }\n\
     locus Sink { accept(c: Child) { } fn send(subject: String, bytes: Bytes) { } }\n\
     locus Pub { bus { publish Beat; } run() { Beat <- Tick { n: 1 }; } }\n\
     main locus App { params { p: Pub = Pub { }; } bindings { Beat: Sink { }; } }\n\
     fn main() { App { }; }\n";

const RULE_6: &str = "binding entry `Beat`: adapter `Sink` runs pinned";

/// Rule 17 (GH #826): a root that pins a field, built inside a loop. The
/// literal is line 4, column 21.
const PINNED_ROOT_IN_A_LOOP: &str = "locus Worker { run() { } }\n\
     main locus App { params { w: Worker = Worker { }; } placement { w: pinned; } }\n\
     fn main() {\n\
     \x20   for i in 0..3 { App { }; }\n\
     }\n";

const RULE_17: &str = "locus `App` is instantiated inside a loop, but its `placement { }` block pins field `w`";

fn seed(tag: &str, src: &str) -> PathBuf {
    let d: PathBuf = std::env::temp_dir().join(format!(
        "hale_check_lowering_laws_{}_{}",
        std::process::id(),
        tag
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir");
    std::fs::write(d.join("main.hl"), src).expect("write");
    d
}

fn hale(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

#[test]
fn check_refuses_a_pinned_adapter_that_accepts_at_the_binding_entry() {
    let d = seed("check", ADAPTER_ACCEPTS);
    let (ok, out) = hale(&["check", &d.to_string_lossy()]);
    assert!(!ok, "check must fail:\n{out}");
    assert!(out.contains(RULE_6) && out.contains("(rule 6)"), "{out}");
    assert!(out.contains(":6:"), "the span is the binding entry's line:\n{out}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn build_refuses_it_at_the_check_with_the_same_span() {
    let d = seed("build", ADAPTER_ACCEPTS);
    let bin = d.join("out");
    let (ok, out) = hale(&["build", &d.to_string_lossy(), "-o", &bin.to_string_lossy()]);
    assert!(!ok, "build must fail:\n{out}");
    assert!(out.contains(RULE_6) && out.contains(":6:"), "{out}");
    assert!(!bin.exists(), "nothing was lowered");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn check_and_build_refuse_a_pinned_root_in_a_loop_at_the_literal() {
    for verb in ["check", "build"] {
        let d = seed(&format!("rule17_{verb}"), PINNED_ROOT_IN_A_LOOP);
        let bin = d.join("out");
        let dir = d.to_string_lossy().to_string();
        let out_path = bin.to_string_lossy().to_string();
        let args: Vec<&str> = match verb {
            "check" => vec!["check", &dir],
            _ => vec!["build", &dir, "-o", &out_path],
        };
        let (ok, out) = hale(&args);
        assert!(!ok, "{verb} must fail:\n{out}");
        assert!(out.contains(RULE_17) && out.contains(":4:21:"), "{verb}: located at the literal:\n{out}");
        assert!(!bin.exists(), "nothing was lowered");
        let _ = std::fs::remove_dir_all(&d);
    }
}
