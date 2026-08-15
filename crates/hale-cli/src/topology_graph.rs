//! GH #476 Track A — deterministic renderer over topology artifacts.
//!
//! `hale topology graph <artifact> [--view V] [--format F] [...]`
//!
//! This is an **artifact client**: it reads exactly the JSON a third
//! party reads (`--dump-topology` output), never touches Hale source,
//! and has no dependency on the future `hale-model` crate. Its input
//! adapter can later switch from artifact rows to the canonical
//! `ApplicationModel` without changing render configs or output.
//!
//! `RenderGraph` below is deliberately presentational and disposable —
//! it is NOT the canonical model, carries no semantic authority, and
//! may be replaced without migration guarantees.
//!
//! Determinism rules (the point of the tool — outputs are
//! regression-tested snapshots, and slide decks are generated from
//! them):
//!   - node identity derives from artifact names, never allocation or
//!     iteration order; every collection is sorted before rendering;
//!   - fixed character-cell text metrics (no font measurement — text
//!     extents are `len * CHAR_W`, so output is byte-stable across
//!     machines regardless of installed fonts);
//!   - no timestamps, no absolute paths, no machine identity;
//!   - the SVG layout is a simple layered two-column arrangement
//!     computed with integer arithmetic only.

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::Value;

const CHAR_W: i64 = 8; // fixed glyph advance for font-size 13 monospace
const LINE_H: i64 = 18;
const PAD: i64 = 10;
const BOX_GAP: i64 = 24;
const COL_GAP: i64 = 140;

pub fn run_topology(rest: &[String]) -> ExitCode {
    if rest.first().map(String::as_str) != Some("graph") {
        usage();
        return ExitCode::from(2);
    }
    let mut artifact_path: Option<PathBuf> = None;
    let mut view = "system".to_string();
    let mut format = "svg".to_string();
    let mut out_path: Option<PathBuf> = None;
    let mut claim: Option<String> = None;
    let mut config_path: Option<PathBuf> = None;

    let mut it = rest[1..].iter().peekable();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--view" => match it.next() {
                Some(v) => view = v.clone(),
                None => return flag_err("--view needs a value"),
            },
            "--format" => match it.next() {
                Some(v) => format = v.clone(),
                None => return flag_err("--format needs a value"),
            },
            "-o" | "--output" => match it.next() {
                Some(v) => out_path = Some(PathBuf::from(v)),
                None => return flag_err("-o needs a path"),
            },
            "--claim" => match it.next() {
                Some(v) => claim = Some(v.clone()),
                None => return flag_err("--claim needs a name"),
            },
            "--config" => match it.next() {
                Some(v) => config_path = Some(PathBuf::from(v)),
                None => return flag_err("--config needs a path"),
            },
            other if other.starts_with('-') => {
                eprintln!("unknown flag: {}", other);
                usage();
                return ExitCode::from(2);
            }
            other => {
                if artifact_path.is_some() {
                    eprintln!("unexpected extra argument: {}", other);
                    usage();
                    return ExitCode::from(2);
                }
                artifact_path = Some(PathBuf::from(other));
            }
        }
    }
    let Some(path) = artifact_path else {
        usage();
        return ExitCode::from(2);
    };
    if !matches!(view.as_str(), "system" | "code" | "bus" | "claim" | "residue") {
        eprintln!(
            "unknown view `{}` (expect system|code|bus|claim|residue)",
            view
        );
        return ExitCode::from(2);
    }
    if !matches!(format.as_str(), "svg" | "mermaid" | "dot") {
        eprintln!("unknown format `{}` (expect svg|mermaid|dot)", format);
        return ExitCode::from(2);
    }
    if view == "claim" && claim.is_none() {
        eprintln!("--view claim requires --claim <name>");
        return ExitCode::from(2);
    }

    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot read {}: {}", path.display(), e);
            return ExitCode::from(1);
        }
    };
    // Admission, in the fleet loader's order: integrity BEFORE
    // meaning, meaning before rendering. The renderer does NOT
    // require a clean verdict (violations are worth drawing), but it
    // refuses artifacts it cannot trust or cannot correctly read.
    let art = match Artifact::admit(&path, &raw) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::from(1);
        }
    };

    let config = match config_path {
        None => RenderConfig::default(),
        Some(p) => match RenderConfig::load(&p) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("bad --config {}: {}", p.display(), e);
                return ExitCode::from(1);
            }
        },
    };

    let mut graph = match build_graph(&art, &view, claim.as_deref()) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::from(1);
        }
    };
    config.apply(&mut graph);

    let output = match format.as_str() {
        "mermaid" => render_mermaid(&graph),
        "dot" => render_dot(&graph),
        _ => render_svg(&graph),
    };
    match out_path {
        None => {
            print!("{}", output);
            ExitCode::SUCCESS
        }
        Some(p) => match std::fs::write(&p, output) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("cannot write {}: {}", p.display(), e);
                ExitCode::from(1)
            }
        },
    }
}

fn usage() {
    eprintln!(
        "usage: hale topology graph <artifact.json>\n\
         \x20   [--view system|code|bus|claim|residue]   (default: system)\n\
         \x20   [--format svg|mermaid|dot]               (default: svg)\n\
         \x20   [--claim <name>]                         (required for --view claim)\n\
         \x20   [--config <render-config.json>]\n\
         \x20   [-o <path>]                              (default: stdout)\n\
         \n\
         Renders a `--dump-topology` artifact deterministically. An\n\
         artifact client: reads only the committed JSON, never source.\n\
         Experimental surface (pre-1.0)."
    );
}

fn flag_err(msg: &str) -> ExitCode {
    eprintln!("{}", msg);
    usage();
    ExitCode::from(2)
}

// ---------------------------------------------------------------
// Artifact accessors (schema 1.x)
// ---------------------------------------------------------------

struct Artifact {
    v: Value,
}

impl Artifact {
    /// Admission: (1) whole-body `artifact_digest` verifies — a
    /// hand-edited claims/topics section is refused, not rendered
    /// under a stale identity; (2) `semantics` matches this build —
    /// rows that share a shape but mean different things are not
    /// renderable; (3) the schema minor is one this adapter's field
    /// set actually covers (`topics` landed in 1.2, `verdict` in
    /// 1.4 — older 1.x artifacts would silently render as having
    /// neither); higher minors are accepted per the artifact's
    /// additive-minor contract; (4) the sections this adapter reads
    /// are present with the right JSON types — absence is an error,
    /// never an empty graph.
    fn admit(
        path: &std::path::Path,
        raw: &str,
    ) -> Result<Artifact, String> {
        match hale_types::topology::verify_artifact_digest(raw) {
            Some(true) => {}
            Some(false) => {
                return Err(format!(
                    "{}: artifact_digest does not match its contents — \
                     refusing to render an edited or corrupted artifact",
                    path.display()
                ))
            }
            None => {
                return Err(format!(
                    "{}: no artifact_digest (predates schema 1.3) — an \
                     unverifiable artifact is not renderable; re-dump \
                     with a current compiler",
                    path.display()
                ))
            }
        }
        let v: Value = serde_json::from_str(raw)
            .map_err(|e| format!("{}: not a JSON artifact: {}", path.display(), e))?;
        let sem = v["semantics"].as_u64();
        if sem != Some(hale_types::topology::MODEL_SEMANTICS as u64) {
            return Err(format!(
                "{}: model semantics {} — this build speaks {}; the rows \
                 may share a shape and mean different things",
                path.display(),
                sem.map(|s| s.to_string()).unwrap_or_else(|| "absent".into()),
                hale_types::topology::MODEL_SEMANTICS
            ));
        }
        let art = Artifact { v };
        let schema = art.schema();
        let minor: Option<u32> = schema
            .strip_prefix("1.")
            .and_then(|m| m.parse().ok());
        match minor {
            Some(m) if m >= 4 => {}
            _ => {
                return Err(format!(
                    "{}: unsupported topology artifact schema `{}` — this \
                     renderer's adapter covers 1.4+ (topics landed in \
                     1.2, verdict in 1.4; an older artifact would render \
                     misleadingly incomplete)",
                    path.display(),
                    schema
                ))
            }
        }
        // Structural presence: every section this adapter reads.
        let need = |ok: bool, what: &str| -> Result<(), String> {
            if ok {
                Ok(())
            } else {
                Err(format!(
                    "{}: malformed artifact — {} (digest verified, so \
                     this is a schema drift, not corruption)",
                    path.display(),
                    what
                ))
            }
        };
        need(
            art.v["sorts"]["loci"].is_array(),
            "sorts.loci must be an array",
        )?;
        need(
            art.v["sorts"]["fns"].is_array(),
            "sorts.fns must be an array",
        )?;
        for rel in ["calls", "calls_via_stdlib", "publishes", "subscribes"] {
            need(
                art.v["relations"][rel].is_array(),
                &format!("relations.{} must be an array", rel),
            )?;
        }
        need(art.v["topics"].is_array(), "topics must be an array")?;
        need(art.v["claims"].is_array(), "claims must be an array")?;
        need(art.v["unknowns"].is_array(), "unknowns must be an array")?;
        need(art.v["groups"].is_object(), "groups must be an object")?;
        need(art.v["phases"].is_object(), "phases must be an object")?;
        need(art.v["effects"].is_object(), "effects must be an object")?;
        need(art.v["verdict"].is_string(), "verdict must be a string")?;
        need(
            art.v["shape_hash"].is_string(),
            "shape_hash must be a string",
        )?;
        Ok(art)
    }

    fn schema(&self) -> String {
        match &self.v["schema"] {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => String::new(),
        }
    }
    fn str_list(&self, path: &[&str]) -> Vec<String> {
        let mut cur = &self.v;
        for p in path {
            cur = &cur[*p];
        }
        let mut out: Vec<String> = cur
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out
    }
    fn rows(&self, path: &[&str]) -> Vec<&Value> {
        let mut cur = &self.v;
        for p in path {
            cur = &cur[*p];
        }
        cur.as_array().map(|a| a.iter().collect()).unwrap_or_default()
    }
    fn loci(&self) -> Vec<String> {
        self.str_list(&["sorts", "loci"])
    }
    fn fns(&self) -> Vec<String> {
        self.str_list(&["sorts", "fns"])
    }
    fn topics(&self) -> Vec<TopicRow> {
        let mut t: Vec<TopicRow> = self
            .rows(&["topics"])
            .iter()
            .filter_map(|r| {
                Some(TopicRow {
                    name: r["name"].as_str()?.to_string(),
                    subject: r["subject"].as_str().unwrap_or("").to_string(),
                    payload_hash: r["payload_hash"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                })
            })
            .collect();
        t.sort_by(|a, b| a.name.cmp(&b.name));
        t
    }
    fn publishes(&self) -> Vec<(String, String)> {
        let mut p: Vec<(String, String)> = self
            .rows(&["relations", "publishes"])
            .iter()
            .filter_map(|r| {
                Some((
                    r["fn"].as_str()?.to_string(),
                    r["subject"].as_str()?.to_string(),
                ))
            })
            .collect();
        p.sort();
        p
    }
    /// (topic, locus, handler)
    fn subscribes(&self) -> Vec<(String, String, String)> {
        let mut s: Vec<(String, String, String)> = self
            .rows(&["relations", "subscribes"])
            .iter()
            .filter_map(|r| {
                Some((
                    r["subject"].as_str()?.to_string(),
                    r["locus"].as_str()?.to_string(),
                    r["handler"].as_str()?.to_string(),
                ))
            })
            .collect();
        s.sort();
        s
    }
    fn calls(&self, via_stdlib: bool) -> Vec<(String, String)> {
        let key = if via_stdlib { "calls_via_stdlib" } else { "calls" };
        let mut c: Vec<(String, String)> = self
            .rows(&["relations", key])
            .iter()
            .filter_map(|r| {
                Some((
                    r["from"].as_str()?.to_string(),
                    r["to"].as_str()?.to_string(),
                ))
            })
            .collect();
        c.sort();
        c
    }
    fn groups(&self) -> Vec<(String, Vec<String>)> {
        let mut g: Vec<(String, Vec<String>)> = self.v["groups"]
            .as_object()
            .map(|o| {
                o.iter()
                    .map(|(k, v)| {
                        let mut members: Vec<String> = v
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(|x| {
                                        x.as_str().map(str::to_string)
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        members.sort();
                        (k.clone(), members)
                    })
                    .collect()
            })
            .unwrap_or_default();
        g.sort();
        g
    }
    fn phase_of(&self, f: &str) -> Option<(String, String)> {
        let p = &self.v["phases"][f];
        Some((
            p["phase"].as_str()?.to_string(),
            p["kind"].as_str()?.to_string(),
        ))
    }
    fn effects_of(&self, f: &str) -> Vec<String> {
        // Effect ORDER inside a fn's row is semantic in the artifact
        // (declaration order); keep it, don't sort.
        self.v["effects"][f]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }
    /// (fn, reasons)
    fn unknowns(&self) -> Vec<(String, Vec<String>)> {
        let mut u: Vec<(String, Vec<String>)> = self
            .rows(&["unknowns"])
            .iter()
            .filter_map(|r| {
                let mut reasons: Vec<String> = r["reasons"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                reasons.sort();
                Some((r["fn"].as_str()?.to_string(), reasons))
            })
            .collect();
        u.sort();
        u
    }
    /// (name, form, result)
    fn claims(&self) -> Vec<(String, String, String)> {
        let mut c: Vec<(String, String, String)> = self
            .rows(&["claims"])
            .iter()
            .filter_map(|r| {
                Some((
                    r["name"].as_str()?.to_string(),
                    r["form"].as_str().unwrap_or("").to_string(),
                    r["result"].as_str().unwrap_or("").to_string(),
                ))
            })
            .collect();
        c.sort();
        c
    }
    fn footer(&self) -> String {
        let shape = self.v["shape_hash"].as_str().unwrap_or("?");
        let verdict = self.v["verdict"].as_str().unwrap_or("?");
        format!(
            "schema {} · shape {} · verdict {}",
            self.schema(),
            shape,
            verdict
        )
    }
}

struct TopicRow {
    name: String,
    subject: String,
    payload_hash: String,
}

// ---------------------------------------------------------------
// RenderGraph — presentational only
// ---------------------------------------------------------------

#[derive(Clone, PartialEq)]
enum EdgeKind {
    Call,
    CallViaStdlib,
    Publish,
    Subscribe,
    Hole,
}

#[derive(Clone)]
struct FnLine {
    full: String,  // "Locus::fn" or bare fn — the anchor identity
    label: String, // rendered text (short name + chips)
    highlight: bool,
}

#[derive(Clone)]
struct LocusBox {
    name: String, // locus name, or "(free functions)"
    lines: Vec<FnLine>,
    highlight: bool,
}

#[derive(Clone)]
struct TopicNode {
    name: String,
    sub: String, // "subject · payload abcd1234"
    highlight: bool,
}

#[derive(Clone)]
struct UnknownNode {
    id: String, // "unknown:<fn>" — the edge from its fn carries the anchor
    label: String,
}

#[derive(Clone)]
struct Edge {
    from: String, // anchor id: fn full name, "topic:<name>", "unknown:<fn>"
    to: String,
    kind: EdgeKind,
    label: String,
    highlight: bool,
}

#[derive(Clone)]
struct Card {
    title: String,
    lines: Vec<String>,
    tone: CardTone,
}

#[derive(Clone, PartialEq)]
enum CardTone {
    Ok,
    Bad,
    Warn,
}

struct RenderGraph {
    title: String,
    footer: String,
    boxes: Vec<LocusBox>,
    topics: Vec<TopicNode>,
    unknowns: Vec<UnknownNode>,
    edges: Vec<Edge>,
    cards: Vec<Card>,
}

/// The owning locus of a function by LONGEST declared-locus
/// prefix. Author spellings may contain seed aliases
/// (`p::Store::get` is method `get` on imported locus `p::Store`),
/// so membership is never inferred from the first `::` — only a
/// declared locus name followed by `::` claims a function. No
/// declared prefix ⇒ free function, even when the spelling is
/// qualified (`p::helper`).
fn owner_of<'a>(loci: &'a [String], fn_full: &str) -> Option<&'a str> {
    loci.iter()
        .filter(|l| {
            fn_full.len() > l.len() + 2
                && fn_full.starts_with(l.as_str())
                && fn_full[l.len()..].starts_with("::")
        })
        .max_by_key(|l| l.len())
        .map(|l| l.as_str())
}

const FREE_FNS: &str = "(free functions)";

fn build_graph(
    art: &Artifact,
    view: &str,
    claim: Option<&str>,
) -> Result<RenderGraph, String> {
    let loci = art.loci();
    let fns = art.fns();
    let declared_topics = art.topics();
    let publishes = art.publishes();
    let subscribes = art.subscribes();
    let with_chips = view != "bus";

    // Which fns appear as lines. bus view: only endpoint fns.
    let keep_fn = |f: &String| -> bool {
        if view != "bus" {
            return true;
        }
        publishes.iter().any(|(pf, _)| pf == f)
            || subscribes
                .iter()
                .any(|(_, l, h)| format!("{}::{}", l, h) == *f)
    };

    // Locus boxes with fn lines, membership by longest declared
    // prefix; everything unclaimed is free.
    let mut boxes: Vec<LocusBox> = Vec::new();
    for locus in &loci {
        let lines: Vec<FnLine> = fns
            .iter()
            .filter(|f| owner_of(&loci, f) == Some(locus.as_str()))
            .filter(|f| keep_fn(f))
            .map(|f| FnLine {
                full: f.clone(),
                label: fn_label(art, f, &f[locus.len() + 2..], with_chips),
                highlight: false,
            })
            .collect();
        boxes.push(LocusBox {
            name: locus.clone(),
            lines,
            highlight: false,
        });
    }
    let free: Vec<FnLine> = fns
        .iter()
        .filter(|f| owner_of(&loci, f).is_none())
        .filter(|f| keep_fn(f))
        .map(|f| FnLine {
            full: f.clone(),
            label: fn_label(art, f, f, with_chips),
            highlight: false,
        })
        .collect();
    if !free.is_empty() {
        boxes.push(LocusBox {
            name: FREE_FNS.to_string(),
            lines: free,
            highlight: false,
        });
    }

    // Bus nodes: the UNION of subjects referenced by publish and
    // subscribe rows, plus declared topics. Literal and wildcard
    // subjects have no declared-topic row but are real endpoints —
    // they get a visible node, enriched with wire/payload identity
    // only when a declaration exists.
    let topic_nodes: Vec<TopicNode> = if view == "code" {
        Vec::new()
    } else {
        let mut names: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        for (_, t) in &publishes {
            names.insert(t.clone());
        }
        for (t, _, _) in &subscribes {
            names.insert(t.clone());
        }
        for t in &declared_topics {
            names.insert(t.name.clone());
        }
        names
            .into_iter()
            .map(|name| {
                let sub = match declared_topics
                    .iter()
                    .find(|t| t.name == name)
                {
                    Some(t) => format!(
                        "{} · payload {}",
                        t.subject,
                        t.payload_hash.chars().take(8).collect::<String>()
                    ),
                    None if name.contains('*') => {
                        "(pattern subject)".to_string()
                    }
                    None => "(literal subject)".to_string(),
                };
                TopicNode {
                    name,
                    sub,
                    highlight: false,
                }
            })
            .collect()
    };
    let mut topic_nodes = topic_nodes;

    // Edges.
    let mut edges: Vec<Edge> = Vec::new();
    if view != "code" {
        for (f, topic) in &publishes {
            edges.push(Edge {
                from: f.clone(),
                to: format!("topic:{}", topic),
                kind: EdgeKind::Publish,
                label: "publish".to_string(),
                highlight: false,
            });
        }
        for (topic, locus, handler) in &subscribes {
            edges.push(Edge {
                from: format!("topic:{}", topic),
                to: format!("{}::{}", locus, handler),
                kind: EdgeKind::Subscribe,
                label: "subscribe".to_string(),
                highlight: false,
            });
        }
    }
    if view != "bus" {
        for (from, to) in art.calls(false) {
            edges.push(Edge {
                from,
                to,
                kind: EdgeKind::Call,
                label: "calls".to_string(),
                highlight: false,
            });
        }
        for (from, to) in art.calls(true) {
            edges.push(Edge {
                from,
                to,
                kind: EdgeKind::CallViaStdlib,
                label: "calls (via stdlib)".to_string(),
                highlight: false,
            });
        }
    }

    // Unknown residue nodes (residue view renders them; every other
    // view — claim included — notes them in a card, so residue is
    // never invisible).
    let unknown_rows = art.unknowns();
    let mut unknowns: Vec<UnknownNode> = Vec::new();
    if view == "residue" {
        for (f, reasons) in &unknown_rows {
            let id = format!("unknown:{}", f);
            unknowns.push(UnknownNode {
                id: id.clone(),
                label: reasons.join("; "),
            });
            edges.push(Edge {
                from: f.clone(),
                to: id,
                kind: EdgeKind::Hole,
                label: "unresolved".to_string(),
                highlight: false,
            });
        }
    }

    // Cards + view-specific decoration. NO early returns: the
    // residue card at the bottom must reach every non-residue view.
    let mut cards: Vec<Card> = Vec::new();
    match view {
        "residue" => {
            if unknown_rows.is_empty() {
                cards.push(Card {
                    title: "no unresolved residue".to_string(),
                    lines: vec![
                        "every relation in this artifact is exact".to_string()
                    ],
                    tone: CardTone::Ok,
                });
            }
        }
        "claim" => {
            let name = claim.expect("checked in run_topology");
            let Some((cname, form, result)) =
                art.claims().into_iter().find(|(n, _, _)| n == name)
            else {
                let known: Vec<String> =
                    art.claims().into_iter().map(|(n, _, _)| n).collect();
                return Err(format!(
                    "claim `{}` is not in this artifact (has: {})",
                    name,
                    if known.is_empty() {
                        "none".to_string()
                    } else {
                        known.join(", ")
                    }
                ));
            };
            let tone = match result.as_str() {
                "holds" => CardTone::Ok,
                "violated" => CardTone::Bad,
                _ => CardTone::Warn,
            };
            cards.push(Card {
                title: format!("claim {} — {}", cname, result),
                lines: vec![form.clone()],
                tone,
            });
            // Highlight every group member (loci AND free fns) and
            // topic the claim's rendered form names. (The artifact
            // carries the claim as normalized text; structured
            // ClaimIr rows are the Track B upgrade — this stays
            // presentation-side.)
            let mut hl: Vec<String> = Vec::new();
            for (gname, members) in art.groups() {
                if form_mentions(&form, &gname) {
                    hl.extend(members);
                }
            }
            for l in &loci {
                if form_mentions(&form, l) {
                    hl.push(l.clone());
                }
            }
            for b in &mut boxes {
                if hl.iter().any(|h| h == &b.name) {
                    b.highlight = true;
                }
                for line in &mut b.lines {
                    if hl.iter().any(|h| h == &line.full) {
                        line.highlight = true;
                    }
                }
            }
            for t in &mut topic_nodes {
                if form_mentions(&form, &t.name) {
                    t.highlight = true;
                }
            }
        }
        _ => {}
    }
    if view != "residue" && !unknown_rows.is_empty() {
        cards.push(Card {
            title: format!("{} unresolved", unknown_rows.len()),
            lines: unknown_rows
                .iter()
                .map(|(f, r)| format!("{}: {}", f, r.join("; ")))
                .collect(),
            tone: CardTone::Warn,
        });
    }

    Ok(RenderGraph {
        title: format!("topology · {} view", view),
        footer: art.footer(),
        boxes,
        topics: topic_nodes,
        unknowns,
        edges,
        cards,
    })
}

/// Word-boundary mention check over the claim's normalized form, so
/// group `stores` doesn't light up locus `Store` by prefix accident.
fn form_mentions(form: &str, name: &str) -> bool {
    let bytes = form.as_bytes();
    let mut start = 0;
    while let Some(pos) = form[start..].find(name) {
        let a = start + pos;
        let b = a + name.len();
        let left_ok = a == 0 || !is_ident(bytes[a - 1]);
        let right_ok = b >= bytes.len() || !is_ident(bytes[b]);
        if left_ok && right_ok {
            return true;
        }
        start = a + 1;
    }
    false
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// `short` is the display name inside the owning box (the full name
/// with the owner's longest-prefix stripped — computed by the
/// caller, which knows the owner; a free fn keeps its full author
/// spelling, alias-qualified or not).
fn fn_label(art: &Artifact, f: &str, short: &str, with_chips: bool) -> String {
    let mut label = short.to_string();
    if !with_chips {
        return label;
    }
    if let Some((_, kind)) = art.phase_of(f) {
        if kind == "hook" {
            label.push_str("  [hook]");
        }
    }
    let effects = art.effects_of(f);
    if !effects.is_empty() {
        label.push_str("  {");
        label.push_str(&effects.join(","));
        label.push('}');
    }
    label
}

// ---------------------------------------------------------------
// Render config — presentation data, not semantics
// ---------------------------------------------------------------

#[derive(Default)]
struct RenderConfig {
    title: Option<String>,
    highlight: Vec<String>,
    hide: Vec<String>,
    focus: Vec<String>,
}

impl RenderConfig {
    fn load(path: &PathBuf) -> Result<RenderConfig, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let v: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        let list = |k: &str| -> Vec<String> {
            v[k].as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        Ok(RenderConfig {
            title: v["title"].as_str().map(str::to_string),
            highlight: list("highlight"),
            hide: list("hide"),
            focus: list("focus"),
        })
    }

    fn apply(&self, g: &mut RenderGraph) {
        if let Some(t) = &self.title {
            g.title = t.clone();
        }
        let named = |name: &str, list: &[String]| list.iter().any(|x| x == name);
        if !self.hide.is_empty() {
            g.boxes.retain(|b| !named(&b.name, &self.hide));
            g.topics.retain(|t| !named(&t.name, &self.hide));
        }
        if !self.focus.is_empty() {
            // Keep focused boxes/topics plus anything sharing an edge
            // with a focused endpoint. Ownership comes from the built
            // boxes (longest-prefix membership), never re-derived
            // from name shape.
            let fn_owner: std::collections::BTreeMap<String, String> = g
                .boxes
                .iter()
                .flat_map(|b| {
                    b.lines
                        .iter()
                        .map(|l| (l.full.clone(), b.name.clone()))
                        .collect::<Vec<_>>()
                })
                .collect();
            let anchor_owner = |anchor: &str| -> String {
                if let Some(t) = anchor.strip_prefix("topic:") {
                    t.to_string()
                } else {
                    fn_owner
                        .get(anchor)
                        .cloned()
                        .unwrap_or_else(|| FREE_FNS.to_string())
                }
            };
            let mut keep: Vec<String> = self.focus.clone();
            for e in &g.edges {
                let fo = anchor_owner(&e.from);
                let to = anchor_owner(&e.to);
                if named(&fo, &self.focus) || named(&to, &self.focus) {
                    keep.push(fo);
                    keep.push(to);
                }
            }
            g.boxes.retain(|b| named(&b.name, &keep));
            g.topics.retain(|t| named(&t.name, &keep));
        }
        for b in &mut g.boxes {
            if named(&b.name, &self.highlight) {
                b.highlight = true;
            }
            for l in &mut b.lines {
                if named(&l.full, &self.highlight) {
                    l.highlight = true;
                }
            }
        }
        for t in &mut g.topics {
            if named(&t.name, &self.highlight) {
                t.highlight = true;
            }
        }
        // Drop edges whose endpoints were hidden/defocused.
        let boxes = &g.boxes;
        let topics = &g.topics;
        let unknowns = &g.unknowns;
        let has_anchor = |a: &str| -> bool {
            if let Some(t) = a.strip_prefix("topic:") {
                return topics.iter().any(|x| x.name == t);
            }
            if a.starts_with("unknown:") {
                return unknowns.iter().any(|u| u.id == a);
            }
            boxes
                .iter()
                .any(|b| b.lines.iter().any(|l| l.full == a))
        };
        g.edges.retain(|e| has_anchor(&e.from) && has_anchor(&e.to));
        // Highlighted endpoints light their edges.
        let hl_anchor: Vec<String> = g
            .boxes
            .iter()
            .flat_map(|b| {
                b.lines
                    .iter()
                    .filter(|l| l.highlight || b.highlight)
                    .map(|l| l.full.clone())
                    .collect::<Vec<_>>()
            })
            .chain(
                g.topics
                    .iter()
                    .filter(|t| t.highlight)
                    .map(|t| format!("topic:{}", t.name)),
            )
            .collect();
        for e in &mut g.edges {
            if hl_anchor.iter().any(|a| a == &e.from || a == &e.to) {
                e.highlight = true;
            }
        }
    }
}

// ---------------------------------------------------------------
// Mermaid
// ---------------------------------------------------------------

/// Injective Mermaid node ID: a kind prefix (functions `n_`, topics
/// `t_`, unknowns `u_`, subgraphs `g_`) plus bytewise hex-escaping
/// of every non-alphanumeric byte (`_` included — `A::B` →
/// `n_A_3a_3aB`, `A__B` → `n_A_5f_5fB`). Distinct entities can
/// never collapse onto one Mermaid node, and the cross-sort prefix
/// prevents a function literally named `t_X` from colliding with
/// topic `X`.
fn mm_id(kind: &str, name: &str) -> String {
    let mut s = String::with_capacity(name.len() + 4);
    s.push_str(kind);
    s.push('_');
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() {
            s.push(b as char);
        } else {
            s.push('_');
            s.push_str(&format!("{:02x}", b));
        }
    }
    s
}

fn mm_escape(s: &str) -> String {
    s.replace('"', "&quot;")
}

fn render_mermaid(g: &RenderGraph) -> String {
    let mut out = String::new();
    out.push_str("flowchart LR\n");
    out.push_str(&format!(
        "  %% {} — {}\n",
        mm_escape(&g.title),
        mm_escape(&g.footer)
    ));
    for b in &g.boxes {
        out.push_str(&format!(
            "  subgraph {}[\"{}\"]\n",
            mm_id("g", &b.name),
            mm_escape(&b.name)
        ));
        for l in &b.lines {
            out.push_str(&format!(
                "    {}[\"{}\"]\n",
                mm_id("n", &l.full),
                mm_escape(&l.label)
            ));
        }
        out.push_str("  end\n");
    }
    for t in &g.topics {
        out.push_str(&format!(
            "  {}([\"{}<br/>{}\"])\n",
            mm_id("t", &t.name),
            mm_escape(&t.name),
            mm_escape(&t.sub)
        ));
    }
    for u in &g.unknowns {
        out.push_str(&format!(
            "  {}[/\"{}\"/]\n",
            mm_id("u", &u.id),
            mm_escape(&u.label)
        ));
    }
    let anchor = |a: &str| -> String {
        if let Some(t) = a.strip_prefix("topic:") {
            mm_id("t", t)
        } else if a.starts_with("unknown:") {
            mm_id("u", a)
        } else {
            mm_id("n", a)
        }
    };
    for e in &g.edges {
        let arrow = match e.kind {
            EdgeKind::Publish | EdgeKind::Call => "-->",
            EdgeKind::Subscribe | EdgeKind::CallViaStdlib => "-.->",
            EdgeKind::Hole => "-.->",
        };
        out.push_str(&format!(
            "  {} {}|{}| {}\n",
            anchor(&e.from),
            arrow,
            mm_escape(&e.label),
            anchor(&e.to)
        ));
    }
    // Highlight classes.
    let mut hl_nodes: Vec<String> = Vec::new();
    for b in &g.boxes {
        for l in &b.lines {
            if l.highlight || b.highlight {
                hl_nodes.push(mm_id("n", &l.full));
            }
        }
    }
    for t in &g.topics {
        if t.highlight {
            hl_nodes.push(mm_id("t", &t.name));
        }
    }
    if !hl_nodes.is_empty() {
        out.push_str("  classDef hl stroke:#d69e2e,stroke-width:3px\n");
        out.push_str(&format!("  class {} hl\n", hl_nodes.join(",")));
    }
    for c in &g.cards {
        out.push_str(&format!(
            "  %% card: {} | {}\n",
            mm_escape(&c.title),
            mm_escape(&c.lines.join(" | "))
        ));
    }
    out
}

// ---------------------------------------------------------------
// DOT
// ---------------------------------------------------------------

fn render_dot(g: &RenderGraph) -> String {
    let mut out = String::new();
    out.push_str("digraph topology {\n");
    out.push_str("  rankdir=LR;\n  fontname=\"monospace\";\n");
    out.push_str("  node [fontname=\"monospace\", shape=box];\n");
    out.push_str(&format!(
        "  label=\"{} — {}\";\n",
        dot_escape(&g.title),
        dot_escape(&g.footer)
    ));
    for (i, b) in g.boxes.iter().enumerate() {
        out.push_str(&format!(
            "  subgraph cluster_{} {{\n    label=\"{}\";\n{}",
            i,
            dot_escape(&b.name),
            if b.highlight {
                "    color=\"#d69e2e\"; penwidth=2;\n"
            } else {
                ""
            }
        ));
        for l in &b.lines {
            out.push_str(&format!(
                "    \"{}\" [label=\"{}\"{}];\n",
                dot_escape(&l.full),
                dot_escape(&l.label),
                if l.highlight {
                    ", color=\"#d69e2e\", penwidth=2"
                } else {
                    ""
                }
            ));
        }
        out.push_str("  }\n");
    }
    for t in &g.topics {
        out.push_str(&format!(
            "  \"topic:{}\" [label=\"{}\\n{}\", shape=ellipse{}];\n",
            dot_escape(&t.name),
            dot_escape(&t.name),
            dot_escape(&t.sub),
            if t.highlight {
                ", color=\"#d69e2e\", penwidth=2"
            } else {
                ""
            }
        ));
    }
    for u in &g.unknowns {
        out.push_str(&format!(
            "  \"{}\" [label=\"{}\", shape=diamond, color=\"#c53030\"];\n",
            dot_escape(&u.id),
            dot_escape(&u.label)
        ));
    }
    for e in &g.edges {
        let style = match e.kind {
            EdgeKind::Publish => "solid",
            EdgeKind::Subscribe => "dashed",
            EdgeKind::Call => "solid",
            EdgeKind::CallViaStdlib => "dashed",
            EdgeKind::Hole => "dotted",
        };
        out.push_str(&format!(
            "  \"{}\" -> \"{}\" [label=\"{}\", style={}{}];\n",
            dot_escape(&e.from),
            dot_escape(&e.to),
            dot_escape(&e.label),
            style,
            if e.highlight {
                ", color=\"#d69e2e\", penwidth=2"
            } else {
                ""
            }
        ));
    }
    out.push_str("}\n");
    out
}

fn dot_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

// ---------------------------------------------------------------
// SVG — fixed-metrics layered layout
// ---------------------------------------------------------------

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn text_w(s: &str) -> i64 {
    s.chars().count() as i64 * CHAR_W
}

struct Placed {
    x: i64,
    y: i64,
    w: i64,
    h: i64,
}

fn render_svg(g: &RenderGraph) -> String {
    // ---- layout: left column of locus boxes, right column of
    // topics, further-right column of unknown nodes, cards top-right.
    let title_h = 2 * LINE_H + PAD;

    // Left column: uniform width from the widest content line.
    let mut left_w: i64 = text_w("(free functions)");
    for b in &g.boxes {
        left_w = left_w.max(text_w(&b.name) + 2 * PAD);
        for l in &b.lines {
            left_w = left_w.max(text_w(&l.label) + 3 * PAD);
        }
    }
    // Call edges between left-column boxes route through a left
    // gutter; size it from the widest call label so nothing clips.
    let call_labels_w = g
        .edges
        .iter()
        .filter(|e| {
            matches!(e.kind, EdgeKind::Call | EdgeKind::CallViaStdlib)
        })
        .map(|e| text_w(&e.label))
        .max()
        .unwrap_or(0);
    let gutter = if call_labels_w > 0 {
        call_labels_w + 3 * PAD
    } else {
        0
    };
    let mut y = title_h + BOX_GAP;
    let x0 = PAD * 2 + gutter;
    let mut box_pos: Vec<Placed> = Vec::new();
    let mut fn_anchor: Vec<(String, i64, i64, i64)> = Vec::new(); // (full, right-x, left-x, y-center)
    for b in &g.boxes {
        let h = LINE_H + (b.lines.len() as i64) * LINE_H + PAD;
        box_pos.push(Placed {
            x: x0,
            y,
            w: left_w,
            h,
        });
        for (i, l) in b.lines.iter().enumerate() {
            let cy = y + LINE_H + (i as i64) * LINE_H + LINE_H / 2;
            fn_anchor.push((l.full.clone(), x0 + left_w, x0, cy));
        }
        y += h + BOX_GAP;
    }
    let left_bottom = y;

    // Topic column.
    let mut topic_w: i64 = 0;
    for t in &g.topics {
        topic_w = topic_w.max(text_w(&t.name).max(text_w(&t.sub)) + 2 * PAD);
    }
    let tx = x0 + left_w + COL_GAP;
    let mut ty = title_h + BOX_GAP;
    let mut topic_pos: Vec<(String, Placed)> = Vec::new();
    for t in &g.topics {
        let h = 2 * LINE_H + PAD;
        topic_pos.push((
            t.name.clone(),
            Placed {
                x: tx,
                y: ty,
                w: topic_w,
                h,
            },
        ));
        ty += h + BOX_GAP;
    }

    // Unknown column.
    let mut unk_w: i64 = 0;
    for u in &g.unknowns {
        unk_w = unk_w.max(text_w(&u.label) + 2 * PAD);
    }
    let ux = if g.topics.is_empty() {
        x0 + left_w + COL_GAP
    } else {
        tx + topic_w + COL_GAP
    };
    let mut uy = title_h + BOX_GAP;
    let mut unk_pos: Vec<(String, Placed)> = Vec::new();
    for u in &g.unknowns {
        let h = LINE_H + PAD;
        unk_pos.push((
            u.id.clone(),
            Placed {
                x: ux,
                y: uy,
                w: unk_w,
                h,
            },
        ));
        uy += h + BOX_GAP;
    }

    // Cards under the rightmost column.
    let mut card_w: i64 = 0;
    for c in &g.cards {
        card_w = card_w.max(text_w(&c.title) + 2 * PAD);
        for l in &c.lines {
            card_w = card_w.max(text_w(l) + 2 * PAD);
        }
    }
    let cx = if !g.unknowns.is_empty() {
        ux
    } else if !g.topics.is_empty() {
        tx + topic_w + COL_GAP
    } else {
        x0 + left_w + COL_GAP
    };
    let mut cy0 = [ty, uy, title_h + BOX_GAP]
        .iter()
        .copied()
        .max()
        .unwrap_or(title_h);
    let mut card_pos: Vec<Placed> = Vec::new();
    // Cards stack in their own column top-down.
    let mut ccy = title_h + BOX_GAP;
    if !g.unknowns.is_empty() || !g.topics.is_empty() {
        ccy = cy0;
    }
    for c in &g.cards {
        let h = LINE_H + (c.lines.len() as i64) * LINE_H + PAD;
        card_pos.push(Placed {
            x: cx,
            y: ccy,
            w: card_w.max(topic_w),
            h,
        });
        ccy += h + BOX_GAP;
    }
    cy0 = ccy;

    let width =
        (cx + card_w.max(topic_w).max(unk_w) + 2 * PAD).max(x0 + left_w + 2 * PAD);
    let height = left_bottom.max(ty).max(cy0).max(uy) + 2 * LINE_H;

    // ---- emit
    let mut s = String::new();
    s.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" \
         height=\"{h}\" viewBox=\"0 0 {w} {h}\" font-family=\"monospace\" \
         font-size=\"13\">\n",
        w = width,
        h = height
    ));
    s.push_str(&format!(
        "<rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"#ffffff\"/>\n",
        width, height
    ));
    s.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" font-size=\"15\" font-weight=\"bold\" \
         fill=\"#1a202c\">{}</text>\n",
        x0,
        LINE_H,
        xml_escape(&g.title)
    ));
    s.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" fill=\"#718096\">{}</text>\n",
        x0,
        2 * LINE_H,
        xml_escape(&g.footer)
    ));

    // Edges first (under boxes).
    let find_fn = |a: &str| fn_anchor.iter().find(|(f, _, _, _)| f == a);
    let find_topic = |a: &str| {
        a.strip_prefix("topic:")
            .and_then(|t| topic_pos.iter().find(|(n, _)| n == t))
    };
    let find_unk = |a: &str| unk_pos.iter().find(|(n, _)| n == a);
    for e in &g.edges {
        let (color, dash) = match e.kind {
            EdgeKind::Publish => ("#2f855a", ""),
            EdgeKind::Subscribe => ("#6b46c1", " stroke-dasharray=\"6,4\""),
            EdgeKind::Call => ("#4a5568", ""),
            EdgeKind::CallViaStdlib => {
                ("#718096", " stroke-dasharray=\"6,4\"")
            }
            EdgeKind::Hole => ("#c53030", " stroke-dasharray=\"2,3\""),
        };
        let (color, width_attr) = if e.highlight {
            ("#d69e2e", " stroke-width=\"2.5\"")
        } else {
            (color, " stroke-width=\"1.5\"")
        };
        // Resolve endpoints to (x, y) pairs.
        let seg: Option<((i64, i64), (i64, i64), bool)> = match (
            find_fn(&e.from),
            find_topic(&e.from),
            find_fn(&e.to),
            find_topic(&e.to),
            find_unk(&e.to),
        ) {
            // fn -> topic (publish)
            (Some((_, rx, _, fy)), _, _, Some((_, p)), _) => {
                Some(((*rx, *fy), (p.x, p.y + p.h / 2), false))
            }
            // topic -> fn (subscribe): arrow back into the left column
            (_, Some((_, p)), Some((_, rx, _, fy)), _, _) => {
                Some(((p.x, p.y + p.h / 2), (*rx, *fy), false))
            }
            // fn -> unknown
            (Some((_, rx, _, fy)), _, _, _, Some((_, p))) => {
                Some(((*rx, *fy), (p.x, p.y + p.h / 2), false))
            }
            // fn -> fn (call): bulge left of the column
            (Some((_, _, lx, fy)), _, Some((_, _, lx2, ty2)), _, _) => {
                Some(((*lx, *fy), (*lx2, *ty2), true))
            }
            _ => None,
        };
        let Some(((x1, y1), (x2, y2), bulge_left)) = seg else {
            continue;
        };
        let bulge = (gutter - PAD).max(40);
        let (c1x, c2x) = if bulge_left {
            (x1 - bulge, x2 - bulge)
        } else {
            (x1 + (x2 - x1) / 2, x1 + (x2 - x1) / 2)
        };
        s.push_str(&format!(
            "<path d=\"M {x1} {y1} C {c1x} {y1}, {c2x} {y2}, {x2} {y2}\" \
             fill=\"none\" stroke=\"{color}\"{width_attr}{dash}/>\n"
        ));
        // Arrowhead: small triangle at the destination, pointing +x
        // (or -x for the leftward bulge return).
        let dir = if bulge_left || x2 >= x1 { 1 } else { -1 };
        s.push_str(&format!(
            "<path d=\"M {x2} {y2} l {} {} l 0 {} z\" fill=\"{color}\"/>\n",
            -7 * dir,
            -4,
            8
        ));
        // Edge label. Midpoints of opposing publish/subscribe pairs
        // collide, so each kind anchors at a different fraction of
        // the chord; call labels sit in the left gutter.
        let (mx, my) = if bulge_left {
            let bx = x1.min(x2) - bulge;
            (bx.max(PAD), (y1 + y2) / 2 - 4)
        } else {
            let frac_num: i64 = match e.kind {
                EdgeKind::Publish => 30,
                EdgeKind::Subscribe => 70,
                _ => 50,
            };
            let lx = x1 + (x2 - x1) * frac_num / 100
                - text_w(&e.label) / 2;
            let ly = y1 + (y2 - y1) * frac_num / 100 - 6;
            (lx, ly)
        };
        s.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-size=\"11\" fill=\"{}\">{}</text>\n",
            mx,
            my,
            color,
            xml_escape(&e.label)
        ));
    }

    // Locus boxes.
    for (b, p) in g.boxes.iter().zip(&box_pos) {
        let stroke = if b.highlight { "#d69e2e" } else { "#4a5568" };
        let sw = if b.highlight { 2.5 } else { 1.5 };
        s.push_str(&format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"6\" \
             fill=\"#f7fafc\" stroke=\"{}\" stroke-width=\"{}\"/>\n",
            p.x, p.y, p.w, p.h, stroke, sw
        ));
        s.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-weight=\"bold\" fill=\"#1a202c\">{}</text>\n",
            p.x + PAD,
            p.y + LINE_H - 4,
            xml_escape(&b.name)
        ));
        for (i, l) in b.lines.iter().enumerate() {
            let ly = p.y + LINE_H + (i as i64) * LINE_H + LINE_H - 5;
            let fill = if l.highlight { "#b7791f" } else { "#2d3748" };
            s.push_str(&format!(
                "<text x=\"{}\" y=\"{}\" fill=\"{}\">{}</text>\n",
                p.x + 2 * PAD,
                ly,
                fill,
                xml_escape(&l.label)
            ));
        }
    }

    // Topic nodes.
    for (t, (_, p)) in g.topics.iter().zip(&topic_pos) {
        let stroke = if t.highlight { "#d69e2e" } else { "#2b6cb0" };
        let sw = if t.highlight { 2.5 } else { 1.5 };
        s.push_str(&format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"14\" \
             fill=\"#ebf8ff\" stroke=\"{}\" stroke-width=\"{}\"/>\n",
            p.x, p.y, p.w, p.h, stroke, sw
        ));
        s.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-weight=\"bold\" fill=\"#2b6cb0\">{}</text>\n",
            p.x + PAD,
            p.y + LINE_H - 4,
            xml_escape(&t.name)
        ));
        s.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-size=\"11\" fill=\"#4a5568\">{}</text>\n",
            p.x + PAD,
            p.y + 2 * LINE_H - 6,
            xml_escape(&t.sub)
        ));
    }

    // Unknown nodes.
    for (u, (_, p)) in g.unknowns.iter().zip(&unk_pos) {
        s.push_str(&format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"4\" \
             fill=\"#fff5f5\" stroke=\"#c53030\" stroke-width=\"1.5\" \
             stroke-dasharray=\"4,3\"/>\n",
            p.x, p.y, p.w, p.h
        ));
        s.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" fill=\"#c53030\">{}</text>\n",
            p.x + PAD,
            p.y + LINE_H - 4,
            xml_escape(&u.label)
        ));
    }

    // Cards.
    for (c, p) in g.cards.iter().zip(&card_pos) {
        let (stroke, fill, tfill) = match c.tone {
            CardTone::Ok => ("#2f855a", "#f0fff4", "#22543d"),
            CardTone::Bad => ("#c53030", "#fff5f5", "#742a2a"),
            CardTone::Warn => ("#b7791f", "#fffff0", "#744210"),
        };
        s.push_str(&format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"6\" \
             fill=\"{}\" stroke=\"{}\" stroke-width=\"1.5\"/>\n",
            p.x, p.y, p.w, p.h, fill, stroke
        ));
        s.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-weight=\"bold\" fill=\"{}\">{}</text>\n",
            p.x + PAD,
            p.y + LINE_H - 4,
            tfill,
            xml_escape(&c.title)
        ));
        for (i, l) in c.lines.iter().enumerate() {
            s.push_str(&format!(
                "<text x=\"{}\" y=\"{}\" font-size=\"12\" fill=\"{}\">{}</text>\n",
                p.x + PAD,
                p.y + LINE_H + (i as i64 + 1) * LINE_H - 8,
                tfill,
                xml_escape(l)
            ));
        }
    }

    s.push_str("</svg>\n");
    s
}
