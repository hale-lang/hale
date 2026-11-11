//! `hale habitat forge publish <organism> [--branch <name>] [--dry-run]`:
//! the organism's main at the habitat's Forgejo carried to GitHub as one
//! pull request.
//!
//! Development lands and is reviewed at the habitat's own Forgejo; GitHub is
//! a sink, fed by this explicit, batched step, which goes out as the owner's
//! push. One publish is one pull request: the internal main is fetched,
//! pushed to GitHub as the branch `sync/<YYYY-MM-DD>` (UTC; `--branch` names
//! another), and one pull request against GitHub's `main` is opened, its
//! body the internal pull requests merged since the last publish (their
//! titles and internal numbers, no link into the private forge). A second
//! publish the same day moves that branch forward and updates its pull
//! request rather than opening another.
//!
//! "Since the last publish" is read from git, not kept: a merged internal
//! pull request is listed when its merge commit is on the internal main and
//! not on GitHub's, so the first publish lists everything since GitHub's
//! main, and nothing is recorded that could drift from what GitHub holds.
//!
//! Nothing is ever force-pushed, and the internal main is never touched.
//! When GitHub's main has commits the internal main lacks (its merge queue
//! merges a publish's pull request with a merge commit), the branch pushed
//! is a merge of the two, made in the publish's scratch repository and
//! authored as the owner the GitHub token belongs to (`/user`); so is a
//! merge with the branch an earlier publish the same day left at GitHub.
//! A merge that conflicts stops the publish, naming the files.
//!
//! A habitat on another machine publishes there: the verb runs in its
//! toolchain image (`remote::run_there`), where its vault and its Forgejo
//! are.
//!
//! Where it goes is the `habitat.publish <id>` row `hale habitat forge
//! <organism> github --repo <owner/name> --publish` records (beside the
//! organism's forge, which stays as allocated); the GitHub token is the
//! vault slot that row names (`forge-token` by default, as a GitHub forge's),
//! the Forgejo token the organism's own (`forgejo-<id>`). A token is handed
//! to git through a credential helper reading it from git's own environment
//! and to curl in a config read from its stdin: never on a command line.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use super::*;

const USAGE: &str = "usage: hale habitat forge publish <organism> [--branch <name>] [--dry-run]";

/// The variable git's credential helper reads the token from: set on the
/// one `git` that needs it, never in this process.
const TOKEN_ENV: &str = "HALE_FORGE_PUBLISH_TOKEN";

/// How many pull requests one page of Forgejo's listing asks for.
const PAGE: usize = 50;

/// `hale habitat forge publish` arguments.
#[derive(Debug, PartialEq)]
struct PublishArgs {
    organism: String,
    branch: Option<String>,
    dry_run: bool,
}

fn publish_args(args: &[String]) -> Option<PublishArgs> {
    let (mut branch, mut dry_run, mut words) = (None, false, Vec::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--branch" => branch = Some(it.next()?.clone()),
            "--dry-run" => dry_run = true,
            w if w.starts_with("--") => return None,
            w => words.push(w.to_string()),
        }
    }
    let [organism] = <[String; 1]>::try_from(words).ok()?;
    Some(PublishArgs { organism, branch, dry_run })
}

pub(super) fn cmd(args: &[String]) -> ExitCode {
    let Some(PublishArgs { organism, branch, dry_run }) = publish_args(args) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    // a habitat on another machine: the publish runs there, in its
    // toolchain image, where its vault and its Forgejo are
    if let Some(p) = super::super::remote::pointer() {
        let mut there: Vec<&str> = vec!["forge", "publish"];
        there.extend(args.iter().map(String::as_str));
        return super::super::remote::run_there(&p, "forge", &there);
    }
    let run = || -> Result<Vec<String>, String> {
        let rows = rows_of(&genome_dir());
        let child = organism_child(&rows, &organism)?;
        let vault = crate::dna::vault_local_dir().ok_or("the vault is served over HTTP (HALE_VAULT_ADDR): a publish reads its tokens from the habitat's own vault alone")?;
        let read = |slot: &str| std::fs::read_to_string(vault.join(slot)).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        let (internal, github) = ends_from(&rows, &organism, &child[..12], &read)?;
        let date = today();
        let branch = branch.unwrap_or_else(|| format!("sync/{date}"));
        publish(&internal, &github, &branch, &date, dry_run)
    };
    match run() {
        Ok(lines) => {
            for l in lines {
                println!("{l}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("hale habitat forge publish: {e}");
            ExitCode::from(1)
        }
    }
}

/// The organism's repository at the habitat's Forgejo.
struct Internal {
    git_url: String,
    api: String,
    repo: String,
    user: String,
    token: String,
}

/// The organism's repository at GitHub.
struct GitHub {
    git_url: String,
    api: String,
    repo: String,
    token: String,
}

/// Both ends of a publish, from the habitat's record `rows` for the organism
/// keyed `key`, with `token` reading a vault slot: or why it cannot go.
fn ends_from(rows: &[serde_json::Value], organism: &str, key: &str, token: &dyn Fn(&str) -> Option<String>) -> Result<(Internal, GitHub), String> {
    let forge = latest(rows, "habitat.allocated").remove(&format!("{key}/forge")).unwrap_or_default();
    let field = |b: &serde_json::Value, k: &str| b[k].as_str().unwrap_or("").to_string();
    let (provider, address, repo, slot) = (field(&forge, "provider"), field(&forge, "address"), field(&forge, "repo"), field(&forge, "key_vault"));
    if provider != "forgejo" || address.is_empty() || repo.is_empty() {
        let is = if provider.is_empty() { "none".to_string() } else { provider };
        return Err(format!("`{organism}` has no Forgejo allocation (its forge is {is}): a publish carries the main of the organism's repository at the habitat's Forgejo; `hale habitat forge {organism} forgejo` allocates one"));
    }
    let target = latest(rows, "habitat.publish").remove(key).unwrap_or_default();
    let (to, to_slot) = (field(&target, "repo"), field(&target, "key_vault"));
    if field(&target, "provider") != "github" || to.is_empty() {
        return Err(format!("`{organism}` names no GitHub repository to publish to: `hale habitat forge {organism} github --repo <owner/name> --publish` names one, its token the vault slot `forge-token` (or `--key-vault <slot>`)"));
    }
    let to_slot = if to_slot.is_empty() { "forge-token".to_string() } else { to_slot };
    let Some(github_token) = token(&to_slot) else {
        return Err(format!("the vault holds no GitHub token in the slot `{to_slot}` (`hale dna secret set FORGE_TOKEN` fills `forge-token`): nothing was pushed"));
    };
    let Some(internal_token) = token(&slot) else {
        return Err(format!("the vault holds no token in the slot `{slot}`, {organism}'s own at the habitat's Forgejo: nothing was pushed"));
    };
    let user = repo.split('/').next().unwrap_or("").to_string();
    let address = address.trim_end_matches('/');
    Ok((
        Internal { git_url: format!("{address}/{repo}.git"), api: format!("{address}/api/v1"), repo, user, token: internal_token },
        GitHub { git_url: format!("https://github.com/{to}.git"), api: "https://api.github.com".to_string(), repo: to, token: github_token },
    ))
}

/// An internal pull request merged into the internal main.
#[derive(Debug, Clone, PartialEq)]
struct Merged {
    number: u64,
    title: String,
    merge: String,
}

/// One publish from `internal` to `github` as `branch`, the pull request
/// titled for `date`; with `dry_run`, what it would push and the pull
/// request's body, pushing and opening nothing. What was done, a line each.
fn publish(internal: &Internal, github: &GitHub, branch: &str, date: &str, dry_run: bool) -> Result<Vec<String>, String> {
    if branch == "main" || !Command::new("git").args(["check-ref-format", "--branch", branch]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success()) {
        return Err(format!("`--branch {branch}` names no branch a publish can push (and never `main`)"));
    }
    let scratch = Scratch::new()?;
    let dir = scratch.0.as_path();
    git(dir, None, &["init", "-q", "--bare"])?;
    let internal_auth = Some((internal.user.as_str(), internal.token.as_str()));
    let github_auth = Some(("x-access-token", github.token.as_str()));
    // the internal main
    git(dir, internal_auth, &["fetch", "-q", "--no-tags", &internal.git_url, "+refs/heads/main:refs/internal/main"]).map_err(|e| format!("the internal main could not be fetched from {}: {e}", internal.repo))?;
    let ours = git(dir, None, &["rev-parse", "refs/internal/main"])?;
    // GitHub's main, and the branch when it is there already
    let heads = git(dir, github_auth, &["ls-remote", &github.git_url, "refs/heads/main", &format!("refs/heads/{branch}")]).map_err(|e| format!("GitHub's {} could not be read: {e}", github.repo))?;
    let at = |r: &str| heads.lines().find_map(|l| l.split_once('\t').filter(|(_, name)| *name == r).map(|(sha, _)| sha.to_string()));
    let Some(theirs) = at("refs/heads/main") else {
        return Err(format!("GitHub's {} has no `main` for a pull request to go against: push its first commit there yourself", github.repo));
    };
    git(dir, github_auth, &["fetch", "-q", "--no-tags", &github.git_url, "+refs/heads/main:refs/github/main"]).map_err(|e| format!("GitHub's main could not be fetched from {}: {e}", github.repo))?;
    // GitHub's main holds the internal main already (equal, or a merge
    // made there of a publish's pull request): nothing is new
    if is_ancestor(dir, &ours, &theirs)? {
        return Ok(vec![format!("{:<8}nothing to publish: GitHub's main ({}) holds the internal main ({})", "publish", short(&theirs), short(&ours))]);
    }
    let range = git(dir, None, &["rev-list", "--reverse", "--topo-order", &format!("{theirs}..{ours}")])?;
    let range: Vec<&str> = range.lines().collect();
    // the branch at GitHub, when it is there already (an earlier publish
    // the same day): what is pushed contains it, so it moves forward only
    let branch_at = at(&format!("refs/heads/{branch}"));
    if branch_at.as_deref().is_some_and(|s| s != ours) {
        git(dir, github_auth, &["fetch", "-q", "--no-tags", &github.git_url, &format!("+refs/heads/{branch}:refs/github/branch")]).map_err(|e| format!("GitHub's {branch} could not be fetched: {e}"))?;
    }
    // What is pushed: the internal main, merged with the branch already at
    // GitHub and with GitHub's main where it does not contain them (a merge
    // GitHub's merge queue made is a commit the internal main lacks). The
    // merges are made here, in the scratch repository, as the owner: the
    // internal main is never touched, and nothing is forced.
    let mut target = ours.clone();
    let mut merges: Vec<String> = Vec::new();
    let mut owner: Option<(String, String)> = None;
    for (other, what) in [(branch_at.clone(), format!("GitHub's {branch}")), (Some(theirs.clone()), "GitHub's main".to_string())] {
        let Some(other) = other else { continue };
        if is_ancestor(dir, &other, &target)? {
            continue;
        }
        if is_ancestor(dir, &target, &other)? {
            target = other;
            continue;
        }
        if owner.is_none() {
            owner = Some(github_identity(github)?);
        }
        let (name, email) = owner.clone().unwrap_or_default();
        let message = format!("Merge {what} ({}) into the internal main ({}) for publish {date}", short(&other), short(&ours));
        target = merge(dir, &target, &other, &name, &email, &message).map_err(|files| {
            format!("{what} ({}) and the internal main ({}) conflict in {}: a publish never force-pushes and never resolves a conflict, so it stops here; take {what} into the internal forge and resolve it there, then publish again", short(&other), short(&ours), files.join(", "))
        })?;
        merges.push(format!("{:<8}{what} {} into the internal main as {} (by {name} <{email}>)", "merged", short(&other), short(&target)));
    }
    let merged: Vec<Merged> = {
        let mut m = merged_pulls(internal)?;
        let place = |sha: &str| range.iter().position(|c| *c == sha);
        m.retain(|p| place(&p.merge).is_some());
        m.sort_by_key(|p| (place(&p.merge), p.number));
        m
    };
    let title = format!("Publish {date}");
    let body = pr_body(&ours, &theirs, range.len(), &merged, target != ours);
    let open = open_pull(github, branch)?;
    let mut out = vec![format!("{:<8}the internal main {}, {} GitHub's main {} lacks", "publish", short(&ours), commits(range.len()), short(&theirs))];
    if dry_run {
        out.extend(merges.iter().map(|m| format!("{:<8}would have {}", "dry-run", m.trim_start_matches("merged").trim_start())));
        out.push(match branch_at.as_deref() {
            Some(sha) if sha == target => format!("{:<8}GitHub's {branch} is at {} already: nothing to push", "dry-run", short(&target)),
            _ => format!("{:<8}would push {} to github {} as {branch}", "dry-run", short(&target), github.repo),
        });
        out.push(match &open {
            Some((_, url)) => format!("{:<8}would update {url}", "dry-run"),
            None => format!("{:<8}would open a pull request against main", "dry-run"),
        });
        out.push(String::new());
        out.push(format!("title: {title}"));
        out.push(String::new());
        out.push(body.trim_end().to_string());
        out.push(String::new());
        out.push(format!("{:<8}nothing was pushed", "dry-run"));
        return Ok(out);
    }
    out.extend(merges);
    if branch_at.as_deref() == Some(target.as_str()) {
        out.push(format!("{:<8}GitHub's {branch} is at {} already", "pushed", short(&target)));
    } else {
        git(dir, github_auth, &["push", "-q", &github.git_url, &format!("{target}:refs/heads/{branch}")]).map_err(|e| format!("the push to GitHub's {} failed (nothing was forced): {e}", github.repo))?;
        out.push(format!("{:<8}{} to github {} {branch}", "pushed", short(&target), github.repo));
    }
    match open {
        Some((number, url)) => {
            let (status, said) = api("PATCH", &format!("{}/repos/{}/pulls/{number}", github.api, github.repo), &bearer(github), Some(&serde_json::json!({ "title": title, "body": body }).to_string()))?;
            if status != 200 {
                return Err(format!("the branch is pushed, but GitHub answered {status} updating {url}: {}", message(&said)));
            }
            out.push(format!("{:<8}{url}", "updated"));
        }
        None => {
            let (status, said) = api("POST", &format!("{}/repos/{}/pulls", github.api, github.repo), &bearer(github), Some(&serde_json::json!({ "title": title, "head": branch, "base": "main", "body": body }).to_string()))?;
            if status != 201 {
                return Err(format!("the branch is pushed, but GitHub answered {status} opening its pull request: {}", message(&said)));
            }
            let made: serde_json::Value = serde_json::from_str(&said).unwrap_or_default();
            out.push(format!("{:<8}{}", "opened", made["html_url"].as_str().unwrap_or("a pull request against main")));
        }
    }
    Ok(out)
}

/// The pull request's body: what it carries and the internal pull requests
/// merged since the last publish, by title and internal number. No link into
/// the private forge, no `#<n>` GitHub would link to its own, and no
/// `@name` it would notify (`inert`). `carried`: the branch is a merge of
/// the internal main and this repository's main, which it lacked.
fn pr_body(ours: &str, theirs: &str, count: usize, merged: &[Merged], carried: bool) -> String {
    let mut b = format!("The internal forge's main at `{}`: {} this repository's main (`{}`) lacks.\n\n", short(ours), commits(count), short(theirs));
    if carried {
        b.push_str("This repository's main has commits the internal main lacks (a merge made here), so the branch merges the two; the internal main is unchanged.\n\n");
    }
    if merged.is_empty() {
        b.push_str("No internal pull request was merged in this range: its commits reached the internal main directly.\n");
        return b;
    }
    b.push_str("Merged internally since the last publish:\n\n");
    for m in merged {
        b.push_str(&format!("- {} (internal PR {})\n", inert(&m.title), m.number));
    }
    b
}

/// An internal title made inert for GitHub: on one line, with a zero-width
/// joiner after every `#` and `@`, so `#12` links no issue of GitHub's and
/// `@name` mentions no one, while it reads as written.
fn inert(title: &str) -> String {
    let line = title.split_whitespace().collect::<Vec<_>>().join(" ");
    line.replace('#', "#\u{200D}").replace('@', "@\u{200D}")
}

/// Who a merge made for a publish is by: the owner of the GitHub token
/// (`/user`), its public email or GitHub's no-reply address for it.
fn github_identity(github: &GitHub) -> Result<(String, String), String> {
    let (status, body) = api("GET", &format!("{}/user", github.api), &bearer(github), None)?;
    if status != 200 {
        return Err(format!("GitHub answered {status} asking whose the token is, for the merge's author: {}", message(&body)));
    }
    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
    let login = v["login"].as_str().unwrap_or("").to_string();
    if login.is_empty() {
        return Err("GitHub named no login for the token, so the merge has no author".to_string());
    }
    let name = v["name"].as_str().filter(|n| !n.trim().is_empty()).unwrap_or(&login).to_string();
    let email = match (v["email"].as_str().filter(|e| e.contains('@')), v["id"].as_u64()) {
        (Some(e), _) => e.to_string(),
        (None, Some(id)) => format!("{id}+{login}@users.noreply.github.com"),
        (None, None) => format!("{login}@users.noreply.github.com"),
    };
    Ok((name, email))
}

/// A merge commit of `a` (first parent) and `b`, by `name <email>`, made in
/// the bare repository `dir` without a work tree: its sha, or the files the
/// two conflict in.
fn merge(dir: &Path, a: &str, b: &str, name: &str, email: &str, message: &str) -> Result<String, Vec<String>> {
    let o = git_command(dir, None, &["merge-tree", "--write-tree", "--name-only", "--no-messages", a, b]).output().map_err(|e| vec![format!("(git merge-tree: {e})")])?;
    let said = String::from_utf8_lossy(&o.stdout).to_string();
    let mut lines = said.lines();
    let tree = lines.next().unwrap_or("").trim().to_string();
    if o.status.code() != Some(0) || tree.is_empty() {
        let mut files: Vec<String> = lines.map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
        files.dedup();
        if files.is_empty() {
            files.push(format!("(git merge-tree: {})", String::from_utf8_lossy(&o.stderr).trim()));
        }
        return Err(files);
    }
    let mut c = git_command(dir, None, &["commit-tree", &tree, "-p", a, "-p", b, "-m", message]);
    for k in ["GIT_AUTHOR", "GIT_COMMITTER"] {
        c.env(format!("{k}_NAME"), name).env(format!("{k}_EMAIL"), email);
    }
    let o = c.output().map_err(|e| vec![format!("(git commit-tree: {e})")])?;
    if !o.status.success() {
        return Err(vec![format!("(git commit-tree: {})", String::from_utf8_lossy(&o.stderr).trim())]);
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn commits(n: usize) -> String {
    if n == 1 { "1 commit".to_string() } else { format!("{n} commits") }
}

fn short(sha: &str) -> &str {
    &sha[..12.min(sha.len())]
}

/// Every pull request merged at the internal forge, as its API lists the
/// closed ones, page by page until a page comes back empty.
fn merged_pulls(internal: &Internal) -> Result<Vec<Merged>, String> {
    let mut out = Vec::new();
    for page in 1..=1000 {
        let url = format!("{}/repos/{}/pulls?state=closed&sort=recentupdate&limit={PAGE}&page={page}", internal.api, internal.repo);
        let (status, body) = api("GET", &url, &format!("Authorization: token {}", internal.token), None)?;
        if status != 200 {
            return Err(format!("the internal forge answered {status} listing {}'s pull requests: {}", internal.repo, message(&body)));
        }
        let list: Vec<serde_json::Value> = serde_json::from_str(&body).map_err(|e| format!("the internal forge's pull requests did not read: {e}"))?;
        if list.is_empty() {
            break;
        }
        for p in list.iter().filter(|p| p["merged"] == serde_json::json!(true)) {
            let merge = p["merge_commit_sha"].as_str().unwrap_or("").to_string();
            if let (Some(number), false) = (p["number"].as_u64(), merge.is_empty()) {
                out.push(Merged { number, title: p["title"].as_str().unwrap_or("").to_string(), merge });
            }
        }
    }
    Ok(out)
}

/// The open pull request from `branch` against GitHub's main: its number
/// and its url.
fn open_pull(github: &GitHub, branch: &str) -> Result<Option<(u64, String)>, String> {
    let owner = github.repo.split('/').next().unwrap_or("");
    let url = format!("{}/repos/{}/pulls?state=open&base=main&head={}", github.api, github.repo, crate::api_client::percent_encode(&format!("{owner}:{branch}")));
    let (status, body) = api("GET", &url, &bearer(github), None)?;
    if status != 200 {
        return Err(format!("GitHub answered {status} listing {}'s pull requests: {}", github.repo, message(&body)));
    }
    let list: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap_or_default();
    Ok(list.first().and_then(|p| Some((p["number"].as_u64()?, p["html_url"].as_str().unwrap_or("").to_string()))))
}

fn bearer(github: &GitHub) -> String {
    format!("Authorization: Bearer {}", github.token)
}

/// The `message` an API's error body carries, else the body.
fn message(body: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    v["message"].as_str().map(str::to_string).unwrap_or_else(|| body.trim().chars().take(200).collect())
}

/// One API call through curl: its status and body. The `auth` header (and
/// the body) go in the config curl reads from its stdin, so the token is on
/// no command line.
fn api(method: &str, url: &str, auth: &str, body: Option<&str>) -> Result<(u16, String), String> {
    let mut config = format!("header = \"{}\"\n", config_quoted(auth));
    if let Some(b) = body {
        config.push_str("header = \"Content-Type: application/json\"\n");
        config.push_str(&format!("data-binary = \"{}\"\n", config_quoted(b)));
    }
    let mut child = Command::new("curl")
        .args(["-sS", "-K", "-", "-X", method, "-H", "Accept: application/json", "-H", "User-Agent: hale-forge-publish", "-w", "\n%{http_code}", "--", url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(config.as_bytes()).map_err(|e| format!("curl: {e}"))?;
    }
    let o = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    let text = String::from_utf8_lossy(&o.stdout).to_string();
    let (said, code) = text.rsplit_once('\n').unwrap_or(("", text.as_str()));
    let status: u16 = code.trim().parse().unwrap_or(0);
    if !o.status.success() || status == 0 {
        return Err(format!("{method} {} did not answer: {}", url.split('?').next().unwrap_or(url), String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok((status, said.to_string()))
}

/// `s` inside a double-quoted string of a curl config file.
fn config_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

/// `git` on the bare repository `dir`; with `auth` (a user and a token),
/// the token handed to git's credential helper in git's own environment.
/// Its stdout, trimmed.
fn git(dir: &Path, auth: Option<(&str, &str)>, args: &[&str]) -> Result<String, String> {
    let o = git_command(dir, auth, args).output().map_err(|e| format!("git: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        let e = String::from_utf8_lossy(&o.stderr).trim().to_string();
        Err(if e.is_empty() { format!("git {} failed", args[0]) } else { e })
    }
}

fn git_command(dir: &Path, auth: Option<(&str, &str)>, args: &[&str]) -> Command {
    let mut c = Command::new("git");
    c.arg("--git-dir").arg(dir).env("GIT_TERMINAL_PROMPT", "0").env_remove(TOKEN_ENV).stdin(Stdio::null());
    if let Some((user, token)) = auth {
        c.args(["-c", "credential.helper=", "-c", &format!("credential.helper=!f() {{ echo username={user}; echo \"password=${TOKEN_ENV}\"; }}; f")]).env(TOKEN_ENV, token);
    }
    c.args(args);
    c
}

/// Whether `a` is an ancestor of (or is) `b`.
fn is_ancestor(dir: &Path, a: &str, b: &str) -> Result<bool, String> {
    let o = git_command(dir, None, &["merge-base", "--is-ancestor", a, b]).output().map_err(|e| format!("git: {e}"))?;
    match o.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(format!("git merge-base: {}", String::from_utf8_lossy(&o.stderr).trim())),
    }
}

/// A private scratch directory (0700) for the publish's bare repository,
/// removed when the publish ends, however it ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Scratch, String> {
        use std::os::unix::fs::DirBuilderExt;
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
        let p = std::env::temp_dir().join(format!("hale-forge-publish-{}-{nanos}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::DirBuilder::new().mode(0o700).create(&p).map_err(|e| format!("create {}: {e}", p.display()))?;
        Ok(Scratch(p))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Today's date, UTC, `YYYY-MM-DD`.
fn today() -> String {
    utc_date(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
}

/// The UTC date `secs` after the epoch (the civil calendar from a day
/// count, after Howard Hinnant's `civil_from_days`).
fn utc_date(secs: u64) -> String {
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    fn args(line: &str) -> Vec<String> {
        line.split(' ').map(str::to_string).collect()
    }

    #[test]
    fn the_date_is_utc_civil() {
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(951_782_400), "2000-02-29");
        assert_eq!(utc_date(1_709_164_800), "2024-02-29");
        assert_eq!(utc_date(1_794_355_200 + 86_399), "2026-11-11");
        assert_eq!(utc_date(4_102_444_799), "2099-12-31");
    }

    #[test]
    fn publish_takes_one_organism_and_its_two_options() {
        assert_eq!(publish_args(&args("shop")), Some(PublishArgs { organism: "shop".into(), branch: None, dry_run: false }));
        assert_eq!(publish_args(&args("--dry-run shop --branch sync/x")), Some(PublishArgs { organism: "shop".into(), branch: Some("sync/x".into()), dry_run: true }));
        assert!(publish_args(&args("shop --branch")).is_none(), "an option without its value");
        assert!(publish_args(&args("shop other")).is_none() && publish_args(&[]).is_none());
        assert!(publish_args(&args("shop --force")).is_none(), "no force, not even as a word");
    }

    #[test]
    fn the_body_names_titles_and_internal_numbers_and_links_nowhere() {
        let merged = [Merged { number: 12, title: "Add the\nledger".into(), merge: "c".repeat(40) }, Merged { number: 15, title: "Fix it".into(), merge: "d".repeat(40) }];
        let b = pr_body(&"a".repeat(40), &"b".repeat(40), 3, &merged, false);
        assert!(b.contains("- Add the ledger (internal PR 12)\n- Fix it (internal PR 15)\n"), "{b}");
        assert!(b.contains("`aaaaaaaaaaaa`: 3 commits this repository's main (`bbbbbbbbbbbb`) lacks"), "{b}");
        assert!(!b.contains('#') && !b.contains("http") && !b.contains("merges the two"), "no link, and no #<n> GitHub would link to its own: {b}");
        assert!(pr_body("a", "b", 1, &[], false).contains("1 commit this") && pr_body("a", "b", 1, &[], false).contains("No internal pull request"));
        assert!(pr_body("a", "b", 1, &[], true).contains("the branch merges the two; the internal main is unchanged"));
    }

    /// An internal title reaches GitHub inert: a zero-width joiner after each
    /// `#` and `@`, so no issue of GitHub's is linked and no one is mentioned,
    /// and the title reads as it was written.
    #[test]
    fn a_title_links_no_issue_and_mentions_no_one() {
        let t = inert("Fix #12 and owner/repo#3 for @alice,\n cc @org/team");
        assert_eq!(t, "Fix #\u{200D}12 and owner/repo#\u{200D}3 for @\u{200D}alice, cc @\u{200D}org/team");
        let reference = |s: &str| s.char_indices().any(|(i, c)| (c == '#' || c == '@') && s[i + 1..].starts_with(|n: char| n.is_ascii_alphanumeric()));
        assert!(!reference(&t), "{t}");
        assert_eq!(t.replace('\u{200D}', ""), "Fix #12 and owner/repo#3 for @alice, cc @org/team", "it reads as written");
        let b = pr_body("a", "b", 1, &[Merged { number: 4, title: "Close #9, thanks @bob".into(), merge: "e".into() }], false);
        assert!(!reference(&b) && b.contains("- Close #\u{200D}9, thanks @\u{200D}bob (internal PR 4)"), "{b}");
    }

    fn row(kind: &str, entity: &str, body: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "kind": kind, "entity": entity, "body": body.to_string() })
    }

    /// Each refusal says what is missing and how to give it, before anything
    /// is fetched.
    #[test]
    fn a_publish_refuses_what_it_lacks() {
        let key = "0123456789ab";
        let forgejo = row("habitat.allocated", &format!("{key}/forge"), serde_json::json!({ "provider": "forgejo", "address": "http://127.0.0.1:16001", "repo": "shop-0123456789ab/shop", "key_vault": "forgejo-0123456789ab" }));
        let target = row("habitat.publish", key, serde_json::json!({ "provider": "github", "repo": "acme/shop", "key_vault": "forge-token" }));
        let all = |slot: &str| Some(format!("token-of-{slot}"));
        let none = |_: &str| None;
        let only_forgejo = |slot: &str| (slot != "forge-token").then(|| "t".to_string());
        let refused = |rows: &[serde_json::Value], token: &dyn Fn(&str) -> Option<String>| ends_from(rows, "shop", key, token).err().unwrap_or_default();

        let github_forge = row("habitat.allocated", &format!("{key}/forge"), serde_json::json!({ "provider": "github", "repo": "acme/shop" }));
        assert!(refused(&[github_forge, target.clone()], &all).contains("`shop` has no Forgejo allocation (its forge is github)"));
        assert!(refused(&[target.clone()], &all).contains("has no Forgejo allocation (its forge is none)"));
        let e = refused(&[forgejo.clone()], &all);
        assert!(e.contains("names no GitHub repository") && e.contains("hale habitat forge shop github --repo <owner/name> --publish"), "{e}");
        let stopped = row("habitat.publish", key, serde_json::json!({ "provider": "none", "repo": "" }));
        assert!(refused(&[forgejo.clone(), target.clone(), stopped], &all).contains("names no GitHub repository"), "the latest row stands");
        let e = refused(&[forgejo.clone(), target.clone()], &only_forgejo);
        assert!(e.contains("no GitHub token in the slot `forge-token`") && e.contains("hale dna secret set FORGE_TOKEN"), "{e}");
        assert!(refused(&[forgejo.clone(), target.clone()], &none).contains("GitHub token"), "GitHub's token is asked first");

        let (i, g) = ends_from(&[forgejo, target], "shop", key, &all).unwrap();
        assert_eq!((i.git_url.as_str(), i.api.as_str(), i.user.as_str(), i.token.as_str()), ("http://127.0.0.1:16001/shop-0123456789ab/shop.git", "http://127.0.0.1:16001/api/v1", "shop-0123456789ab", "token-of-forgejo-0123456789ab"));
        assert_eq!((g.git_url.as_str(), g.api.as_str(), g.repo.as_str(), g.token.as_str()), ("https://github.com/acme/shop.git", "https://api.github.com", "acme/shop", "token-of-forge-token"));
    }

    // ---- a publish end to end, against stand-ins -------------------------
    //
    // Both git remotes are bare repositories on disk; both APIs are one
    // server in this process, bound to port 0 and held for the test's life,
    // answering as Forgejo (`/api/v1`) and GitHub (`/gh`) do.

    #[derive(Default)]
    struct Fake {
        /// the internal forge's closed pull requests
        closed: Vec<serde_json::Value>,
        /// GitHub's open pull requests, each with its branch
        open: Vec<(String, serde_json::Value)>,
        /// every request: method, path, authorization, body
        seen: Vec<(String, String, String, String)>,
    }

    fn serve(fake: Arc<Mutex<Fake>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for s in listener.incoming().flatten() {
                answer(s, &fake);
            }
        });
        format!("http://{addr}")
    }

    fn answer(mut s: TcpStream, fake: &Mutex<Fake>) {
        let mut raw = Vec::new();
        let mut buf = [0u8; 4096];
        let head_end = loop {
            let n = s.read(&mut buf).unwrap_or(0);
            if n == 0 {
                return;
            }
            raw.extend_from_slice(&buf[..n]);
            if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
        };
        let head = String::from_utf8_lossy(&raw[..head_end]).to_string();
        let header = |name: &str| head.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string())).unwrap_or_default();
        let length: usize = header("content-length").parse().unwrap_or(0);
        while raw.len() < head_end + length {
            let n = s.read(&mut buf).unwrap_or(0);
            if n == 0 {
                break;
            }
            raw.extend_from_slice(&buf[..n]);
        }
        let body = String::from_utf8_lossy(&raw[head_end..]).to_string();
        let mut words = head.split_whitespace();
        let (method, target) = (words.next().unwrap_or("").to_string(), words.next().unwrap_or("").to_string());
        let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
        let mut f = fake.lock().unwrap();
        f.seen.push((method.clone(), target.clone(), header("authorization"), body.clone()));
        let (status, reply) = match (method.as_str(), path) {
            ("GET", "/api/v1/repos/o/r/pulls") => {
                let page: usize = query.split('&').find_map(|q| q.strip_prefix("page=")).and_then(|p| p.parse().ok()).unwrap_or(1);
                let list: Vec<_> = f.closed.iter().skip((page - 1) * PAGE).take(PAGE).cloned().collect();
                (200, serde_json::json!(list))
            }
            ("GET", "/gh/user") => (200, serde_json::json!({ "login": "owner", "name": "The Owner", "email": null, "id": 42 })),
            ("GET", "/gh/repos/acme/r/pulls") => {
                let head = query.split('&').find_map(|q| q.strip_prefix("head=")).map(crate::api_client::percent_decode).unwrap_or_default();
                let list: Vec<_> = f.open.iter().filter(|(b, _)| format!("acme:{b}") == head).map(|(_, p)| p.clone()).collect();
                (200, serde_json::json!(list))
            }
            ("POST", "/gh/repos/acme/r/pulls") => {
                let sent: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                let n = 7 + f.open.len();
                let made = serde_json::json!({ "number": n, "html_url": format!("https://github.invalid/acme/r/pull/{n}") });
                f.open.push((sent["head"].as_str().unwrap_or("").to_string(), made.clone()));
                (201, made)
            }
            ("PATCH", p) if p.starts_with("/gh/repos/acme/r/pulls/") => (200, serde_json::json!({})),
            _ => (404, serde_json::json!({ "message": "not here" })),
        };
        let reply = reply.to_string();
        let _ = write!(s, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
    }

    fn sh_git(cwd: &Path, args: &[&str]) -> String {
        let o = Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "-c", "init.defaultBranch=main"]).args(args).current_dir(cwd).output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    /// A commit on the work tree's main, pushed to the internal forge: its sha.
    fn land(work: &Path, internal: &Path, name: &str) -> String {
        std::fs::write(work.join(name), name).unwrap();
        sh_git(work, &["add", "-A"]);
        sh_git(work, &["commit", "-q", "-m", name]);
        sh_git(work, &["push", "-q", &internal.display().to_string(), "main"]);
        sh_git(work, &["rev-parse", "HEAD"])
    }

    fn pull(number: u64, title: &str, merged: bool, merge: &str) -> serde_json::Value {
        serde_json::json!({ "number": number, "title": title, "merged": merged, "merge_commit_sha": merge, "html_url": format!("http://127.0.0.1:16001/o/r/pulls/{number}") })
    }

    #[test]
    fn a_publish_pushes_the_internal_main_and_opens_one_pull_request_then_updates_it_carrying_github_merges() {
        let root = Scratch::new().unwrap();
        let (work, internal, github) = (root.0.join("work"), root.0.join("internal.git"), root.0.join("github.git"));
        std::fs::create_dir_all(&work).unwrap();
        sh_git(&root.0, &["init", "-q", "--bare", "-b", "main", &internal.display().to_string()]);
        sh_git(&root.0, &["init", "-q", "--bare", "-b", "main", &github.display().to_string()]);
        sh_git(&work, &["init", "-q", "-b", "main"]);
        let base = land(&work, &internal, "base");
        sh_git(&work, &["push", "-q", &github.display().to_string(), "main"]);
        let b = land(&work, &internal, "b");
        let c = land(&work, &internal, "c");

        let fake = Arc::new(Mutex::new(Fake::default()));
        let api = serve(fake.clone());
        // a page and a half of closed pull requests: the two merged in the
        // range, one merged before GitHub's main, one closed unmerged
        {
            let mut f = fake.lock().unwrap();
            f.closed.push(pull(3, "Add c", true, &c));
            f.closed.push(pull(2, "Abandoned", false, ""));
            for n in 100..(100 + PAGE as u64) {
                f.closed.push(pull(n, "Before the last publish", true, &base));
            }
            f.closed.push(pull(1, "Add b", true, &b));
        }
        let i = Internal { git_url: internal.display().to_string(), api: format!("{api}/api/v1"), repo: "o/r".into(), user: "o".into(), token: "internal-token".into() };
        let g = GitHub { git_url: github.display().to_string(), api: format!("{api}/gh"), repo: "acme/r".into(), token: "github-token".into() };
        let at_github = |r: &str| Command::new("git").args(["--git-dir", &github.display().to_string(), "rev-parse", "--verify", "-q", r]).output().unwrap().stdout;

        // a dry run says it all and changes nothing
        let said = publish(&i, &g, "sync/2026-11-11", "2026-11-11", true).unwrap().join("\n");
        assert!(said.contains(&format!("would push {} to github acme/r as sync/2026-11-11", short(&c))), "{said}");
        assert!(said.contains("would open a pull request against main") && said.contains("title: Publish 2026-11-11"), "{said}");
        assert!(said.contains("- Add b (internal PR 1)\n- Add c (internal PR 3)"), "in the order they reached main: {said}");
        assert!(!said.contains("Abandoned") && !said.contains("Before the last publish") && !said.contains("127.0.0.1"), "{said}");
        assert!(at_github("refs/heads/sync/2026-11-11").is_empty(), "a dry run pushes nothing");
        assert!(fake.lock().unwrap().seen.iter().all(|(m, ..)| m == "GET"), "a dry run writes nothing");

        // the publish: the branch at the internal main, one pull request
        let said = publish(&i, &g, "sync/2026-11-11", "2026-11-11", false).unwrap().join("\n");
        assert!(said.contains("opened  https://github.invalid/acme/r/pull/7"), "{said}");
        assert_eq!(String::from_utf8_lossy(&at_github("refs/heads/sync/2026-11-11")).trim(), c);
        {
            let f = fake.lock().unwrap();
            let posted: Vec<_> = f.seen.iter().filter(|(m, ..)| m == "POST").collect();
            assert_eq!(posted.len(), 1);
            let sent: serde_json::Value = serde_json::from_str(&posted[0].3).unwrap();
            assert_eq!((sent["head"].as_str(), sent["base"].as_str(), sent["title"].as_str()), (Some("sync/2026-11-11"), Some("main"), Some("Publish 2026-11-11")));
            assert!(sent["body"].as_str().unwrap().contains("- Add c (internal PR 3)\n"));
            // each end is asked with its own token, and only it
            for (m, path, auth, _) in &f.seen {
                let want = if path.starts_with("/api/v1") { "token internal-token" } else { "Bearer github-token" };
                assert_eq!(auth, want, "{m} {path}");
            }
        }

        // the same day again, after another merge: the branch moves forward
        // and the pull request is updated, not opened again
        let d = land(&work, &internal, "d");
        fake.lock().unwrap().closed.insert(0, pull(4, "Add d", true, &d));
        let said = publish(&i, &g, "sync/2026-11-11", "2026-11-11", false).unwrap().join("\n");
        assert!(said.contains("updated https://github.invalid/acme/r/pull/7"), "{said}");
        assert_eq!(String::from_utf8_lossy(&at_github("refs/heads/sync/2026-11-11")).trim(), d);
        {
            let f = fake.lock().unwrap();
            assert_eq!(f.seen.iter().filter(|(m, ..)| m == "POST").count(), 1, "one pull request per publish");
            let patched: Vec<_> = f.seen.iter().filter(|(m, ..)| m == "PATCH").collect();
            assert_eq!(patched.len(), 1);
            assert!(patched[0].3.contains("Add d (internal PR 4)"), "{}", patched[0].3);
        }

        // GitHub's merge queue merges the pull request with a merge commit:
        // GitHub's main now holds the internal main, and nothing is new
        let other = root.0.join("other");
        sh_git(&root.0, &["clone", "-q", &github.display().to_string(), &other.display().to_string()]);
        sh_git(&other, &["fetch", "-q", "origin", "sync/2026-11-11"]);
        sh_git(&other, &["merge", "-q", "--no-ff", "-m", "Merge pull request #7", "FETCH_HEAD"]);
        sh_git(&other, &["push", "-q", "origin", "main"]);
        let queued = sh_git(&other, &["rev-parse", "HEAD"]);
        let said = publish(&i, &g, "sync/2026-11-12", "2026-11-12", false).unwrap().join("\n");
        assert!(said.contains("nothing to publish"), "{said}");

        // a GitHub-only merge commit is carried: the branch pushed is a merge
        // of the internal main and GitHub's main, by the token's owner, and
        // the internal main is untouched
        let e = land(&work, &internal, "e");
        fake.lock().unwrap().closed.insert(0, pull(5, "Add e", true, &e));
        let said = publish(&i, &g, "sync/2026-11-12", "2026-11-12", true).unwrap().join("\n");
        assert!(said.contains(&format!("would have GitHub's main {} into the internal main", short(&queued))) && said.contains("merges the two"), "{said}");
        assert!(at_github("refs/heads/sync/2026-11-12").is_empty(), "a dry run pushes nothing");
        let said = publish(&i, &g, "sync/2026-11-12", "2026-11-12", false).unwrap().join("\n");
        assert!(said.contains(&format!("merged  GitHub's main {} into the internal main", short(&queued))) && said.contains("by The Owner <42+owner@users.noreply.github.com>"), "{said}");
        assert!(said.contains("opened  https://github.invalid/acme/r/pull/8"), "a new day, a new pull request: {said}");
        let carried = String::from_utf8_lossy(&at_github("refs/heads/sync/2026-11-12")).trim().to_string();
        let gh = |args: &[&str]| sh_git(&root.0, &[&["--git-dir", &github.display().to_string()], args].concat());
        assert_eq!(gh(&["rev-parse", &format!("{carried}^1"), &format!("{carried}^2")]), format!("{e}\n{queued}"), "the internal main first, GitHub's second");
        assert_eq!(gh(&["log", "-1", "--format=%an <%ae>|%cn <%ce>", &carried]), "The Owner <42+owner@users.noreply.github.com>|The Owner <42+owner@users.noreply.github.com>");
        assert_eq!(sh_git(&root.0, &["--git-dir", &internal.display().to_string(), "rev-parse", "main"]), e, "the internal main is untouched");
        {
            let f = fake.lock().unwrap();
            let body = f.seen.iter().rev().find(|(m, ..)| m == "POST").map(|r| r.3.clone()).unwrap();
            assert!(body.contains("Add e (internal PR 5)") && body.contains("merges the two") && !body.contains("Add d"), "{body}");
        }

        // the same day again: the branch moves forward over the merge it
        // holds (merged in, never forced) and its one pull request is updated
        let f2 = land(&work, &internal, "f");
        let said = publish(&i, &g, "sync/2026-11-12", "2026-11-12", false).unwrap().join("\n");
        assert!(said.contains("updated https://github.invalid/acme/r/pull/8"), "{said}");
        let moved = String::from_utf8_lossy(&at_github("refs/heads/sync/2026-11-12")).trim().to_string();
        assert!(Command::new("git").args(["--git-dir", &github.display().to_string(), "merge-base", "--is-ancestor", &carried, &moved]).status().unwrap().success(), "a fast-forward");
        assert!(Command::new("git").args(["--git-dir", &github.display().to_string(), "merge-base", "--is-ancestor", &f2, &moved]).status().unwrap().success());
        assert_eq!(fake.lock().unwrap().seen.iter().filter(|(m, ..)| m == "POST").count(), 2, "one pull request per publish");

        // a GitHub-only change that conflicts with the internal main stops
        // the publish, naming the file, and pushes nothing
        std::fs::write(other.join("g"), "made at GitHub").unwrap();
        sh_git(&other, &["add", "-A"]);
        sh_git(&other, &["commit", "-q", "-m", "made at GitHub"]);
        sh_git(&other, &["push", "-q", "origin", "main"]);
        land(&work, &internal, "g");
        let e = publish(&i, &g, "sync/2026-11-13", "2026-11-13", false).unwrap_err();
        assert!(e.contains("conflict in g:") && e.contains("never force-pushes"), "{e}");
        assert!(at_github("refs/heads/sync/2026-11-13").is_empty());
        assert!(publish(&i, &g, "main", "2026-11-13", true).unwrap_err().contains("never `main`"));
    }
}
