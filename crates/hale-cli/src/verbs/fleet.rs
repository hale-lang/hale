use std::process::ExitCode;
use std::path::Path;
use std::path::PathBuf;
use crate::fleet;
use crate::sign;
/// Flags `check` / `verify` accept, and whether each takes a value.
/// Anything not on this list is a usage error rather than something
/// quietly ignored.
///
/// `--dump-topology` is deliberately absent from the value-taking
/// set: it takes its destination in the `=<path>` form ONLY, and a
/// bare `--dump-topology` writes to stdout. Making it consume a
/// following token would make `hale check --dump-topology app.hl`
/// ambiguous — is `app.hl` the artifact destination or the target? —
/// and flags are supposed to be positionable on either side.
/// Check every deployment declared in `[fleets]`.
///
/// Separate from `--matrix`, which is the ENTRYPOINT x ENVIRONMENT
/// axis. A fleet is an arrangement of deployed instances; an
/// environment is law bound to an entrypoint. A workspace declares
/// both, and `production` in one need not mean `production` in the
/// other.
pub(crate) fn run_fleet_all(from: Option<&Path>, if_declared: bool) -> ExitCode {
    let cwd = from
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let mut dir = cwd.canonicalize().unwrap_or(cwd);
    let manifest = loop {
        let m = dir.join("hale.toml");
        if m.exists() {
            break Some(m);
        }
        match dir.parent() {
            Some(p) => dir = p.to_path_buf(),
            None => break None,
        }
    };
    let Some(manifest) = manifest else {
        if if_declared {
            println!("no hale.toml at or above {}: no fleets declared", dir.display());
            return ExitCode::SUCCESS;
        }
        eprintln!(
            "`hale fleet check` with no plan checks every fleet in \
             `[fleets]`, and no `hale.toml` was found at or above the \
             current directory. Name a plan explicitly, or add one."
        );
        return ExitCode::from(2);
    };
    let fleets = match crate::pkg::read_fleets(&manifest) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::from(2);
        }
    };
    // GH #408 Phase 7: `[fleet_trust]` binds every declared fleet.
    // A key that fails to load is a configuration error for the
    // whole run — skipping it would narrow the trust set silently.
    let trust_paths = match crate::pkg::read_fleet_trust(&manifest) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::from(2);
        }
    };
    let trust = match sign::Trust::load(&trust_paths) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::from(2);
        }
    };
    if fleets.is_empty() {
        if if_declared {
            println!("{} declares no [fleets]", manifest.display());
            return ExitCode::SUCCESS;
        }
        eprintln!(
            "{} declares no `[fleets]`. Add `<name> = \"<plan path>\"` \
             entries, or name a plan explicitly — reporting success for \
             zero deployments would say nothing.",
            manifest.display()
        );
        return ExitCode::from(2);
    }
    let base = manifest.parent().unwrap_or(Path::new(".")).to_path_buf();

    let mut failed: Vec<(String, u8)> = Vec::new();
    for (name, rel) in &fleets {
        let path = base.join(rel);
        println!("=== fleet `{}` ({}) ===", name, path.display());
        if !path.exists() {
            eprintln!("  plan not found: {}", path.display());
            failed.push((name.clone(), 2));
            continue;
        }
        // Every fleet runs. Stopping at the first failure would report
        // a subset of the deployments as if it were all of them.
        match fleet::compose(&path, &trust) {
            Ok(artifact) => {
                let v: serde_json::Value =
                    serde_json::from_str(&artifact).unwrap_or_default();
                println!(
                    "  ok — {} instance(s), {} route(s), fleet_shape_hash {}",
                    v["instances"].as_array().map(|a| a.len()).unwrap_or(0),
                    v["routes"].as_array().map(|a| a.len()).unwrap_or(0),
                    v["fleet_shape_hash"].as_str().unwrap_or("?")
                );
            }
            Err(errs) => {
                for e in &errs {
                    eprintln!("  {}", e);
                }
                failed.push((name.clone(), 1));
            }
        }
    }

    println!();
    if failed.is_empty() {
        println!("ok: {} fleet(s) checked", fleets.len());
        return ExitCode::SUCCESS;
    }
    eprintln!("{} of {} fleet(s) failed:", failed.len(), fleets.len());
    for (n, _) in &failed {
        eprintln!("  {}", n);
    }
    // Worst code wins, so a missing plan is not masked by an ordinary
    // claim failure elsewhere.
    ExitCode::from(failed.iter().map(|(_, c)| *c).max().unwrap_or(1))
}

/// `hale fleet check <plan>` / `hale fleet dump <plan>`.
///
/// `check` composes and reports; `dump` writes the fleet artifact.
/// Both fail on anything a composition cannot honestly build on — an
/// unverifiable component, a semantics mismatch, a component whose
/// own law fails, or endpoints that disagree about a wire contract.
pub(crate) fn run_fleet(rest: &[String]) -> ExitCode {
    let sub = rest.first().map(String::as_str);

    // GH #408 Phase 7: key handling and attestation live under the
    // same verb as composition — the fleet is where certificates
    // change hands.
    match sub {
        Some("keygen") => {
            let Some(prefix) = rest.get(1) else {
                eprintln!("hale fleet keygen <prefix>   write <prefix>.pem + <prefix>.pub.pem");
                return ExitCode::from(2);
            };
            return match sign::keygen(Path::new(prefix)) {
                Ok(key_id) => {
                    println!(
                        "ok: {prefix}.pem (private, 0600) and \
                         {prefix}.pub.pem — key_id {key_id}"
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{}", e);
                    ExitCode::from(1)
                }
            };
        }
        Some("sign") => {
            let (file, key) = match (rest.get(1), rest.get(2), rest.get(3)) {
                (Some(f), Some(flag), Some(k)) if flag == "--key" => (f, k),
                _ => {
                    eprintln!("hale fleet sign <file> --key <priv.pem>   write <file>.sig (ES256 over exact bytes)");
                    return ExitCode::from(2);
                }
            };
            return match sign::sign(Path::new(file), Path::new(key)) {
                Ok((sig_path, key_id)) => {
                    println!(
                        "ok: {} — key_id {}",
                        sig_path.display(),
                        key_id
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{}", e);
                    ExitCode::from(1)
                }
            };
        }
        Some("attest") => {
            let Some(plan) = rest.get(1) else {
                eprintln!("hale fleet attest <plan.json>   compare each instance's binary to its binary_sha256");
                return ExitCode::from(2);
            };
            return match fleet::attest(Path::new(plan)) {
                Ok(msg) => {
                    println!("{}", msg);
                    ExitCode::SUCCESS
                }
                Err(errs) => {
                    for e in &errs {
                        eprintln!("{}", e);
                    }
                    ExitCode::from(1)
                }
            };
        }
        _ => {}
    }

    // `--trust <pub.pem>` (repeatable) on check/dump: strict when
    // given, exactly like `[fleet_trust]` in the manifest.
    let mut args: Vec<&String> = Vec::new();
    let mut trust_paths: Vec<PathBuf> = Vec::new();
    // GH #566 F5: `--in <dir>` checks the fleets a workspace elsewhere
    // declares (a DNA candidate's worktree); `--if-declared` makes a
    // workspace with no fleets a success that says so, for a
    // verification step that runs on every workspace.
    let mut from: Option<PathBuf> = None;
    let mut if_declared = false;
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        if a == "--trust" {
            match it.next() {
                Some(k) => trust_paths.push(PathBuf::from(k)),
                None => {
                    eprintln!("--trust needs a public key path");
                    return ExitCode::from(2);
                }
            }
        } else if a == "--in" {
            match it.next() {
                Some(d) => from = Some(PathBuf::from(d)),
                None => {
                    eprintln!("--in needs a directory");
                    return ExitCode::from(2);
                }
            }
        } else if a == "--if-declared" {
            if_declared = true;
        } else {
            args.push(a);
        }
    }
    let sub = args.first().map(|s| s.as_str());
    let plan = args.get(1);
    // GH #408 Phase 5: `hale fleet check` with no plan checks EVERY
    // deployment the workspace declares. A repository usually has
    // more than one — production, staging, a reconciliation
    // arrangement — and checking whichever one you remembered to name
    // is the same partial-coverage problem `--matrix` solves for
    // entrypoints.
    if sub == Some("check") && plan.is_none() {
        if !trust_paths.is_empty() {
            eprintln!(
                "--trust with no plan: the all-fleets form takes its \
                 trust roots from `[fleet_trust]` in hale.toml, so one \
                 flag cannot quietly rebind every deployment"
            );
            return ExitCode::from(2);
        }
        return run_fleet_all(from.as_deref(), if_declared);
    }
    let (sub, plan) = match (sub, plan) {
        (Some("check"), Some(p)) | (Some("dump"), Some(p)) => {
            (sub.unwrap(), p)
        }
        _ => {
            eprint!("{}", fleet_usage());
            return ExitCode::from(2);
        }
    };
    let trust = match sign::Trust::load(&trust_paths) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::from(2);
        }
    };
    // Phase 4 (C2): a fleet seed composes through its `deployment`
    // rows; `check` says what it derived.
    let target = Path::new(plan);
    let composed = if crate::deployment::is_fleet_seed(target) {
        match crate::deployment::derive_deployment(target) {
            Ok(rows) => {
                if sub == "check" {
                    print_rows(&rows);
                }
                fleet::compose_plan(&rows.composition(), &rows.root, &trust)
            }
            Err(e) => Err(e),
        }
    } else {
        fleet::compose(target, &trust)
    };
    match composed {
        Ok(artifact) => {
            if sub == "dump" {
                print!("{}", artifact);
            } else {
                let v: serde_json::Value =
                    serde_json::from_str(&artifact).unwrap_or_default();
                println!(
                    "ok: fleet `{}` composed — {} instance(s), {} \
                     route(s), fleet_shape_hash {}",
                    v["name"].as_str().unwrap_or("?"),
                    v["instances"].as_array().map(|a| a.len()).unwrap_or(0),
                    v["routes"].as_array().map(|a| a.len()).unwrap_or(0),
                    v["fleet_shape_hash"].as_str().unwrap_or("?")
                );
            }
            ExitCode::SUCCESS
        }
        Err(errs) => {
            for e in &errs {
                eprintln!("{}", e);
            }
            ExitCode::from(1)
        }
    }
}

/// What a fleet seed's `deployment` rows say: each component's
/// instances and placement default, and each derived route.
fn print_rows(rows: &crate::deployment::Deployment) {
    for c in &rows.components {
        let ids: Vec<&str> = rows.instances.iter().filter(|i| i.component == c.param).map(|i| i.id.as_str()).collect();
        let at = match rows.placement.iter().find(|p| p.component == c.param) {
            Some(p) if p.host.is_some() => format!(", on host `{}`", p.host.as_deref().unwrap_or_default()),
            Some(p) if p.replicas > 1 => format!(", {} replicas", p.replicas),
            _ => String::new(),
        };
        println!(
            "component `{}` runs `{}` from {} (shape {}){}: {}",
            c.param,
            c.main,
            c.seed.display(),
            c.shape_hash,
            at,
            ids.join(", ")
        );
        for e in rows.endpoints.iter().filter(|e| e.component == c.param) {
            println!(
                "  {} {} as `{}`{}",
                if e.publish { "publishes" } else { "subscribes" },
                e.subject,
                e.topic,
                match &e.bound {
                    Some(t) => format!(", bound {t}"),
                    None => ", in process".to_string(),
                }
            );
        }
    }
    for r in &rows.routes {
        let ends = |v: &[(String, String)]| v.iter().map(|(i, _)| i.as_str()).collect::<Vec<_>>().join(", ");
        println!(
            "route `{}` ({}, {}): {} -> {}",
            r.id,
            r.subject,
            r.transport,
            ends(&r.publishers),
            ends(&r.subscribers)
        );
    }
}

/// The `hale fleet` surface. One text for two callers: the usage
/// error (stderr, exit 2) and `--help` (stdout, exit 0).
pub(crate) fn fleet_usage() -> &'static str {
    "\
hale fleet check [fleet seed]  compose and check
                                (a seed declaring `fleet locus`, or a plan.json;
                                 none: every fleet in [fleets];
                                 --in <dir> another workspace's, --if-declared: none is ok)
hale fleet dump  <fleet seed>   write the fleet artifact
hale fleet attest <plan.json>   binaries match the plan's sha256 rows
hale fleet keygen <prefix>      ES256 keypair for signing
hale fleet sign <file> --key K  detached .sig over exact bytes

check/dump take --trust <pub.pem> (repeatable):
with trust roots declared, every component must
verify under one of them.

A fleet names exact application INSTANCES (other
seeds' mains) and the topics that wire them; its
routes are derived from the instances' artifacts.
It composes artifacts, never source: matching wire
identities establish compatibility, but only a
binding creates a fleet edge.
"
}
