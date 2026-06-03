//! Integration test: every example program parses AND
//! typechecks. Multi-file projects (fitter-applier-pair) are checked
//! as a bundle. This is the Phase 0 exit-gate test for
//! milestone 2 — Phase 1.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use hale_syntax::ast::{Program, TopDecl};
use hale_syntax::{Diag, parse_source};
use hale_types::{check_bundle, Bundle};

fn examples_dir() -> PathBuf {
    // Examples moved to crates/hale-codegen/tests/fixtures/examples/
    // during the public-release cleanup. From this crate's
    // manifest dir (crates/hale-types/), pop to crates/, then
    // descend through hale-codegen/tests/fixtures/examples.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.push("hale-codegen");
    p.push("tests");
    p.push("fixtures");
    p.push("examples");
    p
}

fn list_projects(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).expect("read examples dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            out.push(path);
        }
    }
    out.sort();
    out
}

fn ap_files_in(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .expect("read project dir")
        .filter_map(|e| {
            let e = e.ok()?;
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) == Some("hl") {
                Some(p)
            } else {
                None
            }
        })
        .collect();
    out.sort();
    out
}

fn render_diags(diags: &[Diag], sources: &BTreeMap<String, &str>) -> String {
    diags
        .iter()
        .map(|d| {
            // We don't know which file the diag came from; render
            // against any source — the line/col is still useful
            // as long as the snippet shows. For multi-file bundles
            // we pick the first source.
            let s = sources
                .values()
                .next()
                .copied()
                .unwrap_or("");
            d.render(s)
        })
        .collect::<Vec<_>>()
        .join("\n  ")
}

#[test]
fn all_examples_parse_and_check() {
    let dir = examples_dir();
    let projects = list_projects(&dir);
    assert!(!projects.is_empty(), "no example projects found");

    let mut failures: Vec<String> = Vec::new();
    let mut total_files = 0;

    for project in &projects {
        let files = ap_files_in(project);
        if files.is_empty() {
            continue;
        }
        total_files += files.len();

        // Parse all files in this project as one bundle.
        let mut sources: BTreeMap<String, String> = BTreeMap::new();
        let mut programs: BTreeMap<String, Program> = BTreeMap::new();
        let mut parse_failures: Vec<String> = Vec::new();

        for file in &files {
            let source = fs::read_to_string(file).expect("read .hl");
            let key = file.strip_prefix(&dir).unwrap_or(file).to_string_lossy().into_owned();
            match parse_source(&source) {
                Ok(p) => {
                    programs.insert(key.clone(), p);
                    sources.insert(key, source);
                }
                Err(diags) => {
                    let rendered: Vec<String> =
                        diags.iter().map(|d| d.render(&source)).collect();
                    parse_failures.push(format!(
                        "  {}\n    {}",
                        file.strip_prefix(&dir).unwrap_or(file).display(),
                        rendered.join("\n    ")
                    ));
                }
            }
        }

        if !parse_failures.is_empty() {
            failures.push(format!(
                "[{}] parse failures:\n{}",
                project.file_name().unwrap().to_string_lossy(),
                parse_failures.join("\n")
            ));
            continue;
        }

        // Partition: each file with `fn main()` is a binary
        // entry point; each binary bundles with all non-main
        // files in the project (the shared modules).
        let (mains, shared): (Vec<_>, Vec<_>) = programs
            .iter()
            .partition(|(_, p)| has_main(p));
        let project_label = project.file_name().unwrap().to_string_lossy().into_owned();

        let bundles: Vec<Vec<(String, &Program)>> = if mains.is_empty() {
            // No main; check the project as one bundle (e.g.,
            // a future module-only project).
            vec![programs.iter().map(|(k, v)| (k.clone(), v)).collect()]
        } else {
            mains
                .iter()
                .map(|(k, p)| {
                    let mut b: Vec<(String, &Program)> = Vec::new();
                    b.push(((*k).clone(), *p));
                    for (sk, sp) in &shared {
                        b.push(((*sk).clone(), *sp));
                    }
                    b
                })
                .collect()
        };

        for bundle_files in bundles {
            let label = bundle_files
                .iter()
                .map(|(k, _)| k.as_str())
                .collect::<Vec<_>>()
                .join(" + ");
            let bundle_programs: BTreeMap<String, &Program> =
                bundle_files.into_iter().collect();
            let bundle = Bundle {
                programs: bundle_programs,
            };
            let diags = check_bundle(&bundle);
            // Warnings (e.g. blocking-syscall-on-cooperative-pool) are
            // advisory and don't fail the corpus typecheck — only
            // errors do.
            let errs: Vec<_> =
                diags.iter().filter(|d| d.is_error()).cloned().collect();
            if !errs.is_empty() {
                let source_refs: BTreeMap<String, &str> =
                    sources.iter().map(|(k, v)| (k.clone(), v.as_str())).collect();
                failures.push(format!(
                    "[{}] bundle {} type errors:\n  {}",
                    project_label,
                    label,
                    render_diags(&errs, &source_refs)
                ));
            }
        }
    }

    fn has_main(p: &Program) -> bool {
        p.items.iter().any(|item| match item {
            TopDecl::Fn(f) => f.name.name == "main",
            _ => false,
        })
    }

    if !failures.is_empty() {
        panic!(
            "type-check failures in {} of {} example projects ({} files):\n\n{}",
            failures.len(),
            projects.len(),
            total_files,
            failures.join("\n\n")
        );
    }
    eprintln!(
        "parsed + typechecked {} example projects ({} files)",
        projects.len(),
        total_files
    );
}
