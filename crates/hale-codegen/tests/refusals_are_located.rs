//! GH #1941: every refusal lowering hands a program names its place.
//!
//! A construct the compiler cannot lower (or a program error only
//! lowering finds) reached the user as a bare `codegen error:` line: about
//! 1,100 sites raise `CodegenError::Unsupported` with no span, and only the
//! expression wrapper in `lower_expr` placed them. Lowering now places a
//! positionless refusal at the nearest enclosing site at every boundary it
//! crosses (an expression, a statement, a locus member, a declaration, a
//! fn body; a stdlib body's in the stdlib's own space, which the renderers
//! name as the stdlib file and line), so this file holds the build to it:
//!
//! - [`every_known_refusal_names_its_place`] (the ordinary suite): each
//!   program under `tests/fixtures/refused/` is refused by lowering, at
//!   the line its first comment names (`// refused at line N: <words>`),
//!   with the words in the message and no Rust `Debug` text in it;
//! - [`every_refusal_a_corpus_program_reaches_names_its_place`]
//!   (`#[ignore]`d, run beside the check/build agreement sweep in CI's
//!   sanitizer job): every corpus program, the ones the check refuses
//!   included (the harness lowers without the check, so they reach the
//!   refusals the check now stands in front of), built; a refusal without
//!   a place fails.
//!
//! A load or a lowering view the snapshot refuses is not lowering's
//! refusal and is not judged here: those are the check's diagnostics,
//! located by the check.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use hale_codegen::CodegenError;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// What lowering said about one program, when lowering is what refused it.
enum Outcome {
    /// It did not reach lowering (a load or view refusal), or it built.
    NotLowering,
    Built,
    Refused(CodegenError),
}

fn lower(source: &str, tag: &str) -> Outcome {
    let options = build_opts::options();
    let Ok(snap) = build_opts::load_seed(source, build_opts::harness_config(&options)) else {
        return Outcome::NotLowering;
    };
    let Ok(view) = snap.demand_lowering() else { return Outcome::NotLowering };
    let bin = harness::unique_bin(tag);
    let out = hale_codegen::build_resolved(view, &bin, &options);
    let _ = std::fs::remove_file(&bin);
    match out {
        Ok(()) => Outcome::Built,
        Err(e) => Outcome::Refused(e),
    }
}

/// Is this a refusal of the program (as opposed to the toolchain failing
/// to emit or link what lowering built)?
fn is_refusal(e: &CodegenError) -> bool {
    !matches!(e, CodegenError::LlvmInit(_) | CodegenError::LlvmEmit(_) | CodegenError::Link(_))
}

/// Rust `Debug` shapes a user must never read: a variant wrapping a string
/// or a number, a discriminant, a span.
fn debug_text(msg: &str) -> Option<&'static str> {
    for shape in [
        "TypeRef(\"",
        "LocusRef(\"",
        "Enum(\"",
        "Interface(\"",
        "Discriminant(",
        "Some(",
        "Span {",
        "Pos(",
        "Decimal(",
        "Array(",
        "Tuple([",
    ] {
        if msg.contains(shape) {
            return Some(shape);
        }
    }
    None
}

fn refused_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/refused")
}

/// `// refused at line N: <words>` on the fixture's first line.
fn expectation(src: &str) -> (usize, String) {
    let first = src.lines().next().unwrap_or("");
    let rest = first
        .strip_prefix("// refused at line ")
        .unwrap_or_else(|| panic!("a refused fixture opens with `// refused at line N: <words>`, not {first:?}"));
    let (n, words) = rest.split_once(':').expect("`N: <words>`");
    (n.trim().parse().expect("a line number"), words.trim().to_string())
}

#[test]
fn every_known_refusal_names_its_place() {
    let mut files: Vec<PathBuf> = std::fs::read_dir(refused_dir())
        .expect("tests/fixtures/refused")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "hl"))
        .collect();
    files.sort();
    assert!(files.len() >= 8, "the refused fixtures are the gate's evidence; found {}", files.len());
    let mut wrong = Vec::new();
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let src = std::fs::read_to_string(path).expect("read fixture");
        let (line, words) = expectation(&src);
        let err = match lower(&src, &format!("hale_refused_{}", name.trim_end_matches(".hl"))) {
            Outcome::Refused(e) => e,
            Outcome::Built => {
                wrong.push(format!("{name}: built; it is a refused fixture"));
                continue;
            }
            Outcome::NotLowering => {
                wrong.push(format!("{name}: never reached lowering"));
                continue;
            }
        };
        let shown = err.to_string();
        eprintln!("{name}: {shown}");
        let at = match &err {
            CodegenError::UnsupportedAt(_, span) | CodegenError::Refused(_, span) => Some(span.line_col(&src).0),
            _ => None,
        };
        if at != Some(line) {
            wrong.push(format!("{name}: expected a refusal at line {line}, got {at:?}: {shown}"));
        }
        if !shown.contains(&words) {
            wrong.push(format!("{name}: the message does not say {words:?}: {shown}"));
        }
        if let Some(shape) = debug_text(&shown) {
            wrong.push(format!("{name}: Rust Debug text ({shape}) in {shown}"));
        }
        if shown.contains("codegen v0") {
            wrong.push(format!("{name}: still worded as an internal stage's limit: {shown}"));
        }
    }
    assert!(wrong.is_empty(), "refused fixtures:\n  {}", wrong.join("\n  "));
}

/// How many programs the sweep lowers at once (`corpus_check_build_agreement`'s
/// knob, `HALE_CORPUS_SWEEP_THREADS`).
fn sweep_threads() -> usize {
    if let Some(n) = std::env::var("HALE_CORPUS_SWEEP_THREADS").ok().and_then(|v| v.parse::<usize>().ok()) {
        if n > 0 {
            return n;
        }
    }
    std::thread::available_parallelism().map(|n| n.get().min(8)).unwrap_or(4)
}

#[test]
#[ignore = "lowers ~1500 programs; run explicitly beside corpus_check_build_agreement"]
fn every_refusal_a_corpus_program_reaches_names_its_place() {
    let programs = hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok());
    // A program the check refuses can be ill-typed in a way lowering never
    // meets behind the check, and panic there instead of refusing it: not
    // a refusal, so not this gate's (counted, and silenced: the sweep
    // reads them as data).
    let panicked = AtomicUsize::new(0);
    let quiet = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let next = AtomicUsize::new(0);
    let programs = &programs;
    let workers = sweep_threads().min(programs.len().max(1));
    let found: Vec<(usize, String, bool, Option<&'static str>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                let next = &next;
                let panicked = &panicked;
                scope.spawn(move || {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(p) = programs.get(i) else { break };
                        // an import's other half is not in the fragment, and
                        // an `@ffi` symbol's host side is the test's
                        if p.source.contains("import \"") || p.source.contains("@ffi(") {
                            continue;
                        }
                        let tag = format!("hale_located_{i}");
                        let Ok(outcome) = std::panic::catch_unwind(|| lower(&p.source, &tag)) else {
                            panicked.fetch_add(1, Ordering::Relaxed);
                            continue;
                        };
                        if let Outcome::Refused(e) = outcome {
                            if is_refusal(&e) {
                                let shown = e.to_string();
                                let debug = debug_text(&shown);
                                mine.push((i, shown, e.is_located(), debug));
                            }
                        }
                    }
                    mine
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e))).collect()
    });
    std::panic::set_hook(quiet);
    let refused = found.len();
    eprintln!(
        "{refused} corpus programs reached a refusal; {} panicked in lowering instead",
        panicked.load(Ordering::Relaxed)
    );
    let mut bad: Vec<String> = found
        .into_iter()
        .filter(|(_, _, located, debug)| !located || debug.is_some())
        .map(|(i, shown, located, debug)| {
            let why = match (located, debug) {
                (false, Some(d)) => format!("no place, and Rust Debug text ({d})"),
                (false, None) => "no place".to_string(),
                (true, Some(d)) => format!("Rust Debug text ({d})"),
                (true, None) => unreachable!(),
            };
            format!("{} ({why}): {shown}", programs[i].origin)
        })
        .collect();
    bad.sort();
    assert!(refused > 50, "only {refused} corpus programs reached a refusal: the sweep is broken, not the compiler");
    assert!(
        bad.is_empty(),
        "{} of {refused} refusals a corpus program reaches name no place (or print Rust Debug text): place them at \
         the boundary they cross (`Cx::place`, `CodegenError::placed`):\n  {}",
        bad.len(),
        bad.join("\n  ")
    );
}
