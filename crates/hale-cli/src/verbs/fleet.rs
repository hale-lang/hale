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
            "`hale fleet check` with no fleet seed checks every fleet in \
             `[fleets]`, and no `hale.toml` was found at or above the \
             current directory. Name a fleet seed explicitly, or add one."
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
            "{} declares no `[fleets]`. Add `<name> = \"<fleet seed>\"` \
             entries, or name a fleet seed explicitly — reporting success for \
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
            eprintln!("  fleet seed not found: {}", path.display());
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
    // Worst code wins, so a missing seed is not masked by an ordinary
    // claim failure elsewhere.
    ExitCode::from(failed.iter().map(|(_, c)| *c).max().unwrap_or(1))
}

/// `hale fleet check|dump|export|sign <fleet seed>` and `keygen`.
///
/// A fleet is a seed declaring `fleet locus`; its `deployment` rows are
/// the one input. `check` derives and composes them and reports;
/// `dump` writes the fleet artifact; `export` writes the rows as the
/// plan document, for whoever reads one; `sign` writes a signature over
/// the rows that `--trust` (or `[fleet_trust]`) verifies.
pub(crate) fn run_fleet(rest: &[String]) -> ExitCode {
    let sub = rest.first().map(String::as_str);
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
            let (seed, key) = match (rest.get(1), rest.get(2), rest.get(3)) {
                (Some(f), Some(flag), Some(k)) if flag == "--key" => (f, k),
                _ => {
                    eprintln!("hale fleet sign <fleet seed> --key <priv.pem>   sign the fleet's deployment rows (ES256)");
                    return ExitCode::from(2);
                }
            };
            let rows = match derive(Path::new(seed)) {
                Ok(r) => r,
                Err(code) => return code,
            };
            let sig = rows.signature_path();
            return match sign::sign(rows.export().as_bytes(), &sig, Path::new(key)) {
                Ok(key_id) => {
                    println!("ok: {} — key_id {}", sig.display(), key_id);
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{}", e);
                    ExitCode::from(1)
                }
            };
        }
        Some("export") => {
            let Some(seed) = rest.get(1) else {
                eprintln!("hale fleet export <fleet seed>   write the deployment rows as the plan document");
                return ExitCode::from(2);
            };
            return match derive(Path::new(seed)) {
                Ok(rows) => {
                    print!("{}", rows.export());
                    ExitCode::SUCCESS
                }
                Err(code) => code,
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
    let seed = args.get(1);
    // GH #408 Phase 5: `hale fleet check` with no seed checks EVERY
    // deployment the workspace declares.
    if sub == Some("check") && seed.is_none() {
        if !trust_paths.is_empty() {
            eprintln!(
                "--trust with no fleet seed: the all-fleets form takes its \
                 trust roots from `[fleet_trust]` in hale.toml, so one \
                 flag cannot quietly rebind every deployment"
            );
            return ExitCode::from(2);
        }
        return run_fleet_all(from.as_deref(), if_declared);
    }
    let (sub, seed) = match (sub, seed) {
        (Some("check"), Some(p)) | (Some("dump"), Some(p)) => (sub.unwrap(), p),
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
    let rows = match derive(Path::new(seed)) {
        Ok(r) => r,
        Err(code) => return code,
    };
    if sub == "check" {
        print_rows(&rows);
    }
    let composed = fleet::verify_rows(&rows, &trust)
        .and_then(|signed_by| fleet::compose_plan(&rows.composition(), &rows.root, signed_by));
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

/// The `deployment` rows of the fleet seed `path`, or the exit code
/// after saying why there are none.
fn derive(path: &Path) -> Result<crate::deployment::Deployment, ExitCode> {
    if !crate::deployment::is_fleet_seed(path) {
        eprintln!(
            "{}: not a fleet seed. A fleet is a seed (a directory or .hl file) declaring \
             `fleet locus NAME {{ … }}`; a plan JSON is not an input (`hale fleet export \
             <seed>` writes one)",
            path.display()
        );
        return Err(ExitCode::from(2));
    }
    crate::deployment::derive_deployment(path).map_err(|errs| {
        for e in &errs {
            eprintln!("{}", e);
        }
        ExitCode::from(1)
    })
}

/// What a fleet seed's `deployment` rows say: each component's
/// instances and placement default, and each derived route.
fn print_rows(rows: &crate::deployment::Deployment) {
    for c in &rows.components {
        let ids: Vec<&str> = rows.instances.iter().filter(|i| i.component == c.param).map(|i| i.id.as_str()).collect();
        let at = match rows.placement.iter().find(|p| p.component == c.param) {
            Some(p) if p.hosts.len() == 1 => format!(", on host `{}`", p.hosts[0]),
            Some(p) if !p.hosts.is_empty() => format!(", on hosts `{}`", p.hosts.join("`, `")),
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
hale fleet check [fleet seed]   derive the deployment rows, compose and check
                                 (none: every fleet in [fleets];
                                  --in <dir> another workspace's, --if-declared: none is ok)
hale fleet dump   <fleet seed>  write the fleet artifact
hale fleet export <fleet seed>  write the deployment rows as the plan document
hale fleet sign   <fleet seed> --key K   sign the rows (fleet.sig beside the seed)
hale fleet keygen <prefix>      ES256 keypair for signing

check/dump take --trust <pub.pem> (repeatable): with
trust roots declared, the rows must carry a signature
under one of them.

A fleet seed declares `fleet locus NAME { … }`: its
INSTANCES (other seeds' mains), their placement
defaults, the topics that wire them and the claims
across them. Its routes are derived from the
instances' artifacts: matching wire identities
establish compatibility, but only a binding creates
a fleet edge.
"
}
