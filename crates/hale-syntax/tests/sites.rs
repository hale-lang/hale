//! `hale_syntax::sites`: the one walk over the AST's identity fields
//! (F.40 phase 1.1b).
//!
//! Two properties. The walk visits the sites a hand-written program
//! holds in the pre-order the module promises, one of each kind. And
//! on every corpus program it misses nothing: after the mutable walk
//! numbers every site it reaches, the program's Debug rendering — a
//! route through the AST that shares no code with the walk — holds no
//! `NodeId::NONE`, and the number of sites equals the number of
//! identity fields that rendering showed before the walk.

use std::collections::BTreeSet;

use hale_syntax::ast::{NodeId, Program};
use hale_syntax::sites::{for_each_site, for_each_site_mut, SiteKind};

/// How `NodeId::NONE` renders under `{:?}`.
const NONE_RENDERED: &str = "NodeId(4294967295)";

fn kinds(p: &Program) -> Vec<SiteKind> {
    let mut out = Vec::new();
    for_each_site(p, &mut |k, _, _| out.push(k));
    out
}

const EVERY_KIND: &str = r#"
topic Tick { payload: Int; }
type Point { x: Int; y: Int; }
const LIMIT: Int = 3;
interface Store { fn get() -> Int; }
group workers = { Worker };
module util {
    fn helper() -> Int { return 1; }
}
perspective Router { fn route(code: Int) -> Int; }
locus Worker {
    params { n: Int = 0; }
    bus {
        subscribe Tick as on_tick;
        publish Tick;
    }
    closure steady { self.n ~~ self.n within 0; }
    birth { let x = 1; }
    mode bulk() -> Int { return self.n; }
    on_failure(c: Worker, err: ClosureViolation) { }
    fn on_tick(v: Int) {
        let (a, b) = (v, 2);
        self.n = helper();
        for i in 0..3 { }
        let p = Point { x: a, y: b };
        Tick <- p.x;
    }
}
main locus App {
    params { w: Worker = Worker { }; }
    placement { w: cooperative; }
    bindings { Tick: unix("/tmp/sites.sock"); }
}
"#;

#[test]
fn one_of_each_kind_in_pre_order() {
    use SiteKind::*;
    let p = hale_syntax::parse_source(EVERY_KIND).expect("parses");
    let expected = vec![
        Topic,
        Type,
        Const,
        Interface,
        Group,
        Module,
        Fn, // util::helper
        Perspective,
        Fn, // Router::route
        Locus,
        Param,
        Subscribe,
        Publish,
        Closure,
        Lifecycle,
        Let, // let x
        Mode,
        Failure,
        Fn, // on_tick
        LetTuple,
        Assign,
        Call, // helper()
        For,
        Let, // let p
        StructLiteral,
        Send,
        Locus, // App
        Param,
        StructLiteral, // Worker { }
        PlacementEntry,
        BindingEntry,
    ];
    assert_eq!(kinds(&p), expected);

    // The program exercises every kind the enum names.
    let seen: BTreeSet<SiteKind> = expected.iter().copied().collect();
    let all: BTreeSet<SiteKind> = [
        Locus, Fn, Topic, Type, Interface, Closure, Lifecycle, Mode, Failure,
        Perspective, Const, Group, Module, Param, PlacementEntry,
        BindingEntry, Publish, Subscribe, Let, LetTuple, Assign, For, Send,
        StructLiteral, Call,
    ]
    .into_iter()
    .collect();
    assert_eq!(seen, all);

    // The parser mints nothing.
    for_each_site(&p, &mut |k, _, id| {
        assert!(id.is_none(), "{k:?} parsed with an id");
    });
}

/// Number every site with the mutable walk, read the numbers back
/// with the read-only one, and check the Debug rendering for a site
/// neither walk reached. Returns the site count.
fn number_and_check(origin: &str, p: &mut Program) -> usize {
    let before = format!("{p:?}").matches(NONE_RENDERED).count();

    let mut mut_kinds = Vec::new();
    let mut next = 0u32;
    for_each_site_mut(p, &mut |k, _, id| {
        *id = NodeId(next);
        next += 1;
        mut_kinds.push(k);
    });

    let mut read_kinds = Vec::new();
    let mut expect = 0u32;
    for_each_site(p, &mut |k, _, id| {
        assert_eq!(id.0, expect, "{origin}: site {expect} ({k:?}) read back {}", id.0);
        expect += 1;
        read_kinds.push(k);
    });
    assert_eq!(mut_kinds, read_kinds, "{origin}: the two walks disagree on kinds");

    let after = format!("{p:?}").matches(NONE_RENDERED).count();
    assert_eq!(after, 0, "{origin}: {after} identity field(s) the walk never reached");
    assert_eq!(next as usize, before, "{origin}: walk count vs identity fields");
    next as usize
}

#[test]
fn corpus_walk_reaches_every_identity_field() {
    let programs =
        hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok());
    assert!(programs.len() > 900, "corpus yielded only {} programs", programs.len());
    let mut total = 0usize;
    let mut by_kind = std::collections::BTreeMap::<SiteKind, usize>::new();
    for prog in &programs {
        let mut p = hale_syntax::parse_source(&prog.source).expect("parseable");
        total += number_and_check(&prog.origin, &mut p);
        for k in kinds(&p) {
            *by_kind.entry(k).or_default() += 1;
        }
    }
    eprintln!("{} programs, {} sites: {:?}", programs.len(), total, by_kind);
    // Vacuity: every kind turns up somewhere in the corpus.
    assert_eq!(by_kind.len(), 25, "kinds seen: {:?}", by_kind.keys());
}

/// The same over the syntax crate's own desugars, which synthesize
/// declarations, statements and calls the parser never writes.
#[test]
fn desugared_corpus_walk_reaches_every_identity_field() {
    let programs =
        hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok());
    let mut total = 0usize;
    for prog in &programs {
        let mut p = hale_syntax::parse_source(&prog.source).expect("parseable");
        hale_syntax::chains::desugar_chains(&mut p);
        hale_syntax::desugar::desugar_topics(&mut p);
        hale_syntax::desugar::desugar_repr_accessors(&mut p);
        hale_syntax::desugar::desugar_intra_locus_topics(&mut p);
        hale_syntax::desugar::desugar_omitted_run(&mut p);
        total += number_and_check(&prog.origin, &mut p);
    }
    eprintln!("{} desugared programs, {} sites", programs.len(), total);
}
