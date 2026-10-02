//! The reclaim boundary is relative to the loop analyzed (F.40 phase 3,
//! E3b, a classified correction; GH #1208).
//!
//! The checker's reclaim model said `reclaim@locus-dissolve` for every
//! allocation that is not sent, while lowering gives a scratch-local
//! free fn (`alloc_routing`'s `scratch_local` row, GH #1148) its own
//! subregion, freed at return. The summary now reads that row: the
//! same classifier over the declarations it holds, so the row
//! (`FnSummary::frame`) and lowering never disagree. A `Local`
//! allocation in a scratch-local fn that is not recursive reclaims at
//! `reclaim@fn-return`, and the boundary is judged relative to the loop:
//! the fn's return falls inside each iteration of a caller's loop and
//! outside every iteration of the fn's own, so a function return is not
//! an iteration's reclamation. Such a fn is a frame with a boundary of
//! its own, like a method's per-call scratch.
//!
//! `Local` is not scratch. A `Local` allocation of a free fn that is not
//! scratch-local lands in its caller's arena; run once per iteration of
//! an unbounded loop in a long-lived frame (`main`, `run`, or a free fn
//! they call), it accumulates there, and the model never said so
//! (`LeakReason::InCallersArena`). A value that escapes the fn (its
//! return value, a store to `self`, a bus payload) keeps its boundary,
//! a recursive fn and a fn whose call leaves the class (an unresolved
//! callee) keep the locus boundary, and a locus instantiation is not a
//! value in the caller's arena (its instance's arena is its own).
//!
//! The old answer is the corrected summary with every fn's frame cleared
//! (`FnSummary::frame`, `None`: not classified) and every
//! `reclaim@fn-return` put back to `reclaim@locus-dissolve`: the verdict
//! reads nothing else that moved. Measured against the base build's
//! `--dump-alloc-summary` on every corpus program, `tests/hale` program,
//! DNA seed and stdlib file whose dump moved, its dump is the base's,
//! line for line.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;
use hale_types::alloc_summary::{summarize_identified, AllocSummary, FnKey, Frame, LeakSite, LoopAt, ReclaimScope};

/// The summary of `src` alone, minted as `app.hl`.
fn summarize(src: &str) -> AllocSummary {
    let mut program = hale_syntax::parse_source(src).expect("parse");
    let ids = hale_types::snapshot::mint([("app.hl", &mut program)], &[]);
    summarize_identified(&[(&program, &ids)], &[])
}

/// The model before the correction, from the corrected summary.
fn old_model(now: &AllocSummary) -> AllocSummary {
    let mut old = now.clone();
    for f in old.fns.values_mut() {
        f.frame = None;
        for s in &mut f.sites {
            if s.reclaim == ReclaimScope::FnReturn {
                s.reclaim = ReclaimScope::EnclosingLocus;
            }
        }
    }
    old
}

fn site(l: &LeakSite) -> String {
    format!("{} {:?} @{}..{} {:?}", l.owner.display(), l.kind, l.span.start.0, l.span.end.0, l.reason)
}

/// The leak sites of the program's own fns, now and before.
fn leaks(now: &AllocSummary) -> (Vec<String>, Vec<String>) {
    let own = |s: &AllocSummary, l: &LeakSite| s.is_own(&l.owner);
    let is: Vec<String> = now.leak_sites().iter().filter(|l| own(now, l)).map(site).collect();
    let old = old_model(now);
    let was: Vec<String> = old.leak_sites().iter().filter(|l| own(&old, l)).map(site).collect();
    assert!(!was.iter().any(|s| s.ends_with("InCallersArena")), "the old model has no caller-arena rule");
    (was, is)
}

/// The dump's lines of one fn, from its `fn` line to the next.
fn fn_block(dump: &str, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for l in dump.lines() {
        if let Some(rest) = l.strip_prefix("fn ") {
            inside = rest.split_whitespace().next() == Some(name);
        }
        if inside && !l.is_empty() {
            out.push(l.split_whitespace().collect::<Vec<_>>().join(" "));
        }
    }
    out
}

#[test]
fn the_boundary_is_relative_to_the_loop() {
    assert!(ReclaimScope::FnReturn.accumulates_in_loop(LoopAt::OwnBody), "a fn's return ends no iteration of its own loop");
    assert!(!ReclaimScope::FnReturn.accumulates_in_loop(LoopAt::Caller), "a fn's return ends each iteration of a caller's loop");
    for at in [LoopAt::OwnBody, LoopAt::Caller] {
        assert!(ReclaimScope::EnclosingLocus.accumulates_in_loop(at));
        assert!(!ReclaimScope::AfterBusDispatch.accumulates_in_loop(at));
    }
}

/// Both loop shapes a caller writes — a `for` over a collection and a
/// `while` with a counter — around a call: a scratch-local fn's local
/// allocation is reclaimed at its return, inside the iteration (no
/// verdict moves, its column does); a caller-arena fn's lands in `run`'s
/// arena and accumulates (it appears).
#[test]
fn both_caller_loop_shapes() {
    let src = r#"
        type Row { text: String; }
        @form(vec) locus Rows { capacity { heap items of Row; } }
        fn shout(s: String) -> Int { let t = s + "!"; return len(t); }
        fn shout_row(r: Row) -> Int { let t = r.text + "!"; return len(t); }
        locus App {
            params { rows: Rows = Rows { }; n: Int = 3; total: Int = 0; }
            run() {
                for r in self.rows { self.total = self.total + shout(r.text) + shout_row(r); }
                let mut i = 0;
                while i < self.n { self.total = self.total + shout("x") + shout_row(Row { text: "y" }); i = i + 1; }
            }
        }
        fn main() { App { }; }
    "#;
    let now = summarize(src);
    let dump = now.render();
    assert_eq!(
        fn_block(&dump, "shout"),
        [
            "fn shout [invoked-unboundedly, scratch-local]",
            "alloc string-concat local once-per-invocation reclaim@fn-return @147..154",
            "call <unresolved: len> loop_depth=0 result=escaping=return",
        ]
    );
    assert_eq!(
        fn_block(&dump, "shout_row"),
        [
            "fn shout_row [invoked-unboundedly]",
            "alloc string-concat local ACCUMULATES-UNBOUNDED reclaim@locus-dissolve @219..231 <-- LEAK",
            "call <unresolved: len> loop_depth=0 result=escaping=return",
        ]
    );
    let (was, is) = leaks(&now);
    let appear: Vec<&String> = is.iter().filter(|s| !was.contains(s)).collect();
    assert_eq!(appear, ["shout_row StringConcat @219..231 InCallersArena"]);
    assert!(was.iter().all(|s| is.contains(s)), "nothing goes: {was:?} -> {is:?}");
    // Each shape alone repeats the call.
    for (shape, body) in [
        ("for", "for r in self.rows { self.total = self.total + shout_row(r); }"),
        ("while", "let mut i = 0; while i < self.n { self.total = self.total + shout_row(Row { text: \"y\" }); i = i + 1; }"),
    ] {
        let src = format!(
            r#"
            type Row {{ text: String; }}
            @form(vec) locus Rows {{ capacity {{ heap items of Row; }} }}
            fn shout_row(r: Row) -> Int {{ let t = r.text + "!"; return len(t); }}
            locus App {{ params {{ rows: Rows = Rows {{ }}; n: Int = 3; total: Int = 0; }} run() {{ {body} }} }}
            fn main() {{ App {{ }}; }}
            "#
        );
        let (_, is) = leaks(&summarize(&src));
        assert!(is.iter().any(|s| s.starts_with("shout_row StringConcat") && s.ends_with("InCallersArena")), "{shape}: {is:?}");
    }
    // A loop whose trip count is a constant bounds the accumulation.
    let bounded = r#"
        type Row { text: String; }
        fn shout_row(r: Row) -> Int { let t = r.text + "!"; return len(t); }
        locus App { params { total: Int = 0; } run() { let mut i = 0; while i < 100 { self.total = self.total + shout_row(Row { text: "y" }); i = i + 1; } } }
        fn main() { App { }; }
    "#;
    let (_, is) = leaks(&summarize(bounded));
    assert!(!is.iter().any(|s| s.starts_with("shout_row StringConcat")), "{is:?}");
}

/// The fn's own loop: a scratch-local fn's in-loop local accumulates
/// until the fn returns, bounded by the activation, as a method's is
/// (it goes); the value it returns escapes the fn and keeps its verdict.
#[test]
fn the_fns_own_loop() {
    let src = r#"
        fn pad(n: Int) -> String {
            let mut s = "";
            let mut i = 0;
            while i < n { s = s + "ab" + to_string(i); i = i + 1; }
            return s;
        }
        locus App { params { n: Int = 3; } run() { let x = pad(self.n); println(x); } }
        fn main() { App { }; }
    "#;
    let now = summarize(src);
    assert_eq!(
        fn_block(&now.render(), "pad"),
        [
            "fn pad [scratch-local]",
            "loop while depth=0 @103..158",
            "alloc string-concat escaping=return ACCUMULATES-UNBOUNDED reclaim@locus-dissolve @121..144 <-- LEAK",
            "alloc string-concat local per-iteration-reclaim reclaim@fn-return @121..129",
            "call <unresolved: to_string> loop_depth=1 result=local",
        ]
    );
    let (was, is) = leaks(&now);
    assert_eq!(was, ["pad StringConcat @121..144 InUnboundedLoop", "pad StringConcat @121..129 InUnboundedLoop"]);
    assert_eq!(is, ["pad StringConcat @121..144 InUnboundedLoop"]);
}

/// Escaping returns: a scratch-local fn's return value is deep-copied
/// into its caller's arena and keeps its verdict; consumed inside another
/// scratch-local fn it dies with that fn's subregion (it goes).
#[test]
fn escaping_returns() {
    let src = r#"
        fn tag(s: String) -> String { return "<" + s + ">"; }
        fn wrap(s: String) -> Int { let t = tag(s); return len(t); }
        locus App {
            params { n: Int = 3; total: Int = 0; last: String = ""; }
            run() {
                let mut i = 0;
                while i < self.n { self.total = self.total + wrap("x"); self.last = tag("y"); i = i + 1; }
            }
        }
        fn main() { App { }; }
    "#;
    let now = summarize(src);
    let (was, is) = leaks(&now);
    // `tag` is called from `run` (its value lands in run's arena) and
    // from `wrap` (in wrap's subregion): `run` alone still accumulates
    // it, so it stays.
    assert_eq!(was, is);
    assert!(is.iter().any(|s| s.starts_with("tag StringConcat") && s.ends_with("InvokedUnboundedly")), "{is:?}");
    // Consumed only inside `wrap`, it goes.
    let inner = src.replace("self.last = tag(\"y\"); ", "");
    let now = summarize(&inner);
    let (was, is) = leaks(&now);
    assert!(was.iter().any(|s| s.starts_with("tag StringConcat") && s.ends_with("InvokedUnboundedly")), "{was:?}");
    assert!(!is.iter().any(|s| s.starts_with("tag ")), "{is:?}");
    // The local temporary inside the return value's concat is `wrap`'s
    // subregion's either way.
    assert!(fn_block(&now.render(), "tag").iter().any(|l| l.contains("local") && l.contains("reclaim@fn-return")));
}

/// A store to `self` and a bus payload keep their boundaries: neither
/// can be written in a scratch-local fn, and the method or hook they are
/// written in keeps its verdict.
#[test]
fn stores_and_bus_payloads() {
    let src = r#"
        type Msg { text: String; }
        locus App {
            params { n: Int = 3; last: String = ""; }
            fn keep(s: String) { self.last = s + "!"; }
            bus { publish "out" of type Msg; }
            run() {
                let mut i = 0;
                while i < self.n { self.keep("x"); "out" <- Msg { text: "m" + to_string(i) }; i = i + 1; }
            }
        }
        fn main() { App { }; }
    "#;
    let now = summarize(src);
    let dump = now.render();
    assert_eq!(
        fn_block(&dump, "App::keep"),
        [
            "fn App::keep [invoked-unboundedly]",
            "alloc string-concat escaping=self-store per-iteration-reclaim reclaim@locus-dissolve @155..162",
        ]
    );
    assert_eq!(dump, old_model(&now).render(), "nothing here moves");
    let run = fn_block(&dump, "App::run");
    assert!(run.iter().any(|l| l.contains("escaping=bus-send") && l.contains("per-iteration-reclaim") && l.contains("reclaim@bus-dispatch")), "{run:?}");
    assert!(
        !now.fns.values().any(|f| matches!(f.frame, Some(Frame::ScratchLocal { .. }))),
        "no fn here is scratch-local"
    );
    let (was, is) = leaks(&now);
    assert_eq!(was, is);
}

/// A recursive scratch-local fn and one whose call leaves the class (an
/// unresolved callee) keep the old answer: the hole's conservative one.
#[test]
fn recursive_and_unresolved_callees() {
    let src = r#"
        fn rep(n: Int) -> String {
            if n <= 0 { return ""; }
            let mut s = "";
            let mut i = 0;
            while i < n { s = s + "x" + to_string(i); i = i + 1; }
            return s + rep(n - 1);
        }
        fn stamp(n: Int) -> String {
            let mut s = "";
            let mut i = 0;
            while i < n { s = s + "x" + (std::io::fs::read_file("/etc/hostname") or ""); i = i + 1; }
            return s;
        }
        locus App { params { n: Int = 3; } run() { println(rep(self.n) + stamp(self.n)); } }
        fn main() { App { }; }
    "#;
    let now = summarize(src);
    let rep = &now.fns[&FnKey::free_fn("rep")];
    assert_eq!(rep.frame, Some(Frame::ScratchLocal { recursive: true }));
    assert!(!rep.frees_at_return());
    let stamp = &now.fns[&FnKey::free_fn("stamp")];
    assert_eq!(stamp.frame, Some(Frame::CallersArena), "a call outside the class demotes the fn, as lowering's row does");
    let dump = now.render();
    assert_eq!(fn_block(&dump, "rep")[0], "fn rep [scratch-local, recursive]");
    for f in ["rep", "stamp"] {
        let block = fn_block(&dump, f);
        assert!(!block.iter().any(|l| l.contains("reclaim@fn-return")), "{f}: {block:?}");
        assert!(block.iter().any(|l| l.contains(" local ") && l.ends_with("<-- LEAK")), "{f}: {block:?}");
    }
    let (was, is) = leaks(&now);
    assert_eq!(was, is);
}

/// A locus instantiated in a caller-arena fn run once per iteration is
/// not a value in the caller's arena: its instance's arena is its own,
/// and the fn's dissolve frame destroys it (`std::io::tcp`'s
/// `__handle_one_connection` is the shape). A value literal beside it is.
#[test]
fn a_locus_instance_is_not_in_the_callers_arena() {
    let src = r#"
        type Row { text: String; }
        locus Conn { params { fd: Int = 0; } }
        fn handle(fd: Int, r: Row) -> Int { let c = Conn { fd: fd }; let p = Row { text: r.text }; return len(p.text); }
        locus App { params { n: Int = 3; t: Int = 0; } run() { let mut i = 0; while i < self.n { self.t = self.t + handle(i, Row { text: "r" }); i = i + 1; } } }
        fn main() { App { }; }
    "#;
    let (_, is) = leaks(&summarize(src));
    assert!(!is.iter().any(|s| s.contains("StructLit(\"Conn\")")), "{is:?}");
    assert!(is.iter().any(|s| s.starts_with("handle StructLit(\"Row\")") && s.ends_with("InCallersArena")), "{is:?}");
}

/// The advisory says where a caller-arena allocation goes.
#[test]
fn the_callers_arena_wording() {
    let src = r#"
        type Row { text: String; }
        fn shout_row(r: Row) -> Int { let t = r.text + "!"; return len(t); }
        locus App { params { n: Int = 3; t: Int = 0; } run() { let mut i = 0; while i < self.n { self.t = self.t + shout_row(Row { text: "y" }); i = i + 1; } } }
        fn main() { App { }; }
    "#;
    let mut program = hale_syntax::parse_source(src).expect("parse");
    let ids = hale_types::snapshot::mint([("app.hl", &mut program)], &[]);
    let summary = summarize_identified(&[(&program, &ids)], &[]);
    let diags = hale_types::alloc_summary::unbounded_alloc_diags(&summary, &[&program], &ids, &[], true);
    let msgs: Vec<&str> = diags.iter().map(|d| d.message.as_str()).filter(|m| m.contains("shout_row")).collect();
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    assert!(
        msgs[0].starts_with(
            "unbounded allocation: this string-concat in `shout_row` lands in its caller's arena — \
             `shout_row` has no scratch of its own, so its return does not reclaim it — and \
             `shout_row` runs once per iteration of an unbounded loop in a long-lived frame"
        ),
        "{}",
        msgs[0]
    );
}

// --- per target -----------------------------------------------------------

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// The corpus fixtures, the `tests/hale` programs and the DNA seeds.
fn targets() -> Vec<String> {
    let root = root();
    let mut out = Vec::new();
    let mut push_dir = |dir: &str, keep: &dyn Fn(&Path) -> bool| {
        for e in std::fs::read_dir(root.join(dir)).unwrap() {
            let p = e.unwrap().path();
            if keep(&p) {
                out.push(p.strip_prefix(&root).unwrap().to_string_lossy().to_string());
            }
        }
    };
    push_dir("crates/hale-codegen/tests/fixtures/examples", &|p| p.is_dir());
    push_dir("tests/hale", &|p| p.to_string_lossy().ends_with("_test.hl"));
    push_dir("dna", &|p| {
        p.is_dir()
            && !p.ends_with("tests")
            && std::fs::read_dir(p).unwrap().any(|f| f.unwrap().path().extension().is_some_and(|x| x == "hl"))
    });
    out.sort();
    out
}

fn over_target<T: Send>(target: &str, f: impl FnOnce(&Snapshot, &AllocSummary) -> T + Send) -> Option<T> {
    let path = root().join(target);
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn_scoped(s, move || {
                let config = Config::check(path.is_dir(), false);
                let snap = Snapshot::load(&path, LoadMode::WholeSeed, &Disk, config).ok()?;
                let summary = snap.demand_alloc_summary().ok()?;
                Some(f(&snap, summary))
            })
            .unwrap()
            .join()
            .unwrap()
    })
}

/// The summary's frame is lowering's row: over every target, the free
/// fns whose frame is scratch-local are the summary's free fns the
/// lowering view's `alloc_routing` calls scratch-local (`main` aside,
/// which lowering never lowers as a free fn).
#[test]
fn the_frame_is_lowerings_row() {
    let mut marked = 0;
    for t in targets() {
        let Some((column, row)) = over_target(&t, |snap, now| {
            let view = snap.demand_lowering().ok()?;
            let column: BTreeSet<String> = now
                .fns
                .values()
                .filter(|f| matches!(f.frame, Some(Frame::ScratchLocal { .. })))
                .map(|f| f.key.fn_name.clone())
                .collect();
            let row: BTreeSet<String> = now
                .fns
                .keys()
                .filter(|k| k.locus.is_none() && k.fn_name != "main" && view.alloc_routing.is_scratch_local(&k.fn_name))
                .map(|k| k.fn_name.clone())
                .collect();
            Some((column, row))
        })
        .flatten() else {
            continue;
        };
        assert_eq!(column, row, "{t}: the column and lowering's row disagree");
        marked += column.len();
    }
    assert!(marked > 0, "the targets hold scratch-local fns");
}

/// Per target, what the correction moves in the program's own fns: the
/// leak sites that go (each a site of a fn that frees at return, or a
/// value returned into one), the ones that appear (each in a caller's
/// arena), and the dump's lines that move: a fn's tag (`scratch-local`),
/// a site's reclaim column alone (`reclaim@fn-return`), a site's verdict.
#[derive(Debug, Default, PartialEq)]
struct Moved {
    gone: usize,
    appear: usize,
    tags: usize,
    reclaim: usize,
    verdicts: usize,
}

/// The dump's moved lines, by class (tags, reclaim column, verdict).
fn dump_moves(old: &str, now: &str) -> (usize, usize, usize) {
    assert_eq!(old.lines().count(), now.lines().count(), "the dump keeps its shape");
    let norm = |s: &str| {
        s.replace("reclaim@locus-dissolve", "R").replace("reclaim@fn-return", "R").split_whitespace().collect::<Vec<_>>().join(" ")
    };
    let (mut tags, mut reclaim, mut verdicts) = (0, 0, 0);
    for (o, n) in old.lines().zip(now.lines()).filter(|(o, n)| o != n) {
        if o.starts_with("fn ") {
            assert!(n.contains("scratch-local"), "a fn line moves only by its frame: {o} -> {n}");
            tags += 1;
        } else if o.trim_start().starts_with("alloc ") && norm(o) == norm(n) {
            assert!(n.contains("reclaim@fn-return") && o.contains("reclaim@locus-dissolve"), "{o} -> {n}");
            reclaim += 1;
        } else {
            assert!(o.trim_start().starts_with("alloc "), "only a fn's tag and its sites move: {o} -> {n}");
            verdicts += 1;
        }
    }
    (tags, reclaim, verdicts)
}

fn moved(target: &str) -> Option<Moved> {
    over_target(target, |_, now| {
        let (tags, reclaim, verdicts) = dump_moves(&old_model(now).render(), &now.render());
        let (was, is) = leaks(now);
        let gone: Vec<&String> = was.iter().filter(|s| !is.contains(s)).collect();
        let appear: Vec<&String> = is.iter().filter(|s| !was.contains(s)).collect();
        for s in &appear {
            assert!(s.ends_with("InCallersArena"), "{target}: {s} appears for another reason");
        }
        let frees: BTreeSet<String> = now.fns.values().filter(|f| f.frees_at_return()).map(|f| f.key.display()).collect();
        let callers: BTreeMap<String, BTreeSet<String>> = now.fns.values().fold(BTreeMap::new(), |mut m, f| {
            for c in &f.calls {
                if let hale_types::alloc_summary::Callee::Resolved(k) = &c.callee {
                    m.entry(k.display()).or_default().insert(f.key.display());
                }
            }
            m
        });
        for s in &gone {
            let owner = s.split_whitespace().next().unwrap();
            assert!(
                frees.contains(owner) || callers.get(owner).is_some_and(|cs| cs.iter().any(|c| frees.contains(c))),
                "{target}: {s} goes, and neither it nor a caller frees at return"
            );
        }
        Moved { gone: gone.len(), appear: appear.len(), tags, reclaim, verdicts }
    })
}

/// The targets whose leak sites or dump move, and how; every other
/// target's stay where they were.
#[test]
fn per_target_moves() {
    let mut moves = Vec::new();
    for t in targets() {
        let Some(m) = moved(&t) else { continue };
        if m != Moved::default() {
            moves.push(format!("{t}: {} go, {} appear; dump {} tags, {} reclaim, {} verdicts", m.gone, m.appear, m.tags, m.reclaim, m.verdicts));
        }
    }
    assert_eq!(moves, PINNED);
}

/// `dna/api` and `api_binding_run_test` lose the stdlib api helpers'
/// `__api_json_str` local and `__api_http_header` return, consumed in a
/// scratch-local fn; `factory_field_owner_test` loses `churn`'s in-loop
/// temporary; `dna/host` loses three of its own fns' (two in
/// `magnitude_facets`), and gains the caller-arena sites of its CLI verbs
/// and of `Host::run`'s tick (`exit_code` → `alive`): of those with an
/// author position, the advisory's, 133 appear and 2 go. Every other
/// target moves its dump's tags and reclaim columns alone.
const PINNED: &[&str] = &[
    "crates/hale-codegen/tests/fixtures/examples/09-functions: 0 go, 0 appear; dump 3 tags, 0 reclaim, 0 verdicts",
    "crates/hale-codegen/tests/fixtures/examples/23-ranges: 0 go, 0 appear; dump 2 tags, 0 reclaim, 0 verdicts",
    "crates/hale-codegen/tests/fixtures/examples/24-default-params: 0 go, 0 appear; dump 2 tags, 0 reclaim, 0 verdicts",
    "crates/hale-codegen/tests/fixtures/examples/27-strings: 0 go, 0 appear; dump 2 tags, 1 reclaim, 0 verdicts",
    "crates/hale-codegen/tests/fixtures/examples/28-to-string: 0 go, 0 appear; dump 2 tags, 5 reclaim, 0 verdicts",
    "crates/hale-codegen/tests/fixtures/examples/29-helpers: 0 go, 0 appear; dump 1 tags, 0 reclaim, 0 verdicts",
    "crates/hale-codegen/tests/fixtures/examples/46-fn-arenas: 0 go, 0 appear; dump 1 tags, 3 reclaim, 0 verdicts",
    "crates/hale-codegen/tests/fixtures/examples/65-hashmap-anchor-churn: 0 go, 0 appear; dump 1 tags, 0 reclaim, 0 verdicts",
    "crates/hale-codegen/tests/fixtures/examples/74-effect-contracts: 0 go, 0 appear; dump 7 tags, 0 reclaim, 0 verdicts",
    "crates/hale-codegen/tests/fixtures/examples/90-unowned-literal-positions: 0 go, 0 appear; dump 1 tags, 2 reclaim, 0 verdicts",
    "crates/hale-codegen/tests/fixtures/examples/91-module-decls: 0 go, 0 appear; dump 1 tags, 0 reclaim, 0 verdicts",
    "crates/hale-codegen/tests/fixtures/examples/docs-server: 0 go, 0 appear; dump 4 tags, 9 reclaim, 0 verdicts",
    "crates/hale-codegen/tests/fixtures/examples/multi-file-seed: 0 go, 0 appear; dump 2 tags, 1 reclaim, 0 verdicts",
    "dna/api: 2 go, 0 appear; dump 312 tags, 633 reclaim, 2 verdicts",
    "dna/core: 0 go, 0 appear; dump 253 tags, 560 reclaim, 0 verdicts",
    "dna/host: 3 go, 180 appear; dump 377 tags, 628 reclaim, 516 verdicts",
    "dna/oidc: 0 go, 0 appear; dump 1 tags, 0 reclaim, 0 verdicts",
    "dna/operations: 0 go, 0 appear; dump 298 tags, 621 reclaim, 0 verdicts",
    "dna/organism: 0 go, 0 appear; dump 264 tags, 578 reclaim, 0 verdicts",
    "dna/organization_runtime: 0 go, 0 appear; dump 298 tags, 621 reclaim, 0 verdicts",
    "dna/organization_source: 0 go, 0 appear; dump 253 tags, 560 reclaim, 0 verdicts",
    "dna/reflexes: 0 go, 0 appear; dump 14 tags, 19 reclaim, 0 verdicts",
    "dna/ui: 0 go, 0 appear; dump 255 tags, 560 reclaim, 0 verdicts",
    "tests/hale/api_big_reply_test.hl: 0 go, 0 appear; dump 1 tags, 19 reclaim, 0 verdicts",
    "tests/hale/api_binding_run_test.hl: 2 go, 0 appear; dump 7 tags, 10 reclaim, 2 verdicts",
    "tests/hale/api_binding_test.hl: 0 go, 0 appear; dump 2 tags, 36 reclaim, 0 verdicts",
    "tests/hale/api_clients_test.hl: 0 go, 0 appear; dump 2 tags, 29 reclaim, 0 verdicts",
    "tests/hale/api_context_test.hl: 0 go, 0 appear; dump 2 tags, 46 reclaim, 0 verdicts",
    "tests/hale/api_roles_test.hl: 0 go, 0 appear; dump 4 tags, 47 reclaim, 0 verdicts",
    "tests/hale/api_roles_xseed_test.hl: 0 go, 0 appear; dump 2 tags, 41 reclaim, 0 verdicts",
    "tests/hale/api_serve_test.hl: 0 go, 0 appear; dump 3 tags, 29 reclaim, 0 verdicts",
    "tests/hale/block_tail_return_test.hl: 0 go, 0 appear; dump 2 tags, 0 reclaim, 0 verdicts",
    "tests/hale/chains_tranche2_test.hl: 0 go, 0 appear; dump 1 tags, 0 reclaim, 0 verdicts",
    "tests/hale/decorator_stack_test.hl: 0 go, 0 appear; dump 1 tags, 0 reclaim, 0 verdicts",
    "tests/hale/factory_field_owner_test.hl: 1 go, 0 appear; dump 1 tags, 0 reclaim, 1 verdicts",
    "tests/hale/imported_fn_value_test.hl: 0 go, 0 appear; dump 2 tags, 0 reclaim, 0 verdicts",
    "tests/hale/let_block_scope_test.hl: 0 go, 0 appear; dump 5 tags, 0 reclaim, 0 verdicts",
    "tests/hale/sigterm_drains_pinned_test.hl: 0 go, 0 appear; dump 1 tags, 16 reclaim, 0 verdicts",
    "tests/hale/sigterm_no_live_drain_reader_test.hl: 0 go, 0 appear; dump 3 tags, 13 reclaim, 0 verdicts",
];
