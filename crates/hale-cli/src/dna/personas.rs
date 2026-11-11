//! The chart's personas at the habitat's Forgejo.
//!
//! The habitat's Forgejo shows the parts of each organism discussing and
//! cooperating, not one account talking to itself: every position its
//! chart states, its Board, and every person who holds a position there
//! is a Forgejo user of its own, with write access to the organism's
//! repository, an avatar drawn from its name and a token in the habitat's
//! vault. A position's user and the Board's are the organism's
//! (`<organism user>-<position>`, its token `forgejo-<key>-<slug>`); a
//! person's is the habitat's, one however many organisms the person is a
//! member of (`<member>`, its token `forgejo-person-<user>`).
//!
//! `hale habitat graft` reconciles the organism it grafts once its own
//! user is made, and every `hale habitat apply` reconciles them all: a
//! persona missing is made, one whose position retired or closed is
//! removed with its slot, and a person's user goes once no organism's
//! chart has the person. A persona whose user exists is left alone, its
//! token minted again only when its slot is missing, so a second apply
//! changes nothing.
//!
//! The chart is read with `hale dna chart`, the core's `org_chart` lines
//! and a `person <name>` line per person holding a position; Forgejo is spoken to through `habitat-forge-personas`, which
//! lists its users and carries out a plan's steps. The plan itself is made
//! here, from the charts, Forgejo's users and the vault's slots, by
//! functions that touch nothing (`plan`), so what is made and removed is
//! decided in one place and tested without a Forgejo.
//!
//! Personas never make a graft or an apply refuse: a chart that cannot be
//! read (its memory just moved or recreated, not answering), or a Forgejo
//! that does not, leaves them as they are, said as `skipped`, and the next
//! apply reconciles them (`applied_entries`).
//!
//! A user is marked by its email: `<user>@<key>.persona.habitat.invalid`
//! for an organism's, `<user>@person.habitat.invalid` for a person's. Only
//! a user so marked is ever removed, and a user of the same name not so
//! marked is a conflict, said and left alone.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::{genome_dir, host_run, local_habitat_dir};

/// Forgejo's limit on a user's name.
const USER_MAX: usize = 39;
/// The hex digits of the hash a cut or reshaped name carries.
const HASH_LEN: usize = 8;
/// The Board: a position like any other (`position:board`), and a
/// persona every organism has, stated or not.
pub(super) const BOARD: &str = "position:board";
/// The email domain that marks a person's user.
pub(super) const PERSON_DOMAIN: &str = "person.habitat.invalid";
/// The start of a person's vault slot.
pub(super) const PERSON_SLOT: &str = "forgejo-person-";
/// The longest position slug a vault slot carries whole.
const SLUG_MAX: usize = 60;
/// Names Forgejo keeps for itself, and the habitat's administrator's: a
/// person called so is given a reshaped name.
const RESERVED: &[&str] = &[
    "hale-admin", "admin", "api", "assets", "attachments", "avatar", "avatars", "captcha", "commits", "debug", "devtest", "error", "explore", "favicon", "forgejo-actions", "ghost", "gitea-actions", "help", "install", "issues", "login", "manifest", "metrics", "milestones", "new", "notifications", "org", "pulls", "raw", "repo", "repo-avatars", "robots", "search", "serviceworker", "sitemap", "ssh-info", "swagger", "user", "v2",
];

/// The email domain that marks a persona of the organism keyed `key`.
pub(super) fn persona_domain(key: &str) -> String {
    format!("{key}.persona.habitat.invalid")
}

/// The first `HASH_LEN` hex digits of `text`'s SHA-256.
fn short_hash(text: &str) -> String {
    openssl::sha::sha256(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>()[..HASH_LEN].to_string()
}

/// Whether `s` is a name every reader keeps as it is: lowercase letters,
/// digits and single dashes between them.
fn plain_word(s: &str) -> bool {
    !s.is_empty() && !s.starts_with('-') && !s.ends_with('-') && !s.contains("--") && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Whether a position's name is plain: each of its `/`-separated parts a
/// plain word, so joining them with `--` (a slot) or `_` (a user) can be
/// read back.
fn plain_name(name: &str) -> bool {
    name.split('/').all(plain_word)
}

/// `s` lowercased, every character but a letter or digit a dash, runs of
/// dashes one, none at either end.
fn tidy(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// `s` cut to at most `n` characters (all ASCII), with no separator left
/// at its end.
fn cut(s: &str, n: usize) -> String {
    s[..s.len().min(n)].trim_end_matches(['-', '_']).to_string()
}

/// The name of a position, its id without `position:`.
fn position_name(id: &str) -> &str {
    id.strip_prefix("position:").unwrap_or(id)
}

/// The slug a position's vault slot carries: its name, each `/` as `--`
/// (`api/dev` is `api--dev`). A name that is not plain (capitals, spaces,
/// a doubled dash), or one too long for a slot, is tidied and carries a
/// hash of the position's id, so two positions never share a slot.
pub(super) fn position_slug(id: &str) -> String {
    let name = position_name(id);
    if plain_name(name) && name.len() <= SLUG_MAX {
        return name.replace('/', "--");
    }
    let tidied: Vec<String> = name.split('/').map(tidy).filter(|p| !p.is_empty()).collect();
    let body = if tidied.is_empty() { "position".to_string() } else { tidied.join("--") };
    format!("{}-{}", cut(&body, SLUG_MAX - HASH_LEN - 1), short_hash(id))
}

/// The Forgejo user of the position `id` of the organism whose own user is
/// `organism`: `<organism>-<name>`, each `/` of the name as `_` (Forgejo
/// takes no doubled dash). When that is longer than Forgejo's 39
/// characters, or the name is not plain, it is cut and carries a hash of
/// the organism and the position, so it is stable from apply to apply and
/// never another position's.
pub(super) fn position_user(organism: &str, id: &str) -> String {
    let name = position_name(id);
    let plain = plain_name(name);
    let body = if plain { name.split('/').collect::<Vec<_>>().join("_") } else { tidy(name) };
    let whole = format!("{organism}-{body}");
    if plain && whole.len() <= USER_MAX {
        return whole;
    }
    let hash = short_hash(&format!("{organism}\n{id}"));
    // the position's name kept readable before the organism's, which the
    // hash stands for
    let room = USER_MAX - HASH_LEN - 1;
    let b = cut(if body.is_empty() { "position" } else { &body }, 20);
    let o = cut(organism, room.saturating_sub(b.len() + 1));
    if o.is_empty() {
        format!("{b}-{hash}")
    } else {
        format!("{o}-{b}-{hash}")
    }
}

/// The Forgejo user of the person `member`: the member's name, when it is
/// plain, fits and is not a name Forgejo or the habitat keeps; else the
/// name tidied and cut, with a hash of it.
pub(super) fn person_user(member: &str) -> String {
    if plain_word(member) && member.len() <= USER_MAX && !RESERVED.contains(&member) {
        return member.to_string();
    }
    let t = tidy(member);
    let body = if t.is_empty() { "person".to_string() } else { cut(&t, USER_MAX - HASH_LEN - 1) };
    format!("{body}-{}", short_hash(&format!("person\n{member}")))
}

/// What a persona stands for: a position of an organism's chart (the
/// Board among them), or a person who holds one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Hat {
    Position { key: String, id: String },
    Person { member: String },
}

/// A persona's avatar: drawn from its seed. Kept as data of the persona's,
/// so a later avatar per hat (chosen by the hat worn) replaces the seed or
/// the drawing, never the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Avatar {
    pub seed: String,
}

/// One persona: who it stands for, its Forgejo user, the email that marks
/// it, its token's vault slot, the name Forgejo shows and its avatar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Persona {
    pub hat: Hat,
    pub user: String,
    pub email: String,
    pub slot: String,
    pub full_name: String,
    pub avatar: Avatar,
}

/// The persona of the position `id` of the organism keyed `key`, called
/// `name`, whose own user is `organism`.
pub(super) fn position_persona(key: &str, name: &str, organism: &str, id: &str) -> Persona {
    let user = position_user(organism, id);
    let shown = if id == BOARD { format!("the Board of {name}") } else { format!("{} of {name}", position_name(id)) };
    Persona { hat: Hat::Position { key: key.to_string(), id: id.to_string() }, email: format!("{user}@{}", persona_domain(key)), slot: format!("forgejo-{key}-{}", position_slug(id)), full_name: shown, avatar: Avatar { seed: user.clone() }, user }
}

/// The persona of the person `member`.
pub(super) fn person_persona(member: &str) -> Persona {
    let user = person_user(member);
    Persona { hat: Hat::Person { member: member.to_string() }, email: format!("{user}@{PERSON_DOMAIN}"), slot: format!("{PERSON_SLOT}{user}"), full_name: member.to_string(), avatar: Avatar { seed: user.clone() }, user }
}

/// An organism's chart, as far as its personas go: the positions it
/// states, in order, and the people who hold them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Chart {
    pub positions: Vec<String>,
    pub persons: Vec<String>,
}

/// The chart in a reader's lines: `position <id>` for each stated
/// position, `person <name>` for each person, and `holds <id> <holder>`,
/// whose holder is a person unless it is an organization
/// (`organization:<name>`). Every other line is left (`mandate`, `under`,
/// `practice`, …); a holder with a `:` (an organization) is no person.
/// Each name once, in the order first read.
pub(super) fn parse_chart(text: &str) -> Chart {
    let mut c = Chart::default();
    let add = |list: &mut Vec<String>, v: &str| {
        let v = v.trim();
        if !v.is_empty() && !list.iter().any(|x| x == v) {
            list.push(v.to_string());
        }
    };
    for line in text.lines() {
        if let Some(id) = line.strip_prefix("position ") {
            add(&mut c.positions, id);
        } else if let Some(who) = line.strip_prefix("person ") {
            add(&mut c.persons, who);
        } else if let Some(rest) = line.strip_prefix("holds ") {
            if let Some((_, holder)) = rest.split_once(' ') {
                if !holder.contains(':') {
                    add(&mut c.persons, holder);
                }
            }
        }
    }
    c
}

/// An organism with a repository at the habitat's Forgejo, and its chart
/// when it was read (`None`: not read, so nothing of its is removed).
#[derive(Debug, Clone)]
pub(super) struct Organism {
    pub key: String,
    pub name: String,
    /// its own Forgejo user, the owner of its repository
    pub user: String,
    /// `owner/name`
    pub repo: String,
    /// the vault slot of its own user's token
    pub slot: String,
    pub chart: Option<Chart>,
}

/// The personas `o`'s chart asks for: the Board, then each stated
/// position, then each person.
pub(super) fn personas_of(o: &Organism, chart: &Chart) -> Vec<Persona> {
    let mut out = vec![position_persona(&o.key, &o.name, &o.user, BOARD)];
    for p in chart.positions.iter().filter(|p| p.as_str() != BOARD) {
        out.push(position_persona(&o.key, &o.name, &o.user, p));
    }
    for m in &chart.persons {
        out.push(person_persona(m));
    }
    out
}

/// What the habitat's Forgejo and vault hold now: each user's email, by
/// its name, and the vault's slots.
#[derive(Debug, Clone, Default)]
pub(super) struct Held {
    pub users: BTreeMap<String, String>,
    pub slots: BTreeSet<String>,
}

/// One step of a plan, in the order the steps are taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Step {
    /// the persona's user made when Forgejo holds none, and its token
    /// minted into its slot when `mint` (its user was missing) or the slot
    /// is; a token minted, its name and avatar are set with it
    Ensure { persona: Persona, mint: bool },
    /// write access to `repo` for `user`, by the owner's token in `owner_slot`,
    /// when it has none
    Grant { user: String, repo: String, owner_slot: String },
    /// the user removed, when Forgejo still lists it with this email
    Remove { user: String, email: String },
    /// the vault slot removed
    Forget { slot: String },
}

/// A plan: its steps, the users it may not take (a name held by a user it
/// did not make), and each organism's personas, by its key, for its ledger.
#[derive(Debug, Clone, Default)]
pub(super) struct Plan {
    pub steps: Vec<Step>,
    pub conflicts: Vec<String>,
    pub personas: BTreeMap<String, Vec<Persona>>,
}

/// The plan that brings the habitat's Forgejo and vault (`held`) to the
/// charts of `organisms`. With `every`, `organisms` are every organism the
/// habitat holds with a Forgejo repository: a persona of an organism not
/// among them is removed, and, when every chart was read, a person's user
/// no chart has. Without it (a graft), only the organisms named are
/// reconciled and no person is removed. An organism whose chart was not
/// read keeps everything it has.
pub(super) fn plan(organisms: &[Organism], held: &Held, every: bool) -> Plan {
    let mut out = Plan::default();
    let mut wanted_users: BTreeSet<String> = BTreeSet::new();
    let mut wanted_slots: BTreeSet<String> = BTreeSet::new();
    let mut ensured: BTreeSet<String> = BTreeSet::new();
    let mut conflicted: BTreeSet<String> = BTreeSet::new();
    let mut grants: Vec<Step> = Vec::new();
    for o in organisms {
        let Some(chart) = &o.chart else { continue };
        let mine = personas_of(o, chart);
        for p in &mine {
            wanted_users.insert(p.user.clone());
            wanted_slots.insert(p.slot.clone());
            // a person of two organisms is made once and granted on each
            if ensured.insert(p.user.clone()) {
                match held.users.get(&p.user) {
                    None => out.steps.push(Step::Ensure { persona: p.clone(), mint: true }),
                    Some(e) if e == &p.email => {
                        if !held.slots.contains(&p.slot) {
                            out.steps.push(Step::Ensure { persona: p.clone(), mint: false });
                        }
                    }
                    Some(e) => {
                        out.conflicts.push(format!("the Forgejo user {} is not this habitat's persona of {} (its email is {e}): left as it is", p.user, p.full_name));
                        conflicted.insert(p.user.clone());
                    }
                }
            }
            if !conflicted.contains(&p.user) {
                grants.push(Step::Grant { user: p.user.clone(), repo: o.repo.clone(), owner_slot: o.slot.clone() });
            }
        }
        out.personas.insert(o.key.clone(), mine);
    }
    out.steps.extend(grants);
    // what is held and no chart asks for
    let read: BTreeSet<&str> = organisms.iter().filter(|o| o.chart.is_some()).map(|o| o.key.as_str()).collect();
    let known: BTreeSet<&str> = organisms.iter().map(|o| o.key.as_str()).collect();
    let every_read = every && organisms.iter().all(|o| o.chart.is_some());
    for (user, email) in &held.users {
        if wanted_users.contains(user) {
            continue;
        }
        let Some((_, domain)) = email.split_once('@') else { continue };
        let gone = if domain == PERSON_DOMAIN {
            every_read
        } else if let Some(key) = domain.strip_suffix(".persona.habitat.invalid") {
            read.contains(key) || (every && !known.contains(key))
        } else {
            false
        };
        if gone {
            out.steps.push(Step::Remove { user: user.clone(), email: email.clone() });
        }
    }
    for slot in &held.slots {
        if wanted_slots.contains(slot) {
            continue;
        }
        let gone = if slot.starts_with(PERSON_SLOT) { every_read } else { read.iter().any(|k| slot.starts_with(&format!("forgejo-{k}-"))) };
        if gone {
            out.steps.push(Step::Forget { slot: slot.clone() });
        }
    }
    out
}

/// A step as a line of the plan `habitat-forge-personas` carries out, its
/// fields tab-separated (no field holds a tab or a newline).
fn step_line(s: &Step) -> String {
    let clean = |t: &str| t.replace(['\t', '\n', '\r'], " ");
    match s {
        Step::Ensure { persona: p, mint } => format!("ensure\t{}\t{}\t{}\t{}\t{}\t{}", p.user, p.email, p.slot, if *mint { "1" } else { "0" }, clean(&p.full_name), openssl::base64::encode_block(&identicon(&p.avatar.seed))),
        Step::Grant { user, repo, owner_slot } => format!("grant\t{user}\t{repo}\t{owner_slot}"),
        Step::Remove { user, email } => format!("remove\t{user}\t{email}"),
        Step::Forget { slot } => format!("forget\t{slot}"),
    }
}

/// An organism's ledger (`allocations/<key>/personas`): a line per persona,
/// `<position|person>\t<id or member>\t<user>\t<slot>`. Status counts it;
/// removing an organism reads every other one to keep a person's user that
/// another organism's chart has.
pub(super) fn ledger_text(personas: &[Persona]) -> String {
    personas
        .iter()
        .map(|p| match &p.hat {
            Hat::Position { id, .. } => format!("position\t{id}\t{}\t{}\n", p.user, p.slot),
            Hat::Person { member } => format!("person\t{member}\t{}\t{}\n", p.user, p.slot),
        })
        .collect()
}

fn ledger_path(key: &str) -> PathBuf {
    PathBuf::from(local_habitat_dir()).join("allocations").join(key).join("personas")
}

/// `hale habitat status`'s word on the organism keyed `key`'s personas,
/// from its ledger; None when it has no repository at the habitat's
/// Forgejo.
pub(super) fn status_line(key: &str) -> Option<String> {
    forge_of(key)?;
    let Ok(text) = std::fs::read_to_string(ledger_path(key)) else {
        return Some(format!("{:<8} none yet: `hale habitat apply` makes them at the habitat's Forgejo", "personas"));
    };
    let positions = text.lines().filter(|l| l.starts_with("position\t")).count();
    let people = text.lines().filter(|l| l.starts_with("person\t")).count();
    Some(format!("{:<8} {} at the habitat's Forgejo ({} with the Board, {})", "personas", positions + people, plural(positions, "position", "positions"), plural(people, "person", "people")))
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The organism keyed `key`'s repository at the habitat's Forgejo, from its
/// forge cache: its owner (the organism's user), `owner/name` and its
/// token's slot.
fn forge_of(key: &str) -> Option<(String, String, String)> {
    let path = PathBuf::from(local_habitat_dir()).join("allocations").join(key).join("forge.json");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let f = |k: &str| v[k].as_str().unwrap_or("").to_string();
    let repo = f("repo");
    let (owner, _) = repo.split_once('/')?;
    (f("provider") == "forgejo" && !owner.is_empty() && !f("key_vault").is_empty()).then(|| (owner.to_string(), repo.clone(), f("key_vault")))
}

/// What a reconcile came to for one organism: its personas reconciled,
/// with the line that says so, or skipped, with why (its chart not read).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Outcome {
    Done(String),
    Skipped(String),
}

/// What a reconcile came to, an organism at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Reconciled {
    pub key: String,
    pub outcome: Outcome,
}

/// The personas of every organism the habitat holds with a repository at
/// its Forgejo (`only`: of that organism's key alone, as its graft does),
/// reconciled: read, planned and carried out. An error is the habitat's
/// Forgejo not answering, or its steps failing; an organism whose chart
/// could not be read is said in its own outcome.
pub(super) fn reconcile(only: Option<&str>) -> Result<Vec<Reconciled>, String> {
    let mut organisms = Vec::new();
    let mut roots: BTreeMap<String, PathBuf> = BTreeMap::new();
    for g in super::models::grafted() {
        if only.is_some_and(|k| k != g.key) {
            continue;
        }
        let Some((user, repo, slot)) = forge_of(&g.key) else { continue };
        roots.insert(g.key.clone(), g.root.clone());
        organisms.push(Organism { key: g.key, name: g.name, user, repo, slot, chart: None });
    }
    if organisms.is_empty() {
        return Ok(Vec::new());
    }
    let mut unread: BTreeMap<String, String> = BTreeMap::new();
    for o in &mut organisms {
        match read_chart(&roots[&o.key]) {
            Ok(c) => o.chart = Some(c),
            Err(e) => {
                unread.insert(o.key.clone(), e);
            }
        }
    }
    let vault = crate::dna::vault_local_dir().ok_or("the personas' tokens are minted into the local vault, and the vault is at HALE_VAULT_ADDR")?;
    let slots: BTreeSet<String> = std::fs::read_dir(&vault).map(|es| es.flatten().filter_map(|e| e.file_name().to_str().map(str::to_string)).filter(|n| n.starts_with("forgejo-")).collect()).unwrap_or_default();
    let listed = host_run("habitat-forge-personas", &genome_dir(), &["--list".to_string()])?;
    let users = listed.lines().filter_map(|l| l.split_once('\t')).map(|(u, e)| (u.to_string(), e.to_string())).collect();
    let p = plan(&organisms, &Held { users, slots }, only.is_none());
    let mut done: BTreeMap<String, usize> = BTreeMap::new();
    if !p.steps.is_empty() {
        let file = PathBuf::from(local_habitat_dir()).join(format!(".personas-{}.plan", std::process::id()));
        let text: String = p.steps.iter().map(|s| format!("{}\n", step_line(s))).collect();
        std::fs::write(&file, text).map_err(|e| format!("write {}: {e}", file.display()))?;
        let ran = host_run("habitat-forge-personas", &genome_dir(), &[file.display().to_string()]);
        let _ = std::fs::remove_file(&file);
        for l in ran?.lines() {
            let mut w = l.split('\t');
            let (Some(word), Some(_)) = (w.next(), w.next()) else { continue };
            *done.entry(word.to_string()).or_default() += 1;
        }
    }
    let mut out = Vec::new();
    for o in &organisms {
        if let Some(e) = unread.get(&o.key) {
            out.push(Reconciled { key: o.key.clone(), outcome: Outcome::Skipped(format!("chart not read: {e}")) });
            continue;
        }
        let mine = &p.personas[&o.key];
        let path = ledger_path(&o.key);
        let text = ledger_text(mine);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            std::fs::write(&path, &text).map_err(|e| format!("write {}: {e}", path.display()))?;
        }
        let people = mine.iter().filter(|x| matches!(x.hat, Hat::Person { .. })).count();
        out.push(Reconciled { key: o.key.clone(), outcome: Outcome::Done(format!("{} at the habitat's Forgejo ({}, {})", mine.len(), plural(mine.len() - people, "position", "positions"), plural(people, "person", "people"))) });
    }
    // the steps are the habitat's, not an organism's: what they came to is
    // said once, on the first organism's line
    if let Some(first) = out.iter_mut().find(|r| matches!(r.outcome, Outcome::Done(_))) {
        let said: Vec<String> = ["made", "minted", "granted", "removed", "forgot"].iter().filter_map(|w| done.get(*w).map(|n| format!("{w} {n}"))).collect();
        let mut extra = if said.is_empty() { "unchanged".to_string() } else { said.join(", ") };
        for c in &p.conflicts {
            extra.push_str(&format!("; {c}"));
        }
        if let Outcome::Done(line) = &mut first.outcome {
            line.push_str(&format!(": {extra}"));
        }
    }
    Ok(out)
}

/// The chart of the organism at `root`, as its reader answers it.
fn read_chart(root: &Path) -> Result<Chart, String> {
    // the org chart's own reader; it fails when the record cannot be read,
    // never answering an unread chart as an empty one
    host_run("chart", root, &[]).map(|t| parse_chart(&t))
}

/// `reconcile`'s outcome as the lines a graft says: never a failure, a
/// graft is whole without its personas.
pub(super) fn lines_of(r: Result<Vec<Reconciled>, String>) -> Vec<String> {
    match r {
        Err(e) => vec![format!("{:<8}the chart's personas at the habitat's Forgejo: skipped ({e}); `hale habitat apply` makes them", "note")],
        Ok(list) => list
            .into_iter()
            .map(|x| match x.outcome {
                Outcome::Done(l) => format!("personas {}: {l}", x.key),
                Outcome::Skipped(e) => format!("{:<8}the personas of {}: skipped ({e}); `hale habitat apply` makes them", "note", x.key),
            })
            .collect(),
    }
}

/// `reconcile`'s outcome as the entries an apply lists among what it
/// applied, each an allocation `<key>/personas` (`personas` when nothing
/// was reached): every one ok, a skipped one saying why in its state, so
/// personas never make an apply refuse or fail.
pub(super) fn applied_entries(r: Result<Vec<Reconciled>, String>) -> Vec<serde_json::Value> {
    let entry = |allocation: String, state: String| serde_json::json!({ "ok": true, "allocation": allocation, "host": "local", "provider": "forgejo", "state": state });
    match r {
        Err(e) => vec![entry("personas".to_string(), format!("skipped ({e})"))],
        Ok(list) => list
            .into_iter()
            .map(|x| match x.outcome {
                Outcome::Done(l) => entry(format!("{}/personas", x.key), l),
                Outcome::Skipped(e) => entry(format!("{}/personas", x.key), format!("skipped ({e})")),
            })
            .collect(),
    }
}

// ---- the avatar: an identicon drawn from the persona's seed --------------

/// A 96×96 PNG drawn from `seed`: a 5×5 grid, mirrored about its middle
/// column, in one colour on a light ground, both from the seed's SHA-256.
/// Two colours, one bit a pixel, stored uncompressed: about 1.3 KiB.
pub(super) fn identicon(seed: &str) -> Vec<u8> {
    let h = openssl::sha::sha256(seed.as_bytes());
    let fg = [48 + h[0] % 144, 48 + h[1] % 144, 48 + h[2] % 144];
    let bg = [240u8, 240, 240];
    const CELL: usize = 16;
    const BORDER: usize = 8;
    const SIZE: usize = 5 * CELL + 2 * BORDER;
    let on = |r: usize, c: usize| {
        let c = if c > 2 { 4 - c } else { c };
        let bit = r * 3 + c;
        (h[3 + bit / 8] >> (bit % 8)) & 1 == 1
    };
    let stride = SIZE.div_ceil(8);
    let mut raw = Vec::with_capacity(SIZE * (stride + 1));
    for y in 0..SIZE {
        raw.push(0u8); // no filter
        let mut row = vec![0u8; stride];
        for x in 0..SIZE {
            let inside = (BORDER..BORDER + 5 * CELL).contains(&x) && (BORDER..BORDER + 5 * CELL).contains(&y);
            if inside && on((y - BORDER) / CELL, (x - BORDER) / CELL) {
                row[x / 8] |= 0x80 >> (x % 8);
            }
        }
        raw.extend(row);
    }
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend((SIZE as u32).to_be_bytes());
    ihdr.extend((SIZE as u32).to_be_bytes());
    ihdr.extend([1, 3, 0, 0, 0]); // 1 bit, palette, deflate, no filter, no interlace
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"PLTE", &[bg[0], bg[1], bg[2], fg[0], fg[1], fg[2]]);
    chunk(&mut png, b"IDAT", &zlib_stored(&raw));
    chunk(&mut png, b"IEND", &[]);
    png
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend(kind);
    out.extend(data);
    let crc = crc32(&out[start..]);
    out.extend(crc.to_be_bytes());
}

/// `data` as a zlib stream of stored (uncompressed) deflate blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = if data.is_empty() { vec![&[][..]] } else { data.chunks(65535).collect() };
    for (i, b) in blocks.iter().enumerate() {
        out.push(u8::from(i + 1 == blocks.len()));
        let n = b.len() as u16;
        out.extend(n.to_le_bytes());
        out.extend((!n).to_le_bytes());
        out.extend(*b);
    }
    let (mut a, mut s) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        s = (s + a) % 65521;
    }
    out.extend(((s << 16) | a).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 == 1 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn org(key: &str, name: &str, chart: Option<Chart>) -> Organism {
        let user = format!("{name}-{key}");
        Organism { key: key.into(), name: name.into(), repo: format!("{user}/{name}"), slot: format!("forgejo-{key}"), user, chart }
    }

    fn chart(positions: &[&str], persons: &[&str]) -> Chart {
        Chart { positions: positions.iter().map(|s| s.to_string()).collect(), persons: persons.iter().map(|s| s.to_string()).collect() }
    }

    /// Forgejo as a plan left it: every persona made, its slot held.
    fn held_after(p: &Plan) -> Held {
        let mut h = Held::default();
        for list in p.personas.values() {
            for x in list {
                h.users.insert(x.user.clone(), x.email.clone());
                h.slots.insert(x.slot.clone());
            }
        }
        h
    }

    const K: &str = "abcdef012345";
    const K2: &str = "0123456789ab";

    #[test]
    fn a_position_slug_maps_its_slash_to_a_double_dash() {
        assert_eq!(position_slug("position:board"), "board");
        assert_eq!(position_slug("position:fleet-operator"), "fleet-operator");
        assert_eq!(position_slug("position:api/dev"), "api--dev");
        // not plain: tidied, and a hash of the id keeps it its own
        let odd = position_slug("position:API Dev");
        assert!(odd.starts_with("api-dev-") && odd.len() == "api-dev-".len() + HASH_LEN, "{odd}");
        assert_ne!(position_slug("position:a/-b"), position_slug("position:a-/b"));
        assert_ne!(position_slug("position:api-dev"), position_slug("position:API-dev"));
        let long = position_slug(&format!("position:{}", "x".repeat(80)));
        assert!(long.len() <= SLUG_MAX, "{long}");
    }

    #[test]
    fn a_position_user_is_the_organisms_user_and_the_position() {
        let o = format!("todo-{K}");
        assert_eq!(position_user(&o, BOARD), format!("todo-{K}-board"));
        assert_eq!(position_user(&o, "position:api/dev"), format!("todo-{K}-api_dev"));
        // `api/dev` and `api-dev` never meet
        assert_ne!(position_user(&o, "position:api/dev"), position_user(&o, "position:api-dev"));
    }

    #[test]
    fn a_position_user_fits_forgejo_and_is_stable() {
        let o = format!("{}-{K}", "a".repeat(26));
        let id = "position:fleet-operator";
        let u = position_user(&o, id);
        assert!(u.len() <= USER_MAX, "{u}");
        assert_eq!(u, position_user(&o, id), "the same name from apply to apply");
        assert!(u.contains("fleet-operator"), "the position stays readable: {u}");
        // two organisms whose names cut alike never share a user
        let o2 = format!("{}-{K2}", "a".repeat(26));
        assert_ne!(u, position_user(&o2, id));
        // nor two positions whose names cut alike
        let a = position_user(&o, &format!("position:{}-one", "p".repeat(30)));
        let b = position_user(&o, &format!("position:{}-two", "p".repeat(30)));
        assert_ne!(a, b);
        for name in [u, a, b, position_user(&o, "position:Odd Name!")] {
            assert!(valid_forgejo_name(&name), "{name}");
        }
    }

    /// Forgejo's own rule for a user's name: a letter or digit first, then
    /// letters, digits, `-`, `_` and `.`, never two of those together and
    /// never one last.
    fn valid_forgejo_name(n: &str) -> bool {
        let b = n.as_bytes();
        let special = |c: u8| c == b'-' || c == b'_' || c == b'.';
        !b.is_empty() && n.len() <= USER_MAX && b[0].is_ascii_alphanumeric() && b.iter().all(|&c| c.is_ascii_alphanumeric() || special(c)) && !b.windows(2).any(|w| special(w[0]) && special(w[1])) && !special(b[b.len() - 1])
    }

    #[test]
    fn a_person_is_named_by_the_member_name() {
        assert_eq!(person_user("grace"), "grace");
        assert_eq!(person_persona("grace").slot, "forgejo-person-grace");
        assert_eq!(person_persona("grace").email, "grace@person.habitat.invalid");
        // the habitat's administrator, a name Forgejo keeps, a name not
        // plain: reshaped, with a hash
        for m in ["hale-admin", "admin", "Ada Lovelace", "x--y", &"m".repeat(50)] {
            let u = person_user(m);
            assert_ne!(u, m);
            assert!(valid_forgejo_name(&u), "{u}");
        }
        assert_ne!(person_user("Ada Lovelace"), person_user("ada lovelace"));
    }

    #[test]
    fn the_chart_is_read_from_its_lines() {
        let c = parse_chart("position position:board\nmandate position:board\nposition position:api/dev\nunder position:api/dev seed:api\nholds position:board grace\nholds position:api/dev organization:shop\nholds position:api/dev team:ops\nholds position:api/dev ada\nperson grace\npractice standard/x\n");
        assert_eq!(c.positions, vec!["position:board", "position:api/dev"]);
        assert_eq!(c.persons, vec!["grace", "ada"]);
    }

    #[test]
    fn a_first_reconcile_makes_every_persona_and_grants_it_the_repository() {
        let o = org(K, "todo", Some(chart(&["position:board", "position:dev"], &["grace"])));
        let p = plan(&[o], &Held::default(), true);
        let made: Vec<&str> = p.steps.iter().filter_map(|s| if let Step::Ensure { persona, mint: true } = s { Some(persona.user.as_str()) } else { None }).collect();
        assert_eq!(made, vec![format!("todo-{K}-board"), format!("todo-{K}-dev"), "grace".to_string()]);
        let granted = p.steps.iter().filter(|s| matches!(s, Step::Grant { repo, .. } if repo == &format!("todo-{K}/todo"))).count();
        assert_eq!(granted, 3, "the people too");
        assert_eq!(p.personas[K].iter().map(|x| x.slot.as_str()).collect::<Vec<_>>(), vec![format!("forgejo-{K}-board"), format!("forgejo-{K}-dev"), "forgejo-person-grace".to_string()]);
    }

    #[test]
    fn the_board_is_a_persona_stated_or_not() {
        let p = plan(&[org(K, "todo", Some(chart(&[], &[])))], &Held::default(), true);
        assert_eq!(p.personas[K].len(), 1);
        assert_eq!(p.personas[K][0].user, format!("todo-{K}-board"));
    }

    #[test]
    fn a_second_reconcile_makes_and_removes_nothing() {
        let orgs = [org(K, "todo", Some(chart(&["position:dev"], &["grace"]))), org(K2, "shop", Some(chart(&[], &["grace"])))];
        let first = plan(&orgs, &Held::default(), true);
        let second = plan(&orgs, &held_after(&first), true);
        assert!(second.steps.iter().all(|s| matches!(s, Step::Grant { .. })), "only the grants, each a read when it holds: {:?}", second.steps);
    }

    #[test]
    fn a_missing_slot_is_minted_again_and_the_user_left() {
        let orgs = [org(K, "todo", Some(chart(&["position:dev"], &[])))];
        let first = plan(&orgs, &Held::default(), true);
        let mut held = held_after(&first);
        held.slots.remove(&format!("forgejo-{K}-dev"));
        let p = plan(&orgs, &held, true);
        let ensures: Vec<_> = p.steps.iter().filter_map(|s| if let Step::Ensure { persona, mint } = s { Some((persona.user.clone(), *mint)) } else { None }).collect();
        assert_eq!(ensures, vec![(format!("todo-{K}-dev"), false)]);
    }

    #[test]
    fn a_retired_position_loses_its_user_and_its_slot() {
        let before = [org(K, "todo", Some(chart(&["position:dev", "position:ops"], &[])))];
        let held = held_after(&plan(&before, &Held::default(), true));
        let after = [org(K, "todo", Some(chart(&["position:dev"], &[])))];
        let p = plan(&after, &held, true);
        assert!(p.steps.contains(&Step::Remove { user: format!("todo-{K}-ops"), email: format!("todo-{K}-ops@{K}.persona.habitat.invalid") }));
        assert!(p.steps.contains(&Step::Forget { slot: format!("forgejo-{K}-ops") }));
        assert_eq!(p.steps.iter().filter(|s| matches!(s, Step::Remove { .. } | Step::Forget { .. })).count(), 2);
    }

    #[test]
    fn a_person_goes_only_when_no_chart_has_them() {
        let both = [org(K, "todo", Some(chart(&[], &["grace", "ada"]))), org(K2, "shop", Some(chart(&[], &["grace"])))];
        let held = held_after(&plan(&both, &Held::default(), true));
        // ada leaves todo; grace is still shop's
        let after = [org(K, "todo", Some(chart(&[], &[]))), org(K2, "shop", Some(chart(&[], &["grace"])))];
        let p = plan(&after, &held, true);
        let removed: Vec<&str> = p.steps.iter().filter_map(|s| if let Step::Remove { user, .. } = s { Some(user.as_str()) } else { None }).collect();
        assert_eq!(removed, vec!["ada"]);
        assert!(p.steps.contains(&Step::Forget { slot: "forgejo-person-ada".into() }));
        // a graft reconciles its own organism and removes no person
        let g = plan(&after[..1], &held, false);
        assert!(!g.steps.iter().any(|s| matches!(s, Step::Remove { user, .. } if user == "ada" || user == "grace")));
        // nor does an apply that could not read every chart
        let unread = [org(K, "todo", Some(chart(&[], &[]))), org(K2, "shop", None)];
        let u = plan(&unread, &held, true);
        assert!(!u.steps.iter().any(|s| matches!(s, Step::Remove { user, .. } if user == "ada" || user == "grace")));
    }

    #[test]
    fn an_unread_chart_keeps_what_its_organism_has() {
        let held = held_after(&plan(&[org(K, "todo", Some(chart(&["position:dev"], &[])))], &Held::default(), true));
        let p = plan(&[org(K, "todo", None)], &held, true);
        assert!(p.steps.is_empty(), "{:?}", p.steps);
    }

    #[test]
    fn a_removed_organisms_personas_go_at_the_next_apply() {
        let held = held_after(&plan(&[org(K, "todo", Some(chart(&["position:dev"], &[])))], &Held::default(), true));
        let p = plan(&[org(K2, "shop", Some(chart(&[], &[])))], &held, true);
        assert!(p.steps.contains(&Step::Remove { user: format!("todo-{K}-dev"), email: format!("todo-{K}-dev@{K}.persona.habitat.invalid") }));
        // a graft of another organism leaves them
        let g = plan(&[org(K2, "shop", Some(chart(&[], &[])))], &held, false);
        assert!(!g.steps.iter().any(|s| matches!(s, Step::Remove { .. })));
    }

    #[test]
    fn a_user_the_habitat_did_not_make_is_never_taken_or_removed() {
        let mut held = Held::default();
        held.users.insert("grace".into(), "grace@example.invalid".into());
        held.users.insert("someone".into(), "someone@example.invalid".into());
        held.users.insert("hale-admin".into(), "hale-admin@habitat.invalid".into());
        held.users.insert(format!("todo-{K}"), format!("todo-{K}@habitat.invalid"));
        let p = plan(&[org(K, "todo", Some(chart(&[], &["grace"])))], &held, true);
        assert!(p.conflicts.iter().any(|c| c.contains("grace")), "{:?}", p.conflicts);
        assert!(!p.steps.iter().any(|s| matches!(s, Step::Remove { user, .. } | Step::Grant { user, .. } if user == "grace")), "{:?}", p.steps);
        assert!(!p.steps.iter().any(|s| matches!(s, Step::Remove { .. })));
    }

    #[test]
    fn the_ledger_says_each_persona_once() {
        let o = org(K, "todo", Some(chart(&["position:api/dev"], &["grace"])));
        let p = plan(std::slice::from_ref(&o), &Held::default(), true);
        assert_eq!(ledger_text(&p.personas[K]), format!("position\tposition:board\ttodo-{K}-board\tforgejo-{K}-board\nposition\tposition:api/dev\ttodo-{K}-api_dev\tforgejo-{K}-api--dev\nperson\tgrace\tgrace\tforgejo-person-grace\n"));
    }

    #[test]
    fn a_chart_not_read_never_makes_an_apply_or_a_graft_refuse() {
        let unread = || {
            Ok(vec![
                Reconciled { key: K.into(), outcome: Outcome::Done("3 at the habitat's Forgejo".into()) },
                Reconciled { key: K2.into(), outcome: Outcome::Skipped("chart not read: hale dna chart: the record could not be read from memory".into()) },
            ])
        };
        let entries = applied_entries(unread());
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e["ok"] == serde_json::json!(true)), "{entries:?}");
        assert_eq!(entries[1]["allocation"], format!("{K2}/personas"));
        assert!(entries[1]["state"].as_str().unwrap().starts_with("skipped (chart not read: "));
        // nor a Forgejo that does not answer
        let down = applied_entries(Err("the habitat's Forgejo did not answer".into()));
        assert!(down[0]["ok"] == serde_json::json!(true) && down[0]["state"].as_str().unwrap().starts_with("skipped ("));
        // the graft says it as a note
        let lines = lines_of(unread());
        assert!(lines[0].starts_with(&format!("personas {K}: ")));
        assert!(lines[1].starts_with("note    the personas of ") && lines[1].contains("skipped (chart not read: "), "{lines:?}");
        assert!(lines_of(Err("x".into()))[0].starts_with("note    "));
    }

    #[test]
    fn an_avatar_is_a_png_drawn_from_its_seed() {
        let a = identicon("todo-abcdef012345-board");
        assert_eq!(&a[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(a, identicon("todo-abcdef012345-board"));
        assert_ne!(a, identicon("grace"));
        // every chunk's CRC holds
        let mut at = 8;
        while at < a.len() {
            let n = u32::from_be_bytes(a[at..at + 4].try_into().unwrap()) as usize;
            let crc = u32::from_be_bytes(a[at + 8 + n..at + 12 + n].try_into().unwrap());
            assert_eq!(crc, crc32(&a[at + 4..at + 8 + n]));
            at += 12 + n;
        }
        assert_eq!(at, a.len());
        assert!(a.len() < 2048, "{}", a.len());
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }
}
