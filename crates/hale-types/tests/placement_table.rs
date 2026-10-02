//! The placement table (F.40 phase 3, P1; the correspondence is
//! `notes/f40-placement-correspondence.md`, hale-lang/hale#1296).
//!
//! Every case loads its seed the way every verb does — the frontend's
//! load, the desugar sequence, the mint — from a fixture under
//! `fixtures/placement/`, never from a bundle the test mints itself.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot, Target};
use hale_frontend::source::Disk;
use hale_syntax::ast::{flat_decls, Expr, LocusDecl, LocusMember, ParamInit, Program, TopDecl};
use hale_syntax::sites::SiteKind;
use hale_types::placement::{
    join_lowering, provenance, Bound, Decision, DeclRef, DomainId, DomainKind, Enclosing, HoleAt, HoleKind,
    InstanceKey, InstanceRow, LoweringRef, Origin, OwnerRelative, PlacementTable, SiteRef, SiteUniverse,
};
use hale_types::check::PoolId;
use hale_types::snapshot::STDLIB_SEED;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/placement").join(name)
}

/// A build's load (`hale build <seed>` for the host): the snapshot whose
/// lowering view is the merged mint.
fn build(seed: &Path) -> Snapshot {
    match Snapshot::load(seed, LoadMode::WholeSeed, &Disk, Config::build(Target::host())) {
        Ok(s) => s,
        Err(_) => panic!("{} does not load", seed.display()),
    }
}

/// `hale check <seed>`'s load.
fn check(seed: &Path) -> Snapshot {
    match Snapshot::load(seed, LoadMode::WholeSeed, &Disk, Config::check(seed.is_dir(), false)) {
        Ok(s) => s,
        Err(_) => panic!("{} does not load", seed.display()),
    }
}

fn errors(s: &Snapshot) -> Vec<String> {
    let checked = s.demand_check().unwrap_or_else(|_| panic!("the check is blocked"));
    checked.diags.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect()
}

/// A fixture that checks clean, and its table.
fn clean(name: &str) -> Snapshot {
    let s = check(&fixture(name));
    let e = errors(&s);
    assert!(e.is_empty(), "{name} must check clean: {e:?}");
    s
}

fn table(s: &Snapshot) -> &PlacementTable {
    s.demand_placement().unwrap_or_else(|_| panic!("placement is blocked"))
}

/// A key's path as `field.field`, an alternative step marked `?`, and
/// its replica as `[i]`.
fn path(k: &InstanceKey) -> String {
    let p: Vec<String> =
        k.path.iter().map(|s| format!("{}{}", s.field, if s.alternative.is_some() { "?" } else { "" })).collect();
    match k.replica {
        Some(i) => format!("{}[{i}]", p.join(".")),
        None => p.join("."),
    }
}

/// The rows at `p` (every origin, every alternative).
fn rows<'t>(t: &'t PlacementTable, p: &str) -> Vec<(&'t InstanceKey, &'t InstanceRow)> {
    t.instances.iter().filter(|(k, _)| path(k) == p).collect()
}

fn one<'t>(t: &'t PlacementTable, p: &str) -> (&'t InstanceKey, &'t InstanceRow) {
    let r = rows(t, p);
    assert_eq!(r.len(), 1, "one row at `{p}`, not {}", r.len());
    r[0]
}

fn lowered(r: &InstanceRow) -> &str {
    r.realizes.as_ref().map(|d| d.lowered.as_str()).unwrap_or("<hole>")
}

fn is_pinned(t: &PlacementTable, d: DomainId) -> bool {
    matches!(t.domain(d).kind, DomainKind::Pinned { .. })
}

fn pool_name(t: &PlacementTable, d: DomainId) -> Option<&str> {
    match &t.domain(d).kind {
        DomainKind::Pool { name, .. } => Some(name),
        _ => None,
    }
}

fn locus<'p>(program: &'p Program, name: &str) -> &'p LocusDecl {
    flat_decls(&program.items)
        .find_map(|i| match i {
            TopDecl::Locus(l) if l.name.name == name => Some(l),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no locus `{name}`"))
}

/// The struct literal a params field's default builds.
fn default_literal(l: &LocusDecl, field: &str) -> hale_syntax::ast::NodeId {
    l.members
        .iter()
        .find_map(|m| match m {
            LocusMember::Params(pb) => pb.params.iter().find(|p| p.name.name == field),
            _ => None,
        })
        .and_then(|p| match &p.init {
            ParamInit::Value(Expr::Struct { id, .. }) => Some(*id),
            _ => None,
        })
        .unwrap_or_else(|| panic!("`{}.{field}` has no default literal", l.name.name))
}

/// Every fixture is as `hale fmt` writes it: the formatter `hale fmt
/// --check` runs, over each `.hl` under `fixtures/placement/`.
#[test]
fn the_placement_fixtures_are_formatted() {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "hl") {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(&fixture(""), &mut files);
    assert!(!files.is_empty());
    for f in files {
        let src = std::fs::read_to_string(&f).unwrap();
        let formatted = hale_syntax::fmt::format_source(&src).unwrap_or_else(|_| panic!("{} does not format", f.display()));
        assert!(formatted == src, "{} is not formatted", f.display());
    }
}

/// The design's identity acceptance case (§ 3 case 12): two universes,
/// one numeric id. A user locus declaration is minted with the id the
/// stdlib analysis copy gives a stdlib locus; their refs stay apart,
/// provenance answers from the right store, and the lowering join
/// resolves every relevant site into the merged mint exactly once.
#[test]
fn two_universes_one_numeric_id_stay_two_identities() {
    let snap = build(&fixture("two_universes"));
    let user = snap.identities();
    let program = snap.program().expect("a whole seed holds one program");
    let stdlib = hale_types::stdlib_bodies::program().expect("the stdlib parses");
    let stdlib_ids = hale_types::stdlib_bodies::identities().expect("the stdlib is minted");

    let collider = locus(program, "Collider");
    let reader = locus(stdlib, "__StdIoUdpReader");
    let collider_id = user.site_id(collider.id).expect("minted");
    let reader_id = stdlib_ids.site_id(reader.id).expect("minted");
    // The collision is present, or the case proves nothing.
    assert_eq!(collider_id, reader_id, "the padding no longer lands `Collider` on the reader's id");

    let user_decl = DeclRef { site: SiteRef::user(collider_id), args: Vec::new(), lowered: "Collider".into() };
    let stdlib_decl =
        DeclRef { site: SiteRef::stdlib(reader_id), args: Vec::new(), lowered: "__StdIoUdpReader".into() };
    assert_eq!(user_decl.site.id, stdlib_decl.site.id);
    assert_ne!(user_decl.site.universe, stdlib_decl.site.universe);
    assert_ne!(user_decl, stdlib_decl, "one numeric id, two canonical declaration keys");
    let keyed: BTreeSet<&DeclRef> = [&user_decl, &stdlib_decl].into_iter().collect();
    assert_eq!(keyed.len(), 2, "a set keyed by `DeclRef` holds both");

    // Default-literal provenance resolves into the store the universe
    // names: the reader's `buf` default into the analysis copy, naming
    // the stdlib file; `Collider`'s `inner` default into the snapshot,
    // naming the user's file.
    let stdlib_literal = SiteRef::stdlib(stdlib_ids.site_id(default_literal(reader, "buf")).unwrap());
    let user_literal = SiteRef::user(user.site_id(default_literal(collider, "inner")).unwrap());
    let s = provenance(stdlib_literal, user, stdlib_ids).expect("the analysis copy minted it");
    assert_eq!(s.kind, SiteKind::StructLiteral);
    let at = hale_types::stdlib_bodies::stdlib_span_location(s.span).expect("a stdlib position");
    assert!(at.starts_with("io_udp.hl:"), "the reader's default literal is in the stdlib's udp file, not {at}");
    let u = provenance(user_literal, user, stdlib_ids).expect("the snapshot minted it");
    assert_eq!(u.kind, SiteKind::StructLiteral);
    assert!(user.seeds[u.id.seed.0 as usize].ends_with("a_pads.hl"), "the user's default literal is in a_pads.hl");
    // The same numeric id asked of the other store is another site, or
    // none: a bare id never names its site.
    let crossed = provenance(SiteRef { universe: SiteUniverse::StdlibAnalysis, id: collider_id }, user, stdlib_ids);
    assert_eq!(crossed.map(|s| s.kind), Some(SiteKind::Locus));
    assert_ne!(crossed.map(|s| s.span), user.site(collider_id).map(|s| s.span));

    // The seed's display string alone does not tell the universes
    // apart: both ids are seed 0, which in the analysis copy is
    // `STDLIB_SEED` and in the snapshot is the first user file. Reading
    // `seeds[id.seed]` presupposes knowing which store to ask, which is
    // what the universe says.
    assert_eq!(collider_id.seed, reader_id.seed);
    assert_eq!(stdlib_ids.seeds[reader_id.seed.0 as usize], STDLIB_SEED);
    assert_ne!(user.seeds[collider_id.seed.0 as usize], STDLIB_SEED);

    // The lowering correspondence: both declarations, both default
    // literals and the root's field literals resolve into the merged mint
    // exactly once.
    let lowering = snap.demand_lowering().unwrap_or_else(|b| panic!("lowering blocked: {:?}", b.refused));
    let app = locus(program, "App");
    let mut refs: Vec<LoweringRef<'_>> = vec![
        LoweringRef::Decl(&user_decl),
        LoweringRef::Decl(&stdlib_decl),
        LoweringRef::Site(stdlib_literal),
        LoweringRef::Site(user_literal),
    ];
    let root_literals: Vec<SiteRef> = ["c", "u", "l"]
        .iter()
        .map(|f| SiteRef::user(user.site_id(default_literal(app, f)).unwrap()))
        .collect();
    refs.extend(root_literals.iter().map(|s| LoweringRef::Site(*s)));
    let joined = join_lowering(&refs, user, &lowering.merged, &lowering.snapshot).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(joined.len(), refs.len(), "every ref resolves");
    let targets: BTreeSet<_> = joined.values().collect();
    assert_eq!(targets.len(), refs.len(), "no two refs resolve to one merged site");
    assert_eq!(joined[&user_decl.site], collider_id, "a user site keeps its id in the merged mint");
    let merged_reader = lowering.snapshot.site(joined[&stdlib_decl.site]).expect("a merged site");
    assert_eq!(merged_reader.kind, SiteKind::Locus);
    assert_eq!(merged_reader.span, reader.span);
    assert_ne!(joined[&stdlib_decl.site], reader_id, "the merged mint numbers the stdlib past the user's sites");
}

// ---------------------------------------------------- the producer

/// The table is a family of the snapshot: demanded, it runs once. The
/// check reads it (the F.31 rule and sync inference, per instance), and
/// a demand after the check reads the same table.
#[test]
fn the_table_is_demanded_once_per_snapshot() {
    let s = clean("two_instances.hl");
    assert_eq!(s.builds()["placement"], 1, "the check demands the table once");
    let first: *const PlacementTable = table(&s);
    let again: *const PlacementTable = table(&s);
    assert_eq!(first, again);
    assert_eq!(s.builds()["placement"], 1);
}

/// Case 1: one type, three instances, three domains; each nested `K`
/// inherits its own owner's.
#[test]
fn two_instances_of_one_type_have_two_domains() {
    let s = clean("two_instances.hl");
    let t = table(&s);
    let (_, a) = one(t, "a");
    let (_, b) = one(t, "b");
    let (_, c) = one(t, "c");
    assert!(lowered(a) == "W" && lowered(b) == "W" && lowered(c) == "W");
    assert!(is_pinned(t, a.domain));
    assert_eq!(pool_name(t, b.domain), Some("io"));
    assert_eq!(c.domain, PlacementTable::MAIN);
    assert_eq!(c.decided_by, Decision::Default);
    let w_domains: BTreeSet<DomainId> = [a.domain, b.domain, c.domain].into_iter().collect();
    assert_eq!(w_domains.len(), 3);
    for (owner, field) in [(a, "a.k"), (b, "b.k"), (c, "c.k")] {
        let (k, row) = one(t, field);
        assert_eq!(lowered(row), "K");
        assert_eq!(row.domain, owner.domain, "`{field}` inherits its owner's domain");
        assert_eq!(row.owner_relative, OwnerRelative::SameAsOwner);
        assert!(matches!(&row.decided_by, Decision::Inherited { from } if from.path.len() == 1 && k.path.len() == 2));
    }
}

/// Case 2: three deep, on a pool, pinned, and pinned with three
/// replicas: each replica its own domain, `[i]` on every nested key.
#[test]
fn replicas_are_k_domains_each_nesting_its_own_tree() {
    let s = clean("nested_inheritance.hl");
    let t = table(&s);
    let (_, p) = one(t, "p");
    assert_eq!(pool_name(t, p.domain), Some("io"));
    assert_eq!(one(t, "p.s").1.domain, p.domain, "the subscriber under a pool owner runs on the pool");
    let (_, q) = one(t, "q");
    assert!(is_pinned(t, q.domain));
    assert_eq!(one(t, "q.s").1.domain, q.domain, "the subscriber under a pinned owner runs on its thread");
    let mut replicas = BTreeSet::new();
    for i in 0..3 {
        let (k, r) = one(t, &format!("r[{i}]"));
        assert!(is_pinned(t, r.domain));
        assert!(matches!(&t.domain(r.domain).kind, DomainKind::Pinned { anchor, .. } if anchor == k));
        replicas.insert(r.domain);
        let (sk, sr) = one(t, &format!("r.s[{i}]"));
        assert_eq!(sk.replica, Some(i));
        assert_eq!(sr.domain, r.domain, "replica {i}'s subscriber runs on replica {i}'s thread");
    }
    assert_eq!(replicas.len(), 3, "each replica is its own domain");
    assert!(rows(t, "r").is_empty(), "a replicated field has no unindexed row");
}

/// Case 3: the declaration built is the literal's, not the field's
/// declared type, and the override's literal is the row's literal.
#[test]
fn overrides_realize_the_literal_that_was_built() {
    let s = clean("overrides.hl");
    let t = table(&s);
    let (_, gw) = one(t, "gw");
    let (_, router) = one(t, "gw.router");
    assert_eq!(lowered(router), "RouterV2", "the construction literal's override, not the default RouterV1");
    let lit = provenance(router.literal.unwrap(), s.identities(), hale_types::stdlib_bodies::identities().unwrap())
        .expect("minted");
    let fn_main = s.sources().values().next().unwrap().find("fn main").unwrap() as u32;
    assert!(lit.span.start.0 > fn_main, "the override written in `fn main`, not `Gateway`'s default");
    assert_eq!(router.domain, gw.domain);
    let (_, j) = one(t, "j");
    assert_eq!(lowered(j), "Churner", "a contract-typed param realizes the impl built");
}

/// Case 4: a qualified stdlib field and a user locus named like its
/// last segment are two declarations in two universes.
#[test]
fn a_qualified_field_and_its_last_segment_twin_are_two_declarations() {
    let s = clean("qualified.hl");
    let t = table(&s);
    let (_, l) = one(t, "l");
    let (_, own) = one(t, "own");
    let ld = l.realizes.as_ref().unwrap();
    assert_eq!(ld.lowered, "__StdIoTcpListener");
    assert_eq!(ld.site.universe, SiteUniverse::StdlibAnalysis);
    assert_eq!(pool_name(t, l.domain), Some("io"));
    let od = own.realizes.as_ref().unwrap();
    assert_eq!((od.lowered.as_str(), od.site.universe), ("Listener", SiteUniverse::User));
    assert_eq!(own.domain, PlacementTable::MAIN, "the user's `Listener` is placed by its own instance");
}

/// Case 5: rule 3 admits an aliased placed field, and each row realizes
/// the declaration its alias names.
#[test]
fn an_alias_realizes_the_declaration_it_names() {
    let s = clean("aliased.hl");
    let t = table(&s);
    let (_, h) = one(t, "h");
    assert_eq!(lowered(h), "Holder");
    assert!(is_pinned(t, h.domain));
    let (_, twig) = one(t, "h.t");
    assert_eq!(lowered(twig), "Leaf");
    assert_eq!(twig.domain, h.domain);
}

/// Case 6: a generic locus as a params field is refused by the checker
/// (`generic_monomorph_agreement.rs` pins the build's refusal too), so
/// the shape never reaches a consumer. The table, total over a program
/// that does not typecheck, still keys the two specializations apart.
#[test]
fn a_generic_locus_field_is_refused_before_any_consumer() {
    let s = check(&fixture("generic.hl"));
    let e = errors(&s);
    assert!(e.iter().any(|m| m.contains("param `c`: declared `Cache_Int_String`")), "{e:?}");
    let t = table(&s);
    let c = one(t, "c").1.realizes.clone().unwrap();
    let d = one(t, "d").1.realizes.clone().unwrap();
    assert_eq!(c.site, d.site, "one template");
    assert_eq!((c.lowered.as_str(), d.lowered.as_str()), ("Cache_Int_String", "Cache_Int_Int"));
    assert_eq!((c.args.len(), d.args.len()), (2, 2));
    assert_ne!(c, d);
}

/// Case 7: a module-qualified field resolves; a seed whose only `main`
/// is module-nested has rows, because lowering deploys it, and its root
/// is not the entry.
#[test]
fn a_module_nested_main_is_the_root_and_not_the_entry() {
    let s = clean("module_qualified.hl");
    let t = table(&s);
    assert_eq!(lowered(one(t, "k").1), "K");
    assert!(t.root.as_ref().unwrap().is_entry);

    let s = check(&fixture("module_nested_main.hl"));
    let t = table(&s);
    let root = t.root.as_ref().expect("lowering deploys the nested main");
    assert!(!root.is_entry);
    assert!(root.decl.module_nested);
    let (_, w) = one(t, "w");
    assert!(is_pinned(t, w.domain));
}

/// Case 8: an imported `main` is never the root.
#[test]
fn an_imported_main_is_never_the_root() {
    let s = clean("imported/no_own_main");
    let t = table(&s);
    assert!(t.root.is_none(), "the only `main` is the import's");
    let (k, w) = one(t, "");
    assert!(lowered(w).ends_with("_Worker") && w.domain == PlacementTable::MAIN, "`fn main`'s own literal, on main");
    assert_eq!(t.entry_literals.len(), 1);
    assert_eq!(k.origin, Origin::Construction(t.entry_literals[0].literal));
    assert_eq!(t.domains.len(), 1, "no pinned domain from the library's entry");

    let s = clean("imported/own_main");
    let t = table(&s);
    let root = t.root.as_ref().unwrap();
    assert_eq!(root.decl.name, "Mine");
    let (_, w) = one(t, "w");
    assert!(lowered(w).starts_with("__lib_") && lowered(w).ends_with("_Worker"));
    assert_eq!(w.domain, PlacementTable::MAIN, "the library's pinned entry places nothing here");
    assert!(is_pinned(t, one(t, "l").1.domain));
    let pinned = t.domains.iter().filter(|d| matches!(d.kind, DomainKind::Pinned { .. })).count();
    assert_eq!(pinned, 1);
}

/// Case 9: an inline adapter is a row of its own origin, decided by its
/// binding and pinned on a domain anchored at itself; built once
/// however many constructions the root has.
#[test]
fn an_inline_adapter_is_a_pinned_binding_row() {
    let s = clean("adapter.hl");
    let t = table(&s);
    let (k, fwd) = t.instances.iter().find(|(k, _)| matches!(k.origin, Origin::Binding(_))).expect("an adapter row");
    assert!(k.path.is_empty() && fwd.owner.is_none());
    assert_eq!(lowered(fwd), "Fwd");
    assert!(matches!(fwd.decided_by, Decision::Binding { .. }));
    assert!(matches!(&t.domain(fwd.domain).kind, DomainKind::Pinned { anchor, .. } if anchor == k));
    let (_, p) = one(t, "p");
    assert!(is_pinned(t, p.domain));
    assert_ne!(p.domain, fwd.domain, "two pinned domains, disjoint");

    let s = clean("adapter_two_sites.hl");
    let t = table(&s);
    let adapters = t.instances.keys().filter(|k| matches!(k.origin, Origin::Binding(_))).count();
    assert_eq!(adapters, 1, "the bindings prelude builds the adapter once");
    assert_eq!(t.root.as_ref().unwrap().constructions.len(), 2);
    let ps = rows(t, "p");
    assert_eq!(ps.len(), 2, "a root field row per construction");
    let pinned: BTreeSet<DomainId> =
        t.domains.iter().filter(|d| matches!(d.kind, DomainKind::Pinned { .. })).map(|d| d.id).collect();
    assert_eq!(pinned.len(), 3);
}

/// Case 10: dynamic sites, each with the domains its enclosing scope
/// runs in and its bound; the root built by a factory called in a loop
/// is one template, unbounded.
#[test]
fn dynamic_sites_carry_their_domains_and_bounds() {
    let s = clean("dynamic.hl");
    let t = table(&s);
    let root = t.root.as_ref().unwrap();
    assert_eq!(root.constructions.len(), 1);
    assert!(matches!(&root.constructions[0].bound, Bound::Unbounded(why) if why.contains("loop")));
    // The pinned-in-a-loop rule reads the cause: the root's literal is in
    // a factory called in a loop, each call of which joins its own
    // threads, not in a loop itself.
    assert!(!root.constructions[0].bound.built_in_a_loop(), "{:?}", root.constructions[0].bound);
    let user: Vec<_> = t.dynamic.iter().filter(|d| d.literal.universe == SiteUniverse::User).collect();
    let job = user.iter().find(|d| d.realizes.as_ref().is_some_and(|r| r.lowered == "Job")).expect("Job");
    assert!(matches!(&job.enclosing, Enclosing::Locus(d) if d.lowered == "App"));
    assert_eq!(job.domains, [PlacementTable::MAIN].into_iter().collect());
    assert!(job.bound.built_in_a_loop(), "{:?}", job.bound);
    let child = user.iter().find(|d| d.realizes.as_ref().is_some_and(|r| r.lowered == "Child")).expect("Child");
    assert!(matches!(&child.enclosing, Enclosing::Locus(d) if d.lowered == "Hub"));
    assert_eq!(child.domains, [PlacementTable::MAIN].into_iter().collect());
    assert!(matches!(child.bound, Bound::Unbounded(_)));
    assert_eq!(user.len(), 2, "the construction literal is a template, not a dynamic site");
}

/// The domains of the one user dynamic site that builds `name`.
fn dynamic_domains(t: &PlacementTable, name: &str) -> BTreeSet<DomainId> {
    let user: Vec<_> = t
        .dynamic
        .iter()
        .filter(|d| d.literal.universe == SiteUniverse::User && d.realizes.as_ref().is_some_and(|r| r.lowered == name))
        .collect();
    assert_eq!(user.len(), 1, "one dynamic `{name}`");
    user[0].domains.clone()
}

/// A placed field's literal runs its body where the entry placed it,
/// whatever scope the literal is written in (review of hale-lang/hale#1306):
/// restating a pinned or pooled field's default in `fn main` adds no main
/// execution of its body, so a locus built there runs in the placed
/// domain alone, as with the default spelling. A declaration built both
/// as a placed field and dynamically on main runs in both.
#[test]
fn a_static_fields_literal_runs_its_body_where_it_was_placed() {
    for name in ["static_field_explicit.hl", "static_field_default.hl"] {
        let s = clean(name);
        let t = table(&s);
        let (_, w) = one(t, "w");
        let (_, p) = one(t, "p");
        let (_, sh) = one(t, "s");
        assert!(is_pinned(t, w.domain) && is_pinned(t, sh.domain), "{name}");
        assert_eq!(pool_name(t, p.domain), Some("io"), "{name}");
        assert_eq!(dynamic_domains(t, "Child"), [w.domain].into_iter().collect(), "{name}: `Child` runs pinned alone");
        assert_eq!(dynamic_domains(t, "Leaf"), [p.domain].into_iter().collect(), "{name}: `Leaf` runs on `io` alone");
        assert_eq!(
            dynamic_domains(t, "Grand"),
            [PlacementTable::MAIN, sh.domain].into_iter().collect(),
            "{name}: `Shared` runs pinned and, built in `App.run()`, on main"
        );
        assert!(t.holes.iter().all(|h| !matches!(h.kind, HoleKind::UnknownDomains { .. })), "{name}");
    }
}

/// Case 11: two constructions of one root are two templates; a choice
/// among literals is one guarded step per alternative.
#[test]
fn two_constructions_are_two_templates() {
    let s = clean("two_constructions.hl");
    let t = table(&s);
    let root = t.root.as_ref().unwrap();
    assert_eq!(root.constructions.len(), 3);
    let gws = rows(t, "gw");
    assert_eq!(gws.len(), 3);
    let domains: BTreeSet<DomainId> = gws.iter().map(|(_, r)| r.domain).collect();
    assert_eq!(domains.len(), 3, "each construction's `gw` its own pinned domain");
    let routers: BTreeSet<&str> = rows(t, "gw.router").iter().map(|(_, r)| lowered(r)).collect();
    assert_eq!(routers, ["RouterV1", "RouterV2", "RouterV3"].into_iter().collect());
    for (k, r) in rows(t, "gw.router") {
        let owner = &t.instances[r.owner.as_ref().unwrap()];
        assert_eq!(r.domain, owner.domain);
        assert_eq!(k.origin, r.owner.as_ref().unwrap().origin, "a key's origin decides every row under it");
    }
    let alts = rows(t, "side?");
    assert_eq!(alts.len(), 2);
    assert!(alts.iter().all(|(k, r)| r.guarded && k.path[0].alternative == r.literal));
    let under: BTreeSet<&str> = rows(t, "side?.router").iter().map(|(_, r)| lowered(r)).collect();
    assert_eq!(under, ["RouterV1", "RouterV2"].into_iter().collect());
    assert!(rows(t, "side?.router").iter().all(|(_, r)| r.guarded));
    assert!(rows(t, "side").iter().all(|(_, r)| !r.guarded));
}

/// The choices the checker refuses: a conditional for a placed field
/// (rule 18), and `if` arms of two declarations.
#[test]
fn a_choice_at_a_placed_field_is_refused() {
    let s = check(&fixture("placed_conditional.hl"));
    let e = errors(&s);
    assert!(e.iter().any(|m| m.contains("placement entry `w`") && m.contains("conditional")), "{e:?}");
    assert!(e.iter().any(|m| m.contains("mismatched types")), "{e:?}");
}

/// The site of the seed's top-level `fn main`.
fn fn_main_site(s: &Snapshot) -> SiteRef {
    let id = s
        .programs()
        .values()
        .flat_map(|p| p.items.iter())
        .find_map(|i| match i {
            TopDecl::Fn(f) if f.name.name == "main" => Some(f.id),
            _ => None,
        })
        .expect("a `fn main`");
    SiteRef::user(s.identities().site_id(id).expect("minted"))
}

/// The root's site.
fn root_site(t: &PlacementTable) -> SiteRef {
    t.root.as_ref().expect("a root").realizes.site
}

/// Case 13: a claims-only `main locus`, which no literal builds, is the
/// entry's implicit template: `Origin::Entry` at `fn main`'s site, its
/// tower enumerated under it on main.
#[test]
fn a_claims_only_main_is_the_entrys_construction() {
    let s = clean("claims_only.hl");
    let t = table(&s);
    let root = t.root.as_ref().unwrap();
    assert!(root.constructions.is_empty(), "no literal builds the root");
    let entry = Origin::Entry(fn_main_site(&s));
    let (k, top) = one(t, "");
    assert_eq!(k.origin, entry);
    assert_eq!((lowered(top), top.domain, top.literal), ("App", PlacementTable::MAIN, Some(fn_main_site(&s))));
    for p in ["a", "a.k"] {
        let (k, r) = one(t, p);
        assert_eq!(k.origin, entry, "`{p}` is under the entry");
        assert_eq!(r.domain, PlacementTable::MAIN);
    }
}

/// Case 14: a library seed checked alone roots at its lowering root; with
/// no `fn main`, the entry is the root's own site, and its entries place
/// the tower under it.
#[test]
fn a_library_seed_alone_roots_at_its_lowering_root() {
    let s = clean("library_alone.hl");
    let t = table(&s);
    let entry = Origin::Entry(root_site(t));
    let (_, w) = one(t, "w");
    assert!(is_pinned(t, w.domain));
    assert!(matches!(&t.domain(w.domain).kind, DomainKind::Pinned { anchor, .. } if anchor.origin == entry));
    assert!(matches!(w.decided_by, Decision::Entry { .. }));
    assert_eq!(one(t, "w.k").1.domain, w.domain);
    assert_eq!(one(t, "idle").1.domain, PlacementTable::MAIN);
    assert!(t.instances.keys().all(|k| k.origin == entry));
    assert!(t.entry_literals.is_empty());
}

/// Case 15: a `fn main` building loci by verb. The root is the entry's;
/// each literal in `fn main` is a template on main bound by its
/// statement's loop context, with its fields as rows, and no dynamic site.
#[test]
fn a_fn_main_building_loci_by_verb_constructs_each() {
    let s = clean("verbs.hl");
    let t = table(&s);
    assert!(t.root.as_ref().unwrap().constructions.is_empty());
    assert_eq!(one(t, "l").0.origin, Origin::Entry(fn_main_site(&s)));
    let tops: BTreeMap<&str, &InstanceKey> = rows(t, "").into_iter().map(|(k, r)| (lowered(r), k)).collect();
    assert_eq!(tops.keys().copied().collect::<Vec<_>>(), ["App", "Listener", "Sender"]);
    let bound = |name: &str| {
        let Origin::Construction(lit) = tops[name].origin else { panic!("`{name}` is a literal's template") };
        t.entry_literals.iter().find(|c| c.literal == lit).map(|c| c.bound.clone()).expect("an entry literal")
    };
    assert_eq!(bound("Listener"), Bound::Once);
    assert!(matches!(bound("Sender"), Bound::Unbounded(why) if why == "built in a loop"));
    let (k, leaf) = one(t, "leaf");
    assert_eq!(k.origin, tops["Listener"].origin, "a field of the literal's locus is a row under it");
    assert_eq!((lowered(leaf), leaf.domain), ("Leaf", PlacementTable::MAIN));
    let user = t.dynamic.iter().filter(|d| d.literal.universe == SiteUniverse::User).count();
    assert_eq!(user, 0, "`fn main`'s literals are templates, not dynamic sites");
}

/// A field initialized from an existing instance claims none: a `Reuse`
/// hole naming the source, with its owner's domain and no literal.
#[test]
fn a_field_from_an_existing_instance_is_a_reuse_hole() {
    let s = clean("reuse.hl");
    let t = table(&s);
    let (_, h) = one(t, "h");
    let (k, roles) = one(t, "h.roles");
    assert_eq!((lowered(roles), roles.literal), ("Roles", None));
    assert_eq!(roles.domain, h.domain, "no domain of its own");
    assert!(matches!(&roles.decided_by, Decision::Inherited { from } if from.path.len() == 1));
    let reuse = HoleKind::Reuse { source: "r".into() };
    assert!(t.holes.iter().any(|x| x.at == HoleAt::Instance(k.clone()) && x.kind == reuse));
}

/// Case 16: a held instance's subtree lives in its holder's domain (K-8 /
/// M-8). `fn main` builds `r` on main and hands it to `h`, placed pinned:
/// the held row keeps the `Reuse` hole naming `r`, the source's rows are
/// projected under it in the pinned domain, inherited, and each row
/// names the source template's row it was built as. The legacy checker's
/// map and the model's arrangement place the subtree under the holder,
/// and the table now agrees with both.
#[test]
fn a_held_instances_subtree_lives_in_its_holders_domain() {
    let s = clean("held_subtree.hl");
    let t = table(&s);
    let (_, h) = one(t, "h");
    assert!(is_pinned(t, h.domain) && matches!(h.decided_by, Decision::Entry { .. }));

    // The source: `fn main`'s `Roles { }`, a template on main.
    let tops: BTreeMap<&str, &InstanceKey> = rows(t, "").into_iter().map(|(k, r)| (lowered(r), k)).collect();
    let source = tops["Roles"];
    assert!(matches!(source.origin, Origin::Construction(lit) if t.entry_literals.iter().any(|c| c.literal == lit)));
    let source_k = InstanceKey { origin: source.origin, path: one(t, "k").0.path.clone(), replica: None };
    assert_eq!(t.instances[source].domain, PlacementTable::MAIN, "built on main");
    assert_eq!(t.instances[&source_k].domain, PlacementTable::MAIN);

    // The held row: the hole, the holder's domain, built as the source.
    let (held, roles) = one(t, "h.roles");
    assert_eq!((lowered(roles), roles.literal, roles.domain), ("Roles", None, h.domain));
    assert!(matches!(&roles.decided_by, Decision::Inherited { from } if from.path.len() == 1));
    assert_eq!(roles.built_by.as_ref(), Some(source));
    let at_held: Vec<&HoleKind> = t.holes.iter().filter(|x| x.at == HoleAt::Instance(held.clone())).map(|x| &x.kind).collect();
    assert_eq!(at_held, [&HoleKind::Reuse { source: "r".into() }]);

    // Its subtree: the source's, under the holder, no hole.
    let (k, kr) = one(t, "h.roles.k");
    assert_eq!((lowered(kr), kr.domain), ("K", h.domain));
    assert_eq!(kr.literal, t.instances[&source_k].literal, "the literal that built the source's `k`");
    assert!(matches!(&kr.decided_by, Decision::Inherited { from } if from == held));
    assert_eq!(kr.built_by.as_ref(), Some(&source_k));
    assert!(!t.holes.iter().any(|x| x.at == HoleAt::Instance(k.clone())), "the hole is the held row's alone");
    assert_eq!(t.handed_off(), [source, &source_k].into_iter().collect::<BTreeSet<_>>());

    // Where `K` and `Roles` run, as the checker's F.31 rule and sync
    // inference read it (`running`, the handed-off rows skipped), is the
    // holder's pinned domain, as the legacy checker's map said; the
    // model's `App.h.roles.k` is the table's row.
    let running = t.running();
    for name in ["Roles", "K"] {
        let site = t.instances.values().find(|r| lowered(r) == name).and_then(|r| r.realizes.as_ref()).unwrap().site;
        let runs_in: BTreeSet<DomainId> = running.of_decl(site).iter().map(|k| t.instances[*k].domain).collect();
        assert_eq!(runs_in, [h.domain].into_iter().collect(), "`{name}` runs in the holder's domain");
        assert_eq!(PoolId::of_domain(t, h.domain), PoolId::Pinned("h".into()), "displayed as pinned at `h`");
    }
    let model = s.demand_model().unwrap_or_else(|_| panic!("the model is blocked"));
    let e = &model.entities;
    let i = e.locus_instances.iter().position(|x| x.path == "App.h.roles.k").expect("the arrangement holds `App.h.roles.k`");
    let placed = model.relations.placed_in.iter().find(|p| p.instance.0 as usize == i).expect("placed");
    assert_eq!(e.thread_domains[placed.domain.0 as usize].name, "pinned:App.h");
}

/// Case 16 with an override: `r` is built with `k: B { }`, not `Roles`'
/// default `A { }`. The held tree is the source's actual rows projected
/// under the holder, never the declaration's defaults: `h.roles.k`
/// realizes `B` and `h.roles.k.child` is a row in the holder's domain,
/// each naming its own source row, and both source rows are handed off.
#[test]
fn a_held_subtree_projects_its_sources_overrides() {
    let s = clean("held_override.hl");
    let t = table(&s);
    let (_, h) = one(t, "h");
    assert!(is_pinned(t, h.domain));
    let tops: BTreeMap<&str, &InstanceKey> = rows(t, "").into_iter().map(|(k, r)| (lowered(r), k)).collect();
    let source = tops["Roles"];
    let at_source = |p: &str| {
        let (k, r) = t.instances.iter().find(|(k, _)| k.origin == source.origin && path(k) == p).expect("a source row");
        assert_eq!(r.domain, PlacementTable::MAIN, "`{p}` was built on main");
        k
    };
    let (source_k, source_child) = (at_source("k"), at_source("k.child"));
    assert_eq!(lowered(&t.instances[source_k]), "B");

    let (held, roles) = one(t, "h.roles");
    assert_eq!((lowered(roles), roles.built_by.as_ref()), ("Roles", Some(source)));
    let (k, kr) = one(t, "h.roles.k");
    assert_eq!((lowered(kr), kr.domain, kr.built_by.as_ref()), ("B", h.domain, Some(source_k)));
    assert_eq!(kr.literal, t.instances[source_k].literal, "the override literal, not `A {{ }}`");
    assert!(matches!(&kr.decided_by, Decision::Inherited { from } if from == held));
    let (_, child) = one(t, "h.roles.k.child");
    assert_eq!((lowered(child), child.domain, child.built_by.as_ref()), ("Child", h.domain, Some(source_child)));
    assert!(matches!(&child.decided_by, Decision::Inherited { from } if from == k));
    assert!(!t.instances.values().any(|r| lowered(r) == "A"), "no row realizes the default nothing built");
    assert_eq!(t.handed_off(), [source, source_k, source_child].into_iter().collect::<BTreeSet<_>>());
    let at_held: Vec<&HoleKind> = t.holes.iter().filter(|x| x.at == HoleAt::Instance(held.clone())).map(|x| &x.kind).collect();
    assert_eq!(at_held, [&HoleKind::Reuse { source: "r".into() }]);
    assert!(!t.holes.iter().any(|x| x.at == HoleAt::Instance(k.clone())));
}

/// Case 16 with an unlinked source: the held instance reaches the root's
/// literal through a parameter, so no template is its source. The held
/// row keeps its `Reuse` hole, links nothing, and has nothing below it:
/// the subtree is unknown, and the declaration's defaults are not what
/// was built.
#[test]
fn a_held_row_whose_source_is_unlinked_asserts_no_subtree() {
    let s = clean("held_unlinked.hl");
    let t = table(&s);
    let (_, h) = one(t, "h");
    assert!(is_pinned(t, h.domain));
    let (held, roles) = one(t, "h.roles");
    assert_eq!((lowered(roles), roles.literal, roles.domain, roles.built_by.as_ref()), ("Roles", None, h.domain, None));
    let at_held: Vec<&HoleKind> = t.holes.iter().filter(|x| x.at == HoleAt::Instance(held.clone())).map(|x| &x.kind).collect();
    assert_eq!(at_held, [&HoleKind::Reuse { source: "r".into() }]);
    let below: Vec<String> = t
        .instances
        .keys()
        .filter(|k| k.origin == held.origin && k.path.len() > held.path.len() && k.path.starts_with(&held.path))
        .map(path)
        .collect();
    assert!(below.is_empty(), "nothing below an unlinked held row: {below:?}");
    assert!(t.handed_off().is_empty());
}

/// Checkpoint 4: an alternative one step under a replicated field. Every
/// row under replica `i` carries `Some(i)`, every row on or under the
/// choice is guarded, and the replicas stay three domains.
#[test]
fn an_alternative_under_replicas_keeps_its_replica_and_its_guard() {
    let s = clean("alternatives_under_replicas.hl");
    let t = table(&s);
    for i in 0..3u32 {
        let (_, v) = one(t, &format!("v[{i}]"));
        assert_eq!(rows(t, &format!("v.inner?[{i}]")).len(), 2);
        let under: Vec<_> = t.instances.iter().filter(|(k, _)| k.replica == Some(i) && k.path.len() >= 2).collect();
        assert_eq!(under.len(), 2 + 1 + 1 + 2, "two boxes, a leaf slot, a pair slot and the pair's two leaves");
        assert!(under.iter().all(|(_, r)| r.guarded && r.domain == v.domain));
    }
    let threads = t.domains.iter().filter(|d| matches!(d.kind, DomainKind::Pinned { .. })).count();
    assert_eq!(threads, 3, "three replicas, three threads, whatever the alternatives");
}

/// Case 12, the table's half: the stdlib rows are minted by the analysis
/// copy and say so, the colliding user declaration keeps its own key,
/// and every site the table names resolves into lowering's merged mint
/// exactly once.
#[test]
fn the_table_names_each_universe_and_joins_lowering_once() {
    let snap = build(&fixture("two_universes"));
    let t = table(&snap);
    let (_, c) = one(t, "c");
    let (_, u) = one(t, "u");
    let (_, buf) = one(t, "u.buf");
    let cd = c.realizes.as_ref().unwrap();
    let ud = u.realizes.as_ref().unwrap();
    assert_eq!(cd.site.id, ud.site.id, "the collision reaches the table");
    assert_eq!((cd.site.universe, ud.site.universe), (SiteUniverse::User, SiteUniverse::StdlibAnalysis));
    assert_eq!(lowered(buf), "__StdBytesBytesBuilder");
    assert_eq!(buf.literal.unwrap().universe, SiteUniverse::StdlibAnalysis);
    assert_eq!(buf.domain, u.domain, "the stdlib default inherits the stdlib locus's domain");
    let decls: BTreeSet<&DeclRef> = t.instances.values().filter_map(|r| r.realizes.as_ref()).collect();
    assert!(decls.contains(cd) && decls.contains(ud));

    let lowering = snap.demand_lowering().unwrap_or_else(|b| panic!("lowering blocked: {:?}", b.refused));
    let mut refs: Vec<LoweringRef<'_>> = Vec::new();
    for (k, r) in &t.instances {
        if let Some(d) = &r.realizes {
            refs.push(LoweringRef::Decl(d));
        }
        refs.extend(r.literal.map(LoweringRef::Site));
        refs.extend(k.path.iter().filter_map(|s| s.alternative).map(LoweringRef::Site));
    }
    let joined = join_lowering(&refs, snap.identities(), &lowering.merged, &lowering.snapshot)
        .unwrap_or_else(|e| panic!("{e}"));
    let distinct: BTreeSet<SiteRef> = refs
        .iter()
        .map(|r| match r {
            LoweringRef::Decl(d) => d.site,
            LoweringRef::Site(s) => *s,
        })
        .collect();
    assert_eq!(joined.len(), distinct.len(), "total over the table's refs");
    assert_eq!(joined.values().collect::<BTreeSet<_>>().len(), distinct.len(), "injective");
}

/// The fixtures every law test walks: those that check clean.
const CLEAN: [&str; 19] = [
    "claims_only.hl",
    "library_alone.hl",
    "verbs.hl",
    "two_instances.hl",
    "nested_inheritance.hl",
    "overrides.hl",
    "qualified.hl",
    "aliased.hl",
    "module_qualified.hl",
    "imported/no_own_main",
    "imported/own_main",
    "adapter.hl",
    "adapter_two_sites.hl",
    "dynamic.hl",
    "static_field_explicit.hl",
    "static_field_default.hl",
    "two_constructions.hl",
    "alternatives_under_replicas.hl",
    "two_universes",
];

/// The table's laws, over every fixture that checks clean: every owner
/// is a row; a nested row inherits its owner's domain unless an entry or
/// a binding decides it, and only a root field has an entry; replica
/// rows are exactly `0..K`; two rows share a pinned domain only when one
/// is under the other; one pool domain per name; no accepting owner is
/// anchored in a pinned domain (rule 6); and a clean program's static
/// rows have no hole.
#[test]
fn the_table_keeps_its_laws_over_every_fixture() {
    for name in CLEAN {
        let s = clean(name);
        let t = table(&s);
        let mut pools: BTreeMap<&str, DomainId> = BTreeMap::new();
        for d in &t.domains {
            if let DomainKind::Pool { name: p, .. } = &d.kind {
                assert!(pools.insert(p, d.id).is_none(), "{name}: pool `{p}` is one domain");
            }
        }
        let accepting: BTreeSet<String> = s
            .programs()
            .values()
            .flat_map(|p| flat_decls(&p.items))
            .filter_map(|i| match i {
                TopDecl::Locus(l)
                    if l.members.iter().any(|m| {
                        matches!(m, LocusMember::Lifecycle(lc) if lc.kind == hale_syntax::ast::LifecycleKind::Accept)
                    }) =>
                {
                    Some(l.name.name.clone())
                }
                _ => None,
            })
            .collect();
        for (k, r) in &t.instances {
            match &r.owner {
                None => assert!(k.path.is_empty(), "{name}: only an origin's top has no owner"),
                Some(o) => {
                    let owner = t.instances.get(o).unwrap_or_else(|| panic!("{name}: `{}`'s owner is a row", path(k)));
                    match &r.decided_by {
                        Decision::Entry { .. } => assert!(
                            k.path.len() == 1 && !matches!(k.origin, Origin::Binding(_)),
                            "{name}: entries decide root fields only"
                        ),
                        Decision::Inherited { from } => {
                            assert_eq!(from, o);
                            assert_eq!(r.domain, owner.domain, "{name}: `{}` inherits", path(k));
                        }
                        Decision::Default => assert_eq!(r.domain, PlacementTable::MAIN),
                        Decision::Binding { .. } => panic!("{name}: a binding decides only an adapter's top"),
                    }
                    assert_eq!(r.owner_relative == OwnerRelative::OffOwner, r.domain != owner.domain);
                    assert_eq!(k.origin, o.origin);
                }
            }
            if let DomainKind::Pinned { anchor, .. } = &t.domain(r.domain).kind {
                assert!(
                    anchor == k
                        || (k.origin == anchor.origin && k.replica == anchor.replica && k.path.starts_with(&anchor.path)),
                    "{name}: `{}` shares a pinned domain only under its anchor",
                    path(k)
                );
                assert!(!accepting.contains(lowered(r)), "{name}: an accepting owner is never in a pinned domain");
            }
        }
        let mut families: BTreeMap<(Origin, String), BTreeSet<u32>> = BTreeMap::new();
        for k in t.instances.keys().filter(|k| k.path.len() == 1) {
            if let Some(i) = k.replica {
                families.entry((k.origin, k.path[0].field.clone())).or_default().insert(i);
            }
        }
        for ((_, f), idx) in &families {
            assert_eq!(idx.iter().copied().collect::<Vec<_>>(), (0..idx.len() as u32).collect::<Vec<_>>(), "{name}: `{f}`");
        }
        let static_holes: Vec<_> =
            t.holes.iter().filter(|h| matches!(h.at, hale_types::placement::HoleAt::Instance(_))).collect();
        assert!(static_holes.is_empty(), "{name}: {static_holes:?}");
    }
}
