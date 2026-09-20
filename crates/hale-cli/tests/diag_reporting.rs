//! Downstream handoff (2026-08-11) — two CLI diagnostic-reporting
//! defects, both worst on the duplicate-top-level-name error (the
//! easiest error to hit under the per-directory seed model):
//!
//! 1. The previous declaration's location was `{:?}`-formatted into
//!    the message (`... at Span { start: Pos(5), end: Pos(11) }`).
//!    It now rides as a structured related span, rendered by the
//!    text renderer as `note: previous declaration at path:line:col`
//!    and by `--json` as a `related` array.
//! 2. The `apply_sync_inference` pre-pass printed its resolver
//!    diagnostics through a bare `render()` + early bail: no
//!    filename, `--json` ignored (empty stdout, exit 1 — a CI gate
//!    saw a failed build with zero explaining diagnostics), wrong
//!    stream, and multi-file positions resolved against the wrong
//!    file's text. The pre-pass diags are now discarded and
//!    re-raised by `check_bundle` through the normal reporting path
//!    — exactly what `hale lsp` always did, which is why the LSP
//!    attributed the same diagnostic correctly while the CLI did
//!    not.
//!
//! GH #777 is the third instance of defect 2, in the one remaining
//! place that still printed before the reporting path existed:
//! `parse_files`, which reads the target's own files. A seed that does
//! not PARSE failed `check --json` with an empty stdout — exit 1 and
//! nothing saying why, for every syntactic error there is. Parse
//! diagnostics now travel to the same site every other finding goes
//! through, so `--json` carries one record per parse error and the
//! text rendering is untouched.

use std::path::{Path, PathBuf};
use std::process::Command;

fn seed_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("hale_diagrep_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir seed");
    d
}

/// (stdout, stderr, exit code) for a subcommand that takes a target.
fn hale_cmd(cmd: &str, args: &[&str], target: &Path) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg(cmd)
        .args(args)
        .arg(target)
        .output()
        .unwrap_or_else(|e| panic!("run hale {}: {}", cmd, e));
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// (stdout, stderr, exit code)
fn hale_check(args: &[&str], target: &Path) -> (String, String, i32) {
    hale_cmd("check", args, target)
}

#[test]
fn duplicate_name_json_emits_ndjson_with_its_file() {
    // Two files in one seed, each declaring `main`. `--json` must
    // emit NDJSON on stdout naming the SECOND file at a position
    // inside it — not plain text on stderr, and never an empty
    // stdout with exit 1.
    let d = seed_dir("json");
    std::fs::write(d.join("a.hl"), "fn main() {\n    println(\"a\");\n}\n")
        .unwrap();
    std::fs::write(d.join("b.hl"), "fn main() {\n    println(\"b\");\n}\n")
        .unwrap();

    let (stdout, stderr, code) = hale_check(&["--json"], &d);
    assert_eq!(code, 1, "duplicate main fails the check");
    assert!(
        !stdout.trim().is_empty(),
        "--json must not produce an empty stdout on a failing check \
         (stderr was: {})",
        stderr
    );
    let line = stdout.lines().next().unwrap();
    let v: serde_json::Value =
        serde_json::from_str(line).expect("stdout is NDJSON");
    assert!(
        v["file"].as_str().unwrap().ends_with("b.hl"),
        "the second declaration is the one to change: {}",
        line
    );
    assert_eq!(v["line"], 1, "a position inside b.hl: {}", line);
    assert!(
        v["message"].as_str().unwrap().contains("duplicate top-level"),
        "got: {}",
        line
    );
    // The previous declaration rides as structured related info.
    assert!(
        v["related"][0]["file"].as_str().unwrap().ends_with("a.hl"),
        "related names the first declaration's file: {}",
        line
    );
    assert!(
        !stderr.contains("duplicate top-level"),
        "diagnostics belong on stdout under --json, not stderr: {}",
        stderr
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn duplicate_name_plain_names_both_locations() {
    let d = seed_dir("plain");
    std::fs::write(
        d.join("a.hl"),
        "type Widget { n: Int; }\n\nfn main() {\n    println(\"a\");\n}\n",
    )
    .unwrap();
    std::fs::write(d.join("b.hl"), "type Widget { m: Int; }\n").unwrap();

    let (stdout, stderr, code) = hale_check(&[], &d);
    let all = format!("{}{}", stdout, stderr);
    assert_eq!(code, 1);
    assert!(
        all.contains("b.hl:1:6"),
        "the duplicate resolves to ITS file's coordinates: {}",
        all
    );
    assert!(
        all.contains("note: previous declaration at")
            && all.contains("a.hl:1:6"),
        "the first declaration renders as path:line:col: {}",
        all
    );
    assert!(
        !all.contains("Span {"),
        "no Debug-formatted span may reach a user: {}",
        all
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn ordinary_json_shape_is_unchanged() {
    // The control from the report: an ordinary error keeps its
    // exact NDJSON shape, `related`-free.
    let d = seed_dir("control");
    std::fs::write(
        d.join("a.hl"),
        "fn main() {\n    let x: Int = \"nope\";\n    println(x);\n}\n",
    )
    .unwrap();

    let (stdout, _, code) = hale_check(&["--json"], &d);
    assert_eq!(code, 1);
    let v: serde_json::Value =
        serde_json::from_str(stdout.lines().next().unwrap()).expect("NDJSON");
    assert!(v.get("related").is_none(), "no related key when empty: {}", v);
    assert!(v["file"].as_str().unwrap().ends_with("a.hl"));
    let _ = std::fs::remove_dir_all(&d);
}

/// Downstream handoff (2026-08-11), soundness follow-through: the
/// handler/payload mismatch must refuse to BUILD, not just fail
/// `check` — before the fix it built and printed a live heap
/// address from safe code.
#[test]
fn payload_mismatch_refuses_to_build() {
    let d = seed_dir("buspay");
    std::fs::write(
        d.join("main.hl"),
        r#"
        type Greeting { text: String = "hello"; n: Int = 42; }
        type Other { a: Int = 0; b: Int = 0; }
        topic Hello { payload: Greeting; subject: "hello"; }
        locus Pub {
            bus { publish Hello; }
            birth() { Hello <- Greeting { text: "P", n: 7 }; }
        }
        locus Sub {
            bus { subscribe Hello as on_hello; }
            fn on_hello(msg: Other) { println("a=", msg.a); }
        }
        fn main() { Sub { }; Pub { }; }
        "#,
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("build")
        .arg(&d)
        .output()
        .expect("run hale build");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !out.status.success(),
        "the reinterpreting program must not build: {}",
        all
    );
    assert!(
        all.contains("carries payload `Greeting`"),
        "the refusal names the mismatch: {}",
        all
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// GH #725: a reserved word used as a name, in a multi-FILE seed.
///
/// Two defects met here. The parser abandoned the whole declaration,
/// so the author's first line was the wreckage downstream rather than
/// the word they wrote. And `parse_source_at` shifted a parse
/// diagnostic's span by the file's virtual base a SECOND time (the
/// spans already came from base-shifted tokens), so in every file but
/// the first of a seed the error rendered `base` bytes too far — for
/// this fixture line 15 of a 14-line file, past EOF, which also cost
/// it the source snippet and caret. `check` bails before typechecking
/// a seed with a parse hole, so the historical "missing type
/// WorkResult" reports were what a mislocated parse error looked
/// like, not a separate diagnostic. The assertion pins both halves:
/// one parse error, at the word, inside the file that holds it.
#[test]
fn reserved_word_in_a_sibling_file_is_located_at_the_word() {
    let d = seed_dir("reserved");
    // Sorted first, so `work.hl` below is parsed at a NON-ZERO base.
    std::fs::write(
        d.join("app.hl"),
        "main locus App {\n\
         \x20   run {\n\
         \x20       let w = Worker { };\n\
         \x20       let r: WorkResult = w.produce();\n\
         \x20       println(\"{}\", r.tally);\n\
         \x20   }\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        d.join("work.hl"),
        "type WorkResult {\n\
         \x20   ok: Bool;\n\
         \x20   tally: Int;\n\
         }\n\
         \n\
         locus Worker {\n\
         \x20   params {\n\
         \x20       epoch: Int = 0;\n\
         \x20   }\n\
         \n\
         \x20   fn produce() -> WorkResult {\n\
         \x20       return WorkResult { ok: true, tally: 1 };\n\
         \x20   }\n\
         }\n",
    )
    .unwrap();

    let (_, stderr, code) = hale_check(&[], &d);
    assert_eq!(code, 1, "the seed must not check: {}", stderr);
    let errs: Vec<&str> =
        stderr.lines().filter(|l| l.contains("error:")).collect();
    assert_eq!(
        errs.len(),
        1,
        "one diagnostic for one mistake; got:\n{}",
        stderr
    );
    assert!(
        errs[0].contains("work.hl:8:9:")
            && errs[0].contains(
                "`epoch` is a reserved word and cannot name a params field"
            ),
        "the error is at the reserved word in work.hl; got: {}",
        errs[0]
    );
    // The snippet + caret only render when the span really is inside
    // the file — the double-shifted span pointed past EOF and lost
    // them silently.
    assert!(
        stderr.contains("epoch: Int = 0;") && stderr.contains("^^^^^"),
        "the caret underlines the word: {}",
        stderr
    );
    // And nothing about the type the sibling file reads from it.
    assert!(
        !stderr.contains("WorkResult") && !stderr.contains("unknown"),
        "no follow-on about the sibling's view of the seed: {}",
        stderr
    );
    let _ = std::fs::remove_dir_all(&d);
}

// ---------------------------------------------------------------
// GH #777: a parse error is a record under `--json` too.
//
// `parse_files` predates the JSON reporting path: it rendered parse
// diagnostics to stderr as text and handed back a bare exit code, so
// `hale check --json` on a seed that does not parse exited non-zero
// with an EMPTY stdout. A CI gate, an admission step or an LSP client
// consuming the machine-readable channel saw a real failure with
// nothing explaining it, for every syntactic error there is. The
// diagnostics now reach the same reporting site the checker's own
// findings do.
// ---------------------------------------------------------------

/// A seed whose only fault is syntactic: `where` is a reserved word
/// (`spec/tokens.md`), so this does not parse. Line 2, column 9.
const RESERVED_WORD_SEED: &str =
    "fn main() {\n    let where = 1;\n    println(\"x\");\n}\n";

#[test]
fn parse_error_is_an_ndjson_record() {
    let d = seed_dir("parsejson");
    std::fs::write(d.join("main.hl"), RESERVED_WORD_SEED).unwrap();

    let (stdout, stderr, code) = hale_check(&["--json"], &d);
    assert_eq!(code, 1, "a seed that does not parse fails check");
    assert!(
        !stdout.trim().is_empty(),
        "--json must not answer a parse failure with an empty stream \
         (stderr was: {})",
        stderr
    );
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 1, "one record for one mistake: {}", stdout);
    let v: serde_json::Value =
        serde_json::from_str(lines[0]).expect("stdout is NDJSON");
    assert!(
        v["file"].as_str().unwrap().ends_with("main.hl"),
        "the record names the file: {}",
        lines[0]
    );
    assert_eq!(v["line"], 2, "the line the word is on: {}", lines[0]);
    assert_eq!(v["col"], 9, "the column the word starts at: {}", lines[0]);
    assert_eq!(v["severity"], "error");
    assert_eq!(
        v["kind"], "parse error",
        "a parse error says so: {}",
        lines[0]
    );
    assert!(
        v["message"].as_str().unwrap().contains("reserved word"),
        "got: {}",
        lines[0]
    );
    // Under `--json` the diagnostics are the stdout stream; the text
    // rendering must not also go out on stderr.
    assert!(
        !stderr.contains("reserved word"),
        "diagnostics belong on stdout under --json: {}",
        stderr
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// The position has to survive the merged coordinate space: every
/// file is parsed at its own virtual base, and a parse error in the
/// second or later file of a seed used to be shifted by that base
/// twice (GH #776 / #765). The JSON channel is where a mislocation is
/// invisible — it comes out as `"file":""`.
#[test]
fn parse_error_in_a_sibling_file_carries_its_own_position() {
    let d = seed_dir("parsejson2");
    // Sorted first, so `b.hl` is parsed at a NON-ZERO base.
    std::fs::write(d.join("a.hl"), "fn helper() -> Int {\n    return 1;\n}\n")
        .unwrap();
    std::fs::write(
        d.join("b.hl"),
        "main locus App {\n    run {\n        let tier = 1;\n        \
         println(\"t\");\n    }\n}\n",
    )
    .unwrap();

    let (stdout, _, code) = hale_check(&["--json"], &d);
    assert_eq!(code, 1);
    let line = stdout.lines().next().unwrap_or("");
    let v: serde_json::Value =
        serde_json::from_str(line).expect("stdout is NDJSON");
    assert!(
        v["file"].as_str().unwrap().ends_with("b.hl"),
        "the file holding the mistake, not the first of the seed: {}",
        line
    );
    assert_eq!(v["line"], 3, "`tier` is on line 3 of b.hl: {}", line);
    assert_eq!(v["col"], 13, "and at column 13: {}", line);
    let _ = std::fs::remove_dir_all(&d);
}

/// The commonest parse error of all sits at EOF — a missing closing
/// brace reports `expected }, got Eof`, and the `Eof` token's span is
/// the one-past-the-last-byte position. The file-base windows were
/// half-open, so that position belonged to NO file: routing parse
/// errors through the shared renderers would have lost the filename
/// from the text rendering and emitted `"file":"","line":0,"col":0`
/// under `--json`. Both channels keep the file and the position.
#[test]
fn parse_error_at_eof_keeps_its_file() {
    let d = seed_dir("parseeof");
    std::fs::write(d.join("main.hl"), "fn main() {\n    println(\"x\");\n")
        .unwrap();

    let (stdout, _, code) = hale_check(&["--json"], &d);
    assert_eq!(code, 1);
    let line = stdout.lines().next().unwrap_or("");
    let v: serde_json::Value =
        serde_json::from_str(line).expect("stdout is NDJSON");
    assert!(
        v["file"].as_str().unwrap().ends_with("main.hl"),
        "an EOF-positioned error still names its file: {}",
        line
    );
    assert_eq!(v["line"], 3, "the line past the last one: {}", line);
    assert_eq!(v["col"], 1, "at its start: {}", line);
    assert!(
        v["message"].as_str().unwrap().contains("Eof"),
        "got: {}",
        line
    );

    // The text rendering is the same as it always was.
    let (_, stderr, code) = hale_check(&[], &d);
    assert_eq!(code, 1);
    assert!(
        stderr.contains("main.hl:3:1: parse error: expected }, got Eof"),
        "the text path is unchanged: {}",
        stderr
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// Two broken files, two records: nothing is collapsed or dropped on
/// the way to the one reporting site.
#[test]
fn every_parse_error_gets_its_own_record() {
    let d = seed_dir("parsemany");
    std::fs::write(
        d.join("a.hl"),
        "fn a() -> Int {\n    let where = 1;\n    return 2;\n}\n",
    )
    .unwrap();
    std::fs::write(
        d.join("b.hl"),
        "main locus App {\n    run {\n        let tier = 2;\n    }\n}\n",
    )
    .unwrap();

    let (stdout, _, code) = hale_check(&["--json"], &d);
    assert_eq!(code, 1);
    let files: Vec<String> = stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let v: serde_json::Value =
                serde_json::from_str(l).expect("stdout is NDJSON");
            v["file"].as_str().unwrap().to_string()
        })
        .collect();
    assert_eq!(files.len(), 2, "one record per parse error: {}", stdout);
    assert!(
        files[0].ends_with("a.hl") && files[1].ends_with("b.hl"),
        "each names the file it came from: {}",
        stdout
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// The human path is the control: located text with the snippet and
/// caret, on stderr, and nothing on stdout.
#[test]
fn parse_error_text_rendering_is_unchanged() {
    let d = seed_dir("parsetext");
    std::fs::write(d.join("main.hl"), RESERVED_WORD_SEED).unwrap();

    let (stdout, stderr, code) = hale_check(&[], &d);
    assert_eq!(code, 1);
    assert!(
        stdout.trim().is_empty(),
        "text mode writes no stdout: {}",
        stdout
    );
    assert!(
        stderr.contains("main.hl:2:9: parse error:")
            && stderr.contains("reserved word"),
        "located on stderr as before: {}",
        stderr
    );
    assert!(
        stderr.contains("let where = 1;") && stderr.contains("^^^^^"),
        "with its snippet and caret: {}",
        stderr
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// `verify` shares the reporting path, so it reports the same record.
/// A seed that does not parse has nothing to verify, and answering
/// with an empty stream is exactly the fail-open shape a gate cannot
/// tell from a clean run.
#[test]
fn verify_json_reports_a_parse_error_too() {
    let d = seed_dir("parseverify");
    std::fs::write(d.join("main.hl"), RESERVED_WORD_SEED).unwrap();

    let (stdout, _, code) = hale_cmd("verify", &["--json"], &d);
    assert_ne!(code, 0, "verify refuses a seed that does not parse");
    let line = stdout.lines().next().unwrap_or("");
    let v: serde_json::Value =
        serde_json::from_str(line).expect("stdout is NDJSON");
    assert_eq!(v["kind"], "parse error", "got: {}", line);
    assert!(v["file"].as_str().unwrap().ends_with("main.hl"), "{}", line);
    assert_eq!(v["line"], 2, "{}", line);
    let _ = std::fs::remove_dir_all(&d);
}

/// And a seed that parses says nothing at all: the stream stays empty
/// on success, so an empty stdout with exit 0 keeps meaning "clean".
#[test]
fn a_clean_seed_emits_no_records() {
    let d = seed_dir("parseclean");
    std::fs::write(
        d.join("main.hl"),
        "fn main() {\n    println(\"ok\");\n}\n",
    )
    .unwrap();

    let (stdout, _, code) = hale_check(&["--json"], &d);
    assert_eq!(code, 0, "the seed checks clean");
    assert!(stdout.trim().is_empty(), "nothing to report: {}", stdout);
    let _ = std::fs::remove_dir_all(&d);
}
