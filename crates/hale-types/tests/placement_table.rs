//! The placement table (F.40 phase 3, P1; the correspondence is
//! `notes/f40-placement-correspondence.md`, hale-lang/hale#1296).
//!
//! Every case loads its seed the way every verb does — the frontend's
//! load, the desugar sequence, the mint — from a fixture under
//! `fixtures/placement/`, never from a bundle the test mints itself.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot, Target};
use hale_frontend::source::Disk;
use hale_syntax::ast::{flat_decls, Expr, LocusDecl, LocusMember, ParamInit, Program, TopDecl};
use hale_syntax::sites::SiteKind;
use hale_types::placement::{join_lowering, provenance, DeclRef, LoweringRef, SiteRef, SiteUniverse};
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
