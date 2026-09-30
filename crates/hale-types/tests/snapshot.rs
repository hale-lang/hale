//! The snapshot's identity laws (F.40 phase 1.1b): every semantic site
//! has one id; ids are unique across seeds even where spans overlap;
//! minting is idempotent; a clone keeps its ids; generated declarations
//! are sites too.

use hale_graph::ids::SeedId;
use hale_syntax::sites::{for_each_site, SiteKind};
use hale_types::snapshot::{mint, Origin};
use hale_types::symbol::SourceFile;

fn parse(src: &str) -> hale_syntax::ast::Program {
    hale_syntax::parse_source(src).expect("parse")
}

fn ids(p: &hale_syntax::ast::Program) -> Vec<u32> {
    let mut out = Vec::new();
    for_each_site(p, &mut |_, _, id| out.push(id.0));
    out
}

#[test]
fn shadowed_bindings_are_distinct_sites() {
    let mut p = parse(
        r#"
fn e(t: Bool) -> Int {
    let r = 1;
    if t { let r = 2; }
    return r;
}
fn main() { e(true); }
"#,
    );
    let snap = mint([("app.hl", &mut p)], &[]);
    let lets: Vec<_> = snap.sites.iter().filter(|s| s.kind == SiteKind::Let).collect();
    assert_eq!(lets.len(), 2, "two `let r` sites: {:?}", snap.sites);
    assert_ne!(lets[0].id, lets[1].id);
    assert!(ids(&p).iter().all(|i| *i != u32::MAX), "every site is numbered");
    let all = ids(&p);
    let mut sorted = all.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), all.len(), "ids are unique");
}

#[test]
fn overlapping_spans_in_two_seeds_get_distinct_ids_and_their_own_seed() {
    let src = "locus A { params { n: Int = 0; } }\nfn main() { A { }; }\n";
    let mut user = parse(src);
    let mut lib = parse(src); // identical text, identical spans
    let snap = mint([("app.hl", &mut user), ("lib.hl", &mut lib)], &[]);
    let user_ids = ids(&user);
    let lib_ids = ids(&lib);
    assert!(!user_ids.is_empty());
    assert!(user_ids.iter().all(|i| !lib_ids.contains(i)), "no id is shared across seeds");
    let seeds: std::collections::BTreeSet<SeedId> = snap.sites.iter().map(|s| s.id.seed).collect();
    assert_eq!(seeds.len(), 2, "each program is its own seed without a source map");
    assert_eq!(snap.seeds, vec!["app.hl".to_string(), "lib.hl".to_string()]);
}

#[test]
fn the_seed_comes_from_the_source_map_when_there_is_one() {
    let src = "fn main() { let x = 1; }\n";
    let mut p = parse(src);
    let sources = vec![SourceFile {
        id: 0,
        path: "main.hl".into(),
        digest: "0".into(),
        base: 0,
        len: src.len() as u32,
    }];
    let snap = mint([("main.hl", &mut p)], &sources);
    assert!(snap.sites.iter().all(|s| s.id.seed == SeedId(0)));
    assert_eq!(snap.seeds, vec!["main.hl".to_string()]);
}

/// Review of phase 1, finding 9: the resolved program mints its merged
/// program with the bundle's source map, so a user site keeps the seed
/// the bundle's mint gave it, and the bundled stdlib, whose spans
/// overlap the first file's, is a seed of its own under its name.
#[test]
fn the_resolved_snapshot_seeds_by_the_bundle_and_names_the_stdlib() {
    let src = "locus L { params { n: Int = 0; } }\nfn main() { L { }; }\n";
    let mut p = parse(src);
    let sources = vec![SourceFile {
        id: 0,
        path: "main.hl".into(),
        digest: "0".into(),
        base: 0,
        len: src.len() as u32,
    }];
    let bundle_snap = mint([("main.hl", &mut p)], &sources);
    let resolved = hale_types::resolved::resolve_program(&p, &sources, &[], None, None)
        .expect("resolve");
    let snap = &resolved.snapshot;
    assert_eq!(
        snap.seeds,
        vec!["main.hl".to_string(), hale_types::snapshot::STDLIB_SEED.to_string()]
    );
    for site in &bundle_snap.sites {
        assert_eq!(snap.site(site.id).map(|s| s.id.seed), Some(SeedId(0)), "{site:?}");
    }
    assert!(
        snap.sites.iter().any(|s| s.id.seed == SeedId(1)),
        "the stdlib's sites carry the stdlib's seed"
    );
}

#[test]
fn minting_is_idempotent_and_a_clone_keeps_its_ids() {
    let mut p = parse("locus L { fn f() { } }\nfn main() { L { }; }\n");
    let first = mint([("app.hl", &mut p)], &[]);
    let before = ids(&p);
    let second = mint([("app.hl", &mut p)], &[]);
    assert_eq!(ids(&p), before, "a second mint changes no id");
    assert_eq!(first.len(), second.len());
    let clone = p.clone();
    assert_eq!(ids(&clone), before, "a clone is the same sites");
}

#[test]
fn a_pre_numbered_node_keeps_its_id_and_the_counter_continues_past_it() {
    let mut p = parse("fn main() { let a = 1; let b = 2; }\n");
    // Number one site by hand, as a pre-pass would.
    let mut numbered = false;
    hale_syntax::sites::for_each_site_mut(&mut p, &mut |kind, _, id| {
        if kind == SiteKind::Let && id.is_none() && !numbered {
            *id = hale_syntax::ast::NodeId(40);
            numbered = true;
        }
    });
    let snap = mint([("app.hl", &mut p)], &[]);
    assert!(snap.site(hale_graph::ids::SiteId::new(SeedId(0), 40)).is_some());
    assert!(ids(&p).iter().all(|i| *i != u32::MAX), "all numbered");
    assert!(ids(&p).iter().any(|i| *i > 40), "the counter continued past the pre-numbered id");
    let mut all = ids(&p);
    all.sort();
    all.dedup();
    assert_eq!(all.len(), ids(&p).len(), "no collision with the pre-numbered site");
}

#[test]
fn nested_modules_and_generated_declarations_are_sites() {
    let src = r#"
module m {
    locus Inner { params { n: Int = 0; } }
}
type Order { id: Int; }
fn main() { let o = Order { id: 1 }; }
"#;
    let mut plain = parse(src);
    let plain_count = mint([("app.hl", &mut plain)], &[]).len();
    let mut generated = parse(src);
    hale_syntax::json_gen::generate_json_parsers(&mut generated);
    let generated_count = mint([("app.hl", &mut generated)], &[]).len();
    assert!(plain_count > 0);
    assert!(
        generated_count >= plain_count,
        "generated declarations are sites too ({generated_count} vs {plain_count})"
    );
    let kinds: Vec<SiteKind> = {
        let mut out = Vec::new();
        for_each_site(&plain, &mut |k, _, _| out.push(k));
        out
    };
    assert!(kinds.contains(&SiteKind::Module) && kinds.contains(&SiteKind::Locus));
}

/// The sites of `item`, by index.
fn item_sites(item: &hale_syntax::ast::TopDecl) -> Vec<u32> {
    let mut out = Vec::new();
    hale_syntax::sites::for_each_site_in_item(item, &mut |_, _, id| out.push(id.0));
    out
}

fn origin_at(snap: &hale_types::snapshot::Snapshot, index: u32) -> Option<Origin> {
    snap.origins.iter().find(|(s, _)| s.index == index).map(|(_, o)| *o)
}

/// F.40 phase 1.1b-iii: a bundle built the way `hale check` builds one
/// (parse, the JSON parsers, sync inference, the api surface, then the
/// mint over the source map) carries its snapshot, and every
/// declaration the desugars generated has an origin row, as does every
/// site inside it.
#[test]
fn a_check_shaped_bundle_carries_its_snapshot_and_every_generated_declaration_has_an_origin() {
    let src = r#"
type Order { id: Int `json:"id"`; note: String `json:"note"`; }
type Verdict { review_id: Int; verdict: String; }
type VerdictResult { ok: Bool; note: String; }
topic Verdicts { payload: Verdict; subject: "app.verdict"; }
locus Billing {
    params { seen: Int = 0; }
    bus { subscribe Verdicts as on_verdict; }
    fn on_verdict(v: Verdict) -> VerdictResult {
        self.seen = self.seen + 1;
        return VerdictResult { ok: true, note: v.verdict };
    }
}
main locus App {
    params { billing: Billing = Billing { }; }
    bindings { api: unix("/tmp/t.sock", bound: 8, on_full: refuse); }
}
fn main() {
    let o = Order::from_json("{\"id\": 1, \"note\": \"n\"}") or Order { id: 0, note: "" };
    println(o.note);
    App { };
}
"#;
    let path = std::path::PathBuf::from("app.hl");
    let mut programs = std::collections::BTreeMap::new();
    programs.insert(path.clone(), parse(src));
    for prog in programs.values_mut() {
        hale_syntax::json_gen::generate_json_parsers(prog);
        let _ = hale_types::apply_sync_inference(prog);
    }
    {
        let mut refs: Vec<&mut hale_syntax::ast::Program> = programs.values_mut().collect();
        assert!(
            hale_syntax::api_gen::generate_api(&mut refs, None).is_some(),
            "the api binding lowers"
        );
    }
    let sources = vec![SourceFile {
        id: 0,
        path: "app.hl".into(),
        digest: "0".into(),
        base: 0,
        len: src.len() as u32,
    }];
    let names: Vec<String> = programs.keys().map(|p| p.display().to_string()).collect();
    let snapshot = mint(names.iter().map(String::as_str).zip(programs.values_mut()), &sources);
    let bundle_programs: std::collections::BTreeMap<String, &hale_syntax::ast::Program> =
        programs.iter().map(|(p, prog)| (p.display().to_string(), prog)).collect();
    let mut bundle = hale_types::Bundle::new(bundle_programs);
    bundle.sources = sources;
    bundle.snapshot = snapshot;

    assert!(bundle.snapshot.len() > 0);
    let prog = &programs[&path];
    let mut json = 0;
    let mut api = 0;
    for item in &prog.items {
        use hale_syntax::ast::TopDecl;
        let expected = match item {
            TopDecl::Fn(fd)
                if fd.name.name.starts_with("__json_parse_")
                    || fd.name.name.starts_with("__json_to_json_") =>
            {
                Some(Origin::JsonParsers)
            }
            TopDecl::Type(t) if t.synthetic => Some(Origin::JsonParsers),
            TopDecl::Fn(fd)
                if fd.name.name.starts_with("__api_decode_")
                    || fd.name.name.starts_with("__api_encode_") =>
            {
                Some(Origin::ApiSurface)
            }
            other if other.span().start.0 >= hale_syntax::api_gen::API_SYNTH_BASE => {
                Some(Origin::ApiSurface)
            }
            _ => None,
        };
        let sites = item_sites(item);
        match expected {
            Some(origin) => {
                match origin {
                    Origin::JsonParsers => json += 1,
                    _ => api += 1,
                }
                assert!(!sites.is_empty());
                for index in sites {
                    assert_eq!(
                        origin_at(&bundle.snapshot, index),
                        Some(origin),
                        "site {index} of a generated declaration: {:?}",
                        bundle.snapshot.site(hale_graph::ids::SiteId::new(SeedId(0), index))
                    );
                }
            }
            None => {
                // A written declaration's own site has no row; its
                // generated members (the api subscriber) may.
                assert_eq!(origin_at(&bundle.snapshot, sites[0]), None);
            }
        }
    }
    assert!(json >= 3, "JsonError and Order's parser and emitter ({json})");
    assert!(api > 0, "the api surface's declarations ({api})");
    // The subscriber member the api surface adds to a written locus.
    let generated_subscribe = bundle.snapshot.sites.iter().any(|s| {
        s.kind == SiteKind::Subscribe
            && s.span.start.0 >= hale_syntax::api_gen::API_SYNTH_BASE
            && bundle.snapshot.origin(s.id) == Some(Origin::ApiSurface)
    });
    assert!(generated_subscribe);
}

/// Review of phase 1, finding 19: the file-entry verbs mint before any
/// desugar, and the api surface then copies the entry's expressions
/// (the socket path, `roles:`, `principals:`, the HTTP host and port,
/// a subscriber's key filter) into what it generates while the entry
/// stays. Every copy is a new site: the second mint neither panics on
/// a shared id nor lends a copy the original's.
#[test]
fn the_api_surface_copies_the_entry_expressions_as_new_sites() {
    let src = r#"
fn sock() -> String { return "/tmp/api.sock"; }
fn port() -> Int { return 8080; }
fn which() -> String { return "k"; }
locus Table { fn holds(p: std::api::Principal, r: String) -> Bool { return true; } }
locus Tokens {
    fn principal(token: String) -> std::api::Principal { return std::api::Principal { mode: "bearer", name: "" }; }
    fn refused() -> String { return "no"; }
}
type Ping { key: String = ""; }
topic Pings { payload: Ping; subject: "t.ping"; keyed_by key; }
locus Echo {
    bus { subscribe Pings as on_ping where key == which(); }
    fn on_ping(p: Ping) -> Ping { return p; }
}
main locus App {
    params { echo: Echo = Echo { }; }
    bindings {
        api: unix(sock(), bound: 8, on_full: refuse, roles: Table { }),
            http("127.0.0.1", port(), principals: Tokens { });
    }
}
fn main() { App { }; }
"#;
    let mut p = parse(src);
    mint([("app.hl", &mut p)], &[]);
    let first: std::collections::BTreeSet<u32> = ids(&p).into_iter().collect();
    {
        let mut refs = vec![&mut p];
        assert!(hale_syntax::api_gen::generate_api(&mut refs, None).is_some(), "the api binding lowers");
    }
    // Two sites with one id is a panic here.
    let snap = mint([("app.hl", &mut p)], &[]);
    let after = ids(&p);
    assert!(after.iter().all(|i| *i != u32::MAX), "every site is numbered");
    // Each copied expression is two sites at one span: the entry's,
    // keeping its id, and the copy's, with an id the first mint never
    // gave out.
    for (text, kind) in [
        ("sock()", SiteKind::Call),
        ("port()", SiteKind::Call),
        ("which()", SiteKind::Call),
        ("Table { }", SiteKind::StructLiteral),
        ("Tokens { }", SiteKind::StructLiteral),
    ] {
        let start = src.find(&format!("{text},")).or_else(|| src.find(&format!("{text})")))
            .or_else(|| src.find(&format!("{text};")))
            .expect(text) as u32;
        let at: Vec<u32> = snap
            .sites
            .iter()
            .filter(|s| s.kind == kind && s.span.start.0 == start)
            .map(|s| s.id.index)
            .collect();
        assert_eq!(at.len(), 2, "`{text}`: the entry's site and its copy's ({at:?})");
        assert_eq!(
            at.iter().filter(|i| first.contains(i)).count(),
            1,
            "`{text}`: the copy's id is fresh ({at:?}, first mint {first:?})"
        );
    }
}

/// The per-site markers: the `run` the omitted-run desugar adds, and
/// the bindings the chains rewrite introduces.
#[test]
fn the_omitted_run_and_the_chain_bindings_have_origins() {
    let mut p = parse(
        r#"
locus L { params { n: Int = 0; } }
fn count(xs: Vec<Int>) -> Int { let c = xs.filter(it > 2).count(); return c; }
fn main() { L { }; }
"#,
    );
    hale_syntax::desugar::desugar_omitted_run(&mut p);
    let snap = mint([("app.hl", &mut p)], &[]);
    let of = |kind: SiteKind| -> Vec<Option<Origin>> {
        snap.sites.iter().filter(|s| s.kind == kind).map(|s| snap.origin(s.id)).collect()
    };
    assert_eq!(of(SiteKind::Lifecycle), vec![Some(Origin::OmittedRun)]);
    assert_eq!(of(SiteKind::Locus), vec![None]);
    let lets = of(SiteKind::Let);
    assert!(lets.contains(&Some(Origin::ChainDesugar)), "{lets:?}");
    assert!(lets.contains(&None), "the written `let c`: {lets:?}");
}
