//! GH #732: an IMPORTED interface keeps its methods' `fallible(E)`
//! and enforces it. The error type lives in the library seed, so it
//! reaches the consumer only through the cross-seed rename table —
//! a single-seed test cannot see it. `check` must reject a consumer
//! method whose error type differs, and a conforming one must build
//! and dispatch both channels.

use std::path::{Path, PathBuf};
use std::process::Command;

const LIB: &str = r#"
type StoreError { kind: String = ""; }
type OtherError { kind: String = ""; }
interface Store { fn put(k: String) -> Int fallible(StoreError); }
"#;

const CONSUMER_OK: &str = r#"
import "lib" as lib;

locus Good {
    fn put(k: String) -> Int fallible(lib::StoreError) {
        if k == "" { fail lib::StoreError { kind: "empty" }; }
        return 2;
    }
}

fn use_it(s: lib::Store, k: String) -> Int { return s.put(k) or 0; }

fn main() {
    let g = Good { };
    println("ok=", use_it(g, "a"));
    println("failed=", use_it(g, ""));
}
"#;

const CONSUMER_BAD: &str = r#"
import "lib" as lib;

locus Bad {
    fn put(k: String) -> Int fallible(lib::OtherError) { return 1; }
}

fn use_it(s: lib::Store, k: String) -> Int { return s.put(k) or 0; }

fn main() { let b = Bad { }; println(use_it(b, "a")); }
"#;

/// A two-seed project in a per-test directory, unique per process
/// and per tag.
fn seed(tag: &str, consumer: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("hale_iface_fallible_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lib")).expect("create seed dirs");
    std::fs::write(dir.join("lib").join("main.hl"), LIB).expect("write lib");
    std::fs::write(dir.join("main.hl"), consumer).expect("write consumer");
    dir
}

fn hale(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("invoke hale");
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
fn an_imported_fallible_interface_dispatches_both_channels() {
    let dir = seed("ok", CONSUMER_OK);
    let (ok, out) = hale(&dir, &["check", "main.hl"]);
    assert!(ok, "check: {out}");
    let (ok, out) = hale(&dir, &["run", "main.hl"]);
    assert!(ok && out.contains("ok=2") && out.contains("failed=0"), "run: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_imported_interface_enforces_its_error_type() {
    let dir = seed("bad", CONSUMER_BAD);
    let (ok, out) = hale(&dir, &["check", "main.hl"]);
    assert!(!ok, "a different error type must not check: {out}");
    assert!(
        out.contains("locus `Bad` method `put` declares a different error type")
            && out.contains("fallible(lib::StoreError)")
            && out.contains("fallible(lib::OtherError)"),
        "{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
