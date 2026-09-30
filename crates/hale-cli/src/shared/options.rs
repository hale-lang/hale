use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::imports::{resolve_import, ImportTarget};
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
    "--dump-model",
    "--check-topology",
    "--check-topology-shape",
    "--dump-effects-manifest",
    "--check-effects-manifest",
    "--dump-resource-budget",
    "--check-resource-budget",
    "--dump-alloc-summary",
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
    ("--workspace", false),
    // GH #409
    ("--env", true),
    ("--matrix", false),
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
                // Canonical triples, not just the two aliases. A target
                // the compiler can NAME is not necessarily one it can
                // BUILD, so say which of the two this is rather than
                // failing later inside the linker (GH #445).
                let spec = hale_codegen::target::TargetSpec::parse(v)
                    .map_err(|e| format!("--target: {}", e))?;
                let host = hale_codegen::target::TargetSpec::host();
                match spec.support_from(&host) {
                    hale_codegen::target::TargetSupport::Planned => {
                        return Err(format!(
                            "--target: `{}` is not buildable yet\n\n{}\n\n\
                             The target model knows this platform; the codegen \
                             and runtime for it do not exist yet. Track GH #445.",
                            spec.triple,
                            spec.describe_from(&host),
                        ));
                    }
                    hale_codegen::target::TargetSupport::Cross
                    | hale_codegen::target::TargetSupport::ForeignHost
                    | hale_codegen::target::TargetSupport::Supported
                    | hale_codegen::target::TargetSupport::ObjectOnly => {}
                }
                // GH #969: a native triple that is not the host must not
                // become `Native`, which IS the host — that built a host
                // binary under the target's name. It is its own target
                // (GH #970): linked through zig where the target has a
                // cross toolchain here, emitted as an object otherwise.
                opts.target = if spec.is_wasm() {
                    hale_codegen::CompileTarget::Wasm32
                } else if spec.triple != host.triple {
                    hale_codegen::CompileTarget::Foreign(spec)
                } else {
                    hale_codegen::CompileTarget::Native
                };
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
        let target = match resolve_import(importer_dir, workspace_root, &imp.path) {
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
    options.api_roles = Some(crate::pkg::roles_table(&spec.roles));
    Ok(Some((spec, base)))
}

/// GH #1109: bind the resolved environment to the parsed program —
/// adopt its constitution as `check --env` does — and lower the api
/// binding with the role table. Says so, once, when the program gates
/// something and no environment mapped its roles.
pub(crate) fn bind_build_env(
    program: &mut hale_syntax::ast::Program,
    env_spec: &Option<(crate::pkg::EnvSpec, Option<String>)>,
    options: &hale_codegen::BuildOptions,
) -> Result<(), String> {
    if let Some((spec, base)) = env_spec {
        let has_main = program
            .items
            .iter()
            .any(|i| matches!(i, hale_syntax::ast::TopDecl::Locus(l) if l.is_main));
        if !has_main {
            return Err(format!(
                "`--env {}` names a deployment target, and a deployment target is an \
                 ENTRYPOINT — this program declares no `main locus`",
                options.env.as_deref().unwrap_or("")
            ));
        }
        for c in env_adopts(spec, base) {
            inject_adopt(program, &c);
        }
    }
    let surface = hale_syntax::api_gen::generate_api(&mut [program], options.api_roles.as_deref());
    if let Some(surface) = surface {
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
    Ok(())
}

/// Which constitution does environment `env` require? Walks up from
/// the target for the nearest `hale.toml`, so `hale check apps/a
/// --env prod` works from anywhere in the tree.
pub(crate) fn resolve_env_constitution(
    target: &Path,
    env: &str,
) -> Result<Vec<String>, String> {
    let (spec, base) = resolve_env_spec(target, env)?;
    Ok(env_adopts(&spec, &base))
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
/// base. `check --env` reads its constitution; `build --env` and
/// `run --env` read that and its `roles` table.
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

/// Shared core of `hale check` (advisories print, only errors
/// fail) and `hale verify` (every finding fails — the CI
/// discipline gate; same ~10 ms analysis, no execution).
/// Returns the process exit CODE rather than an `ExitCode`, because
/// `--workspace` runs this once per seed and has to aggregate the
/// results — and `ExitCode` is opaque, so a caller cannot ask whether
/// one succeeded.
/// Add `adopt <name>;` to a program's main-locus `claims` block,
/// creating the block if the main has none. Returns whether a main
/// was found.
///
/// A duplicate is not added: an entrypoint that already writes
/// `adopt Dev;` and is also deployed to an environment requiring
/// `Dev` adopts it once, not twice.
pub(crate) fn inject_adopt(prog: &mut hale_syntax::ast::Program, name: &str) -> bool {
    use hale_syntax::ast::{ClaimsBlock, Ident, LocusMember, TopDecl};
    let mut found = false;
    for item in &mut prog.items {
        let TopDecl::Locus(l) = item else { continue };
        if !l.is_main {
            continue;
        }
        found = true;
        let id = Ident { name: name.to_string(), span: l.name.span };
        if let Some(LocusMember::Claims(cb)) = l
            .members
            .iter_mut()
            .find(|m| matches!(m, LocusMember::Claims(_)))
        {
            if !cb.adopts.iter().any(|a| a.name == name) {
                cb.adopts.push(id);
            }
        } else {
            l.members.push(LocusMember::Claims(ClaimsBlock {
                entries: Vec::new(),
                adopts: vec![id],
                lib_tier: false,
                span: l.name.span,
            }));
        }
    }
    found
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
///   - every source file's FULL normalized path, byte length, and
///     contents, each length-framed (no concatenation ambiguity).
///
/// Structural `shape_hash` says "same model"; this says "same build
/// inputs". Residue it cannot see: the LLVM/libc toolchain outside
/// this binary and the linker environment — a post-link binary
/// digest is the staged stronger form.
pub(crate) fn exec_digest(
    sources: &BTreeMap<PathBuf, String>,
    entry: &Path,
    options_fp: &str,
    plan_digest: u64,
) -> [u64; 4] {
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
    buf.extend_from_slice(&(sources.len() as u64).to_le_bytes());
    // Logical (entry-relative) source ids: identical trees checked
    // out under different roots are the same build inputs.
    let base = entry.parent().map(Path::to_path_buf);
    for (path, src) in sources {
        let logical = base
            .as_deref()
            .and_then(|b| path.strip_prefix(b).ok())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| {
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
        frame(logical.as_bytes(), &mut buf);
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

/// GH #476 Change 8: what the BUILD stamps as its identity beside the
/// sources — the digest of the dispatch plan codegen lowers (folded
/// into the execution identity by [`exec_digest`]) and the canonical
/// entity ids codegen stamps into the observation manifest.
///
/// The plan is the resolved program's (F.40 phase 1.5): the one
/// codegen reads, over the program it lowers, so the digest names
/// exactly the lowering the binary carries. The model is still
/// derived here, from the checked bundle, for a different concern:
/// the observation entity ids are the model's identities, which a
/// consumer joins the live manifest to.
///
/// `options.no_bus_devirt` (`LOTUS_NO_BUS_DEVIRT=1`, the differential
/// harness's control arm) makes codegen emit the empty plan — every
/// subject dynamic — so the identity folded into the exec digest must
/// be the EMPTY plan's. Otherwise the control arm and the live arm
/// would share a build identity while running different lowerings,
/// and a recording taken under one would be admitted against the
/// other.
pub(crate) fn model_identity(
    bundle: &hale_types::Bundle<'_>,
    resolved: &hale_types::resolved::ResolvedProgram,
    options: &hale_codegen::BuildOptions,
) -> (u64, Vec<hale_model::obs_ids::ObsEntityId>) {
    let model = hale_types::model_builder::derive_application_model(bundle);
    let plan_digest = if options.no_bus_devirt {
        hale_model::dispatch_plan::DispatchPlan::default().digest()
    } else {
        resolved.plan.digest()
    };
    (plan_digest, hale_model::obs_ids::obs_entity_ids(&model))
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
