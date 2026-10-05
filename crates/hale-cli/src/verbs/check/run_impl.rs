use super::MODEL_DUMP_DEMANDED;
use std::cell::OnceCell;
use std::sync::atomic::Ordering;
use std::path::Path;
use crate::shared::frontend::LoadMode;
use crate::shared::source::Disk;
use crate::verbs::model::diff_lines;
use hale_frontend::snapshot::{Config, Environment, LoadError, Snapshot};
use crate::shared::diag::render_diag_json;
use crate::shared::diag::render_flows;
use crate::shared::diag::render_located;
use crate::shared::frontend::retain_owned_advisories;
use crate::shared::options::{compile_target, configured_target, flag_value_in, parse_target};
/// Resolve a flat list of import directives originating from one
/// importer directory: for each import, locate the target on disk
/// (entry-relative file or dir, workspace-root fallback dir),
/// parse every `.hl` file, mangle each sub-program with the
/// import alias + the file's stem, and merge the mangled items
/// into `merged_items`. Populates `renames` with
/// `(["<alias>", "<TopName>"], mangled_name)` entries so the
/// codegen can resolve `alias::Name` references downstream.
///
/// Imports inside the imported libs ARE followed (A4, G34): for
/// each lib file's own `import` directives, recurse with the lib's
/// directory as the importer_dir. The `visited` canonical-path set
/// breaks cycles. Each lib gets its own alias-prefixed mangled
/// names, so a transitive util lib reached through two different
/// libs lives twice in the binary — no re-export, no dedup, just
/// per-importer scoped resolution.
/// The topology artifact of the snapshot's model, rendered into `cell`
/// the first time a flag asks for it. A program that does not typecheck
/// has no model, so `doing` is refused, with the error kind that
/// blocked it, and the exit code is the error.
fn topology_artifact<'c>(
    cell: &'c OnceCell<String>,
    snap: &Snapshot,
    target: &Path,
    doing: &str,
) -> Result<&'c str, u8> {
    if cell.get().is_none() {
        match snap
            .demand_model()
            .and_then(|model| {
                Ok((model, snap.demand_effect_certificates()?, snap.demand_alloc_summary()?, snap.demand_law_selection()?))
            })
        {
            Ok((model, effects, summary, laws)) => {
                // The artifact's law rows, constitution identities and
                // environment label are the snapshot's law selection, the
                // one the check reported (outside review of #1283,
                // finding 1). Its law evidence reads the check's effects
                // certificate report and the allocation summary the check
                // read.
                let art = hale_types::topology::dump_topology_over(&snap.bundle(), model, effects, summary, laws);
                let _ = cell.set(art);
            }
            Err(b) => return Err(refuse_without_model(target, doing, b)),
        }
    }
    Ok(cell.get().map(String::as_str).unwrap_or_default())
}

/// The refusal every flag that needs the model prints when the
/// program does not typecheck: exit 1, the program named, the kind of
/// the first error.
fn refuse_without_model(target: &Path, doing: &str, b: &hale_frontend::snapshot::Blocked) -> u8 {
    eprintln!(
        "refusing to {}: `{}` does not typecheck, so its model \
         is not a truthful description of any program. Fix the \
         {} first.",
        doing,
        target.display(),
        b.because.first().map_or("error", |d| d.kind_str())
    );
    1
}

pub(crate) fn run_check_impl(target: &Path, gate_warnings: bool) -> u8 {
    run_check_impl_env(target, gate_warnings, &[])
}

/// GH #409: `adopt_env` names a constitution the *deployment target*
/// requires, from `[environments.<name>]` in `hale.toml`. It is
/// injected into the main locus's `claims` block exactly as if the
/// source had written `adopt C;` — same evaluation, same closed
/// world, same union with whatever the source already adopts.
///
/// Binding it here rather than in source is what lets ONE entrypoint
/// satisfy different claimsets in different environments: it cannot
/// write two conflicting `adopt` lines, but it can be checked twice.
pub(crate) fn run_check_impl_env(
    target: &Path,
    gate_warnings: bool,
    adopt_env: &[String],
) -> u8 {
    run_check_impl_labelled(target, gate_warnings, adopt_env, None)
}

pub(crate) fn run_check_impl_labelled(
    target: &Path,
    gate_warnings: bool,
    adopt_env: &[String],
    env_label: Option<&str>,
) -> u8 {
    // F.18: a whole seed (a directory) is checked to what `build`
    // accepts — a call to a bare name nothing binds is an error here;
    // one file of a seed keeps the permissive reading for a sibling's fn
    //
    // GH #721: a bare IDENTIFIER nothing binds follows the same line.
    // One file of a multi-file seed reads consts its siblings declare,
    // so the leniency is load-bearing there and only there.
    let mut config = Config::check(
        target.is_dir(),
        std::env::args().any(|a| a == "--allow-unowned-subscriber"),
    );
    // `--target`: the target the program is checked for, parsed as
    // `hale build --target` parses it, and the effective target as it
    // is the build's (T1(b)). Without it, a written declaration selects
    // wasm32 and the host is the fallback, as in the build and the
    // editor.
    let args: Vec<String> = std::env::args().collect();
    match flag_value_in(&args, "--target") {
        Ok(None) => {}
        Ok(Some(v)) => match parse_target(&v) {
            Ok(spec) => config.target = configured_target(compile_target(spec), true),
            Err(msg) => {
                eprintln!("{}", msg);
                return 2;
            }
        },
        Err(msg) => {
            eprintln!("{}", msg);
            return 2;
        }
    }
    // GH #409: an environment binds law to an ENTRYPOINT; the snapshot
    // refuses a seed with no main locus, and adopts the environment's
    // constitutions into the one it has.
    config.environment = env_label.map(|name| Environment {
        name: name.to_string(),
        adopt: adopt_env.to_vec(),
    });
    // F.40 phase 2.2a: one snapshot, and the check demanded from it.
    // `check` resolves cross-seed imports the same way `build` and
    // `run` do (an imported seed's bodies are in the program the
    // analysis walks, so effect assertions, budgets and taint do not
    // stop at a seed boundary), then runs the one sequence before the
    // check: sync inference, the desugars, the mint.
    let snap = match Snapshot::load(target, LoadMode::WholeSeed, &Disk, config) {
        Ok(s) => s,
        // GH #765: a failure that carries diagnostics renders them
        // here, honouring `--json` and resolving each span against
        // the file it lives in — including a file reached only
        // through an `import`.
        Err(LoadError::Load(f)) => return f.report(),
        Err(LoadError::Refused(msg)) => {
            eprintln!("{}", msg);
            return 2;
        }
    };
    let (sources, file_bases, import_renames, own_files) =
        (snap.sources(), snap.file_bases(), snap.import_renames(), snap.own_files());
    let bundle = snap.bundle();
    // GH #18 item 1 (step 1): dump the per-method allocation summary +
    // call graph and exit. A diagnostic view of the scaffold; no
    // bound-proving yet.
    if std::env::args().any(|a| a == "--dump-alloc-summary") {
        match snap.demand_alloc_summary() {
            Ok(summary) => print!("{}", hale_types::dump_alloc_summary(summary)),
            Err(b) => {
                for d in &b.because {
                    eprintln!("{}", d.message);
                }
                return 1;
            }
        }
        return 0;
    }
    // GH #18 item 5: dump the per-program resource budget (pinned threads,
    // cooperative pools, bus subjects) and exit.
    // GH #265 step 7: the `.hale.effects` manifest — declared
    // contracts + inferred effect sets, stable-sorted. Emit it for
    // review, or DIFF it against a committed copy so an effect
    // regression (a handler that quietly gained a syscall) fails CI
    // the way an API break does.
    // The manifest reads the snapshot's effect rows; a whole seed's load
    // always has a scope, so they are never blocked here.
    let manifest = || match snap.demand_effects() {
        Ok(effects) => hale_types::dump_effects_manifest(&bundle, effects),
        Err(_) => String::new(),
    };
    if std::env::args().any(|a| a == "--dump-effects-manifest") {
        print!("{}", manifest());
    }
    if let Some(path) = std::env::args()
        .position(|a| a == "--check-effects-manifest")
        .and_then(|i| std::env::args().nth(i + 1))
    {
        let current = manifest();
        match std::fs::read_to_string(&path) {
            Ok(expected) => {
                if expected != current {
                    eprintln!(
                        "effect manifest changed — {} no longer matches the \
                         program's effects.",
                        path
                    );
                    for line in diff_lines(&expected, &current) {
                        eprintln!("{}", line);
                    }
                    eprintln!(
                        "\nIf the change is intended, regenerate:\n  \
                         hale check <target> --dump-effects-manifest > {}",
                        path
                    );
                    return 1;
                }
            }
            Err(_) => {
                eprintln!(
                    "effect manifest baseline not found: {}\nCreate it:\n  \
                     hale check <target> --dump-effects-manifest > {}",
                    path, path
                );
                return 1;
            }
        }
    }
    if std::env::args().any(|a| a == "--dump-resource-budget") {
        match snap.demand_placement().and_then(|table| Ok((table, snap.demand_alloc_summary()?))) {
            Ok((table, summary)) => print!("{}", hale_types::dump_resource_budget(&bundle, table, summary)),
            Err(b) => {
                for d in &b.because {
                    eprintln!("{}", d.message);
                }
                return 1;
            }
        }
        return 0;
    }
    // GH #382 phase 2: the topology artifact — the serialized model
    // (sorts, relations) + every named claim's result, with a
    // `shape_hash` identity over the model half. Emit for review /
    // third-party re-evaluation, or DIFF against a committed copy so
    // an unreviewed topology or law change fails CI.
    // Dump-mode must not change what `check` MEANS. Returning
    // SUCCESS here made `hale check failing.hl --dump-topology` exit
    // 0 with no diagnostics, so a CI job that added the flag to
    // collect an artifact silently stopped gating — the same file
    // without the flag exits 1 with its witness. Print the artifact,
    // then fall through to the ordinary checker so the exit status
    // and diagnostics are unchanged by observing the program.
    //
    // Flag operands accept both spellings (`--flag value` and
    // `--flag=value`). Previously `--check-topology=base.json` was
    // silently ignored and the command SUCCEEDED — the worst
    // failure mode for a CI gate, since the job looks green while
    // gating nothing. A missing operand is likewise a hard usage
    // error rather than a silent no-op.
    let argv: Vec<String> = std::env::args().collect();
    let flag_value = |flag: &str| -> Result<Option<String>, String> {
        if let Some(eq) = argv.iter().find(|a| {
            a.starts_with(flag) && a.as_bytes().get(flag.len()) == Some(&b'=')
        }) {
            let v = &eq[flag.len() + 1..];
            if v.is_empty() {
                return Err(format!("{}= requires a path", flag));
            }
            return Ok(Some(v.to_string()));
        }
        if let Some(i) = argv.iter().position(|a| a == flag) {
            return match argv.get(i + 1) {
                Some(v) if !v.starts_with('-') => Ok(Some(v.clone())),
                _ => Err(format!(
                    "{} requires a path. Use `{} <path>` or \
                     `{}=<path>`.",
                    flag, flag, flag
                )),
            };
        }
        Ok(None)
    };

    let dump_topology = argv.iter().any(|a| a == "--dump-topology");
    // `=<path>` ONLY, never "consume the next token". Sharing
    // `flag_value` here was destructive: it took the following
    // argument as the destination, so `hale check --dump-topology
    // app.hl` — a flag order this command now accepts — OVERWROTE
    // `app.hl` with the artifact. Losing the file you asked it to
    // inspect is the worst possible reading of an ambiguous
    // argument, and the ambiguity is unresolvable in general
    // because the value is optional. A bare `--dump-topology`
    // writes to stdout.
    let dump_topology_to = argv
        .iter()
        .find_map(|a| a.strip_prefix("--dump-topology="))
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());
    // One analysis pass, shared by the artifact gate below and the
    // diagnostic report further down: the snapshot computes the check
    // once, and the model with it when the program declares a law.
    // Its whole-load success means nothing blocks it; a block would
    // report what blocked it.
    let checked: Vec<hale_syntax::Diag> = match snap.demand_check() {
        Ok(c) => c.diags.clone(),
        Err(b) => b.because.clone(),
    };
    // F.40 phase 2.3: the artifact projects the snapshot's model — the
    // one the check judged the laws over, so a program with claims
    // derives one model, not two — rendered once for every flag that
    // reads it (the dump and both gates).
    let artifact_cell: OnceCell<String> = OnceCell::new();

    if dump_topology || dump_topology_to.is_some() {
        // The artifact's EXISTENCE means the model is sound.
        //
        // A program that fails to typecheck still produced a full
        // artifact — populated relations, and claims evaluated over a
        // graph derived from source the compiler could not
        // understand. A claim would report `"result": "holds"` for a
        // program that cannot compile: a certificate asserting a
        // property of something that will never run. Worse for a
        // consumer than no artifact, because an admission step
        // looking for "no violated claims" passes it.
        //
        // A VIOLATED claim is the opposite case and still emits: the
        // model is well-defined, the row is a truthful report, and
        // being able to replay a violation independently is the point
        // of publishing the model at all. `DiagKind::Claim` is what
        // separates the two.
        if let Some(d) = checked
            .iter()
            .find(|d| d.is_error() && d.kind != hale_syntax::error::DiagKind::Claim)
        {
            eprintln!(
                "refusing to emit a topology artifact: `{}` does not \
                 typecheck, so its model is not a truthful \
                 description of any program. Fix the {} first.",
                target.display(),
                d.kind_str()
            );
            return 1;
        }
        let artifact = match topology_artifact(&artifact_cell, &snap, target, "emit a topology artifact") {
            Ok(a) => a,
            Err(code) => return code,
        };
        match &dump_topology_to {
            Some(path) => {
                if let Err(e) = std::fs::write(path, artifact) {
                    eprintln!("could not write {}: {}", path, e);
                    return 2;
                }
            }
            None => print!("{}", artifact),
        }
    }
    // GH #1107: the api binding's description. Same refusal rule as
    // the artifact. A program with no `api:` entry prints nothing and
    // succeeds: there is nothing to describe, and `hale describe` says
    // so.
    let dump_api = argv.iter().any(|a| a == "--dump-api");
    let dump_api_to = argv
        .iter()
        .find_map(|a| a.strip_prefix("--dump-api="))
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string());
    if dump_api || dump_api_to.is_some() {
        if let Some(d) = checked
            .iter()
            .find(|d| d.is_error() && d.kind != hale_syntax::error::DiagKind::Claim)
        {
            eprintln!(
                "refusing to emit an api description: `{}` does not \
                 typecheck, so its description would name a program \
                 that does not exist. Fix the {} first.",
                target.display(),
                d.kind_str()
            );
            return 1;
        }
        // F.40 phase 2.3: the snapshot's surface, the one its desugar
        // sequence generated the binding for, so the description is the
        // binding's by construction. The bytes the binding serves, never
        // re-serialized (a `Value` round trip would sort the keys;
        // spec/model.md promises the two documents agree byte for byte).
        let text = match snap.api_surface() {
            Some(surface) => hale_syntax::api_gen::describe(surface) + "\n",
            None => String::new(),
        };
        match &dump_api_to {
            Some(path) => {
                if let Err(e) = std::fs::write(path, &text) {
                    eprintln!("could not write {}: {}", path, e);
                    return 2;
                }
            }
            None => print!("{}", text),
        }
    }
    // GH #476 Change 2: the canonical-model demand surface. Same
    // refusal rule as the artifact — a model of a program that does
    // not typecheck describes nothing.
    if argv.iter().any(|a| a == "--dump-model")
        || MODEL_DUMP_DEMANDED.load(Ordering::Relaxed)
    {
        if let Some(d) = checked.iter().find(|d| {
            d.is_error()
                && d.kind != hale_syntax::error::DiagKind::Claim
        }) {
            eprintln!(
                "refusing to derive a model: `{}` does not typecheck, \
                 so its model is not a truthful description of \
                 any program. Fix the {} first.",
                target.display(),
                d.kind_str()
            );
            return 1;
        }
        // The check's own model when the program declares a law, so a
        // dump of a claim-bearing program derives it once.
        let model = match snap.demand_model() {
            Ok(m) => m,
            Err(b) => {
                eprintln!(
                    "refusing to derive a model: `{}` does not typecheck, \
                     so its model is not a truthful description of \
                     any program. Fix the {} first.",
                    target.display(),
                    b.because.first().map_or("error", |d| d.kind_str())
                );
                return 1;
            }
        };
        if let Err(e) = model.validate() {
            // A builder bug, never user error: the derivation
            // produced a value that is not a model. Loud, named, and
            // fatal — an invalid model must not print as one.
            eprintln!(
                "internal error: derived model violates a model law: \
                 {:?} (this is a hale bug — please report it)",
                e
            );
            return 2;
        }
        print!(
            "{}",
            hale_types::model_builder::render_internal(model)
        );
    }
    // P2 from the devex review: `--check-topology` compares the
    // ENTIRE artifact text, so a claim rename or a comment-only edit
    // that moves every provenance offset fails the gate and reports
    // that the program's "model" changed — even when `shape_hash`,
    // which is the model's identity, did not. Both gates now exist
    // and are named for what they compare:
    //   --check-topology        exact artifact snapshot (law +
    //                           model + provenance)
    //   --check-topology-shape  the model identity only
    // `shape_hash` was already verified stable across renames and
    // source motion, and sensitive to a real graph change, so it is
    // the right key for the loose gate.
    let shape_gate = match flag_value("--check-topology-shape") {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{}", msg);
            return 2;
        }
    };
    if let Some(path) = shape_gate {
        // The current side is the model's identity read from the
        // model (`project_shape_hash`, the value the artifact stamps),
        // rendered as the artifact renders it; only the baseline, a
        // file, is read from text (F.40 phase 2.4). A program that
        // does not typecheck has no model to compare, and is refused
        // by name as the artifact gate refuses it.
        let current = match snap.demand_model() {
            Ok(model) => {
                format!("{:016x}", hale_types::topology_projection::project_shape_hash(model))
            }
            Err(b) => return refuse_without_model(target, "compare a topology baseline", b),
        };
        // The hash VALUE, not the raw line — the gate's whole point
        // is that this is the model's identity, and a diagnostic
        // that makes you read past `"shape_hash": ` and a trailing
        // comma to compare two hex strings is working against that.
        let hash_of = |s: &str| -> Option<String> {
            s.lines()
                .find(|l| l.contains("\"shape_hash\""))
                .and_then(|l| l.split(':').nth(1))
                .map(|v| v.trim().trim_matches(',').trim_matches('"').to_string())
        };
        match std::fs::read_to_string(&path) {
            Ok(expected) if matches!(
                hale_types::topology::verify_artifact_digest(&expected),
                Some(false)
            ) =>
            {
                // This gate greps ONE line out of the baseline, so a
                // baseline whose `shape_hash` line was edited to match
                // would pass while the model it claims to describe
                // says otherwise. The whole-body digest is what makes
                // the grepped line trustworthy; a baseline that fails
                // it is not a mismatch to report, it is a file that
                // cannot be reasoned about.
                eprintln!(
                    "topology baseline {} is corrupt: its \
                     `artifact_digest` does not match its contents. \
                     Regenerate it:\n  hale check <target> \
                     --dump-topology > {}",
                    path, path
                );
                return 2;
            }
            Ok(expected) => match (hash_of(&expected), Some(current)) {
                (Some(a), Some(b)) if a != b => {
                    eprintln!(
                        "topology SHAPE changed — the program's \
                         model no longer matches {}.\n  baseline: \
                         {}\n  current:  {}\n\nClaim renames and \
                         source motion do NOT affect this gate; a \
                         changed graph does. Regenerate:\n  hale \
                         check <target> --dump-topology > {}",
                        path, a, b, path
                    );
                    return 1;
                }
                (None, _) | (_, None) => {
                    eprintln!(
                        "topology baseline {} has no shape_hash line",
                        path
                    );
                    return 2;
                }
                _ => {}
            },
            Err(_) => {
                eprintln!(
                    "topology baseline not found: {}\nCreate \
                     it:\n  hale check <target> --dump-topology > {}",
                    path, path
                );
                return 1;
            }
        }
    }
    let check_topology_path = match flag_value("--check-topology") {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{}", msg);
            return 2;
        }
    };
    if let Some(path) = check_topology_path {
        let current = match topology_artifact(&artifact_cell, &snap, target, "compare a topology baseline") {
            Ok(a) => a,
            Err(code) => return code,
        };
        match std::fs::read_to_string(&path) {
            Ok(expected) => {
                if expected != current {
                    eprintln!(
                        "topology artifact changed — {} no longer matches \
                         byte-for-byte. This gate covers law, model AND \
                         provenance, so a claim rename or source motion \
                         trips it even when the model is identical; use \
                         --check-topology-shape to gate the model alone.",
                        path
                    );
                    for line in diff_lines(&expected, current) {
                        eprintln!("{}", line);
                    }
                    eprintln!(
                        "\nIf the change is intended, regenerate:\n  \
                         hale check <target> --dump-topology > {}",
                        path
                    );
                    return 1;
                }
            }
            Err(_) => {
                eprintln!(
                    "topology baseline not found: {}\nCreate it:\n  \
                     hale check <target> --dump-topology > {}",
                    path, path
                );
                return 1;
            }
        }
    }
    // GH #18 item 5: the CI gate. `--check-resource-budget <path>` reads a
    // TOML ceiling file and fails the build if any count exceeds it.
    {
        let cli_args: Vec<String> = std::env::args().collect();
        let ceiling_path = cli_args
            .iter()
            .position(|a| a == "--check-resource-budget")
            .and_then(|i| cli_args.get(i + 1));
        if let Some(path) = ceiling_path {
            #[derive(serde::Deserialize, Default)]
            #[serde(deny_unknown_fields)]
            struct CeilingToml {
                pinned_threads: Option<usize>,
                cooperative_pools: Option<usize>,
                bus_subjects: Option<usize>,
                fd_open_sites: Option<usize>,
            }
            let text = match std::fs::read_to_string(path) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("--check-resource-budget: cannot read `{}`: {}", path, e);
                    return 1;
                }
            };
            let ct: CeilingToml = match toml::from_str(&text) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("--check-resource-budget: invalid budget file `{}`: {}", path, e);
                    return 1;
                }
            };
            let ceiling = hale_types::resource_budget::ResourceCeiling {
                pinned_threads: ct.pinned_threads,
                cooperative_pools: ct.cooperative_pools,
                bus_subjects: ct.bus_subjects,
                fd_open_sites: ct.fd_open_sites,
            };
            let (table, summary) =
                match snap.demand_placement().and_then(|table| Ok((table, snap.demand_alloc_summary()?))) {
                    Ok(both) => both,
                    Err(b) => {
                        for d in &b.because {
                            eprintln!("{}", d.message);
                        }
                        return 1;
                    }
                };
            let violations = hale_types::check_resource_ceiling(&bundle, table, summary, &ceiling);
            if violations.is_empty() {
                println!("resource budget OK (within `{}`)", path);
                return 0;
            }
            for v in &violations {
                eprintln!(
                    "resource budget exceeded: {} — raise the ceiling in `{}` if intentional",
                    v, path
                );
            }
            return 1;
        }
    }
    let mut diags = checked;
    // Advisories about code the target does not own are dropped.
    //
    // `check` resolving imports is what makes cross-seed ERRORS
    // visible, and that is the point — a soundness violation reached
    // through an import is still your violation. But the same change
    // drags every advisory lint in every imported seed into the
    // target's output: checking one downstream app began reporting 47
    // hot-path warnings from `lib/` and `pond/`, and since `hale
    // verify` gates on ANY finding, 10 of 12 apps that passed it
    // started failing. A gate that goes red for library internals
    // you cannot edit from here is a gate people switch off.
    //
    // Nothing is lost: an advisory about a seed is reported when that
    // seed is checked, which is how a multi-seed project is checked
    // anyway — a real multi-seed project checks every seed directly).
    // Errors are NEVER filtered, wherever they originate.

    // GH #18 item 1 → M3 stage 5 (2026-07-02): unbounded-allocation
    // warnings are DEFAULT-ON (Riley's flip call after the 402-warning
    // audit: every audited true positive preserved, every residual FP
    // in a documented accepted class — see
    // notes/unbounded-alloc-audit-2026-07-02.md). The analysis itself
    // spares run-to-exit programs (a `main` with no run loop and no
    // bus handler warns nothing), so scripts still owe nothing.
    //
    // Surfaces:
    //  - default: the whole-program survey, every site.
    //  - `--no-warn-unbounded-alloc` — the opt-OUT.
    //  - `--warn-unbounded-alloc` — accepted-and-ignored (former
    //    opt-in spelling).
    //  - `@unbounded fn` carves a fn out; `@bounded locus` is now
    //    redundant with the default but still accepted.
    // Warnings print but never fail the build (only errors do).
    let survey_all =
        !std::env::args().any(|a| a == "--no-warn-unbounded-alloc");
    // Over the snapshot's summary: the one the check's certificate
    // engine read, blocked only with the scope.
    if let Ok(summary) = snap.demand_alloc_summary() {
        diags.extend(hale_types::unbounded_alloc_warnings(&bundle, summary, survey_all));
    }
    // GH #18 item 5: opt-in fd-resource-leak warnings.
    if std::env::args().any(|a| a == "--warn-resource-leak") {
        if let Ok(summary) = snap.demand_alloc_summary() {
            diags.extend(hale_types::resource_leak_warnings(summary));
        }
    }
    // GH #436: opt-in fail-closed `@secret` containment. The default
    // `@secret` pass is a LINT (warnings, narrow traversal). This one
    // walks every branch, propagates aliases, and reports
    // `uncertified` for anything it cannot follow — it newly fails
    // programs that compile today, which is why it is a flag and not
    // the default. See spec/verification.md § "Secrets".
    // GH #436: which loci could be `@sealed` today, and what it would
    // cost. `@sealed` is opt-in, so adopting it across an existing
    // codebase is otherwise a question you can only answer by reading.
    if std::env::args().any(|a| a == "--sealable") {
        let progs: Vec<&hale_syntax::ast::Program> =
            bundle.programs.values().copied().collect();
        let rows = hale_types::sealability::survey(&progs);
        eprint!("{}", hale_types::sealability::render(&rows));
    }
    // GH #736: which `release` clause makes a locus type a flow. Whether
    // `T` is a flow is decided over the whole program, imported seeds
    // included, so a child still reclaimed when its `run()` returns after
    // its own owner dropped the hook is answered here: every clause that
    // names its type, with the file and line.
    // The rows are the snapshot's, the ones the check judged with.
    if std::env::args().any(|a| a == "--flows") {
        let flows = snap.demand_flows().expect("a whole seed has no hole, so its flow rows are never blocked");
        eprint!("{}", render_flows(flows, file_bases, sources, import_renames));
    }
    if std::env::args().any(|a| a == "--strict-secret") {
        let progs: Vec<&hale_syntax::ast::Program> =
            bundle.programs.values().copied().collect();
        diags.extend(hale_types::frontier::secret_taint_strict(&progs));
    }
    // GH #730 / #1048: a borrow must outlive its holder — a handle stored
    // by name into a locus-carrying field, or kept by a method
    // (`Router.add`), is never the holder's to reclaim, so the frame, the
    // dispatch or the binding that owns it must last longer than the
    // holder. Errors, with the witness call site where a parameter carries
    // the handle in. Beside it the GH #737 notice. `build`, `run` and
    // `test` refuse the same through `check_bundle_for_build`.
    // Which locus accepts which child is the snapshot's ownership graph's
    // (its rows; the bundle's own walk where the graph is blocked).
    {
        let progs: Vec<&hale_syntax::ast::Program> =
            bundle.programs.values().copied().collect();
        let own;
        let ownership = match snap.demand_ownership_graph() {
            Ok(graph) => &graph.rows,
            Err(_) => {
                own = hale_types::ownership_graph::OwnershipRows::of(&bundle);
                &own
            }
        };
        diags.extend(hale_types::borrow_lifetime::borrow_lifetime_diags_with_renames(
            &progs,
            &bundle.snapshot,
            &bundle.import_renames,
            ownership,
        ));
    }
    // #8 LSP groundwork (2026-07-02): `hale check --json` emits
    // NDJSON diagnostics on STDOUT (one object per line: file,
    // line, col, severity, kind, message) for editor/LSP
    // consumption. The human rendering stays on stderr otherwise.
    // With `hale check` at ~10 ms on the largest apps, an
    // on-save/on-keystroke loop needs nothing more than this.
    let json_mode = std::env::args().any(|a| a == "--json");
    // Every diagnostic renders in the spelling the author wrote.
    // Effect witnesses were demangled at their source, but any other
    // check that names a type or method — the no-locus-return rule,
    // for one — still emitted `__lib_lib_a_b_OrderBook.query_bulk`,
    // a symbol that appears nowhere in their program. Doing it once
    // here covers every pass rather than each remembering.
    hale_types::stdlib_bodies::demangle_imports(&mut diags, import_renames);
    retain_owned_advisories(&mut diags, own_files, file_bases);
    if !diags.is_empty() {
        for d in &diags {
            if json_mode {
                println!("{}", render_diag_json(d, file_bases, sources));
            } else {
                eprintln!("{}", render_located(d, file_bases, sources));
            }
        }
        // check: warnings print but don't fail; only errors do.
        // verify: everything gates.
        if gate_warnings {
            if !json_mode {
                eprintln!(
                    "verify: {} finding(s) — the discipline gate \
                     fails on advisories too",
                    diags.len()
                );
            }
            return 1;
        }
        if diags.iter().any(|d| d.is_error()) {
            return 1;
        }
    }
    // T4: the link libraries a build of this program would take —
    // `--link`, and each imported package's `[ffi] link` — held to the
    // `LinkLibrary` cell, as `hale build` holds them before any tool is
    // looked up: a record against the manifest's line, or the flag.
    if let Ok(row) = snap.demand_target() {
        let args: Vec<String> = std::env::args().collect();
        let links: Vec<String> =
            args.windows(2).filter(|w| w[0] == "--link").map(|w| w[1].clone()).collect();
        let entry_dir = if target.is_dir() {
            target.to_path_buf()
        } else {
            target.parent().unwrap_or(Path::new(".")).to_path_buf()
        };
        let inputs = crate::shared::options::link_inputs(
            &links,
            snap.entry_imports(),
            &entry_dir,
            crate::shared::workspace::find_workspace_root(target).as_deref(),
        );
        let refused = crate::shared::options::link_refusals(row, &inputs);
        if !refused.is_empty() {
            for r in &refused {
                if json_mode {
                    println!("{}", serde_json::json!({ "severity": "error", "kind": "capability", "message": r }));
                } else {
                    eprintln!("{r}");
                }
            }
            return 1;
        }
    }
    if !json_mode {
        // Count the target's own files, not `programs` entries — a
        // multi-file seed merges into one program before checking.
        let n_files = own_files.len().max(snap.programs().len());
        if gate_warnings {
            eprintln!("verified: {} file(s), 0 findings", n_files);
        } else {
            eprintln!("ok: {} file(s) typechecked", n_files);
        }
    }
    0
}
