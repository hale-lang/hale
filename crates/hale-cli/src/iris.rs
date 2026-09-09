//! `hale iris` — the embedded observer (GH #527 B3).
//!
//! The observer is a Hale program the `hale` binary carries as source
//! (`hale-iris`). This module materializes it into the toolchain-hashed
//! cache, builds it with THIS binary (`hale build` on the seed, so the
//! observer is compiled by the exact compiler that built the observed
//! program), and execs the result with the user's arguments. Nothing
//! here is a second scheduler or a second decoder: fuse-hl attaches
//! over the shm segment like any other consumer.
//!
//!   hale iris [port] [artifact]         fusion + HTTP/SSE at :port (8787)
//!   hale iris inspect <artifact> [url]  artifact-side inspector
//!   hale iris --where                   print the cache directory
//!   hale iris --build-only              materialize + build, print the binary
//!
//! `hale run --observe <target>` sets `LOTUS_OBS=1` on the program and
//! launches `hale iris` beside it for the program's lifetime.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

/// Materialize the sources; build a seed if its binary is missing.
/// Returns the binary path.
fn ensure_built(seed: &str, bin: &str) -> Result<(PathBuf, PathBuf), String> {
    let root = hale_iris::materialize().map_err(|e| format!("hale iris: cannot materialize sources: {e}"))?;
    let bin_path = root.join(bin);
    if bin_path.is_file() {
        return Ok((root, bin_path));
    }
    let me = std::env::current_exe().map_err(|e| format!("hale iris: cannot locate the hale binary: {e}"))?;
    eprintln!("hale iris: building the observer ({} @ {})", seed, root.display());
    let status = Command::new(&me)
        .arg("build")
        .arg(root.join(seed))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .map_err(|e| format!("hale iris: build failed to start: {e}"))?;
    if !status.success() {
        return Err(format!("hale iris: building {} failed ({status})", seed));
    }
    if !bin_path.is_file() {
        return Err(format!("hale iris: build produced no binary at {}", bin_path.display()));
    }
    Ok((root, bin_path))
}

fn exec(bin: &Path, args: &[String]) -> ExitCode {
    match Command::new(bin).args(args).status() {
        Ok(st) => match st.code() {
            Some(c) => ExitCode::from(c.clamp(0, 255) as u8),
            None => ExitCode::from(1),
        },
        Err(e) => {
            eprintln!("hale iris: cannot exec {}: {e}", bin.display());
            ExitCode::from(1)
        }
    }
}

/// `hale iris ...` — `args` are the words after `iris`.
pub fn run(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("--help") | Some("-h") => {
            eprintln!("usage: hale iris [port] [artifact.json]");
            eprintln!("       hale iris inspect <artifact.json> [http://host:port]");
            eprintln!("       hale iris --where | --build-only");
            ExitCode::SUCCESS
        }
        Some("--where") => match hale_iris::cache_dir() {
            Some(d) => {
                println!("{}", d.display());
                ExitCode::SUCCESS
            }
            None => {
                eprintln!("hale iris: no cache directory (set XDG_CACHE_HOME or HOME)");
                ExitCode::from(1)
            }
        },
        Some("--build-only") => match ensure_built(hale_iris::FUSE_SEED, hale_iris::FUSE_BIN) {
            Ok((_, bin)) => {
                println!("{}", bin.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Some("inspect") => {
            let (_, bin) = match ensure_built(hale_iris::INSPECT_SEED, hale_iris::INSPECT_BIN) {
                Ok(x) => x,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::from(1);
                }
            };
            exec(&bin, &args[1..])
        }
        _ => {
            let (root, bin) = match ensure_built(hale_iris::FUSE_SEED, hale_iris::FUSE_BIN) {
                Ok(x) => x,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::from(1);
                }
            };
            // fuse-hl's positional argv: port, webroot, [artifact].
            let port = args.first().cloned().unwrap_or_else(|| "8787".to_string());
            let mut fargs = vec![port, root.join(hale_iris::WEBROOT).display().to_string()];
            if let Some(artifact) = args.get(1) {
                fargs.push(artifact.clone());
            }
            exec(&bin, &fargs)
        }
    }
}

/// `hale run --observe`: an iris session for the program's lifetime.
/// Returns the child so the caller can reap it after the program.
pub fn spawn_session() -> Option<std::process::Child> {
    let me = std::env::current_exe().ok()?;
    match Command::new(me).arg("iris").stdin(Stdio::null()).spawn() {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("hale run --observe: could not launch hale iris: {e}");
            None
        }
    }
}
