//! The law account round-trips through admission (F.40 phase 4, A5).
//!
//! Admission (`validate_law_account`) evaluates no law: it checks an
//! artifact against itself and refuses one whose sections disagree
//! (`spec/verification.md`). What it checks with must be what the
//! emitter wrote with, and this file holds the two to each other over
//! every artifact the emitter produces — the example corpus, the CLI
//! fixtures, `tests/hale`, every DNA main, and every claim-bearing
//! program embedded in a test — dumped through the built binary.
//!
//! For each artifact:
//!
//! * admission accepts it;
//! * every certificate row's verdict is the model's aggregation over
//!   its certificates (`hale_model::certificate_row_verdict`), and the
//!   document's verdict is the model's over everything it states
//!   (`hale_model::document_verdict`) — the one function the emitter
//!   and admission both call.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Admission's own source: the decoder and renderers under test are
/// the ones `hale topology graph` and `hale fleet` run.
#[allow(dead_code)]
#[path = "../src/topology_law.rs"]
mod topology_law;

use hale_model::VerdictIr;

/// Artifacts the emitter writes and admission refuses today: (the test
/// file the program is embedded in, the refusal, how many programs).
/// Each is a disagreement between the two that predates this test,
/// and each cell is asserted to FAIL so a fix has to remove it:
///
/// * law selection refuses a group (an unknown member, an unannounced
///   empty group, …): the row judges `invalid` and its explanation is
///   the table's selection issue in `law.issues`, which admission does
///   not accept as the row's — it wants the row to retain it;
/// * a certificate row whose subject is analyzed but which the
///   engines produced no certificate for (an undeclared `@budget`
///   class; an annotated function inside a `module` nothing calls):
///   the judgment writes it with no certificates, and admission
///   requires the count its law generates.
const KNOWN_OPEN: &[(&str, &str, usize)] = &[
    (
        "crates/hale-cli/tests/law_selection_reaches_the_artifact.rs",
        "asserts `invalid` with neither a decodable invalidity nor its \
         judgment's explanation",
        6,
    ),
    (
        "crates/hale-types/tests/judgment_certificates.rs",
        "carries 0 certificates, its law generates 1",
        2,
    ),
];

/// Source text that carries a law the artifact's law account rows.
const LAW_SYNTAX: &[&str] = &[
    "claims {",
    "@effects",
    "@no_panic",
    "@budget",
    "@phase_effects",
    "constitution ",
];

fn collect(dir: &Path, keep: &dyn Fn(&Path) -> bool, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> =
        entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect(&p, keep, out);
        } else if p.extension().is_some_and(|e| e == "hl") && keep(&p) {
            out.push(p);
        }
    }
}

/// One program the round trip dumps.
struct Program {
    /// Which part of the tree it comes from, for the totals.
    class: &'static str,
    /// Its name in a failure: a repo path, or `file.rs#N` for the
    /// Nth literal of a test file.
    name: String,
    path: PathBuf,
}

/// Every program the round trip dumps. Embedded programs are written
/// under `scratch` first.
fn programs(scratch: &Path) -> Vec<Program> {
    let root = hale_corpus::repo_root();
    let mut out: Vec<Program> = Vec::new();
    let mut on_disk = |class: &'static str,
                       rel: &str,
                       keep: &dyn Fn(&Path) -> bool| {
        let mut paths = Vec::new();
        collect(&root.join(rel), keep, &mut paths);
        for path in paths {
            let name = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .display()
                .to_string();
            out.push(Program { class, name, path });
        }
    };
    on_disk("examples", "crates/hale-codegen/tests/fixtures/examples", &|_| {
        true
    });
    on_disk("cli fixtures", "crates/hale-cli/tests/fixtures", &|_| true);
    on_disk("tests/hale", "tests/hale", &|_| true);
    on_disk("dna mains", "dna", &|p| {
        p.file_name().is_some_and(|n| n == "main.hl")
    });
    let mut seen = std::collections::BTreeSet::new();
    for (i, prog) in hale_corpus::embedded().into_iter().enumerate() {
        if !LAW_SYNTAX.iter().any(|s| prog.source.contains(s))
            || !seen.insert(prog.source.clone())
        {
            continue;
        }
        let dir = scratch.join(format!("embedded{}", i));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("main.hl");
        std::fs::write(&path, &prog.source).expect("write program");
        out.push(Program {
            class: "embedded, claim-bearing",
            name: prog.origin,
            path,
        });
    }
    out
}

/// One emitted artifact.
struct Artifact {
    class: &'static str,
    name: String,
    /// Whether `hale check` passed: a program with a law error still
    /// emits its artifact, which records the failing verdict.
    checked: bool,
    v: Value,
}

/// Dump every program's artifact through the built binary, in
/// parallel. A program that does not typecheck emits none.
fn artifacts(scratch: &Path) -> (Vec<Program>, Vec<Artifact>) {
    let progs = programs(scratch);
    let next = AtomicUsize::new(0);
    let found: Mutex<Vec<(usize, Artifact)>> = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(8);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some(p) = progs.get(i) else { break };
                let artifact = scratch.join(format!("{}.topology", i));
                let out = Command::new(env!("CARGO_BIN_EXE_hale"))
                    .arg("check")
                    .arg(&p.path)
                    .arg(format!("--dump-topology={}", artifact.display()))
                    .current_dir(p.path.parent().unwrap_or(scratch))
                    .output()
                    .expect("hale check --dump-topology");
                let Ok(text) = std::fs::read_to_string(&artifact) else {
                    continue;
                };
                let v: Value = serde_json::from_str(&text).unwrap_or_else(
                    |e| panic!("{}: not JSON: {}", p.name, e),
                );
                found.lock().unwrap().push((
                    i,
                    Artifact {
                        class: p.class,
                        name: p.name.clone(),
                        checked: out.status.success(),
                        v,
                    },
                ));
            });
        }
    });
    let mut found = found.into_inner().unwrap();
    found.sort_by_key(|(i, _)| *i);
    (progs, found.into_iter().map(|(_, a)| a).collect())
}

fn stated(v: &Value, key: &str) -> Option<VerdictIr> {
    v[key].as_str().and_then(VerdictIr::from_word)
}

/// What the walk counted.
#[derive(Default)]
struct Totals {
    law_rows: usize,
    cert_rows: usize,
}

/// Hold one admitted artifact to the model's functions; each
/// disagreement is one line.
fn differences_of(a: &Artifact, totals: &mut Totals) -> Vec<String> {
    let (name, v) = (&a.name, &a.v);
    let mut out = Vec::new();
    let cx = topology_law::RefContext::from_artifact(v)
        .expect("admitted, so its catalogs read");
    let rows = v["law"]["rows"].as_array().cloned().unwrap_or_default();
    totals.law_rows += rows.len();
    for row in &rows {
        let law = topology_law::decode_law(&row["law"], &cx)
            .expect("admitted, so every payload decodes");
        // The row's verdict is the model's aggregation over the
        // certificates it states.
        let certs = row["certs"].as_array().cloned().unwrap_or_default();
        if !certs.is_empty() {
            totals.cert_rows += 1;
            let statically_invalid = topology_law::has_unresolved(&law)
                || topology_law::law_class_invalid(&law, &cx.classes);
            let expect = hale_model::certificate_row_verdict(
                certs.iter().filter_map(|c| stated(c, "result")),
                statically_invalid,
            );
            if stated(row, "verdict") != Some(expect) {
                out.push(format!(
                    "{}: law row {} states `{}`, the model's aggregation \
                     over its certificates is `{}`",
                    name,
                    row["ordinal"],
                    row["verdict"].as_str().unwrap_or("?"),
                    expect.as_str()
                ));
            }
        }
    }
    // The document's verdict is the model's over everything it states.
    let every = |key: &str, field: &str| -> Vec<VerdictIr> {
        v[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| stated(r, field))
            .collect()
    };
    let document = hale_model::document_verdict(
        every("claims", "result")
            .into_iter()
            .chain(every("lowered", "result"))
            .chain(rows.iter().filter_map(|r| stated(r, "verdict"))),
        v["law"]["issues"].as_array().map_or(0, |i| i.len()),
    );
    if v["verdict"].as_str() != Some(document.as_str()) {
        out.push(format!(
            "{}: the document states `{}`, the model's verdict over it \
             is `{}`",
            name,
            v["verdict"].as_str().unwrap_or("?"),
            document.as_str()
        ));
    }
    out
}

#[test]
fn every_emitted_law_account_round_trips_through_admission() {
    let scratch = std::env::temp_dir()
        .join(format!("hale_law_round_trip_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("mkdir");
    let (programs, artifacts) = artifacts(&scratch);
    let _ = std::fs::remove_dir_all(&scratch);

    let mut differences: Vec<String> = Vec::new();
    let mut totals = Totals::default();
    let mut known_open = vec![0usize; KNOWN_OPEN.len()];
    for a in &artifacts {
        match topology_law::validate_law_account(&a.v, &a.name) {
            Ok(()) => differences.extend(differences_of(a, &mut totals)),
            Err(e) => {
                let open = KNOWN_OPEN.iter().position(|(file, refusal, _)| {
                    a.name.starts_with(&format!("{}#", file))
                        && e.contains(refusal)
                });
                match open {
                    Some(k) => known_open[k] += 1,
                    None => differences.push(format!(
                        "{} (check {}): admission refuses: {}",
                        a.name,
                        if a.checked { "passes" } else { "fails" },
                        e
                    )),
                }
            }
        }
    }
    for ((file, refusal, count), seen) in KNOWN_OPEN.iter().zip(&known_open)
    {
        if seen != count {
            differences.push(format!(
                "KNOWN_OPEN names {} program(s) of {} refused with `{}`, \
                 and {} are: a fixed cell leaves the table",
                count, file, refusal, seen
            ));
        }
    }

    let mut classes: Vec<&str> = programs.iter().map(|p| p.class).collect();
    classes.dedup();
    for class in classes {
        eprintln!(
            "  {}: {} programs, {} artifacts",
            class,
            programs.iter().filter(|p| p.class == class).count(),
            artifacts.iter().filter(|a| a.class == class).count()
        );
    }
    eprintln!(
        "law account round trip: {} programs, {} artifacts ({} known \
         open), {} law rows ({} with certificates), {} differences",
        programs.len(),
        artifacts.len(),
        known_open.iter().sum::<usize>(),
        totals.law_rows,
        totals.cert_rows,
        differences.len()
    );
    // Not vacuous: the corpus emits artifacts and they carry law.
    assert!(artifacts.len() >= 100, "only {} artifacts", artifacts.len());
    assert!(totals.law_rows >= 100, "only {} law rows", totals.law_rows);
    assert!(
        differences.is_empty(),
        "{} disagreement(s) between the emitter and admission:\n{}",
        differences.len(),
        differences.join("\n")
    );
}
