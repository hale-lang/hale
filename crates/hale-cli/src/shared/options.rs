use std::path::{Path, PathBuf};

use super::imports::{resolve_import, ImportTarget};
use super::source::Disk;
use crate::build_env;

/// The `hale build` / `hale run` flags whose value is the NEXT argv
/// entry rather than part of the flag (there is no `--flag=value`
/// shorthand). The splitter has to know their arity: without it,
/// `hale build --link raylib app.hl` would take `raylib` — the
/// first argument that does not start with `-` — for the target.
pub(crate) const VALUE_FLAGS: &[&str] = &[
    "--link",
    "--csrc",
    "--target",
    "--target-cpu",
    "--target-cache",
    "--api",
    "--env",
    "-o",
    "--out",
];

/// GH #904: the flags `hale build` takes that describe the BUILD
/// rather than the program — a report on stderr, a budget gate, a
/// wasm entry point. `run` and `replay` compile in order to execute,
/// and honor none of them, so they are refused by name instead of
/// being accepted and quietly dropped (the whole complaint of #904).
pub(crate) const BUILD_ONLY_FLAGS: &[&str] = &[
    "-o",
    "--out",
    "--locality-report",
    "--target-cache",
    "--strict",
    "--wrap-main",
];

/// Flags that describe ONE evaluation: an artifact to emit, or a
/// baseline to gate against. Meaningless when the command runs many
/// evaluations.
pub(crate) const PER_SEED_FLAGS: &[&str] = &[
    "--dump-topology",
    "--dump-api",
    "--api",
    "--dump-model",
    "--check-topology",
    "--check-topology-shape",
    "--dump-effects-manifest",
    "--check-effects-manifest",
    "--dump-resource-budget",
    "--check-resource-budget",
    "--dump-alloc-summary",
    "--units",
];

pub(crate) const CHECK_FLAGS: &[(&str, bool)] = &[
    ("--allow-unowned-subscriber", false),
    ("--check-effects-manifest", true),
    ("--check-resource-budget", true),
    ("--check-topology", true),
    ("--check-topology-shape", true),
    ("--dump-alloc-summary", false),
    ("--dump-effects-manifest", false),
    ("--dump-resource-budget", false),
    ("--dump-topology", false),
    // GH #1107: the api binding's description, the model's first
    // wire form. `=<path>` writes it, bare prints it.
    ("--dump-api", false),
    // GH #1417 (R1): the surface rows' documents: the inventory, one
    // exposure's description for a caller, one surface's projections.
    ("--api", false),
    ("--exposure", true),
    ("--caller", true),
    ("--holds", true),
    ("--surface", true),
    ("--openapi", false),
    ("--json-schema", false),
    ("--mcp", false),
    // GH #476 Change 2: derive + print the canonical
    // ApplicationModel (internal format). The demand surface the
    // `hale model dump` shim routes through.
    ("--dump-model", false),
    ("--json", false),
    ("--no-warn-unbounded-alloc", false),
    // Retired opt-in spelling from when the unbounded-alloc survey
    // was off by default. It is a no-op now (the survey is on), but
    // it was accepted for a release and rejecting it would break
    // pipelines that still pass it. Accepted-because-ignored is
    // exactly what this list is replacing, so it is written down
    // rather than left to chance.
    ("--warn-unbounded-alloc", false),
    ("--warn-resource-leak", false),
    // GH #436
    ("--strict-secret", false),
    // GH #738
    ("--sealable", false),
    // GH #736
    ("--flows", false),
    // GH #1076 (U5)
    ("--units", false),
    ("--workspace", false),
    // GH #409
    ("--env", true),
    ("--matrix", false),
    // F.40 P3: the target the program is checked for, parsed as
    // `hale build --target` parses it (design §1.3).
    ("--target", true),
    // F.40 P3 (T4): a link library the build would take, held to the
    // target's `LinkLibrary` cell as the build holds it.
    ("--link", true),
];

/// GH #861: the one argument splitter `hale build` and `hale run`
/// share. Given everything after the subcommand, it returns the
/// flags BEFORE the target, the target, and everything after it.
///
/// The rule is `hale check`'s: the first argument that is not a flag
/// is the target, and a flag is a flag wherever it stands. `build`
/// and `run` used to have no rule at all — the target was argv[2]
/// and `parse_build_options` read from argv[3] — so `hale build
/// --dev app.hl` failed with `not a file or directory: --dev`.
///
/// A value-taking flag missing its value is NOT an error here: it is
/// passed through so `parse_build_options` can report it in the words
/// it always used (`--link requires a library name`), rather than
/// this function inventing a second vocabulary for the same mistake.
pub(crate) fn split_target_args(
    rest: &[String],
) -> (Vec<String>, Option<String>, Vec<String>) {
    let mut before: Vec<String> = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        let a = &rest[i];
        // `-` alone is a target (stdin-ish spellings), not a flag.
        if a == "-" || !a.starts_with('-') {
            return (before, Some(a.clone()), rest[i + 1..].to_vec());
        }
        before.push(a.clone());
        if VALUE_FLAGS.contains(&a.as_str()) {
            if let Some(v) = rest.get(i + 1) {
                before.push(v.clone());
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    (before, None, Vec::new())
}

/// `flags` is every `hale build` flag the splitter found, from both
/// sides of the target (GH #861). The other build-time switches
/// below are read straight from `std::env::args()` with a scan that
/// never depended on position, so they keep working unchanged.
/// `hale build`'s `-o <path>` / `--out <path>`: where the artifact goes,
/// exactly, instead of beside the target. Returns the other flags and
/// the path. The parent directory is created by the caller, so a build
/// can be pointed into a directory that is not there yet
/// (`-o target/seeds/api/api`).
pub(crate) fn take_output_flag(
    flags: &[String],
) -> Result<(Vec<String>, Option<PathBuf>), String> {
    let mut rest = Vec::new();
    let mut out: Option<PathBuf> = None;
    let mut i = 0;
    while i < flags.len() {
        let f = flags[i].as_str();
        if f == "-o" || f == "--out" {
            let Some(v) = flags.get(i + 1).filter(|v| !v.is_empty()) else {
                return Err(format!("hale build: {f} requires the artifact's path"));
            };
            if out.is_some() {
                return Err(format!("hale build: {f} given twice — one build, one artifact"));
            }
            out = Some(PathBuf::from(v));
            i += 2;
        } else {
            rest.push(flags[i].clone());
            i += 1;
        }
    }
    Ok((rest, out))
}

/// Stage-1 FFI (2026-05-22): parse `--link` / `--csrc` flags from
/// `hale build`'s argv. Each flag is repeatable; the flag and its
/// value are two separate argv entries (no `=` shorthand at Stage
/// 1). Unknown flags surface as a clear diagnostic so the user knows
/// we didn't silently swallow them.
///
/// GH #861: takes the flags the splitter found on BOTH sides of the
/// target. It used to read `std::env::args()` from index 3, which is
/// what made a flag before the target unreachable.
///
/// GH #904: `cmd` is the subcommand whose flags these are — `build`,
/// `run` or `replay`, all three of which compile through here — so
/// an unknown flag is reported against the command the user typed.
pub(crate) fn parse_build_options(
    cmd: &str,
    args: &[String],
) -> Result<hale_codegen::BuildOptions, String> {
    let mut opts = build_env::build_options_from_env();
    // #8 dev profile: `HALE_DEV=1` is the environment spelling of
    // `--dev` below. Read here, with the flag, so every command that
    // compiles honors it identically (GH #904; `run` honored
    // neither).
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--link" => {
                let v = args.get(i + 1).ok_or_else(|| {
                    "--link requires a library name (e.g. --link raylib)"
                        .to_string()
                })?;
                opts.link_libs.push(v.clone());
                i += 2;
            }
            "--csrc" => {
                let v = args.get(i + 1).ok_or_else(|| {
                    "--csrc requires a path to a .c file".to_string()
                })?;
                opts.csrc_files.push(std::path::PathBuf::from(v));
                i += 2;
            }
            // GH #1106: the zero-code api path. Accepted by build and
            // run alike; the entry it synthesizes is source-shaped, so
            // the checker sees exactly what an author would write.
            "--api" => {
                let v = args.get(i + 1).ok_or_else(|| {
                    "--api requires a socket path (e.g. --api /run/app.sock)"
                        .to_string()
                })?;
                let path = v.strip_prefix("unix:").unwrap_or(v);
                if path.is_empty() || path.starts_with("--") {
                    return Err(
                        "--api requires a socket path (e.g. --api /run/app.sock)"
                            .to_string(),
                    );
                }
                opts.api = Some(path.to_string());
                i += 2;
            }
            // GH #1109: the deployment target. Its constitution is
            // adopted as `check --env` adopts it, and its role table
            // is baked into the api binding.
            "--env" => {
                let v = args.get(i + 1).ok_or_else(|| {
                    "--env requires an environment name from hale.toml (e.g. --env prod)"
                        .to_string()
                })?;
                if v.is_empty() || v.starts_with("--") {
                    return Err(
                        "--env requires an environment name from hale.toml (e.g. --env prod)"
                            .to_string(),
                    );
                }
                opts.env = Some(v.clone());
                i += 2;
            }
            // F.32-2 (2026-05-25): operator-facing per-locus
            // working-set report. Consumed in main.rs before
            // codegen; recognized here so parse_build_options
            // doesn't error out on an unknown flag.
            "--locality-report" => {
                i += 1;
            }
            // F.32-2 v0.2 (2026-05-25): cache-budget gate.
            // `--target-cache l1|l2|l3` runs the working-set
            // estimator against the named tier and emits a
            // warning (or, with `--strict`, a build error) for
            // any locus whose total exceeds the budget. The
            // value is taken from the next argv entry, parallel
            // to --link / --csrc. Consumed in main.rs; just
            // skipped here so the unknown-flag arm doesn't
            // fire.
            "--target-cache" => {
                // Eat the tier value too; main.rs will re-parse
                // env::args. Defensive: if --target-cache is
                // the last arg we still consume one entry and
                // let main.rs surface the missing-value error
                // (keeps parse_build_options simple).
                if args.get(i + 1).is_some() {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--strict" => {
                i += 1;
            }
            // Browser-playground entry synthesis (handled in the build
            // flow, before typecheck — see `wrap_main_as_wasm_export`).
            // Accepted here so it isn't an "unknown flag".
            "--wrap-main" => {
                i += 1;
            }
            // WASM plan: select the compilation backend. Distinct from
            // `--target-cache` (a working-set gate). `wasm32` emits the
            // relocatable wasm object for the browser/full-stack-web target.
            "--target" => {
                let v = args.get(i + 1).ok_or_else(|| {
                    "--target requires a value (native|wasm32|<triple>)".to_string()
                })?;
                opts.target = compile_target(parse_target(v)?);
                i += 2;
            }
            // Backend CPU tuning for the native target. `native` tunes to
            // the host (best perf, not portable); `baseline` pins a
            // portable x86-64-v3 baseline for distributed artifacts.
            "--target-cpu" => {
                let v = args.get(i + 1).ok_or_else(|| {
                    "--target-cpu requires a value (native|baseline)".to_string()
                })?;
                opts.target_cpu = match v.as_str() {
                    "native" => hale_codegen::TargetCpu::Native,
                    "baseline" | "x86-64-v3" => hale_codegen::TargetCpu::X86_64V3,
                    other => {
                        return Err(format!(
                            "--target-cpu: unknown value `{}` (expected native|baseline)",
                            other
                        ));
                    }
                };
                i += 2;
            }
            // #8 dev profile (2026-07-02): LLVM O1 instead of the
            // O3 release default — build-latency mode. Set HERE
            // since GH #904: `run_build` re-derived it from
            // `env::args()` inside the debug-info branch, so the
            // flag was invisible to every other caller of this
            // parser (and a no-op under LOTUS_NO_DEBUGINFO=1).
            "--dev" => {
                opts.dev_profile = true;
                i += 1;
            }
            other => {
                return Err(format!(
                    "unknown `hale {}` flag: {}",
                    cmd, other
                ));
            }
        }
    }
    Ok(opts)
}

/// `--target`'s value, as `hale build` and `hale check` read it alike:
/// canonical triples, not just the two aliases. A target the compiler
/// can NAME is not necessarily one it can BUILD, so say which of the two
/// this is rather than failing later inside the linker (GH #445).
pub(crate) fn parse_target(v: &str) -> Result<hale_codegen::target::TargetSpec, String> {
    let spec = hale_codegen::target::TargetSpec::parse(v).map_err(|e| format!("--target: {}", e))?;
    let host = hale_codegen::target::TargetSpec::host();
    match spec.support_from(&host) {
        hale_codegen::target::TargetSupport::Planned => Err(format!(
            "--target: `{}` is not buildable yet\n\n{}\n\n\
             The target model knows this platform; the codegen \
             and runtime for it do not exist yet. Track GH #445.",
            spec.triple,
            spec.describe_from(&host),
        )),
        hale_codegen::target::TargetSupport::Cross
        | hale_codegen::target::TargetSupport::ForeignHost
        | hale_codegen::target::TargetSupport::Supported
        | hale_codegen::target::TargetSupport::ObjectOnly => Ok(spec),
    }
}

/// The backend a target selects. GH #969: a native triple that is not
/// the host must not become `Native`, which IS the host — that built a
/// host binary under the target's name. It is its own target (GH #970):
/// linked through zig where the target has a cross toolchain here,
/// emitted as an object otherwise.
pub(crate) fn compile_target(spec: hale_codegen::target::TargetSpec) -> hale_codegen::CompileTarget {
    if spec.is_wasm() {
        hale_codegen::CompileTarget::Wasm32
    } else if spec.triple != hale_codegen::target::TargetSpec::host().triple {
        hale_codegen::CompileTarget::Foreign(spec)
    } else {
        hale_codegen::CompileTarget::Native
    }
}

/// The configured target a snapshot is loaded with: the one `--target`
/// names (`explicit`), or the host when nothing names one. The host is
/// named `host` either way, as the build has always named it.
pub(crate) fn configured_target(
    target: hale_codegen::CompileTarget,
    explicit: bool,
) -> hale_frontend::snapshot::Target {
    let spec = target.spec();
    hale_frontend::snapshot::Target {
        name: match target {
            hale_codegen::CompileTarget::Native => "host".to_string(),
            _ => spec.triple.to_string(),
        },
        spec,
        explicit,
    }
}

/// The refusal of a command that executes what it builds (`run`,
/// `replay`) when the program's effective target is wasm32: a written
/// `target wasm`/`browser_js` declaration selects the wasm backend
/// (T1(b)), whose artifact this host cannot execute — the invocation
/// cell `Run × Wasm32` (`--target wasm32` itself is refused at argument
/// parsing, before any program is read).
pub(crate) fn refuse_unexecutable(cmd: &str, snap: &hale_frontend::snapshot::Snapshot) -> Option<String> {
    let row = snap.demand_target().ok()?;
    let decl = row.declaration.as_ref().filter(|_| row.is_wasm32())?;
    Some(format!(
        "hale {cmd}: this program declares `target {}`, so it builds a wasm32 \
         module this host cannot execute — build it with `hale build` and run \
         it in a host that can",
        decl.name
    ))
}

/// GH #904: the build options of a command that compiles AND THEN
/// EXECUTES — `hale run` and `hale replay`. Exactly `hale build`'s
/// parser, so the three cannot drift, minus the build-reporting
/// flags above and minus a target this host cannot exec.
///
/// `run` used to take no build option at all: it compiled with
/// `BuildOptions::default()` and fingerprinted the DEFAULTS into the
/// execution identity, so `--dev`, `--target-cpu`, `--link` and
/// `--csrc` were unreachable from the documented spot-check, and a
/// recording said it was made under options it was not.
pub(crate) fn parse_exec_build_options(
    cmd: &str,
    flags: &[String],
) -> Result<hale_codegen::BuildOptions, String> {
    if let Some(bad) =
        flags.iter().find(|f| BUILD_ONLY_FLAGS.contains(&f.as_str()))
    {
        return Err(format!(
            "hale {cmd}: `{bad}` is a `hale build` flag — it reports \
             on a build, and `{cmd}` compiles in order to run. \
             `{cmd}` takes the options that change the emitted \
             binary: --dev, --target-cpu, --link, --csrc"
        ));
    }
    let opts = parse_build_options(cmd, flags)?;
    if matches!(opts.target, hale_codegen::CompileTarget::Wasm32) {
        return Err(format!(
            "hale {cmd}: --target wasm32 emits an artifact this host \
             cannot execute — build it with `hale build --target \
             wasm32` and run it in a host that can"
        ));
    }
    if let hale_codegen::CompileTarget::Foreign(spec) = opts.target {
        return Err(format!(
            "hale {cmd}: --target {} is not this host's platform, so \
             nothing it builds can run here — build it with `hale build \
             --target {}` and run it where it belongs",
            spec.triple, spec.triple,
        ));
    }
    Ok(opts)
}

/// Stage-2 FFI (2026-05-22): walk a program's top-level imports,
/// resolve each one against the entry's directory + workspace
/// root (same lookup `resolve_imports` uses), and accumulate the
/// `[ffi]` section of each imported lib's `hale.toml` into a
/// `BuildOptions`. `csrc` paths are resolved relative to the
/// lib's own directory; `link` libs append unconditionally.
///
/// Single-file imports (`import "helpers"` resolving to
/// `helpers.hl`) carry no `hale.toml` and contribute nothing
/// here. Imports that don't resolve are silently skipped — the
/// main resolver surfaces those as diagnostics; double-erroring
/// here just adds noise.
///
/// De-duplication: a lib referenced under two aliases or pulled
/// in transitively (Stage 2 only walks the top-level imports;
/// transitive FFI is a Stage 2-follow-on if/when needed)
/// contributes its flags once per unique lib directory.
pub(crate) fn collect_ffi_from_imports(
    imports: &[hale_syntax::ast::Import],
    importer_dir: &Path,
    workspace_root: Option<&Path>,
) -> hale_codegen::BuildOptions {
    let mut opts = build_env::build_options_from_env();
    let mut seen_dirs: std::collections::BTreeSet<PathBuf> =
        std::collections::BTreeSet::new();
    for imp in imports {
        if imp.path.starts_with("std/") || imp.path == "std" {
            continue;
        }
        let target = match resolve_import(importer_dir, workspace_root, &imp.path, &Disk) {
            Some(t) => t,
            None => continue,
        };
        let lib_dir = match target {
            ImportTarget::SingleFile(_) => continue,
            ImportTarget::Directory(d) => d,
        };
        let canon = lib_dir.canonicalize().unwrap_or_else(|_| lib_dir.clone());
        if !seen_dirs.insert(canon) {
            continue;
        }
        match crate::pkg::read_lib_ffi(&lib_dir) {
            Ok(Some(ffi)) => {
                for lib in ffi.link {
                    opts.link_libs.push(lib);
                }
                for csrc in ffi.csrc {
                    let csrc_path = lib_dir.join(csrc);
                    opts.csrc_files.push(csrc_path);
                }
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!(
                    "warning: reading hale.toml in {}: {}",
                    lib_dir.display(),
                    e,
                );
            }
        }
    }
    opts
}

/// Where a link library came from (T4, design §1.5): an imported
/// package's `hale.toml`, at its `[ffi] link` key's line, or `--link`.
pub(crate) enum LinkInput {
    Manifest { path: PathBuf, line: usize, libs: Vec<String> },
    Flag { lib: String },
}

/// Every link library a build of these imports would take, with where
/// each came from: the `--link` flags, then each imported package's
/// `[ffi] link` ([`collect_ffi_from_imports`]'s walk).
pub(crate) fn link_inputs(
    flags: &[String],
    imports: &[hale_syntax::ast::Import],
    importer_dir: &Path,
    workspace_root: Option<&Path>,
) -> Vec<LinkInput> {
    let mut out: Vec<LinkInput> = flags.iter().map(|l| LinkInput::Flag { lib: l.clone() }).collect();
    let mut seen_dirs = std::collections::BTreeSet::new();
    for imp in imports {
        if imp.path.starts_with("std/") || imp.path == "std" {
            continue;
        }
        let Some(ImportTarget::Directory(lib_dir)) = resolve_import(importer_dir, workspace_root, &imp.path, &Disk) else {
            continue;
        };
        if !seen_dirs.insert(lib_dir.canonicalize().unwrap_or_else(|_| lib_dir.clone())) {
            continue;
        }
        let Ok(Some(ffi)) = crate::pkg::read_lib_ffi(&lib_dir) else { continue };
        if ffi.link.is_empty() {
            continue;
        }
        let path = lib_dir.join("hale.toml");
        // The `link` key's line under `[ffi]`, for the record's position.
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut in_ffi = false;
        let mut line = 1;
        for (i, l) in text.lines().enumerate() {
            let t = l.trim();
            if t.starts_with('[') {
                in_ffi = t == "[ffi]";
            } else if in_ffi && t.starts_with("link") && t[4..].trim_start().starts_with('=') {
                line = i + 1;
                break;
            }
        }
        out.push(LinkInput::Manifest { path, line, libs: ffi.link });
    }
    out
}

/// The `LinkLibrary` cell's refusals for the effective target, one per
/// input, rendered before any tool is looked up (T4): a record against
/// the manifest's `[ffi] link` line, or the `--link` argument named as
/// such. Empty where the target links system libraries.
pub(crate) fn link_refusals(row: &hale_types::capability::TargetRow, inputs: &[LinkInput]) -> Vec<String> {
    use hale_types::capability::{derive_capability_matrix, Capability};
    let Some(class) = row.class else { return Vec::new() };
    let m = derive_capability_matrix();
    let cell = m.behaviour(class, Capability::LinkLibrary).expect("a row");
    let Some(refusal) = cell.refusal() else { return Vec::new() };
    inputs
        .iter()
        .map(|input| match input {
            LinkInput::Manifest { path, line, libs } => {
                let libs = hale_types::capability::libs_hole(libs);
                format!("{}:{line}:1: error: {}", path.display(), refusal.render(&cell.witness, &[("libs", &libs)]))
            }
            LinkInput::Flag { lib } => {
                let libs = hale_types::capability::libs_hole(std::slice::from_ref(lib));
                format!("error: --link {lib}: {}", refusal.render(&cell.witness, &[("libs", &libs)]))
            }
        })
        .collect()
}

/// GH #1109: what `build --env` and `run --env` resolve before the
/// program is parsed: the environment's section (for the constitution
/// it binds) and, onto `options`, its role table, which the api binding
/// bakes in and the fingerprint covers. Without `--env` there is no
/// table: every gate refuses until `LOTUS_API_ROLES` says otherwise.
pub(crate) fn resolve_build_env(
    target: &Path,
    options: &mut hale_codegen::BuildOptions,
) -> Result<Option<(crate::pkg::EnvSpec, Option<String>)>, String> {
    let Some(env) = options.env.clone() else { return Ok(None) };
    let (spec, base) = resolve_env_spec(target, &env)?;
    options.api_roles = Some(env_roles(&spec));
    Ok(Some((spec, base)))
}

/// GH #1109: the role table an environment binds, the one line the api
/// binding bakes in. `build --env`, `run --env`, `check --env` and the
/// matrix's pair all take it from here, so the check judges the binding
/// the build lowers (F.40 phase 4, A1).
pub(crate) fn env_roles(spec: &crate::pkg::EnvSpec) -> String {
    crate::pkg::roles_table(&spec.roles)
}

/// The options the execution identity is computed from (F.40 phase 4,
/// I2): one function for `hale build`, `run` and `replay`, each passing
/// the options its own flags parsed with its `--env` resolved onto them
/// ([`resolve_build_env`], before the load). It appends what the flags
/// do not say, the `[ffi]` link libraries and C sources each imported
/// package's `hale.toml` declares ([`collect_ffi_from_imports`]), so a
/// program importing such a package has one identity whichever verb
/// computes it. All three fingerprint what this returns and build with
/// it, so a package's `[ffi] csrc` and `link` reach `run` and `replay`
/// without `--csrc` / `--link` flags.
///
/// A flag naming what a manifest names is a no-op, not an error: a
/// `--csrc` whose canonical path is a manifest `csrc`, or a `--link` of
/// a library a manifest links, is dropped in favour of the manifest's
/// entry. Appending both handed clang the C file twice, a duplicate
/// symbol at link, and `hale run --csrc glue.c` is the documented way to
/// run an `@ffi` program; dropping the flag's copy leaves the options,
/// and so the identity, exactly the flagless invocation's.
pub(crate) fn identity_options(
    options: &hale_codegen::BuildOptions,
    snap: &hale_frontend::snapshot::Snapshot,
    target: &Path,
) -> hale_codegen::BuildOptions {
    // The imports are the target's own, resolved against the directory
    // they were written in.
    let entry_dir = if target.is_dir() {
        target.to_path_buf()
    } else {
        target.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    let toml_opts = collect_ffi_from_imports(
        snap.entry_imports(),
        &entry_dir,
        super::workspace::find_workspace_root(target).as_deref(),
    );
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let manifest_csrc: std::collections::BTreeSet<PathBuf> =
        toml_opts.csrc_files.iter().map(|p| canon(p)).collect();
    let mut o = options.clone();
    o.link_libs.retain(|lib| !toml_opts.link_libs.contains(lib));
    o.csrc_files.retain(|p| !manifest_csrc.contains(&canon(p)));
    o.link_libs.extend(toml_opts.link_libs);
    o.csrc_files.extend(toml_opts.csrc_files);
    o
}

/// GH #1109: the config a build's snapshot is loaded with, from its
/// flags: the target it compiles for (`explicit` when `--target` named
/// it, so it overrides a source declaration), `--api`, and `--env`'s
/// role table and constitutions (resolved by [`resolve_build_env`]).
/// The environment is a pass of the snapshot's load and part of its key.
pub(crate) fn build_config(
    options: &hale_codegen::BuildOptions,
    env_spec: &Option<(crate::pkg::EnvSpec, Option<String>)>,
    explicit: bool,
) -> hale_frontend::snapshot::Config {
    let target = configured_target(options.target, explicit);
    let mut config = hale_frontend::snapshot::Config::build(target);
    config.api = options.api.clone();
    config.api_roles = options.api_roles.clone();
    config.environment = env_spec.as_ref().map(|(spec, base)| hale_frontend::snapshot::Environment {
        name: options.env.clone().unwrap_or_default(),
        adopt: env_adopts(spec, base),
    });
    config.allow_unowned_subscriber =
        std::env::args().any(|a| a == "--allow-unowned-subscriber");
    config
}

/// Say so, once, when the api binding the sequence generated gates an
/// operation and no environment mapped its roles.
pub(crate) fn note_unmapped_roles(
    surface: Option<&hale_syntax::api_gen::ApiSurface>,
    options: &hale_codegen::BuildOptions,
) {
    let Some(surface) = surface else { return };
    let gated = surface.commands.iter().filter(|c| c.role.is_some()).count()
        + surface.reads.iter().filter(|r| r.role.is_some()).count()
        + surface.streams.iter().filter(|s| s.role.is_some()).count();
    if gated > 0 && options.api_roles.is_none() && surface.binding.roles.is_none() {
        eprintln!(
            "note: {} gated operation(s) and no role table: pass `--env <name>` to bake \
             `[environments.<name>.roles]` from hale.toml, or set LOTUS_API_ROLES at run \
             time; until then every gate refuses",
            gated
        );
    }
}

/// Which constitutions does environment `env` require, and which role
/// table does it bind? Walks up from the target for the nearest
/// `hale.toml`, so `hale check apps/a --env prod` works from anywhere
/// in the tree.
pub(crate) fn resolve_env_check(
    target: &Path,
    env: &str,
) -> Result<(Vec<String>, String), String> {
    let (spec, base) = resolve_env_spec(target, env)?;
    Ok((env_adopts(&spec, &base), env_roles(&spec)))
}

/// The constitutions an environment binds: the workspace base first,
/// then the environment's own addition when it differs.
pub(crate) fn env_adopts(spec: &crate::pkg::EnvSpec, base: &Option<String>) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    if let Some(b) = base {
        v.push(b.clone());
    }
    if let Some(c) = &spec.constitution {
        if Some(c) != v.first() {
            v.push(c.clone());
        }
    }
    v
}

/// GH #1109: the `[environments.<env>]` section the nearest
/// `hale.toml` at or above `target` declares, with the workspace
/// base. `check --env`, `build --env` and `run --env` read its
/// constitution and its `roles` table.
pub(crate) fn resolve_env_spec(
    target: &Path,
    env: &str,
) -> Result<(crate::pkg::EnvSpec, Option<String>), String> {
    let start = if target.is_dir() {
        target.to_path_buf()
    } else {
        target.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    let mut dir = start.canonicalize().unwrap_or(start);
    loop {
        let m = dir.join("hale.toml");
        if m.exists() {
            let (envs, base) = crate::pkg::read_claims_config(&m)?;
            return match envs.get(env) {
                Some(spec) => Ok((spec.clone(), base)),
                None => Err(format!(
                    "no environment `{}` in {} (declared: {})",
                    env,
                    m.display(),
                    if envs.is_empty() {
                        "none".to_string()
                    } else {
                        envs.keys().cloned().collect::<Vec<_>>().join(", ")
                    }
                )),
            };
        }
        match dir.parent() {
            Some(p) => dir = p.to_path_buf(),
            None => {
                return Err(format!(
                    "`--env {}` needs a `hale.toml` declaring \
                     `[environments.{}]`; none found at or above {}",
                    env,
                    env,
                    target.display()
                ))
            }
        }
    }
}

/// GH #296: build-manifest identity — a FRAMED SHA-256 over the
/// build inputs this binary can see:
///
///   - the toolchain source hash (`HALE_TOOLCHAIN_SHA256`, computed
///     by build.rs over the identity-covered crates this CLI was
///     linked against — parser, checker, model, graph core, codegen
///     and its runtime, the stdlib's tables and `.hl` seeds, and this
///     CLI — so two different compiler commits under one version
///     differ);
///   - the CLI crate version;
///   - the build options that alter emitted code;
///   - every source file's source-map path and contents
///     ([`source_frames`]), each length-framed (no concatenation
///     ambiguity), in the source map's order.
///
/// Structural `shape_hash` says "same model"; this says "same build
/// inputs". Residue it cannot see: the LLVM/libc toolchain outside
/// this binary and the linker environment — a post-link binary
/// digest is the staged stronger form.
pub(crate) fn exec_digest(files: &[(&str, &str)], options_fp: &str, plan_digest: u64) -> [u64; 4] {
    let mut buf: Vec<u8> = Vec::new();
    let frame = |b: &[u8], buf: &mut Vec<u8>| {
        buf.extend_from_slice(&(b.len() as u64).to_le_bytes());
        buf.extend_from_slice(b);
    };
    // Round 3: the toolchain half is the FULL workspace-source
    // SHA-256 (parser, analyses, codegen, every runtime TU incl.
    // lotus_obs.c, stdlib seeds, the replay CLI) + rustc version +
    // git commit — computed by build.rs. The old 64-bit stale-CLI
    // hash covered three files and missed the record/replay
    // implementation entirely.
    frame(env!("HALE_TOOLCHAIN_SHA256").as_bytes(), &mut buf);
    frame(env!("CARGO_PKG_VERSION").as_bytes(), &mut buf);
    frame(options_fp.as_bytes(), &mut buf);
    // GH #476 Change 8: the DISPATCH PLAN is part of what a build
    // is. Two builds of byte-identical sources by one toolchain
    // still run different code if the bus lowering differs (an
    // env-forced all-dynamic plan, a future placement-driven
    // flavor), and a recording admitted across that boundary would
    // replay against a program whose bus behaves differently. The
    // plan's digest is framed here so the boundary is a refusal.
    frame(&plan_digest.to_le_bytes(), &mut buf);
    buf.extend_from_slice(&(files.len() as u64).to_le_bytes());
    for (path, src) in files {
        frame(path.as_bytes(), &mut buf);
        frame(src.as_bytes(), &mut buf);
    }
    let d = openssl::sha::sha256(&buf);
    let mut out = [0u64; 4];
    for (i, part) in out.iter_mut().enumerate() {
        *part = u64::from_le_bytes(d[i * 8..i * 8 + 8].try_into().unwrap());
    }
    if out == [0; 4] {
        out[0] = 1;
    }
    out
}

/// The sources [`exec_digest`] frames: each file of the snapshot's
/// source map, by its source-map path, with its text, in the map's order
/// (F.40 phase 4, I3). The source map is the program's one naming of its
/// files — relative to one root, the same whichever target loaded them
/// and wherever the tree is checked out — so the identity has no path
/// logic of its own: a directory build and the replay of its entry file
/// frame one program alike, and two imports with one file name are two
/// paths.
pub(crate) fn source_frames(snap: &hale_frontend::snapshot::Snapshot) -> Vec<(&str, &str)> {
    snap.source_map()
        .iter()
        .zip(snap.file_bases())
        .map(|(file, (_, path, _))| {
            (file.path.as_str(), snap.sources().get(path).map_or("", String::as_str))
        })
        .collect()
}

/// What a build stamps as its identity beside the sources, all of it
/// read from the build's snapshot (F.40 phase 2.3).
pub(crate) struct BuildIdentity {
    /// The model identity (downstream handoff P26): the snapshot's
    /// model's `shape_hash`, the value its topology artifact stamps.
    pub(crate) model_hash: u64,
    /// The digest of the dispatch plan codegen lowers, folded into the
    /// execution identity by [`exec_digest`].
    pub(crate) plan_digest: u64,
    /// The canonical entity ids codegen stamps into the observation
    /// manifest.
    pub(crate) obs_ids: Vec<hale_model::obs_ids::ObsEntityId>,
}

/// GH #476 Change 8: what the BUILD stamps as its identity beside the
/// sources ([`BuildIdentity`]).
///
/// The plan is the lowering view's (F.40 phase 1.5): the one codegen
/// reads, over the program it lowers, so the digest names exactly the
/// lowering the binary carries. The model is the snapshot's, demanded
/// here for a different concern: the model hash is its identity, and
/// the observation entity ids are its entities' identities, which a
/// consumer joins the live manifest to. Both used to come from a second
/// model: the hash rendered a whole artifact over a model of its own
/// and scraped `shape_hash` out of the text. A checked program denotes
/// a model, so the build's snapshot always has one; a blocked model is
/// the caller's to report.
///
/// `options.no_bus_devirt` (`LOTUS_NO_BUS_DEVIRT=1`, the differential
/// harness's control arm) makes codegen emit the empty plan — every
/// subject dynamic — so the identity folded into the exec digest must
/// be the EMPTY plan's. Otherwise the control arm and the live arm
/// would share a build identity while running different lowerings,
/// and a recording taken under one would be admitted against the
/// other.
pub(crate) fn model_identity<'s>(
    snap: &'s hale_frontend::snapshot::Snapshot,
    resolved: &hale_types::resolved::LoweringView,
    options: &hale_codegen::BuildOptions,
) -> Result<BuildIdentity, &'s hale_frontend::snapshot::Blocked> {
    let model = snap.demand_model()?;
    let plan_digest = if options.no_bus_devirt {
        hale_model::dispatch_plan::DispatchPlan::default().digest()
    } else {
        resolved.plan.digest()
    };
    Ok(BuildIdentity {
        model_hash: hale_types::topology_projection::project_shape_hash(model),
        plan_digest,
        obs_ids: hale_model::obs_ids::obs_entity_ids(model),
    })
}
/// A `--flag value` / `--flag=value` reader over an explicit argv
/// slice. `flag_value` inside `run_check_impl` reads the process
/// argv; the arg parser needs the same rules before it has decided
/// what to run.
pub(crate) fn flag_value_in(
    rest: &[String],
    flag: &str,
) -> Result<Option<String>, String> {
    if let Some(eq) = rest.iter().find(|a| {
        a.starts_with(flag) && a.as_bytes().get(flag.len()) == Some(&b'=')
    }) {
        let v = &eq[flag.len() + 1..];
        if v.is_empty() {
            return Err(format!("{}= requires a value", flag));
        }
        return Ok(Some(v.to_string()));
    }
    if let Some(i) = rest.iter().position(|a| a == flag) {
        return match rest.get(i + 1) {
            Some(v) if !v.starts_with('-') => Ok(Some(v.clone())),
            _ => Err(format!(
                "{} requires a value. Use `{} <name>` or `{}=<name>`.",
                flag, flag, flag
            )),
        };
    }
    Ok(None)
}
