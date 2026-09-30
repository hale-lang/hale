use std::collections::BTreeMap;
use std::process::ExitCode;
use std::path::Path;
use std::path::PathBuf;
use crate::shared::process::RunScratch;
use crate::shared::options::collect_ffi_from_imports;
use crate::shared::diag::diag_file_name;
use std::env;
use std::fs;
use crate::shared::frontend::LoadMode;
use crate::shared::source::Overlay;
use crate::shared::workspace::find_workspace_root;
use crate::shared::diag::render_blocked;
use crate::shared::diag::render_codegen_error;
use hale_frontend::snapshot::{Config, LoadError, Snapshot, Target};
/// `hale bench [file | dir] [-run <substr>] [--json]` — the Layer-3
/// runner (spec/testing.md). Discovers `*_bench.hl` files; each
/// zero-param free fn named `bench_*` is a benchmark. The runner
/// appends a synthesized driver `main` to the file, read in place of
/// the file itself (an overlay on its path, so relative imports
/// resolve identically), checks and compiles it at the release
/// profile with the same `[ffi]` pickup as build/test, and runs it. The driver self-calibrates: batch sizes grow ×10
/// until a batch takes ≥100ms, then reports ns/op and allocs/op
/// (`std::diag::heap_alloc_count` — shown as `-` when the counting
/// shim is absent). Baselines and `-compare` remain planned.
pub(crate) fn run_bench(args: &[String]) -> ExitCode {
    let mut target: Option<PathBuf> = None;
    let mut run_filter: Option<String> = None;
    let mut json = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "-run" || a == "--run" {
            match args.get(i + 1) {
                Some(v) => {
                    run_filter = Some(v.clone());
                    i += 2;
                }
                None => {
                    eprintln!("hale bench: {} requires a substring", a);
                    return ExitCode::from(2);
                }
            }
        } else if a == "--json" {
            json = true;
            i += 1;
        } else if a.starts_with('-') {
            eprintln!("hale bench: unknown flag `{}`", a);
            return ExitCode::from(2);
        } else if target.is_none() {
            target = Some(PathBuf::from(a));
            i += 1;
        } else {
            eprintln!("hale bench: unexpected extra argument `{}`", a);
            return ExitCode::from(2);
        }
    }
    let target = target
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let mut files: Vec<PathBuf> = Vec::new();
    if target.is_file() {
        files.push(target.clone());
    } else if target.is_dir() {
        collect_bench_files(&target, &mut files);
        files.sort();
    } else {
        eprintln!("hale bench: {} not found", target.display());
        return ExitCode::from(1);
    }
    if files.is_empty() {
        eprintln!("hale bench: no *_bench.hl files under {}", target.display());
        return ExitCode::from(1);
    }

    let mut failed = false;
    let mut json_items: Vec<serde_json::Value> = Vec::new();
    for f in &files {
        match run_bench_file(f, run_filter.as_deref()) {
            Ok(results) => {
                for r in results {
                    if json {
                        json_items.push(serde_json::json!({
                            "file": f.display().to_string(),
                            "name": r.name,
                            "iters": r.iters,
                            "ns_per_op": r.ns_per_op,
                            "allocs_per_op": r.allocs_per_op,
                        }));
                    } else {
                        let allocs = match r.allocs_per_op {
                            Some(a) => format!("{} allocs/op", a),
                            None => "- allocs/op".to_string(),
                        };
                        println!(
                            "{:<40} {:>12} iters {:>12} ns/op   {}",
                            r.name, r.iters, r.ns_per_op, allocs
                        );
                    }
                }
            }
            Err(e) => {
                // GH #848: a located codegen error opens with this
                // file's canonical path, line and column already —
                // prefixing it names the file twice and pushes the
                // `path:line:col` off the start of the line, which
                // is where anything reading compiler output looks
                // for it. Every other complaint (`does not parse`,
                // `read: …`) still needs the file said once.
                if e.starts_with(&diag_file_name(f)) {
                    eprintln!("{}", e);
                } else {
                    eprintln!("hale bench: {}: {}", f.display(), e);
                }
                failed = true;
            }
        }
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json_items)
                .unwrap_or_else(|_| "[]".into())
        );
    }
    if failed {
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

pub(crate) fn collect_bench_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let p = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            if name == "vendor" || name.starts_with('.') {
                continue;
            }
            collect_bench_files(&p, out);
        } else if name.ends_with("_bench.hl") {
            out.push(p);
        }
    }
}

pub(crate) struct BenchResult {
    pub(crate) name: String,
    pub(crate) iters: i64,
    pub(crate) ns_per_op: i64,
    pub(crate) allocs_per_op: Option<i64>,
}

pub(crate) fn run_bench_file(
    entry: &Path,
    filter: Option<&str>,
) -> Result<Vec<BenchResult>, String> {
    let src = fs::read_to_string(entry)
        .map_err(|e| format!("read: {}", e))?;
    let program = hale_syntax::parse_source(&src)
        .map_err(|_| "does not parse".to_string())?;

    // Contract: bench_* zero-param free fns; no main of its own
    // (the driver synthesizes one).
    let mut benches: Vec<String> = Vec::new();
    let mut has_main = false;
    for item in &program.items {
        if let hale_syntax::ast::TopDecl::Fn(fd) = item {
            if fd.name.name == "main" {
                has_main = true;
            }
            if fd.name.name.starts_with("bench_") && fd.params.is_empty() {
                if let Some(f) = filter {
                    if !fd.name.name.contains(f) {
                        continue;
                    }
                }
                benches.push(fd.name.name.clone());
            }
        }
    }
    if has_main {
        return Err(
            "a *_bench.hl must not define `main` — the runner \
             synthesizes the driver"
                .into(),
        );
    }
    if benches.is_empty() {
        return Ok(Vec::new());
    }

    // Synthesized driver: per bench fn, calibrate batch ×10 until a
    // batch takes >= 100ms, then report the final batch's numbers.
    let mut driver = String::from("\n// --- hale bench driver (synthesized) ---\n");
    for b in &benches {
        driver.push_str(&format!(
            r#"fn __bench_drive_{b}() {{
    let mut batch = 1;
    let mut elapsed = 1;
    let mut allocs = 0;
    while true {{
        let a0 = std::diag::heap_alloc_count();
        let t0 = std::time::monotonic_ns();
        let mut i = 0;
        while i < batch {{ {b}(); i = i + 1; }}
        let t1 = std::time::monotonic_ns();
        let a1 = std::diag::heap_alloc_count();
        elapsed = t1 - t0;
        if elapsed < 1 {{ elapsed = 1; }}
        allocs = a1 - a0;
        if a0 < 0 {{ allocs = 0 - batch; }}
        if elapsed >= 100000000 {{ break; }}
        if batch >= 100000000 {{ break; }}
        batch = batch * 10;
    }}
    println("HALE_BENCH {b} ", batch, " ", elapsed / batch, " ", allocs / batch);
}}
"#,
            b = b
        ));
    }
    driver.push_str("fn main() {\n");
    for b in &benches {
        driver.push_str(&format!("    __bench_drive_{}();\n", b));
    }
    driver.push_str("}\n");

    // The bench file with the driver appended, read AS the bench file:
    // an overlay on its own path, so relative imports resolve from its
    // directory, and every position — the user's text is a prefix of
    // the overlay — names the file the reader can open (GH #848). There
    // is no temp copy to write, re-label or delete.
    let mut augmented = src.clone();
    augmented.push_str(&driver);
    let buffers: BTreeMap<PathBuf, String> = std::iter::once((entry.to_path_buf(), augmented)).collect();

    let bench_scratch = RunScratch::new("bench")?;
    // F.40 phase 2.2b: the snapshot every build path loads, for the
    // host, and the lowering view demanded from it after the check.
    let snap = match Snapshot::load(entry, LoadMode::WholeSeed, &Overlay::new(&buffers), Config::build(Target::host())) {
        Ok(s) => s,
        Err(LoadError::Load(f)) => return Err(f.text()),
        Err(LoadError::Refused(msg)) => return Err(msg),
    };
    let (sources, file_bases) = (snap.sources(), snap.file_bases());
    let entry_dir = entry.parent().unwrap_or(Path::new(".")).to_path_buf();
    let options = collect_ffi_from_imports(
        snap.entry_imports(),
        &entry_dir,
        find_workspace_root(entry).as_deref(),
    );
    let bin = bench_scratch.path("bench");
    // Release profile on purpose: benchmarks measure the shipped
    // optimization level. A check that reports an error blocks the
    // view, and is the failure; a warning is not printed.
    let view = snap.demand_lowering().map_err(|b| render_blocked(b, file_bases, sources))?;
    hale_codegen::build_resolved(view, &bin, &options)
        .map_err(|e| render_codegen_error(&e, file_bases, sources))?;

    let out = std::process::Command::new(&bin)
        .output()
        .map_err(|e| format!("run: {}", e));
    let out = out?;
    if !out.status.success() {
        return Err(format!(
            "bench binary exited {:?}:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut results = Vec::new();
    for line in stdout.lines() {
        let Some(rest) = line.strip_prefix("HALE_BENCH ") else {
            // Benchmarks may print their own output — pass through.
            if !line.trim().is_empty() {
                println!("{}", line);
            }
            continue;
        };
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.len() != 4 {
            continue;
        }
        let allocs: i64 = parts[3].parse().unwrap_or(0);
        results.push(BenchResult {
            name: parts[0].to_string(),
            iters: parts[1].parse().unwrap_or(0),
            ns_per_op: parts[2].parse().unwrap_or(0),
            allocs_per_op: if allocs < 0 { None } else { Some(allocs) },
        });
    }
    Ok(results)
}
