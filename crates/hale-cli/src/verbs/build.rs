use crate::shared::imports::AliasScopes;
use std::collections::BTreeMap;
use crate::EffectTable;
use crate::shared::frontend::EntryCtx;
use std::process::ExitCode;
use crate::shared::imports::FileClaims;
use crate::shared::imports::ImportDiag;
use crate::shared::imports::ImportRenames;
use std::path::Path;
use std::path::PathBuf;
use hale_syntax::ast::Program;
use crate::shared::options::bind_build_env;
use crate::build_env;
use crate::shared::frontend::collect_ap_files;
use crate::shared::options::collect_ffi_from_imports;
use crate::shared::options::exec_digest;
use crate::shared::workspace::find_workspace_root;
use crate::shared::frontend::merge_programs;
use crate::shared::options::model_identity;
use crate::shared::options::parse_build_options;
use crate::shared::frontend::parse_files;
use crate::shared::frontend::parse_with_imports;
use crate::shared::diag::render_codegen_error;
use crate::shared::diag::render_located;
use crate::shared::diag::report_import_diags;
use crate::shared::options::resolve_build_env;
use crate::shared::imports::resolve_imports;
use crate::shared::imports::scope_import_aliases;
use crate::shared::options::take_output_flag;
use crate::shared::imports::unscoped_alias_uses;
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

    // File targets follow `import "..."` directives starting from
    // the entry's directory; directory targets bundle every .hl
    // file in the directory as one seed (the per-dir package
    // model — myapp/{main,render,topology}.hl → one binary). The
    // directory shape is the user-facing answer to the
    // single-file-app-monolith friction; the file shape stays for
    // backwards compatibility and for one-off scripts.
    let (mut program, renames, sources, file_bases, output, entry_ctx) = if target.is_file() {
        let (program, renames, sources, file_bases, ctx) = match parse_with_imports(target) {
            Ok(x) => x,
            Err(errors) => return report_import_diags(&errors),
        };
        // hello-world.hl → hello-world
        let output = target.with_extension("");
        (program, renames, sources, file_bases, output, ctx)
    } else if target.is_dir() {
        let files = match collect_ap_files(target) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("{}", e);
                return ExitCode::from(1);
            }
        };
        let (programs, sources, mut dir_file_bases) = match parse_files(&files) {
            Ok(x) => x,
            // As for `run`: located text, unchanged (GH #777).
            Err(f) => return f.report_text(),
        };
        // Collect the union of all imports across the bundle's
        // files. Multiple files in one seed may share an import
        // alias (e.g. both reference `lib/foo`); the visited-set
        // inside resolve_imports dedupes by canonical file path,
        // so the same import resolved twice is a no-op.
        let mut union_imports: Vec<hale_syntax::ast::Import> = Vec::new();
        for prog in programs.values() {
            for imp in &prog.imports {
                union_imports.push(imp.clone());
            }
        }
        let merged = match merge_programs(programs.values()) {
            Some(m) => m,
            None => {
                eprintln!("no .hl files in {}", target.display());
                return ExitCode::from(1);
            }
        };
        // Resolve the union of imports against the directory's
        // own dir as the importer dir + the workspace fallback.
        let workspace_root = find_workspace_root(target);
        let mut effects = EffectTable::from_seed(&merged);
    let mut merged_items = merged.items;
        // Identity-seeded: `merged`'s items are already merged.
        let mut renames: ImportRenames = Vec::new();
    let mut seed_cache: BTreeMap<PathBuf, std::collections::HashMap<String, String>> = BTreeMap::new();
        let mut path_sources: BTreeMap<PathBuf, String> =
            sources.into_iter().collect();
        let mut visited: std::collections::BTreeSet<PathBuf> =
            std::collections::BTreeSet::new();
        for f in &files {
            if let Ok(c) = f.canonicalize() {
                visited.insert(c);
            } else {
                visited.insert(f.clone());
            }
        }
        // GH #820: as on the entry path — the seed being run is not
        // one of the libraries its imports name.
        let mut claims: FileClaims = FileClaims::new();
        let mut import_errors: Vec<ImportDiag> = Vec::new();
        // GH #746: the directory is one seed; its aliases are scoped
        // to it.
        let target_scope =
            target.canonicalize().unwrap_or_else(|_| target.to_path_buf());
        let mut alias_scopes = AliasScopes::default();
        alias_scopes.record_files(
            &target_scope,
            files
                .iter()
                .map(|f| f.canonicalize().unwrap_or_else(|_| f.clone()))
                .collect(),
        );
        if resolve_imports(
            &union_imports,
            target,
            workspace_root.as_deref(),
            &mut visited,
            &mut claims,
            &mut path_sources,
            &mut dir_file_bases,
            &mut import_errors,
            &mut merged_items,
            &mut renames,
            &mut seed_cache,
            &mut effects,
            &target_scope,
            &mut alias_scopes,
        )
        .is_err()
            || !import_errors.is_empty()
        {
            return report_import_diags(&import_errors);
        }
        let mut with_imports = Program {
            declared_effects: effects.declared_indices(),
            effect_defs: effects.defs,
            effect_names: effects.names,
            imports: Vec::new(),
            items: merged_items,
            span: merged.span,
        };
        // GH #762: refuse a reference to an alias this seed never
        // declared, the same as `check` does.
        let unscoped = unscoped_alias_uses(
            &with_imports,
            &dir_file_bases,
            &path_sources,
            &alias_scopes,
            &seed_cache,
        );
        if !unscoped.is_empty() {
            for u in &unscoped {
                eprintln!(
                    "{}",
                    render_located(&u.diag, &dir_file_bases, &path_sources)
                );
            }
            return ExitCode::from(1);
        }
        // GH #746: scope a contested alias to its declaring seed.
        scope_import_aliases(
            &mut with_imports,
            &mut renames,
            &dir_file_bases,
            &alias_scopes,
            &seed_cache,
        );
        // brained F.1: rewrite qualified-path TypeExprs in the
        // entry program before typecheck (see parse_with_imports
        // for the rationale).
        hale_codegen::mangle::apply_qualified_path_renames(
            &mut with_imports,
            &renames,
        );
        // myapp/ → myapp; output lands next to target. When the
        // user passes `.` (or any path without a useful trailing
        // component — `./`, `..`), `Path::file_name` returns None;
        // canonicalize to recover the actual directory name so the
        // emitted binary is `<dir>/<dir>` instead of `<dir>/main`.
        let bin_name = target
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .or_else(|| {
                target.canonicalize().ok().and_then(|p| {
                    p.file_name().map(|s| s.to_string_lossy().into_owned())
                })
            })
            .unwrap_or_else(|| "main".to_string());
        let mut output = target.to_path_buf();
        output.push(&bin_name);
        let ctx = EntryCtx {
            entry_dir: target.to_path_buf(),
            workspace_root,
            imports: union_imports,
        };
        (with_imports, renames, path_sources, dir_file_bases, output, ctx)
    } else {
        eprintln!("not a file or directory: {}", target.display());
        return ExitCode::from(1);
    };

    // FUv0.8.2 #4 (2026-05-25): auto-apply sync inference
    // BEFORE typecheck. Walks the program, runs F.32-1∞ on
    // `@form(hashmap)` loci without explicit `sync = `, and
    // injects the picked discipline as a synthetic FormArg.
    // The subsequent typecheck sees an explicit sync and the
    // F.32-0 cross-pool diagnostic stays quiet for auto-
    // inferable cases. Loci with existing sync kwarg or
    // single-pool use are left alone.

    // `--wrap-main` (browser playground): synthesize the wasm `@export`
    // entry from a bare `fn main` on the AST, BEFORE typecheck — so the
    // checker sees the synthesized `target wasm` gate + `@export` locus,
    // and every diagnostic keeps the user's original line/col (no textual
    // wrap, no offset). Wasm-only: there is no native entry inversion to
    // wrap, so on a native build it is a hard error rather than a silent
    // no-op (which would mask a misconfigured playground build).
    if std::env::args().any(|a| a == "--wrap-main") {
        let args: Vec<String> = std::env::args().collect();
        let target_wasm = args.windows(2).any(|w| {
            w[0] == "--target" && (w[1] == "wasm32" || w[1] == "wasm")
        });
        if !target_wasm {
            eprintln!(
                "error: --wrap-main requires --target wasm32 — it \
                 synthesizes the wasm @export entry from `fn main`, and \
                 there is no native entry-inversion to wrap"
            );
            return ExitCode::from(2);
        }
        hale_syntax::desugar::wrap_main_as_wasm_export(&mut program);
    }

    hale_syntax::json_gen::generate_json_parsers(&mut program);
    // Options first: the check answers target questions (GH #970), so
    // it has to know the target, and `--api` (GH #1106) shapes the
    // program before the bundle borrows it.
    let mut options = match parse_build_options("build", flags) {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("{}", msg);
            return ExitCode::from(2);
        }
    };
    // GH #1109: `--env` — the deployment target's role table and law.
    let env_spec = match resolve_build_env(target, &mut options) {
        Ok(e) => e,
        Err(msg) => {
            eprintln!("{}", msg);
            return ExitCode::from(2);
        }
    };
    if let Some(path) = &options.api {
        if let Err(msg) = hale_syntax::api_gen::inject_api_entry(&mut program, path) {
            eprintln!("{}", msg);
            return ExitCode::from(2);
        }
    }
    if let Err(msg) = bind_build_env(&mut program, &env_spec, &options) {
        eprintln!("{}", msg);
        return ExitCode::from(2);
    }
    // Pre-pass diags are re-raised by `check_bundle_opts` below
    // through the normal rendering — bailing here double-reported
    // (see the `check` site for the full story).
    let _ = hale_types::apply_sync_inference(&mut program);

    // Typecheck before lowering. Render diagnostics against the
    // entry-file's source — diagnostic spans currently point into
    // the merged item stream which doesn't have a single source
    // string; this is good enough for v0.
    let mut bundle_programs: BTreeMap<String, &Program> = BTreeMap::new();
    bundle_programs.insert(target.display().to_string(), &program);
    // The rename table must reach the analysis here too, not only in
    // `check`. Without it a cross-seed call is an unresolved edge, so
    // an effect assertion violated one seed away compiles, links and
    // ships — a downstream fleet gates on `build` across 109 binaries,
    // and "it built" must not be weaker than "it checked" on a
    // contract the compiler already knows how to evaluate.
    let mut bundle = hale_types::Bundle::new(bundle_programs);
    bundle.import_renames = renames.clone();
    bundle.target_has_async_io = options.target.spec().has_async_io();
    bundle.target_label = options.target.spec().platform_label();
    let allow_unowned =
        std::env::args().any(|a| a == "--allow-unowned-subscriber");
    let diags = hale_types::check_bundle_for_build(&bundle, allow_unowned);
    if !diags.is_empty() {
        for d in &diags {
            eprintln!("{}", render_located(d, &file_bases, &sources));
        }
        // Warnings print but don't fail the build; only errors do.
        if diags.iter().any(|d| d.is_error()) {
            return ExitCode::from(1);
        }
    }
    // P26: stamp the model identity of the bundle just checked into
    // the binary, for the observation segment header.
    options.model_hash =
        Some(hale_types::topology::model_shape_hash(&bundle));
    // GH #476 Change 8: the canonical entity ids a consumer joins
    // the live manifest to that model with, and the dispatch
    // plan's digest — held here and folded into the execution
    // identity once the options are FINAL (below), since the
    // fingerprint covers options that are still being set.
    let (plan_digest, obs_ids) = model_identity(&bundle, &options);
    options.obs_entity_ids = obs_ids;
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
    let any_locality_annotation = program.items.iter().any(|item| {
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
        let map =
            hale_types::working_set::compute_program_working_set(
                &program.items,
            );
        if want_report {
            eprint!(
                "{}",
                hale_types::working_set::render_locality_report(&map)
            );
        }
        let breaches =
            hale_types::working_set::breaches_with_per_locus_budgets(
                &map,
                &program.items,
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
    // escape hatch); toml-sourced flags append. Duplicates are
    // tolerated — clang's `-lX -lX` is harmless, and the linker
    // dedupes csrc translation-unit contents at symbol level.
    let toml_opts = collect_ffi_from_imports(
        &entry_ctx.imports,
        &entry_ctx.entry_dir,
        entry_ctx.workspace_root.as_deref(),
    );
    options.link_libs.extend(toml_opts.link_libs);
    options.csrc_files.extend(toml_opts.csrc_files);
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
    options.exec_digest = Some(exec_digest(
        &sources,
        target,
        &build_env::options_fingerprint(&options),
        plan_digest,
    ));
    match hale_codegen::build_executable_with_options(
        &program,
        &output,
        &renames,
        &options,
    ) {
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
                render_codegen_error(&e, &file_bases, &sources)
            );
            ExitCode::from(1)
        }
    }
}
