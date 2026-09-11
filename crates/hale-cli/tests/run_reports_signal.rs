//! GH #577 — `hale run` says when the program was killed by a signal,
//! and exits 128 + the signal, where it used to print nothing and exit
//! 1. The program here sends itself SIGSEGV through the process API.

use std::process::Command;

const SRC: &str = r#"fn main() {
    println("about to die");
    let me = std::process::Child { pid: std::process::pid() };
    std::process::signal(me, 11) or discard;
    std::time::sleep(2s);
    println("still here");
}
"#;

#[test]
fn hale_run_names_the_signal_that_killed_the_program() {
    let d = std::env::temp_dir().join(format!("hale_run_signal_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("main.hl"), SRC).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(["run", &d.to_string_lossy()]).output().expect("hale run");
    let err = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("about to die") && !stdout.contains("still here"), "the program died mid-way:\n{stdout}");
    assert!(err.contains("killed by SIGSEGV (signal 11)"), "hale run names the signal:\n{err}");
    assert_eq!(out.status.code(), Some(139), "exit is 128 + the signal");
    let _ = std::fs::remove_dir_all(&d);
}
