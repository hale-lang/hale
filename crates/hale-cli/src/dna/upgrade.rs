//! `hale habitat organism upgrade <organism> [--plan]`: a grafted organism
//! moved to the revision its sapling is pinned at now.
//!
//! A nursery's newer revision of a pinned sapling is filed for the Board
//! (`habitat.sapling.proposed`) and pinned only when the Board ratifies it
//! (`hale habitat nursery ratify`); nothing grafted moves with the pin. The
//! upgrade is the act that moves one organism, keeping its record, its data
//! and its allocations:
//!
//! 1. where it stands: the revision its graft row names, or the latest
//!    `habitat.organism.upgraded` row's; refused when that is the pinned
//!    tree already (a newer revision only proposed is named, with the
//!    ratify that pins it);
//! 2. the pinned revision fetched into the habitat's sapling store
//!    (`<habitat>/saplings/<name>`, the organism's own directory) and its
//!    digest checked as graft checks it: a mismatch refuses, nothing moves;
//! 3. the tree checked out there at it, the graft's own changes carried
//!    (its manifest's record pin and the tables it added, its `.gitignore`
//!    lines), anything else changed in the checkout carried only when it
//!    applies cleanly at the new revision, else refused before anything
//!    moves; `vendor/dna` re-materialized when the graft wrote one;
//! 4. its heart migrations run at the new revision, its services
//!    allocation re-applied through its provider (which builds its images
//!    from the new tree), and a `habitat.organism.upgraded` row recorded;
//! 5. each host command placed for it (`organism place`) re-run at the new
//!    revision, over the person's SSH logins: at a terminal, or listed as the
//!    `place` lines left to run.
//!
//! Through a pointer (`up --host`), steps 1 to 4 run on the habitat's
//! machine (`--no-placements`) and step 5 from this one, as `place` does.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use serde_json::{json, Value};

use super::*;

/// A revision as the habitat's record names it: the sapling's revision and
/// its tree's digest.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Rev {
    pub revision: String,
    pub digest: String,
}

impl Rev {
    fn of(body: &Value, rev: &str) -> Rev {
        Rev { revision: body[rev].as_str().unwrap_or("").to_string(), digest: body["digest"].as_str().unwrap_or("").to_string() }
    }
    fn said(&self) -> String {
        format!("{} ({})", self.revision, self.digest)
    }
}

/// Where a grafted organism stands against its sapling's pin, as the
/// habitat's record says.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Standing {
    pub organism: String,
    pub child: String,
    pub sapling: String,
    /// the roles its graft allocated
    pub needs: Vec<String>,
    /// the revision it is at: its graft's, or its latest upgrade's
    pub at: Rev,
    /// the sapling's pin, its `habitat.sapling` body; `None` when the
    /// habitat pins it no longer
    pub pin: Option<Value>,
    /// a newer revision filed after the pin, which the Board has not
    /// ratified
    pub proposed: Option<Rev>,
}

/// The row bodies of `rows` (`{kind, entity, body}`, the body a JSON
/// string), parsed.
fn body_of(r: &Value) -> Value {
    r["body"].as_str().and_then(|b| serde_json::from_str(b).ok()).unwrap_or_default()
}

/// Where the organism `name` (or keyed `name`) stands, from the habitat's
/// record `rows`.
pub(super) fn standing(rows: &[Value], name: &str) -> Result<Standing, String> {
    let keys = live_grafts_named(rows, name);
    let key = match keys.as_slice() {
        [k] => k.clone(),
        [] => return Err(format!("the habitat grafted no organism `{name}`")),
        _ => return Err(format!("the habitat grafted more than one organism named `{name}`: name it by its key")),
    };
    let graft = rows.iter().filter(|r| r["kind"] == "habitat.grafted" && r["entity"] == key.as_str()).last().map(body_of).unwrap_or_default();
    let organism = graft["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(&key).to_string();
    let sapling = graft["sapling"].as_str().unwrap_or("").to_string();
    if sapling.is_empty() {
        return Err(format!("`{organism}` was not grafted from a sapling: an upgrade moves an organism grafted from a sapling the habitat pins to the revision its Board ratified (a directory grafted is its own checkout, which its git moves)"));
    }
    let mut at = Rev::of(&graft, "revision");
    // its upgrades since, in the record's order
    for r in rows.iter().filter(|r| r["kind"] == "habitat.organism.upgraded" && r["entity"] == key.as_str()) {
        at = Rev::of(&body_of(r), "to");
    }
    // the pin, and a proposal filed after it
    let mut pin: Option<(usize, Value)> = None;
    let mut proposal: Option<(usize, Value)> = None;
    for (i, r) in rows.iter().enumerate().filter(|(_, r)| r["entity"] == sapling.as_str()) {
        match r["kind"].as_str() {
            Some("habitat.sapling") => pin = Some((i, body_of(r))),
            Some("habitat.sapling.proposed") => proposal = Some((i, body_of(r))),
            _ => {}
        }
    }
    let proposed = match (&pin, &proposal) {
        (Some((pi, p)), Some((qi, q))) if qi > pi && q["digest"] != p["digest"] => Some(Rev::of(q, "revision")),
        _ => None,
    };
    let needs = graft["needs"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    Ok(Standing { organism, child: graft["child"].as_str().unwrap_or(&key).to_string(), sapling, needs, at, pin: pin.map(|(_, b)| b), proposed })
}

/// What an upgrade moves, from and to.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Move {
    pub from: Rev,
    pub to: Rev,
}

/// The move the standing asks for, or why there is none: the habitat pins
/// the sapling no longer, or the organism is at the pinned tree already (a
/// newer revision only proposed is named, with the ratify that pins it).
pub(super) fn decide(s: &Standing) -> Result<Move, String> {
    let Some(pin) = &s.pin else {
        return Err(format!("the habitat pins no sapling `{}` any more: there is no revision to upgrade `{}` to", s.sapling, s.organism));
    };
    let to = Rev::of(pin, "revision");
    if to.digest == s.at.digest {
        let waiting = match &s.proposed {
            Some(p) => format!("; {} is proposed and not ratified: `hale habitat nursery ratify {}` (the Board's) pins it, and the upgrade moves to it then", p.revision, s.sapling),
            None => String::new(),
        };
        return Err(format!("`{}` is already at {}, the revision the habitat pins for `{}`{waiting}", s.organism, to.said(), s.sapling));
    }
    Ok(Move { from: s.at.clone(), to })
}

// ---------------------------------------------------------------
// the arguments
// ---------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub(super) struct Args {
    pub organism: String,
    pub plan: bool,
    /// the habitat-side part alone: the placements are left to whoever ran
    /// it (the person's machine, through a pointer)
    pub no_placements: bool,
}

pub(super) fn parse(args: &[String]) -> Option<Args> {
    let (mut organism, mut plan, mut no_placements) = (None, false, false);
    for a in args {
        match a.as_str() {
            "--plan" => plan = true,
            "--no-placements" => no_placements = true,
            w if w.starts_with('-') => return None,
            w => {
                if organism.replace(w.to_string()).is_some() {
                    return None;
                }
            }
        }
    }
    Some(Args { organism: organism?, plan, no_placements })
}

pub(super) const USAGE: &str = "usage: hale habitat organism upgrade <organism> [--plan]";

/// `hale habitat organism upgrade <organism> [--plan] [--no-placements]`.
pub(super) fn cmd(args: &[String]) -> ExitCode {
    let Some(a) = parse(args) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let fail = |why: String| {
        eprintln!("hale habitat organism upgrade {}: {why}", a.organism);
        ExitCode::from(1)
    };
    // through a pointer: the habitat-side part on the habitat's machine,
    // the placements from here, where the person's SSH logins are
    if let Some(p) = remote::pointer() {
        let mut there: Vec<&str> = vec!["organism", "upgrade", a.organism.as_str(), "--no-placements"];
        if a.plan {
            there.push("--plan");
        }
        let code = remote::run_there(&p, "organism", &there);
        if code != ExitCode::SUCCESS || a.no_placements {
            return code;
        }
        return placements_part(&named(&a.organism), None, a.plan);
    }
    let rows = rows_of(&genome_dir());
    let s = match standing(&rows, &a.organism) {
        Ok(s) => s,
        Err(why) => return fail(why),
    };
    let m = match decide(&s) {
        Ok(m) => m,
        Err(why) => return fail(why),
    };
    let manifest = match habitat_side(&s, &m, a.plan) {
        Ok(text) => text,
        Err(why) => return fail(why),
    };
    if a.no_placements {
        return ExitCode::SUCCESS;
    }
    placements_part(&s.organism, Some(&manifest), a.plan)
}

// ---------------------------------------------------------------
// the habitat's side: fetch, move, migrate, apply, record
// ---------------------------------------------------------------

fn git_in(dir: &Path, args: &[&str]) -> Result<String, String> {
    let o = Command::new("git").arg("-C").arg(dir).args(args).output().map_err(|e| format!("git: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).to_string())
    } else {
        Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim()))
    }
}

/// `git` in `dir` with `input` on its stdin (a patch), in the index file
/// `index` when one is given.
fn git_with(dir: &Path, args: &[&str], input: &[u8], index: Option<&Path>) -> Result<(), String> {
    let mut c = Command::new("git");
    c.arg("-C").arg(dir).args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    if let Some(i) = index {
        c.env("GIT_INDEX_FILE", i);
    }
    let mut child = c.spawn().map_err(|e| format!("git: {e}"))?;
    if let Some(mut w) = child.stdin.take() {
        use std::io::Write;
        w.write_all(input).map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    }
    let o = child.wait_with_output().map_err(|e| format!("git: {e}"))?;
    if o.status.success() {
        Ok(())
    } else {
        Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim()))
    }
}

/// The pinned revision `pin` of the sapling `name` fetched into its store
/// and its digest checked, as graft fetches and checks it: the commit. A
/// tree that is not the pinned one is refused, and nothing moves.
pub(super) fn fetch_pinned(store: &Path, name: &str, pin: &Value) -> Result<String, String> {
    let field = |k: &str| pin[k].as_str().unwrap_or("").to_string();
    let (repo, revision, digest, nursery) = (field("repo"), field("revision"), field("digest"), field("nursery"));
    if !store.join(".git").exists() {
        return Err(format!("the sapling's store {} holds no repository: the organism's tree is not on this machine", store.display()));
    }
    let commit = if let Some(path) = repo.strip_prefix(sapling::EMBEDDED) {
        // a tree the toolchain carries: made a repository beside the store,
        // checked as it is made, and fetched from there
        if nursery != "toolchain" {
            return Err(format!("the sapling `{name}` names the toolchain's tree but comes from {nursery}: only the toolchain's nursery offers what the toolchain carries"));
        }
        let made = store.with_file_name(format!(".{name}.upgrade-{}", std::process::id()));
        let commit = sapling::embedded_genome(&sapling::embedded_files(path), &made, &digest, name).map_err(|e| e.replace("nothing grafted", "nothing moved"))?;
        let fetched = git_in(store, &["fetch", "-q", &made.to_string_lossy(), "HEAD"]);
        let _ = std::fs::remove_dir_all(&made);
        fetched.map_err(|e| format!("the sapling `{name}` the toolchain carries could not be fetched into {}: {e}", store.display()))?;
        commit
    } else {
        let fetch_repo = if !Path::new(&repo).exists() && crate::dna::pack::staged_repo_dir(&repo).is_dir() { crate::dna::pack::staged_repo_dir(&repo).display().to_string() } else { repo.clone() };
        if fetch_repo != repo {
            let _ = git_in(store, &["remote", "set-url", "origin", &fetch_repo]);
        }
        super::progress(&format!("fetching `{name}` at {revision} from {repo}"));
        let fetched = git_in(store, &["fetch", "-q", "--tags", "--force", "origin"]);
        if fetch_repo != repo {
            let _ = git_in(store, &["remote", "set-url", "origin", &repo]);
        }
        fetched.map_err(|e| format!("the sapling `{name}` could not be fetched from {repo}: {e}"))?;
        revision.clone()
    };
    let (commit, found) = crate::dna::pack::digest_at(store, &commit)?;
    if found != digest {
        return Err(format!("the sapling `{name}` at {revision} is {found}, not the {digest} the habitat pins: refused, nothing moved"));
    }
    Ok(commit)
}

/// The files a graft writes into the sapling's tree itself, carried by an
/// upgrade rather than replayed: its manifest (the record's pin, the tables
/// it adds), its `.gitignore` lines, and the lock its `vendor/dna` pins.
const GRAFT_OWNED: &[&str] = &["hale.toml", ".gitignore", "hale.lock"];

/// The manifest at the new revision (`new`) with what the graft wrote into
/// the old one carried: each table the working manifest (`working`) holds
/// and its revision (`head`) does not, when the new revision holds none by
/// that name (one that declares its environments keeps its own, as a graft
/// keeps them), and the record's pin `id`.
pub(super) fn carried_manifest(head: &str, working: &str, new: &str, id: &str) -> Result<String, String> {
    let parse = |t: &str, what: &str| t.parse::<toml_edit::DocumentMut>().map_err(|e| format!("{what} hale.toml does not parse: {e}"));
    let head = parse(head, "the grafted revision's")?;
    let work = parse(working, "the checkout's")?;
    let mut doc = parse(new, "the new revision's")?;
    let declares_environments = doc.get("environments").is_some();
    for (k, item) in work.iter() {
        if k == "dna" || head.get(k).is_some() || doc.get(k).is_some() {
            continue;
        }
        if declares_environments && k == "claims" {
            continue;
        }
        doc.insert(k, item.clone());
    }
    crate::dna::pinned_manifest(&doc.to_string(), id)
}

/// The `.gitignore` at the new revision (`new`) with the lines the graft
/// added to the old one (`working`'s that `head` lacks) carried.
pub(super) fn carried_lines(head: &str, working: &str, new: &str) -> String {
    let had: BTreeSet<&str> = head.lines().map(str::trim).collect();
    let mut out = new.to_string();
    for l in working.lines().filter(|l| !l.trim().is_empty() && !had.contains(l.trim())) {
        if out.lines().any(|n| n.trim() == l.trim()) {
            continue;
        }
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(l);
        out.push('\n');
    }
    out
}

/// What the checkout carries to the new revision, read before anything
/// moves.
struct Carry {
    /// the graft-owned files as the checkout and its revision hold them
    owned: Vec<(&'static str, Option<String>, Option<String>)>,
    /// the checkout's other changes to tracked files, as a patch
    patch: Vec<u8>,
    /// the files that patch changes
    changed: Vec<String>,
}

/// The file `rel` at `rev` of `store`, `None` when the revision tracks none.
fn at_rev(store: &Path, rev: &str, rel: &str) -> Option<String> {
    git_in(store, &["show", &format!("{rev}:{rel}")]).ok()
}

/// What the checkout at `store` carries to `commit`, or why it cannot be
/// carried: a change of its own that does not apply at the new revision, or
/// a file it holds untracked that the new revision tracks otherwise.
fn carry_check(store: &Path, commit: &str, child: &str) -> Result<Carry, String> {
    let owned: Vec<(&'static str, Option<String>, Option<String>)> = GRAFT_OWNED.iter().map(|f| (*f, at_rev(store, "HEAD", f), std::fs::read_to_string(store.join(f)).ok())).collect();
    // the manifest holds nothing beyond what its graft wrote: carried onto
    // its own revision, it is the checkout's
    if let Some((_, head, Some(working))) = owned.iter().find(|(f, _, _)| *f == "hale.toml") {
        let head = head.clone().unwrap_or_default();
        let replayed = carried_manifest(&head, working, &head, child)?;
        let same = |t: &str| t.parse::<toml::Table>().ok();
        if same(&replayed) != same(working) {
            return Err(format!("{}/hale.toml has changes beyond its graft's (its record's pin and the tables the graft added): the upgrade carries only those; commit or drop the rest there first, nothing moved", store.display()));
        }
    }
    let mut diff = vec!["diff".to_string(), "--binary".to_string(), "HEAD".to_string(), "--".to_string(), ".".to_string()];
    diff.extend(GRAFT_OWNED.iter().map(|f| format!(":(exclude){f}")));
    let diff_args: Vec<&str> = diff.iter().map(String::as_str).collect();
    let patch = Command::new("git").arg("-C").arg(store).args(&diff_args).output().map_err(|e| format!("git: {e}"))?.stdout;
    let mut names = diff.clone();
    names.insert(1, "--name-only".to_string());
    let name_args: Vec<&str> = names.iter().map(String::as_str).collect();
    let changed: Vec<String> = git_in(store, &name_args)?.lines().filter(|l| !l.is_empty()).map(str::to_string).collect();
    if !patch.is_empty() {
        // applied to the new revision's tree in an index of its own, never
        // the checkout's
        let index = store.with_file_name(format!(".{}.upgrade-index-{}", store.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), std::process::id()));
        let checked = git_with(store, &["read-tree", commit], b"", Some(&index)).and_then(|_| git_with(store, &["apply", "--cached", "--check", "-"], &patch, Some(&index)));
        let _ = std::fs::remove_file(&index);
        checked.map_err(|e| format!("the checkout's own changes ({}) do not apply at the new revision ({e}): commit or drop them in {} first, nothing moved", changed.join(", "), store.display()))?;
    }
    // a file the checkout holds untracked (the graft's organization, its
    // vendor/dna, its state) that the new revision tracks with other bytes
    let untracked: BTreeSet<String> = git_in(store, &["ls-files", "--others"])?.lines().map(str::to_string).collect();
    let mut in_way = Vec::new();
    for l in git_in(store, &["ls-tree", "-r", commit])?.lines() {
        let Some((meta, path)) = l.split_once('\t') else { continue };
        if !untracked.contains(path) || GRAFT_OWNED.contains(&path) {
            continue;
        }
        let blob = meta.split_whitespace().nth(2).unwrap_or("");
        let here = git_in(store, &["hash-object", "--", path]).unwrap_or_default();
        if here.trim() != blob {
            in_way.push(path.to_string());
        }
    }
    if !in_way.is_empty() {
        return Err(format!("the new revision tracks {}, which the checkout holds untracked with other contents: move them aside in {} first, nothing moved", in_way.join(", "), store.display()));
    }
    Ok(Carry { owned, patch, changed })
}

/// The checkout at `store` moved to `commit`, carrying what `carry_check`
/// read; put back where it was when any step fails. What was done.
pub(super) fn move_tree(store: &Path, commit: &str, child: &str) -> Result<String, String> {
    let old = git_in(store, &["rev-parse", "HEAD"])?.trim().to_string();
    if old == commit {
        return Ok(format!("{:<8}{} is at {} already", "tree", store.display(), &commit[..12.min(commit.len())]));
    }
    let carry = carry_check(store, commit, child)?;
    let restore = |carry: &Carry| {
        let _ = git_in(store, &["checkout", "-q", "-f", "--detach", &old]);
        for (f, _, working) in &carry.owned {
            match working {
                Some(t) => {
                    let _ = std::fs::write(store.join(f), t);
                }
                None => {
                    let _ = std::fs::remove_file(store.join(f));
                }
            }
        }
        if !carry.patch.is_empty() {
            let _ = git_with(store, &["apply", "-"], &carry.patch, None);
        }
    };
    let moved = (|| -> Result<(), String> {
        git_in(store, &["checkout", "-q", "-f", "--detach", commit])?;
        for (f, head, working) in &carry.owned {
            let new = at_rev(store, commit, f);
            let text = match *f {
                "hale.toml" => Some(carried_manifest(head.as_deref().unwrap_or(""), working.as_deref().unwrap_or(""), new.as_deref().unwrap_or(""), child)?),
                ".gitignore" if working.is_some() => Some(carried_lines(head.as_deref().unwrap_or(""), working.as_deref().unwrap_or(""), new.as_deref().unwrap_or(""))),
                // the lock: the new revision's, or the checkout's own when
                // it tracks none; `vendor/dna` re-pins it
                _ if new.is_none() && head.is_some() => working.clone(),
                _ => None,
            };
            if let Some(t) = text {
                std::fs::write(store.join(f), t).map_err(|e| format!("write {}: {e}", store.join(f).display()))?;
            }
        }
        if !carry.patch.is_empty() {
            git_with(store, &["apply", "-"], &carry.patch, None)?;
        }
        match crate::dna::manifest_pin(&store.join("hale.toml"))? {
            Some(id) if id == child => Ok(()),
            other => Err(format!("hale.toml pins {} after the move, not the organism's record {child}", other.unwrap_or_else(|| "no record".to_string()))),
        }
    })();
    if let Err(e) = moved {
        restore(&carry);
        return Err(format!("{e}; the tree is put back at {}", &old[..12.min(old.len())]));
    }
    let kept = if carry.changed.is_empty() { String::new() } else { format!(" and its own changes ({})", carry.changed.join(", ")) };
    Ok(format!("{:<8}{} checked out at {}… (from {}…), its graft's record pin, tables and ignores carried{kept}", "tree", store.display(), &commit[..12.min(commit.len())], &old[..12.min(old.len())]))
}

/// The services allocation of the organism keyed `key`, its latest
/// `habitat.allocated <key>/services` body.
fn services_allocation(rows: &[Value], key: &str) -> Option<Value> {
    let entity = format!("{key}/services");
    rows.iter().filter(|r| r["kind"] == "habitat.allocated" && r["entity"] == entity.as_str()).last().map(body_of).filter(|b| b["host"].as_str().is_some_and(|h| !h.is_empty()))
}

/// The habitat's side of the upgrade (or, with `plan`, what it would do):
/// the manifest at the new revision, for the placements part.
fn habitat_side(s: &Standing, m: &Move, plan: bool) -> Result<String, String> {
    let key = s.child[..12.min(s.child.len())].to_string();
    let store = PathBuf::from(local_habitat_dir()).join("saplings").join(&s.sapling);
    let pin = s.pin.clone().unwrap_or_default();
    println!("{:<8}{}: {} -> {}, the revision the habitat pins for `{}`{}", "upgrade", s.organism, m.from.said(), m.to.said(), s.sapling, if plan { " (the plan: nothing moves)" } else { "" });
    let commit = fetch_pinned(&store, &s.sapling, &pin)?;
    println!("{:<8}{} at {} ({}…), its digest {} checked", "fetched", s.sapling, m.to.revision, &commit[..12.min(commit.len())], m.to.digest);
    let manifest = at_rev(&store, &commit, "hale.toml").unwrap_or_default();
    // what it needs at the new revision is what its graft allocated
    let needs = needs_of(Some(&manifest))?;
    let added: Vec<&String> = needs.iter().filter(|r| !s.needs.contains(r)).collect();
    if !added.is_empty() {
        return Err(format!("{} needs {}, which its graft did not allocate: an upgrade moves the tree and allocates nothing new (the organism is removed and grafted again to take them), nothing moved", m.to.revision, added.iter().map(|r| r.as_str()).collect::<Vec<_>>().join(", ")));
    }
    let rows = rows_of(&genome_dir());
    let services = services_allocation(&rows, &key);
    let root = crate::dna::organism_root(&store)?;
    if plan {
        carry_check(&store, &commit, &s.child)?;
        println!("{:<8}{} checked out at {}, its graft's record pin, tables and ignores carried", "tree", store.display(), m.to.revision);
        if root.join("vendor/dna").is_dir() {
            println!("{:<8}vendor/dna re-materialized for this toolchain, hale.lock re-pinned", "vendor");
        }
        let declared = heart_migrations_of(&manifest, &root)?;
        if declared.is_empty() {
            println!("{:<8}none declared at {}", "heart", m.to.revision);
        }
        for h in declared {
            println!("{:<8}the heart's {} migration runs (`{}`)", "heart", h.role, h.command.join(" "));
        }
        match &services {
            Some(b) => println!("{:<8}{key}/services re-applied on {} through its provider, its images built from {}", "apply", b["host"].as_str().unwrap_or(""), m.to.revision),
            None => println!("{:<8}no services allocation (its graft placed none on a host)", "apply"),
        }
        println!("{:<8}habitat.organism.upgraded {key}: {} -> {}", "record", m.from.revision, m.to.revision);
        return Ok(manifest);
    }
    super::progress(&format!("moving `{}` from {} to {}", s.organism, m.from.revision, m.to.revision));
    println!("{}", move_tree(&store, &commit, &s.child)?);
    // what a graft that goes on checks: the revision the tree is at now
    let checked = PathBuf::from(local_habitat_dir()).join("saplings").join(format!("{}.grafted.json", s.sapling));
    let mut kept = pin.clone();
    kept["commit"] = json!(commit);
    std::fs::write(&checked, format!("{kept}\n")).map_err(|e| format!("write {}: {e}", checked.display()))?;
    if root.join("vendor/dna").is_dir() {
        let (w, same) = crate::dna::materialize_vendor(&root)?;
        println!("{:<8}vendor/dna for this toolchain ({w} written, {same} unchanged), hale.lock re-pinned", "vendor");
    }
    let finish = |why: String| format!("{why}; the tree is at {} and the upgrade is not recorded: `hale habitat organism upgrade {}` again finishes it", m.to.revision, s.organism);
    super::progress("running the heart's migrations at the new revision");
    let ran = run_heart_migrations(&root).map_err(finish)?;
    if ran.is_empty() {
        println!("{:<8}none declared at {}", "heart", m.to.revision);
    }
    for l in ran {
        println!("{l}");
    }
    match services {
        Some(b) => {
            let host = b["host"].as_str().unwrap_or("").to_string();
            // what the services render ships may have moved with the tree
            let tracked = git_in(&root, &["ls-files"]).unwrap_or_default();
            let mounted = sapling::mounted_entries(&tracked).join("\n");
            if b["render_mounted"].as_str() != Some(mounted.as_str()) {
                let mut nb = b.clone();
                nb["render_mounted"] = json!(mounted);
                sapling::record(&[json!({ "kind": "habitat.allocated", "entity": format!("{key}/services"), "body": nb.to_string() })]).map_err(finish)?;
            }
            super::progress(&format!("re-applying {key}/services on {host}: its images are built from {}", m.to.revision));
            let lines = match sapling::head_call("Commands::apply_allocations", json!({ "host": host, "fresh": false })) {
                Ok(v) => {
                    let ok = v["ok"] == json!(true);
                    let lines = sapling::applied_lines(&v);
                    if !ok {
                        for l in &lines {
                            println!("{l}");
                        }
                        return Err(finish(format!("{key}/services was not applied on {host}")));
                    }
                    lines
                }
                Err(e) if e.contains("deadline") => sapling::follow_apply(&format!("{key}/services"), &host).map_err(finish)?,
                Err(e) => return Err(finish(e)),
            };
            for l in lines {
                println!("{l}");
            }
        }
        None => println!("{:<8}no services allocation (its graft placed none on a host)", "apply"),
    }
    let body = json!({ "organism": s.organism, "child": s.child, "sapling": s.sapling, "from": m.from.revision, "from_digest": m.from.digest, "to": m.to.revision, "digest": m.to.digest, "commit": commit });
    sapling::record(&[json!({ "kind": "habitat.organism.upgraded", "entity": key, "body": body.to_string() })]).map_err(finish)?;
    println!("{:<8}habitat.organism.upgraded {key}: {} -> {}, in the habitat's record", "record", m.from.revision, m.to.revision);
    Ok(manifest)
}

// ---------------------------------------------------------------
// the placements: from the person's machine
// ---------------------------------------------------------------

/// A placement to re-run at the new revision, and whether its command runs
/// as root (`None`: not known here, through a pointer).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Rerun {
    pub placement: placements::Placement,
    pub root: Option<bool>,
}

/// The placements of `organism` that hold, each with whether its command
/// runs as root at the new revision, as `manifest` declares it.
pub(super) fn reruns(current: &[placements::Placement], organism: &str, manifest: Option<&str>) -> Vec<Rerun> {
    let declared = manifest.and_then(|m| hosted::host_commands(m).ok());
    current.iter().filter(|p| p.organism == organism && p.placed).map(|p| Rerun { placement: p.clone(), root: declared.as_ref().map(|d| d.get(&p.command).is_some_and(|c| c.root)) }).collect()
}

/// The `place` that re-runs `p`'s install.
pub(super) fn place_command(p: &placements::Placement) -> String {
    format!("hale habitat organism place {} {} --host {} --id {}", p.organism, p.command, p.host, p.id)
}

/// Whether `r` can run here: at a terminal anything can (sudo asks there),
/// off one only a command known not to run as root.
pub(super) fn runs_here(r: &Rerun, tty: bool) -> bool {
    tty || r.root == Some(false)
}

/// The plan's line for re-running `r`.
pub(super) fn rerun_line(r: &Rerun) -> String {
    let p = &r.placement;
    let how = match r.root {
        Some(true) => "as root, through your sudo: it needs your terminal",
        Some(false) => "as your login",
        None => "a command that runs as root asks for your sudo at a terminal",
    };
    format!("{:<8}{}'s `{}` {} on {}: its install re-run at the new revision, over your SSH login ({how})", "place", p.organism, p.command, p.id, p.host)
}

/// The name of the organism named or keyed `organism`, as the habitat's
/// head lists it (a placement names its organism by name).
fn named(organism: &str) -> String {
    let index = sapling::head_call("Commands::index", json!({ "limit": 500 })).unwrap_or_default();
    let organisms: Vec<Value> = index["organisms"].as_str().and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
    organisms
        .iter()
        .find(|o| o["name"] == organism)
        .or_else(|| organisms.iter().find(|o| organism.len() >= 12 && o["key"].as_str().is_some_and(|k| k.starts_with(organism))))
        .and_then(|o| o["name"].as_str())
        .unwrap_or(organism)
        .to_string()
}

/// Each host command placed for `organism` re-run at the revision now
/// pinned (or, with `plan`, listed): at a terminal, each; off one, those
/// that need none, and the `place` lines left to run.
fn placements_part(organism: &str, manifest: Option<&str>, plan: bool) -> ExitCode {
    let current = match placements::from_head() {
        Ok((c, _)) => c,
        Err(why) => {
            eprintln!("hale habitat organism upgrade {organism}: its placements were not read: {why}");
            return ExitCode::from(1);
        }
    };
    let list = reruns(&current, organism, manifest);
    if list.is_empty() {
        println!("{:<8}none: no host command of {organism}'s is placed (`hale habitat organism place`)", "place");
        return ExitCode::SUCCESS;
    }
    if plan {
        for r in &list {
            println!("{}", rerun_line(r));
        }
        return ExitCode::SUCCESS;
    }
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let mut left = Vec::new();
    let mut failed = false;
    for r in &list {
        let p = &r.placement;
        if !runs_here(r, tty) {
            left.push(place_command(p));
            continue;
        }
        if let Err(why) = hosted::place_lines(&p.organism, &p.command, &p.host, Some(&p.id), false, true) {
            println!("{:<8}{}'s `{}` {} on {}: {why}", "left", p.organism, p.command, p.id, p.host);
            left.push(place_command(p));
            failed = true;
        }
    }
    if !left.is_empty() {
        println!("{:<8}what is left to run, at a terminal (sudo asks there for a command that runs as root):", "place");
        for l in &left {
            println!("  {l}");
        }
    }
    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHILD: &str = "0123456789abcdef0123456789abcdef01234567";

    fn row(kind: &str, entity: &str, body: Value) -> Value {
        json!({ "kind": kind, "entity": entity, "body": body.to_string() })
    }

    fn pin(rev: &str, digest: &str) -> Value {
        json!({ "name": "sprout", "repo": "/src/sprout", "revision": rev, "digest": digest, "nursery": "/n", "packs": [] })
    }

    fn grafted() -> Value {
        row("habitat.grafted", &CHILD[..12], json!({ "child": CHILD, "name": "sprout", "needs": ["memory", "nerves"], "sapling": "sprout", "revision": "v1", "digest": "sha256:1", "nursery": "/n" }))
    }

    fn words(line: &str) -> Vec<String> {
        line.split(' ').map(str::to_string).collect()
    }

    /// The organism, then `--plan` (and the pointer's `--no-placements`)
    /// anywhere; nothing else.
    #[test]
    fn its_arguments() {
        assert_eq!(parse(&words("sprout")), Some(Args { organism: "sprout".into(), plan: false, no_placements: false }));
        assert_eq!(parse(&words("--plan sprout")), Some(Args { organism: "sprout".into(), plan: true, no_placements: false }));
        assert_eq!(parse(&words("sprout --no-placements --plan")), Some(Args { organism: "sprout".into(), plan: true, no_placements: true }));
        assert_eq!(parse(&words("--plan")), None, "an organism");
        assert_eq!(parse(&words("sprout bud")), None, "one organism");
        assert_eq!(parse(&words("sprout --fresh")), None, "a flag it does not take");
    }

    /// The pin moved past the organism's revision: it moves from where its
    /// graft (or its last upgrade) left it to the pin.
    #[test]
    fn a_ratified_pin_is_a_move() {
        let rows = vec![row("habitat.sapling", "sprout", pin("v1", "sha256:1")), grafted(), row("habitat.sapling.proposed", "sprout", pin("v2", "sha256:2")), row("habitat.sapling", "sprout", pin("v2", "sha256:2"))];
        let s = standing(&rows, "sprout").unwrap();
        assert_eq!((s.organism.as_str(), s.sapling.as_str(), s.child.as_str()), ("sprout", "sprout", CHILD));
        assert_eq!(s.proposed, None, "the proposal is the pin now");
        let m = decide(&s).unwrap();
        assert_eq!(m.from, Rev { revision: "v1".into(), digest: "sha256:1".into() });
        assert_eq!(m.to, Rev { revision: "v2".into(), digest: "sha256:2".into() });
        // named by its key too
        assert_eq!(decide(&standing(&rows, &CHILD[..12]).unwrap()).unwrap(), m);
    }

    /// At the pinned tree already: refused, saying so; a newer revision
    /// filed and not ratified is named, with the ratify that pins it.
    #[test]
    fn at_the_pin_it_is_refused() {
        let mut rows = vec![row("habitat.sapling", "sprout", pin("v1", "sha256:1")), grafted()];
        let why = decide(&standing(&rows, "sprout").unwrap()).unwrap_err();
        assert!(why.contains("already at v1 (sha256:1)") && !why.contains("proposed"), "{why}");
        rows.push(row("habitat.sapling.proposed", "sprout", pin("v2", "sha256:2")));
        let why = decide(&standing(&rows, "sprout").unwrap()).unwrap_err();
        assert!(why.contains("already at v1") && why.contains("v2 is proposed and not ratified") && why.contains("hale habitat nursery ratify sprout"), "{why}");
        // ratified and upgraded: at the pin again
        rows.push(row("habitat.sapling", "sprout", pin("v2", "sha256:2")));
        rows.push(row("habitat.organism.upgraded", &CHILD[..12], json!({ "organism": "sprout", "from": "v1", "to": "v2", "digest": "sha256:2" })));
        let s = standing(&rows, "sprout").unwrap();
        assert_eq!(s.at, Rev { revision: "v2".into(), digest: "sha256:2".into() });
        assert!(decide(&s).unwrap_err().contains("already at v2 (sha256:2)"));
        // a revision renamed onto another tree is the tree: it moves
        rows.push(row("habitat.sapling", "sprout", pin("v2", "sha256:2b")));
        assert_eq!(decide(&standing(&rows, "sprout").unwrap()).unwrap().to.digest, "sha256:2b");
    }

    /// No pin, no sapling, no such organism: each refused by name.
    #[test]
    fn what_has_no_upgrade() {
        let why = decide(&standing(&[grafted()], "sprout").unwrap()).unwrap_err();
        assert!(why.contains("pins no sapling `sprout` any more"), "{why}");
        let dir = row("habitat.grafted", "aaaaaaaaaaaa", json!({ "child": "aaaaaaaaaaaa00", "name": "shop", "needs": ["memory", "nerves"] }));
        assert!(standing(&[dir], "shop").unwrap_err().contains("was not grafted from a sapling"));
        assert!(standing(&[grafted()], "bud").unwrap_err().contains("grafted no organism `bud`"));
        let removed = row("habitat.grafted", &CHILD[..12], json!({ "child": CHILD, "name": "sprout", "state": "removed" }));
        assert!(standing(&[grafted(), removed], "sprout").unwrap_err().contains("grafted no organism"));
    }

    /// The manifest at the new revision keeps the graft's record pin and
    /// the tables it added; one the new revision declares is its own.
    #[test]
    fn the_grafts_manifest_is_carried() {
        let head = "[dna]\nneeds = [\"memory\", \"nerves\"]\n";
        let working = "[dna]\nneeds = [\"memory\", \"nerves\"]\nrecord = \"abc\"\n\n[claims]\nno_base = true\n\n[environments.org]\nsource_only = true\nentrypoints = [\"dna/org\"]\n";
        let new = "[deps]\nx = \"1\"\n\n[dna]\nneeds = [\"memory\", \"nerves\"]\n\n[dna.migrations.database]\ncommand = [\"m\"]\n";
        let out: toml::Table = carried_manifest(head, working, new, "abc").unwrap().parse().unwrap();
        assert_eq!(out["dna"]["record"].as_str(), Some("abc"));
        assert!(out["dna"]["migrations"]["database"].is_table(), "the new revision's own");
        assert_eq!(out["deps"]["x"].as_str(), Some("1"));
        assert_eq!(out["claims"]["no_base"].as_bool(), Some(true));
        assert!(out["environments"]["org"].is_table());
        // a new revision that declares its environments keeps its own
        let declares = "[dna]\nneeds = [\"memory\", \"nerves\"]\n\n[environments.web]\nentrypoints = [\"web\"]\n";
        let out: toml::Table = carried_manifest(head, working, declares, "abc").unwrap().parse().unwrap();
        assert!(out["environments"].get("org").is_none() && out.get("claims").is_none(), "{out:?}");
        // the ignores the graft added, once
        assert_eq!(carried_lines("target\n", "target\n/vendor/\n/.hale/\n", "target\n/.hale/\nnew\n"), "target\n/.hale/\nnew\n/vendor/\n");
        assert_eq!(carried_lines("", "/vendor/\n", ""), "/vendor/\n");
    }

    /// The placements of the organism that hold are re-run, each by the
    /// `place` that placed it; off a terminal only one known not to need root.
    #[test]
    fn the_placements_to_rerun() {
        let p = |org: &str, cmd: &str, host: &str, placed: bool| placements::Placement { organism: org.into(), command: cmd.into(), host: host.into(), id: host.into(), placed, ..Default::default() };
        let current = vec![p("voice", "node", "gpu-1", true), p("voice", "relay", "mac-1", true), p("tts", "node", "gpu-1", true), p("voice", "node", "old-1", false)];
        let manifest = "[dna.host.node]\nroot = true\ninstall = [\"i\"]\nuninstall = [\"u\"]\n\n[dna.host.relay]\ninstall = [\"i\"]\nuninstall = [\"u\"]\n";
        let r = reruns(&current, "voice", Some(manifest));
        assert_eq!(r.iter().map(|r| (r.placement.host.as_str(), r.root)).collect::<Vec<_>>(), vec![("gpu-1", Some(true)), ("mac-1", Some(false))]);
        assert_eq!(place_command(&r[0].placement), "hale habitat organism place voice node --host gpu-1 --id gpu-1");
        assert!(runs_here(&r[0], true) && !runs_here(&r[0], false), "root needs the terminal");
        assert!(runs_here(&r[1], false), "a login's own command runs off one");
        assert!(rerun_line(&r[0]).contains("it needs your terminal"));
        // through a pointer the declaration is not here: a terminal is asked for
        let blind = reruns(&current, "voice", None);
        assert!(blind.iter().all(|r| r.root.is_none() && !runs_here(r, false)));
        assert!(reruns(&current, "shop", Some(manifest)).is_empty());
    }

    fn git(d: &Path, args: &[&str]) -> String {
        let o = Command::new("git").arg("-C").arg(d).args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"]).args(args).output().unwrap();
        assert!(o.status.success(), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    /// A sapling's source at v1 and v2, and its store grafted at v1 with
    /// what a graft writes there: the record pinned in its manifest, the
    /// tables and ignores it adds, its organization and vendor/dna untracked.
    fn grafted_store(tag: &str) -> (PathBuf, PathBuf, String) {
        let base = std::env::temp_dir().join(format!("hale-upgrade-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let src = base.join("src");
        std::fs::create_dir_all(&src).unwrap();
        git(&src, &["init", "-q", "-b", "main"]);
        std::fs::write(src.join("hale.toml"), "[dna]\nneeds = [\"memory\", \"nerves\"]\n").unwrap();
        std::fs::write(src.join("README.md"), "v1\n").unwrap();
        git(&src, &["add", "-A"]);
        git(&src, &["commit", "-q", "-m", "v1"]);
        git(&src, &["tag", "v1"]);
        std::fs::write(src.join("hale.toml"), "[dna]\nneeds = [\"memory\", \"nerves\"]\n\n[dna.host.node]\ninstall = [\"i\"]\nuninstall = [\"u\"]\n").unwrap();
        std::fs::write(src.join("README.md"), "v2\n").unwrap();
        std::fs::write(src.join("CHANGELOG.md"), "v2\n").unwrap();
        git(&src, &["add", "-A"]);
        git(&src, &["commit", "-q", "-m", "v2"]);
        git(&src, &["tag", "v2"]);
        let store = base.join("saplings").join("sprout");
        std::fs::create_dir_all(store.parent().unwrap()).unwrap();
        git(&base, &["clone", "-q", "--no-checkout", &src.to_string_lossy(), &store.to_string_lossy()]);
        git(&store, &["checkout", "-q", "--detach", "v1"]);
        std::fs::write(store.join("hale.toml"), format!("[dna]\nneeds = [\"memory\", \"nerves\"]\nrecord = \"{CHILD}\"\n\n[claims]\nno_base = true\n\n[environments.org]\nsource_only = true\nentrypoints = [\"dna/org\"]\n")).unwrap();
        std::fs::write(store.join(".gitignore"), "/vendor/\n/.hale/\n").unwrap();
        std::fs::create_dir_all(store.join("dna/org")).unwrap();
        std::fs::write(store.join("dna/org/purpose.hl"), "// the organism's own\n").unwrap();
        std::fs::create_dir_all(store.join("vendor/dna")).unwrap();
        std::fs::write(store.join("vendor/dna/core.hl"), "// the toolchain's\n").unwrap();
        (base, store, src.to_string_lossy().to_string())
    }

    fn pinned_at(src: &str, store: &Path, rev: &str) -> Value {
        let (_, d) = crate::dna::pack::digest_at(store, rev).unwrap();
        json!({ "repo": src, "revision": rev, "digest": d, "nursery": "/n" })
    }

    /// The pinned revision is fetched and checked against its digest: a
    /// tree that is not the pinned one is refused, and the checkout stays.
    #[test]
    fn a_digest_mismatch_moves_nothing() {
        let (base, store, src) = grafted_store("digest");
        let mut p = pinned_at(&src, &PathBuf::from(&src), "v2");
        let good = p["digest"].as_str().unwrap().to_string();
        p["digest"] = json!("sha256:0000");
        let why = fetch_pinned(&store, "sprout", &p).unwrap_err();
        assert!(why.contains("not the sha256:0000 the habitat pins: refused, nothing moved") && why.contains(&good), "{why}");
        assert_eq!(git(&store, &["rev-parse", "HEAD"]), git(&PathBuf::from(&src), &["rev-parse", "v1"]), "nothing moved");
        p["digest"] = json!(good);
        assert_eq!(fetch_pinned(&store, "sprout", &p).unwrap(), git(&PathBuf::from(&src), &["rev-parse", "v2"]));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The tree moves to the new revision with what its graft wrote carried
    /// and what it holds untracked kept; a second move is nothing.
    #[test]
    fn the_tree_moves_and_keeps_its_graft() {
        let (base, store, src) = grafted_store("move");
        let commit = fetch_pinned(&store, "sprout", &pinned_at(&src, &PathBuf::from(&src), "v2")).unwrap();
        let said = move_tree(&store, &commit, CHILD).unwrap();
        assert!(said.contains("checked out at"), "{said}");
        assert_eq!(git(&store, &["rev-parse", "HEAD"]), commit);
        assert_eq!(std::fs::read_to_string(store.join("CHANGELOG.md")).unwrap(), "v2\n");
        let m: toml::Table = std::fs::read_to_string(store.join("hale.toml")).unwrap().parse().unwrap();
        assert_eq!(m["dna"]["record"].as_str(), Some(CHILD), "the record's pin");
        assert!(m["dna"]["host"]["node"].is_table() && m["environments"]["org"].is_table(), "{m:?}");
        assert_eq!(std::fs::read_to_string(store.join(".gitignore")).unwrap(), "/vendor/\n/.hale/\n");
        assert!(store.join("dna/org/purpose.hl").is_file() && store.join("vendor/dna/core.hl").is_file(), "untracked: kept");
        assert!(move_tree(&store, &commit, CHILD).unwrap().contains("already"));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A change of the checkout's own that the new revision also changes,
    /// or an untracked file the new revision tracks otherwise, is refused
    /// before anything moves; one that applies is carried.
    #[test]
    fn what_cannot_be_carried_is_refused() {
        let (base, store, src) = grafted_store("refuse");
        let v1 = git(&store, &["rev-parse", "HEAD"]);
        let commit = fetch_pinned(&store, "sprout", &pinned_at(&src, &PathBuf::from(&src), "v2")).unwrap();
        std::fs::write(store.join("README.md"), "mine\n").unwrap();
        let why = move_tree(&store, &commit, CHILD).unwrap_err();
        assert!(why.contains("README.md") && why.contains("nothing moved"), "{why}");
        assert_eq!(git(&store, &["rev-parse", "HEAD"]), v1);
        assert_eq!(std::fs::read_to_string(store.join("README.md")).unwrap(), "mine\n");
        git(&store, &["checkout", "-q", "--", "README.md"]);
        std::fs::write(store.join("CHANGELOG.md"), "not v2's\n").unwrap();
        let why = move_tree(&store, &commit, CHILD).unwrap_err();
        assert!(why.contains("tracks CHANGELOG.md"), "{why}");
        assert_eq!(git(&store, &["rev-parse", "HEAD"]), v1);
        // the same bytes as the new revision's are no conflict
        std::fs::write(store.join("CHANGELOG.md"), "v2\n").unwrap();
        // a manifest edited beyond its graft is not carried silently
        let m = std::fs::read_to_string(store.join("hale.toml")).unwrap();
        std::fs::write(store.join("hale.toml"), m.replace("[claims]", "[deps]\nlocal = \"1\"\n\n[claims]").replace("needs = [\"memory\", \"nerves\"]", "needs = [\"memory\", \"nerves\", \"senses\"]")).unwrap();
        assert!(move_tree(&store, &commit, CHILD).unwrap_err().contains("changes beyond its graft's"));
        std::fs::write(store.join("hale.toml"), &m).unwrap();
        let _ = std::fs::remove_dir_all(&base);
    }
}
