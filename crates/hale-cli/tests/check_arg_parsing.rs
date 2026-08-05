//! `hale check` / `hale verify` argument handling.
//!
//! From the v0.15.0 claims developer-experience review: `check` took
//! its target from `argv[2]` and treated everything else as scenery.
//! An unknown flag, a stray second positional, and `--help` were all
//! silently ignored while the command still reported SUCCESS — the
//! same fail-open shape as the topology gates, and worse here,
//! because a typo'd gate flag means CI checks nothing and says so in
//! green.
//!
//! `--help` was the clearest symptom: it was interpreted as a path,
//! printed `not a file or directory: --help`, and exited 0.

use std::process::Command;

fn write_tmp(tag: &str, src: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "hale_argparse_{}_{}.hl",
        std::process::id(),
        tag
    ));
    std::fs::write(&path, src).expect("write program");
    path
}

fn hale(args: &[&std::ffi::OsStr]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .output()
        .expect("run hale");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

const OK_SRC: &str = r#"
    type T { v: Int; }
    locus Q { params { n: Int = 0; } fn go() -> Int { return self.n; } }
    main locus App { params { q: Q = Q { }; } }
    fn main() { App { }; }
"#;

#[test]
fn help_is_answered_not_treated_as_a_path() {
    for cmd in ["check", "verify"] {
        let (out, code) = hale(&[cmd.as_ref(), "--help".as_ref()]);
        assert_eq!(code, 0, "`hale {} --help` must succeed: {}", cmd, out);
        assert!(
            !out.contains("not a file or directory"),
            "`--help` must not be read as a target: {}",
            out
        );
        // it must document the surface the review found undiscoverable
        for flag in
            ["--dump-topology", "--check-topology", "--check-topology-shape"]
        {
            assert!(
                out.contains(flag),
                "`hale {} --help` must list `{}`: {}",
                cmd,
                flag,
                out
            );
        }
    }
}

#[test]
fn an_unknown_flag_is_a_usage_error() {
    let path = write_tmp("unknown", OK_SRC);
    let (out, code) =
        hale(&["check".as_ref(), path.as_os_str(), "--typo-gate".as_ref()]);
    let _ = std::fs::remove_file(&path);
    assert_eq!(
        code, 2,
        "a misspelled flag must fail — silently ignoring it means a \
         CI gate that checks nothing and reports green: {}",
        out
    );
    assert!(out.contains("--typo-gate"), "name the offender: {}", out);
}

#[test]
fn a_second_positional_is_a_usage_error() {
    let a = write_tmp("pos_a", OK_SRC);
    let b = write_tmp("pos_b", OK_SRC);
    let (out, code) =
        hale(&["check".as_ref(), a.as_os_str(), b.as_os_str()]);
    let _ = std::fs::remove_file(&a);
    let _ = std::fs::remove_file(&b);
    assert_eq!(
        code, 2,
        "checking only the first of two targets reports on less than \
         it was handed: {}",
        out
    );
}

#[test]
fn a_missing_target_is_a_usage_error() {
    let (out, code) = hale(&["check".as_ref(), "--dump-topology".as_ref()]);
    assert_eq!(code, 2, "no target is a usage error: {}", out);
}

/// Flags on either side of the target must behave identically.
#[test]
fn flags_may_precede_or_follow_the_target() {
    let path = write_tmp("order", OK_SRC);
    let (_, after) =
        hale(&["check".as_ref(), path.as_os_str(), "--json".as_ref()]);
    let (_, before) =
        hale(&["check".as_ref(), "--json".as_ref(), path.as_os_str()]);
    let _ = std::fs::remove_file(&path);
    assert_eq!(after, 0, "flag after target");
    assert_eq!(before, 0, "flag before target");
}

/// The one that matters most: `--dump-topology` takes an OPTIONAL
/// value, so "consume the next token" is unresolvable — and it used
/// to consume the target. With flags now legal before the target,
/// `hale check --dump-topology app.hl` wrote the artifact OVER
/// `app.hl`. Losing the file you asked the tool to inspect is the
/// worst available reading of an ambiguous argument.
///
/// The destination is `=<path>` only; bare means stdout.
#[test]
fn dump_topology_never_overwrites_the_target() {
    let path = write_tmp("noclobber", OK_SRC);
    let before = std::fs::read_to_string(&path).expect("read back");

    let (out, code) = hale(&[
        "check".as_ref(),
        "--dump-topology".as_ref(),
        path.as_os_str(),
    ]);
    let after = std::fs::read_to_string(&path).expect("source still there");
    let _ = std::fs::remove_file(&path);

    assert_eq!(
        before, after,
        "the source file must be untouched — it was overwritten with \
         the artifact"
    );
    assert_eq!(code, 0, "the program is valid: {}", out);
    assert!(
        out.contains("\"shape_hash\""),
        "a bare --dump-topology writes the artifact to stdout: {}",
        out
    );
}
