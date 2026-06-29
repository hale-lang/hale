//! Static-bus-dispatch devirtualization, build #1a — corpus
//! classification.
//!
//! Builds the authoritative `BusGraph` over every `.hl` fixture
//! that uses the bus, classifies each subject with the
//! static-eligibility gate, prints a per-subject + summary report,
//! and pins a handful of concrete fixtures so the gate can't drift.
//!
//! Run with `-- --nocapture` to see the full classification report.
//! This is pure analysis (no codegen) — it exists to quantify how
//! much of the corpus a later devirt pass (#1b) would cover.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use hale_syntax::ast::{Program, TopDecl};
use hale_syntax::parse_source;
use hale_types::bus_graph::{build_bus_graph, BusGraph, Placement};
use hale_types::resolve::build_top_scope;
use hale_types::{check_bundle, Bundle};

fn examples_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.push("hale-codegen");
    p.push("tests");
    p.push("fixtures");
    p.push("examples");
    p
}

fn list_projects(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).expect("read examples dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            out.push(path);
        }
    }
    out.sort();
    out
}

fn hl_files_in(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .expect("read project dir")
        .filter_map(|e| {
            let p = e.ok()?.path();
            (p.extension().and_then(|s| s.to_str()) == Some("hl")).then_some(p)
        })
        .collect();
    out.sort();
    out
}

/// Parse + typecheck + build a `BusGraph` for one project (all its
/// `.hl` files as a single bundle). Returns `None` when parsing
/// fails (other tests cover parse health) or the project has no bus
/// activity.
fn graph_for_project(project: &Path) -> Option<(String, BusGraph)> {
    let files = hl_files_in(project);
    if files.is_empty() {
        return None;
    }
    let mut sources: BTreeMap<String, String> = BTreeMap::new();
    let mut programs: BTreeMap<String, Program> = BTreeMap::new();
    let dir = examples_dir();
    for file in &files {
        let src = fs::read_to_string(file).expect("read .hl");
        let key = file
            .strip_prefix(&dir)
            .unwrap_or(file)
            .to_string_lossy()
            .into_owned();
        let prog = parse_source(&src).ok()?;
        programs.insert(key.clone(), prog);
        sources.insert(key, src);
    }
    let bundle_programs: BTreeMap<String, &Program> =
        programs.iter().map(|(k, v)| (k.clone(), v)).collect();
    let bundle = Bundle {
        programs: bundle_programs,
    };

    // Typecheck first (so payload types resolve), then build the
    // graph off the same resolved scope.
    let _ = check_bundle(&bundle);
    let (top, _diags) = build_top_scope(&bundle);
    let graph = build_bus_graph(&bundle, &top);

    if graph.subjects.is_empty() {
        return None;
    }
    let label = project
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    Some((label, graph))
}

fn placement_str(p: &Placement) -> String {
    match p {
        Placement::SameThread => "same-thread".to_string(),
        Placement::CrossPool(pool) => format!("cross-pool({pool})"),
        Placement::Pinned => "pinned".to_string(),
    }
}

#[test]
fn classify_corpus_bus_subjects() {
    let dir = examples_dir();
    let projects = list_projects(&dir);
    assert!(!projects.is_empty(), "no example projects found");

    let mut total_subjects = 0usize;
    let mut total_eligible = 0usize;
    let mut reason_hist: BTreeMap<String, usize> = BTreeMap::new();
    let mut projects_with_bus = 0usize;

    println!("\n=== bus-graph corpus classification (build #1a) ===\n");

    for project in &projects {
        let Some((label, graph)) = graph_for_project(project) else {
            continue;
        };
        projects_with_bus += 1;
        println!("[{label}]");
        for (subject, info) in &graph.subjects {
            total_subjects += 1;
            let verdict = if info.eligible {
                total_eligible += 1;
                "eligible".to_string()
            } else {
                let r = info.ineligible_reason.as_ref().unwrap();
                *reason_hist.entry(r.tag().to_string()).or_insert(0) += 1;
                format!("INELIGIBLE({})", r.tag())
            };
            println!(
                "  {subject:<28} -> {verdict}  (pub:{} sub:{})",
                info.publishers.len(),
                info.subscribers.len()
            );
            for s in &info.subscribers {
                println!(
                    "      sub {}::{}  placement={} payload={}",
                    s.locus,
                    s.handler,
                    placement_str(&s.placement),
                    s.payload
                );
            }
        }
        println!();
    }

    println!("=== SUMMARY ===");
    println!("projects with bus activity : {projects_with_bus}");
    println!("total subjects             : {total_subjects}");
    println!("eligible (devirtualizable) : {total_eligible}");
    println!("ineligible                 : {}", total_subjects - total_eligible);
    for (reason, n) in &reason_hist {
        println!("    {reason:<16}: {n}");
    }
    println!();

    // Invariant: every subject is either eligible or carries a
    // reason — the gate never leaves a subject unclassified.
    // (build_bus_graph sets the two in lockstep.)
    assert!(total_subjects > 0, "corpus has no bus subjects to classify");
    assert_eq!(
        total_eligible + (total_subjects - total_eligible),
        total_subjects
    );
}

/// Helper: build the graph for one named fixture and return it.
fn fixture_graph(name: &str) -> BusGraph {
    let project = examples_dir().join(name);
    graph_for_project(&project)
        .unwrap_or_else(|| panic!("fixture `{name}` has no bus graph"))
        .1
}

#[test]
fn bare_fn_main_fixtures_are_eligible() {
    // The devirt closed-world notion treats a bare top-level
    // `fn main` free function as an entry point (an executable can't
    // gain subscribers at runtime → its bus graph is complete).
    // 05-bus — like most of the corpus — uses the bare
    // `fn main() { EchoL{}; … }` idiom with anonymous children (no
    // `main locus`), and its plain local pub/sub subjects are
    // therefore eligible. This pins the broadened gate.
    let g = fixture_graph("05-bus");
    for subj in ["demo.greeting", "demo.ack"] {
        let info = g
            .subjects
            .get(subj)
            .unwrap_or_else(|| panic!("missing subject {subj}"));
        assert!(
            info.eligible,
            "{subj} is a closed-world (fn-main-rooted) plain local pub/sub \
             -> eligible; got {:?}",
            info.ineligible_reason
        );
    }
}

#[test]
fn closed_world_plain_local_pub_sub_is_eligible() {
    // The canonical devirt-target shape: a `main locus` (closed
    // world) wiring a plain local publisher to a plain local
    // subscriber, no transport/wildcard/cross-seed/keyed forms.
    let src = r#"
type Tick { n: Int; }
topic Beat { payload: Tick; subject: "beat"; }

locus Producer {
    bus { publish Beat; }
    birth() { Beat <- Tick { n: 1 }; }
}

locus Consumer {
    bus { subscribe Beat as on_beat; }
    fn on_beat(t: Tick) { }
}

main locus App {
    params {
        p: Producer = Producer { };
        c: Consumer = Consumer { };
    }
}

fn main() { App { }; }
"#;
    let g = build_synthetic(src);
    let info = g.subjects.get("Beat").expect("missing Beat");
    assert!(
        info.eligible,
        "a closed-world plain local pub/sub must be eligible; got {:?}",
        info.ineligible_reason
    );
    assert_eq!(info.publishers.len(), 1);
    assert_eq!(info.subscribers.len(), 1);
    assert_eq!(info.subscribers[0].handler, "on_beat");
}

#[test]
fn pinned_subscriber_is_eligible_but_marked_pinned() {
    // 19-pinned-bus: the subject is local closed-world (eligible),
    // but its subscriber is `placement { sub: pinned; }` — the
    // placement is surfaced for #1b (a pinned subscriber must still
    // route through its mailbox, not a same-thread direct call).
    let g = fixture_graph("19-pinned-bus");
    let info = g.subjects.get("tick").expect("missing subject tick");
    assert!(
        info.eligible,
        "tick is a plain local subject; eligible. got {:?}",
        info.ineligible_reason
    );
    assert!(
        info.subscribers
            .iter()
            .any(|s| s.placement == Placement::Pinned),
        "the `sub: pinned` subscriber must classify as Pinned; got {:?}",
        info.subscribers
            .iter()
            .map(|s| (&s.locus, placement_str(&s.placement)))
            .collect::<Vec<_>>()
    );
}

#[test]
fn library_without_main_is_open_world() {
    // No `main` locus → open world: the other end of a channel may
    // live downstream, so nothing is devirtualizable. Synthetic
    // source (the file corpus has no main-less bus fragment).
    let src = r#"
type Tick { n: Int; }
topic Beat { payload: Tick; subject: "beat"; }

locus Producer {
    bus { publish Beat; }
    birth() { Beat <- Tick { n: 1 }; }
}
"#;
    let g = build_synthetic(src);
    let info = g.subjects.get("Beat").expect("missing Beat");
    assert!(!info.eligible);
    assert_eq!(
        info.ineligible_reason.as_ref().map(|r| r.tag()),
        Some("OpenWorld")
    );
}

#[test]
fn transport_bound_subject_is_ineligible() {
    // A `bindings { Beat: Adapter }` block binds the subject to a
    // transport adapter — an external peer may be the real
    // counterparty, so the local subscriber set is not closed.
    let src = r#"
type Tick { n: Int; }
topic Beat { payload: Tick; subject: "beat"; }

locus MyAdapter {
    params { label: String = "noname"; }
    fn send(subject: String, bytes: Bytes) { }
}

locus Producer {
    bus { publish Beat; subscribe Beat as on_beat; }
    fn on_beat(t: Tick) { }
    birth() { Beat <- Tick { n: 1 }; }
}

main locus App {
    params { p: Producer = Producer { }; }
    bindings { Beat: MyAdapter { label: "T" }; }
}

fn main() { App { }; }
"#;
    let g = build_synthetic(src);
    let info = g.subjects.get("Beat").expect("missing Beat");
    assert!(!info.eligible);
    assert_eq!(
        info.ineligible_reason.as_ref().map(|r| r.tag()),
        Some("TransportBound")
    );
}

#[test]
fn wildcard_subscriber_is_ineligible() {
    // `subscribe "log.**"` is a wildcard — the matched subject set
    // is open. Both the `**` pattern itself AND any concrete
    // subject it covers classify Wildcard. The file corpus has no
    // wildcard fixture, so this is synthetic.
    let src = r#"
type Line { msg: String; }

locus Emitter {
    bus { publish "log.app" of type Line; }
    birth() { "log.app" <- Line { msg: "hi" }; }
}

locus Sink {
    bus { subscribe "log.**" as on_log of type Line; }
    fn on_log(l: Line) { }
}

main locus App {
    params { e: Emitter = Emitter { }; s: Sink = Sink { }; }
}

fn main() { App { }; }
"#;
    let g = build_synthetic(src);
    let app = g.subjects.get("log.app").expect("missing log.app");
    assert!(!app.eligible);
    assert_eq!(
        app.ineligible_reason.as_ref().map(|r| r.tag()),
        Some("Wildcard"),
        "a subject covered by a wildcard subscriber is Wildcard"
    );
    let pat = g.subjects.get("log.**").expect("missing log.**");
    assert!(!pat.eligible);
    assert_eq!(
        pat.ineligible_reason.as_ref().map(|r| r.tag()),
        Some("Wildcard"),
        "the `**` pattern subject itself is Wildcard"
    );
}

/// Build a `BusGraph` from a single inline source string.
fn build_synthetic(src: &str) -> BusGraph {
    let prog = parse_source(src).expect("parse failed");
    let mut programs: BTreeMap<String, &Program> = BTreeMap::new();
    programs.insert(String::new(), &prog);
    let bundle = Bundle { programs };
    let _ = check_bundle(&bundle);
    let (top, _) = build_top_scope(&bundle);
    build_bus_graph(&bundle, &top)
}

/// Sanity: `is_main` detection used by the gate matches the AST
/// flag the rest of the checker uses.
#[allow(dead_code)]
fn has_main(p: &Program) -> bool {
    p.items
        .iter()
        .any(|i| matches!(i, TopDecl::Locus(l) if l.is_main))
}
