//! GH #738 — a bare fallible stdlib call (no `or`) is an error, in
//! `hale check`, `hale verify` and `hale build` alike. A handled call
//! and a deliberately discarded one say nothing. The legacy Int-status
//! form is the bare form and is refused the same way. `--strict-fallible`
//! is gone: there is no mode in which the bare call is accepted.

use std::path::{Path, PathBuf};
use std::process::Command;

fn run(verb: &str, flags: &[&str], src: &str, tag: &str) -> (bool, String) {
    let d: PathBuf = std::env::temp_dir()
        .join(format!("hale_strict_fallible_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let f = d.join("main.hl");
    std::fs::write(&f, src).unwrap();
    let mut args: Vec<&str> = vec![verb];
    args.extend_from_slice(flags);
    let target = f.to_string_lossy().to_string();
    args.push(&target);
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(&args)
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&d);
    (out.status.success(), text)
}

const NOTICE: &str = "can fail (IoError) and this call says nothing about it";

const UNHANDLED: &str = "fn main() {\n    std::io::fs::write_file(\"/tmp/hale-738-t\", \"x\");\n    println(\"done\");\n}\n";
const DISCARDED: &str = "fn main() {\n    std::io::fs::write_file(\"/tmp/hale-738-t\", \"x\") or discard;\n    println(\"done\");\n}\n";
const HANDLED: &str = "fn main() {\n    std::io::fs::write_file(\"/tmp/hale-738-t\", \"x\") or raise;\n    let s = std::io::fs::read_file(\"/tmp/hale-738-t\") or \"\";\n    let n = std::str::parse_int(\"7\") or 0;\n    println(s, n);\n}\n";
const LEGACY_STATUS: &str = "fn main() {\n    let r: Int = std::io::fs::write_file(\"/tmp/hale-738-t\", \"x\");\n    println(r);\n}\n";

#[test]
fn an_unhandled_call_is_an_error_in_check_verify_and_build() {
    for verb in ["check", "verify", "build"] {
        let (ok, out) = run(verb, &[], UNHANDLED, verb);
        assert!(!ok, "{verb} must refuse the bare call: {out}");
        assert!(
            out.contains("`std::io::fs::write_file`") && out.contains(NOTICE),
            "{verb}: the error names the callee, its payload and the missing disposition: {out}"
        );
        assert!(
            out.contains("or raise") && out.contains("or discard") && out.contains("or handler(err)"),
            "{verb}: {out}"
        );
        assert!(out.contains("error since v0.22.0"), "{verb}: {out}");
    }
}

#[test]
fn a_discarded_or_handled_call_says_nothing() {
    for (src, tag) in [(DISCARDED, "discard"), (HANDLED, "handled")] {
        let (ok, out) = run("check", &[], src, tag);
        assert!(ok && !out.contains(NOTICE), "{tag}: {out}");
        let (ok, out) = run("verify", &[], src, &format!("{tag}_v"));
        assert!(ok && out.contains("0 findings"), "{tag} under verify: {out}");
    }
}

#[test]
fn the_legacy_status_form_is_the_bare_form() {
    let (ok, out) = run("check", &[], LEGACY_STATUS, "legacy");
    assert!(!ok && out.contains(NOTICE), "{out}");
}

#[test]
fn the_strict_flag_is_gone() {
    let (ok, out) = run("check", &["--strict-fallible"], DISCARDED, "flag");
    assert!(!ok && out.contains("--strict-fallible"), "an unknown flag is refused, not ignored: {out}");
}
