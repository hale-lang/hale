//! The placement table against every legacy producer (F.40 phase 3, P1).
//!
//! The placement table's shadow
//! (`notes/f40-placement-correspondence.md` § 3, hale-lang/hale#1296). It
//! loads every seed through the frontend's snapshot — the load, the
//! desugar sequence, the mint, the imports — and compares the table
//! (`Snapshot::demand_placement`) with each legacy producer the snapshot
//! can reach, one column each, every key prefixed by its column:
//!
//! - `bus`: `collect_subscriber_placements`, per type (no row is
//!   `SameThread`, as the graph defines it);
//! - `ownership`: the ownership graph's verbatim copy, `collect_placements`;
//! - `model`: the arrangement's `PlacedIn`, per instance path;
//! - `desugar`: `collect_off_owner_thread_fields`, per `Owner.field`;
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
//! became the diagnostics that switch pins.
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
use hale_graph::shadow::{gate_message, program_id, Class, Divergence, Kind, Report};
use hale_syntax::ast::{flat_decls, LocusDecl, LocusMember, TopDecl, TypeExpr};
use hale_types::bus_graph::{collect_subscriber_placements, Placement};
use hale_types::placement::legacy::collect_placements;
use hale_types::placement::{
    Bound, DeclRef, DomainId, DomainKind, HoleAt, HoleKind, InstanceKey, InstanceRow, Origin, OwnerRelative,
    PlacementTable, SiteUniverse,
};
use hale_types::resolve::TopScope;
use hale_types::stdlib_bodies::mangled_locus_name;
use hale_types::symbol::{TopSymbol, TypeKind};
use hale_types::Bundle;

fn placement_key(p: &Placement) -> String {
    match p {
        Placement::SameThread => "main".into(),
        Placement::CrossPool(name) => format!("pool:{name}"),
        Placement::Pinned => "pinned".into(),
    }
}

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

/// A domain as every column renders it.
fn domain_key(t: &PlacementTable, d: DomainId) -> String {
    match &t.domain(d).kind {
        DomainKind::Main => "main".into(),
        DomainKind::Pool { name, .. } => format!("pool:{name}"),
        DomainKind::Pinned { .. } => "pinned".into(),
    }
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

/// The declaration a row realizes, read from the store its universe names.
fn decl_of<'a>(bundle: &Bundle<'a>, d: &DeclRef) -> Option<&'a LocusDecl> {
    let find = |items: &'a [TopDecl], ids: &hale_types::snapshot::Snapshot| {
        flat_decls(items).find_map(|i| match i {
            TopDecl::Locus(l) if ids.site_id(l.id) == Some(d.site.id) => Some(l),
            _ => None,
        })
    };
    match d.site.universe {
        SiteUniverse::User => bundle.programs.values().find_map(|p| find(&p.items, &bundle.snapshot)),
        SiteUniverse::StdlibAnalysis => {
            find(&hale_types::stdlib_bodies::program()?.items, hale_types::stdlib_bodies::identities()?)
        }
    }
}

/// What a params field's written type is, as the legacy producers read
/// it: a single-segment locus name, or what they cannot see.
fn field_kind(decl: &LocusDecl, field: &str, top: &TopScope) -> &'static str {
    let param = decl.members.iter().find_map(|m| match m {
        LocusMember::Params(pb) => pb.params.iter().find(|p| p.name.name == field),
        _ => None,
    });
    match param.and_then(|p| p.ty.as_ref()) {
        None => "inferred",
        Some(TypeExpr::Named { generic_args, .. }) if !generic_args.is_empty() => "generic",
        Some(TypeExpr::Named { path, .. }) if path.segments.len() > 1 => {
            let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
            match mangled_locus_name(&segs).and_then(|m| top.lookup(m)) {
                Some(TopSymbol::Interface(_)) | Some(TopSymbol::Perspective(_)) => "contract-typed",
                _ if segs[0] == "std" => "qualified-stdlib",
                _ => "qualified",
            }
        }
        Some(TypeExpr::Named { path, .. }) => match top.lookup(&path.segments[0].name) {
            Some(TopSymbol::Locus(_)) => "locus",
            Some(TopSymbol::Type(info)) if matches!(info.kind, TypeKind::Alias(_)) => "aliased",
            Some(TopSymbol::Interface(_)) | Some(TopSymbol::Perspective(_)) => "contract-typed",
            _ => "other",
        },
        Some(_) => "contract-typed",
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

    fn prefix(k: &InstanceKey, i: usize) -> InstanceKey {
        InstanceKey { origin: k.origin, path: k.path[..i].to_vec(), replica: if i == 0 { None } else { k.replica } }
    }

    /// The first row on `k`'s path a legacy producer does not see, and
    /// why: the written type of the field that holds it.
    fn first_unseen(
        &self,
        k: &InstanceKey,
        bundle: &Bundle<'_>,
        top: &TopScope,
        seen: &dyn Fn(&InstanceKey, &InstanceRow) -> bool,
    ) -> Option<String> {
        for i in 0..=k.path.len() {
            let pk = Self::prefix(k, i);
            let pr = self.t.instances.get(&pk)?;
            if seen(&pk, pr) {
                continue;
            }
            if i == 0 {
                return Some(match k.origin {
                    Origin::Binding(_) => "an adapter of the root's bindings".into(),
                    o if self.is_root(o) => format!("the root `{}`", self.root),
                    o => format!("a literal `fn main` builds: `{}`", self.top(o)),
                });
            }
            let owner = self.t.instances.get(&Self::prefix(k, i - 1)).and_then(|r| r.realizes.as_ref());
            let kind = owner.and_then(|d| decl_of(bundle, d)).map(|d| field_kind(d, &k.path[i - 1].field, top));
            let stdlib = pr.realizes.as_ref().is_some_and(|d| d.site.universe == SiteUniverse::StdlibAnalysis);
            return Some(format!(
                "{} {}field `{}`{}",
                kind.unwrap_or("unknown"),
                if i == 1 { "root " } else { "nested " },
                self.path(&pk),
                if stdlib { " realizing a stdlib declaration" } else { "" }
            ));
        }
        None
    }
}

fn shadow_seed(seed: &Seed) -> Option<Shadowed> {
    let (id, s) = load(seed)?;
    let checked = s.demand_check().ok()?;
    if checked.diags.iter().any(|d| d.is_error()) {
        return None;
    }
    let t = s.demand_placement().ok()?;
    let top = s.demand_scope().ok()?;
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

    // A held instance runs where its held row is: the rows its source
    // template built it as answer where it was built (K-8 / M-8), so no
    // projection of where instances run reads them.
    let handed_off = t.handed_off();
    // The held rows, and the declarations of those whose source the
    // producer could not link (a parameter, a name bound twice): a
    // template `fn main` builds of one of them may be that source, still
    // standing on main where it was built.
    let held: Vec<&InstanceKey> = t
        .holes
        .iter()
        .filter_map(|h| match (&h.at, &h.kind) {
            (HoleAt::Instance(k), HoleKind::Reuse { .. }) => Some(k),
            _ => None,
        })
        .collect();
    let unlinked: BTreeSet<&str> = held
        .iter()
        .filter_map(|k| t.instances.get(*k))
        .filter(|r| r.built_by.is_none())
        .filter_map(|r| r.realizes.as_ref().map(|d| d.lowered.as_str()))
        .collect();
    let under_held =
        |k: &InstanceKey| held.iter().any(|h| h.origin == k.origin && h.replica == k.replica && k.path.starts_with(&h.path));
    let unlinked_source = |k: &InstanceKey| {
        t.entry_literals.iter().any(|c| Origin::Construction(c.literal) == k.origin)
            && t.instances
                .get(&InstanceKey { origin: k.origin, path: Vec::new(), replica: None })
                .and_then(|r| r.realizes.as_ref())
                .is_some_and(|d| unlinked.contains(d.lowered.as_str()))
    };

    // Per locus type (the lowered name), the domains its instances run
    // in, and the rows that decided them.
    let mut by_type: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut rows_of: BTreeMap<String, Vec<(&InstanceKey, &InstanceRow)>> = BTreeMap::new();
    for (k, r) in &t.instances {
        if handed_off.contains(k) {
            continue;
        }
        let Some(d) = &r.realizes else { continue };
        by_type.entry(d.lowered.clone()).or_default().insert(domain_key(t, r.domain));
        rows_of.entry(d.lowered.clone()).or_default().push((k, r));
    }
    let typed_new: Vec<(String, String)> = by_type.iter().map(|(k, v)| (k.clone(), joined(v))).collect();
    let describe = |k: &InstanceKey, r: &InstanceRow| {
        let own = match k.path.len() {
            0 => "top".to_string(),
            n => {
                let owner = t.instances.get(&Paths::prefix(k, n - 1)).and_then(|o| o.realizes.as_ref());
                owner.and_then(|d| decl_of(&bundle, d)).map(|d| field_kind(d, &k.path[n - 1].field, top)).unwrap_or("unknown").to_string()
            }
        };
        format!(
            "`{}` on {} ({}, {} field{}{})",
            paths.path(k),
            domain_key(t, r.domain),
            r.decided_by_kind(),
            own,
            if r.guarded { ", guarded" } else { "" },
            if under_held(k) {
                ", held"
            } else if unlinked_source(k) {
                ", a template an unlinked held row may be"
            } else {
                ""
            }
        )
    };
    let type_witness = |seen: &dyn Fn(&InstanceKey, &InstanceRow) -> bool| -> BTreeMap<String, Vec<String>> {
        rows_of
            .iter()
            .map(|(ty, rows)| {
                let mut w: Vec<String> = rows.iter().map(|(k, r)| describe(k, r)).collect();
                let causes: BTreeSet<String> =
                    rows.iter().filter_map(|(k, _)| paths.first_unseen(k, &bundle, top, seen)).collect();
                w.extend(causes.into_iter().map(|c| format!("cause: {c}")));
                (ty.clone(), w)
            })
            .collect()
    };

    // Owner.field → the declarations the field's rows realize.
    let mut field_decls: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (k, r) in &t.instances {
        if handed_off.contains(k) {
            continue;
        }
        let (Some(o), Some(step)) = (&r.owner, k.path.last()) else { continue };
        let Some(owner) = t.instances.get(o).and_then(|o| o.realizes.as_ref()) else { continue };
        field_decls
            .entry(format!("{}.{}", owner.lowered, step.field))
            .or_default()
            .insert(r.realizes.as_ref().map(|d| d.lowered.clone()).unwrap_or_else(|| "<hole>".into()));
    }
    let field_decls: BTreeMap<String, String> = field_decls.iter().map(|(k, v)| (k.clone(), joined(v))).collect();

    // bus and ownership: the per-type label, keyed by the last segment of
    // the placed field's written type; no row is `SameThread`. A label's
    // consumers match it against a declaration's name, so a key that names
    // a declaration the table places is that one; only a key that names
    // none (a qualified field's last segment) goes through the root field
    // that wrote it.
    let mut last_to_lowered: BTreeMap<String, String> = BTreeMap::new();
    let mut labelled = type_witness(&|k: &InstanceKey, _: &InstanceRow| k.path.len() <= 1);
    if let Some(root) = entry.lowering_root.as_ref().and_then(|m| m.decl(&bundle)) {
        for m in &root.members {
            let LocusMember::Params(pb) = m else { continue };
            for p in &pb.params {
                let Some(TypeExpr::Named { path, .. }) = &p.ty else { continue };
                let Some(last) = path.segments.last() else { continue };
                let realized: BTreeSet<&str> = t
                    .instances
                    .iter()
                    .filter(|(k, _)| k.path.len() == 1 && k.path[0].field == p.name.name)
                    .filter_map(|(_, r)| r.realizes.as_ref().map(|d| d.lowered.as_str()))
                    .collect();
                // B-5: a qualified field whose last segment is another
                // declaration's name labels that declaration.
                if path.segments.len() > 1 && by_type.contains_key(&last.name) && !realized.contains(last.name.as_str()) {
                    labelled.entry(last.name.clone()).or_default().push(format!(
                        "cause: a qualified root field `{}` whose last segment names another declaration",
                        p.name.name
                    ));
                }
                if realized.len() == 1 {
                    last_to_lowered.insert(last.name.clone(), realized.into_iter().next().unwrap().to_string());
                }
            }
        }
    }
    // A label another `main`'s entries wrote.
    for w in labelled.values_mut() {
        w.extend(global.iter().cloned());
    }
    for (name, labels, slice) in [
        (
            "bus",
            collect_subscriber_placements(&bundle),
            "the direct-call gate, SubscriberSite::placement, check_bounded_bus, hale/busGraph",
        ),
        (
            "ownership",
            collect_placements(&bundle),
            "the ownership graph's edge classes (SameTower / CrossPool), lowering's bubble plans",
        ),
    ] {
        let map_old: BTreeMap<String, String> = labels
            .keys()
            .filter(|k| !by_type.contains_key(*k))
            .filter_map(|k| last_to_lowered.get(k).map(|l| (k.clone(), l.clone())))
            .collect();
        let mut old: Vec<(String, String)> = labels.iter().map(|(k, v)| (k.clone(), placement_key(v))).collect();
        let covered: BTreeSet<String> =
            labels.keys().map(|k| map_old.get(k).cloned().unwrap_or_else(|| k.clone())).collect();
        for (k, _) in &typed_new {
            if !covered.contains(k) {
                old.push((k.clone(), "main".into()));
            }
        }
        columns.push(Column {
            name,
            old,
            new: typed_new.clone(),
            map_old,
            witness: labelled.clone(),
            global: global.clone(),
            slice,
            decl: BTreeMap::new(),
        });
    }

    // model: the arrangement's instances, by path, against the table's
    // rows projected to the same paths (every construction and every
    // alternative at a path, as one set). An adapter is no instance of
    // the arrangement.
    if let Ok(model) = s.demand_model() {
        let e = &model.entities;
        let mut old = Vec::new();
        for (i, inst) in e.locus_instances.iter().enumerate() {
            let decl = &e.loci[inst.decl.0 as usize].name;
            let dom = model
                .relations
                .placed_in
                .iter()
                .find(|p| p.instance.0 as usize == i)
                .map(|p| e.thread_domains[p.domain.0 as usize].name.clone())
                .unwrap_or_else(|| "-".into());
            old.push((inst.path.clone(), format!("{decl}@{dom}")));
        }
        let arranged: BTreeSet<String> = old.iter().map(|(p, _)| p.clone()).collect();
        let mut model_global = global.clone();
        if let Some(first) = e.locus_instances.iter().map(|i| i.path.split('.').next().unwrap_or("")).min_by_key(|p| p.len()) {
            if first != paths.root {
                model_global.push(format!("cause: the arrangement's root `{first}` is not the table's `{}`", paths.root));
            }
        }
        let mut new: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut witness: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let seen = |pk: &InstanceKey, _: &InstanceRow| arranged.contains(&paths.path(pk));
        let mut model_decls: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (path, fact) in &old {
            model_decls.entry(path.clone()).or_default().insert(fact.split('@').next().unwrap_or("").to_string());
        }
        for (k, r) in &t.instances {
            if matches!(k.origin, Origin::Binding(_)) || handed_off.contains(k) {
                continue;
            }
            let path = paths.path(k);
            let dom = match &t.domain(r.domain).kind {
                DomainKind::Main => "main".to_string(),
                DomainKind::Pool { name, .. } => format!("pool:{name}"),
                DomainKind::Pinned { anchor, .. } => format!("pinned:{}", paths.path(anchor)),
            };
            let decl = r.realizes.as_ref().map(|d| d.lowered.clone()).unwrap_or_else(|| "<hole>".into());
            new.entry(path.clone()).or_default().insert(format!("{decl}@{dom}"));
            model_decls.entry(path.clone()).or_default().insert(decl);
            let w = witness.entry(path.clone()).or_default();
            w.push(describe(k, r));
            if let Some(c) = paths.first_unseen(k, &bundle, top, &seen) {
                w.push(format!("cause: {c}"));
            }
        }
        // K-9 / M-9: below a held row whose source the producer could not
        // link the table asserts nothing; a path the arrangement has there
        // is the declared type's default subtree it built under the holder.
        let unlinked_held: Vec<String> = held
            .iter()
            .filter(|k| t.instances.get(**k).is_some_and(|r| r.built_by.is_none()))
            .map(|k| paths.path(k))
            .collect();
        for (path, _) in &old {
            if new.contains_key(path) {
                continue;
            }
            if let Some(h) = unlinked_held.iter().find(|h| path.starts_with(&format!("{h}."))) {
                witness
                    .entry(path.clone())
                    .or_insert_with(|| model_global.clone())
                    .push(format!("cause: a path under the unlinked held row `{h}`"));
            }
        }
        columns.push(Column {
            name: "model",
            old,
            new: new.iter().map(|(k, v)| (k.clone(), joined(v))).collect(),
            map_old: BTreeMap::new(),
            witness,
            global: model_global,
            slice: "the model's LocusInstance / PlacedIn / Owns (--dump-model, the reachability judgment)",
            decl: model_decls.iter().map(|(k, v)| (k.clone(), joined(v))).collect(),
        });
    }

    // desugar: the intra-locus rewrite's off-owner-thread fields.
    let mut old = Vec::new();
    for p in bundle.programs.values() {
        for (owner, field) in hale_syntax::desugar::shadow_support::collect_off_owner_thread_fields(&p.items) {
            old.push((format!("{owner}.{field}"), "off".to_string()));
        }
    }
    let mut new: BTreeSet<String> = BTreeSet::new();
    for (k, r) in &t.instances {
        if r.owner_relative != OwnerRelative::OffOwner || handed_off.contains(k) {
            continue;
        }
        let (Some(o), Some(step)) = (&r.owner, k.path.last()) else { continue };
        if let Some(owner) = t.instances.get(o).and_then(|o| o.realizes.as_ref()) {
            new.insert(format!("{}.{}", owner.lowered, step.field));
        }
    }
    columns.push(Column {
        name: "desugar",
        old,
        new: new.into_iter().map(|k| (k, "off".to_string())).collect(),
        map_old: BTreeMap::new(),
        witness: BTreeMap::new(),
        global: global.clone(),
        slice: "the intra-locus rewrite (a publish to an off-thread field's handler stays a publish)",
        decl: field_decls,
    });

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

trait DecidedBy {
    fn decided_by_kind(&self) -> &'static str;
}

impl DecidedBy for hale_types::placement::InstanceRow {
    fn decided_by_kind(&self) -> &'static str {
        match self.decided_by {
            hale_types::placement::Decision::Entry { .. } => "entry",
            hale_types::placement::Decision::Binding { .. } => "binding",
            hale_types::placement::Decision::Inherited { .. } => "inherited",
            hale_types::placement::Decision::Default => "default",
        }
    }
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
    (
        "K-8",
        Class::Correction,
        "a held instance's subtree lives in its holder's domain (K-8 / M-8, § 10.8; case 16): the table projects \
         the source's actual rows under the holder, never the declaration's defaults, and the source template's \
         rows answer where it was built (`built_by`), read by no domain question. Agreement where the producer \
         links the source; where it cannot (a parameter, a name bound twice), the source template stands on main \
         beside the held row and the type's set holds both. § 2.1",
    ),
    (
        "B-1",
        Class::KnownOldBug,
        "a locus nested under a root field placed off main inherits its owner's domain (sem:3511-3542, rt:364-371); \
         the graph labels it SameThread. § 2.2",
    ),
    (
        "B-3",
        Class::KnownOldBug,
        "a qualified root field's label is keyed by its last segment, which names no declaration; the table places \
         the declaration by identity. § 2.2 (the ownership graph's copy: the same principle, § 2.3)",
    ),
    (
        "B-4",
        Class::Correction,
        "an imported `__lib_` root's entries label types (first-wins over every block); the table has no rows for a \
         root lowering never deploys (ent:1-31). § 2.2 (the ownership graph's copy: the same principle, § 2.3)",
    ),
    (
        "B-5",
        Class::KnownOldBug,
        "a last-segment collision: a qualified root field placed off main labels the user declaration its last \
         segment names; each declaration is placed by its own instances (case 4). § 2.2 (the ownership graph's \
         copy: the same principle, § 2.3)",
    ),
    (
        "B-6",
        Class::KnownOldBug,
        "one type, instances in several domains (K-3 / B-6, case 1): the graph labels the type first-wins, the table \
         gives the set, and the direct-call gate holds only when it is {main}. § 2.2",
    ),
    (
        "B-7",
        Class::KnownOldBug,
        "an adapter locus in `bindings { }` is pinned on its own thread; the bus graph labels it SameThread, and the \
         checker's map has no row for it. Runtime confirmation pending: § 4's adapter test, the correction in the \
         graph-switch PR. § 2.2, § 10.2",
    ),
    (
        "O-1/O-2",
        Class::KnownOldBug,
        "B-1's principle reaching the ownership graph: a locus nested under a root field placed off main inherits its \
         owner's domain; the copy labels it SameThread. § 2.3",
    ),
    (
        "O-3",
        Class::SpecDisagreement,
        "one enclosing type with instances in several domains; first-wins gives one edge class for all (U-1, pending \
         the owner). § 2.3",
    ),
    (
        "O-7",
        Class::KnownOldBug,
        "an adapter locus in `bindings { }` as an edge's enclosing or owning side: the copy labels it SameThread, the \
         table pins it on its own thread. Runtime confirmation pending: § 4's adapter test, the correction in the \
         graph-switch PR. § 2.3, § 10.2",
    ),
    (
        "F-R",
        Class::Correction,
        "fixture F-R's principle: the walk reads an imported root's placement block; a `__lib_` root is never \
         deployed, so nothing observable differs. § 2.5",
    ),
    (
        "M-1",
        Class::Correction,
        "the arrangement is rooted at the first `main` in program order, the table at lowering's root (fixture F-R's \
         principle). § 2.4",
    ),
    (
        "M-2",
        Class::SpecDisagreement,
        "a stdlib instance (or one under it); the arrangement holds user declarations only (U-4, pending the owner). \
         § 2.4",
    ),
    (
        "M-3",
        Class::KnownOldBug,
        "a field typed by a qualified stdlib path whose last segment names a user locus is arranged as the user's \
         locus; the table realizes the stdlib declaration. § 2.4",
    ),
    (
        "M-5",
        Class::KnownOldBug,
        "a contract-typed, aliased or generic field (or one under it); the arrangement reads a field's last segment \
         and skips what is not a user locus. § 2.4",
    ),
    (
        "M-7",
        Class::Correction,
        "the entry is an implicit construction: a literal directly in `fn main` adds paths the arrangement (the \
         root's tree) does not hold, and contract 3 applies to them. § 2.4, § 10.1",
    ),
    (
        "M-9",
        Class::Correction,
        "an unlinked held source (K-9 / M-9, § 10.9): the table records the `Reuse` hole and asserts no subtree; \
         the arrangement builds the declared type's default subtree under the holder (a known old bug: it \
         fabricates rows an override may contradict), so a path below the held row is the arrangement's alone. A \
         consumer switch treats the hole as unknown: it disables a proof or an optimization and never defaults to \
         main or to pinned. § 2.4",
    ),
    (
        "M-c3",
        Class::Correction,
        "contract 3: one arrangement path, several construction templates or alternatives that disagree; the \
         projection makes it a model hole naming them. § 2.4",
    ),
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
    let multi = d.new.as_deref().is_some_and(|n| n.contains('|'));
    // Every cause must name a row; no cause names none.
    let every = |f: &dyn Fn(&str) -> Option<&'static str>| -> Option<Vec<&'static str>> {
        let ids: BTreeSet<&'static str> = causes.iter().map(|c| f(c)).collect::<Option<_>>()?;
        (!ids.is_empty()).then(|| ids.into_iter().collect())
    };
    // K-8's residue: the set's off-main rows are all held, and on main
    // stands a template an unlinked held row may have been built as, so
    // the "several domains" are one instance before and after its handoff.
    // Held rows alone in several domains (one instance several holders
    // hold) are K-3's shape, not this.
    let rows: Vec<&String> = d.witnesses.iter().filter(|w| w.starts_with('`')).collect();
    let source = ", a template an unlinked held row may be)";
    let held_residue = rows.iter().any(|w| w.contains(", held)"))
        && rows.iter().any(|w| w.contains("` on main (") && w.contains(source))
        && rows.iter().all(|w| w.contains(", held)") || (w.contains("` on main (") && w.contains(source)));
    let ids = match (col, d.kind) {
        ("bus", Kind::Disagreement) | ("ownership", Kind::Disagreement) => {
            let o = col == "ownership";
            if multi && held_residue && d.old.as_deref() == Some("main") {
                // The held rows off main are B-1's (O-1's) shape; the
                // source standing on main beside them is K-8's residue.
                if o {
                    vec!["K-8", "O-1/O-2"]
                } else {
                    vec!["B-1", "K-8"]
                }
            } else if multi {
                vec![if o { "O-3" } else { "B-6" }]
            } else if d.old.as_deref() == Some("main") {
                if rows.iter().any(|w| w.contains("(binding,")) {
                    vec![if o { "O-7" } else { "B-7" }]
                } else if rows.iter().all(|w| w.contains("(inherited,")) {
                    vec![if o { "O-1/O-2" } else { "B-1" }]
                } else if rows.iter().any(|w| w.contains("(entry, qualified")) {
                    vec!["B-3"]
                } else {
                    return None;
                }
            } else if causes.iter().any(|c| c.starts_with("a qualified root field `")) {
                vec!["B-5"]
            } else if d.new.as_deref() == Some("main") && causes.iter().any(|c| c.contains("(imported), with entries")) {
                // The table places every instance on main, so only another
                // block wrote the label: an imported root's.
                vec!["B-4"]
            } else {
                return None;
            }
        }
        ("bus" | "ownership" | "desugar", Kind::OnlyOld) => {
            if !causes.iter().any(|c| c.contains("(imported), with entries")) {
                return None;
            }
            vec![if col == "desugar" { "F-R" } else { "B-4" }]
        }
        ("model", Kind::OnlyNew) => every(&|c| {
            if c.contains("realizing a stdlib declaration") {
                Some("M-2")
            } else if c.starts_with("the root ") {
                Some("M-1")
            } else if c.starts_with("a literal `fn main` builds") {
                Some("M-7")
            } else if ["contract-typed", "aliased", "generic"].iter().any(|k| c.starts_with(k)) {
                Some("M-5")
            } else {
                None
            }
        })?,
        ("model", Kind::OnlyOld) if causes.iter().any(|c| c.starts_with("the arrangement's root")) => vec!["M-1"],
        ("model", Kind::OnlyOld) if causes.iter().any(|c| c.starts_with("a path under the unlinked held row")) => {
            vec!["M-9"]
        }
        ("model", Kind::Disagreement) if multi => vec!["M-c3"],
        ("model", Kind::Disagreement) => {
            // M-3: the arrangement names the user's locus where the table
            // realizes a stdlib one.
            let decl = |f: &Option<String>| f.as_deref().and_then(|v| v.split('@').next()).unwrap_or("").to_string();
            if decl(&d.new).starts_with("__Std") && decl(&d.old) != decl(&d.new) {
                vec!["M-3"]
            } else {
                return None;
            }
        }
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

