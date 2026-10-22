//! A fleet workspace for the fleet tests: seeds written into a temp
//! directory, and a fleet seed (`fleet locus`) over their mains. A fleet
//! is checked from its seed (`hale fleet check <dir>`); its routes are
//! derived from the instances' artifacts and its bindings.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// A fresh directory for one test's workspace.
pub fn root(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hale_fleetseed_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir");
    d
}

pub fn write(root: &Path, rel: &str, src: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(&p, src).expect("write");
}

/// `hale <args>` in `cwd`: the combined output and the exit code.
pub fn hale_in(cwd: &Path, args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .current_dir(cwd)
        .args(args)
        .env("HALE_SKIP_STALE_CHECK", "1")
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

/// `hale <args>` in `cwd`: stdout alone.
pub fn stdout_in(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .current_dir(cwd)
        .args(args)
        .env("HALE_SKIP_STALE_CHECK", "1")
        .output()
        .expect("run hale");
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// A fleet seed's source: `imports` as `(alias, path)`, then the fleet
/// locus with each block's lines as written (a block with no lines is
/// left out).
pub fn fleet(
    imports: &[(&str, &str)],
    params: &[&str],
    placement: &[&str],
    bindings: &[&str],
    claims: &[&str],
) -> String {
    let mut s = String::new();
    for (alias, path) in imports {
        s.push_str(&format!("import \"{path}\" as {alias};\n"));
    }
    s.push_str("\nfleet locus Prod {\n");
    for (name, lines) in [
        ("params", params),
        ("placement", placement),
        ("bindings", bindings),
        ("claims", claims),
    ] {
        if lines.is_empty() {
            continue;
        }
        s.push_str(&format!("    {name} {{\n"));
        for l in lines {
            s.push_str(&format!("        {l}\n"));
        }
        s.push_str("    }\n");
    }
    s.push_str("}\n");
    s
}
