//! F.40 phase 3, C2 — a qualified bus subject is resolved once.
//!
//! `alias::Topic` in a `subscribe`, a `publish` or a send names the
//! imported topic declaration. The desugar sequence resolves the path
//! to the topic's own (mangled) name before the check, and the
//! resolved-program step does not resolve it again; the sequence's own
//! behaviour is pinned in `hale-types/tests/qualified_subjects.rs`.
//! These are the same shape through the verbs, end to end: the
//! declared payload checks, builds and delivers, and a wrong one is
//! refused at `check`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn tree(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let d: PathBuf = std::env::temp_dir().join(format!("hale_qualified_topic_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    for (name, src) in files {
        let p = d.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).expect("mkdir");
        std::fs::write(&p, src).expect("write");
    }
    d
}

fn hale(cwd: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("hale");
    (
        out.status.success(),
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
    )
}

const LIB: &str = "type Beat {\n    n: Int;\n}\n\n\
                   topic Heartbeat {\n    payload: Beat;\n    subject: \"src.heartbeat\";\n}\n";

fn app(send: &str) -> String {
    format!(
        "import \"../lib\" as source;\n\
         \n\
         locus Sub {{\n\
         \x20   bus {{ subscribe source::Heartbeat as on_beat; }}\n\
         \x20   fn on_beat(b: source::Beat) {{\n\
         \x20       println(\"got n=\" + b.n);\n\
         \x20   }}\n\
         }}\n\
         \n\
         locus Pub {{\n\
         \x20   bus {{ publish source::Heartbeat; }}\n\
         \x20   run() {{\n\
         \x20       source::Heartbeat <- {send};\n\
         \x20   }}\n\
         }}\n\
         \n\
         fn main() {{\n\
         \x20   let s = Sub {{ }};\n\
         \x20   Pub {{ }};\n\
         }}\n"
    )
}

/// The control: the declared payload, through the qualified path,
/// checks, builds and delivers.
#[test]
fn a_qualified_topic_carries_the_declarations_payload() {
    let d = tree("ok", &[("lib/topics.hl", LIB), ("app/main.hl", &app("source::Beat { n: 42 }"))]);
    let app = d.join("app");
    let (ok, out) = hale(&app, &["check", "."]);
    assert!(ok, "check must pass:\n{out}");
    let (ok, out) = hale(&app, &["run", "."]);
    assert!(ok, "run must pass:\n{out}");
    assert!(out.contains("got n=42"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

/// A send whose value is not the declared payload is refused at
/// `check`, naming the payload.
#[test]
fn a_send_of_the_wrong_payload_on_a_qualified_topic_is_refused_at_check() {
    let d = tree("wrong", &[("lib/topics.hl", LIB), ("app/main.hl", &app("7"))]);
    let app = d.join("app");
    let (ok, out) = hale(&app, &["check", "."]);
    assert!(!ok, "a payload that is not the declared one must be refused:\n{out}");
    assert!(out.contains("Beat"), "the refusal names the declared payload:\n{out}");
    let _ = std::fs::remove_dir_all(&d);
}
