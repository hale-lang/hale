//! On-disk fixtures must pass the typechecker, not merely compile.
//!
//! `build_executable` — the entry every codegen test uses — parses
//! and lowers but never runs `check_program`. So programs compiled
//! and executed by the suite were never typechecked. Measured across
//! the corpus, **8.5% of the programs embedded in codegen tests do
//! not pass `hale check`** while compiling and running fine.
//!
//! Some of that is legitimate: a codegen test may deliberately lower
//! a shape the checker rejects. But it also hid a real bug — the
//! `err.kind` pattern, which the docs and spec both show, failed to
//! typecheck because the stdlib error types were injected only for
//! programs using `@form` machinery. Nothing caught it, because the
//! tests exercising that shape never typechecked and the typecheck
//! tests never used it.
//!
//! Closing the embedded-program gap wholesale is a bigger job than
//! it looks (many of those 85 are intentional). What this pins is
//! the part that should hold unconditionally: the **on-disk example
//! corpus** — the programs presented as exemplary Hale — typechecks
//! clean.

/// Multi-file projects: a single `main.hl` legitimately can't see
/// types its siblings declare, so checking it in isolation reports
/// "unknown type" for reasons that are not defects.
const MULTI_FILE_PROJECTS: &[&str] = &["25-imports", "fitter-applier-pair"];

#[test]
fn on_disk_example_corpus_typechecks_clean() {
    let mut failures: Vec<(String, String)> = Vec::new();
    let mut checked = 0;
    for p in hale_corpus::fixtures() {
        if !p.origin.contains("tests/fixtures/examples") {
            continue;
        }
        if MULTI_FILE_PROJECTS.iter().any(|m| p.origin.contains(m)) {
            continue;
        }
        let Ok(program) = hale_syntax::parse_source(&p.source) else {
            continue;
        };
        checked += 1;
        let errs: Vec<String> = hale_types::check_program(&program)
            .into_iter()
            .filter(|d| d.is_error())
            .map(|d| d.message)
            .collect();
        if !errs.is_empty() {
            failures.push((p.origin.clone(), errs[0].clone()));
        }
    }
    assert!(
        checked > 80,
        "only {} example programs checked — the sweep is not seeing \
         the corpus",
        checked
    );
    assert!(
        failures.is_empty(),
        "{} example programs fail `hale check` while still compiling \
         (codegen does not typecheck, so this can rot silently):\n{:#?}",
        failures.len(),
        failures
    );
}
