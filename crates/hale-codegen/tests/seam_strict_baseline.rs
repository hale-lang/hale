//! GH #2111 (phase 1): the strict-mode burn-down.
//!
//! Lowering takes a value's type from the checker through one seam
//! (`Cx::ty_at`); where the checker left no row, lowering still types the
//! expression itself. `BuildOptions::seam_strict` turns each such site
//! into a refusal (`CodegenError::MissingRows`), listing every one, before
//! anything is emitted.
//!
//! This test builds every corpus example, every `tests/hale` program and
//! every `dna/tests` fixture under strict mode, and counts the sites by
//! the survey's family, the expression's kind, and where the site is: the
//! stdlib, a program's files, or a node a rewrite after the check built
//! (`generated`, with no place in any file). A site counts once however
//! many programs reach it (a stdlib body, a seed many fixtures import); a
//! generated one once per program. The counts are held to
//! `fixtures/seam_strict_baseline.txt`:
//!
//! - a count above its baseline fails: a change made lowering type
//!   something itself that the checker did not record;
//! - a count below its baseline fails too, until the baseline is
//!   regenerated in the same change, so the file always says where the
//!   burn-down stands:
//!
//! ```text
//! cargo test --release -p hale-codegen --test seam_strict_baseline \
//!     --config 'env.HALE_SEAM_BASELINE_REGEN="1"'
//! ```
//!
//! Phase 1 is finished when the file's total is zero. To see the sites
//! themselves (a slice's plan), name a file in `HALE_SEAM_REPORT`: every
//! site is written there, with its place and its row.

use std::collections::{BTreeMap, BTreeSet};
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
    let test_file = |p: &Path| p.is_file() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with("_test.hl"));
    dir("tests/hale", &test_file);
    dir("dna/tests", &test_file);
    out
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
/// checker's type that refuse a missing row (`Cx::ty_at`,
/// `Cx::call_result`) outside the seam itself, which may only go up.
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
                reads += code.matches(".ty_at(").count() + code.matches(".call_result(").count();
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
         # go up; every other number in this file may only go down.\n\
         # `total` and the rows below it: expressions lowering types without the checker's row\n\
         # (strict mode) over the corpus, tests/hale and dna/tests, as\n\
         # family<TAB>kind<TAB>where<TAB>reason<TAB>count. A site counts once however many programs\n\
         # reach it; `generated` is a node with no place in any file (code a rewrite generated).\n",
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
            s.spawn(|| loop {
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
            });
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
    let goes_up: BTreeSet<&str> = measures.iter().filter(|m| m.2).map(|m| m.0).collect();
    let keys: BTreeSet<_> = was.keys().chain(is.keys()).cloned().collect();
    let mut worse = Vec::new();
    let mut better = Vec::new();
    for k in keys {
        let (a, b) = (was.get(&k).copied().unwrap_or(0), is.get(&k).copied().unwrap_or(0));
        let line = format!("{k}: {a} -> {b}");
        let up_is_better = goes_up.contains(k.as_str());
        if a == b {
            continue;
        }
        if (b > a) != up_is_better {
            worse.push(line);
        } else {
            better.push(line);
        }
    }
    assert!(
        worse.is_empty(),
        "lowering types more itself than the baseline holds (GH #2111): record the type in the \
         checker and read it through the seam instead; a typer deleted by inlining it into its \
         callers moves the lowering numbers too:\n{}",
        worse.join("\n")
    );
    assert!(
        better.is_empty(),
        "the burn-down moved: regenerate the baseline in this change \
         (HALE_SEAM_BASELINE_REGEN=1, see the file's head):\n{}",
        better.join("\n")
    );
}


/// The knob itself: off, a program builds as it always has; on, an
/// expression lowered without the checker's row refuses the build, and the
/// refusal names the site, its kind, its family and why. A literal has no
/// row of its own today (the checker records none, by design until family
/// 7 is burned down), so `1` is one.
#[test]
fn strict_mode_refuses_a_value_lowered_without_a_row() {
    let src = "fn main() { let x = 41; println(x + 1); }\n";
    let out = harness::unique_bin("seam_strict_off");
    build_opts::build_source(src, &out, &build_opts::options()).expect("off, it builds");
    let _ = std::fs::remove_file(&out);
    let strict = BuildOptions { seam_strict: true, ..build_opts::options() };
    let out = harness::unique_bin("seam_strict_on");
    let err = build_opts::build_source(src, &out, &strict).expect_err("on, it is refused");
    assert!(!out.exists(), "nothing is emitted");
    let CodegenError::MissingRows(rows) = &err else { panic!("a MissingRows refusal: {err}") };
    let lit = rows
        .iter()
        .find(|r| r.kind == "literal" && !r.stdlib && r.span.end.0 - r.span.start.0 == 2)
        .unwrap_or_else(|| panic!("the literal `41` is named: {err}"));
    assert_eq!((lit.family, lit.reason, lit.in_fn.as_str()), (7, "no row", "main"), "{err}");
    assert!(err.to_string().contains("program `literal` (family 7)"), "{err}");
    // the read of `x` has the checker's row: not named
    assert!(!rows.iter().any(|r| !r.stdlib && r.kind == "ident"), "{err}");
}
