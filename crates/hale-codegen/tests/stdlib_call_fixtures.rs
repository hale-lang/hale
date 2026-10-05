//! F.40 phase 4, S0: the build-only call fixtures under
//! `fixtures/stdlib_calls/` check clean and lower to IR.
//!
//! Each fixture calls stdlib paths at the positions the rest of the
//! line S shadow set (`stdlib_dispatch_coverage`) does not reach, one
//! file per stdlib module. They are never run: the line's proof is IR
//! identity, so a fixture only has to reach the dispatch arm it names,
//! and a call on an unconnected socket value is as good as any. The
//! shadow builds them with `hale build`, which checks first, so a
//! fixture the checker refuses would add nothing to the IR compared:
//! this test holds them to both.
//!
//! Scaffolding with the coverage test: it goes when that one does (S3/S4).

#[path = "support/harness.rs"]
mod harness;

#[path = "../../hale-types/tests/support/entries.rs"]
mod entries;
use std::path::PathBuf;

fn fixtures() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stdlib_calls");
    let mut v: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "hl"))
        .collect();
    v.sort();
    v
}

#[test]
fn every_stdlib_call_fixture_checks_and_lowers() {
    let files = fixtures();
    assert!(files.len() >= 20, "only {} call fixtures found", files.len());
    let failures: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = files
            .iter()
            .map(|f| {
                s.spawn(move || {
                    let name = f.file_name().unwrap().to_string_lossy().to_string();
                    let source = std::fs::read_to_string(f).unwrap();
                    let program = match hale_syntax::parse_source(&source) {
                        Ok(p) => p,
                        Err(d) => return Some(format!("{name}: does not parse: {d:?}")),
                    };
                    let errors: Vec<String> = entries::check_program(&program)
                        .iter()
                        .filter(|d| d.is_error())
                        .map(|d| format!("{d:?}"))
                        .collect();
                    if !errors.is_empty() {
                        return Some(format!("{name}: the checker refuses it:\n  {}", errors.join("\n  ")));
                    }
                    let bin = harness::unique_bin(&format!("stdlib_calls_{}", name.trim_end_matches(".hl")));
                    match harness::build_source_ir_text(&source, &bin) {
                        Ok(_) => {
                            let _ = std::fs::remove_file(&bin);
                            None
                        }
                        Err(e) => Some(format!("{name}: does not lower: {e:?}")),
                    }
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().unwrap()).collect()
    });
    assert!(failures.is_empty(), "{} call fixture(s) fail:\n{}", failures.len(), failures.join("\n"));
}
