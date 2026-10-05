//! The lowering view's correspondence (F.40 phase 3, C5): every site of
//! the merged program lowering walks is a site of the checked programs
//! or of the stdlib's analysis copy, and the law holds over every program
//! the repo lowers.
//!
//! The merged program is the checked program after the intra-locus and
//! topic rewrites, with the bundled stdlib appended and minted again
//! (`hale_types::resolved`). The correspondence
//! (`hale_types::correspondence`) places each merged site:
//!
//! - `Checked`: the checked site at the same index, of the same kind and
//!   span (the merged mint keeps every id it finds);
//! - `RewrittenSend`: the call the intra-locus rewrite put in a send's
//!   place, which keeps the send's id (`IntraLocusRewrite::send`);
//! - `Stdlib`: the analysis copy's site in the same walk position.
//!
//! The holes, each explained. One merged site has no checked image: the
//! `std::api::local_context()` call the intra-locus rewrite passes to a
//! handler that takes a `std::api::Context` (GH #1108), the one site a
//! rewrite generates, placed as `Generated` by its relation
//! (`IntraLocusRewrite::context`); no corpus program has the shape, and
//! `tests/hale/api_context_test.hl`'s single-file program does. And a
//! checked site no merged site is, which the rewrites erased and their
//! relations record. There is one kind, the subject of a send, an
//! identifier expression (`Use`):
//!
//! - `TopicSubject`: the topic rewrite replaced it with its wire literal,
//!   which is no site (`TopicRewrite::erased`);
//! - `IntraLocusSubject`: the intra-locus rewrite dropped it with the
//!   send, since the direct call names no topic
//!   (`IntraLocusRewrite::erased`).
//!
//! Beside the context call, the rewrites generate no site, so no other
//! merged site is without an image.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot, Target};
use hale_frontend::source::Disk;
use hale_syntax::ast::NodeId;
use hale_types::correspondence::{Erased, Image};
use hale_types::resolved::LoweringView;
use hale_types::snapshot::STDLIB_SEED;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// The corpus fixtures, the `tests/hale` programs and the DNA seeds.
fn targets() -> Vec<PathBuf> {
    let root = root();
    let mut out = Vec::new();
    let mut push_dir = |dir: &str, keep: &dyn Fn(&Path) -> bool| {
        for e in std::fs::read_dir(root.join(dir)).unwrap() {
            let p = e.unwrap().path();
            if keep(&p) {
                out.push(p);
            }
        }
    };
    push_dir("crates/hale-codegen/tests/fixtures/examples", &|p| p.is_dir());
    push_dir("tests/hale", &|p| p.to_string_lossy().ends_with("_test.hl"));
    push_dir("dna", &|p| p.is_dir() && p.join("main.hl").exists());
    out.sort();
    out
}

fn on_big_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|s| std::thread::Builder::new().stack_size(256 << 20).spawn_scoped(s, f).unwrap().join().unwrap())
}

/// What one view's correspondence holds, by kind: the law already held,
/// or the view would have been refused.
#[derive(Debug, Default, PartialEq, Eq)]
struct Tally {
    checked: usize,
    rewritten_sends: usize,
    stdlib: usize,
    generated: usize,
    topic_subjects: usize,
    intra_locus_subjects: usize,
}

/// The view's tally, with the law's totals checked against the two
/// mints: every merged site has its image, every checked site is an
/// image or erased, and every analysis-copy site is paired.
fn tally(user: &hale_types::snapshot::Snapshot, view: &LoweringView, what: &str) -> Tally {
    let mut t = Tally::default();
    for (index, image) in view.correspondence.images() {
        let site = view.snapshot.site(view.snapshot.site_id(NodeId(index)).unwrap()).unwrap();
        match image {
            Image::Checked(id) => {
                let own = user.site(id).unwrap();
                assert_eq!((own.kind, own.span), (site.kind, site.span), "{what}: site {index}");
                t.checked += 1;
            }
            Image::RewrittenSend(id) => {
                assert!(view.intra_locus.iter().any(|r| r.send.0 == id.index), "{what}: site {index} is no rewritten send");
                t.rewritten_sends += 1;
            }
            Image::Stdlib(_) => {
                assert_eq!(view.snapshot.seeds[site.id.seed.0 as usize], STDLIB_SEED, "{what}: site {index}");
                t.stdlib += 1;
            }
            Image::Generated { send } => {
                assert_eq!(site.kind, hale_syntax::sites::SiteKind::Call, "{what}: site {index}");
                assert!(
                    view.intra_locus.iter().any(|r| r.context && r.send.0 == send.index),
                    "{what}: site {index} is no context call of a rewritten send"
                );
                t.generated += 1;
            }
        }
    }
    for (_, erased) in view.correspondence.erased() {
        match erased {
            Erased::TopicSubject { .. } => t.topic_subjects += 1,
            Erased::IntraLocusSubject { .. } => t.intra_locus_subjects += 1,
        }
    }
    assert_eq!(t.checked + t.rewritten_sends + t.stdlib + t.generated, view.snapshot.len(), "{what}: a merged site has no image");
    let contexts = view.intra_locus.iter().filter(|r| r.context && !r.send.is_none()).count();
    assert_eq!(t.generated, contexts, "{what}: each context rewrite generated its call");
    assert_eq!(t.checked + t.rewritten_sends + t.topic_subjects + t.intra_locus_subjects, user.len(), "{what}: a checked site is lost");
    let analysis = hale_types::stdlib_bodies::identities().expect("the stdlib parses");
    assert_eq!(t.stdlib, analysis.len(), "{what}: an analysis-copy site has no merged site");
    let sends = view.intra_locus.iter().filter(|r| !r.send.is_none()).count();
    assert_eq!(t.rewritten_sends, sends, "{what}: the rewritten sends are the relation's");
    assert_eq!(t.intra_locus_subjects, sends, "{what}: each rewritten send erased its subject");
    let topic_sends = view.topic_rewrites.iter().filter(|r| !r.erased.is_none()).count();
    assert_eq!(t.topic_subjects, topic_sends, "{what}: each rewritten send subject is the relation's");
    t
}

const BOTH_REWRITES: &str = r#"
    type Ping { n: Int = 0; }
    topic PingT { payload: Ping; subject: "p.ping"; }
    topic LogT { payload: Ping; subject: "p.log"; }

    locus Worker {
        bus { subscribe PingT as on_ping; }
        fn on_ping(p: Ping) { println("ping ", p.n); }
    }

    locus Sink {
        bus { subscribe LogT as on_log; }
        fn on_log(p: Ping) { println("log ", p.n); }
    }

    main locus App {
        params { w: Worker = Worker { }; }
        bus { publish PingT; publish LogT; }
        run() {
            PingT <- Ping { n: 1 };
            LogT <- Ping { n: 2 };
        }
    }

    fn main() { Sink { }; App { }; }
"#;

/// Each image and each hole, in one program: the send to the field's
/// subscriber is rewritten into a direct call (its call keeps the send's
/// id, its subject is erased), the send to the other locus keeps its
/// site and loses its subject to the topic rewrite, and the stdlib's
/// sites are the analysis copy's.
#[test]
fn each_image_and_each_hole_has_its_rewrite() {
    let program = hale_syntax::parse_source(BOTH_REWRITES).expect("parse");
    on_big_stack(move || {
        let snap = Snapshot::from_program(program, Default::default(), Config::harness(Target::host()))
            .unwrap_or_else(|_| panic!("loads"));
        let view = snap.demand_lowering().unwrap_or_else(|b| panic!("lowers: {:?} {:?}", b.refused, b.because));
        let t = tally(snap.identities(), view, "both rewrites");
        assert_eq!((t.rewritten_sends, t.intra_locus_subjects, t.topic_subjects, t.generated), (1, 1, 1, 0), "{t:?}");
        assert!(t.checked > 0 && t.stdlib > 0, "{t:?}");
        // The rewritten send's call is the checked send, and the send it
        // replaced named the field's subscriber.
        let rw = &view.intra_locus[0];
        let Some(Image::RewrittenSend(send)) = view.correspondence.image(rw.send) else { panic!("the call has its image") };
        assert_eq!(view.correspondence.checked(rw.send), Some(send));
        assert_eq!(snap.identities().site(send).map(|s| s.kind), Some(hale_syntax::sites::SiteKind::Send));
    });
}

const CONTEXT_HANDLER: &str = r#"
    type Ping { n: Int; }
    topic Pings { payload: Ping; subject: "solo.ping"; }
    locus Solo {
        bus { subscribe Pings as on_ping; publish Pings; }
        fn on_ping(p: Ping, ctx: std::api::Context) { println(ctx.via + " " + to_string(p.n)); }
        run() { Pings <- Ping { n: 3 }; }
    }
    fn main() { Solo { }; }
"#;

/// The one site a rewrite generates: a self-publish to a handler that
/// takes a `std::api::Context` becomes a direct call passing
/// `std::api::local_context()` (GH #1108), a call no checked site is,
/// placed by its relation as `Generated`, at its send's span.
#[test]
fn the_context_call_is_the_rewrites_own_site() {
    let program = hale_syntax::parse_source(CONTEXT_HANDLER).expect("parse");
    on_big_stack(move || {
        let snap = Snapshot::from_program(program, Default::default(), Config::harness(Target::host()))
            .unwrap_or_else(|_| panic!("loads"));
        let view = snap.demand_lowering().unwrap_or_else(|b| panic!("lowers: {:?} {:?}", b.refused, b.because));
        let t = tally(snap.identities(), view, "a context handler");
        assert_eq!((t.rewritten_sends, t.intra_locus_subjects, t.generated, t.topic_subjects), (1, 1, 1, 0), "{t:?}");
        let rw = &view.intra_locus[0];
        assert!(rw.context, "the handler takes a context");
        let call = view.snapshot.site(view.snapshot.site_id(rw.send).unwrap()).unwrap().span;
        let generated: Vec<_> = view
            .correspondence
            .images()
            .filter(|(_, image)| matches!(image, Image::Generated { .. }))
            .map(|(index, _)| view.snapshot.site(view.snapshot.site_id(NodeId(index)).unwrap()).unwrap().span)
            .collect();
        assert_eq!(generated, vec![call], "the context call sits at its send's span");
    });
}

/// The law over every program the repo lowers: the corpus fixtures, the
/// `tests/hale` programs and the DNA seeds, each as `hale build` loads it,
/// and as the test harness does where the build's check refuses it.
#[test]
fn every_lowered_program_corresponds() {
    let mut totals = Tally::default();
    let mut lowered = 0;
    let mut by_target: BTreeMap<String, Tally> = BTreeMap::new();
    for t in targets() {
        let name = t.strip_prefix(root()).unwrap().display().to_string();
        let tally = on_big_stack(|| {
            for config in [Config::build(Target::host()), Config::harness(Target::host())] {
                let Ok(snap) = Snapshot::load(&t, LoadMode::WholeSeed, &Disk, config) else { return None };
                match snap.demand_lowering() {
                    Ok(view) => return Some(tally(snap.identities(), view, &name)),
                    // The law refuses the view by name: never a blocked view.
                    Err(b) => assert!(
                        !b.refused.as_deref().is_some_and(|m| m.contains("merged")),
                        "{name}: {:?}",
                        b.refused
                    ),
                }
            }
            None
        });
        if let Some(tally) = tally {
            lowered += 1;
            totals.checked += tally.checked;
            totals.rewritten_sends += tally.rewritten_sends;
            totals.stdlib += tally.stdlib;
            totals.generated += tally.generated;
            totals.topic_subjects += tally.topic_subjects;
            totals.intra_locus_subjects += tally.intra_locus_subjects;
            by_target.insert(name, tally);
        }
    }
    eprintln!("{lowered} programs lowered: {totals:?}");
    assert!(lowered > 150, "the targets lower: {lowered}");
    assert!(totals.rewritten_sends > 0 && totals.topic_subjects > 0, "the holes occur: {totals:?}");
}
