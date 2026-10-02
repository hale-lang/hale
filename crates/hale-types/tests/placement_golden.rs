//! The placement table's golden (F.40 phase 3, P1).
//!
//! The table's shadow compared it, one column per legacy producer, with
//! every producer the snapshot could reach: the checker's per-type map,
//! F.31's owner-relative answer, the bus graph's and the ownership
//! graph's labels, the model's arrangement, the desugar's off-thread
//! fields and the resource budget. A column went as its consumer came to
//! read the table (P1, parts 2 to 5), and the shadow went with the last
//! of them. What it classified as a known old bug or a correction is the
//! table's answer, and here it is a row like any other.
//!
//! This is the table's own fixture: every row of the table, over the
//! seeds the shadow loaded, through the frontend's snapshot (the load,
//! the desugar sequence, the mint, the imports). The seeds are the
//! corpus (each program a bare snapshot, cited by its content id,
//! `hale_graph::shadow::program_id`, so the fixture does not churn when
//! a test file gains a literal above it), every `*_test.hl` under
//! `tests/hale` and `dna/`, every directory under `dna/` holding a
//! `main locus` that is not a test file's, and the table's own coverage
//! fixtures (`fixtures/placement/`), each cited by its path. A seed that
//! does not check clean, or whose table is empty, has no section.
//!
//! A section is the seed's root, the bound of each template not built
//! once, its domains other than main, its holes, one line per instance
//! row, and its dynamic sites. Nothing in it is a site id: a template is
//! named by its top's declaration and its place among the seed's
//! templates of its kind, an alternative by its place among the seed's
//! alternatives, so an edit that moves no placement fact moves no line.
//!
//! Two shapes keep it a size a review can read. The rows under one root
//! field, or of one other template, are a group written from its base
//! (`~`), and a group another seed holds too (the same library tree a
//! dozen DNA tests build) is written once, at the end, and cited by its
//! content's hash. The dynamic sites are written as their consumers read
//! them (`PlacementTable::domains_by_type`, the ownership graph's
//! pairing): per declaration built, the domains its literals run on and,
//! where the table cannot say, the hole's reason, a set per directory of
//! the enclosing source. How many literals say one thing, and how many
//! occurrences of each can be live, moves with any unrelated edit and no
//! consumer counts it; a declaration built only in code the seed does
//! not reach (a locus with no instance, a fn with no caller the table
//! places) has no line.
//!
//! Any difference fails, naming the seeds whose sections differ and
//! their first differing line. Regenerate under `HALE_SHADOW_REGEN=1`
//! (`cargo test --config 'env.HALE_SHADOW_REGEN="1"'`) once the change is
//! understood, and say in the commit which rows moved and why.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;
use hale_graph::shadow::program_id;
use hale_types::placement::{
    Bound, Decision, DomainId, DomainKind, Enclosing, HoleAt, HoleKind, InstanceKey, Origin, OwnerRelative,
    PlacementTable, SiteRef, SiteUniverse,
};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/placement_golden.txt")
}

/// A seed the golden loads.
enum Seed {
    /// A corpus program, loaded as a bare snapshot.
    Program { origin: String, source: String },
    /// A file or a directory on disk, loaded as `hale check` loads it.
    Path { origin: String, path: PathBuf },
}

fn hl_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            hl_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "hl") {
            out.push(p);
        }
    }
}

fn seeds() -> Vec<Seed> {
    let mut out: Vec<Seed> = hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok())
        .into_iter()
        .map(|p| Seed::Program { origin: p.origin, source: p.source })
        .collect();
    let root = hale_corpus::repo_root();
    let rel = |p: &Path| p.strip_prefix(&root).unwrap_or(p).display().to_string();
    let mut files = Vec::new();
    hl_files(&root.join("tests/hale"), &mut files);
    hl_files(&root.join("dna"), &mut files);
    let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
    for f in &files {
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with("_test.hl") {
            out.push(Seed::Path { origin: rel(f), path: f.clone() });
        } else if f.starts_with(root.join("dna"))
            && std::fs::read_to_string(f).is_ok_and(|s| s.lines().any(|l| l.trim_start().starts_with("main locus")))
        {
            dirs.insert(f.parent().unwrap().to_path_buf());
        }
    }
    for d in dirs {
        out.push(Seed::Path { origin: rel(&d), path: d });
    }
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/placement");
    let mut own = Vec::new();
    hl_files(&fixtures, &mut own);
    let mut own_dirs: BTreeSet<PathBuf> = BTreeSet::new();
    for f in own {
        let parent = f.parent().unwrap().to_path_buf();
        if parent == fixtures {
            out.push(Seed::Path { origin: rel(&f), path: f });
        } else if !parent.ends_with("lib") {
            own_dirs.insert(parent);
        }
    }
    for d in own_dirs {
        out.push(Seed::Path { origin: rel(&d), path: d });
    }
    out
}

/// The seed's id and its snapshot: a corpus program by its content, a
/// path by itself.
fn load(seed: &Seed) -> Option<(String, Snapshot)> {
    match seed {
        Seed::Program { origin, source } => {
            let program = hale_syntax::parse_source(source).ok()?;
            let s = Snapshot::from_program(program, Vec::new(), Config::check(false, false)).ok()?;
            Some((program_id(origin, source), s))
        }
        Seed::Path { origin, path } => {
            let s = Snapshot::load(path, LoadMode::WholeSeed, &Disk, Config::check(path.is_dir(), false)).ok()?;
            Some((origin.clone(), s))
        }
    }
}

// ------------------------------------------------------------ rendering

/// Names for one table's templates and alternatives, none of them a
/// site id.
struct Names<'t> {
    t: &'t PlacementTable,
    /// Each template's origin, by the name a key starts with.
    origins: BTreeMap<Origin, String>,
    /// Each alternative, by its place among the seed's alternatives.
    alternatives: BTreeMap<SiteRef, usize>,
}

impl<'t> Names<'t> {
    fn new(t: &'t PlacementTable) -> Names<'t> {
        let top_decl = |o: Origin| {
            t.instances
                .get(&InstanceKey { origin: o, path: Vec::new(), replica: None })
                .and_then(|r| r.realizes.as_ref())
                .map(|d| d.lowered.clone())
                .unwrap_or_else(|| "<hole>".into())
        };
        let roots: BTreeSet<SiteRef> = t.root.iter().flat_map(|r| &r.constructions).map(|c| c.literal).collect();
        let entry_literals: BTreeSet<SiteRef> = t.entry_literals.iter().map(|c| c.literal).collect();
        let mut all: BTreeSet<Origin> = t.instances.keys().map(|k| k.origin).collect();
        all.extend(roots.iter().chain(&entry_literals).map(|s| Origin::Construction(*s)));
        let mut counters: BTreeMap<&str, usize> = BTreeMap::new();
        let mut origins = BTreeMap::new();
        for o in all {
            let kind = match o {
                Origin::Entry(_) => "entry",
                Origin::Binding(_) => "binding",
                Origin::Construction(c) if roots.contains(&c) => "literal",
                Origin::Construction(_) => "main",
            };
            let n = counters.entry(kind).or_default();
            *n += 1;
            let name = match o {
                Origin::Entry(_) => format!("{}@entry", top_decl(o)),
                _ => format!("{}@{kind}{n}", top_decl(o)),
            };
            origins.insert(o, name);
        }
        let alternatives: BTreeSet<SiteRef> =
            t.instances.keys().flat_map(|k| k.path.iter().filter_map(|s| s.alternative)).collect();
        let alternatives = alternatives.into_iter().enumerate().map(|(i, s)| (s, i + 1)).collect();
        Names { t, origins, alternatives }
    }

    /// A key: its template, its fields (an alternative's place after the
    /// field that takes it), its replica after the replicated field.
    fn key(&self, k: &InstanceKey) -> String {
        let mut out = self.origins.get(&k.origin).cloned().unwrap_or_else(|| "?".into());
        for (i, step) in k.path.iter().enumerate() {
            out.push('.');
            out.push_str(&step.field);
            if let Some(a) = step.alternative {
                out.push_str(&format!("{{a{}}}", self.alternatives[&a]));
            }
            if i == 0 {
                if let Some(r) = k.replica {
                    out.push_str(&format!("[{r}]"));
                }
            }
        }
        if k.path.is_empty() {
            if let Some(r) = k.replica {
                out.push_str(&format!("[{r}]"));
            }
        }
        out
    }

    fn domain(&self, d: DomainId) -> String {
        match &self.t.domain(d).kind {
            DomainKind::Main => "main".into(),
            DomainKind::Pool { name, .. } => format!("pool:{name}"),
            DomainKind::Pinned { anchor, .. } => format!("pinned:{}", self.key(anchor)),
        }
    }
}

fn bound(b: &Bound) -> String {
    match b {
        Bound::Once => "once".into(),
        Bound::AtMost(n) => format!("at most {n}"),
        Bound::Unbounded(why) => format!("unbounded ({why})"),
    }
}

fn hole_kind(k: &HoleKind) -> String {
    match k {
        HoleKind::UnresolvedDeclaration { written } => format!("unresolved declaration `{written}`"),
        HoleKind::UnenumerableInitializer => "unenumerable initializer".into(),
        HoleKind::Reuse { source } => format!("reuse `{source}`"),
        HoleKind::UnresolvedArguments => "unresolved arguments".into(),
        HoleKind::EntryDecidesNothing { field } => format!("entry decides nothing (`{field}`)"),
        // The producer's reason, shortened: it repeats on most lines.
        HoleKind::UnknownDomains { reason } => {
            let short = match reason.as_str() {
                "the enclosing locus has no instance the table places" => "no instance".to_string(),
                "the enclosing fn has no caller the table places" => "no caller".to_string(),
                r => match r.strip_suffix(" is passed as a value, so its callers are not all known") {
                    Some(f) => format!("{f} passed as a value"),
                    None => r.to_string(),
                },
            };
            format!("unknown ({short})")
        }
    }
}

/// One seed's section: its head (the root, its templates and their
/// tops' rows, the domains, the holes), then its groups. A group two
/// seeds share is written once ([`golden`]).
struct Section {
    head: Vec<String>,
    groups: Vec<Group>,
}

/// Lines a seed holds as one: the rows under a root field or of another
/// template, written from their base; the dynamic sites on known domains.
struct Group {
    /// Where in the seed: the base's key, or `dynamic`.
    at: String,
    /// What the lines are of: the base's declaration, or the sites.
    what: String,
    lines: Vec<String>,
}

/// One seed's section: every row of its table. `None` for an empty
/// table.
fn section(t: &PlacementTable, seeds: &[String], repo_root: &str) -> Result<Option<Section>, String> {
    if t.root.is_none() && t.instances.is_empty() && t.dynamic.is_empty() && t.holes.is_empty() {
        return Ok(None);
    }
    let names = Names::new(t);
    let mut out = Vec::new();
    if let Some(r) = &t.root {
        out.push(format!("root\t{}\t{}", r.realizes.lowered, if r.is_entry { "the entry" } else { "not the entry" }));
    }
    // A template is built once unless its line says otherwise.
    let mut templates: Vec<String> = t
        .templates()
        .iter()
        .filter(|(_, b)| *b != Bound::Once)
        .map(|(k, b)| format!("template\t{}\t{}", names.key(k), bound(b)))
        .collect();
    templates.sort();
    templates.dedup();
    out.extend(templates);
    for d in t.domains.iter().filter(|d| d.kind != DomainKind::Main) {
        let mut line = format!("domain\t{}", names.domain(d.id));
        let cpus = |c: &hale_types::placement::CoreSet| {
            c.0.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
        };
        match &d.kind {
            DomainKind::Main => unreachable!("main is every table's first domain, and not written"),
            DomainKind::Pool { async_io, affinity, .. } => {
                if *async_io {
                    line.push_str("\tasync_io");
                }
                if let Some(c) = affinity {
                    line.push_str(&format!("\tcpus {}", cpus(c)));
                }
            }
            DomainKind::Pinned { affinity, numa_node, .. } => {
                if let Some(c) = affinity {
                    line.push_str(&format!("\tcpus {}", cpus(c)));
                }
                if let Some(n) = numa_node {
                    line.push_str(&format!("\tnuma {n}"));
                }
            }
        }
        out.push(line);
    }
    // A dynamic site's holes are written on its own line, below.
    let mut at_dynamic: BTreeMap<SiteRef, Vec<String>> = BTreeMap::new();
    for h in &t.holes {
        match &h.at {
            HoleAt::Instance(k) => out.push(format!("hole\t{}\t{}", names.key(k), hole_kind(&h.kind))),
            HoleAt::Entry(_) => out.push(format!("hole\tthe entry\t{}", hole_kind(&h.kind))),
            HoleAt::Dynamic(s) => at_dynamic.entry(*s).or_default().push(hole_kind(&h.kind)),
        }
    }
    // The rows, a group per root field (the field's row and everything
    // under it) and per other template (its top and everything under it),
    // each key and anchor written from the group's base, `~`: two seeds
    // that build one tree share its group. A root template's top is in
    // the head.
    let root_template = |o: Origin| match o {
        Origin::Entry(_) => true,
        Origin::Construction(c) => t.root.as_ref().is_some_and(|r| r.constructions.iter().any(|x| x.literal == c)),
        Origin::Binding(_) => false,
    };
    // Per base: its declaration, and each row written from the base and
    // whole (a group of one row is written whole, in the head).
    let mut groups: BTreeMap<String, (String, Vec<(String, String)>)> = BTreeMap::new();
    let mut keys: BTreeMap<String, &InstanceKey> = BTreeMap::new();
    for (k, r) in &t.instances {
        let key = names.key(k);
        if let Some(other) = keys.insert(key.clone(), k) {
            return Err(format!("two rows render as `{key}`: {other:?} and {k:?}"));
        }
        let base = match (root_template(k.origin), k.path.len()) {
            (true, 0) => None,
            (true, _) => Some(InstanceKey { origin: k.origin, path: k.path[..1].to_vec(), replica: k.replica }),
            (false, _) => Some(InstanceKey { origin: k.origin, path: Vec::new(), replica: None }),
        };
        let base_key = base.as_ref().map(|b| names.key(b));
        let line = |from: Option<&str>| {
            let from_base = |s: String| match from {
                Some(b) if s.starts_with(b) => format!("~{}", &s[b.len()..]),
                _ => s,
            };
            let mut line = format!(
                "row\t{}\t{}\t{}\t{}",
                from_base(key.clone()),
                r.realizes.as_ref().map(|d| d.lowered.as_str()).unwrap_or("<hole>"),
                match names.domain(r.domain).split_once(':') {
                    Some(("pinned", anchor)) => format!("pinned:{}", from_base(anchor.to_string())),
                    _ => names.domain(r.domain),
                },
                match &r.decided_by {
                    Decision::Entry { .. } => "entry",
                    Decision::Binding { .. } => "binding",
                    Decision::Inherited { .. } => "inherited",
                    Decision::Default => "default",
                },
            );
            // Same as its owner unless the line says otherwise.
            if r.owner_relative == OwnerRelative::OffOwner {
                line.push_str("\toff owner");
            }
            if r.guarded {
                line.push_str("\tguarded");
            }
            if let Some(b) = &r.built_by {
                line.push_str(&format!("\tbuilt by {}", from_base(names.key(b))));
            }
            line
        };
        match &base_key {
            None => out.push(line(None)),
            Some(b) => {
                let decl = base
                    .as_ref()
                    .and_then(|b| t.instances.get(b))
                    .and_then(|r| r.realizes.as_ref())
                    .map_or_else(|| "<hole>".to_string(), |d| d.lowered.clone());
                let g = groups.entry(b.clone()).or_insert_with(|| (decl, Vec::new()));
                g.1.push((line(Some(b)), line(None)));
            }
        }
    }
    let mut rows: Vec<Group> = Vec::new();
    for (base, (decl, lines)) in groups {
        if let [(_, whole)] = &lines[..] {
            out.push(whole.clone());
        } else {
            rows.push(Group { at: base, what: decl, lines: lines.into_iter().map(|(from_base, _)| from_base).collect() });
        }
    }
    // The dynamic sites, by what a consumer reads of them
    // (`domains_by_type`, the ownership graph's pairing): per declaration
    // built, every domain a literal of it runs on, `unknown` where the
    // table cannot say (with the holes' reasons), per directory of the
    // source that encloses the literals. How many literals say the same
    // thing, and how many of each can be live, is the source's to say and
    // moves with every unrelated edit; no consumer counts them.
    let mut dynamic: BTreeMap<String, BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();
    for s in &t.dynamic {
        let enclosing = match &s.enclosing {
            Enclosing::Locus(d) => d.site,
            Enclosing::Fn(f) => *f,
        };
        let from = match enclosing.universe {
            SiteUniverse::StdlibAnalysis => "the stdlib".to_string(),
            SiteUniverse::User => seeds
                .get(enclosing.id.seed.0 as usize)
                .and_then(|f| Path::new(f.strip_prefix(repo_root).unwrap_or(f)).parent().map(|p| p.display().to_string()))
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| "the seed".to_string()),
        };
        let on = dynamic
            .entry(from)
            .or_default()
            .entry(s.realizes.as_ref().map_or_else(|| "<hole>".to_string(), |d| d.lowered.clone()))
            .or_default();
        on.extend(s.domains.iter().map(|d| names.domain(*d)));
        let holes = at_dynamic.get(&s.literal).map(Vec::as_slice).unwrap_or_default();
        on.extend(holes.iter().cloned());
        if s.domains.is_empty() && holes.is_empty() {
            on.insert("unknown".to_string());
        }
    }
    let literals: BTreeSet<SiteRef> = t.dynamic.iter().map(|s| s.literal).collect();
    for (s, kinds) in &at_dynamic {
        if !literals.contains(s) {
            return Err(format!("a hole at a dynamic site no dynamic row names: {kinds:?}"));
        }
    }
    let mut groups = rows;
    // A declaration built only in code the seed does not reach (a locus
    // with no instance, a fn with no caller the table places) has no line:
    // it is every library's code the seed imports and never runs, and it
    // runs nowhere the table can say.
    let unreached = ["unknown (no instance)", "unknown (no caller)"];
    for (from, built) in dynamic {
        let lines: Vec<String> = built
            .into_iter()
            .filter(|(_, on)| !on.iter().all(|o| unreached.contains(&o.as_str())))
            .map(|(decl, on)| format!("dynamic\t{decl}\ton {}", on.into_iter().collect::<Vec<_>>().join("|")))
            .collect();
        match &lines[..] {
            [] => {}
            [line] => out.push(format!("{line}\tin {from}")),
            _ => groups.push(Group { at: "dynamic".to_string(), what: format!("sites in {from}"), lines }),
        }
    }
    Ok(Some(Section { head: out, groups }))
}

/// Every seed's section, in seed order: the golden's text, and how many
/// seeds and rows it holds.
struct Golden {
    text: String,
    seeds: usize,
    rows: usize,
}

fn golden() -> Golden {
    let seeds = seeds();
    // Interleaved, not chunked: the slow seeds (the DNA's) sit together
    // at the end of the list.
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    let seeds = &seeds;
    type Loaded = (usize, String, Result<Option<Section>, String>, usize);
    let mut sections: Vec<Loaded> =
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..workers)
                .map(|w| {
                    scope.spawn(move || {
                        (w..seeds.len())
                            .step_by(workers)
                            .filter_map(|i| {
                                let (id, s) = load(&seeds[i])?;
                                let checked = s.demand_check().ok()?;
                                if checked.diags.iter().any(|d| d.is_error()) {
                                    return None;
                                }
                                let t = s.demand_placement().ok()?;
                                let root = hale_corpus::repo_root().display().to_string() + "/";
                                let section = section(t, &s.bundle().snapshot.seeds, &root);
                                Some((i, id, section, t.instances.len()))
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().expect("a golden worker")).collect()
        });
    sections.sort_by_key(|(i, ..)| *i);
    let mut ids = BTreeSet::new();
    let mut kept: Vec<(String, Section)> = Vec::new();
    let mut rows = 0;
    for (_, id, section, n_rows) in sections {
        assert!(ids.insert(id.clone()), "two seeds are cited as `{id}`");
        let Some(section) = section.unwrap_or_else(|e| panic!("{id}: {e}")) else { continue };
        rows += n_rows;
        kept.push((id, section));
    }
    // A group two seeds share, as exactly the same lines of the same
    // thing, is written once at the end and cited from each by what it is
    // of and its content's hash.
    let mut uses: BTreeMap<(&str, &[String]), usize> = BTreeMap::new();
    for (_, s) in &kept {
        for g in &s.groups {
            *uses.entry((g.what.as_str(), g.lines.as_slice())).or_default() += 1;
        }
    }
    let shared = |g: &Group| uses[&(g.what.as_str(), g.lines.as_slice())] > 1 && g.lines.len() > 1;
    let name = |g: &Group| format!("{} {}", g.what, program_id("", &g.lines.join("\n")));
    let mut text = String::from(HEADER);
    let mut blocks: BTreeMap<String, &[String]> = BTreeMap::new();
    for (id, s) in &kept {
        text.push_str(&format!("\n== {id}\n"));
        for l in &s.head {
            text.push_str(l);
            text.push('\n');
        }
        for g in &s.groups {
            if shared(g) {
                let name = name(g);
                text.push_str(&format!("shared\t{}\t{name}\n", g.at));
                if let Some(other) = blocks.insert(name.clone(), &g.lines) {
                    assert_eq!(other, g.lines.as_slice(), "two shared groups are cited as `{name}`");
                }
            } else {
                text.push_str(&format!("-- {}\t{}\n", g.at, g.what));
                for l in &g.lines {
                    text.push_str(l);
                    text.push('\n');
                }
            }
        }
    }
    for (name, lines) in &blocks {
        text.push_str(&format!("\n== shared {name}\n"));
        for l in *lines {
            text.push_str(l);
            text.push('\n');
        }
    }
    Golden { text, seeds: kept.len(), rows }
}

const HEADER: &str = "\
# The placement table's golden (F.40 phase 3, P1): every row of the table, per seed.
# Generated by `placement_golden.rs` under HALE_SHADOW_REGEN=1. A section is `== <seed>`
# (a corpus program by its content id, a path by itself), then, tab-separated:
#   root      <declaration> <the entry | not the entry>        (none: no root deployed)
#   template  <key of its top> <bound>                        (none: built once)
#   domain    <pool:<name> | pinned:<anchor key>> [async_io] [cpus <set>] [numa <node>]
#   hole      <key | the entry> <what>
#   row       <key> <realizes> <domain> <decided by> [off owner] [guarded] [built by <key>]
#   dynamic   <realizes> on <domains | unknown (why)>, joined by `|` [in <directory>]
# A key is <top declaration>@<entry | literalN | mainN | bindingN>, then .field per step
# ({aN} after a field whose alternative it takes, [i] after a replicated field). Main is
# every table's first domain. The rows under one root field, or of one other template,
# are a group, `-- <base key> <declaration>` and its rows written from the base (`~`);
# the dynamic sites, a group per directory of their enclosing source, `-- dynamic sites in
# <directory>`. A group another seed has too is `shared <at> <what> #<hash>`, written once
# under `== shared <what> #<hash>` at the end. A declaration built only in code the seed
# does not reach has no dynamic line.
";

/// The seeds whose sections differ, each with its first differing line.
fn differences(old: &str, new: &str) -> Vec<String> {
    let split = |text: &str| -> BTreeMap<String, Vec<String>> {
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut cur: Option<String> = None;
        for l in text.lines() {
            if let Some(id) = l.strip_prefix("== ") {
                cur = Some(id.to_string());
                out.entry(id.to_string()).or_default();
            } else if let Some(id) = &cur {
                if !l.is_empty() {
                    out.get_mut(id).unwrap().push(l.to_string());
                }
            }
        }
        out
    };
    let (old, new) = (split(old), split(new));
    let mut out = Vec::new();
    for id in old.keys().chain(new.keys()).collect::<BTreeSet<_>>() {
        match (old.get(id), new.get(id)) {
            (Some(_), None) => out.push(format!("{id}: the section is gone")),
            (None, Some(_)) => out.push(format!("{id}: a new section")),
            (Some(a), Some(b)) if a != b => {
                let i = a.iter().zip(b).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
                out.push(format!(
                    "{id}: line {} was `{}`, is `{}`",
                    i + 1,
                    a.get(i).map(String::as_str).unwrap_or("<none>"),
                    b.get(i).map(String::as_str).unwrap_or("<none>")
                ));
            }
            _ => {}
        }
    }
    out
}

#[test]
fn the_table_is_its_golden() {
    let g = golden();
    assert!(g.seeds > 300, "the golden is vacuous ({} seeds with a table)", g.seeds);
    assert!(g.rows > 1000, "the golden is vacuous ({} rows)", g.rows);
    eprintln!("placement golden: {} seeds with a table, {} instance rows", g.seeds, g.rows);
    let path = fixture_path();
    if std::env::var("HALE_SHADOW_REGEN").as_deref() == Ok("1") {
        std::fs::write(&path, &g.text).expect("write the golden");
        return;
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if existing != g.text {
        let diffs = differences(&existing, &g.text);
        panic!(
            "the placement table moved in {} seed(s):\n{}\nRegenerate \
             crates/hale-types/tests/fixtures/placement_golden.txt under HALE_SHADOW_REGEN=1 once the change \
             is understood.",
            diffs.len(),
            diffs.iter().take(40).cloned().collect::<Vec<_>>().join("\n")
        );
    }
}
