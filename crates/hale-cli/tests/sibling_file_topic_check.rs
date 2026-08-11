//! `hale check` on a multi-file seed resolves sibling-file
//! declarations the same way `hale build` does (downstream
//! handoff, 2026-08-11).
//!
//! A `topic` declared in one file of a seed and subscribed /
//! published from a sibling file built and ran fine, but `check`
//! reported "unknown topic": the no-import multi-file path kept one
//! Program PER FILE, and `apply_sync_inference` runs a
//! single-program resolver pass over each — so the file using the
//! topic was resolved alone. The fix routes a multi-file seed
//! through the same merge the import-bearing path already used.

use std::path::{Path, PathBuf};
use std::process::Command;

fn seed_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("hale_sibtopic_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir seed");
    d
}

fn hale_check(target: &Path) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(target)
        .output()
        .expect("run hale check");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn sibling_file_topic_checks_clean() {
    let d = seed_dir("ok");
    std::fs::write(
        d.join("topic.hl"),
        r#"
        type PingMsg { n: Int; }
        topic Ping { payload: PingMsg; }
        "#,
    )
    .unwrap();
    std::fs::write(
        d.join("pub.hl"),
        r#"
        locus Publisher {
            params { }
            bus { publish Ping; }
            fn go() { Ping <- PingMsg { n: 1 }; }
        }
        "#,
    )
    .unwrap();
    std::fs::write(
        d.join("main.hl"),
        r#"
        main locus App {
            params { p: Publisher = Publisher { }; }
            bus { subscribe Ping as on_ping; }
            fn on_ping(m: PingMsg) { println("got ping n=", to_string(m.n)); }
            run() { self.p.go(); terminate; }
        }

        fn main() { App { }; }
        "#,
    )
    .unwrap();

    let (out, code) = hale_check(&d);
    assert_eq!(code, 0, "check failed:\n{}", out);
    assert!(
        out.contains("3 file(s) typechecked"),
        "expected all 3 files counted:\n{}",
        out
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn genuinely_unknown_topic_still_rejected() {
    // The merge must not swallow the real diagnostic: a topic with
    // no declaration in ANY file of the seed still errors.
    let d = seed_dir("bad");
    std::fs::write(
        d.join("a.hl"),
        r#"
        type PingMsg { n: Int; }
        "#,
    )
    .unwrap();
    std::fs::write(
        d.join("b.hl"),
        r#"
        locus Publisher {
            params { }
            bus { publish Ping; }
            fn go() { Ping <- PingMsg { n: 1 }; }
        }
        fn main() { let p = Publisher { }; p.go(); }
        "#,
    )
    .unwrap();

    let (out, code) = hale_check(&d);
    assert_ne!(code, 0, "expected failure:\n{}", out);
    assert!(
        out.contains("unknown topic `Ping`"),
        "expected unknown-topic diagnostic:\n{}",
        out
    );
    let _ = std::fs::remove_dir_all(&d);
}
