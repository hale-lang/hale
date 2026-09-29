use std::sync::atomic::Ordering;
use crate::verbs::check::cli::run_check_cli;
use crate::verbs;
use std::process::ExitCode;
/// GH #527 B4: `hale model diff <a> <b> [--json|--text]`.
pub(crate) fn run_model_diff(rest: &[String]) -> ExitCode {
    let mut paths: Vec<&String> = Vec::new();
    let mut text = false;
    for a in rest {
        match a.as_str() {
            "--json" => text = false,
            "--text" => text = true,
            "--help" | "-h" => {
                eprintln!("usage: hale model diff <a.topology> <b.topology> [--json|--text]");
                return ExitCode::SUCCESS;
            }
            f if f.starts_with("--") => {
                eprintln!("hale model diff: unknown flag `{f}`");
                return ExitCode::from(2);
            }
            _ => paths.push(a),
        }
    }
    if paths.len() != 2 {
        eprintln!("usage: hale model diff <a.topology> <b.topology> [--json|--text]");
        return ExitCode::from(2);
    }
    let mut admitted = Vec::new();
    for p in &paths {
        let raw = match std::fs::read_to_string(p) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("hale model diff: cannot read {p}: {e}");
                return ExitCode::from(2);
            }
        };
        match hale_types::topology_diff::admit(p, &raw) {
            Ok(a) => admitted.push(a),
            Err(e) => {
                eprintln!("hale model diff: {e}");
                return ExitCode::from(2);
            }
        }
    }
    let d = hale_types::topology_diff::diff(&admitted[0], &admitted[1]);
    if text {
        print!("{}", hale_types::topology_diff::render_text(&d));
    } else {
        println!("{}", serde_json::to_string_pretty(&d).unwrap_or_default());
    }
    ExitCode::SUCCESS
}

/// The `hale model` surface. One text for two callers: the usage
/// error (stderr, exit 2) and `--help` (stdout, exit 0).
pub(crate) fn model_usage() -> &'static str {
    "\
usage: hale model dump <file.hl | dir>
       hale model diff <a.topology> <b.topology> [--json|--text]

`dump` derives the canonical ApplicationModel (GH #476) and prints an internal,
non-stable dump (experimental, pre-1.0).
`diff` compares two --dump-topology artifacts: declarations (added / removed /
renamed / moved / split / joined / ambiguous), per-locus contract deltas, effect
and certificate deltas, law and adequacy deltas, and a source-only vs model-shape
classification. JSON (versioned, digest-bearing) by default; --text for a review view.
"
}

/// GH #265: minimal line diff for the effect-manifest gate — enough
/// to show WHICH fn's effects changed without pulling in a diff
/// crate. Lines are stable-sorted by fn name, so a set difference is
/// an accurate rendering.
pub(crate) fn diff_lines(expected: &str, current: &str) -> Vec<String> {
    use std::collections::BTreeSet;
    let a: BTreeSet<&str> = expected.lines().collect();
    let b: BTreeSet<&str> = current.lines().collect();
    let mut out = Vec::new();
    for gone in a.difference(&b) {
        out.push(format!("  - {}", gone));
    }
    for added in b.difference(&a) {
        out.push(format!("  + {}", added));
    }
    out
}

/// The dispatch arm `main` held inline for this verb, moved out verbatim (C5 step 8).
pub(crate) fn run_model_cmd(args: &[String]) -> ExitCode {
    let rest: Vec<String> = args.iter().skip(2).cloned().collect();
    // GH #527 B4: `hale model diff <a> <b> [--json|--text]` —
    // the semantic difference between two topology artifacts.
    if rest.first().map(String::as_str) == Some("diff") {
        return run_model_diff(&rest[1..]);
    }
    if rest.first().map(String::as_str) != Some("dump") {
        eprint!("{}", model_usage());
        return ExitCode::from(2);
    }
    // The check pipeline's dump section reads PROCESS argv (it
    // is a top-level-command scope), so the flag cannot ride the
    // rest-args the shim forwards; the shim marks the demand on
    // the process instead.
    verbs::check::MODEL_DUMP_DEMANDED.store(true, Ordering::Relaxed);
    let shim: Vec<String> = rest[1..].to_vec();
    return run_check_cli(&shim, false);
}
