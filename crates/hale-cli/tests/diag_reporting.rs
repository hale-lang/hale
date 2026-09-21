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
//!
//! GH #775 is the fourth, on the other side of the split: the
//! commands with no machine-readable channel — `build`, `run`, `test`,
//! `bench`, `replay` — reported an IMPORTED file's diagnostic with a
//! bare `d.render(source)`. Every file of an import graph is parsed at
//! its own virtual base, so the span is an offset into the merged
//! bundle while `source` is the one file; rendering one against the
//! other printed the right file and the right message at a line and
//! column that were not the error's. `check` and `verify` had been
//! right since GH #770. The diagnostics now carry the base they were
//! parsed at and render through it, so every command agrees with
//! `check` about where the mistake is.
//!
//! GH #806 is the last of the family, and the one failure that is not
//! a `Diag` at all: an input that could not be READ — a target that
//! is not there, a file of the seed or of the import graph that would
//! not open. It printed a sentence on stderr and left `--json` empty
//! with exit 1. It is now one record per unreadable input, at
//! `"line":0,"col":0` under `"kind":"io error"`, written by the same
//! writer every other record goes through.
//!
//! GH #822 is the same split one field over. With the position
//! settled, the FILE still differed by channel: `check` resolved it
//! through the canonical `file_bases` and `build` / `run` / `test`
//! through `ImportDiag`, which holds the path as the resolver
//! REACHED it (`/abs/app/../lib/second.hl`), and the target's own
//! files came out exactly as the command line spelled them. All of
//! them name the right file; none of them is a join key, and
//! `check --json`'s `file` field is used as one. Every renderer now
//! spells a path the one way — absolute, canonical, `..`-free.
//!
//! GH #860 closes the family: an `import` that names nothing was the
//! last failure on the check path still reported by an `eprintln!`
//! plus a bare `Err(())` — no record, exit 1. Nothing was opened, so
//! it is not an io failure, and the statement HAS a span: it is a
//! located diagnostic under the path literal now, carried in
//! `ImportDiag::Located` like an imported file's parse error and
//! rendered by every channel with no new path.
//!
//! GH #848 is the family's last member and the one failure that is
//! not the front end's: a program the CHECKER accepts and codegen
//! refuses. A `CodegenError` carries a span, and only `build` ever
//! used it — `run`, `test`, `bench` and `replay` printed the error
//! with `{:?}`, so the reader got `UnsupportedAt("…", Span { start:
//! Pos(55), end: Pos(60) })` and no line to open. All five now
//! report through one helper, so the located line is the same
//! string whichever command found it.

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

// GH #775: an imported file's diagnostic is positioned in its own
// file on the commands that do not go through the check reporting
// path.

/// The library's second file, broken on LINE 3 (the `;` before `}` is
/// missing). It is the second file of the library alphabetically, so
/// it is parsed at a non-zero base — and it is reached through an
/// `import`, so it is the resolver's `errors` vector that carries it,
/// not `parse_files`.
const IMPORTED_BROKEN: &str = "// a helper file\n\
                               fn ok() -> Int { return 1; }\n\
                               fn broken(x: Int) -> Int { return x * 2 }\n";

/// `<tmp>/app/{main,app_test}.hl` importing `<tmp>/lib/`, whose
/// `second.hl` does not parse.
fn import_seed(tag: &str) -> PathBuf {
    let d = seed_dir(tag);
    std::fs::create_dir_all(d.join("lib")).expect("mkdir lib");
    std::fs::create_dir_all(d.join("app")).expect("mkdir app");
    std::fs::write(
        d.join("lib").join("main.hl"),
        "fn greet() -> String {\n    return \"hi\";\n}\n",
    )
    .unwrap();
    std::fs::write(d.join("lib").join("second.hl"), IMPORTED_BROKEN).unwrap();
    std::fs::write(
        d.join("app").join("main.hl"),
        "import \"../lib\" as lib;\n\nfn main() {\n    \
         println(lib::greet());\n}\n",
    )
    .unwrap();
    std::fs::write(
        d.join("app").join("app_test.hl"),
        "import \"../lib\" as lib;\n\nfn main() {\n    \
         std::test::assert(lib::greet() == \"hi\", \"greet\");\n}\n",
    )
    .unwrap();
    d
}

/// The one position every command must print: line 3, column 41, of
/// the library's `second.hl`.
fn assert_located_in_second_hl(what: &str, out: &str) {
    assert!(
        out.contains("second.hl:3:41: parse error: expected ;, got RBrace"),
        "{what} must position the error in the file that holds it \
         (second.hl, line 3, col 41):\n{out}"
    );
    assert!(
        out.contains("fn broken(x: Int) -> Int { return x * 2 }")
            && out.contains('^'),
        "{what} must cut the snippet and caret from that file too:\n{out}"
    );
}

/// `build` on both target shapes. The directory target reports the
/// resolver's vector directly; the single-file target reports
/// `parse_with_imports`'s `Err`. They are separate call sites and
/// were wrong separately.
#[test]
fn build_positions_an_imported_parse_error_in_its_own_file() {
    let d = import_seed("import775build");

    let (_, stderr, code) = hale_cmd("build", &[], &d.join("app"));
    assert_eq!(code, 1, "build must refuse the tree:\n{stderr}");
    assert_located_in_second_hl("build <dir>", &stderr);

    let (_, stderr, code) =
        hale_cmd("build", &[], &d.join("app").join("main.hl"));
    assert_eq!(code, 1, "build must refuse the entry too:\n{stderr}");
    assert_located_in_second_hl("build <file>", &stderr);

    let _ = std::fs::remove_dir_all(&d);
}

/// `run` has the same two shapes and the same two sites.
#[test]
fn run_positions_an_imported_parse_error_in_its_own_file() {
    let d = import_seed("import775run");

    let (_, stderr, code) = hale_cmd("run", &[], &d.join("app"));
    assert_eq!(code, 1, "run must refuse the tree:\n{stderr}");
    assert_located_in_second_hl("run <dir>", &stderr);

    let (_, stderr, code) =
        hale_cmd("run", &[], &d.join("app").join("main.hl"));
    assert_eq!(code, 1, "run must refuse the entry too:\n{stderr}");
    assert_located_in_second_hl("run <file>", &stderr);

    let _ = std::fs::remove_dir_all(&d);
}

/// `test` compiles each `_test.hl` through its own pipeline and
/// reports a compile failure as that test's message — on stdout, with
/// the rest of the run's report.
#[test]
fn test_positions_an_imported_parse_error_in_its_own_file() {
    let d = import_seed("import775test");

    let (stdout, _, code) =
        hale_cmd("test", &[], &d.join("app").join("app_test.hl"));
    assert_eq!(code, 1, "the test must fail:\n{stdout}");
    assert!(stdout.contains("0 passed, 1 failed"), "got:\n{stdout}");
    assert_located_in_second_hl("test", &stdout);

    let _ = std::fs::remove_dir_all(&d);
}

/// `check` is the control: it has been right since GH #770, and the
/// other commands now print exactly what it prints. Pinning them to
/// each other is what stops the two paths drifting apart again.
///
/// GH #822 extends it from the position to the whole located prefix.
/// Once every command agreed about `line:col` the FILE was the last
/// thing left differing: `check` resolves the diagnostic through the
/// canonical `file_bases` and printed `/abs/lib/second.hl`, while
/// `build` / `run` / `test` render it from `ImportDiag`, which holds
/// the path as the resolver REACHED it — through the importer's own
/// directory, so `/abs/app/../lib/second.hl`. Both name the file and
/// both are clickable; neither is a join key, and `check --json`'s
/// `file` field is used as one. The seed is imported via `../`
/// precisely so that a `..` in the output is a failure.
#[test]
fn check_and_build_agree_about_the_position() {
    let d = import_seed("import775agree");
    let app = d.join("app");

    let (_, check_err, check_code) = hale_check(&[], &app);
    let (check_json, _, json_code) = hale_check(&["--json"], &app);
    let (_, build_err, build_code) = hale_cmd("build", &[], &app);
    let (_, run_err, run_code) = hale_cmd("run", &[], &app);
    let (test_out, _, test_code) =
        hale_cmd("test", &[], &app.join("app_test.hl"));
    assert_eq!(check_code, 1, "check refuses it:\n{check_err}");
    assert_eq!(json_code, 1, "check --json refuses it:\n{check_json}");
    assert_eq!(build_code, 1, "build refuses it:\n{build_err}");
    assert_eq!(run_code, 1, "run refuses it:\n{run_err}");
    assert_eq!(test_code, 1, "test refuses it:\n{test_out}");

    // The whole located prefix — `file:line:col` — of the one parse
    // error, from whichever stream that command reports on.
    let located = |what: &str, out: &str| -> String {
        out.lines()
            .find(|l| l.contains("parse error"))
            .and_then(|l| l.split(": parse error").next())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| {
                panic!("no located parse error from {what}:\n{out}")
            })
    };
    // `file:line:col` → `file`. The path may contain no `:` of its
    // own here; splitting from the RIGHT is what makes that true of
    // any path.
    let file_of = |prefix: &str| -> String {
        prefix
            .rsplitn(3, ':')
            .nth(2)
            .unwrap_or_else(|| panic!("not a located prefix: {prefix}"))
            .to_string()
    };

    let from_check = located("check", &check_err);
    for (what, out) in [
        ("build", &build_err),
        ("run", &run_err),
        ("test", &test_out),
    ] {
        assert_eq!(
            located(what, out),
            from_check,
            "{what} must name the file, line and column exactly as \
             check does\ncheck:\n{check_err}\n{what}:\n{out}"
        );
    }

    // The machine-readable channel reads the same string: `file` is
    // what a gate diffing `--json` against a build failure joins on.
    let record: serde_json::Value =
        serde_json::from_str(check_json.lines().next().unwrap_or(""))
            .unwrap_or_else(|e| panic!("stdout is NDJSON ({e}): {check_json}"));
    assert_eq!(
        record["file"].as_str().unwrap(),
        file_of(&from_check),
        "--json `file` is the text renderer's path: {check_json}"
    );

    // And the spelling itself: absolute, canonical, `..`-free — the
    // library is reached through `import \"../lib\"`, so the
    // as-reached path has a `..` in it and the canonical one cannot.
    let file = file_of(&from_check);
    assert!(
        !file.contains(".."),
        "a file reached through `../` is still named without one: \
         {file}"
    );
    assert_eq!(
        file,
        d.join("lib")
            .join("second.hl")
            .canonicalize()
            .expect("the library file exists")
            .display()
            .to_string(),
        "the canonical path of the file that holds the error"
    );

    let _ = std::fs::remove_dir_all(&d);
}

/// The control for the base-0 end of the same rule: a parse error in
/// the ENTRY file of a single-file target is parsed unshifted, and
/// must still come out at its own line and column rather than picking
/// up a base that is not its.
#[test]
fn a_parse_error_in_the_entry_file_keeps_position_zero() {
    let d = seed_dir("import775entry");
    std::fs::write(
        d.join("main.hl"),
        "fn a() -> Int {\n    return 1;\n}\n\nfn b() -> Int { return 2 }\n",
    )
    .unwrap();

    let (_, stderr, code) = hale_cmd("build", &[], &d.join("main.hl"));
    assert_eq!(code, 1, "build must refuse it:\n{stderr}");
    assert!(
        stderr.contains("main.hl:5:26: parse error: expected ;, got RBrace"),
        "the entry file's own position, unshifted:\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

// GH #806: an input that could not be READ is a record too — the
// last text-only hole on the check path. A target that is not there,
// a file of the seed that would not open, or a file of the IMPORT
// GRAPH that would not open printed a sentence on stderr and left
// `--json` empty with exit 1: the same false picture #777 retired
// for diagnostics, for a class that is an environment failure rather
// than a program one. A gate driving `--json` reads an empty stream
// plus a non-zero exit as "the tool crashed" either way.

/// Make `path` unreadable and answer whether it actually is. Running
/// as root (CI containers often do) defeats `chmod 000`, and a test
/// that silently asserts nothing is worse than one that skips.
fn make_unreadable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o000);
    std::fs::set_permissions(path, perms).unwrap();
    std::fs::read_to_string(path).is_err()
}

fn make_readable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(md) = std::fs::metadata(path) {
        let mut perms = md.permissions();
        perms.set_mode(0o644);
        let _ = std::fs::set_permissions(path, perms);
    }
}

/// The one record an unreadable input produces: positionless (there
/// is no text to have a position in), naming the path it is about,
/// under a kind a consumer can branch on.
fn assert_io_record(line: &str, ends_with: &str) -> serde_json::Value {
    let v: serde_json::Value =
        serde_json::from_str(line).expect("stdout is NDJSON");
    assert_eq!(v["kind"], "io error", "got: {}", line);
    assert_eq!(v["severity"], "error", "got: {}", line);
    assert_eq!(v["line"], 0, "no position to report: {}", line);
    assert_eq!(v["col"], 0, "no position to report: {}", line);
    assert!(
        v["file"].as_str().unwrap().ends_with(ends_with),
        "the record names the input it is about ({}): {}",
        ends_with,
        line
    );
    v
}

#[test]
fn a_missing_target_is_one_record() {
    let d = seed_dir("io806missing");
    let missing = d.join("not_here.hl");

    let (stdout, stderr, code) = hale_check(&["--json"], &missing);
    assert_eq!(code, 1, "a target that is not there fails the check");
    assert!(
        !stdout.trim().is_empty(),
        "--json must not answer a missing target with an empty \
         stream (stderr was: {})",
        stderr
    );
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 1, "exactly one record: {}", stdout);
    let v = assert_io_record(lines[0], "not_here.hl");
    assert!(
        v["message"].as_str().unwrap().contains("os error 2"),
        "the OS says why, rather than the prose text mode prints: {}",
        lines[0]
    );
    assert!(
        stderr.trim().is_empty(),
        "under --json the report belongs on stdout: {}",
        stderr
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_unreadable_file_of_the_seed_is_one_record() {
    let d = seed_dir("io806own");
    let f = d.join("main.hl");
    std::fs::write(&f, "fn main() {\n    println(\"x\");\n}\n").unwrap();
    if !make_unreadable(&f) {
        eprintln!("skipped: this user can read a 0o000 file (root?)");
        let _ = std::fs::remove_dir_all(&d);
        return;
    }

    let (stdout, stderr, code) = hale_check(&["--json"], &d);
    assert_eq!(code, 1, "a seed that cannot be read fails the check");
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 1, "exactly one record: {}", stdout);
    let v = assert_io_record(lines[0], "main.hl");
    assert!(
        v["message"].as_str().unwrap().contains("os error 13"),
        "the OS error, verbatim: {}",
        lines[0]
    );
    assert!(
        stderr.trim().is_empty(),
        "under --json the report belongs on stdout: {}",
        stderr
    );

    make_readable(&f);
    let _ = std::fs::remove_dir_all(&d);
}

/// A file reached only through an `import` is the same shape. The
/// resolver printed it and returned a bare failure, so the records
/// the rest of the import graph produces stopped exactly where a file
/// would not open — a library made unreadable by a bad checkout
/// looked, to a gate, like a clean seed that crashed the tool.
#[test]
fn an_unreadable_imported_file_is_one_record() {
    let d = import_seed("io806import");
    let broken = d.join("lib").join("second.hl");
    if !make_unreadable(&broken) {
        eprintln!("skipped: this user can read a 0o000 file (root?)");
        make_readable(&broken);
        let _ = std::fs::remove_dir_all(&d);
        return;
    }

    let (stdout, stderr, code) = hale_check(&["--json"], &d.join("app"));
    assert_eq!(code, 1, "the tree is refused");
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 1, "exactly one record: {}", stdout);
    let v = assert_io_record(lines[0], "second.hl");
    assert!(
        v["message"].as_str().unwrap().contains("os error 13"),
        "the OS error, verbatim: {}",
        lines[0]
    );
    assert!(
        stderr.trim().is_empty(),
        "under --json the report belongs on stdout: {}",
        stderr
    );

    make_readable(&broken);
    let _ = std::fs::remove_dir_all(&d);
}

/// The human path is the control: the same sentences on stderr and
/// nothing on stdout. `build` and `run` share two of these sites and
/// have no machine-readable channel at all, so that text is the whole
/// report there and must not move.
#[test]
fn an_unreadable_input_keeps_its_text_rendering() {
    let d = seed_dir("io806text");
    let missing = d.join("not_here.hl");
    let (stdout, stderr, code) = hale_check(&[], &missing);
    assert_eq!(code, 1);
    assert!(
        stdout.trim().is_empty(),
        "text mode writes no stdout: {}",
        stdout
    );
    assert!(
        stderr.contains("not a file or directory:")
            && stderr.contains("not_here.hl"),
        "the sentence it always printed: {}",
        stderr
    );

    let f = d.join("main.hl");
    std::fs::write(&f, "fn main() {\n    println(\"x\");\n}\n").unwrap();
    if make_unreadable(&f) {
        let (stdout, stderr, code) = hale_check(&[], &d);
        assert_eq!(code, 1);
        assert!(
            stdout.trim().is_empty(),
            "text mode writes no stdout: {}",
            stdout
        );
        assert!(
            stderr.contains("main.hl: Permission denied"),
            "`path: os error`, as before: {}",
            stderr
        );

        // `build` has no `--json` at all; its text is unchanged too.
        let (_, stderr, code) = hale_cmd("build", &[], &d);
        assert_eq!(code, 1);
        assert!(
            stderr.contains("main.hl: Permission denied"),
            "build prints the same line: {}",
            stderr
        );
    }

    make_readable(&f);
    let _ = std::fs::remove_dir_all(&d);
}

/// `verify` shares the reporting path, so a gate built on it sees the
/// same record rather than an empty stream and a bare exit 1.
#[test]
fn verify_json_reports_an_unreadable_input_too() {
    let d = seed_dir("io806verify");
    let missing = d.join("not_here.hl");

    let (stdout, _, code) = hale_cmd("verify", &["--json"], &missing);
    assert_ne!(code, 0, "verify refuses a target it cannot read");
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 1, "exactly one record: {}", stdout);
    assert_io_record(lines[0], "not_here.hl");
    let _ = std::fs::remove_dir_all(&d);
}

// GH #860: the last empty-stream shape on the check path. An import
// that names NOTHING printed `could not resolve import "..."` on
// stderr and returned a bare failure — no record, exit 1 — because
// the resolver reported it before the errors vector existed. It is
// not an io failure (nothing was opened, so there is no OS error to
// report) and the `import` statement has a span, so it is a located
// diagnostic under its path literal on every channel: a record under
// `--json`, the located line and caret in text, and the same line
// from `build` / `run`, which have no other channel.

/// `<tmp>/app/main.hl` importing `"../nowhere"`, which is not there.
/// The `import` is the first line, so every channel must say `1:8` —
/// column 8 is the opening quote of the path literal, the string the
/// diagnostic is about.
fn unresolvable_seed(tag: &str) -> PathBuf {
    let d = seed_dir(tag);
    std::fs::create_dir_all(d.join("app")).expect("mkdir app");
    std::fs::write(
        d.join("app").join("main.hl"),
        "import \"../nowhere\" as nowhere;\n\nfn main() {\n    \
         println(\"x\");\n}\n",
    )
    .unwrap();
    d
}

/// The located prefix every command prints for that seed.
const UNRESOLVABLE_LOCATED: &str =
    "main.hl:1:8: type error: could not resolve import `../nowhere`";

#[test]
fn an_unresolvable_import_is_one_located_record() {
    let d = unresolvable_seed("import860json");
    let app = d.join("app");

    let (stdout, stderr, code) = hale_check(&["--json"], &app);
    assert_eq!(code, 1, "the tree is refused:\n{stderr}");
    assert!(
        !stdout.trim().is_empty(),
        "--json must not answer an unresolvable import with an empty \
         stream (stderr was: {})",
        stderr
    );
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 1, "exactly one record: {}", stdout);
    let v: serde_json::Value =
        serde_json::from_str(lines[0]).expect("stdout is NDJSON");
    assert_eq!(v["severity"], "error", "got: {}", lines[0]);
    assert_ne!(
        v["kind"], "io error",
        "an import that names nothing is a diagnostic about the \
         program, not an unreadable input: {}",
        lines[0]
    );
    assert_eq!(v["line"], 1, "the `import` line: {}", lines[0]);
    assert_eq!(v["col"], 8, "the path literal's own column: {}", lines[0]);
    assert_eq!(
        v["file"].as_str().unwrap(),
        app.join("main.hl")
            .canonicalize()
            .expect("the importer exists")
            .display()
            .to_string(),
        "the record names the file that holds the `import`: {}",
        lines[0]
    );
    let msg = v["message"].as_str().unwrap();
    assert!(
        msg.contains("could not resolve import `../nowhere`"),
        "the record names the import string: {}",
        lines[0]
    );
    assert!(
        msg.contains("tried") && msg.contains("nowhere.hl"),
        "the places the resolver looked are the body of the message: \
         {}",
        lines[0]
    );
    assert!(
        stderr.trim().is_empty(),
        "under --json the report belongs on stdout: {}",
        stderr
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_unresolvable_import_prints_a_located_line_and_caret() {
    let d = unresolvable_seed("import860text");
    let app = d.join("app");

    let (stdout, stderr, code) = hale_check(&[], &app);
    assert_eq!(code, 1, "the tree is refused:\n{stderr}");
    assert!(
        stdout.trim().is_empty(),
        "text mode writes no stdout: {}",
        stdout
    );
    assert!(
        stderr.contains(UNRESOLVABLE_LOCATED),
        "text mode positions it in the importing file:\n{}",
        stderr
    );
    assert!(
        stderr.contains("import \"../nowhere\" as nowhere;")
            && stderr.contains('^'),
        "with the source line and a caret under the path:\n{}",
        stderr
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// `build` and `run` have no machine-readable channel, and reach the
/// resolver by two roads: a directory target resolves the union of
/// its files' imports and reports through `report_import_diags`, a
/// single-file target through `parse_with_imports`'s `Err`. Both used
/// to print the positionless sentence.
#[test]
fn build_and_run_locate_an_unresolvable_import() {
    let d = unresolvable_seed("import860build");
    let app = d.join("app");

    for (shape, target) in
        [("<dir>", app.clone()), ("<file>", app.join("main.hl"))]
    {
        for cmd in ["build", "run"] {
            let (_, stderr, code) = hale_cmd(cmd, &[], &target);
            assert_eq!(
                code, 1,
                "{cmd} {shape} must refuse the tree:\n{stderr}"
            );
            assert!(
                stderr.contains(UNRESOLVABLE_LOCATED),
                "{cmd} {shape} must position it like check does:\n{stderr}"
            );
            assert!(
                stderr.contains('^'),
                "{cmd} {shape} cuts the caret too:\n{stderr}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// `verify` shares the reporting path, so a discipline gate built on
/// it reads the record rather than an empty stream and a bare exit 1.
#[test]
fn verify_json_reports_an_unresolvable_import_too() {
    let d = unresolvable_seed("import860verify");

    let (stdout, _, code) = hale_cmd("verify", &["--json"], &d.join("app"));
    assert_ne!(code, 0, "verify refuses the tree");
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 1, "exactly one record: {}", stdout);
    let v: serde_json::Value =
        serde_json::from_str(lines[0]).expect("stdout is NDJSON");
    assert_eq!(v["line"], 1, "the `import` line: {}", lines[0]);
    assert_eq!(v["col"], 8, "the path literal's own column: {}", lines[0]);
    let _ = std::fs::remove_dir_all(&d);
}

/// The importer is per-IMPORT, not per-call: one call resolves the
/// union of a seed's imports, and a library's own imports are
/// resolved a hop further in. The record must name the file that
/// holds the `import` — here the library's, parsed at a non-zero
/// virtual base, whose text reaches the check reporting path only
/// because the diagnostic puts it there (a library's source is
/// inserted into the map only AFTER its own imports are followed,
/// which for this one never happens).
#[test]
fn an_unresolvable_import_inside_a_library_names_the_library_file() {
    let d = seed_dir("import860lib");
    std::fs::create_dir_all(d.join("lib")).expect("mkdir lib");
    std::fs::create_dir_all(d.join("app")).expect("mkdir app");
    std::fs::write(
        d.join("lib").join("main.hl"),
        "import \"../nowhere\" as nowhere;\n\nfn greet() -> String {\n    \
         return \"hi\";\n}\n",
    )
    .unwrap();
    std::fs::write(
        d.join("app").join("main.hl"),
        "import \"../lib\" as lib;\n\nfn main() {\n    \
         println(lib::greet());\n}\n",
    )
    .unwrap();

    let (stdout, stderr, code) = hale_check(&["--json"], &d.join("app"));
    assert_eq!(code, 1, "the tree is refused:\n{stderr}");
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 1, "exactly one record: {}", stdout);
    let v: serde_json::Value =
        serde_json::from_str(lines[0]).expect("stdout is NDJSON");
    assert_eq!(
        v["file"].as_str().unwrap(),
        d.join("lib")
            .join("main.hl")
            .canonicalize()
            .expect("the library file exists")
            .display()
            .to_string(),
        "the `import` that cannot be resolved is the library's: {}",
        lines[0]
    );
    assert_eq!(v["line"], 1, "its own line, un-shifted: {}", lines[0]);
    assert_eq!(v["col"], 8, "its own column: {}", lines[0]);

    // And the text channel resolves the same base.
    let (_, stderr, code) = hale_check(&[], &d.join("app"));
    assert_eq!(code, 1);
    assert!(
        stderr.contains(UNRESOLVABLE_LOCATED)
            && stderr.contains("import \"../nowhere\" as nowhere;"),
        "the library's own line and caret:\n{}",
        stderr
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// The neighbouring shape, pinned so the two stay distinguishable: a
/// path that DOES resolve, to a directory with no `.hl` files in it,
/// is a GH #806 io record about that directory — the import target
/// was found, and what failed was collecting files from it. It has
/// been a record since #806; only the import that resolves to
/// nothing at all was still an empty stream.
#[test]
fn an_import_of_a_directory_with_no_hl_files_is_one_record() {
    let d = seed_dir("import860empty");
    std::fs::create_dir_all(d.join("app")).expect("mkdir app");
    std::fs::create_dir_all(d.join("empty")).expect("mkdir empty");
    std::fs::write(
        d.join("app").join("main.hl"),
        "import \"../empty\" as e;\n\nfn main() {\n    \
         println(\"x\");\n}\n",
    )
    .unwrap();

    let (stdout, stderr, code) = hale_check(&["--json"], &d.join("app"));
    assert_eq!(code, 1, "the tree is refused:\n{stderr}");
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 1, "exactly one record: {}", stdout);
    let v = assert_io_record(lines[0], "empty");
    assert!(
        v["message"]
            .as_str()
            .unwrap()
            .contains("contains no .hl files"),
        "it says what was wrong with the directory: {}",
        lines[0]
    );
    let _ = std::fs::remove_dir_all(&d);
}

// ── GH #848: one located codegen error, whichever command compiles ──

/// A program `hale check` accepts and codegen refuses, with the
/// refusal carrying the offending generic argument's own span:
/// An array is not a type v0 can mangle into a generic instantiation's
/// name (GH #911 B3 made every primitive one, so `Bytes` no longer
/// serves). Line 6, column 12 is `[` in `Box<[Int; 2]>`.
///
/// (An ordinary string literal, not a raw one, on purpose:
/// `hale-corpus` harvests `r#"…"#` program literals out of the test
/// sources, and a program that checks clean and will not build would
/// land in the committed check/build divergence list for no gain.)
const UNSUPPORTED_GENERIC_ARG: &str = "type Box<T> {\n    \
     item: T;\n}\n\ntype Holder {\n    b: Box<[Int; 2]>;\n}\n\n\
     fn main() {\n    println(\"boxed\");\n}\n";

/// The bench twin: same declarations, same line 6, but a `bench_*`
/// fn instead of a `main` (the runner synthesizes the driver and
/// refuses a bench file that brings its own `main`).
const UNSUPPORTED_GENERIC_ARG_BENCH: &str = "type Box<T> {\n    \
     item: T;\n}\n\ntype Holder {\n    b: Box<[Int; 2]>;\n}\n\n\
     fn bench_nothing() {\n    println(\"\");\n}\n";

/// The located prefix — `file:line:col` — of the one codegen error,
/// from whichever stream the command reports on.
fn located_codegen(what: &str, out: &str) -> String {
    out.lines()
        .find(|l| l.contains(": codegen error:"))
        .and_then(|l| l.split(": codegen error:").next())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| {
            panic!("no located codegen error from {what}:\n{out}")
        })
}

/// Neither the error's Rust variant nor its span may reach the
/// reader: `UnsupportedAt("…", Span { start: Pos(55), … })` is what
/// every command but `build` used to print.
fn assert_no_debug_formatting(what: &str, out: &str) {
    assert!(
        !out.contains("Span {"),
        "{what} printed a debug-formatted span:\n{out}"
    );
    assert!(
        !out.contains("UnsupportedAt"),
        "{what} printed the error's Rust variant name:\n{out}"
    );
}

#[test]
fn build_run_and_test_locate_one_codegen_error_identically() {
    let d = seed_dir("codegen848");
    let f = d.join("boxed.hl");
    std::fs::write(&f, UNSUPPORTED_GENERIC_ARG).unwrap();

    // The premise: the checker is clean, so this reaches the error
    // path of every command that compiles and of no command that
    // does not.
    let (_, check_err, check_code) = hale_check(&[], &f);
    assert_eq!(check_code, 0, "check accepts it:\n{check_err}");

    let (_, build_err, build_code) = hale_cmd("build", &[], &f);
    let (_, run_err, run_code) = hale_cmd("run", &[], &f);
    // An explicitly named file is run whatever its suffix, so all
    // three commands compile the SAME bytes.
    let (test_out, _, test_code) = hale_cmd("test", &[], &f);
    assert_eq!(build_code, 1, "build refuses it:\n{build_err}");
    assert_eq!(run_code, 1, "run refuses it:\n{run_err}");
    assert_eq!(test_code, 1, "test refuses it:\n{test_out}");

    let from_build = located_codegen("build", &build_err);
    assert!(
        from_build.ends_with("boxed.hl:6:12"),
        "the position is the generic argument's own: {from_build}"
    );
    for (what, out) in [("run", &run_err), ("test", &test_out)] {
        assert_eq!(
            located_codegen(what, out),
            from_build,
            "{what} must name the file, line and column exactly as \
             build does\nbuild:\n{build_err}\n{what}:\n{out}"
        );
    }
    for (what, out) in
        [("build", &build_err), ("run", &run_err), ("test", &test_out)]
    {
        assert_no_debug_formatting(what, out);
        assert!(
            out.contains("b: Box<[Int; 2]>;") && out.contains('^'),
            "{what} must cut the snippet and caret from the source \
             too:\n{out}"
        );
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn bench_locates_a_codegen_error_in_the_bench_file() {
    let d = seed_dir("codegen848bench");
    let f = d.join("boxed_bench.hl");
    std::fs::write(&f, UNSUPPORTED_GENERIC_ARG_BENCH).unwrap();

    let (_, stderr, code) = hale_cmd("bench", &[], &f);
    assert_eq!(code, 1, "bench refuses it:\n{stderr}");
    assert_no_debug_formatting("bench", &stderr);
    let prefix = located_codegen("bench", &stderr);
    assert!(
        prefix.ends_with("boxed_bench.hl:6:12"),
        "bench names the bench file at the offending position — not \
         the driver copy it compiles and deletes:\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn replay_locates_a_codegen_error_too() {
    let d = seed_dir("codegen848replay");
    let hello = d.join("hello.hl");
    std::fs::write(&hello, "fn main() {\n    println(\"hi\");\n}\n")
        .unwrap();
    let rec = d.join("hello.halerec");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("run")
        .arg(&hello)
        .env("LOTUS_OBS_RECORD", &rec)
        .output()
        .expect("record a hello program");
    assert!(
        out.status.success() && rec.is_file(),
        "recording failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let f = d.join("boxed.hl");
    std::fs::write(&f, UNSUPPORTED_GENERIC_ARG).unwrap();
    // `--feed` re-executes CHANGED code against a recorded ingress
    // tape, so it is the one way onto replay's build path with a
    // program the recording did not come from — which is what a
    // program that cannot compile necessarily is.
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("replay")
        .arg(&rec)
        .arg(&f)
        .arg("--feed")
        .arg("--allow-live-effects")
        .output()
        .expect("run hale replay");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_eq!(out.status.code(), Some(1), "replay refuses it:\n{stderr}");
    assert_no_debug_formatting("replay", &stderr);
    assert!(
        located_codegen("replay", &stderr).ends_with("boxed.hl:6:12"),
        "replay locates it like every other command:\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

// ── GH #911 B1 (#846): the same located line for a bare unknown name ──

/// A call to a bare name nothing binds.
///
/// `hex` is not a builtin — GH #800 dropped it, because hexadecimal
/// is a format spec (`f"{n:x}"`) and never a call — and this program
/// declares nothing, so the F.18 strict-callee rule refuses the call
/// at its own span. That rule was off on the build path until GH #911
/// B1, so the two layers gave two different answers to one question:
///
/// ```console
/// $ hale check seed/
/// seed/main.hl:2:13: type error: call to `hex`: no free fn, generic
/// fn or fn-pointer binding with that name is in scope
///         let s = hex(255);
///                 ^^^
/// $ hale build seed/
/// codegen error: unsupported in codegen v0: call to `hex`: no free fn
/// / generic fn / fn-pointer binding with that name is in scope — did
/// you mean `__json_hex4`?
/// ```
///
/// No file, no line, no caret, from a layer below the one that had
/// just approved the program — and a did-you-mean naming a compiler
/// internal the author cannot write.
///
/// (An ordinary string literal, not a raw one: `hale-corpus` harvests
/// `r#"…"#` program literals out of the test sources.)
const UNKNOWN_BARE_CALLEE: &str =
    "fn main() {\n    let s = hex(255);\n    println(s);\n}\n";

/// The one `path:line:col: type error: message` line, from whichever
/// stream the command reports on.
fn located_type_error(what: &str, out: &str) -> String {
    let rows: Vec<&str> =
        out.lines().filter(|l| l.contains(": type error:")).collect();
    assert_eq!(
        rows.len(),
        1,
        "one mistake, one located type error from {what}:\n{out}"
    );
    rows[0].trim().to_string()
}

/// Every command that holds the whole program reports the unbound
/// name the way `check` does — same file, same line, same column, same
/// sentence — rather than as codegen's spanless refusal.
///
/// The drift guard for B1: the strictness is one boolean per entry
/// point, so a new command that compiles, or an entry point moved back
/// to `check_bundle_opts`, silently returns to the unlocated answer.
#[test]
fn build_run_and_test_report_a_bare_unknown_name_as_check_does() {
    let d = seed_dir("barename911");
    let f = d.join("main.hl");
    std::fs::write(&f, UNKNOWN_BARE_CALLEE).unwrap();

    // `check` of the SEED is the reference answer: the rule wants a
    // whole program, and one file of a multi-file seed may call what a
    // sibling declares, so `hale check <file>` stays permissive by
    // design and is not the comparison.
    let (check_out, check_err, check_code) = hale_check(&[], &d);
    let check_all = format!("{check_out}{check_err}");
    assert_eq!(check_code, 1, "check refuses it:\n{check_all}");
    let from_check = located_type_error("check", &check_all);
    assert!(
        from_check.contains("main.hl:2:13: type error: call to `hex`: ")
            && from_check.contains(
                "no free fn, generic fn or fn-pointer binding with that \
                 name is in scope"
            ),
        "the reference answer is located at the call: {from_check}"
    );

    // A directory and a single file reach the check through different
    // entry points in every one of these commands, so both are asked.
    for (what, target) in [
        ("build <dir>", d.as_path()),
        ("build <file>", f.as_path()),
        ("run <dir>", d.as_path()),
        ("run <file>", f.as_path()),
        ("test <file>", f.as_path()),
    ] {
        let cmd = what.split_whitespace().next().unwrap();
        let (out, err, code) = hale_cmd(cmd, &[], target);
        let all = format!("{out}{err}");
        assert_eq!(code, 1, "{what} refuses it:\n{all}");
        assert_eq!(
            located_type_error(what, &all),
            from_check,
            "{what} must report it exactly as check does\ncheck:\n\
             {check_all}\n{what}:\n{all}"
        );
        assert!(
            all.contains("let s = hex(255);") && all.contains('^'),
            "{what} must cut the snippet and caret from the source \
             too:\n{all}"
        );
        assert!(
            !all.contains("codegen error"),
            "{what} must not fall through to the backend's answer:\n{all}"
        );
        assert!(
            !all.contains("__json_hex4"),
            "{what} must not offer a compiler-internal spelling as the \
             did-you-mean:\n{all}"
        );
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// A seed whose witness leaf lands in the embedded stdlib, padded
/// past the stdlib offset of that leaf.
///
/// `ship` forbids `alloc` and reaches `std::io::tcp::Stream::send`,
/// whose `fail IoError { … }` is the allocation. That struct literal
/// sits ~13 KB into `hale_stdlib::AP_SOURCE` (`io_tcp.hl`, which the
/// concatenation puts third), so the padding — one filler fn per
/// line, ~40 KB of them — makes the file's own window swallow that
/// offset. Without it the collision does not happen and the defect
/// hides behind "placed nowhere".
fn stdlib_witness_seed(tag: &str) -> PathBuf {
    let d = seed_dir(tag);
    let mut src = String::from(
        "@effects(none: {alloc})\n\
         fn ship(s: std::io::tcp::Stream) {\n\
         \x20   s.send(\"tick\") or discard;\n\
         }\n\
         \n\
         fn main() {\n\
         \x20   let s = std::io::tcp::Stream { conn_fd: 1, owns_fd: false };\n\
         \x20   ship(s);\n\
         }\n",
    );
    for i in 0..1_100 {
        src.push_str(&format!("fn filler_{i}() -> Int {{ return {i}; }}\n"));
    }
    assert!(
        src.len() > 20_000,
        "the seed must be larger than the witness leaf's stdlib \
         offset for the windows to collide: {} bytes",
        src.len()
    );
    std::fs::write(d.join("main.hl"), &src).unwrap();
    d
}

/// GH #856: a diagnostic raised inside a STDLIB body carries an
/// offset into `hale_stdlib::AP_SOURCE`, which parses at base 0 in a
/// coordinate space of its own. Every renderer places a span by
/// testing it against the seed's file windows, and that test can
/// only compare numbers: past the seed's end the span was placed
/// nowhere (rendered against whatever source came first, at a line
/// belonging to nothing), and INSIDE a seed file's window it was
/// reported as that file, at a position the reader never wrote.
///
/// The span's ORIGIN now travels with the diagnostic, so no channel
/// resolves a stdlib offset against a seed file: the location is a
/// note naming the stdlib file and line instead.
#[test]
fn a_stdlib_origin_span_never_renders_as_a_seed_location() {
    let d = stdlib_witness_seed("stdlib856");
    let main = d.join("main.hl");
    let name = main.display().to_string();

    let (_, stderr, code) = hale_check(&[], &d);
    assert_eq!(code, 1, "the effect assertion is violated:\n{stderr}");
    // The finding itself is located in the user's file, as always.
    assert!(
        stderr.contains(&format!("{name}:2:4: type error: effect assertion")),
        "the violation is reported at the asserting fn:\n{stderr}"
    );
    // The witness leaf is a note naming the stdlib, not a line of
    // the seed. A filler line is what it used to be attributed to.
    let leaf = stderr
        .lines()
        .find(|l| l.contains("the `alloc` effect happens here"))
        .unwrap_or_else(|| panic!("the leaf is still reported:\n{stderr}"));
    assert!(
        leaf.trim_start().starts_with("note: "),
        "the leaf reads as a note:\n{stderr}"
    );
    assert!(
        leaf.contains("in the standard library, io_tcp.hl:"),
        "and names where in the stdlib it is:\n{stderr}"
    );
    assert!(
        !leaf.contains(&name),
        "it must never be attributed to a file of the seed:\n{stderr}"
    );
    assert!(
        !leaf.contains("filler_"),
        "nor to a line of one:\n{stderr}"
    );

    // The machine-readable channel says the same thing: no `file`,
    // no position — the shape an unplaceable finding has had since
    // GH #806 — with the stdlib location in the message.
    let (stdout, _, json_code) = hale_check(&["--json"], &d);
    assert_eq!(json_code, 1, "--json refuses it too:\n{stdout}");
    let record = stdout
        .lines()
        .map(|l| {
            serde_json::from_str::<serde_json::Value>(l)
                .unwrap_or_else(|e| panic!("stdout is NDJSON ({e}): {l}"))
        })
        .find(|v| {
            v["message"]
                .as_str()
                .is_some_and(|m| m.contains("the `alloc` effect happens here"))
        })
        .unwrap_or_else(|| panic!("one record per finding:\n{stdout}"));
    assert_eq!(record["file"], "", "no seed file: {record}");
    assert_eq!(record["line"], 0, "no seed line: {record}");
    assert_eq!(record["col"], 0, "no seed column: {record}");
    assert!(
        record["message"]
            .as_str()
            .unwrap_or("")
            .contains("in the standard library, io_tcp.hl:"),
        "the location a reader can act on is in the message: {record}"
    );

    // And the commands with no machine-readable channel, which
    // render through `render_located`, print the same note — the
    // drift guard one field over.
    for (what, target) in [
        ("check <dir>", d.as_path()),
        ("build <dir>", d.as_path()),
        ("run <dir>", d.as_path()),
        ("build <file>", main.as_path()),
    ] {
        let cmd = what.split_whitespace().next().unwrap();
        let (out, err, code) = hale_cmd(cmd, &[], target);
        let all = format!("{out}{err}");
        assert_eq!(code, 1, "{what} refuses it:\n{all}");
        let leaf = all
            .lines()
            .find(|l| l.contains("the `alloc` effect happens here"))
            .unwrap_or_else(|| panic!("{what} reports the leaf:\n{all}"));
        assert!(
            leaf.contains("in the standard library, io_tcp.hl:")
                && !leaf.contains("filler_"),
            "{what} must print the stdlib note, not a seed line:\n{all}"
        );
    }

    let _ = std::fs::remove_dir_all(&d);
}
