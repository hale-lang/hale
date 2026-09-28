//! What `hale run` and `hale test` compile lives in a private per-run
//! directory, and is gone when they return.
//!
//! The binary used to be `temp_dir()/hale_run_<hash>` and
//! `hale_test_<pid>_<n>_<hash>`: a name derived from the program, in a
//! directory every user of the machine can write to. Each run now gets
//! its own directory (mode 0700, a name `create_dir` proved nobody held)
//! and the whole directory is removed on the way out, so nothing a build
//! wrote — the binary, the objects codegen puts beside it — outlives it.
//!
//! The probe program reads where it is running from through `/proc`, so
//! the directory is observed from inside, while it exists.

use std::path::{Path, PathBuf};
use std::process::Command;

const PROBE: &str = r#"
fn sh(argv: String) -> String {
    let o = std::process::run(argv)
        or std::process::ProcessOutput { code: -9, signal: 0, stdout: "", stderr: "run failed" };
    return std::str::trim(o.stdout);
}

locus Probe {
    birth() {
        let exe = sh("readlink\n/proc/" + to_string(std::process::pid()) + "/exe");
        let dir = sh("dirname\n" + exe);
        println(dir);
        println(sh("stat\n-c\n%a\n" + dir));
    }
}

fn main() {
    Probe { };
}
"#;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir = std::env::temp_dir()
            .join(format!("hale_run_scratch_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("tmp")).expect("scratch dir");
        Scratch(dir)
    }

    /// The `TMPDIR` handed to `hale`: what it makes there is all we see.
    fn tmp(&self) -> PathBuf {
        self.0.join("tmp")
    }

    fn leftovers(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.tmp())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn hale(s: &Scratch, args: &[&std::ffi::OsStr]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .env("TMPDIR", s.tmp())
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

/// The two lines the probe prints: its directory and that directory's mode.
fn probe_report(text: &str, tmp: &Path) -> (String, String) {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let dir = lines
        .iter()
        .find(|l| l.starts_with(&*tmp.to_string_lossy()))
        .unwrap_or_else(|| panic!("no probe directory under {} in:\n{text}", tmp.display()))
        .to_string();
    let mode = lines
        .iter()
        .skip_while(|l| **l != dir)
        .nth(1)
        .unwrap_or_else(|| panic!("no mode after the directory in:\n{text}"))
        .to_string();
    (dir, mode)
}

#[test]
fn run_builds_in_a_private_directory_and_removes_it() {
    let s = Scratch::new("run");
    let src = s.0.join("probe.hl");
    std::fs::write(&src, PROBE).unwrap();
    let (text, code) = hale(&s, &["run".as_ref(), src.as_ref()]);
    assert_eq!(code, 0, "{text}");
    let (dir, mode) = probe_report(&text, &s.tmp());
    assert_eq!(mode, "700", "the run's directory is private to its owner: {text}");
    let name = Path::new(&dir).file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("hale-run-"), "{dir}");
    assert_eq!(s.leftovers(), Vec::<String>::new(), "nothing outlives the run");
}

#[test]
fn test_builds_every_binary_in_one_private_directory_and_removes_it() {
    let s = Scratch::new("test");
    let tests = s.0.join("suite");
    std::fs::create_dir_all(&tests).unwrap();
    std::fs::write(tests.join("ok_test.hl"), "fn main() { }\n").unwrap();
    std::fs::write(tests.join("also_ok_test.hl"), "fn main() { }\n").unwrap();
    // A passing test is silent, so the probe FAILS on purpose: its
    // stdout is what a failure reports, and carries the directory.
    std::fs::write(tests.join("probe_test.hl"), PROBE).unwrap();
    let (text, code) = hale(&s, &["test".as_ref(), tests.as_ref()]);
    assert_eq!(code, 1, "the probe prints, so it fails: {text}");
    let (dir, mode) = probe_report(&text, &s.tmp());
    assert_eq!(mode, "700", "{text}");
    let name = Path::new(&dir).file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("hale-test-"), "{dir}");
    assert!(text.contains("2 passed, 1 failed"), "{text}");
    assert_eq!(
        s.leftovers(),
        Vec::<String>::new(),
        "neither a passing nor a failing test leaves its binary behind"
    );
    assert!(
        std::fs::read_dir(&tests).unwrap().flatten().all(|e| {
            e.file_name().to_string_lossy().ends_with(".hl")
        }),
        "the test directory holds only sources afterwards"
    );
}

#[test]
fn a_test_that_does_not_compile_leaves_nothing_either() {
    let s = Scratch::new("broken");
    let tests = s.0.join("suite");
    std::fs::create_dir_all(&tests).unwrap();
    std::fs::write(tests.join("broken_test.hl"), "fn main( {\n").unwrap();
    let (text, code) = hale(&s, &["test".as_ref(), tests.as_ref()]);
    assert_eq!(code, 1, "{text}");
    assert_eq!(s.leftovers(), Vec::<String>::new(), "{text}");
}
