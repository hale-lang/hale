//! The CPU names the linked LLVM knows, per architecture, for
//! `--target-cpu <cpu>`. LLVM's C API lists no CPUs and aborts the
//! process at a target machine for one it does not know, and the
//! system clang's table can differ from the LLVM this crate links (an
//! Apple clang 21 knows `znver4`; LLVM 18 does not), so the table is
//! read here from that LLVM's own `llc -mcpu=help`, found beside its
//! `llvm-config` (the one `llvm-sys` builds against: `LLVM_SYS_180_PREFIX`,
//! else `llvm-config-18` / `llvm-config` on PATH). Written to
//! `$OUT_DIR/llvm_cpus.rs`; `codegen::llvm_cpus` includes it. Without
//! an `llc` the table is empty and a named CPU is refused, saying so.
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn llvm_bindir() -> Option<PathBuf> {
    if let Ok(p) = env::var("LLVM_SYS_180_PREFIX") {
        let bin = PathBuf::from(p).join("bin");
        if bin.is_dir() {
            return Some(bin);
        }
    }
    for cfg in ["llvm-config-18", "llvm-config"] {
        if let Ok(out) = Command::new(cfg).arg("--bindir").output() {
            if out.status.success() {
                return Some(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()));
            }
        }
    }
    None
}

fn llvm_version(bindir: &Path) -> String {
    for cfg in [bindir.join("llvm-config"), PathBuf::from("llvm-config-18"), PathBuf::from("llvm-config")] {
        if let Ok(out) = Command::new(&cfg).arg("--version").output() {
            if out.status.success() {
                return String::from_utf8_lossy(&out.stdout).trim().to_string();
            }
        }
    }
    String::new()
}

/// `llc -mtriple=<triple> -mcpu=help` lists, after the line
/// `Available CPUs for this target:`, one CPU per line as
/// `  <name> - Select the <name> processor.`, then a blank line and
/// the features.
fn cpus(llc: &Path, triple: &str) -> Vec<String> {
    let Ok(out) = Command::new(llc).args([&format!("-mtriple={triple}"), "-mcpu=help"]).output() else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    let mut names = Vec::new();
    let mut in_cpus = false;
    for line in text.lines() {
        if line.starts_with("Available CPUs for this target:") {
            in_cpus = true;
            continue;
        }
        if !in_cpus {
            continue;
        }
        if line.trim().is_empty() {
            if !names.is_empty() {
                break;
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("  ") {
            if let Some(name) = rest.split_whitespace().next() {
                names.push(name.to_string());
            }
        }
    }
    names
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=LLVM_SYS_180_PREFIX");
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let bindir = llvm_bindir();
    let llc = bindir
        .as_ref()
        .map(|b| b.join("llc"))
        .filter(|p| p.is_file())
        .or_else(|| ["llc-18", "llc"].iter().map(PathBuf::from).find(|c| Command::new(c).arg("--version").output().is_ok()));
    let version = bindir.as_deref().map(llvm_version).unwrap_or_default();
    let mut tables = Vec::new();
    for (konst, triple) in [("X86_64", "x86_64-unknown-linux-gnu"), ("AARCH64", "aarch64-unknown-linux-gnu")] {
        let names = match &llc {
            Some(llc) => cpus(llc, triple),
            None => Vec::new(),
        };
        tables.push(format!(
            "pub const {konst}: &[&str] = &[{}];\n",
            names.iter().map(|n| format!("{n:?}")).collect::<Vec<_>>().join(", ")
        ));
    }
    match &llc {
        Some(llc) => println!("cargo:rerun-if-changed={}", llc.display()),
        None => println!("cargo:warning=no llc beside llvm-config: `--target-cpu <cpu>` will be refused by this build"),
    }
    let src = format!(
        "/// The linked LLVM's version, from its `llvm-config --version` at build time.\npub const LLVM_VERSION: &str = {version:?};\n{}",
        tables.join("")
    );
    fs::write(out.join("llvm_cpus.rs"), src).expect("write llvm_cpus.rs");
}
