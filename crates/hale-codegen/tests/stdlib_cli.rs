//! std::cli::Resolver — layered config resolution.
//!
//! Validates the precedence ritual end-to-end: a Resolver
//! configured with `env_prefix` + `argv_keys` returns the
//! highest-populated layer's value (CLI > env > fallback).
//! Each test exercises one precedence claim in isolation so a
//! regression points at the specific layer that broke.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_hale(name: &str, source: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_stdlib_cli_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn resolver_returns_fallback_when_no_cli_or_env() {
    let src = r#"
        fn main() {
            let r = std::cli::Resolver {
                env_prefix: "HALE_TEST_",
                argv_keys:  "dir\nflavor\n",
            };
            let dir = r.get("dir", "default-dir");
            let flavor = r.get("flavor", "default-flavor");
            println("dir=[", dir, "]");
            println("flavor=[", flavor, "]");
        }
    "#;
    let bin = build_hale("fallback", src);
    let out = Command::new(&bin)
        .env_remove("HALE_TEST_DIR")
        .env_remove("HALE_TEST_FLAVOR")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("dir=[default-dir]"), "got: {:?}", stdout);
    assert!(stdout.contains("flavor=[default-flavor]"), "got: {:?}", stdout);
}

#[test]
fn resolver_env_overrides_fallback() {
    let src = r#"
        fn main() {
            let r = std::cli::Resolver {
                env_prefix: "HALE_TEST_",
                argv_keys:  "dir\nflavor\n",
            };
            let dir = r.get("dir", "default-dir");
            let flavor = r.get("flavor", "default-flavor");
            println("dir=[", dir, "]");
            println("flavor=[", flavor, "]");
        }
    "#;
    let bin = build_hale("env_wins", src);
    let out = Command::new(&bin)
        .env("HALE_TEST_DIR", "from-env")
        .env_remove("HALE_TEST_FLAVOR")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("dir=[from-env]"), "got: {:?}", stdout);
    // FLAVOR isn't set — falls through to fallback.
    assert!(stdout.contains("flavor=[default-flavor]"), "got: {:?}", stdout);
}

#[test]
fn resolver_cli_overrides_env() {
    let src = r#"
        fn main() {
            let r = std::cli::Resolver {
                env_prefix: "HALE_TEST_",
                argv_keys:  "dir\nflavor\n",
            };
            let dir = r.get("dir", "default-dir");
            let flavor = r.get("flavor", "default-flavor");
            println("dir=[", dir, "]");
            println("flavor=[", flavor, "]");
        }
    "#;
    let bin = build_hale("cli_wins", src);
    let out = Command::new(&bin)
        .args(["from-cli", "cli-flavor"])
        .env("HALE_TEST_DIR", "should-lose-to-cli")
        .env("HALE_TEST_FLAVOR", "should-also-lose")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("dir=[from-cli]"), "got: {:?}", stdout);
    assert!(stdout.contains("flavor=[cli-flavor]"), "got: {:?}", stdout);
}

#[test]
fn resolver_cli_partial_falls_through_to_env_for_missing_positions() {
    let src = r#"
        fn main() {
            let r = std::cli::Resolver {
                env_prefix: "HALE_TEST_",
                argv_keys:  "dir\nflavor\n",
            };
            let dir = r.get("dir", "default-dir");
            let flavor = r.get("flavor", "default-flavor");
            println("dir=[", dir, "]");
            println("flavor=[", flavor, "]");
        }
    "#;
    let bin = build_hale("partial_cli", src);
    let out = Command::new(&bin)
        .args(["only-first"])
        .env("HALE_TEST_FLAVOR", "env-flavor")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // dir wins from CLI; flavor's CLI slot (argv[2]) is unfilled
    // so it falls through to env.
    assert!(stdout.contains("dir=[only-first]"), "got: {:?}", stdout);
    assert!(stdout.contains("flavor=[env-flavor]"), "got: {:?}", stdout);
}

#[test]
fn resolver_get_int_parses_and_falls_back_on_non_numeric() {
    let src = r#"
        fn main() {
            let r = std::cli::Resolver {
                env_prefix: "HALE_TEST_",
                argv_keys:  "depth\nbroken\n",
            };
            let depth = r.get_int("depth", 4);
            let broken = r.get_int("broken", 99);
            println("depth=", depth);
            println("broken=", broken);
        }
    "#;
    let bin = build_hale("get_int", src);
    let out = Command::new(&bin)
        .args(["7", "not-a-number"])
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("depth=7"), "got: {:?}", stdout);
    // Non-numeric CLI value silently falls back to default
    // rather than crashing the app's first lifecycle method.
    assert!(stdout.contains("broken=99"), "got: {:?}", stdout);
}

#[test]
fn resolver_env_key_normalization_uppercases_key() {
    let src = r#"
        fn main() {
            let r = std::cli::Resolver {
                env_prefix: "HALE_TEST_",
                argv_keys:  "max_depth\n",
            };
            let v = r.get("max_depth", "fallback");
            println("v=[", v, "]");
        }
    "#;
    let bin = build_hale("upper", src);
    let out = Command::new(&bin)
        .env("HALE_TEST_MAX_DEPTH", "uppercased")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Key "max_depth" is normalized to MAX_DEPTH against the
    // prefix, finding HALE_TEST_MAX_DEPTH.
    assert!(stdout.contains("v=[uppercased]"), "got: {:?}", stdout);
}
