//! `FnKey` is the declaration's identity (F.40 phase 3, C3 rest).
//!
//! Every fn-like declaration the allocation summary walks (a free fn, a
//! locus method, an authored lifecycle hook, a mode) is a row keyed by
//! its minted site, as the universe that minted it and its index there
//! (`DeclId`); the (locus name, fn name) pair is the row's display name.
//! The effect rows, the certificate engines and every other reader that
//! joins by `FnKey` join by that identity; a call resolves to a row
//! through the summary's one resolution (`AllocSummary::resolve`).
//!
//! The classified correction: two declarations sharing a name were one
//! row, the later declaration's body, and are now two. The checker
//! refuses such a program (`duplicate top-level name`), so the corpus,
//! `tests/hale`, every DNA seed and the stdlib hold none, and only
//! `--dump-alloc-summary`, which prints whatever the check says, shows
//! the second row.

use hale_types::alloc_summary::{summarize_identified, AllocSummary, DeclId, FnKey};
use hale_types::placement::SiteUniverse;

fn minted(src: &str) -> (hale_syntax::ast::Program, hale_types::snapshot::Snapshot) {
    let mut program = hale_syntax::parse_source(src).expect("parse");
    let ids = hale_types::snapshot::mint([("app.hl", &mut program)], &[]);
    (program, ids)
}

fn summarize(src: &str) -> AllocSummary {
    let (program, ids) = minted(src);
    summarize_identified(&[(&program, &ids)], &[])
}

/// A free fn, a method, an authored hook and a mode, each calling one
/// helper.
const MEMBERS: &str = r#"
        fn helper(n: Int) -> Int { return n + 1; }
        locus W {
            params { n: Int = 0; }
            birth { self.n = helper(1); }
            fn step(k: Int) -> Int { return helper(k); }
            mode bulk() { }
        }
        fn main() { W { }; }
    "#;

/// Each row's key names the declaration it was walked from: a free fn,
/// a method, an authored hook and a mode, by their own sites.
#[test]
fn every_row_is_keyed_by_its_declarations_site() {
    let (program, ids) = minted(MEMBERS);
    let summary = summarize_identified(&[(&program, &ids)], &[]);
    let mut expected: Vec<(String, DeclId)> = Vec::new();
    for item in &program.items {
        match item {
            hale_syntax::ast::TopDecl::Fn(f) => {
                expected.push((f.name.name.clone(), DeclId::user(f.id).expect("minted")));
            }
            hale_syntax::ast::TopDecl::Locus(l) => {
                for m in &l.members {
                    let (name, id) = match m {
                        hale_syntax::ast::LocusMember::Fn(f) => (f.name.name.clone(), f.id),
                        hale_syntax::ast::LocusMember::Lifecycle(lc) if !lc.synthesized => ("birth".to_string(), lc.id),
                        hale_syntax::ast::LocusMember::Mode(md) => ("bulk".to_string(), md.id),
                        _ => continue,
                    };
                    expected.push((format!("{}::{}", l.name.name, name), DeclId::user(id).expect("minted")));
                }
            }
            _ => {}
        }
    }
    expected.sort();
    let mut rows: Vec<(String, DeclId)> =
        summary.fns.keys().map(|k| (k.display(), k.decl.expect("a minted row has an identity"))).collect();
    rows.sort();
    assert_eq!(rows, expected);
    assert!(rows.iter().all(|(_, d)| d.universe == SiteUniverse::User));
    // A call's edge names the callee's row, identity included.
    let helper = summary.resolve(None, "helper").expect("helper resolves").clone();
    let step = &summary.fns[summary.resolve(Some("W"), "step").expect("step resolves")];
    assert!(step.calls.iter().any(|c| c.callee == hale_types::alloc_summary::Callee::Resolved(helper.clone())));
    assert_eq!(helper.decl, expected.iter().find(|(n, _)| n == "helper").map(|(_, d)| *d));
}

/// The model's function rows carry their declaration's site
/// (`Function::decl`), and FunctionId stays the rank of the name (law 2):
/// the builder joins a summary row to its function by that site, so a
/// call relation names the function the callee's row is.
#[test]
fn the_models_functions_carry_their_declarations_site() {
    let (program, _) = minted(MEMBERS);
    let mut sites: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for item in &program.items {
        match item {
            hale_syntax::ast::TopDecl::Fn(f) => {
                sites.insert(f.name.name.clone(), f.id.0);
            }
            hale_syntax::ast::TopDecl::Locus(l) => {
                for m in &l.members {
                    match m {
                        hale_syntax::ast::LocusMember::Fn(f) => {
                            sites.insert(format!("W::{}", f.name.name), f.id.0);
                        }
                        hale_syntax::ast::LocusMember::Lifecycle(lc) if !lc.synthesized => {
                            sites.insert("W::birth".to_string(), lc.id.0);
                        }
                        hale_syntax::ast::LocusMember::Mode(md) => {
                            sites.insert("W::bulk".to_string(), md.id.0);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    let bundle = hale_types::Bundle::new([("app.hl".to_string(), &program)].into_iter().collect());
    let model = hale_types::model_builder::derive_application_model(&bundle);
    model.validate().expect("a model");
    let functions = &model.entities.functions;
    let names: Vec<&str> = functions.iter().map(|f| f.name.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "FunctionId is the rank of the name");
    let by_site: Vec<(String, u32)> =
        functions.iter().map(|f| (f.name.clone(), f.decl.expect("a minted declaration").index)).collect();
    assert_eq!(by_site, sites.into_iter().collect::<Vec<_>>());
    let id = |n: &str| functions.iter().position(|f| f.name == n).expect(n) as u32;
    for caller in ["W::birth", "W::step"] {
        assert!(
            model.relations.calls.iter().any(|c| c.from.0 == id(caller) && c.to.0 == id("helper")),
            "{caller} calls helper"
        );
    }
}

/// The stdlib's analysis copy is a universe of its own: its rows never
/// share an identity with the program's, whatever their indices.
#[test]
fn the_stdlib_copy_is_its_own_universe() {
    let program = hale_syntax::parse_source("fn main() { }").expect("parse");
    let bundle = hale_types::Bundle::new([("app.hl".to_string(), &program)].into_iter().collect());
    let summary = hale_types::alloc_summary::derive_alloc_summary(&bundle);
    assert!(!summary.analysis_copy.is_empty(), "the stdlib's analysis copy is beside the program");
    for k in &summary.analysis_copy {
        assert_eq!(k.decl.map(|d| d.universe), Some(SiteUniverse::StdlibAnalysis), "{}", k.display());
    }
}

/// Two declarations sharing a name are two rows (the classified
/// correction); a bare call of the name reaches the later declaration's,
/// as it did when the two were one row holding the later body.
#[test]
fn two_declarations_sharing_a_name_are_two_rows() {
    let src = r#"
        module a {
            fn f(x: Int) -> Int { return x + 1; }
        }
        module b {
            fn f(x: Int) -> Int { return x * 2; }
        }
        fn g() -> Int { return f(1); }
        fn main() { println(g()); }
    "#;
    let summary = summarize(src);
    let named_f: Vec<&FnKey> = summary.fns.keys().filter(|k| k.display() == "f").collect();
    assert_eq!(named_f.len(), 2, "one row per declaration: {:?}", summary.fns.keys().collect::<Vec<_>>());
    assert_ne!(named_f[0].decl, named_f[1].decl);
    let later = named_f.iter().max_by_key(|k| k.decl).copied().expect("two rows");
    assert_eq!(summary.resolve(None, "f"), Some(later));
    let g = &summary.fns[summary.resolve(None, "g").expect("g")];
    assert!(g.calls.iter().any(|c| c.callee == hale_types::alloc_summary::Callee::Resolved(later.clone())));
    // The dump lists both rows, each under its display name.
    let dump = hale_types::dump_alloc_summary(&summary);
    assert_eq!(dump.matches("\nfn f   [scratch-local]\n").count(), 2, "{dump}");
    assert!(dump.contains("# 4 fns,"), "{dump}");
    // The checker refuses the program, which is why no checked program
    // has two such rows.
    let (program, _) = minted(src);
    let diags = hale_types::check_program(&program);
    assert!(
        diags.iter().any(|d| d.message == "duplicate top-level name `f`"),
        "{:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}
