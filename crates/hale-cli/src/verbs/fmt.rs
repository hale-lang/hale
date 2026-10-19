use std::process::ExitCode;
use std::path::Path;
use std::path::PathBuf;
use std::env;
use std::fs;
/// `hale fmt` — format `.hl` files in place (spec/testing.md:
/// Go-style, zero config). Targets may be files or directories
/// (recursed; `vendor/` and dot-dirs skipped); no target = cwd.
///
///   --check   don't write; exit 1 if any file would change,
///             listing them (CI gate)
///   --diff    don't write; print a unified-ish before/after for
///             files that would change
///   --stdin   read source on stdin, write formatted to stdout
///             (editor integration)
///
/// A file that doesn't lex is reported and skipped (exit 1): the
/// formatter never touches a file it can't fully tokenize. The
/// internal re-lex equivalence gate means a formatter bug can't
/// change what the compiler sees — on gate failure the file is
/// reported and left untouched.
pub(crate) fn run_fmt(rest: &[String]) -> ExitCode {
    let mut check = false;
    let mut diff = false;
    let mut stdin_mode = false;
    let mut targets: Vec<PathBuf> = Vec::new();
    for a in rest {
        match a.as_str() {
            "--check" => check = true,
            "--diff" => diff = true,
            "--stdin" => stdin_mode = true,
            other if other.starts_with('-') => {
                eprintln!("hale fmt: unknown flag {}", other);
                return ExitCode::from(2);
            }
            other => targets.push(PathBuf::from(other)),
        }
    }

    if stdin_mode {
        use std::io::Read;
        let mut src = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut src) {
            eprintln!("hale fmt: reading stdin: {}", e);
            return ExitCode::from(1);
        }
        return match hale_syntax::fmt::format_source(&src) {
            Ok(out) => {
                print!("{}", out);
                ExitCode::SUCCESS
            }
            Err(hale_syntax::fmt::FmtError::Parse(diags)) => {
                for d in &diags {
                    eprintln!("hale fmt: {:?}", d);
                }
                ExitCode::from(1)
            }
            Err(hale_syntax::fmt::FmtError::Changed(_)) => {
                eprintln!(
                    "hale fmt: internal error: formatting would alter \
                     the token stream (bug — input left untouched)"
                );
                ExitCode::from(1)
            }
        };
    }

    if targets.is_empty() {
        targets.push(
            env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        );
    }
    let mut files: Vec<PathBuf> = Vec::new();
    for t in &targets {
        collect_hl_files(t, &mut files);
    }
    files.sort();
    files.dedup();

    let mut changed: Vec<PathBuf> = Vec::new();
    let mut failed = false;
    for f in &files {
        let src = match fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("hale fmt: could not read {}: {}", f.display(), e);
                failed = true;
                continue;
            }
        };
        let out = match hale_syntax::fmt::format_source(&src) {
            Ok(o) => o,
            Err(hale_syntax::fmt::FmtError::Parse(_)) => {
                eprintln!(
                    "hale fmt: {}: does not lex — skipped",
                    f.display()
                );
                failed = true;
                continue;
            }
            Err(hale_syntax::fmt::FmtError::Changed(_)) => {
                eprintln!(
                    "hale fmt: {}: internal equivalence-gate failure \
                     (bug — file left untouched)",
                    f.display()
                );
                failed = true;
                continue;
            }
        };
        if out == src {
            continue;
        }
        changed.push(f.clone());
        if diff {
            print_fmt_diff(f, &src, &out);
        } else if !check {
            if let Err(e) = fs::write(f, &out) {
                eprintln!(
                    "hale fmt: could not write {}: {}",
                    f.display(),
                    e
                );
                failed = true;
            }
        }
    }

    if check {
        for f in &changed {
            println!("{}", f.display());
        }
        if !changed.is_empty() {
            return ExitCode::from(1);
        }
    } else if !diff {
        for f in &changed {
            println!("formatted {}", f.display());
        }
    }
    if failed {
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

/// Recursively collect `.hl` files. Directories named `vendor` or
/// starting with `.` are skipped (vendored pins are frozen — see
/// pond's promotion banners — and formatting them would churn
/// upstream diffs).
pub(crate) fn collect_hl_files(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        if path.extension().is_some_and(|e| e == "hl") {
            out.push(path.to_path_buf());
        }
        return;
    }
    if !path.is_dir() {
        eprintln!("hale fmt: {} not found", path.display());
        return;
    }
    let Ok(entries) = fs::read_dir(path) else { return };
    for entry in entries.flatten() {
        let p = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            if name == "vendor" || name.starts_with('.') {
                continue;
            }
            collect_hl_files(&p, out);
        } else if name.ends_with(".hl") {
            out.push(p);
        }
    }
}

/// Minimal line-based change listing (not a real unified diff —
/// enough to see what fmt would do without writing).
pub(crate) fn print_fmt_diff(path: &Path, before: &str, after: &str) {
    println!("--- {}", path.display());
    let b: Vec<&str> = before.lines().collect();
    let a: Vec<&str> = after.lines().collect();
    let n = b.len().max(a.len());
    for i in 0..n {
        let bl = b.get(i).copied().unwrap_or("");
        let al = a.get(i).copied().unwrap_or("");
        if bl != al {
            println!("{}: - {}", i + 1, bl);
            println!("{}: + {}", i + 1, al);
        }
    }
}
