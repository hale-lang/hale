use std::collections::BTreeMap;
use std::process::ExitCode;
use std::path::Path;
use std::path::PathBuf;
use hale_syntax::ast::Program;
use crate::shared::frontend::collect_checkable;
use crate::shared::options::env_roles;
use crate::shared::source::Disk;
use crate::shared::workspace::collect_seeds;
use std::fs;
use hale_frontend::snapshot::{adopt_into_root, Snapshot};
use super::run_impl::run_check_impl;
use super::run_impl::{check_loaded, load_for_check};
/// GH #409: check every (entrypoint, environment) pair declared in
/// `hale.toml`.
///
/// The property being enforced is "any entrypoint satisfies the
/// claimset for wherever it deploys" — universal quantification over
/// entrypoints, each still checked independently in its own closed
/// world. It composes nothing; it is the workspace sweep with a
/// constitution bound per pair.
pub(crate) fn run_matrix(root: &Path, verify: bool) -> ExitCode {
    let manifest_path = root.join("hale.toml");
    let (envs, base) =
        match crate::pkg::read_claims_config(&manifest_path) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("{}", e);
                return ExitCode::from(2);
            }
        };
    if envs.is_empty() {
        eprintln!(
            "no `[environments.<name>]` sections in {} — `--matrix` \
             checks entrypoints against the claimset each deployment \
             target requires, so it needs at least one",
            manifest_path.display()
        );
        return ExitCode::from(2);
    }

    // Every seed that declares a `main locus` is an entrypoint, and
    // every entrypoint must be accounted for. An entrypoint nobody
    // listed is silently unconstrained — the exact failure this
    // feature exists to remove — so it is an error, not a skip.
    let mut seeds = Vec::new();
    collect_seeds(root, &mut seeds);
    let mut entrypoints: Vec<PathBuf> = Vec::new();
    let mut unparseable: Vec<PathBuf> = Vec::new();
    for s in seeds {
        match seed_entry_kind(&s) {
            EntryKind::Yes => entrypoints.push(s),
            EntryKind::No => {}
            EntryKind::Unparseable(f) => unparseable.push(f),
        }
    }

    let mut bound: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    for (env, spec) in &envs {
        for e in &spec.entrypoints {
            let p = root.join(e);
            bound.entry(p).or_default().push(env.clone());
        }
    }

    let mut failed: Vec<String> = Vec::new();
    let mut seen_identity: BTreeMap<String, (String, String)> =
        BTreeMap::new();
    for f in &unparseable {
        eprintln!(
            "{} does not parse, so whether it is an entrypoint is \
             unknown — and an unknown entrypoint cannot be shown to \
             be covered by any environment. Fix the syntax first",
            f.display()
        );
        failed.push(format!("{} (unparseable)", f.display()));
    }
    for e in &entrypoints {
        let canon = e.canonicalize().unwrap_or_else(|_| e.clone());
        let listed: Vec<&String> = bound
            .iter()
            .filter(|(p, _)| {
                p.canonicalize().map(|c| c == canon).unwrap_or(false)
            })
            .flat_map(|(_, v)| v.iter())
            .collect();
        if listed.is_empty() {
            eprintln!(
                "entrypoint {} is in no environment. Every entrypoint \
                 must say where it deploys — one that is listed \
                 nowhere is checked against no claimset at all",
                e.display()
            );
            failed.push(format!("{} (unbound)", e.display()));
        }
    }

    for (env, spec) in &envs {
        for ep in &spec.entrypoints {
            let target = root.join(ep);
            if !target.exists() {
                eprintln!(
                    "environment `{}` lists {}, which does not exist",
                    env,
                    target.display()
                );
                failed.push(format!("{} @ {} (missing)", ep, env));
                continue;
            }
            println!("=== {} @ {} ===", target.display(), env);
            // The base first, then the environment's own addition.
            // Every pair carries the base, so an environment can only
            // ADD law — monotonicity by construction rather than a
            // rule the manifest is trusted to respect.
            let mut adopt: Vec<String> = Vec::new();
            if let Some(b) = &base {
                adopt.push(b.clone());
            }
            if let Some(c) = &spec.constitution {
                if Some(c) != base.as_ref() {
                    adopt.push(c.clone());
                }
            }
            // The pair's role table too, as `check --env` and `build
            // --env` resolve it (F.40 phase 4, A1).
            let roles = env_roles(spec);
            // One snapshot per pair (F.40 phase 4, A3): the check reads
            // it, and so do the role coverage and the identity
            // comparison below, so a pair loads its seed once. A load
            // that fails has printed why and leaves no snapshot; such a
            // pair (a listed seed the load refuses for having no entry:
            // a library, an imported or a module-nested `main`) is still
            // judged by the loader's programs, as before the snapshot.
            let snap = load_for_check(&target, &adopt, Some(env), Some(&roles));
            let code = match &snap {
                Ok(s) => check_loaded(&target, verify, s),
                Err(code) => *code,
            };
            let snap = snap.ok();
            if code != 0 {
                failed.push(format!("{} @ {}", ep, env));
            }
            // GH #1109: every role the entrypoint declares is mapped
            // here (a `[]` is explicitly nobody), and nothing is
            // mapped that it does not declare: the role rows'
            // projection, not gated on the typing.
            let declared = match &snap {
                Some(s) => s.demand_role_rows().map(|rows| rows.declared_roles()).unwrap_or_default(),
                None => refused_pair_roles(&target),
            };
            for msg in role_coverage(&target, env, &spec.roles, &declared) {
                eprintln!("{}", msg);
                failed.push(format!("{} @ {} (roles)", ep, env));
            }
            // Review finding 3: prove the entrypoints in ONE
            // environment resolved the SAME claimset, not merely the
            // same NAME. Constitution names are flat and unmangled,
            // so two seeds can each declare `Core` with different
            // clauses and both would satisfy the binding. The digest
            // covers the normalized closure, so agreement is real.
            let adopted = match &snap {
                Some(s) => adopted_roots(s),
                None => constitution_identities(&target, &adopt),
            };
            for (name, digest) in adopted {
                // The `[claims] base` is ONE constitution carried by
                // every environment, so it must agree workspace-wide.
                // Keying it per-environment meant two environments
                // with disjoint entrypoints never shared a key, and a
                // base resolving to different closures in dev and
                // prod went undetected — the mechanism proved
                // consistency WITHIN each environment and nothing
                // about the base being shared.
                let key = if Some(&name) == base.as_ref() {
                    format!("base::{}", name)
                } else {
                    format!("env::{}::{}", env, name)
                };
                let scope = if Some(&name) == base.as_ref() {
                    "the workspace base".to_string()
                } else {
                    format!("environment `{}`", env)
                };
                match seen_identity.get(&key) {
                    Some((prev_digest, prev_ep))
                        if *prev_digest != digest =>
                    {
                        eprintln!(
                            "{} resolves `{}` to two different \
                             claimsets: {} sees {}, {} sees {}. One \
                             name must mean one law — the entrypoints \
                             are importing different declarations \
                             that happen to share it",
                            scope, name, prev_ep, prev_digest, ep, digest
                        );
                        failed.push(format!(
                            "{} @ {} (constitution identity)",
                            ep, env
                        ));
                    }
                    Some(_) => {}
                    None => {
                        seen_identity
                            .insert(key, (digest, ep.clone()));
                    }
                }
            }
        }
    }

    println!();
    if failed.is_empty() {
        let pairs: usize =
            envs.values().map(|s| s.entrypoints.len()).sum();
        println!(
            "ok: {} (entrypoint, environment) pair(s) checked",
            pairs
        );
        return ExitCode::SUCCESS;
    }
    eprintln!("{} pair(s) failed:", failed.len());
    for f in &failed {
        eprintln!("  {}", f);
    }
    ExitCode::from(1)
}

/// GH #1109: the role-coverage rule of `--matrix`, per (entrypoint,
/// environment): the roles the entrypoint declares (plus `owner`
/// when it has an api binding), the pair's role rows' projection
/// (`RoleRows::declared_roles`), against the environment's `roles`
/// table. A declared role the table omits is a failure — an omission
/// is indistinguishable from a mistake, and `[]` says "nobody" on
/// purpose; a mapped role nothing declares is one too, because a
/// misspelt key would otherwise map nobody quietly.
pub(crate) fn role_coverage(
    target: &Path,
    env: &str,
    table: &BTreeMap<String, Vec<String>>,
    declared: &[String],
) -> Vec<String> {
    let mut out = Vec::new();
    let missing: Vec<&String> = declared.iter().filter(|r| !table.contains_key(*r)).collect();
    if !missing.is_empty() {
        out.push(format!(
            "{} @ {}: role(s) {} are not mapped in [environments.{}.roles] — map each to \
             its members, or to [] to say explicitly that nobody holds it here",
            target.display(),
            env,
            missing.iter().map(|r| format!("`{}`", r)).collect::<Vec<_>>().join(", "),
            env
        ));
    }
    let extra: Vec<&String> = table.keys().filter(|k| !declared.iter().any(|d| d == *k)).collect();
    if !extra.is_empty() && !declared.is_empty() {
        out.push(format!(
            "{} @ {}: [environments.{}.roles] maps {}, which the entrypoint does not declare \
             (declared: {})",
            target.display(),
            env,
            env,
            extra.iter().map(|r| format!("`{}`", r)).collect::<Vec<_>>().join(", "),
            declared.iter().map(|r| format!("`{}`", r)).collect::<Vec<_>>().join(", ")
        ));
    }
    out
}

/// The `(name, digest)` of each constitution the pair's snapshot
/// adopts: its law selection's identities, the projection the
/// artifact's `evaluation.roots` reads (F.40 phase 4, A3), not gated
/// on the typing.
fn adopted_roots(snap: &Snapshot) -> Vec<(String, String)> {
    let Ok(laws) = snap.demand_law_selection() else {
        return Vec::new();
    };
    let bundle = snap.bundle();
    let programs: Vec<&Program> = bundle.programs.values().copied().collect();
    // ROOTS, not the whole closure: the manifest asked for these by
    // name, so these are what must agree across entrypoints. The
    // closure follows from them.
    laws.identities(&programs).roots.into_iter().map(|i| (i.name, i.digest)).collect()
}

/// The roles a pair with no snapshot declares: a listed seed the load
/// refuses for having no entry, read from the loader's programs before
/// the sequence, as the matrix read every pair's before A3. Dropping
/// the judgment of such a pair would change what the matrix prints, so
/// it is not done here.
fn refused_pair_roles(target: &Path) -> Vec<String> {
    let Ok((programs, _, _, _, _, _)) = collect_checkable(target, &Disk) else {
        return Vec::new();
    };
    let refs: Vec<&Program> = programs.values().collect();
    hale_syntax::api_gen::declared_roles(&refs, hale_types::entry::root_decl(&refs))
}

/// The `(name, digest)` of each constitution adopted when `target`
/// is checked with `adopt`, for a pair with no snapshot (as
/// [`refused_pair_roles`]).
pub(crate) fn constitution_identities(
    target: &Path,
    adopt: &[String],
) -> Vec<(String, String)> {
    let (programs, _s, _fb, renames, _own, _imports) = match collect_checkable(target, &Disk)
    {
        Ok(x) => x,
        Err(_) => return Vec::new(),
    };
    let mut programs = programs;
    let mut refs: Vec<&mut Program> = programs.values_mut().collect();
    adopt_into_root(&mut refs, adopt);
    let bundle_programs: BTreeMap<String, &Program> = programs
        .iter()
        .map(|(p, prog)| (p.display().to_string(), prog))
        .collect();
    let mut bundle = hale_types::Bundle::new(bundle_programs);
    bundle.import_renames = renames;
    let progs: Vec<&Program> =
        bundle.programs.values().copied().collect();
    // The projection the artifact reads, over the programs loaded
    // here: one definition of the identities. No environment is
    // bound: the label only words a diagnostic, which is discarded.
    let ids = hale_types::claims::select_laws(
        &progs,
        &bundle.import_renames,
        &hale_types::claims::EnvBinding::default(),
    )
    .identities(&progs);
    ids.roots.into_iter().map(|i| (i.name, i.digest)).collect()
}

/// Is this seed an entrypoint? Its entry row says (F.40 phase 3, E0):
/// the seed's own top-level `main locus`. Declarations only — an
/// entrypoint is a structural fact, and a seed that fails to TYPECHECK
/// is still an entrypoint whose absence from the matrix must be
/// reported. An imported `main locus` is never the entry (decision 1),
/// so the row is built over the seed's own files and no import is
/// followed: a seed whose import does not resolve is still known to be
/// an entrypoint, and its pair reports the import.
///
/// A seed that fails to PARSE is `Unknown`, never `No`. Treating an
/// unparseable file as "not a main" made a syntax error erase an
/// entrypoint from coverage entirely: a broken seed listed in no
/// environment reported `ok: 1 pair(s) checked`, exit 0, while the
/// same seed made valid was correctly flagged. Breaking your file
/// became a way out of the gate — in the mechanism built to stop law
/// going missing quietly.
pub(crate) enum EntryKind {
    Yes,
    No,
    Unparseable(PathBuf),
}

pub(crate) fn seed_entry_kind(dir: &Path) -> EntryKind {
    let Ok(entries) = fs::read_dir(dir) else {
        return EntryKind::No;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("hl"))
        .collect();
    files.sort();
    // The files in order, up to the first that does not read or parse:
    // an entry before it answers; without one, the seed is unknown.
    let mut parsed: Vec<(String, Program)> = Vec::new();
    let mut broken = None;
    for p in files {
        match fs::read_to_string(&p).ok().and_then(|src| hale_syntax::parse_source(&src).ok()) {
            Some(prog) => parsed.push((p.display().to_string(), prog)),
            None => {
                broken = Some(p);
                break;
            }
        }
    }
    let bundle = hale_types::Bundle::new(parsed.iter().map(|(p, prog)| (p.clone(), prog)).collect());
    if hale_types::entry::entry_row(&bundle).entry().is_some() {
        return EntryKind::Yes;
    }
    match broken {
        Some(p) => EntryKind::Unparseable(p),
        None => EntryKind::No,
    }
}

/// `--workspace`: check EVERY seed under the target, independently.
///
/// It does not connect them. Each seed is its own closed world and
/// gets its own check; this exists so that no library or main-locus
/// claim is silently skipped because nobody remembered to point
/// `check` at that directory. Cross-binary composition is a separate
/// thing entirely and is not what this does.
pub(crate) fn run_workspace(root: &Path, verify: bool) -> ExitCode {
    let mut seeds = Vec::new();
    collect_seeds(root, &mut seeds);
    if seeds.is_empty() {
        eprintln!(
            "no seeds under {} — a seed is a directory holding `.hl` \
             files",
            root.display()
        );
        return ExitCode::from(2);
    }

    let mut failed: Vec<(PathBuf, u8)> = Vec::new();
    for seed in &seeds {
        println!("=== {} ===", seed.display());
        let code = run_check_impl(seed, verify);
        // Every seed runs. Stopping at the first failure would make
        // the command report a subset of the truth, and the whole
        // point is that nothing is silently skipped.
        if code != 0 {
            failed.push((seed.clone(), code));
        }
    }

    println!();
    if failed.is_empty() {
        println!("ok: {} seed(s) checked", seeds.len());
        return ExitCode::SUCCESS;
    }
    eprintln!(
        "{} of {} seed(s) failed:",
        failed.len(),
        seeds.len()
    );
    for (p, code) in &failed {
        eprintln!("  {} (exit {})", p.display(), code);
    }
    // The worst code wins, so a usage error is not masked by an
    // ordinary check failure in another seed.
    ExitCode::from(failed.iter().map(|(_, c)| *c).max().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use hale_frontend::frontend::seed_loads_on_this_thread;

    /// F.40 phase 4, A3: a pair loads its seed once. Its check, its role
    /// coverage and its identity comparison read the one snapshot the
    /// check built; the last two used to load the seed again each, three
    /// loads a pair. The entrypoint declares a role and both environments
    /// adopt a constitution, so both readers have something to read.
    #[test]
    fn a_pair_loads_its_seed_once() {
        let root = std::env::temp_dir().join(format!("hale_matrix_loads_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |rel: &str, src: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
            std::fs::write(&p, src).expect("write");
        };
        write(
            "lib/lib.hl",
            "locus Research { params { n: Int = 0; } fn look() -> Int { return self.n; } }\n\
             group research = { Research };\n\
             constitution Core { solo: count publishers(topic Probe) == 0; }\n\
             type Ping { v: Int; }\n\
             topic Probe { payload: Ping; subject: \"app.probe\"; }\n",
        );
        write(
            "app/main.hl",
            "import \"../lib\" as lb;\nrole support;\n\
             main locus A { params { r: lb::Research = lb::Research { }; } }\nfn main() { A { }; }\n",
        );
        write(
            "hale.toml",
            "[claims]\nno_base = true\n\n\
             [environments.dev]\nconstitution = \"Core\"\nentrypoints = [\"app\"]\n\n\
             [environments.dev.roles]\nsupport = []\nowner = []\n\n\
             [environments.prod]\nconstitution = \"Core\"\nentrypoints = [\"app\"]\n\n\
             [environments.prod.roles]\nsupport = []\nowner = []\n",
        );
        let before = seed_loads_on_this_thread();
        let code = super::run_matrix(&root, false);
        let loads = seed_loads_on_this_thread() - before;
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(code, std::process::ExitCode::SUCCESS, "both pairs hold");
        assert_eq!(loads, 2, "two pairs, one load each");
    }
}
