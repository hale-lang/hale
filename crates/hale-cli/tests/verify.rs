//! `hale verify` — the Layer-2 discipline gate: identical analysis
//! surface to `hale check`, but ANY finding (advisory or error)
//! fails the run. No execution.

use std::process::Command;

fn write(name: &str, src: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hale_verify_{}_{}",
        name,
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let f = dir.join("app.hl");
    std::fs::write(&f, src).expect("write");
    f
}

#[test]
fn clean_program_verifies() {
    let f = write("clean", "fn main() { println(\"hi\"); }\n");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("verify")
        .arg(&f)
        .output()
        .expect("run");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("verified"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(f.parent().unwrap());
}

#[test]
fn advisory_passes_check_but_fails_verify() {
    // Builder-in-a-run-loop: a default-on advisory, not an error.
    let src = r#"locus L {
    params { n: Int = 0; }
    run() {
        let mut i = 0;
        while true {
            let b = std::bytes::BytesBuilder { };
            i = i + 1;
        }
    }
}
fn main() { L { }; }
"#;
    let f = write("warny", src);
    let check = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&f)
        .output()
        .expect("run check");
    assert!(check.status.success(), "check must pass on advisories");

    let verify = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("verify")
        .arg(&f)
        .output()
        .expect("run verify");
    assert_eq!(verify.status.code(), Some(1), "verify must gate");
    assert!(
        String::from_utf8_lossy(&verify.stderr)
            .contains("discipline gate"),
        "{}",
        String::from_utf8_lossy(&verify.stderr)
    );
    let _ = std::fs::remove_dir_all(f.parent().unwrap());
}
