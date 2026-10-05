use std::collections::BTreeMap;
use std::process::ExitCode;
use std::path::Path;
use std::path::PathBuf;
use crate::shared::process::RunScratch;
use crate::build_env;
use crate::shared::frontend::LoadMode;
use crate::shared::source::Disk;
use crate::shared::process::dies_with_us;
use crate::shared::options::build_config;
use crate::shared::options::exec_digest;
use crate::shared::options::identity_options;
use crate::shared::options::model_identity;
use crate::shared::options::note_unmapped_roles;
use crate::shared::diag::render_blocked;
use crate::shared::diag::render_codegen_error;
use crate::shared::diag::render_located;
use crate::shared::options::refuse_unexecutable;
use crate::shared::options::resolve_build_env;
use crate::shared::process::wait_passing_signals;
use hale_frontend::snapshot::{LoadError, Snapshot};

/// Compile the lowering view to a temporary native binary and
/// execute it, forwarding `user_args` as the program's trailing argv.
/// This is the whole of `hale run` — the same codegen backend as
/// `hale build`, so there is no `run`-vs-`build` behavioral divergence.
pub(crate) fn compile_and_exec(
    resolved: &hale_types::resolved::LoweringView,
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
    if let Err(e) = hale_codegen::build_resolved(resolved, &bin, &options) {
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
    // resolved first. The constitution it binds is adopted by the
    // snapshot's load.
    let env_spec = match resolve_build_env(target, &mut options) {
        Ok(e) => e,
        Err(msg) => {
            eprintln!("{}", msg);
            return ExitCode::from(2);
        }
    };
    // F.40 phase 2.2b: one snapshot, the one `hale build` loads, and
    // the lowering view demanded from it. A file follows `import
    // "..."` from its own directory; a directory bundles its `.hl`
    // files as one seed and resolves the union of their imports (WS3.3:
    // `run` and `build` produce the same merged-and-resolved program;
    // `run` execs it instead of writing a binary).
    let config = build_config(&options, &env_spec, false);
    let snap = match Snapshot::load(target, LoadMode::WholeSeed, &Disk, config) {
        Ok(s) => s,
        // `run` has no machine-readable channel: the located text.
        Err(LoadError::Load(f)) => {
            eprintln!("{}", f.text());
            return ExitCode::from(f.code);
        }
        Err(LoadError::Refused(msg)) => {
            eprintln!("{}", msg);
            return ExitCode::from(2);
        }
    };
    if let Some(msg) = refuse_unexecutable("run", &snap) {
        eprintln!("{}", msg);
        return ExitCode::from(2);
    }
    note_unmapped_roles(snap.api_surface(), &options);
    let (sources, file_bases) = (snap.sources(), snap.file_bases());
    // The check, with the build's rules: the rename table reaches the
    // analysis through the snapshot's bundle, so "it ran" is not
    // weaker than "it checked" on a contract one seed away.
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
    // The view before the execution identity: the identity folds in
    // the dispatch plan the view carries. A refused resolve is
    // reported as the build would report it. The model identity (P26)
    // and the entity ids are the snapshot's model's.
    let view = match snap.demand_lowering() {
        Ok(v) => v,
        Err(b) => {
            eprintln!("{}", render_blocked(b, file_bases, sources));
            return ExitCode::from(1);
        }
    };
    // The identity's options are `build`'s and `replay`'s (I2): the
    // `[ffi]` surface of the imported packages among them. `run` builds
    // with its flags alone.
    let options_fp = build_env::options_fingerprint(&identity_options(&options, &snap, target));
    let identity = match model_identity(&snap, view, &options) {
        Ok(x) => x,
        Err(b) => {
            eprintln!("{}", render_blocked(b, file_bases, sources));
            return ExitCode::from(1);
        }
    };
    let digest = exec_digest(sources, target, &options_fp, identity.plan_digest);
    compile_and_exec(
        view,
        user_args,
        observe,
        identity.model_hash,
        digest,
        identity.obs_ids,
        file_bases,
        sources,
        options,
    )
}
