//! GH #724: a locus implements an IMPORTED perspective by its
//! qualified name — `locus X : serves lib::Routing`.
//!
//! The same declaration with a LOCAL perspective always worked, and
//! qualified imported names worked everywhere else (types,
//! `alias::fn()`, qualified enum variants). The three perspective
//! positions were the exception: `serves P`, the `perspective(P)`
//! slot type and `reperspective self.f as Impl` each took a bare
//! IDENTIFIER, so `serves lib::Routing` died in the parser with
//! `expected {, got ColonColon` — a library could declare a contract
//! that no consumer could implement.
//!
//! This is the CLI path on purpose: the fix resolves the alias path
//! through the cross-seed rename table, which only exists once
//! imports are resolved, so a single-seed parser test cannot see it.
//! Both halves matter — `check` must resolve the contract (and keep
//! rejecting a non-conforming impl), and `build` must lower the slot
//! to the same program-global perspective the library's own impl
//! registers with.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The library every case here imports: one contract, one impl of it
/// that lives on the library side (the `reperspective` target).
const LIB: &str = r#"
perspective Routing {
    fn route(x: Int) -> Int;
}

locus Double : serves Routing {
    fn route(x: Int) -> Int { return x * 2; }
}
"#;

/// A consumer-side impl of the imported contract, designated into a
/// consumer-side slot and then swapped for the library's own impl.
/// Every one of the three qualified spellings appears once.
const CONSUMER_OK: &str = r#"
import "lib" as lib;

locus Triple : serves lib::Routing {
    fn route(x: Int) -> Int { return x * 3; }
}

main locus App {
    params { r: perspective(lib::Routing) = Triple { }; }
    run() {
        println("route=", self.r.route(7));
        reperspective self.r as lib::Double;
        println("swapped=", self.r.route(7));
    }
}

fn main() { App { }; }
"#;

/// Conformance is still checked through the alias: `route` takes a
/// `String` where the contract requires an `Int`.
const CONSUMER_BAD_SIG: &str = r#"
import "lib" as lib;

locus Triple : serves lib::Routing {
    fn route(x: String) -> Int { return 3; }
}

main locus App {
    params { r: perspective(lib::Routing) = Triple { }; }
    run() { println("route=", self.r.route(7)); }
}

fn main() { App { }; }
"#;

/// An unknown name behind a real alias must be reported at the PATH,
/// not silently tolerated as an opaque cross-seed reference.
const CONSUMER_UNKNOWN: &str = r#"
import "lib" as lib;

locus Triple : serves lib::Nope {
    fn route(x: Int) -> Int { return x * 3; }
}

main locus App {
    params { r: perspective(lib::Routing) = Triple { }; }
    run() { println("route=", self.r.route(7)); }
}

fn main() { App { }; }
"#;

/// A nested alias (`lib::inner::Routing`) resolves to nothing, and
/// must say so at the path it was written at rather than parse-fail
/// or pass.
const CONSUMER_NESTED_ALIAS: &str = r#"
import "lib" as lib;

locus Triple : serves lib::inner::Routing {
    fn route(x: Int) -> Int { return x * 3; }
}

main locus App {
    params { r: perspective(lib::Routing) = Triple { }; }
    run() { println("route=", self.r.route(7)); }
}

fn main() { App { }; }
"#;

/// A two-seed project in a per-test directory: `<tmp>/main.hl` plus
/// `<tmp>/lib/main.hl`. Unique per process AND per tag, so parallel
/// runs of this file never share the build output path.
fn seed(tag: &str, consumer: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("hale_qserves_{}_{}", std::process::id(), tag));
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
fn qualified_serves_checks_builds_and_dispatches() {
    let dir = seed("ok", CONSUMER_OK);
    let (ok, text) = hale(&dir, &["check", "."]);
    assert!(ok, "`hale check` must accept a qualified `serves`:\n{text}");
    assert!(
        !text.contains("unknown perspective")
            && !text.contains("parse error"),
        "no residual perspective / parse diagnostics:\n{text}"
    );
    let (ok, text) = hale(&dir, &["build", "."]);
    assert!(ok, "`hale build` must succeed:\n{text}");
    let bin = dir.join(dir.file_name().expect("seed dir name"));
    let out = Command::new(&bin).output().expect("run consumer");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "consumer exit: {:?}", out.status);
    // 7*3 through the consumer's own impl, designated into the slot
    // of the IMPORTED contract; 7*2 after the swap to the library's
    // impl — one program-global slot, reached from both seeds.
    assert!(stdout.contains("route=21"), "designate: {stdout:?}");
    assert!(stdout.contains("swapped=14"), "reperspective: {stdout:?}");
}

#[test]
fn qualified_serves_still_checks_the_signature() {
    let dir = seed("badsig", CONSUMER_BAD_SIG);
    let (ok, text) = hale(&dir, &["check", "."]);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!ok, "a wrong-signature impl must fail check:\n{text}");
    assert!(
        text.contains("method `route`")
            && text.contains("requires `Int`")
            && text.contains("has `String`"),
        "the diagnostic must name the offending method and both \
         types:\n{text}"
    );
    // Located: the clause that made the claim, rendered with the
    // alias the author wrote rather than the mangled symbol.
    assert!(
        text.contains("serves lib::Routing")
            && text.contains("`lib::Routing`"),
        "diagnostic must be located at the qualified path:\n{text}"
    );
}

#[test]
fn unknown_name_behind_a_real_alias_is_located() {
    let dir = seed("unknown", CONSUMER_UNKNOWN);
    let (ok, text) = hale(&dir, &["check", "."]);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!ok, "`serves lib::Nope` must fail check:\n{text}");
    assert!(
        text.contains("serves unknown perspective `lib::Nope`"),
        "must report the unknown contract by the path written:\n{text}"
    );
    assert!(
        text.contains("serves lib::Nope"),
        "the diagnostic must cite the source line:\n{text}"
    );
}

#[test]
fn nested_alias_is_located_not_a_parse_error() {
    let dir = seed("nested", CONSUMER_NESTED_ALIAS);
    let (ok, text) = hale(&dir, &["check", "."]);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!ok, "`serves lib::inner::Routing` must fail check:\n{text}");
    assert!(
        !text.contains("parse error"),
        "a nested alias is a RESOLUTION error, not a parse error:\n{text}"
    );
    assert!(
        text.contains("serves unknown perspective `lib::inner::Routing`"),
        "must report the whole path it could not resolve:\n{text}"
    );
}
