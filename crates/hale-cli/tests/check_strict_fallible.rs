//! GH #738 — a bare fallible stdlib call (no `or`) is a warning by
//! default, an error under `hale check --strict-fallible`, and a
//! `hale verify` failure like every advisory. A handled call and a
//! deliberately discarded one say nothing. The legacy Int-status form
//! is the bare form and is reported the same way. The typing is
//! unchanged, so the program still builds either way.

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
fn an_unhandled_call_warns_then_fails_under_the_flag_and_under_verify() {
    let (ok, out) = run("check", &[], UNHANDLED, "warn");
    assert!(ok, "the default is a warning, not a failure: {out}");
    assert!(
        out.contains("warning:") && out.contains("`std::io::fs::write_file`") && out.contains(NOTICE),
        "the warning names the callee, its payload and the missing disposition: {out}"
    );
    assert!(out.contains("or raise") && out.contains("or discard") && out.contains("or handler(err)"), "{out}");
    let (ok, out) = run("check", &["--strict-fallible"], UNHANDLED, "strict");
    assert!(!ok && out.contains("error") && out.contains(NOTICE), "strict: {out}");
    let (ok, out) = run("verify", &[], UNHANDLED, "verify");
    assert!(!ok && out.contains(NOTICE), "verify gates on the advisory: {out}");
    // the typing is unchanged: the bare form still builds
    let (ok, out) = run("build", &[], UNHANDLED, "build");
    assert!(ok, "the legacy form builds: {out}");
}

#[test]
fn a_discarded_or_handled_call_says_nothing() {
    for (src, tag) in [(DISCARDED, "discard"), (HANDLED, "handled")] {
        let (ok, out) = run("check", &["--strict-fallible"], src, tag);
        assert!(ok && !out.contains(NOTICE), "{tag}: {out}");
        let (ok, out) = run("verify", &[], src, &format!("{tag}_v"));
        assert!(ok && out.contains("0 findings"), "{tag} under verify: {out}");
    }
}

#[test]
fn the_legacy_status_form_is_the_bare_form() {
    let (ok, out) = run("check", &[], LEGACY_STATUS, "legacy");
    assert!(ok && out.contains("warning:") && out.contains(NOTICE), "{out}");
    let (ok, out) = run("check", &["--strict-fallible"], LEGACY_STATUS, "legacy_strict");
    assert!(!ok && out.contains(NOTICE), "{out}");
}
