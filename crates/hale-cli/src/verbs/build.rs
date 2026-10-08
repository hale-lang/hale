use crate::iris;
use crate::shared::options::parse_exec_build_options;
use super::run::run_program;
use crate::shared::options::split_target_args;
use super::help::usage;
use std::process::ExitCode;
use std::path::Path;
use std::path::PathBuf;
use crate::build_env;
use crate::shared::frontend::LoadMode;
use crate::shared::source::Disk;
use crate::shared::options::build_config;
use crate::shared::options::compile_target;
use crate::shared::options::exec_digest;
use crate::shared::options::identity_options;
use crate::shared::workspace::find_workspace_root;
use crate::shared::options::model_identity;
use crate::shared::options::parse_build_options;
use crate::shared::diag::render_blocked;
use crate::shared::diag::render_codegen_error;
use crate::shared::diag::render_located;
use crate::shared::options::resolve_build_env;
use crate::shared::options::source_frames;
use crate::shared::options::take_output_flag;
use hale_frontend::snapshot::{LoadError, Snapshot};
pub(crate) fn run_build(target: &Path, flags: &[String]) -> ExitCode {
    let (flags_without_out, out_override) = match take_output_flag(flags) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let flags: &[String] = &flags_without_out;
    // Phase 2i: warn if the CLI binary was built against an older
    // codegen+runtime source tree than what's on disk now. Silent
    // miscompile (stale CLI emitting old lowering against new
    // source) is the worst failure mode for a cold-context agent —
    // see `apps/log-router/FRICTION.md` 2026-05-10. The check is
    // best-effort: it skips when source files aren't locatable
    // (installed binary, moved workspace), or when the user
    // explicitly opts out via `HALE_SKIP_STALE_CHECK=1`. Since GH #785
    // the dispatcher runs it for every source-reading command, this
    // one included, so it is not run again here.

    // `--wrap-main` (browser playground): synthesize the wasm `@export`
    // entry from a bare `fn main` on the AST, BEFORE typecheck — so the
    // checker sees the synthesized `target wasm` gate + `@export` locus,
    // and every diagnostic keeps the user's original line/col (no textual
    // wrap, no offset). The wrap itself is a pass of the snapshot's load
    // (`Config::wrap_main`); whether the program may be wrapped is read
    // from the snapshot's effective target once it is loaded, below.
    let wrap_main = std::env::args().any(|a| a == "--wrap-main");

    // Options first: the check answers target questions (GH #970), so
    // it has to know the target, and `--api` (GH #1106) shapes the
    // program the snapshot loads.
    let mut options = match parse_build_options("build", flags) {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("{}", msg);
            return ExitCode::from(2);
        }
    };
    // GH #1109: `--env` — the deployment target's role table and law.
    let env_spec = match resolve_build_env(target, &options) {
        Ok(e) => e,
        Err(msg) => {
            eprintln!("{}", msg);
            return ExitCode::from(2);
        }
    };
    // `--target` names the target: it overrides a source declaration,
    // where the host a build falls back to does not (T1(b)).
    let explicit = flags.iter().any(|f| f == "--target");
    let mut config = build_config(&options, &env_spec, explicit);
    config.wrap_main = wrap_main;

    // F.40 phase 2.2b: one snapshot, and the lowering view demanded
    // from it. File targets follow `import "..."` directives starting
    // from the entry's directory; directory targets bundle every .hl
    // file in the directory as one seed (the per-dir package model —
    // myapp/{main,render,topology}.hl → one binary) and resolve the
    // union of their imports. The load then runs the one sequence
    // `check` runs before its check: the wasm entry wrap, the
    // environment's constitutions, sync inference, the desugars (the
    // JSON parsers and the api binding with the environment's roles
    // among them), the mint.
    let snap = match Snapshot::load(target, LoadMode::WholeSeed, &Disk, config) {
        Ok(s) => s,
        Err(LoadError::Load(f)) => {
            eprintln!("{}", f.text());
            return ExitCode::from(f.code);
        }
        Err(LoadError::Refused(msg)) => {
            eprintln!("{}", msg);
            return ExitCode::from(2);
        }
    };
    let (sources, file_bases) = (snap.sources(), snap.file_bases());

    // The effective target (T1(b)): `--target`, else a written
    // `target wasm`/`browser_js` declaration, else the host. The build
    // emits for it — a declared program builds the wasm module and its
    // loader — and names its artifact after it.
    match snap.demand_target() {
        Ok(row) => {
            // Wasm-only: there is no native entry inversion to wrap, so on
            // a native build `--wrap-main` is a hard error rather than a
            // silent no-op (which would mask a misconfigured playground
            // build). The declaration the wrap injects is a consequence of
            // the target and never selects it, so the row's target is the
            // written sources' own.
            if wrap_main && !row.is_wasm32() {
                eprintln!("error: {}", hale_types::capability::WRAP_MAIN_WORDING);
                return ExitCode::from(2);
            }
            options.target = compile_target(row.effective);
        }
        Err(b) => {
            eprintln!("{}", render_blocked(b, file_bases, sources));
            return ExitCode::from(1);
        }
    }

    // Typecheck before lowering, with the build's rules beside it (the
    // borrow rule, bare fallible calls). The rename table reaches the
    // analysis through the snapshot's bundle, not only in `check`:
    // without it a cross-seed call is an unresolved edge, so an effect
    // assertion violated one seed away compiles, links and ships — a
    // downstream fleet gates on `build` across 109 binaries, and "it
    // built" must not be weaker than "it checked" on a contract the
    // compiler already knows how to evaluate.
    let diags = match snap.demand_check() {
        Ok(c) => &c.diags,
        Err(b) => {
            eprintln!("{}", render_blocked(b, file_bases, sources));
            return ExitCode::from(1);
        }
    };
    if !diags.is_empty() {
        for d in diags {
            eprintln!("{}", render_located(d, file_bases, sources));
        }
        // Warnings print but don't fail the build; only errors do.
        if diags.iter().any(|d| d.is_error()) {
            return ExitCode::from(1);
        }
    }
    // T4: the link libraries the build would take — `--link`, and each
    // imported package's `[ffi] link` — held to the `LinkLibrary` cell
    // before any tool is looked up, each refusal located at its input,
    // as `hale check` locates it.
    let entry_dir = if target.is_dir() {
        target.to_path_buf()
    } else {
        target.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    if let Ok(row) = snap.demand_target() {
        let inputs = crate::shared::options::link_inputs(
            &options.link_libs,
            snap.entry_imports(),
            &entry_dir,
            find_workspace_root(target).as_deref(),
        );
        let refused = crate::shared::options::link_refusals(row, &inputs);
        if !refused.is_empty() {
            for r in refused {
                eprintln!("{r}");
            }
            return ExitCode::from(1);
        }
    }
    // hello-world.hl → hello-world. myapp/ → myapp; output lands next to
    // target. When the user passes `.` (or any path without a useful
    // trailing component — `./`, `..`), `Path::file_name` returns None;
    // canonicalize to recover the actual directory name so the emitted
    // binary is `<dir>/<dir>` instead of `<dir>/main`.
    let output = if target.is_dir() {
        let bin_name = target
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .or_else(|| {
                target.canonicalize().ok().and_then(|p| {
                    p.file_name().map(|s| s.to_string_lossy().into_owned())
                })
            })
            .unwrap_or_else(|| "main".to_string());
        target.join(bin_name)
    } else {
        target.with_extension("")
    };
    // WASM plan: a wasm build emits `<stem>.wasm` (a relocatable wasm
    // object at this stage) rather than the extension-less native binary.
    // Output naming is a property of the target, not a special case
    // spelled at this one call site (GH #445).
    let output = {
        // A foreign native target with no cross toolchain here ends at
        // its relocatable object (GH #970), so it is named as one.
        let spec = options.target.spec();
        let names = spec.filenames();
        let ext = if spec.links_from(&hale_codegen::target::TargetSpec::host()) {
            names.executable
        } else {
            names.object
        };
        let named = if ext.is_empty() {
            output
        } else {
            output.with_extension(ext)
        };
        // `-o` names the artifact exactly: no extension is added or
        // swapped, and its directory is made.
        match &out_override {
            Some(p) => {
                if let Some(dir) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
                    if let Err(e) = std::fs::create_dir_all(dir) {
                        eprintln!("hale build: cannot create {}: {e}", dir.display());
                        return ExitCode::from(1);
                    }
                }
                p.clone()
            }
            None => named,
        }
    };
    // F.32-2 (2026-05-25): operator-facing per-locus working-set
    // report + budget gate.
    //
    // * `--locality-report` emits the full per-locus table on
    //   stderr (informational; build proceeds).
    // * `--target-cache l1|l2|l3` evaluates each locus against
    //   the named cache tier's budget. Over-budget loci surface
    //   as a stderr warning by default, or — with `--strict` —
    //   a build error (exit 1 before codegen).
    // * Both flags can be combined: `--locality-report
    //   --target-cache l2` shows everything AND gates.
    //
    // The estimator is approximate (alignment padding partially
    // accounted, method scratch heuristic-only). The budget
    // gate consults the same numbers the report shows, so a
    // warning matches what the report attributes to each
    // locus.
    let cli_args: Vec<String> = std::env::args().collect();
    let want_report = cli_args.iter().any(|a| a == "--locality-report");
    let target_cache_arg: Option<&str> = {
        let mut found = None;
        let mut it = cli_args.iter();
        while let Some(a) = it.next() {
            if a == "--target-cache" {
                found = it.next().map(|s| s.as_str());
                break;
            }
        }
        found
    };
    let strict = cli_args.iter().any(|a| a == "--strict");
    // Resolve the global target tier early so a parse error
    // surfaces before any analysis runs.
    let global_target: Option<hale_types::working_set::CacheTier> =
        match target_cache_arg {
            Some(raw) => match hale_types::working_set::parse_cache_tier(raw) {
                Some(t) => Some(t),
                None => {
                    eprintln!(
                        "error: --target-cache: unknown tier `{}` \
                         (expected l1 / l2 / l3)",
                        raw
                    );
                    return ExitCode::from(2);
                }
            },
            None => None,
        };
    // The whole seed's program: the one a whole-seed load holds.
    let items: &[hale_syntax::ast::TopDecl] =
        snap.program().map(|p| p.items.as_slice()).unwrap_or(&[]);
    let any_locality_annotation = items.iter().any(|item| {
        matches!(item, hale_syntax::ast::TopDecl::Locus(l) if l.locality.is_some())
    });
    if strict && global_target.is_none() && !any_locality_annotation {
        // `--strict` gates the working-set breaches that
        // surface from `--target-cache` or `@locality(...)`.
        // Without either, no budget applies and `--strict`
        // is a no-op — surface the misconfiguration so a CI
        // job doesn't silently believe it's enforcing
        // anything.
        eprintln!(
            "warning: --strict has no effect without \
             --target-cache l1|l2|l3 or `@locality(...)` annotations"
        );
    }
    // Always run the per-locus evaluator — even without
    // `--target-cache`, loci carrying `@locality(L1|L2|L3)` are
    // a hard contract and need checking. The early exit when
    // there's nothing to evaluate is cheap.
    if want_report || global_target.is_some() || any_locality_annotation {
        let map = hale_types::working_set::compute_program_working_set(items);
        if want_report {
            eprint!(
                "{}",
                hale_types::working_set::render_locality_report(&map)
            );
        }
        let breaches =
            hale_types::working_set::breaches_with_per_locus_budgets(
                &map,
                items,
                global_target,
            );
        if !breaches.is_empty() {
            let severity = if strict { "error" } else { "warning" };
            eprint!(
                "{}",
                hale_types::working_set::render_breach_diagnostic(
                    &breaches, severity,
                )
            );
            if strict {
                return ExitCode::from(1);
            }
        }
    }
    // Stage-2 FFI: append the FFI surface declared by each
    // imported lib's hale.toml [ffi] section. CLI flags from
    // parse_build_options come first (preserves the manual
    // escape hatch); toml-sourced flags append, and a flag naming
    // what a manifest names is dropped (a C file compiled twice is
    // a duplicate symbol at link). The options the identity is computed from, by the function
    // `run` and `replay` compute theirs with (I2); the build builds
    // with them.
    let mut options = identity_options(&options, &snap, target);
    // 2026-07-01 debug story stage 2: DWARF line tables, ON by
    // default (debug sections cost binary bytes, zero runtime
    // speed). LOTUS_NO_DEBUGINFO=1 opts out. The source table is
    // the same (base, path, len) file map diagnostics demux with,
    // plus each file's text for line-start computation.
    let no_dbg = std::env::var("LOTUS_NO_DEBUGINFO")
        .map(|v| v == "1" || v == "true" || v == "TRUE")
        .unwrap_or(false);
    if !no_dbg {
        // (`--dev` / HALE_DEV — LLVM O1 instead of the O3 release
        // default — is set by `parse_build_options` since GH #904.
        // It was read from `env::args()` here, inside this branch,
        // so `LOTUS_NO_DEBUGINFO=1 hale build --dev` silently built
        // at O3 and no other caller of the parser saw the flag at
        // all.)
        options.debug = Some(hale_codegen::DebugSources {
            files: file_bases
                .iter()
                .filter_map(|(base, path, len)| {
                    sources.get(path).map(|text| {
                        hale_codegen::DebugSourceFile {
                            base: *base,
                            len: *len,
                            path: path.clone(),
                            text: text.clone(),
                        }
                    })
                })
                .collect(),
        });
    }
    // The execution identity, stamped LAST: every option that
    // alters emitted code is set by now, and the digest frames the
    // finalized fingerprint together with the dispatch plan. Before
    // this, `hale build` artifacts carried a model hash and no
    // execution identity at all — so a recording from one could not
    // be refused against a differently-lowered sibling, which is
    // exactly what the identity is for.
    //
    // The lowering view is demanded first: the dispatch plan the
    // digest frames is the view's, the one codegen lowers (F.40 phase
    // 1.5). GH #476 Change 8: the canonical entity ids a consumer
    // joins the live manifest to the model with come from the
    // snapshot's model, and so does the model identity (P26) stamped
    // into the binary for the observation segment header.
    let view = match snap.demand_lowering() {
        Ok(v) => v,
        Err(b) => {
            eprintln!("{}", render_blocked(b, file_bases, sources));
            return ExitCode::from(1);
        }
    };
    let identity = match model_identity(&snap, view, &options) {
        Ok(x) => x,
        Err(b) => {
            eprintln!("{}", render_blocked(b, file_bases, sources));
            return ExitCode::from(1);
        }
    };
    options.model_hash = Some(identity.model_hash);
    options.obs_entity_ids = identity.obs_ids;
    options.exec_digest = Some(exec_digest(
        &source_frames(&snap),
        &build_env::options_fingerprint(&options),
        identity.plan_digest,
    ));
    match hale_codegen::build_resolved(view, &output, &options) {
        Ok(()) => {
            eprintln!("built: {}", output.display());
            if let hale_codegen::CompileTarget::Foreign(spec) = options.target {
                if !spec.links_from(&hale_codegen::target::TargetSpec::host()) {
                    eprintln!(
                        "note: a relocatable object for {}, not an executable — \
                         this host has no toolchain to link for that platform \
                         (GH #970)",
                        spec.triple
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            // GH #241 / GH #808 / GH #848: a span-carrying codegen
            // error renders like a check diagnostic (file:line:col +
            // source caret) and everything else keeps the bare line
            // — through the one helper every command that compiles
            // now reports through.
            eprintln!(
                "{}",
                render_codegen_error(&e, file_bases, sources)
            );
            ExitCode::from(1)
        }
    }
}

/// The dispatch arm `main` held inline for this verb, moved out verbatim (C5 step 8).
pub(crate) fn run_build_or_run(cmd: &str, args: &[String]) -> ExitCode {
    let (before, target, after) = split_target_args(&args[2..]);
    let target = match target {
        Some(t) => PathBuf::from(t),
        None => {
            usage();
            return ExitCode::from(2);
        }
    };
    if cmd == "build" {
        // `build` has no trailing operand of its own, so the
        // flags on both sides are one list.
        let mut flags = before;
        flags.extend(after);
        return run_build(&target, &flags);
    }
    // `hale run` compiles the program to a temporary binary
    // (the same codegen backend as `hale build`) and executes
    // it — there is no separate interpreter. The program's
    // trailing argv is forwarded to the exec'd process, so
    // `hale run script.hl foo bar` makes the program's
    // `std::env::arg(1..)` see ["foo", "bar"] exactly as a
    // built binary run directly would. That is why the
    // splitter's rule stops at the target here: after it, a
    // `--flag` is the PROGRAM's, not ours.
    let mut user_args = after;
    // GH #527 B3: `hale run --observe <target>` — the program
    // publishes its observation segment (LOTUS_OBS=1, inherited
    // by the child) and an iris session runs beside it for the
    // program's lifetime. The flag is consumed here; nothing
    // reaches the program's argv. Accepted immediately after
    // the target too, the spelling that shipped in B3. The
    // session writes to its own pipe and dies with this process
    // whatever kills it (GH #905) — the program's stdout stays
    // the command's output, and there is no orphan left holding
    // it open.
    let mut observe = before.iter().any(|f| f == "--observe");
    if !observe && user_args.first().map(String::as_str) == Some("--observe") {
        observe = true;
        user_args.remove(0);
    }
    // GH #904: everything else before the target is a BUILD
    // option, parsed by the parser `hale build` uses and
    // honored. `run` used to compile with `BuildOptions::
    // default()` no matter what was passed, so a build flag was
    // first read as the target, then (GH #900) named and
    // refused — and the documented spot-check `hale run
    // prog.hl` could exercise neither a dev build nor an FFI
    // program. One parser, so `build` and `run` cannot drift.
    let build_flags: Vec<String> = before
        .iter()
        .filter(|f| *f != "--observe")
        .cloned()
        .collect();
    let options = match parse_exec_build_options("run", &build_flags) {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("{}", msg);
            eprintln!(
                "(`hale run` takes its flags before the target; \
                 everything after the target is the program's argv)"
            );
            return ExitCode::from(2);
        }
    };
    if observe {
        // GH #887: `LOTUS_OBS=1` is for the PROGRAM, and it used
        // to be planted in this process's environment for the
        // child to inherit. `set_var` is undefined behaviour once
        // a process has threads, and `iris::spawn_session()` on
        // the next line starts one — so it travels on the child's
        // own `Command` instead, which is where it was always
        // meant to arrive. Nothing in this process reads it.
        let session = iris::spawn_session();
        let code = run_program(&target, &user_args, options, true);
        if let Some(mut s) = session {
            let _ = s.kill();
            let _ = s.wait();
        }
        return code;
    }
    return run_program(&target, &user_args, options, false);
}
