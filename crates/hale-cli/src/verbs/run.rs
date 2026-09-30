use crate::shared::imports::AliasScopes;
use std::collections::BTreeMap;
use crate::EffectTable;
use std::process::ExitCode;
use crate::shared::imports::FileClaims;
use crate::shared::imports::ImportDiag;
use crate::shared::imports::ImportRenames;
use std::path::Path;
use std::path::PathBuf;
use hale_syntax::ast::Program;
use crate::shared::process::RunScratch;
use crate::shared::options::bind_build_env;
use crate::build_env;
use crate::shared::frontend::collect_ap_files;
use crate::shared::process::dies_with_us;
use crate::shared::options::exec_digest;
use crate::shared::workspace::find_workspace_root;
use crate::shared::frontend::merge_programs;
use crate::shared::options::model_identity;
use crate::shared::frontend::parse_files;
use crate::shared::frontend::parse_with_imports;
use crate::shared::diag::render_codegen_error;
use crate::shared::diag::render_located;
use crate::shared::diag::report_import_diags;
use crate::shared::options::resolve_build_env;
use crate::shared::imports::resolve_imports;
use crate::shared::imports::scope_import_aliases;
use crate::shared::imports::unscoped_alias_uses;
use crate::shared::process::wait_passing_signals;
/// Compile `program` to a temporary native binary and execute it,
/// forwarding `user_args` as the program's trailing argv. This is
/// the whole of `hale run` — the same codegen backend as `hale
/// build`, so there is no `run`-vs-`build` behavioral divergence.
pub(crate) fn compile_and_exec(
    program: &Program,
    renames: &[(Vec<String>, String)],
    user_args: &[String],
    // `LOTUS_OBS=1` on the child: `hale run --observe` (GH #527 B3).
    observe: bool,
    model_hash: u64,
    exec_digest: [u64; 4],
    obs_entity_ids: Vec<hale_model::obs_ids::ObsEntityId>,
    // GH #848: the file table and texts the bundle was parsed with,
    // so a span-carrying codegen error is reported at its source
    // location exactly as `hale build` reports it.
    file_bases: &[(u32, PathBuf, u32)],
    sources: &BTreeMap<PathBuf, String>,
    // GH #904: the options `hale run` was given, already
    // fingerprinted into `exec_digest` by the caller. They were
    // `BuildOptions::default()` here whatever the command line
    // said.
    options: hale_codegen::BuildOptions,
) -> ExitCode {
    let scratch = match RunScratch::new("run") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hale run: {e}");
            return ExitCode::from(1);
        }
    };
    let bin = scratch.path("program");
    let options = hale_codegen::BuildOptions {
        model_hash: Some(model_hash),
        exec_digest: Some(exec_digest),
        obs_entity_ids,
        ..options
    };
    if let Err(e) = hale_codegen::build_executable_with_options(
        program, &bin, renames, &options,
    ) {
        eprintln!("{}", render_codegen_error(&e, file_bases, sources));
        return ExitCode::from(1);
    }
    let mut cmd = std::process::Command::new(&bin);
    cmd.args(user_args);
    if observe {
        cmd.env("LOTUS_OBS", "1");
    }
    // The program is `hale run`'s foreground work, not a daemon it
    // launches: it holds this command's stdin/stdout/stderr, and
    // `hale` exists to wait for it and report how it ended. A `hale`
    // killed out from under it leaves it running against a caller
    // that cannot see its output end — GH #905.
    dies_with_us(&mut cmd);
    let status = wait_passing_signals(&mut cmd);
    match status {
        Ok(s) => {
            // GH #577: a program killed by a signal says so — a segfault
            // that printed nothing used to look like `exit 1`
            use std::os::unix::process::ExitStatusExt;
            if let Some(sig) = s.signal() {
                let name = match sig {
                    libc::SIGSEGV => "SIGSEGV",
                    libc::SIGABRT => "SIGABRT",
                    libc::SIGBUS => "SIGBUS",
                    libc::SIGFPE => "SIGFPE",
                    libc::SIGILL => "SIGILL",
                    libc::SIGKILL => "SIGKILL",
                    libc::SIGTERM => "SIGTERM",
                    _ => "signal",
                };
                eprintln!("hale run: the program was killed by {name} (signal {sig})");
                return ExitCode::from((128 + sig).clamp(0, 255) as u8);
            }
            ExitCode::from(s.code().unwrap_or(1).clamp(0, 255) as u8)
        }
        Err(e) => {
            eprintln!("could not execute compiled program: {}", e);
            ExitCode::from(1)
        }
    }
}

pub(crate) fn run_program(
    target: &Path,
    user_args: &[String],
    // GH #904: the build options this `run` was given — the same
    // set, from the same parser, that `hale build` takes. They are
    // fingerprinted into the execution identity below, so a
    // recording carries the options it was made under.
    mut options: hale_codegen::BuildOptions,
    // GH #527 B3 / GH #887: `--observe` publishes the program's
    // observation segment. It is the CHILD's setting, so it rides
    // down to the `Command` that starts the child rather than being
    // planted in this process's environment for it to inherit.
    observe: bool,
) -> ExitCode {
    // GH #1109: `--env` names the deployment target; its role table is
    // part of the binary (and so of the fingerprint below), so it is
    // resolved first. The constitution it binds is adopted once the
    // program is parsed.
    let env_spec = match resolve_build_env(target, &mut options) {
        Ok(e) => e,
        Err(msg) => {
            eprintln!("{}", msg);
            return ExitCode::from(2);
        }
    };
    // Both single-file and directory targets resolve cross-seed
    // imports and thread the per-build path-rename table into
    // codegen — `run` and `build` agree (WS3.3). A single file
    // follows `import "..."` from its own directory; a directory
    // bundles its `.hl` files as one seed and resolves the union
    // of their imports (see the directory branch below).
    if target.is_file() {
        // `compile_and_exec` passes `renames` to
        // `build_executable_with_imports`, so qualified
        // `alias::Name` references in the entry file resolve the
        // same way `hale build` resolves them.
        let (mut program, renames, sources, file_bases, _ctx) = match parse_with_imports(target) {
            Ok(x) => x,
            Err(errors) => return report_import_diags(&errors),
        };
        // F.40 phase 1.1b-iii: the snapshot. The file entry runs no
        // desugar before the check, so it mints straight after the
        // load; with no source map here, the seed is the program's
        // ordinal.
        let target_name = target.display().to_string();
        let snapshot = hale_types::snapshot::mint([(target_name.as_str(), &mut program)], &[]);
        let mut bundle_programs: BTreeMap<String, &Program> = BTreeMap::new();
        bundle_programs.insert(target_name.clone(), &program);
        // The rename table must reach the analysis here too, not only in
        // `check`. Without it a cross-seed call is an unresolved edge, so
        // an effect assertion violated one seed away compiles, links and
        // ships — a downstream fleet gates on `build` across 109 binaries,
        // and "it built" must not be weaker than "it checked" on a
        // contract the compiler already knows how to evaluate.
        let mut bundle = hale_types::Bundle::new(bundle_programs);
        bundle.import_renames = renames.clone();
        bundle.snapshot = snapshot;
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
        // P26: stamp the model identity of the bundle just checked.
        let model_hash =
            hale_types::topology::model_shape_hash(&bundle);
        let options_fp = build_env::options_fingerprint(&options);
        let (plan_digest, obs_ids) = model_identity(&bundle, &options);
        let digest =
            exec_digest(&sources, target, &options_fp, plan_digest);
        return compile_and_exec(
            &program,
            &renames,
            user_args,
            observe,
            model_hash,
            digest,
            obs_ids,
            &file_bases,
            &sources,
            options,
        );
    }

    let files = match collect_ap_files(target) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::from(1);
        }
    };
    let (programs, sources, mut file_bases) = match parse_files(&files) {
        Ok(x) => x,
        // `run` has no machine-readable channel: the same located
        // text it always printed (GH #777 moved the printing here).
        Err(f) => return f.report_text(),
    };

    // WS3.3 (2026-06-11): a directory `hale run` now resolves
    // cross-seed imports the same way `hale build <dir>` does.
    // Previously it bundled the directory's files but silently
    // dropped every `import "..."`, so a dir-seed app importing a
    // vendored library failed on `alias::Name` references — the
    // exact pond / downstream apps "qualified type not in path-renames table"
    // friction, and the reason a topic decl had to live in the same
    // file as its publisher. `run` and `build` now produce the same
    // merged-and-resolved program for a directory; `run` execs it
    // instead of writing a binary.
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
    let workspace_root = find_workspace_root(target);
    let mut effects = EffectTable::from_seed(&merged);
    let mut merged_items = merged.items;
    // Same identity-seeding rule as the entry path: `merged`'s own
    // items are already in `merged_items` and are never walked, so its
    // table must come first.
    let mut renames: ImportRenames = Vec::new();
    let mut seed_cache: BTreeMap<PathBuf, std::collections::HashMap<String, String>> = BTreeMap::new();
    let mut path_sources: BTreeMap<PathBuf, String> = sources.into_iter().collect();
    let mut visited: std::collections::BTreeSet<PathBuf> =
        std::collections::BTreeSet::new();
    for f in &files {
        match f.canonicalize() {
            Ok(c) => visited.insert(c),
            Err(_) => visited.insert(f.clone()),
        };
    }
    // GH #820: as on the entry path — the seed being built is not one
    // of the libraries its imports name.
    let mut claims: FileClaims = FileClaims::new();
    let mut import_errors: Vec<ImportDiag> = Vec::new();
    // GH #746: the directory is one seed; its aliases are scoped to it.
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
        &mut file_bases,
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
    let mut program = Program {
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
        &program,
        &file_bases,
        &path_sources,
        &alias_scopes,
        &seed_cache,
    );
    if !unscoped.is_empty() {
        for u in &unscoped {
            eprintln!("{}", render_located(&u.diag, &file_bases, &path_sources));
        }
        return ExitCode::from(1);
    }
    // Rewrite qualified-path TypeExprs + synthesize JSON parsers +
    // apply sync inference before typecheck — the same pre-passes
    // `hale build <dir>` runs, so a directory `run` and `build`
    // agree.
    scope_import_aliases(
        &mut program,
        &mut renames,
        &file_bases,
        &alias_scopes,
        &seed_cache,
    );
    hale_codegen::mangle::apply_qualified_path_renames(&mut program, &renames);
    hale_syntax::json_gen::generate_json_parsers(&mut program);
    // GH #1106: `--api` injects the entry; the pass lowers it (and any
    // entry the source spelled) before the checker sees the program.
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
    // F.40 phase 1.1b-iii: the snapshot, after the last desugar. This
    // path builds no source map, so the seed is the program's ordinal.
    let target_name = target.display().to_string();
    let snapshot = hale_types::snapshot::mint([(target_name.as_str(), &mut program)], &[]);

    let bundle_programs: BTreeMap<String, &Program> =
        std::iter::once((target_name.clone(), &program)).collect();
    // The rename table must reach the analysis here too, not only in
    // `check`. Without it a cross-seed call is an unresolved edge, so
    // an effect assertion violated one seed away compiles, links and
    // ships — a downstream fleet gates on `build` across 109 binaries,
    // and "it built" must not be weaker than "it checked" on a
    // contract the compiler already knows how to evaluate.
    let mut bundle = hale_types::Bundle::new(bundle_programs);
    bundle.import_renames = renames.clone();
    bundle.snapshot = snapshot;
    let allow_unowned =
        std::env::args().any(|a| a == "--allow-unowned-subscriber");
    let diags = hale_types::check_bundle_for_build(&bundle, allow_unowned);
    if !diags.is_empty() {
        for d in &diags {
            eprintln!("{}", render_located(d, &file_bases, &path_sources));
        }
        // Warnings print but don't fail the build; only errors do.
        if diags.iter().any(|d| d.is_error()) {
            return ExitCode::from(1);
        }
    }
    // P26: stamp the model identity of the bundle just checked.
    let model_hash = hale_types::topology::model_shape_hash(&bundle);
    let options_fp = build_env::options_fingerprint(&options);
    let (plan_digest, obs_ids) = model_identity(&bundle, &options);
    let digest =
        exec_digest(&path_sources, target, &options_fp, plan_digest);
    compile_and_exec(
        &program,
        &renames,
        user_args,
        observe,
        model_hash,
        digest,
        obs_ids,
        &file_bases,
        &path_sources,
        options,
    )
}
