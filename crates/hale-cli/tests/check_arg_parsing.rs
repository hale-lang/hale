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
//!
//! GH #817 finished that job for the rest of the CLI: `--help` /
//! `-h` as the first argument after ANY subcommand prints that
//! subcommand's usage and exits 0.

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

/// Same, from a scratch directory. A command that reads `--help` as
/// a target acts where it stands — `hale init --help` scaffolds a
/// project into a directory by that name, `hale fmt` rewrites the
/// tree it is in — and the test that proves it no longer does must
/// not do it inside the checkout.
fn hale_in_scratch(tag: &str, args: &[&std::ffi::OsStr]) -> (String, i32) {
    let dir = scratch_dir(tag);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .current_dir(&dir)
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

/// Per-test, not per-process: these tests run in parallel, and a
/// directory one of them removes is a directory another cannot spawn
/// in.
fn scratch_dir(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "hale_argparse_help_{}_{}",
        std::process::id(),
        tag
    ))
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

// ---------------------------------------------------------------
// GH #817: `--help` on every subcommand
// ---------------------------------------------------------------

/// Every command the top-level usage lists. `--help` is the one part
/// of the surface all of them have, so all of them answer it.
const SUBCOMMANDS: &[&str] = &[
    "bench", "build", "check", "dna", "doc", "fetch", "fleet", "fmt",
    "init", "inputs", "iris", "lex", "lsp", "mcp", "model", "node",
    "parse", "replay", "run", "test", "topology", "verify",
];

#[test]
fn every_subcommand_answers_help() {
    for cmd in SUBCOMMANDS {
        for flag in ["--help", "-h"] {
            let (out, code) =
                hale_in_scratch("every", &[cmd.as_ref(), flag.as_ref()]);
            assert_eq!(
                code, 0,
                "`hale {} {}` must print usage and succeed: {}",
                cmd, flag, out
            );
            assert!(
                out.contains(&format!("hale {}", cmd)),
                "`hale {} {}` must print a usage line naming the \
                 command: {}",
                cmd,
                flag,
                out
            );
            // The two shapes the flag used to take: a path, or a
            // flag nobody recognized.
            assert!(
                !out.contains("not a file")
                    && !out.contains("unknown flag"),
                "`hale {} {}` must not be read as an argument: {}",
                cmd,
                flag,
                out
            );
        }
    }
    let _ = std::fs::remove_dir_all(scratch_dir("every"));
}

/// The reported symptom, and the second half of the ask: `hale
/// build` has no `-o`, so its usage is the only place the output
/// path is stated.
#[test]
fn build_help_says_where_the_binary_lands() {
    let (out, code) =
        hale_in_scratch("build", &["build".as_ref(), "--help".as_ref()]);
    let _ = std::fs::remove_dir_all(scratch_dir("build"));
    assert_eq!(code, 0, "`hale build --help` must succeed: {}", out);
    assert!(
        !out.contains("not a file"),
        "`--help` must not be read as the target: {}",
        out
    );
    assert!(
        out.contains("no `-o`"),
        "`hale build` takes no -o; the usage has to say so: {}",
        out
    );
    assert!(
        out.contains("myapp/myapp"),
        "a directory target's binary lands inside it under the \
         directory's own name — the part no flag reveals: {}",
        out
    );
    for flag in ["--target", "--target-cpu", "--link", "--csrc", "--dev"]
    {
        assert!(
            out.contains(flag),
            "`hale build --help` must list `{}`: {}",
            flag,
            out
        );
    }
}

/// `hale --help` / `-h` / `help` keep listing the commands — the
/// subcommand pre-pass sits after them, not in front of them.
#[test]
fn the_top_level_help_still_lists_the_commands() {
    for arg in ["--help", "-h", "help"] {
        let (out, code) = hale_in_scratch("toplevel", &[arg.as_ref()]);
        assert_eq!(code, 0, "`hale {}` must succeed: {}", arg, out);
        for line in ["hale build", "hale check", "hale test"] {
            assert!(
                out.contains(line),
                "`hale {}` must still list `{}`: {}",
                arg,
                line,
                out
            );
        }
    }
    let _ = std::fs::remove_dir_all(scratch_dir("toplevel"));
}

/// Only the FIRST argument after the subcommand. Further along,
/// `--help` belongs to whatever is parsing there: `check` answers it
/// wherever it appears, and `build` — whose flags follow the target
/// — still calls it an unknown flag rather than guessing.
#[test]
fn help_after_the_target_keeps_its_own_meaning() {
    let path = write_tmp("help_after", OK_SRC);
    let (out, code) =
        hale(&["check".as_ref(), path.as_os_str(), "--help".as_ref()]);
    assert_eq!(code, 0, "check answers --help anywhere: {}", out);
    assert!(out.contains("--dump-topology"), "the flag list: {}", out);

    let (out, code) =
        hale(&["build".as_ref(), path.as_os_str(), "--help".as_ref()]);
    let _ = std::fs::remove_file(&path);
    assert_eq!(
        code, 2,
        "a flag after the target is `build`'s to reject: {}",
        out
    );
    assert!(
        out.contains("unknown `hale build` flag"),
        "and it says so: {}",
        out
    );
}
