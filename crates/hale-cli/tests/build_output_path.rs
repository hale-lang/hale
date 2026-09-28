//! `hale build -o <path>`: the artifact lands at exactly that path.
//!
//! Until this flag the binary always landed beside its source, so every
//! seed that was built in place left an untracked file in the tree and
//! `.gitignore` grew a rule per binary. `-o` lets a caller — the DNA host,
//! `dna/face/start.sh`, CI — say where the artifact goes, and nothing is
//! written into the source tree at all.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A per-test scratch directory, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!(
            "hale_build_output_{}_{}",
            std::process::id(),
            tag
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Scratch(dir)
    }

    fn source(&self) -> PathBuf {
        let src = self.0.join("src");
        std::fs::create_dir_all(&src).expect("src dir");
        let file = src.join("app.hl");
        std::fs::write(&file, "fn main() { }\n").expect("write program");
        file
    }

    /// Everything under the scratch directory, relative and sorted.
    fn listing(&self) -> Vec<String> {
        fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, root, out);
                } else {
                    out.push(p.strip_prefix(root).unwrap().display().to_string());
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.0, &self.0, &mut out);
        out.sort();
        out
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn hale(args: &[&std::ffi::OsStr]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .output()
        .expect("run hale");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn native_binary_lands_at_exactly_the_path_and_runs() {
    let s = Scratch::new("native");
    let src = s.source();
    // A name with a dot and directories that do not exist yet: nothing
    // is added to it, swapped in it, or required of it.
    let out = s.0.join("out/seeds/app.v1");
    let (text, code) = hale(&["build".as_ref(), src.as_ref(), "-o".as_ref(), out.as_ref()]);
    assert_eq!(code, 0, "{text}");
    assert!(out.is_file(), "no binary at {}: {text}", out.display());
    assert_eq!(
        s.listing(),
        ["out/seeds/app.v1", "src/app.hl"],
        "the source directory must stay as it was; only the named path is new"
    );
    let ran = Command::new(&out).status().expect("run the built binary");
    assert!(ran.success());
}

#[test]
fn a_directory_target_builds_into_the_named_path_not_inside_itself() {
    let s = Scratch::new("dir");
    let src = s.source();
    let dir = src.parent().unwrap();
    let out = s.0.join("bin/named");
    let (text, code) = hale(&["build".as_ref(), dir.as_ref(), "--out".as_ref(), out.as_ref()]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(s.listing(), ["bin/named", "src/app.hl"], "{text}");
}

#[test]
fn the_flag_may_come_before_the_target() {
    let s = Scratch::new("before");
    let src = s.source();
    let out = s.0.join("here");
    let (text, code) = hale(&["build".as_ref(), "-o".as_ref(), out.as_ref(), src.as_ref()]);
    assert_eq!(code, 0, "{text}");
    assert!(out.is_file(), "{text}");
}

#[test]
fn without_o_the_binary_still_lands_beside_the_source() {
    let s = Scratch::new("beside");
    let src = s.source();
    let (text, code) = hale(&["build".as_ref(), src.as_ref()]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(s.listing(), ["src/app", "src/app.hl"], "{text}");
}

#[test]
fn a_wasm_build_puts_its_loader_beside_the_named_path() {
    let s = Scratch::new("wasm");
    let src = s.source();
    let out = s.0.join("web/mod.wasm");
    let (text, code) = hale(&[
        "build".as_ref(),
        src.as_ref(),
        "--target".as_ref(),
        "wasm".as_ref(),
        "-o".as_ref(),
        out.as_ref(),
    ]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(s.listing(), ["src/app.hl", "web/mod.mjs", "web/mod.wasm"], "{text}");
}

#[test]
fn o_without_a_path_or_given_twice_is_refused() {
    let s = Scratch::new("refused");
    let src = s.source();
    let (text, code) = hale(&["build".as_ref(), src.as_ref(), "-o".as_ref()]);
    assert_eq!(code, 2, "{text}");
    assert!(text.contains("-o requires the artifact's path"), "{text}");

    let (text, code) = hale(&[
        "build".as_ref(),
        src.as_ref(),
        "-o".as_ref(),
        "a".as_ref(),
        "--out".as_ref(),
        "b".as_ref(),
    ]);
    assert_eq!(code, 2, "{text}");
    assert!(text.contains("given twice"), "{text}");
    assert_eq!(s.listing(), ["src/app.hl"], "a refused build writes nothing");
}

#[test]
fn run_refuses_o_because_it_executes_its_binary() {
    let s = Scratch::new("run");
    let src = s.source();
    let (text, code) = hale(&["run".as_ref(), "-o".as_ref(), "x".as_ref(), src.as_ref()]);
    assert_eq!(code, 2, "{text}");
    assert!(text.contains("`-o` is a `hale build` flag"), "{text}");
    assert_eq!(s.listing(), ["src/app.hl"]);
}
