//! Codegen reads no environment variable.
//!
//! Every build knob is a field of `BuildOptions`; the one function that
//! turns the process environment into those fields is
//! `build_options_from_env` in hale-cli (`build_env.rs`). A `LOTUS_*` or
//! `HALE_*` read inside this crate would be a knob that no field
//! documents, that no test can set without mutating the process
//! environment, and that a library caller cannot see. This scans the
//! crate's sources so the next one is refused where it is written.
//!
//! Not banned: `std::env::temp_dir()` (where scratch files go) and
//! `current_exe()` (where the compiler binary is). Neither reads a knob.

use std::path::{Path, PathBuf};

fn sources_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).expect("read dir").flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            sources_under(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn codegen_reads_no_environment() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources_under(&src, &mut files);
    assert!(files.len() > 20, "found only {} source files under src/", files.len());
    // The open parens matter: hale's own `std::env::var` stdlib entry
    // is named in diagnostics ("std::env::var takes 1 arg"), which is
    // text, not a read.
    const READS: [&str; 5] = ["env::var(", "env::var_os(", "env::vars(", "env::vars_os(", "env_flag("];
    let mut offenders = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).expect("read source");
        for (n, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            if READS.iter().any(|r| code.contains(r)) {
                offenders.push(format!("{}:{}: {}", f.strip_prefix(&src).unwrap().display(), n + 1, code));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "hale-codegen reads the environment ({} places):\n{}\n\n\
         A build knob is a field of `BuildOptions`, set from the environment in ONE place, \
         `hale-cli/src/build_env.rs`. Add the field and read the variable there.",
        offenders.len(),
        offenders.join("\n")
    );
}
