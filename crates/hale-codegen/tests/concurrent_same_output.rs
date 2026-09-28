//! Two builds writing the SAME output path must both link.
//!
//! DNA's fixtures build one seed directory from parallel slices, so
//! several `hale build`s target one output path at once. Codegen used
//! to put the object at `output_path.with_extension("o")` and remove it
//! after linking, so one build's cleanup deleted the object another
//! build's clang was still reading ("no such file or directory: ...
//! .o, link failed"). Every intermediate and the binary now carry a
//! build-private name, and the binary reaches `output_path` by rename.

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Barrier};

use hale_codegen::build_executable_with_options;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const SRC: &str = r#"fn main() { println("same-output"); }"#;

const BUILDERS: usize = 8;
const BUILDS_EACH: usize = 12;

#[test]
fn concurrent_builds_to_one_output_path_all_link() {
    let program = Arc::new(hale_syntax::parse_source(SRC).expect("parse"));
    let bin = harness::unique_bin("concurrent_same_output");

    // One build first, so the runtime objects are cached and the race
    // under test is the per-output one, not the runtime's own.
    build_executable_with_options(&program, &bin, &[], &build_opts::options()).expect("first build");

    // Builders run back to back with no barrier between builds, so
    // their emit / link / cleanup phases interleave at random: one
    // build's cleanup lands between another's emit and its link. A
    // barrier per round kept them in lock step and hid the race.
    let start = Arc::new(Barrier::new(BUILDERS));
    let handles: Vec<_> = (0..BUILDERS)
        .map(|i| {
            let program = Arc::clone(&program);
            let bin = bin.clone();
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                std::thread::sleep(std::time::Duration::from_millis(7 * i as u64));
                (0..BUILDS_EACH)
                    .map(|n| build_executable_with_options(&program, &bin, &[], &build_opts::options()).map_err(|e| format!("builder {i}, build {n}: {e:?}")))
                    .collect::<Result<Vec<()>, String>>()
            })
        })
        .collect();
    let failures: Vec<String> = handles
        .into_iter()
        .filter_map(|h| h.join().expect("builder thread").err())
        .collect();
    assert!(failures.is_empty(), "concurrent builds failed: {failures:#?}");

    let out = Command::new(&bin).output().expect("run the built binary");
    assert!(out.status.success(), "the binary must run");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "same-output");

    // No build leaves an intermediate behind under its private name.
    let name = bin.file_name().unwrap().to_string_lossy().into_owned();
    let dir: PathBuf = bin.parent().unwrap().to_path_buf();
    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .expect("read temp dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|f| f.starts_with(&format!("{name}.")))
        .collect();
    assert!(leftovers.is_empty(), "leftover build files: {leftovers:?}");
    let _ = std::fs::remove_file(&bin);
}
