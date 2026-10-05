//! The snapshot-backed test entries (`support/entries.rs`) agree with
//! the bare-bundle entries they replace (F.40 phase 4, T3), before any
//! test moves to them.
//!
//! Over every single-file program `hale_corpus::all()` yields that
//! parses, both paths run and are compared: the diagnostics of
//! `check_program` (each diagnostic whole: kind, span, message, origin,
//! related; and their order), and, for a program whose check reports no
//! error, the topology artifact (`dump_topology` over a bundle of the
//! program as parsed), line by line. A panic on one path and not the
//! other is a difference; a panic on both is the same answer.
//!
//! One difference is classified, and only by proof: the bare
//! `dump_topology` models the program as parsed, while the snapshot's
//! load runs the desugar sequence first, as every entry point does (the
//! api binding's loci, JSON Tier 2's parsers, the declaration passes
//! all join the model). An artifact that differs is that difference
//! when the bare entry over the program the sequence shaped is the
//! snapshot's artifact byte for byte; the test lists each such program
//! and fails on any difference that is not that one.
//!
//! The default run is a deterministic sample (every program whose index
//! in the corpus is a multiple of [`SAMPLE_STRIDE`]); `HALE_MATRIX=full`
//! runs every program, as the ownership matrix's knob does.
//!
//! This test is kept until the bare-bundle entries leave
//! `crates/hale-types/src`, and is deleted with them: it is the proof
//! that moving a test to the snapshot changes none of its answers.

#[path = "support/entries.rs"]
mod entries;

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};

use hale_syntax::ast::Program;
use hale_syntax::Diag;
use hale_types::Bundle;

/// The default run checks every `SAMPLE_STRIDE`th program.
const SAMPLE_STRIDE: usize = 5;

fn full() -> bool {
    matches!(std::env::var("HALE_MATRIX").as_deref(), Ok("full"))
}

fn bundle_of(program: &Program) -> Bundle<'_> {
    let mut programs = BTreeMap::new();
    programs.insert(String::new(), program);
    Bundle::new(programs)
}

/// A path's answer, or the panic it raised instead.
fn answer<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|e| {
        e.downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "a panic with no message".to_string())
    })
}

fn render(d: &Diag) -> String {
    format!("{:?} {}..{} {:?}", d.kind, d.span.start.as_usize(), d.span.end.as_usize(), d.message)
}

/// The first place two diagnostic lists differ, rendered.
fn first_diag_difference(old: &[Diag], new: &[Diag]) -> Option<String> {
    for (i, (o, n)) in old.iter().zip(new).enumerate() {
        if o != n {
            return Some(format!("diagnostic {i}: old {} / new {}", render(o), render(n)));
        }
    }
    match old.len().cmp(&new.len()) {
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Greater => {
            Some(format!("old has {} more, first {}", old.len() - new.len(), render(&old[new.len()])))
        }
        std::cmp::Ordering::Less => {
            Some(format!("new has {} more, first {}", new.len() - old.len(), render(&new[old.len()])))
        }
    }
}

/// The first line two dumps differ at.
fn first_line_difference(old: &str, new: &str) -> Option<String> {
    if old == new {
        return None;
    }
    let (o, n): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
    for i in 0..o.len().max(n.len()) {
        let (a, b) = (o.get(i).copied().unwrap_or("<end>"), n.get(i).copied().unwrap_or("<end>"));
        if a != b {
            return Some(format!("dump line {}: old {a:?} / new {b:?}", i + 1));
        }
    }
    Some("the dumps differ in their line endings".to_string())
}

/// An artifact without its shape hash line, which every change to the
/// model's shape half moves: what is left says where the shape moved.
fn without_hash(dump: &str) -> String {
    dump.lines().filter(|l| !l.trim_start().starts_with("\"shape_hash\"")).collect::<Vec<_>>().join("\n")
}

#[test]
fn the_snapshot_entries_agree_with_the_bare_bundle_entries() {
    let full = full();
    let corpus = hale_corpus::all();
    let mut programs = 0usize;
    let mut checked_clean = 0usize;
    let mut diagnostics = 0usize;
    let mut dumped_bytes = 0usize;
    let mut differences: Vec<String> = Vec::new();
    // Artifacts that differ only because the snapshot ran the desugar
    // sequence over the program the bare entry modelled as parsed.
    let mut sequenced_only: Vec<String> = Vec::new();
    for (index, p) in corpus.iter().enumerate() {
        if !full && index % SAMPLE_STRIDE != 0 {
            continue;
        }
        let Ok(program) = hale_syntax::parse_source(&p.source) else { continue };
        programs += 1;

        let old = answer(|| hale_types::check_program(&program));
        let new = answer(|| entries::check_program(&program));
        let clean = match (&old, &new) {
            (Ok(o), Ok(n)) => {
                diagnostics += o.len();
                if let Some(d) = first_diag_difference(o, n) {
                    differences.push(format!("{}: check: {d}", p.origin));
                }
                !o.iter().any(Diag::is_error)
            }
            (Err(o), Err(n)) => {
                if o != n {
                    differences.push(format!("{}: check: both panic, old {o:?} / new {n:?}", p.origin));
                }
                false
            }
            (Ok(_), Err(n)) => {
                differences.push(format!("{}: check: the new path panics: {n:?}", p.origin));
                false
            }
            (Err(o), Ok(_)) => {
                differences.push(format!("{}: check: the old path panics: {o:?}", p.origin));
                false
            }
        };
        if !clean {
            continue;
        }
        checked_clean += 1;
        let bundle = bundle_of(&program);
        let old = answer(|| hale_types::topology::dump_topology(&bundle));
        let new = answer(|| entries::dump_topology(&bundle));
        match (&old, &new) {
            (Ok(o), Ok(n)) => {
                dumped_bytes += o.len();
                if let Some(d) = first_line_difference(o, n) {
                    // The bare entry models the program as parsed; the
                    // snapshot's load runs the desugar sequence first, as
                    // every entry point does. The bare entry over the
                    // sequenced program equal to the snapshot's is that
                    // difference and no other.
                    let mut sequenced = program.clone();
                    let shaped = answer(|| {
                        hale_types::desugar_sequence::desugar_before_check(
                            &mut [&mut sequenced],
                            &hale_types::desugar_sequence::Sequence { import_renames: &[], api: None, api_roles: None },
                        )
                    });
                    let after = match shaped {
                        Ok(Ok(_)) => answer(|| hale_types::topology::dump_topology(&bundle_of(&sequenced))).ok(),
                        _ => None,
                    };
                    match after.as_deref() == Some(n.as_str()) {
                        true => sequenced_only.push(format!(
                            "{}: {}",
                            p.origin,
                            first_line_difference(&without_hash(o), &without_hash(n))
                                .unwrap_or_else(|| "the shape hash alone".to_string()),
                        )),
                        false => differences.push(format!("{}: topology: {d}", p.origin)),
                    }
                }
            }
            (Err(o), Err(n)) if o == n => {}
            _ => differences.push(format!(
                "{}: topology: old {:?} / new {:?}",
                p.origin,
                old.as_ref().map(|_| "an artifact"),
                new.as_ref().map(|_| "an artifact"),
            )),
        }
    }
    println!(
        "snapshot entries ({}): {programs} programs compared ({diagnostics} diagnostics), \
         {checked_clean} checked clean and dumped ({dumped_bytes} bytes of artifact), \
         {} differences the desugar sequence accounts for, {} unclassified",
        if full { "full" } else { "sample" },
        sequenced_only.len(),
        differences.len(),
    );
    for d in &sequenced_only {
        println!("  the desugar sequence: {d}");
    }
    assert!(
        differences.is_empty(),
        "{} programs answer differently through the snapshot:\n{}",
        differences.len(),
        differences.join("\n"),
    );
}
