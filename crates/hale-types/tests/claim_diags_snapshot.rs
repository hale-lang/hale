//! GH #476 Change 9/10 — the check path's claim diagnostics, pinned
//! over the whole corpus.
//!
//! This began life as a DIFFERENTIAL: `hale check` used to report
//! these families from a second evaluator that re-derived them from
//! source, and the test held that evaluator and the judgment
//! engines byte-equal over every corpus program. That comparison
//! was the right instrument for a cutover — a golden can only prove
//! an engine is self-consistent, while an independent
//! implementation disagreeing is real signal about whether users'
//! diagnostics moved.
//!
//! The cutover is over. Changes 5f–5h migrated the last families,
//! the evaluator answered nothing for anyone shipping, and Change
//! 10 deleted it. What the corpus can still prove is that this
//! surface does not move BY ACCIDENT: the snapshot below was
//! generated from the final green differential run, so it is
//! literally the evaluator's last word, and every line of it had to
//! survive that comparison to be here.
//!
//! Regenerate deliberately, never reflexively:
//!
//! ```sh
//! HALE_REGEN_CLAIM_DIAGS=1 cargo test -p hale-types \
//!     --test claim_diags_snapshot
//! ```
//!
//! A diff here is a user-visible change to what `hale check` says.
//! Read it line by line before blessing it.

use std::collections::BTreeMap;
use std::path::PathBuf;

use hale_types::symbol::SourceFile;
use hale_types::Bundle;

fn render(d: &hale_syntax::Diag) -> String {
    let mut s = format!(
        "{:?} [{}..{}] {}",
        d.kind,
        d.span.start.as_usize(),
        d.span.end.as_usize(),
        d.message
    );
    for r in &d.related {
        s.push_str(&format!(
            " || related [{}..{}] {}",
            r.0.start.as_usize(),
            r.0.end.as_usize(),
            r.1
        ));
    }
    s
}

fn bundle_of<'a>(
    src: &str,
    program: &'a hale_syntax::ast::Program,
) -> Bundle<'a> {
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), program);
    let mut b = Bundle::new(programs);
    b.sources = vec![SourceFile {
        id: 0,
        path: "app.hl".to_string(),
        digest: "0".to_string(),
        base: 0,
        len: src.len() as u32,
    }];
    b
}

/// Exactly what `hale check` appends for the law block: selection,
/// then every migrated judgment family over the canonical model.
fn check_law_diags(bundle: &Bundle<'_>) -> Vec<String> {
    let programs: Vec<&hale_syntax::ast::Program> =
        bundle.programs.values().copied().collect();
    let (top, _) = hale_types::resolve::build_top_scope(bundle);
    let graph = hale_types::bus_graph::build_bus_graph(bundle, &top);
    let mut out = hale_types::claims::selection_diags(
        &programs,
        &graph,
        &bundle.import_renames,
    );
    out.extend(hale_types::judgment::claim_law_diags(bundle));
    out.iter().map(render).collect()
}

fn snapshot_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures/claim_diags_snapshot.txt");
    p
}

#[test]
fn claim_diagnostics_match_the_committed_snapshot() {
    let mut rendered = String::new();
    let mut compared = 0usize;
    let mut with_diags = 0usize;
    for program in hale_corpus::all() {
        let Ok(parsed) = hale_syntax::parse_source(&program.source)
        else {
            continue;
        };
        let bundle = bundle_of(&program.source, &parsed);
        // A model of an ill-typed program describes nothing; the
        // check path only reaches the law block when the bundle
        // typechecks, so hold this snapshot to the same gate.
        if hale_types::check_bundle_opts(&bundle, false)
            .iter()
            .any(|d| {
                d.is_error()
                    && d.kind != hale_syntax::error::DiagKind::Claim
            })
        {
            continue;
        }
        compared += 1;
        let lines = check_law_diags(&bundle);
        if lines.is_empty() {
            continue;
        }
        with_diags += 1;
        rendered.push_str(&format!("--- {} ---\n", program.origin));
        for l in &lines {
            rendered.push_str(l);
            rendered.push('\n');
        }
    }
    assert!(
        compared > 100,
        "the snapshot covered too little: {} programs",
        compared
    );
    assert!(
        with_diags > 0,
        "no corpus program produced a claim diagnostic — the \
         snapshot would pass vacuously"
    );

    let path = snapshot_path();
    if std::env::var("HALE_REGEN_CLAIM_DIAGS").as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &rendered).unwrap();
        eprintln!("regenerated {}", path.display());
        return;
    }
    let committed =
        std::fs::read_to_string(&path).unwrap_or_default();
    if committed == rendered {
        return;
    }
    // Report the moved lines, not a wall of bytes.
    let old: Vec<&str> = committed.lines().collect();
    let new: Vec<&str> = rendered.lines().collect();
    let gone: Vec<&&str> =
        old.iter().filter(|l| !new.contains(l)).take(8).collect();
    let added: Vec<&&str> =
        new.iter().filter(|l| !old.contains(l)).take(8).collect();
    panic!(
        "the check path's claim diagnostics moved ({} lines -> {}).\n\
         gone:\n  {}\nadded:\n  {}\n\nEvery line here is \
         user-visible. If the change is intended, regenerate with \
         HALE_REGEN_CLAIM_DIAGS=1.",
        old.len(),
        new.len(),
        gone.iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n  "),
        added
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n  "),
    );
}
