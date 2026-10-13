//! The publish-before-subscriber advisory (spec/verification.md, "Publish
//! before its subscriber"; friction sweep 2026-10-12, downstream handoff):
//! a send written, in one body, before the literal that builds a locus
//! subscribing to its subject never reaches that subscriber — it registers
//! when the literal runs, and a cell reaches the subscribers registered
//! when it is published. `hale check` warns at the send, with the literal
//! as its witness; not when the subscriber is built first, and not for a
//! send after the literal inside one loop (the next iteration's, which the
//! built instance receives).
//!
//! The programs are escaped string literals rather than raw strings:
//! `hale-corpus` harvests raw-string literals out of test files, and these
//! are deliberately diagnostic.

use std::path::{Path, PathBuf};
use std::process::Command;

fn program(run_body: &str) -> String {
    format!(
        "type Ping {{ n: Int = 0; }}
topic PingT {{ payload: Ping; subject: \"order.ping\"; }}
locus Sub {{
    params {{ got: Int = 0; }}
    bus {{ subscribe PingT as on_p; }}
    fn on_p(p: Ping) {{ self.got = self.got + 1; }}
}}
locus Holder {{ params {{ s: Sub = Sub {{ }}; }} }}
main locus App {{
    bus {{ publish PingT; }}
    run() {{ {run_body} }}
}}
fn main() {{ App {{ }}; }}
"
    )
}

fn check(tag: &str, text: &str) -> (bool, String) {
    let root: PathBuf =
        std::env::temp_dir().join(format!("hale_check_early_publish_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&root);
    let seed = root.join("app");
    std::fs::create_dir_all(&seed).unwrap();
    std::fs::write(seed.join("main.hl"), text).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&seed)
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    let _ = std::fs::remove_dir_all(&root);
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

const FINDING: &str = "comes before the";

#[test]
fn a_publish_before_its_subscriber_is_built_is_warned() {
    let (ok, out) = check("before", &program("PingT <- Ping { n: 1 }; let s = Sub { }; std::time::sleep(10ms);"));
    assert!(ok, "a warning, not an error:\n{out}");
    assert_eq!(out.matches(FINDING).count(), 1, "one finding:\n{out}");
    assert!(
        out.contains("this publish on `order.ping` comes before the `Sub` built below exists, and `Sub` subscribes to it"),
        "names the subject and the locus:\n{out}"
    );
    assert!(out.contains("`Sub` is built here, after the publish"), "the literal is the witness:\n{out}");
    assert!(out.contains("warning"), "{out}");
}

#[test]
fn a_subscriber_its_params_build_counts() {
    let (ok, out) = check("params", &program("PingT <- Ping { n: 1 }; Holder { };"));
    assert!(ok, "{out}");
    assert!(out.contains("the `Sub` it builds subscribes to it"), "{out}");
}

#[test]
fn a_subscriber_built_first_is_not_warned() {
    let (ok, out) = check("after", &program("let s = Sub { }; PingT <- Ping { n: 1 }; std::time::sleep(10ms);"));
    assert!(ok, "{out}");
    assert_eq!(out.matches(FINDING).count(), 0, "no finding:\n{out}");
}

#[test]
fn a_publish_after_the_literal_in_a_loop_is_not_warned() {
    let (ok, out) = check(
        "loop",
        &program("let mut i = 0; while i < 3 { let s = Sub { }; PingT <- Ping { n: i }; std::time::sleep(10ms); i = i + 1; }"),
    );
    assert!(ok, "{out}");
    assert_eq!(out.matches(FINDING).count(), 0, "no finding:\n{out}");
}
