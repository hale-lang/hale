use std::collections::BTreeMap;
use std::process::ExitCode;
use std::path::PathBuf;
use hale_syntax::ast::Program;
use crate::shared::process::RunScratch;
use crate::shared::options::VALUE_FLAGS;
use crate::build_env;
use crate::shared::options::exec_digest;
use crate::shared::options::model_identity;
use crate::shared::options::parse_exec_build_options;
use crate::shared::frontend::parse_with_imports;
use crate::shared::source::Disk;
use crate::shared::diag::render_codegen_error;
use crate::shared::diag::render_located;
use crate::replay;
use crate::shared::diag::report_import_diags;
/// `hale replay <recording> <program.hl> [--diff [--json]] [--at N]`
/// — GH #296. Re-runs a recorded execution: the same binary (model
/// identity checked against the recording header), with the
/// runtime serving journaled inputs (time/entropy/env) and
/// enforcing each consumer's recorded delivery order. `--diff`
/// records the replay and reports the first divergence, or — on a
/// match — which categories it compared (GH #728), with `--json`
/// carrying the same verdict and counts machine-readably. `--at N`
/// stops the program (SIGSTOP) at the Nth consume so a debugger
/// can attach.
pub(crate) fn run_replay(args: &[String]) -> ExitCode {
    let mut rec_arg: Option<PathBuf> = None;
    let mut prog: Option<PathBuf> = None;
    let mut diff = false;
    let mut json = false;
    let mut at: Option<String> = None;
    let mut allow_live_effects = false;
    let mut allow_unverified = false;
    let mut allow_truncated = false;
    let mut feed = false;
    let mut allow_unmatched_feed = false;
    // GH #904: the `hale build` options, in `replay`'s own hand —
    // a recording made under `hale run --dev` is admitted by `hale
    // replay --dev` and by nothing else, so the flag set has to be
    // reachable here too.
    let mut build_flags: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--diff" {
            diff = true;
            i += 1;
        } else if a == "--json" {
            // GH #728: the `--diff` verdict, machine-readable —
            // per-category coverage, not just a match/diverge bit.
            json = true;
            i += 1;
        } else if a == "--allow-live-effects" {
            allow_live_effects = true;
            i += 1;
        } else if a == "--allow-unverified-model" {
            allow_unverified = true;
            i += 1;
        } else if a == "--allow-truncated" {
            allow_truncated = true;
            i += 1;
        } else if a == "--feed" {
            feed = true;
            i += 1;
        } else if a == "--allow-unmatched-feed" {
            allow_unmatched_feed = true;
            i += 1;
        } else if a == "--at" {
            // N (process-wide consume ordinal — only meaningful
            // for a single consumer) or consumer:N (stable across
            // multi-consumer runs).
            match args.get(i + 1) {
                Some(v)
                    if v.parse::<u64>().map(|n| n > 0).unwrap_or(false)
                        || v.split_once(':').is_some_and(|(c, n)| {
                            c.parse::<u64>().is_ok()
                                && n.parse::<u64>()
                                    .map(|n| n > 0)
                                    .unwrap_or(false)
                        }) =>
                {
                    at = Some(v.clone());
                    i += 2;
                }
                _ => {
                    eprintln!(
                        "hale replay: --at takes N or consumer:N (positive)"
                    );
                    return ExitCode::from(2);
                }
            }
        } else if VALUE_FLAGS.contains(&a) {
            // A build option whose value is the next argv entry.
            build_flags.push(a.to_string());
            if let Some(v) = args.get(i + 1) {
                build_flags.push(v.clone());
            }
            i += 2;
        } else if a == "--dev" {
            build_flags.push(a.to_string());
            i += 1;
        } else if a.starts_with('-') {
            eprintln!("hale replay: unknown flag `{}`", a);
            return ExitCode::from(2);
        } else if rec_arg.is_none() {
            rec_arg = Some(PathBuf::from(a));
            i += 1;
        } else if prog.is_none() {
            prog = Some(PathBuf::from(a));
            i += 1;
        } else {
            eprintln!("hale replay: unexpected extra argument `{}`", a);
            return ExitCode::from(2);
        }
    }
    if feed && (diff || at.is_some()) {
        eprintln!(
            "hale replay: --feed re-executes changed code against \
             the recorded ingress tape — there is no recorded \
             schedule to --diff against or --at-stop on"
        );
        return ExitCode::from(2);
    }
    if json && !diff {
        eprintln!(
            "hale replay: --json reports the --diff comparison \
             verdict; pass --diff"
        );
        return ExitCode::from(2);
    }
    // GH #904: one `BuildOptions`, from `hale build`'s parser, for
    // the fingerprint AND the compile below — a replay recompiles
    // the program, so it admits against what IT builds.
    let build_options =
        match parse_exec_build_options("replay", &build_flags) {
            Ok(o) => o,
            Err(msg) => {
                eprintln!("{}", msg);
                return ExitCode::from(2);
            }
        };
    let (rec_path, prog) = match (rec_arg, prog) {
        (Some(r), Some(p)) => (r, p),
        _ => {
            eprintln!(
                "usage: hale replay <recording> <program.hl> \
                 [--diff [--json]] \
                 [--at <n> | --at <consumer-id>:<ordinal>] \
                 [--allow-live-effects] [--allow-unverified-model] \
                 [--allow-truncated] [--feed] [--allow-unmatched-feed] \
                 [--dev] [--target-cpu <v>] [--link <lib>] [--csrc <f.c>]"
            );
            return ExitCode::from(2);
        }
    };

    // ONE file object for admission AND execution (review round 2,
    // finding 1): open O_NOFOLLOW, parse this object, and later dup
    // THIS descriptor into the child — never reopen the pathname.
    let rec_file = {
        use std::os::unix::fs::OpenOptionsExt;
        match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&rec_path)
        {
            Ok(f) => f,
            Err(e) => {
                eprintln!(
                    "hale replay: could not open `{}`: {}",
                    rec_path.display(),
                    e
                );
                return ExitCode::from(1);
            }
        }
    };
    let rec = match replay::parse_file(&rec_file, &rec_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("hale replay: {}", e);
            return ExitCode::from(1);
        }
    };
    if !rec.clean {
        if !allow_truncated {
            eprintln!(
                "hale replay: `{}` has no clean-finalize trailer — \
                 the recording is crash-truncated; re-record, or \
                 pass --allow-truncated to replay the recorded \
                 prefix",
                rec_path.display()
            );
            return ExitCode::from(1);
        }
        eprintln!(
            "hale replay: `{}` has no clean finalize — replaying \
             the recorded prefix (--allow-truncated)",
            rec_path.display()
        );
    }

    if !prog.is_file() {
        eprintln!(
            "hale replay: `{}` is not a file (directory seeds are \
             not replayable yet)",
            prog.display()
        );
        return ExitCode::from(1);
    }
    // Same compile pipeline as `hale run` (parse → check → model
    // hash), so a recording is admitted against exactly what runs.
    let (mut program, renames, sources, file_bases, _ctx) =
        match parse_with_imports(&prog, &Disk) {
            Ok(x) => x,
            Err(errors) => return report_import_diags(&errors),
        };
    // F.40 phase 2.1b: the declaration-shaping sequence, the one every
    // entry point runs before its check.
    hale_types::desugar_sequence::desugar_before_check(
        &mut [&mut program],
        &hale_types::desugar_sequence::Sequence { import_renames: &renames },
    );
    // F.40 phase 1.1b-iii: the snapshot, after the sequence, seeded by
    // the source map `check` mints with.
    let prog_name = prog.display().to_string();
    let source_map = crate::shared::frontend::source_map(&prog, &file_bases, &sources);
    let snapshot = hale_types::snapshot::mint([(prog_name.as_str(), &mut program)], &source_map);
    let mut bundle_programs: BTreeMap<String, &Program> = BTreeMap::new();
    bundle_programs.insert(prog_name.clone(), &program);
    let mut bundle = hale_types::Bundle::new(bundle_programs);
    bundle.import_renames = renames.clone();
    // The map the snapshot minted with, as `check` hands it over.
    bundle.sources = source_map.clone();
    bundle.snapshot = snapshot;
    let diags = hale_types::check_bundle_for_build(&bundle, false);
    if !diags.is_empty() {
        for d in &diags {
            eprintln!("{}", render_located(d, &file_bases, &sources));
        }
        if diags.iter().any(|d| d.is_error()) {
            return ExitCode::from(1);
        }
    }
    let model_hash = hale_types::topology::model_shape_hash(&bundle);
    // Resolved before the identity: the dispatch plan the digest frames
    // is the resolved program's, the one codegen lowers below (F.40
    // phase 1.5).
    let resolved = match hale_types::resolved::resolve_program(
        &program,
        &source_map,
        &renames,
        build_options.api.as_deref(),
        build_options.api_roles.as_deref(),
    ) {
        Ok(r) => r,
        Err(e) => {
            let e = hale_codegen::CodegenError::Unsupported(e);
            eprintln!("{}", render_codegen_error(&e, &file_bases, &sources));
            return ExitCode::from(1);
        }
    };
    let options_fp = build_env::options_fingerprint(&build_options);
    let (plan_digest, obs_ids) = model_identity(&bundle, &resolved, &build_options);
    let digest = exec_digest(&sources, &prog, &options_fp, plan_digest);

    // GH #296 phase 5b (review round): a binding backend with no
    // replay class cannot be suppressed OR injected — replaying or
    // feeding a program that carries one would touch live shared
    // memory. Fail closed with the backend named; no flag overrides
    // this (the runtime independently refuses at open). Applies to
    // strict replay and feed alike.
    {
        fn scan_shm_ring(items: &[hale_syntax::ast::TopDecl]) -> bool {
            items.iter().any(|item| match item {
                hale_syntax::ast::TopDecl::Locus(l) => {
                    l.members.iter().any(|m| match m {
                        hale_syntax::ast::LocusMember::Bindings(b) => {
                            b.entries.iter().any(|e| {
                                matches!(
                                    e.transport,
                                    hale_syntax::ast::TransportSpec::ShmRing { .. }
                                )
                            })
                        }
                        _ => false,
                    })
                }
                hale_syntax::ast::TopDecl::Module(md) => {
                    scan_shm_ring(&md.items)
                }
                _ => false,
            })
        }
        if bundle.programs.values().any(|p| scan_shm_ring(&p.items)) {
            eprintln!(
                "hale replay: this program binds a `shm_ring` — that \
                 backend has no replay class yet (it cannot be \
                 suppressed or injected, and a replayed/fed process \
                 must not touch live shared memory). Remove the \
                 binding or re-record without it (GH #296)"
            );
            return ExitCode::from(1);
        }
    }

    // Ordered BEFORE identity admission: this refusal is inherent
    // to the PROGRAM, so it must not be masked by a
    // recording-mismatch message (and module-gate canaries can
    // assert it with any recording).
    // Safe by default (review finding 3, both rounds): re-execution
    // repeats the program's real side effects. The gate consumes
    // the TYPED inferred rows, and it must fail CLOSED on the two
    // paths the first version failed open on: `unclassified`
    // ("may do anything") and a `publish` whose subject is bound to
    // an external transport (the send is generated deployment
    // machinery, invisible in user-level effects). Coarse by class
    // granularity — over-refusing is the safe direction;
    // per-primitive replay classes are the staged refinement.
    // phase 5b review round: --feed bypasses IDENTITY admission
    // (changed code is its point), never EFFECT safety — feeding a
    // tape is not an opt-in to live syscalls/FFI. Both flags
    // together are the explicit backtest-with-live-effects spelling.
    if !allow_live_effects {
        let programs: Vec<&Program> =
            bundle.programs.values().copied().collect();
        let rows =
            hale_types::effects::effect_manifest_with_inference(&programs);
        let mut residue = std::collections::BTreeSet::new();
        for row in &rows {
            for class in &row.inferred {
                if class == "syscall"
                    || class == "ffi"
                    || class == "unclassified"
                {
                    residue.insert(class.clone());
                }
            }
        }
        // External transport bindings: any `bindings { }` block
        // makes publishes (and listener startup) touch the real
        // world during re-execution.
        // Modules nest top declarations arbitrarily deep (round 3,
        // finding 2) — walk them, or a module-contained binding
        // fails open.
        fn scan_bindings(items: &[hale_syntax::ast::TopDecl]) -> bool {
            items.iter().any(|item| match item {
                hale_syntax::ast::TopDecl::Locus(l) => {
                    l.members.iter().any(|m| {
                        matches!(
                            m,
                            hale_syntax::ast::LocusMember::Bindings(_)
                        )
                    })
                }
                hale_syntax::ast::TopDecl::Module(md) => {
                    scan_bindings(&md.items)
                }
                _ => false,
            })
        }
        // phase 5b: native unix/udp `bindings { }` no longer force
        // the flag — the runtime suppresses those transports under
        // replay (opens nothing, sends nothing) and injects the
        // recorded ingress tape in the listeners' stead. Hermeticity
        // is a BINDING-KIND capability, not a blanket assumption
        // (review round): a backend with no replay class fails
        // CLOSED below. The residue that remains here is genuinely
        // user-level: syscall/ffi writes repeat, unclassified may do
        // anything.
        let _ = scan_bindings;
        if !residue.is_empty() {
            eprintln!(
                "hale replay: this program can reach the live world \
                 during re-execution ({{{}}}) — unclassified calls \
                 may do anything, and syscall/ffi writes repeat. \
                 Refusing by default; pass --allow-live-effects if \
                 you accept that",
                residue.into_iter().collect::<Vec<_>>().join(", ")
            );
            return ExitCode::from(1);
        }
    }

    // Admission, strongest check first (review finding 2):
    // exec_digest is framed-SHA-256 build-input identity;
    // shape_hash alone is only structural compatibility and admits
    // behaviorally different bodies.
    //
    // phase 5b, feed mode: admission is DELIBERATELY not required —
    // feeding a tape to changed code is the entire point. Say what
    // is being fed to what, and skip the checks.
    if feed {
        if rec.model_hash != model_hash {
            eprintln!(
                "hale replay: feed — recording is from model \
                 {:016x}, this program is {:016x} (admission not \
                 required in feed mode; subjects matched by name)",
                rec.model_hash, model_hash
            );
        }
    }
    if !feed && rec.exec_digest != [0; 4] && rec.exec_digest != digest {
        eprintln!(
            "hale replay: `{}` was recorded from different build \
             inputs (recorded exec digest {:016x}…, this compile \
             is {:016x}…) — a structurally compatible model is not \
             the same executable; re-record, or accept behavioral \
             divergence explicitly with --allow-unverified-model",
            rec_path.display(),
            rec.exec_digest[0],
            digest[0]
        );
        if !allow_unverified {
            return ExitCode::from(1);
        }
    }
    if !feed && rec.model_hash != 0 && rec.model_hash != model_hash {
        eprintln!(
            "hale replay: `{}` was recorded from a different model \
             (recorded shape_hash {:016x}, this program is {:016x}) \
             — refusing to misreplay; re-record against this \
             program",
            rec_path.display(),
            rec.model_hash,
            model_hash
        );
        return ExitCode::from(1);
    }
    if !feed
        && (rec.exec_digest == [0; 4] || rec.model_hash == 0)
        && !allow_unverified
    {
        eprintln!(
            "hale replay: `{}` carries no execution identity \
             (unstamped build) — exact replay cannot be admitted. \
             Pass --allow-unverified-model to proceed anyway",
            rec_path.display()
        );
        return ExitCode::from(1);
    }

    if rec.env_redacted && !feed {
        // (feed does not serve the env journal at all — review
        // round 2, finding 7: this warning is replay-only.)
        eprintln!(
            "hale replay: this recording withholds env VALUES \
             (default policy; record with LOTUS_OBS_RECORD_ENV=full \
             to include them) — env reads will replay as named \
             withheld divergences"
        );
    } else if rec.env_redacted && feed {
        eprintln!(
            "hale replay: feed note — the tape withheld env values; \
             feed does not replay env reads, so the current process \
             environment is used"
        );
    }
    let rec_abs = rec_path
        .canonicalize()
        .unwrap_or_else(|_| rec_path.clone());

    let scratch = match RunScratch::new("replay") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hale replay: {e}");
            return ExitCode::from(1);
        }
    };
    let bin = scratch.path("program");
    let options = hale_codegen::BuildOptions {
        model_hash: Some(model_hash),
        exec_digest: Some(digest),
        obs_entity_ids: obs_ids.clone(),
        ..build_options
    };
    if let Err(e) = hale_codegen::build_resolved(resolved, &bin, &options) {
        eprintln!("{}", render_codegen_error(&e, &file_bases, &sources));
        return ExitCode::from(1);
    }

    let verify_path = if diff {
        Some(scratch.path("verify.halerec"))
    } else {
        None
    };

    let status_path = scratch.path("status");
    // Pre-create 0600 so the child's fopen("w") inherits restrictive
    // permissions whoever else can list the scratch directory.
    {
        use std::os::unix::fs::OpenOptionsExt;
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&status_path);
    }
    let mut cmd = std::process::Command::new(&bin);
    if feed {
        cmd.env("LOTUS_REPLAY_FEED", &rec_abs);
        if allow_unmatched_feed {
            cmd.env("LOTUS_REPLAY_FEED_ALLOW_UNMATCHED", "1");
        }
    } else {
        cmd.env("LOTUS_REPLAY", &rec_abs);
    }
    if allow_truncated {
        cmd.env("LOTUS_REPLAY_ALLOW_TRUNCATED", "1");
    }
    if allow_unverified {
        cmd.env("LOTUS_REPLAY_ALLOW_UNVERIFIED", "1");
    }
    cmd.env("LOTUS_REPLAY_STATUS", &status_path);
    // Round 3 + review round 2, finding 1 (artifact trust): the
    // CHILD reads THE file object the CLI admitted — `rec_file`
    // itself, opened once at the top, its descriptor dup2'd to a
    // fixed number post-fork. There is no reopen and therefore no
    // path re-resolution window. The runtime still independently
    // revalidates the structure, snapshots the bytes into anonymous
    // memory, and (defense in depth) refuses a model identity that
    // disagrees with the binary's own.
    {
        use std::os::unix::io::AsRawFd;
        use std::os::unix::process::CommandExt;
        let raw = rec_file.as_raw_fd();
        const REPLAY_FD: i32 = 973;
        cmd.env("LOTUS_REPLAY_FD", REPLAY_FD.to_string());
        unsafe {
            cmd.pre_exec(move || {
                if libc::dup2(raw, REPLAY_FD) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        // Keep the admitted File alive across spawn.
        std::mem::forget(rec_file);
    }
    if let Some(spec) = &at {
        match spec.split_once(':') {
            Some((c, n)) => {
                cmd.env("LOTUS_REPLAY_AT_CONSUMER", c);
                cmd.env("LOTUS_REPLAY_AT", n);
            }
            None => {
                cmd.env("LOTUS_REPLAY_AT", spec);
            }
        }
    }
    if let Some(v) = &verify_path {
        cmd.env("LOTUS_OBS_RECORD", v);
        // The verification recording must apply the SAME env policy
        // the original did, or a full-env recording diffs against a
        // redacted one and reports a spurious withheld divergence.
        if !rec.env_redacted {
            cmd.env("LOTUS_OBS_RECORD_ENV", "full");
        }
    }
    // Test hook (review round 2, finding 1): hold between admission
    // and spawn so the one-object invariant can be tested against a
    // deliberate path replacement, deterministically.
    if let Ok(hold) = std::env::var("HALE_REPLAY_TEST_HOLD") {
        while !std::path::Path::new(&hold).exists() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    let status = cmd.status();
    let code = match status {
        Ok(s) => s.code().unwrap_or(1).clamp(0, 255) as u8,
        Err(e) => {
            eprintln!("could not execute compiled program: {}", e);
            if let Some(v) = &verify_path {
                let _ = std::fs::remove_file(v);
            }
            return ExitCode::from(1);
        }
    };

    // The runtime's machine-readable verdict (review finding 6:
    // success must come from the verdict, not from the absence of
    // one comparator mismatch).
    let runtime_divergences: u64 = std::fs::read_to_string(&status_path)
        .ok()
        .map(|body| {
            body.lines()
                .filter_map(|l| l.split_once('='))
                .filter(|(k, _)| {
                    *k != "consumes" && *k != "post_prefix_live_fallback"
                })
                .filter_map(|(_, v)| v.trim().parse::<u64>().ok())
                .sum()
        })
        .unwrap_or_else(|| {
            eprintln!(
                "hale replay: no runtime verdict was written — \
                 treating as divergent"
            );
            1
        });
    let _ = std::fs::remove_file(&status_path);

    if let Some(v) = &verify_path {
        if !rec.async_schedule_capable {
            eprintln!(
                "hale replay: note — this recording predates \
                 async-schedule support; schedule comparison is \
                 skipped (coverage limitation, not a divergence)"
            );
        }
        if runtime_divergences > 0 {
            let msg = format!(
                "{} runtime divergences (see the summary above)",
                runtime_divergences
            );
            eprintln!("replay DIVERGED: {}", msg);
            if json {
                println!("{}", replay::diverged_json(&msg));
            }
            let _ = std::fs::remove_file(v);
            return ExitCode::from(1);
        }
        let result = match replay::parse(v) {
            Ok(vr) => match replay::diff(&rec, &vr, !rec.clean) {
                None => {
                    // GH #728: name the categories the match
                    // actually compared. "0 consumes across 0
                    // consumers" read as a verified queued schedule
                    // when direct dispatch means there was never a
                    // queued schedule to verify — and it said
                    // nothing about the public bus events and
                    // payloads that WERE compared.
                    let cov = replay::Coverage::of(&rec);
                    if json {
                        println!("{}", cov.json());
                    } else {
                        println!("{}", cov.human());
                    }
                    ExitCode::from(code)
                }
                Some(msg) => {
                    eprintln!("replay DIVERGED: {}", msg);
                    if json {
                        println!("{}", replay::diverged_json(&msg));
                    }
                    ExitCode::from(1)
                }
            },
            Err(e) => {
                eprintln!(
                    "hale replay: verification recording unreadable: {}",
                    e
                );
                if json {
                    println!(
                        "{}",
                        replay::diverged_json(&format!(
                            "verification recording unreadable: {}",
                            e
                        ))
                    );
                }
                ExitCode::from(1)
            }
        };
        let _ = std::fs::remove_file(v);
        return result;
    }
    ExitCode::from(code)
}
