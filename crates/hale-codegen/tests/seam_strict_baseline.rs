//! GH #2111 (phase 1): the strict-mode burn-down.
//!
//! Lowering takes a value's type from the checker through one seam
//! (`Cx::ty_at`); where the checker left no row, lowering still types the
//! expression itself. `BuildOptions::seam_strict` turns each such site
//! into a refusal (`CodegenError::MissingRows`), listing every one, before
//! anything is emitted.
//!
//! This test builds every corpus example, every `tests/hale` program and
//! every `dna/tests` fixture (found as `hale test` finds them, nested
//! directories included) under strict mode, and counts the sites by
//! the survey's family, the expression's kind, and where the site is: the
//! stdlib, a program's files, or a node a rewrite after the check built
//! (`generated`, with no place in any file). A site counts once however
//! many programs reach it (a stdlib body, a seed many fixtures import); a
//! generated one once per program. The counts are held to
//! `fixtures/seam_strict_baseline.txt`:
//!
//! - a static measure (they read codegen's source alone, so no test moves
//!   them) that moves the wrong way fails: lowering types more itself. A
//!   change that has to add lowering (a new type, a new intrinsic)
//!   regenerates the baseline with a one-line reason in its PR body;
//! - the corpus count grows with the corpus (every program a change adds
//!   brings sites, and the merge queue merges changes concurrently), so it
//!   fails only on a new class: a family, kind, place and reason the
//!   baseline held none of, now reached. Growth within a class is reported;
//! - a number that moves the right way passes, and the test prints the new
//!   numbers. The slice that moved it regenerates the file as part of its
//!   own change:
//!
//! ```text
//! cargo test --release -p hale-codegen --test seam_strict_baseline \
//!     --config 'env.HALE_SEAM_BASELINE_REGEN="1"'
//! ```
//!
//! A total of zero is not the end of phase 1, only what makes its end
//! possible. Phase 1 ends with deletion (#2111):
//!
//! - strict is the only mode, and `BuildOptions::seam_strict` is gone;
//! - every fallback path, where lowering types a value itself, is gone;
//! - a missing row is a located internal compiler error.
//!
//! This test and its baseline go with them. To see the sites themselves
//! (a slice's plan), name a file in `HALE_SEAM_REPORT`: every site is
//! written there, with its place and its row.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

use hale_codegen::{BuildOptions, CodegenError, MissingRow};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("the repository root")
}

fn baseline_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/seam_strict_baseline.txt")
}

/// Every program the burn-down covers: (its name in the report, its
/// entry). A corpus example is its directory, a seed of every file in
/// it; the others are a file and the imports it follows.
fn programs() -> Vec<(String, PathBuf)> {
    let root = repo_root();
    let mut out = Vec::new();
    let mut dir = |rel: &str, pick: &dyn Fn(&Path) -> bool| {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(root.join(rel))
            .unwrap_or_else(|e| panic!("{rel}: {e}"))
            .map(|e| e.expect("a directory entry").path())
            .filter(|p| pick(p))
            .collect();
        entries.sort();
        for p in entries {
            let name = p.strip_prefix(&root).expect("under the root").display().to_string();
            out.push((name, p));
        }
    };
    dir("crates/hale-codegen/tests/fixtures/examples", &|p| p.join("main.hl").is_file());
    for rel in ["tests/hale", "dna/tests"] {
        let mut files = Vec::new();
        test_files(&root.join(rel), &mut files);
        for p in files {
            let name = p.strip_prefix(&root).expect("under the root").display().to_string();
            out.push((name, p));
        }
    }
    out
}

/// Every `*_test.hl` under `dir`, as `hale test <dir>` finds them
/// (`collect_test_files`): depth-first in sorted order, skipping `vendor/`
/// and dot-directories.
fn test_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.expect("a directory entry").path())
        .collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if p.is_dir() {
            if name != "vendor" && !name.starts_with('.') {
                test_files(&p, out);
            }
        } else if name.ends_with("_test.hl") {
            out.push(p);
        }
    }
}

/// A missing row's count key: family, kind, where (`stdlib`, `program`,
/// or `generated`), and why (`MissingRow::reason`).
type Bucket = (u8, &'static str, &'static str, &'static str);

fn bucket(p: &Place, r: &MissingRow) -> Bucket {
    let at = match p {
        Place::Stdlib(..) => "stdlib",
        Place::File(..) => "program",
        Place::Generated(..) => "generated",
    };
    (r.family, r.kind, at, r.reason)
}

/// Where a missing row is, for counting it once: a stdlib site by its
/// span (the stdlib's own parse space, the same in every program); a
/// site in a program's files by its file and offset, so a seed many
/// programs import (`dna/core`) counts once; a node with no place in any
/// file (code a rewrite generated: an API codec, a served handler) by
/// the function it is in and its offset from that function's first such
/// site, so one generated function counts once whatever program it is
/// generated into.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Place {
    Stdlib(u32, u32),
    File(PathBuf, u32, u32),
    Generated(String, u32, u32),
}

/// Build `entry` under strict mode, as `hale build` loads it (the
/// harness's configuration): its missing rows, each with its place, or
/// why it did not reach the end of lowering.
fn missing_rows(name: &str, entry: &Path, options: &BuildOptions) -> Result<Vec<(Place, MissingRow)>, String> {
    use hale_frontend::snapshot::{LoadError, Snapshot};
    use hale_frontend::frontend::LoadMode;
    let snap = match Snapshot::load(entry, LoadMode::WholeSeed, &hale_frontend::source::Disk, build_opts::harness_config(options)) {
        Ok(s) => s,
        Err(LoadError::Refused(msg)) => return Err(msg),
        Err(LoadError::Load(f)) => return Err(f.text()),
    };
    let view = snap.demand_lowering().map_err(|b| format!("{:?}", b.refused))?;
    let out = harness::unique_bin("seam_strict");
    let r = hale_codegen::build_resolved(view, &out, options);
    let _ = std::fs::remove_file(&out);
    let rows = match r {
        Err(CodegenError::MissingRows(rows)) => rows,
        Ok(()) => Vec::new(),
        Err(e) => return Err(e.to_string()),
    };
    let _ = name;
    let root = repo_root();
    let in_file = |a: u32, b: u32| {
        snap.file_bases().iter().find(|(base, _, len)| a >= *base && b <= base + len).map(|(base, path, _)| {
            (path.strip_prefix(&root).unwrap_or(path).to_path_buf(), a - base, b - base)
        })
    };
    // each generated function's first site, its sites' origin
    let mut first: BTreeMap<&str, u32> = BTreeMap::new();
    for r in rows.iter().filter(|r| !r.stdlib && in_file(r.span.start.0, r.span.end.0).is_none()) {
        let at = first.entry(r.in_fn.as_str()).or_insert(u32::MAX);
        *at = (*at).min(r.span.start.0);
    }
    let place = |r: &MissingRow| {
        let (a, b) = (r.span.start.0, r.span.end.0);
        if r.stdlib {
            return Place::Stdlib(a, b);
        }
        if let Some((path, a, b)) = in_file(a, b) {
            return Place::File(path, a, b);
        }
        let origin = first.get(r.in_fn.as_str()).copied().unwrap_or(0);
        Place::Generated(r.in_fn.clone(), a.saturating_sub(origin), b.saturating_sub(origin))
    };
    Ok(rows.iter().map(|r| (place(r), r.clone())).collect())
}

/// Every `.rs` file under `crates/hale-codegen/src`, by path.
fn codegen_sources() -> BTreeMap<PathBuf, String> {
    fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
        for e in std::fs::read_dir(dir).expect("the source directory") {
            let p = e.expect("an entry").path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.insert(p.clone(), std::fs::read_to_string(&p).expect("a source file"));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    out
}

/// The survey's measures of how much typing lowering does itself (GH
/// #2111), from the source, each with the direction it may move:
///
/// - `static typers`: fns whose signature names a `CodegenTy` and no IR
///   value (they type an expression without lowering it), outside the
///   seam (`typed_seam.rs`, which reads the checker's answer);
/// - `value+type lowerings`: fns that hand back an IR value and its
///   `CodegenTy`;
/// - `CodegenTy built in an expression`: lines that build a `CodegenTy`
///   variant, not match one;
/// - `seam fallbacks (static)`: the sites that take lowering's own path
///   where the checker's row is missing (`Cx::seam_fallback`,
///   `Cx::ty_or_fallback`), reached by a test or not;
///
/// all of which may only go down; and `seam reads`, the reads of the
/// checker's type outside the seam itself (`Cx::ty_at`, and the ones that
/// refuse a missing row: `Cx::call_result`, `Cx::ty_required`,
/// `Cx::place_ty`), which may only go up.
fn static_measures() -> Vec<(&'static str, usize, bool)> {
    let sources = codegen_sources();
    let (mut typers, mut lowerings, mut built, mut fallbacks, mut reads) = (0, 0, 0, 0, 0);
    for (path, text) in &sources {
        let seam = path.ends_with("typed_seam.rs");
        // a fn's signature: from `fn name(` to its body or `;`
        let bytes = text.as_bytes();
        let mut i = 0;
        while let Some(at) = text[i..].find("fn ") {
            let start = i + at;
            i = start + 3;
            if start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
                continue;
            }
            let rest = &text[i..];
            let name_len = rest.bytes().take_while(|b| b.is_ascii_alphanumeric() || *b == b'_').count();
            if name_len == 0 {
                continue;
            }
            let mut j = i + name_len;
            if text[j..].starts_with('<') {
                let Some(close) = text[j..].find('>') else { continue };
                j += close + 1;
            }
            if !text[j..].starts_with('(') {
                continue;
            }
            let mut depth = 0usize;
            let mut k = j;
            for (off, c) in text[j..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            k = j + off + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let end = [text[k..].find('{'), text[k..].find(';')].into_iter().flatten().min().map_or(text.len(), |e| k + e);
            let ret = &text[k..end];
            // the seam's own fns read the checker's answer: no typer
            if !seam && ret.contains("CodegenTy") {
                if ["BasicValueEnum", "PointerValue", "IntValue"].iter().any(|v| ret.contains(v)) {
                    lowerings += 1;
                } else {
                    typers += 1;
                }
            }
        }
        for line in text.lines() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            let builds = code.match_indices("CodegenTy::").any(|(at, _)| code[at + 11..].starts_with(|c: char| c.is_ascii_uppercase()));
            let matches = code.contains("=>")
                || code.contains("matches!")
                || code.contains("if let")
                || code.contains("while let")
                || code.contains("let Some(CodegenTy")
                || code.contains("let CodegenTy");
            if builds && !matches {
                built += 1;
            }
            if !seam {
                fallbacks += code.matches(".seam_fallback(").count() + code.matches(".ty_or_fallback(").count();
                reads += [".ty_at(", ".call_result(", ".ty_required(", ".place_ty(", ".row_is_unit("].iter().map(|r| code.matches(r).count()).sum::<usize>();
            }
        }
    }
    vec![
        ("static typers", typers, false),
        ("value+type lowerings", lowerings, false),
        ("CodegenTy built in an expression", built, false),
        ("seam fallbacks (static)", fallbacks, false),
        ("seam reads", reads, true),
    ]
}

fn render(measures: &[(&'static str, usize, bool)], counts: &BTreeMap<Bucket, usize>) -> String {
    let total: usize = counts.values().sum();
    let mut s = String::from(
        "# Generated by `seam_strict_baseline.rs` under HALE_SEAM_BASELINE_REGEN=1 (GH #2111).\n\
         # `measure` lines: the survey's measures of crates/hale-codegen/src. `seam reads` may only\n\
         # go up, every other measure only down. The corpus rows below may grow within their class;\n\
         # a class (family, kind, where, reason) absent here may not appear.\n\
         # `total` and the rows below it: expressions lowering types without the checker's row\n\
         # (strict mode) over the corpus, tests/hale and dna/tests, as\n\
         # family<TAB>kind<TAB>where<TAB>reason<TAB>count. A site counts once however many programs\n\
         # reach it; `generated` is a node with no place in any file (code a rewrite generated).\n\
         # A total of zero is not the end of phase 1: it ends with strict the only mode, the knob\n\
         # and every fallback path deleted, and a missing row a located internal error (GH #2111).\n",
    );
    for (name, n, _) in measures {
        s.push_str(&format!("measure\t{name}\t{n}\n"));
    }
    s.push_str(&format!("total\t{total}\n"));
    for ((family, kind, at, reason), n) in counts {
        s.push_str(&format!("{family}\t{kind}\t{at}\t{reason}\t{n}\n"));
    }
    s
}

/// A baseline's numbers by name: a measure by its name, the total as
/// `total`, a row as `family kind (where, reason)`.
fn parse(text: &str) -> BTreeMap<String, usize> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            let (name, n) = match f.as_slice() {
                ["measure", name, n] => (name.to_string(), n),
                ["total", n] => ("total".to_string(), n),
                [family, kind, at, reason, n] => (format!("family {family} {kind} ({at}, {reason})"), n),
                _ => panic!("not a baseline line: {l:?}"),
            };
            (name, n.parse().expect("a count"))
        })
        .collect()
}

/// Stack for a build worker: the compiler's passes recurse (the parser,
/// the check, lowering), and a spawned thread's default (2 MiB) is well
/// under the main thread a build otherwise runs on; the dna fixtures'
/// largest programs overflowed it on macOS. As `hale test`'s workers
/// (`TEST_WORKER_STACK`): a virtual reservation, committed as used.
const WORKER_STACK: usize = 256 << 20;

#[test]
fn the_burn_down_only_moves_forward() {
    let options = BuildOptions { seam_strict: true, ..build_opts::options() };
    let programs = programs();
    assert!(programs.len() > 400, "the corpus, tests/hale and dna/tests: found {}", programs.len());

    let sites: Mutex<BTreeMap<(Place, &'static str), MissingRow>> = Mutex::new(BTreeMap::new());
    let failed: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).min(16);
    std::thread::scope(|s| {
        for _ in 0..workers {
            std::thread::Builder::new()
                .name("seam-strict-worker".to_string())
                .stack_size(WORKER_STACK)
                .spawn_scoped(s, || loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((name, entry)) = programs.get(i) else { break };
                    match missing_rows(name, entry, &options) {
                        Ok(rows) => {
                            let mut sites = sites.lock().expect("the site table");
                            for (p, r) in rows {
                                sites.entry((p, r.kind)).or_insert(r);
                            }
                        }
                        Err(e) => failed.lock().expect("the failures").push(format!("{name}: {e}")),
                    }
                })
                .expect("spawn a build worker");
        }
    });
    let failed = failed.into_inner().expect("the failures");
    assert!(failed.is_empty(), "programs that did not reach the end of lowering:\n{}", failed.join("\n"));

    let sites = sites.into_inner().expect("the site table");
    // the sites themselves, for planning a slice: every one, its place
    // and its row, where `HALE_SEAM_REPORT` names a file
    if let Some(report) = std::env::var_os("HALE_SEAM_REPORT") {
        let lines: Vec<String> = sites.iter().map(|((p, _), r)| format!("{p:?}\t{r}")).collect();
        std::fs::write(report, lines.join("\n") + "\n").expect("write the report");
    }
    let mut counts: BTreeMap<Bucket, usize> = BTreeMap::new();
    for ((p, _), r) in &sites {
        *counts.entry(bucket(p, r)).or_default() += 1;
    }
    let measures = static_measures();
    let now = render(&measures, &counts);
    let path = baseline_path();
    if std::env::var_os("HALE_SEAM_BASELINE_REGEN").is_some() {
        std::fs::write(&path, &now).expect("write the baseline");
        return;
    }
    let held = std::fs::read_to_string(&path).unwrap_or_default();
    let (was, is) = (parse(&held), parse(&now));
    let regen = "cargo test --release -p hale-codegen --test seam_strict_baseline \
                 --config 'env.HALE_SEAM_BASELINE_REGEN=\"1\"'";

    // The static measures read codegen's source alone, so no test a change
    // adds moves them: each is held against its wrong direction.
    let mut worse = Vec::new();
    let mut moved = Vec::new();
    for (name, n, up_is_better) in &measures {
        let a = was.get(*name).copied().unwrap_or(0);
        if a == *n {
            continue;
        }
        let line = format!("{name}: {a} -> {n}");
        if (*n > a) == *up_is_better {
            moved.push(line);
        } else {
            worse.push(line);
        }
    }
    assert!(
        worse.is_empty(),
        "lowering types more itself than the baseline holds (GH #2111): record the type in the checker \
         and read it through the seam instead (a typer deleted by inlining it into its callers moves the \
         lowering numbers too). A change that has to add lowering (a new type, a new intrinsic) \
         regenerates the baseline and says why in one line of its PR body:\n{}\n\n{regen}",
        worse.join("\n")
    );

    // The corpus count grows with the corpus: every program a change adds
    // brings sites, and the merge queue merges changes concurrently. So a
    // class the baseline holds (a family, kind, place and reason) may grow
    // without failing, and is reported; a class the baseline held none of
    // fails, since a construct lowering newly types itself is what the
    // burn-down is for.
    let mut new_class = Vec::new();
    for (k, &n) in is.iter().filter(|(k, _)| k.starts_with("family ")) {
        let a = was.get(k).copied().unwrap_or(0);
        if a == 0 && n > 0 {
            new_class.push(format!("{k}: 0 -> {n}"));
        } else if n != a {
            moved.push(format!("{k}: {a} -> {n}"));
        }
    }
    for (k, &a) in was.iter().filter(|(k, _)| k.starts_with("family ") && !is.contains_key(*k)) {
        moved.push(format!("{k}: {a} -> 0"));
    }
    if let (Some(a), Some(b)) = (was.get("total"), is.get("total")) {
        if a != b {
            moved.push(format!("total: {a} -> {b}"));
        }
    }
    assert!(
        new_class.is_empty(),
        "lowering types a new kind of value itself, where the checker left no row (GH #2111): record \
         its type in the checker instead:\n{}\n\nto see the sites, name a file in HALE_SEAM_REPORT; a \
         slice that moves a site into a new class regenerates the baseline:\n\n{regen}",
        new_class.join("\n")
    );

    // Anything else that moved passes: a slice regenerates the file as part
    // of its own change, and the queue does not hold two changes to it.
    if !moved.is_empty() {
        eprintln!(
            "the burn-down moved (GH #2111); regenerate the baseline in the change that moved it:\n{}\n\n{regen}",
            moved.join("\n")
        );
    }
}


/// The knob itself. On, a program that reaches a value lowering types
/// without the checker's row is refused, with nothing emitted, and the refusal names the site, its kind, its
/// family and why. A whole-set chain terminal written as a statement
/// (`v.take(2).reverse_into(r);`) has no row of its own today (family 6
/// `or`, `unknown`): when it has one, this test takes another site that
/// has none, until there is none and the knob goes. (It took an element
/// chain's accumulator until the chain's nodes had rows of their own, and
/// then a range loop's variable until the check bound it.)
#[test]
fn strict_mode_refuses_a_value_lowered_without_a_row() {
    let strict = BuildOptions { seam_strict: true, ..build_opts::options() };
    let src = "@form(vec)\nlocus Nums { capacity { heap items of Int; } }\n\
               fn main() { let v = Nums { }; v.push(1); v.push(2); v.push(3); let r = Nums { }; \
               v.take(2).reverse_into(r); println(r.len()); }\n";
    let out = harness::unique_bin("seam_strict_off");
    build_opts::build_source(src, &out, &build_opts::options()).expect("off, it builds");
    let _ = std::fs::remove_file(&out);
    let out = harness::unique_bin("seam_strict_on");
    let err = build_opts::build_source(src, &out, &strict).expect_err("on, it is refused");
    assert!(!out.exists(), "nothing is emitted");
    let CodegenError::MissingRows(rows) = &err else { panic!("a MissingRows refusal: {err}") };
    let at = rows.iter().find(|r| !r.stdlib).unwrap_or_else(|| panic!("a site of the program is named: {err}"));
    assert_eq!(at.in_fn, "main", "{err}");
    assert!(err.to_string().contains(&format!("program `{}` (family {})", at.kind, at.family)), "{err}");
}

/// A substitute block that leaves (`f() or { return x; }`) is no value:
/// the checker types it `()` and accepts it beside the call's value only
/// because it never ends. Lowering built it as a value, with a
/// placeholder it typed `Int` itself, and strict mode counted it a block
/// with no layout (836 of the corpus's sites, the RPC decoders' bails
/// most of them). It is built as statements now, at every success type.
#[test]
fn a_substitute_that_leaves_asks_no_value() {
    let strict = BuildOptions { seam_strict: true, ..build_opts::options() };
    let src = "type Pt { x: Int; }\n\
               fn word(i: Int) -> String fallible(String) { if i < 0 { fail \"neg\"; } \"w\" }\n\
               fn point(i: Int) -> Pt fallible(String) { if i < 0 { fail \"neg\"; } Pt { x: i } }\n\
               fn count(i: Int) -> Int fallible(String) { if i < 0 { fail \"neg\"; } i }\n\
               fn first(i: Int) -> String {\n\
                   let s = word(i) or { return \"left at word\"; };\n\
                   let p = point(i) or { return \"left at point: \" + err; };\n\
                   let n = count(i) or { return \"left at count\"; };\n\
                   s + to_string(p.x + n)\n\
               }\n\
               fn main() { println(first(2)); println(first(-1)); }\n";
    let out = harness::unique_bin("seam_strict_leaving_substitute");
    match build_opts::build_source(src, &out, &strict) {
        Ok(()) => {}
        Err(CodegenError::MissingRows(rows)) => {
            let program: Vec<_> = rows.iter().filter(|r| !r.stdlib).collect();
            assert!(program.is_empty(), "the program's sites have rows: {program:?}");
        }
        Err(e) => panic!("it builds: {e}"),
    }
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "w4\nleft at word\n");
}

/// What names no type of its own takes the place's, and the checker's row
/// says so: an empty literal (`[]`) at a `bounded` or array place, a
/// generic record's literal spelled with the template's name at its
/// monomorph's place (a fixed-width argument's too, `Box<U64>`), and a
/// quantity at a denomination no declaration names (`Duration in s`),
/// which is the integer it counts. Each was a row lowering had no layout
/// for.
#[test]
fn a_place_decides_what_names_no_type_of_its_own() {
    let strict = BuildOptions { seam_strict: true, ..build_opts::options() };
    let src = "type Box<T> { value: T; }\n\
               fn main() {\n\
                   let mut tags: bounded[String; 4] = [];\n\
                   push(tags, \"a\") or raise;\n\
                   let none: [Int; 0] = [];\n\
                   let b: Box<U64> = Box { value: 7 };\n\
                   let i: Box<Int> = Box { value: 8 };\n\
                   let secs = (3s + 500ms).in(s) or floor;\n\
                   println(count(tags) + len(none));\n\
                   println(b.value);\n\
                   println(i.value);\n\
                   println(secs);\n\
               }\n";
    let out = harness::unique_bin("seam_strict_place_decides");
    match build_opts::build_source(src, &out, &strict) {
        Ok(()) => {}
        Err(CodegenError::MissingRows(rows)) => {
            let program: Vec<_> = rows.iter().filter(|r| !r.stdlib).collect();
            assert!(program.is_empty(), "the program's sites have rows: {program:?}");
        }
        Err(e) => panic!("it builds: {e}"),
    }
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "1\n7\n8\n3s\n");
}

/// An element chain's nodes have rows of their own. The desugar builds
/// them before the check, but every one at the chain's span, and a row's
/// key told a literal or a block only by its span: the chain's counter
/// `0` and its flag `true`, and its `{ …; n }` and its `or { break; }`,
/// were one key with two answers, so `Unknown`. A literal is keyed by its
/// class too, and a block by whether it gives a value
/// (`hale_types::typed_bodies::expr_name`).
#[test]
fn a_chain_s_nodes_have_rows_of_their_own() {
    let strict = BuildOptions { seam_strict: true, ..build_opts::options() };
    let src = "fn main() {\n\
                   let xs = [1, 2, 3, 4];\n\
                   println(xs.filter(it > 1).count());\n\
                   println(xs.map(it * 2).sum());\n\
                   println(xs.any(it > 3));\n\
                   let f = xs.filter(it > 2).first() or 0;\n\
                   println(f);\n\
               }\n";
    let out = harness::unique_bin("seam_strict_chain_rows");
    match build_opts::build_source(src, &out, &strict) {
        Ok(()) => {}
        Err(CodegenError::MissingRows(rows)) => {
            let program: Vec<_> = rows.iter().filter(|r| !r.stdlib).collect();
            assert!(program.is_empty(), "the program's sites have rows: {program:?}");
        }
        Err(e) => panic!("it builds: {e}"),
    }
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "3\n20\ntrue\n3\n");
}

/// An element chain's operators have rows of their own. Its loop's counter
/// step `ctr + 1` (an `Int`), its bound `ctr >= lim` (a `Bool`) and a
/// sum's `acc + fin` (the sum's type) are all at the chain's span, and a
/// row's key told an operator only by its span: one key with three
/// answers, so `Unknown`. An operator is keyed by itself and its operands'
/// kinds too (`hale_types::typed_bodies::expr_name`).
#[test]
fn a_chain_s_operators_have_rows_of_their_own() {
    let strict = BuildOptions { seam_strict: true, ..build_opts::options() };
    let src = "fn main() {\n\
                   let v = [1, 2, 3, 4, 5, 6, 7, 8];\n\
                   println(v.skip(1).take(3).sum());\n\
                   println(v.filter(it % 2 == 0).skip(1).take(2).sum());\n\
                   let f = [1.5, 2.5];\n\
                   println(f.map(it).sum(0.0));\n\
                   println(v.filter(it > 6).max() or 0);\n\
               }\n";
    let out = harness::unique_bin("seam_strict_chain_operators");
    match build_opts::build_source(src, &out, &strict) {
        Ok(()) => {}
        Err(CodegenError::MissingRows(rows)) => {
            let program: Vec<_> = rows.iter().filter(|r| !r.stdlib).collect();
            assert!(program.is_empty(), "the program's sites have rows: {program:?}");
        }
        Err(e) => panic!("it builds: {e}"),
    }
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "9\n10\n4\n8\n");
}

/// A loop over a form's `entries` / `items`, or over `self.children`, is
/// lowered by its receiver's row. The iterand itself is a form's
/// pseudo-field, which the checker gives no type (it binds the loop
/// variable to the form's cell), or the children, whose type has no
/// layout. Lowering asked the iterand's row whether it was a `bounded`
/// before it looked for either shape, so each loop was a missing row.
#[test]
fn a_loop_over_a_form_or_the_children_asks_its_receiver_s_row() {
    let strict = BuildOptions { seam_strict: true, ..build_opts::options() };
    let src = "type Cell { id: Int; n: Int; }\n\
               @form(hashmap)\n\
               locus Cells { capacity { pool cells of Cell indexed_by id; } }\n\
               @form(vec)\n\
               locus CellList { capacity { heap cells of Cell; } }\n\
               locus Leaf { params { value: Int = 0; } contract { expose value: Int; } }\n\
               locus Sum {\n\
                   contract { consume value: Int; }\n\
                   accept(c: Leaf) { }\n\
                   fn total() -> Int {\n\
                       let mut t = 0;\n\
                       for child in self.children { t = t + child.value; }\n\
                       return t;\n\
                   }\n\
                   run() {\n\
                       let _a = Leaf { value: 10 };\n\
                       let _b = Leaf { value: 20 };\n\
                       println(self.total());\n\
                   }\n\
               }\n\
               fn main() {\n\
                   let m = Cells { };\n\
                   m.set(Cell { id: 1, n: 3 });\n\
                   m.set(Cell { id: 2, n: 4 });\n\
                   let mut a = 0;\n\
                   for e in m.entries { a = a + e.n; }\n\
                   println(a);\n\
                   let v = CellList { };\n\
                   v.push(Cell { id: 1, n: 5 });\n\
                   v.push(Cell { id: 2, n: 6 });\n\
                   let mut b = 0;\n\
                   for x in v.items { b = b + x.n; }\n\
                   println(b);\n\
                   Sum { };\n\
               }\n";
    let out = harness::unique_bin("seam_strict_form_loops");
    match build_opts::build_source(src, &out, &strict) {
        Ok(()) => {}
        Err(CodegenError::MissingRows(rows)) => {
            let program: Vec<_> = rows.iter().filter(|r| !r.stdlib).collect();
            assert!(program.is_empty(), "the program's sites have rows: {program:?}");
        }
        Err(e) => panic!("it builds: {e}"),
    }
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "7\n11\n30\n");
}

/// A path to a fn used as a value (an imported seed's, `apply(lib::add3)`,
/// or a Hale-bodied stdlib fn's, `apply(std::url::encode)`, GH #1082) is
/// the fn pointer of the fn it names, typed by the path's row. Lowering
/// built a name at the path and lowered that instead, and a built name
/// has no row.
#[test]
fn a_path_to_a_fn_used_as_a_value_is_typed_by_its_row() {
    let strict = BuildOptions { seam_strict: true, ..build_opts::options() };
    let src = "fn apply(f: fn (String) -> String, s: String) -> String { return f(s); }\n\
               fn main() {\n\
                   println(apply(std::url::encode, \"a b\"));\n\
                   let f = std::url::encode;\n\
                   println(f(\"c/d\"));\n\
               }\n";
    let out = harness::unique_bin("seam_strict_fn_values");
    match build_opts::build_source(src, &out, &strict) {
        Ok(()) => {}
        Err(CodegenError::MissingRows(rows)) => {
            let program: Vec<_> = rows.iter().filter(|r| !r.stdlib).collect();
            assert!(program.is_empty(), "the program's sites have rows: {program:?}");
        }
        Err(e) => panic!("it builds: {e}"),
    }
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "a%20b\nc%2Fd\n");
}

/// A send the intra-locus rewrite turns into a direct call has rows for
/// the nodes the call is made of. The rewrite is lowering's, after the
/// check, and builds `self.<field>.<handler>(value)` (a child subscribes)
/// or `self.<handler>(value)` (the publisher does) at the send's span: its
/// `self` and its field had no row, and lowering typed them itself. They
/// are typed from the scope's declarations, as the checker types them
/// (`hale_types::typed_bodies::ExprTypes::add_direct_sends`).
#[test]
fn a_direct_send_s_nodes_have_rows() {
    let strict = BuildOptions { seam_strict: true, ..build_opts::options() };
    let src = "type Ping { seq: Int = 0; }\n\
               topic Tick { payload: Ping; subject: \"direct.tick\"; }\n\
               topic Beat { payload: Ping; subject: \"direct.beat\"; }\n\
               locus Sub {\n\
                   params { acc: Int = 0; }\n\
                   bus { subscribe Tick as on_tick; }\n\
                   fn on_tick(p: Ping) { self.acc = self.acc + p.seq; }\n\
                   fn sum() -> Int { return self.acc; }\n\
               }\n\
               locus Pub {\n\
                   params { sub: Sub = Sub { }; n: Int = 0; }\n\
                   bus { publish Tick; publish Beat; subscribe Beat as on_beat; }\n\
                   fn on_beat(p: Ping) { self.n = self.n + p.seq; }\n\
                   fn go() {\n\
                       Tick <- Ping { seq: 3 };\n\
                       Beat <- Ping { seq: 4 };\n\
                   }\n\
                   fn total() -> Int { return self.sub.sum() + self.n; }\n\
               }\n\
               main locus App { params { pub: Pub = Pub { }; } }\n\
               fn main() {\n\
                   let app = App { };\n\
                   app.pub.go();\n\
                   println(app.pub.total());\n\
               }\n";
    let out = harness::unique_bin("seam_strict_direct_send_rows");
    match build_opts::build_source(src, &out, &strict) {
        Ok(()) => {}
        Err(CodegenError::MissingRows(rows)) => {
            let program: Vec<_> = rows.iter().filter(|r| !r.stdlib).collect();
            assert!(program.is_empty(), "the program's sites have rows: {program:?}");
        }
        Err(e) => panic!("it builds: {e}"),
    }
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "7\n");
}

/// A call of a method beside a param of its name, and of a generic enum's
/// monomorph's variant spelled by its mangled name, have rows: the check
/// typed both calls `Unknown` (the first's callee as the param, the
/// second's as no enum it knew), and lowering typed them itself.
#[test]
fn a_method_and_a_monomorph_s_variant_have_call_rows() {
    let strict = BuildOptions { seam_strict: true, ..build_opts::options() };
    let src = "type Value<T> = enum { Some(T), None };\n\
               locus Sub {\n\
                   params { hits: Int = 4; }\n\
                   fn hits() -> Int { return self.hits + 1; }\n\
               }\n\
               locus Pub {\n\
                   params { sub: Sub = Sub { }; }\n\
                   fn total() -> Int { return self.sub.hits(); }\n\
               }\n\
               fn main() {\n\
                   let p = Pub { };\n\
                   println(p.total());\n\
                   let a: Value<Int> = Value_Int::Some(1);\n\
                   println(a == Value_Int::Some(1));\n\
               }\n";
    let out = harness::unique_bin("seam_strict_call_rows");
    match build_opts::build_source(src, &out, &strict) {
        Ok(()) => {}
        Err(CodegenError::MissingRows(rows)) => {
            let program: Vec<_> = rows.iter().filter(|r| !r.stdlib).collect();
            assert!(program.is_empty(), "the program's sites have rows: {program:?}");
        }
        Err(e) => panic!("it builds: {e}"),
    }
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "5\ntrue\n");
}

/// A default left to a literal has its rows where the literal evaluates
/// it: in a stdlib body (`std::metrics`' counter builds its entry with
/// `__StdMetricsEntry { }`), and in a program's body inside a stdlib
/// default (`std::api::Context { }` leaves `caller`, whose default is a
/// `std::api::Principal { }`), where the evaluation path names the
/// stdlib's literal by the id the checker walked it under.
#[test]
fn a_stdlib_default_has_rows_where_it_is_evaluated() {
    let strict = BuildOptions { seam_strict: true, ..build_opts::options() };
    let src = "fn main() {\n\
                   let reg = std::metrics::Registry { namespace: \"app\" };\n\
                   let hits = std::metrics::counter(reg, \"hits\", std::metrics::labels_one(\"r\", \"a\"));\n\
                   hits.inc();\n\
                   let c = std::api::Context { role: \"r\" };\n\
                   println(c.caller.uid);\n\
                   println(c.caller.mode);\n\
               }\n";
    let out = harness::unique_bin("seam_strict_stdlib_defaults");
    match build_opts::build_source(src, &out, &strict) {
        Ok(()) => {}
        Err(CodegenError::MissingRows(rows)) => {
            let defaults: Vec<_> =
                rows.iter().filter(|r| matches!(r.kind, "literal" | "unary" | "struct")).collect();
            assert!(defaults.is_empty(), "the defaults have rows: {defaults:?}");
        }
        Err(e) => panic!("it builds: {e}"),
    }
    let ran = build_opts::build_source(src, &out, &build_opts::options())
        .map(|()| std::process::Command::new(&out).output().expect("it runs"))
        .expect("it builds");
    let _ = std::fs::remove_file(&out);
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "-1\nlocal\n");
}
