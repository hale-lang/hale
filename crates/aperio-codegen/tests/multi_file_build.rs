//! Per-directory seed model — top-level decls in any .ap file
//! are visible to every other file in the same seed (one binary).
//! This is the regression for the dir-seeds milestone (resolves
//! `notes/aperio-friction.md` 2026-05-10 single-file-app-monolith).

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use aperio_codegen::build_executable;
use aperio_syntax::ast::Program;

fn unique_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "aperio_multi_file_{}_{}_{}",
        tag,
        std::process::id(),
        nanos
    ));
    p
}

/// Mirrors the merge step inside `aperio-cli` `run_build` —
/// concat every parsed Program's items into one Program. Order:
/// the iterator's order (the CLI sorts files alphabetically
/// before parsing).
fn merge(programs: Vec<Program>) -> Program {
    let mut iter = programs.into_iter();
    let first = iter.next().expect("at least one program");
    let mut merged = Program {
        items: first.items,
        imports: Vec::new(),
        span: first.span,
    };
    for p in iter {
        merged.items.extend(p.items);
    }
    merged
}

#[test]
fn cross_file_fn_call() {
    // Two files in one seed; main.ap calls helpers defined in
    // helpers.ap. Ordering matters only for the merge order
    // (helpers.ap sorts before main.ap alphabetically); the
    // typechecker's flat scope makes resolution order-free.
    let helpers = r#"
        fn say(s: String) { println("seed says: ", s); }
    "#;
    let main = r#"
        locus AppL {
            params { msg: String = "from main"; }
            run() { say(self.msg); }
        }
        fn main() { AppL { }; }
    "#;
    let p_helpers = aperio_syntax::parse_source(helpers).expect("parse helpers");
    let p_main = aperio_syntax::parse_source(main).expect("parse main");
    let merged = merge(vec![p_helpers, p_main]);

    let dir = unique_dir("cross_fn");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let bin = dir.join("app");
    build_executable(&merged, &bin).expect("build merged");

    let out = Command::new(&bin).output().expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(out.status.success(), "binary exited non-zero: {:?}", out.status);
    assert!(
        stdout.contains("seed says: from main"),
        "expected seed message in output, got: {:?}",
        stdout
    );
}

#[test]
fn cross_file_locus_referenced() {
    // Helpers file declares a type and a fn that returns it;
    // main constructs the type via the helper. Tests that
    // user-defined types in one file are resolvable from another.
    let helpers = r#"
        type Config { who: String; n: Int; }
        fn make_config() -> Config {
            return Config { who: "world", n: 7 };
        }
    "#;
    let main = r#"
        locus AppL {
            params { }
            run() {
                let c = make_config();
                println("who=", c.who, " n=", c.n);
            }
        }
        fn main() { AppL { }; }
    "#;
    let p_helpers = aperio_syntax::parse_source(helpers).expect("parse helpers");
    let p_main = aperio_syntax::parse_source(main).expect("parse main");
    let merged = merge(vec![p_helpers, p_main]);

    let dir = unique_dir("cross_type");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let bin = dir.join("app");
    build_executable(&merged, &bin).expect("build merged");

    let out = Command::new(&bin).output().expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(out.status.success(), "binary exited non-zero: {:?}", out.status);
    assert!(
        stdout.contains("who=world n=7"),
        "expected combined output from cross-file type + fn, got: {:?}",
        stdout
    );
}
