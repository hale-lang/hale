//! The placement table against every legacy producer (F.40 phase 3, P1).
//!
//! The placement table's shadow
//! (`notes/f40-placement-correspondence.md` § 3, hale-lang/hale#1296). It
//! loads every seed through the frontend's snapshot — the load, the
//! desugar sequence, the mint, the imports — and compares the table
//! (`Snapshot::demand_placement`) with each legacy producer the snapshot
//! can reach, one column each, every key prefixed by its column:
//!
//! - `budget`: `budget_for_programs`' threads and pools.
//!
//! The seeds are the corpus (each program a bare snapshot), every
//! `*_test.hl` under `tests/hale` and `dna/`, every directory under `dna/`
//! holding a `main locus` that is not a test file's, and the table's own
//! coverage fixtures (`fixtures/placement/`). Codegen's
//! `collect_main_placement` and its `DeploymentPlan` are the two legacy
//! producers this shadow cannot run: they exist only inside lowering's
//! context, and the lowering view (P1's PR 2) is where they are compared.
//! The checker's per-type map (`compute_pool_of_locus_type`) and F.31's
//! owner-relative answer at `self.f` (`enclosing_field_placement`) had a
//! column each, and the phase-0 shadow compared the map with the bus
//! graph's, until the F.31 rule, sync inference and the blocking check
//! read the table (P1's checker switch): their classified divergences
//! became the diagnostics that switch pins. The intra-locus rewrite's
//! off-owner-thread fields (`collect_off_owner_thread_fields`) had one
//! too, until the rewrite read the table's `off_owner_fields` (P1's part
//! 4): its only divergences were an imported root's entries, which
//! lowering never deploys (fixture F-R's principle). The model's
//! arrangement (`PlacedIn`, per instance path) had one until it became
//! the table's rows, projected (the same part): its M-1, M-3, M-5 and
//! M-9 divergences are the arrangement moves `model_arrangement.rs`
//! pins, and what stays different (stdlib rows, U-4; `fn main`'s other
//! literals, M-7; paths whose templates disagree, contract 3) is the
//! projection's stated coverage, not a legacy answer.
//!
//! **The gate.** The comparison runs per row, in memory, and `classify`
//! names the correspondence's rows (§ 2, § 10) that explain each
//! divergence from its witnesses; a divergence no row explains fails.
//! The committed fixture, `fixtures/shadow_placement_table.txt`, is one
//! line per (producer, design rows, declaration realized) with its class,
//! its count and one seed that shows it — never a line per instance, since
//! a library type repeats in every seed that imports it. A class with no
//! line, a count that moved, a witness that no longer shows its line, or a
//! line no divergence matches fails too. Regenerate under
//! `HALE_SHADOW_REGEN=1` (`cargo test --config 'env.HALE_SHADOW_REGEN="1"'`)
//! once the change is understood. The dynamic sites of unknown domain are
//! holes; their count is printed, not pinned.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;
use hale_graph::shadow::{gate_message, program_id, Class, Divergence, Report};
use hale_syntax::ast::LocusMember;
use hale_types::placement::{Bound, DomainKind, HoleAt, HoleKind, InstanceKey, Origin, PlacementTable};

// ---------------------------------- the table against every legacy producer

fn table_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/shadow_placement_table.txt")
}

/// A seed the table's shadow loads.
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

fn joined(set: &BTreeSet<String>) -> String {
    set.iter().cloned().collect::<Vec<_>>().join("|")
}

/// One column of one seed: the legacy producer's rows and the table's,
/// each under its own key (the legacy side keyed natively, with the
/// correspondence that maps it), and what decided each table key.
struct Column {
    name: &'static str,
    old: Vec<(String, String)>,
    new: Vec<(String, String)>,
    /// Legacy native key → shared key, where the two differ.
    map_old: BTreeMap<String, String>,
    witness: BTreeMap<String, Vec<String>>,
    /// What explains a legacy row the table has no key for.
    global: Vec<String>,
    slice: &'static str,
    /// Shared key → the declarations its rows realize, where the key does
    /// not name one itself (a type-keyed column's key is its declaration).
    decl: BTreeMap<String, String>,
}

struct Shadowed {
    id: String,
    columns: Vec<Column>,
    /// The dynamic sites whose domains are unknown: holes, counted and
    /// not pinned.
    unknown_dynamic: usize,
}

fn load(seed: &Seed) -> Option<(String, Snapshot)> {
    match seed {
        Seed::Program { origin, source } => {
            let program = hale_syntax::parse_source(source).ok()?;
            let s = Snapshot::from_program(program, Vec::new(), Config::check(false, false)).ok()?;
            Some((program_id(origin, source), s))
        }
        Seed::Path { origin, path } => {
            let s = Snapshot::load(path, LoadMode::WholeSeed, &Disk, Config::check(path.is_dir(), false)).ok()?;
            let text: String = s.sources().values().cloned().collect::<Vec<_>>().join("\n");
            Some((program_id(origin, &text), s))
        }
    }
}

/// The table's view of a key: its path as the model spells it, and the
/// prefix keys from its origin's top down to itself.
struct Paths<'t> {
    t: &'t PlacementTable,
    root: String,
}

impl Paths<'_> {
    /// Whether `o` is a template of the root: one of its literals, or the
    /// entry's implicit one.
    fn is_root(&self, o: Origin) -> bool {
        match o {
            Origin::Entry(_) => true,
            Origin::Construction(c) => self.t.root.as_ref().is_some_and(|r| r.constructions.iter().any(|x| x.literal == c)),
            Origin::Binding(_) => false,
        }
    }

    /// The name a path from `o`'s top starts with: the root's, or the
    /// declaration a literal `fn main` builds realizes.
    fn top(&self, o: Origin) -> String {
        match o {
            Origin::Binding(_) => "<binding>".into(),
            _ if self.is_root(o) => self.root.clone(),
            _ => self
                .t
                .instances
                .get(&InstanceKey { origin: o, path: Vec::new(), replica: None })
                .and_then(|r| r.realizes.as_ref())
                .map(|d| d.lowered.clone())
                .unwrap_or_else(|| "<entry literal>".into()),
        }
    }

    fn path(&self, k: &InstanceKey) -> String {
        let mut p = self.top(k.origin);
        for (i, step) in k.path.iter().enumerate() {
            p.push('.');
            p.push_str(&step.field);
            if i == 0 {
                if let Some(r) = k.replica {
                    p.push_str(&format!("[{r}]"));
                }
            }
        }
        p
    }

}

fn shadow_seed(seed: &Seed) -> Option<Shadowed> {
    let (id, s) = load(seed)?;
    let checked = s.demand_check().ok()?;
    if checked.diags.iter().any(|d| d.is_error()) {
        return None;
    }
    let t = s.demand_placement().ok()?;
    let entry = s.demand_entry().ok()?;
    let bundle = s.bundle();
    let paths = Paths { t, root: t.root.as_ref().map(|r| r.decl.name.clone()).unwrap_or_default() };
    let mut columns = Vec::new();

    // What explains a legacy row the table has no key for.
    let mut global: Vec<String> = Vec::new();
    for m in entry.mains.iter().filter(|m| Some(*m) != entry.lowering_root.as_ref()) {
        let placed = m.decl(&bundle).is_some_and(|l| {
            l.members.iter().any(|x| matches!(x, LocusMember::Placement(pb) if !pb.entries.is_empty()))
        });
        // Lowering's own test is the name (`collect_main_placement`): a
        // program handed in already renamed carries the name alone.
        let why = if m.imported || m.name.starts_with("__lib_") {
            "imported"
        } else if m.module_nested {
            "module-nested"
        } else {
            "own"
        };
        global.push(format!("cause: non-root main `{}` ({why}){}", m.name, if placed { ", with entries" } else { "" }));
    }

    // The bus, ownership, intra-locus rewrite and model arrangement now
    // consume the table. Only the budget remains a legacy column here.

    // budget: threads and pools.
    let programs: Vec<&hale_syntax::ast::Program> = bundle.programs.values().copied().collect();
    let summary = s.demand_alloc_summary().ok()?;
    let legacy = hale_types::resource_budget::budget_for_programs(&programs, summary);
    let mut threads: Result<u64, String> = Ok(0);
    let mut budget_why = global.clone();
    for d in &t.domains {
        let DomainKind::Pinned { anchor, .. } = &d.kind else { continue };
        let times = match anchor.origin {
            Origin::Binding(_) => {
                budget_why.push(format!("cause: an adapter anchor `{}`", paths.path(anchor)));
                Ok(1)
            }
            // The entry's implicit template runs once.
            Origin::Entry(_) => Ok(1),
            Origin::Construction(c) => {
                if anchor.replica.is_some() {
                    budget_why.push(format!("cause: a replica anchor `{}`", paths.path(anchor)));
                }
                let mut built = t.root.iter().flat_map(|r| r.constructions.iter()).chain(t.entry_literals.iter());
                match built.find(|x| x.literal == c).map(|x| &x.bound) {
                    Some(Bound::Once) => Ok(1),
                    Some(Bound::AtMost(n)) => Ok(*n as u64),
                    Some(Bound::Unbounded(why)) => Err(why.clone()),
                    None => Err("no construction".to_string()),
                }
            }
        };
        threads = match (threads, times) {
            (Ok(a), Ok(b)) => Ok(a + b),
            (Err(e), _) | (_, Err(e)) => Err(e),
        };
    }
    if let Some(r) = &t.root {
        if r.constructions.len() > 1 || r.constructions.iter().any(|c| c.bound != Bound::Once) {
            budget_why.push(format!("cause: {} constructions of the root", r.constructions.len()));
        }
    }
    if legacy.cooperative_pools.contains("main") {
        budget_why.push("cause: `main` spelled as a pool".into());
    }
    let pools: BTreeSet<String> = t
        .domains
        .iter()
        .filter_map(|d| match &d.kind {
            DomainKind::Pool { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    columns.push(Column {
        name: "budget",
        old: vec![
            ("threads".into(), legacy.pinned_threads.to_string()),
            ("pools".into(), joined(&legacy.cooperative_pools)),
        ],
        new: vec![
            ("threads".into(), threads.map(|n| n.to_string()).unwrap_or_else(|why| format!("uncertain ({why})"))),
            ("pools".into(), joined(&pools)),
        ],
        map_old: BTreeMap::new(),
        witness: [("threads".to_string(), budget_why.clone()), ("pools".to_string(), budget_why)].into_iter().collect(),
        global: Vec::new(),
        slice: "--dump-resource-budget, --check-resource-budget",
        decl: [("threads".to_string(), "-".to_string()), ("pools".to_string(), "-".to_string())].into_iter().collect(),
    });
    let unknown_dynamic = t
        .holes
        .iter()
        .filter(|h| matches!((&h.at, &h.kind), (HoleAt::Dynamic(_), HoleKind::UnknownDomains { .. })))
        .count();
    Some(Shadowed { id, columns, unknown_dynamic })
}

/// Every seed shadowed, in seed order.
struct TableReport {
    /// The seeds that checked clean.
    seeds: usize,
    report: Report,
    /// Per divergence, in the report's order: the declarations its rows
    /// realize.
    decls: Vec<String>,
    /// The dynamic sites of unknown domain, over every seed.
    unknown_dynamic: usize,
}

fn table_report() -> TableReport {
    let seeds = seeds();
    // Interleaved, not chunked: the slow seeds (the DNA's) sit together
    // at the end of the list.
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    let seeds = &seeds;
    let mut shadowed: Vec<(usize, Shadowed)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|w| {
                scope.spawn(move || {
                    (w..seeds.len())
                        .step_by(workers)
                        .filter_map(|i| shadow_seed(&seeds[i]).map(|x| (i, x)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("a shadow worker")).collect()
    });
    shadowed.sort_by_key(|(i, _)| *i);
    let mut report = Report::new("placement");
    let mut decls = Vec::new();
    for (_, sh) in &shadowed {
        for c in &sh.columns {
            let prefix = |k: &str| format!("{}:{k}", c.name);
            let before = report.divergences.len();
            report.compare_rows(
                &sh.id,
                &c.old,
                &c.new,
                |k: &String| Some(prefix(c.map_old.get(k).map(String::as_str).unwrap_or(k))),
                |k: &String| Some(prefix(k)),
                |k: &String| {
                    let native = k.split_once(':').map(|(_, n)| n).unwrap_or(k);
                    c.witness.get(native).cloned().unwrap_or_else(|| c.global.clone())
                },
                |_| vec![c.slice.to_string()],
            );
            for d in &report.divergences[before..] {
                let native = d.key.split_once(':').map(|(_, n)| n).unwrap_or(&d.key);
                decls.push(c.decl.get(native).cloned().unwrap_or_else(|| native.to_string()));
            }
        }
    }
    let unknown_dynamic = shadowed.iter().map(|(_, sh)| sh.unknown_dynamic).sum();
    TableReport { seeds: shadowed.len(), report, decls, unknown_dynamic }
}

// ------------------------------------------------------ classification

/// The correspondence's rows a divergence is classified under
/// (`notes/f40-placement-correspondence.md` § 2 and § 10), each with its
/// class and what it says. Several rows on one divergence take the most
/// open class among them.
const DESIGN_ROWS: &[(&str, Class, &str)] = &[
    ("R-1", Class::KnownOldBug, "`replicas = K` is K threads. § 2.8"),
    ("R-4", Class::KnownOldBug, "a non-root main's entries are counted. § 2.8"),
    ("R-5", Class::KnownOldBug, "the root's constructions times their bounds. § 2.8"),
    ("R-6", Class::SpecDisagreement, "an adapter anchor is a thread (U-3, pending the owner). § 2.8"),
    ("R-7", Class::SpecDisagreement, "`main` is counted as a pool when spelled (U-2, pending the owner). § 2.8"),
];

fn design_row(id: &str) -> (Class, &'static str) {
    let (_, class, note) =
        DESIGN_ROWS.iter().find(|(i, _, _)| *i == id).unwrap_or_else(|| panic!("no design row `{id}`"));
    (*class, note)
}

fn class_label(c: Class) -> &'static str {
    match c {
        Class::Unclassified => "unclassified",
        Class::Regression => "regression",
        Class::KnownOldBug => "known-old-bug",
        Class::Correction => "correction",
        Class::SpecDisagreement => "spec-disagreement",
    }
}

/// How open a class leaves the question, least first.
fn openness(c: Class) -> u8 {
    match c {
        Class::KnownOldBug => 0,
        Class::Correction => 1,
        Class::SpecDisagreement => 2,
        Class::Unclassified | Class::Regression => 3,
    }
}

/// The design rows that explain a divergence, or `None` when none does:
/// a divergence the design does not name fails the gate.
fn classify(col: &str, d: &Divergence) -> Option<Vec<&'static str>> {
    let causes: Vec<&str> = d.witnesses.iter().filter_map(|w| w.strip_prefix("cause: ")).collect();
    let ids = match (col, d.kind) {
        ("budget", _) => {
            let mut ids = BTreeSet::new();
            for c in &causes {
                if c.starts_with("an adapter anchor") {
                    ids.insert("R-6");
                }
                if c.starts_with("a replica anchor") {
                    ids.insert("R-1");
                }
                if c.contains("constructions of the root") {
                    ids.insert("R-5");
                }
                if c.ends_with("), with entries") {
                    ids.insert("R-4");
                }
                if c.contains("`main` spelled") {
                    ids.insert("R-7");
                }
            }
            if ids.is_empty() {
                return None;
            }
            ids.into_iter().collect()
        }
        _ => return None,
    };
    Some(ids)
}

// ----------------------------------------------------------- the fixture

/// A fixture line's key: the producer's column, the design rows, the
/// declaration realized.
type GroupKey = (String, String, String);

/// What the fixture pins per key: the class, how many divergences, and
/// one seed that shows it.
struct Line {
    class: Class,
    count: usize,
    witness: String,
}

const FIXTURE_HEADER: &str = "\
# The placement table's shadow (F.40 phase 3, P1): its divergences from every legacy
# producer, one line per (producer, design rows, declaration realized), counted, with one
# seed that shows it. Generated by `shadow_placement.rs` under HALE_SHADOW_REGEN=1; the
# classification is the test's (`classify`, against the design rows it cites), and a
# divergence no design row explains fails in either mode. Columns:
# producer <TAB> design rows <TAB> declaration <TAB> class <TAB> divergences <TAB> witness seed
";

fn parse_lines(text: &str) -> Result<BTreeMap<GroupKey, Line>, String> {
    let mut out = BTreeMap::new();
    for (i, l) in text.lines().enumerate() {
        if l.trim().is_empty() || l.starts_with('#') {
            continue;
        }
        let c: Vec<&str> = l.split('\t').collect();
        let [col, rows, decl, class, count, witness] = c[..] else {
            return Err(format!("line {}: expected 6 tab-separated columns, found {}", i + 1, c.len()));
        };
        let class = [Class::KnownOldBug, Class::Correction, Class::SpecDisagreement]
            .into_iter()
            .find(|k| class_label(*k) == class)
            .ok_or_else(|| format!("line {}: unknown class `{class}`", i + 1))?;
        let count = count.parse().map_err(|_| format!("line {}: `{count}` is no count", i + 1))?;
        let key = (col.to_string(), rows.to_string(), decl.to_string());
        if out.insert(key, Line { class, count, witness: witness.to_string() }).is_some() {
            return Err(format!("line {}: a second line for one key", i + 1));
        }
    }
    Ok(out)
}

#[test]
fn the_table_and_every_legacy_producer_agree_or_every_divergence_is_classified() {
    let r = table_report();
    assert!(r.seeds > 300, "the table's shadow is vacuous ({} seeds checked clean)", r.seeds);
    let mut unclassified = Vec::new();
    // Each group's class, count, and the seeds that show it, in seed order.
    let mut groups: BTreeMap<GroupKey, (Class, usize, Vec<&str>)> = BTreeMap::new();
    for (d, decl) in r.report.divergences.iter().zip(&r.decls) {
        let col = d.key.split_once(':').map(|(c, _)| c).unwrap_or(&d.key);
        let Some(ids) = classify(col, d) else {
            unclassified.push((d, None));
            continue;
        };
        let class = ids.iter().map(|id| design_row(id).0).max_by_key(|c| openness(*c)).expect("a row");
        let g = groups.entry((col.to_string(), ids.join("+"), decl.clone())).or_insert((class, 0, Vec::new()));
        g.1 += 1;
        let seed = d.program.split('#').next().unwrap_or(&d.program);
        if !g.2.contains(&seed) {
            g.2.push(seed);
        }
    }
    let mut by_row: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for ((col, rows, _), (_, n, _)) in &groups {
        *by_row.entry((col, rows)).or_default() += n;
    }
    eprintln!(
        "placement shadow: {} seeds, {} rows compared, {} divergences ({} classified), {} dynamic sites of unknown \
         domain (holes, not pinned)",
        r.seeds,
        r.report.rows_compared,
        r.report.divergences.len(),
        r.report.divergences.len() - unclassified.len(),
        r.unknown_dynamic
    );
    for ((col, rows), n) in &by_row {
        eprintln!("  {n:6} {col} {rows}");
    }

    let path = table_fixture_path();
    let fixture_name = "crates/hale-types/tests/fixtures/shadow_placement_table.txt";
    let existing = match std::fs::read_to_string(&path) {
        Ok(t) => parse_lines(&t).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
        Err(_) => BTreeMap::new(),
    };
    let unexplained = || gate_message(&r.report, &unclassified, &[], fixture_name);
    if std::env::var("HALE_SHADOW_REGEN").as_deref() == Ok("1") {
        let mut text = FIXTURE_HEADER.to_string();
        for (key, (class, count, seeds)) in &groups {
            // A line keeps its witness while it still shows the group.
            let witness =
                existing.get(key).map(|l| l.witness.as_str()).filter(|w| seeds.contains(w)).unwrap_or(seeds[0]);
            text.push_str(&format!("{}\t{}\t{}\t{}\t{count}\t{witness}\n", key.0, key.1, key.2, class_label(*class)));
        }
        std::fs::write(&path, text).expect("write fixture");
        assert!(unclassified.is_empty(), "{}", unexplained());
        return;
    }
    let mut problems = Vec::new();
    for (key, (class, count, seeds)) in &groups {
        let what = format!("{} {} `{}`", key.0, key.1, key.2);
        match existing.get(key) {
            None => {
                let notes: Vec<String> =
                    key.1.split('+').map(|id| format!("  {id}: {}", design_row(id).1)).collect();
                problems.push(format!("{what}: {count} divergence(s) ({}) with no line\n{}", seeds[0], notes.join("\n")))
            }
            Some(l) if l.class != *class || l.count != *count => problems.push(format!(
                "{what}: {} {count}, the line says {} {}",
                class_label(*class),
                class_label(l.class),
                l.count
            )),
            Some(l) if !seeds.contains(&l.witness.as_str()) => {
                problems.push(format!("{what}: the witness `{}` no longer shows it ({})", l.witness, seeds[0]))
            }
            Some(_) => {}
        }
    }
    for key in existing.keys().filter(|k| !groups.contains_key(*k)) {
        problems.push(format!("stale: {} {} `{}` no longer diverges", key.0, key.1, key.2));
    }
    assert!(
        unclassified.is_empty() && problems.is_empty(),
        "{}\n{}\nRegenerate {fixture_name} under HALE_SHADOW_REGEN=1 once the change is understood.",
        if unclassified.is_empty() { String::new() } else { unexplained() },
        problems.join("\n")
    );
}

