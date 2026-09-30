//! The snapshot's identity laws (F.40 phase 1.1b): every semantic site
//! has one id; ids are unique across seeds even where spans overlap;
//! minting is idempotent; a clone keeps its ids; generated declarations
//! are sites too.

use hale_graph::ids::SeedId;
use hale_syntax::sites::{for_each_site, SiteKind};
use hale_types::snapshot::mint;
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
