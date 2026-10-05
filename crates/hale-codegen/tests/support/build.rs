//! What a test builds with.
//!
//! `BuildOptions` has no default for the runtime-object cache: the caller
//! chooses where it lives. A test's choice is its checkout's own
//! `CARGO_TARGET_TMPDIR` (Cargo sets it for integration tests: a
//! directory under `target/`, per checkout, never the system temp dir and
//! never shared between users). The cache is content-addressed and its
//! objects are written by a unique temp name and renamed into place, so
//! every test process of the run can share it safely, and it stays warm
//! from run to run; a directory of each test's own would recompile the
//! runtime's C (about 3 s) in every one of the suite's 1,600 test
//! processes.
//!
//! Start from [`options`] and set what a test needs on top of it:
//! `BuildOptions { asan: true, ..build::options() }`.
//!
//! A test that holds source text builds it with [`build_source`], which
//! loads it as a verb loads a seed of one file (F.40 phase 4, T1).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hale_codegen::{BuildOptions, CodegenError, CompileTarget};
use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, LoadError, Snapshot, Target};
use hale_frontend::source::Overlay;

/// The directory a test caches compiled runtime objects in.
pub fn cache_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("hale-runtime-cache")
}

/// The default build options for a test, caching in [`cache_dir`].
pub fn options() -> BuildOptions {
    BuildOptions::new(cache_dir())
}

/// Where [`load_seed`] puts a test's source text: a `main.hl` in a
/// directory nothing creates. The text is an overlay buffer at that path
/// and nothing is written there, so every test can share it; the path is
/// fixed so that two builds of one text are one build. An `import` is
/// resolved against the directory, which holds nothing else.
#[allow(dead_code)]
pub const SEED_ENTRY: &str = "/hale-test-seed/main.hl";

/// The harness's configuration for `options`, as
/// `build_executable_with_options` makes it: `Config::harness` (lowering
/// not gated on the check) for the target the options name, the host
/// included, with the options' api and roles.
#[allow(dead_code)]
pub fn harness_config(options: &BuildOptions) -> Config {
    let spec = options.target.spec();
    // A harness build names its target, the host included: the view's
    // effective target is the one lowering emits for, so its cells are
    // the ones the build reads (a harness native build of a program
    // that declares `target wasm` lowers natively, as it always has).
    let target = Target {
        name: match options.target {
            CompileTarget::Native => "host".to_string(),
            _ => spec.triple.to_string(),
        },
        spec,
        explicit: true,
    };
    let mut config = Config::harness(target);
    config.api = options.api.clone();
    config.api_roles = options.api_roles.clone();
    config
}

/// Load `source` as `hale build main.hl` loads a seed of one file
/// (`LoadMode::WholeSeed`), from an overlay buffer at [`SEED_ENTRY`], and
/// shape it as `config` says.
#[allow(dead_code)]
pub fn load_seed(source: &str, config: Config) -> Result<Snapshot, LoadError> {
    let entry = PathBuf::from(SEED_ENTRY);
    let buffers: BTreeMap<PathBuf, String> = std::iter::once((entry.clone(), source.to_string())).collect();
    Snapshot::load(&entry, LoadMode::WholeSeed, &Overlay::new(&buffers), config)
}

/// Build `source` to an executable at `output_path`: [`load_seed`] with
/// [`harness_config`], the lowering view demanded from the snapshot, then
/// `build_resolved`. What `build_executable_with_options` does for a
/// parsed program, from the text a verb would read, with its errors
/// mapped as it maps them. A program that does not parse is refused as
/// the adapter refuses a load that fails, `CodegenError::Unsupported`
/// with the load's rendering, which names the line and column.
#[allow(dead_code)]
pub fn build_source(source: &str, output_path: &Path, options: &BuildOptions) -> Result<(), CodegenError> {
    let snap = match load_seed(source, harness_config(options)) {
        Ok(s) => s,
        Err(LoadError::Refused(msg)) => return Err(CodegenError::Unsupported(msg)),
        Err(LoadError::Load(f)) => return Err(CodegenError::Unsupported(f.text())),
    };
    let view = snap.demand_lowering().map_err(|b| match (b.family, b.because.first()) {
        ("target_capability", Some(d)) => CodegenError::CapabilityRefused(d.message.clone(), Some(d.span)),
        _ => CodegenError::Unsupported(b.refused.clone().unwrap_or_else(|| {
            b.because.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; ")
        })),
    })?;
    hale_codegen::build_resolved(view, output_path, options)
}
