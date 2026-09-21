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
//!
//! GH #861 brought `build` and `run` the rest of the way: their
//! flags were reachable only AFTER the target, because the target
//! was always argv[2] and `parse_build_options` read from argv[3].
//! The same file also holds the two usage-drift guards — every
//! dispatched subcommand is listed, and each command's flag block
//! sits under the command it belongs to.

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
    "parse", "replay", "run", "targets", "test", "topology", "verify",
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
/// wherever it appears, and `build` — which takes real flags on
/// either side of its target (GH #861) — still calls it an unknown
/// flag rather than guessing.
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

// ---------------------------------------------------------------
// GH #861: usage drift, and flags on either side of the target
// ---------------------------------------------------------------

/// `hale build`'s flags were reachable only AFTER the target: the
/// target was always argv[2] and `parse_build_options` read from
/// argv[3], so `hale build --dev app.hl` died with `not a file or
/// directory: --dev` while `hale check --json app.hl` was fine.
/// Both orders now name the same command.
#[test]
fn build_takes_flags_on_either_side_of_the_target() {
    let path = write_tmp("flagorder", OK_SRC);
    // `hale build app.hl` -> `./app`: the basename, minus `.hl`.
    let bin = path.with_extension("");

    for args in [
        vec!["build".to_string(), "--dev".into(), path.display().to_string()],
        vec!["build".to_string(), path.display().to_string(), "--dev".into()],
        // A value-taking flag must not donate its value to the
        // target: without the arity, `native` — the first argument
        // that does not start with `-` — would be the target.
        vec![
            "build".to_string(),
            "--target".into(),
            "native".into(),
            path.display().to_string(),
        ],
    ] {
        let _ = std::fs::remove_file(&bin);
        let owned: Vec<&std::ffi::OsStr> =
            args.iter().map(|a| a.as_ref()).collect();
        let (out, code) = hale(&owned);
        assert_eq!(code, 0, "`hale {}` must build: {}", args.join(" "), out);
        assert!(
            !out.contains("not a file or directory"),
            "the flag was read as the target: {}",
            out
        );
        assert!(
            bin.is_file(),
            "`hale {}` emitted no binary: {}",
            args.join(" "),
            out
        );
    }

    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&path);
}

/// GH #904: `hale run`'s own flags go before the target, because
/// everything after it is the program's argv — and the flags it
/// takes are `hale build`'s, parsed by `hale build`'s parser. `run`
/// used to compile with `BuildOptions::default()` whatever was
/// passed, so a build flag was first read as the target and then
/// (GH #861/#900) named and refused: the documented spot-check
/// could exercise neither a dev build nor an FFI program.
#[test]
fn run_takes_the_build_options_and_names_the_rest() {
    let path = write_tmp("runflag", OK_SRC);
    let (out, code) =
        hale(&["run".as_ref(), "--dev".as_ref(), path.as_os_str()]);
    assert_eq!(code, 0, "`hale run --dev` must build and run: {}", out);
    assert!(
        !out.contains("not a file or directory"),
        "the flag was read as the target: {}",
        out
    );

    // A flag no command has is still named, against `run`.
    let (out, code) =
        hale(&["run".as_ref(), "--bogus".as_ref(), path.as_os_str()]);
    assert_eq!(code, 2, "an unknown flag is a usage error: {}", out);
    assert!(
        out.contains("unknown `hale run` flag: --bogus"),
        "name the offender, against the command typed: {}",
        out
    );

    // A `hale build` flag that reports ON a build rather than
    // changing it: refused by name, not accepted and dropped.
    let (out, code) =
        hale(&["run".as_ref(), "--strict".as_ref(), path.as_os_str()]);
    assert_eq!(code, 2, "a build-only flag is a usage error: {}", out);
    assert!(
        out.contains("--strict") && out.contains("`hale build` flag"),
        "say whose flag it is: {}",
        out
    );

    // `run` execs what it builds, so a target it cannot exec is a
    // refusal rather than an artifact it then fails to run.
    let (out, code) = hale(&[
        "run".as_ref(),
        "--target".as_ref(),
        "wasm32".as_ref(),
        path.as_os_str(),
    ]);
    let _ = std::fs::remove_file(&path);
    assert_eq!(code, 2, "wasm32 is not executable here: {}", out);
    assert!(
        out.contains("cannot execute"),
        "say why, and what to run instead: {}",
        out
    );
}

/// The option has to reach CODEGEN, not just the parser: `--csrc`
/// compiles and links a C source, so a program calling an `@ffi`
/// symbol links under `hale run --csrc` and fails to link without
/// it. Under the old `BuildOptions::default()` no `hale run`
/// invocation could link one at all.
#[test]
fn run_honors_a_build_option_that_changes_the_binary() {
    let c = std::env::temp_dir().join(format!(
        "hale_argparse_{}_ffi.c",
        std::process::id()
    ));
    std::fs::write(
        &c,
        "long long shim_add(long long a, long long b) { return a + b; }\n",
    )
    .expect("write the C source");
    let path = write_tmp(
        "ffi",
        "@ffi(\"c\") fn shim_add(a: Int, b: Int) -> Int;\n\
         fn main() { println(\"sum=\", shim_add(40, 2)); }\n",
    );

    let (out, code) = hale(&["run".as_ref(), path.as_os_str()]);
    assert_ne!(code, 0, "the symbol is undefined without --csrc: {}", out);

    let (out, code) = hale(&[
        "run".as_ref(),
        "--csrc".as_ref(),
        c.as_os_str(),
        path.as_os_str(),
    ]);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&c);
    assert_eq!(code, 0, "`hale run --csrc` must link and run: {}", out);
    assert!(out.contains("sum=42"), "the C symbol ran: {}", out);
}

/// The subcommands `fn main` dispatches, read out of the dispatch
/// itself so this test cannot be the thing that goes stale. Two
/// shapes: the `if cmd == "name"` early returns, and the arms of the
/// trailing `match cmd.as_str()`.
fn dispatched_subcommands() -> Vec<String> {
    const SRC: &str = include_str!("../src/main.rs");
    let start = SRC
        .find("fn main() -> ExitCode {")
        .expect("hale-cli's `fn main` — the dispatch this test reads");
    let body = &SRC[start..];
    // A top-level `}` at column 0 ends the function; everything
    // inside it is indented.
    let body = &body[..body.find("\n}\n").expect("the end of `fn main`")];

    let mut names: Vec<String> = Vec::new();
    let mut rest = body;
    while let Some(i) = rest.find("cmd == \"") {
        let after = &rest[i + "cmd == \"".len()..];
        let end = after.find('"').expect("a closing quote");
        names.push(after[..end].to_string());
        rest = &after[end..];
    }
    let m = body
        .find("match cmd.as_str() {")
        .expect("the trailing dispatch match");
    for line in body[m..].lines() {
        let t = line.trim_start();
        let Some(r) = t.strip_prefix('"') else { continue };
        let Some(q) = r.find('"') else { continue };
        if r[q + 1..].trim_start().starts_with("=>") {
            names.push(r[..q].to_string());
        }
    }
    // `--version` / `--help` and their word spellings are not
    // subcommands: the usage lists them in its own block at the
    // bottom.
    names.retain(|n| !n.starts_with('-') && n != "version" && n != "help");
    names.sort();
    names.dedup();
    names
}

/// Is this the usage line for `cmd`? (`    hale build <file.hl …`)
fn is_usage_line(line: &str, cmd: &str) -> bool {
    line.trim_start()
        .strip_prefix("hale ")
        .and_then(|r| r.strip_prefix(cmd))
        .is_some_and(|r| r.is_empty() || r.starts_with(' '))
}

/// The continuation lines directly under a command's usage line —
/// the block a reader takes for that command's flags.
fn flag_block_after(help: &str, cmd: &str) -> String {
    let mut lines = help.lines().skip_while(|l| !is_usage_line(l, cmd));
    lines.next();
    lines
        .take_while(|l| {
            let t = l.trim_start();
            t.starts_with('[') || t.starts_with('-')
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `hale node` had been dispatched since GH #566 and appeared in no
/// usage at all — a subcommand you could only find by reading
/// `main.rs`; so had `hale targets`. The list is derived from the
/// dispatch, so the next one cannot go missing quietly, and it must
/// agree with the set `every_subcommand_answers_help` drives: one
/// list of subcommands, checked from both ends.
#[test]
fn usage_lists_every_dispatched_subcommand() {
    let dispatched = dispatched_subcommands();
    assert!(
        dispatched.len() > 15,
        "the source scan found almost nothing — it has stopped \
         reading the dispatch and this test proves nothing: {:?}",
        dispatched
    );
    let (help, code) = hale_in_scratch("dispatch", &["--help".as_ref()]);
    assert_eq!(code, 0, "`hale --help` must succeed: {}", help);
    for cmd in &dispatched {
        assert!(
            help.lines().any(|l| is_usage_line(l, cmd)),
            "`hale {}` is dispatched but `hale --help` never lists \
             it: {}",
            cmd,
            help
        );
    }
    let mut answering: Vec<&str> = SUBCOMMANDS.to_vec();
    answering.sort();
    let dispatched: Vec<&str> =
        dispatched.iter().map(String::as_str).collect();
    assert_eq!(
        dispatched, answering,
        "the dispatch and the `--help` set must be the same list"
    );
    let _ = std::fs::remove_dir_all(scratch_dir("dispatch"));
}

/// `replay`'s flag block was indented under the `hale dna` line, so
/// `--feed`, `--at` and the three `--allow-*` read as `dna`'s — the
/// one command whose flags nobody could look up, listed against the
/// one command that does not take them.
#[test]
fn replay_flags_are_listed_under_replay() {
    let (help, code) =
        hale_in_scratch("replayflags", &["--help".as_ref()]);
    assert_eq!(code, 0, "`hale --help` must succeed: {}", help);
    let under_replay = flag_block_after(&help, "replay");
    for flag in [
        "--diff",
        "--json",
        "--at",
        "--feed",
        "--allow-unmatched-feed",
        "--allow-live-effects",
        "--allow-unverified-model",
        "--allow-truncated",
    ] {
        assert!(
            under_replay.contains(flag),
            "`{}` must be listed under `hale replay`: {}",
            flag,
            help
        );
    }
    assert!(
        !flag_block_after(&help, "dna").contains("--feed"),
        "replay's flags must not read as `hale dna`'s: {}",
        help
    );
    let _ = std::fs::remove_dir_all(scratch_dir("replayflags"));
}
