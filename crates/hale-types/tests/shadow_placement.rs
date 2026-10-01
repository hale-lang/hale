//! The first shadow (F.40 phase 0, step 0.5): two producers of the
//! `placement` family, run beside each other over the whole corpus.
//!
//! `check::compute_pool_of_locus_type` is the checker's answer to
//! "which pool does a locus type run in" (type-global: main's params
//! and nested fields, inheriting the owner's pool; a qualified type
//! path gets no row). `bus_graph::collect_subscriber_placements` is
//! the bus graph's answer to the same question (only the loci main's
//! placement entries name directly, keyed by the entry's last path
//! segment). The correspondence resolves the bus graph's names to
//! the checker's (a qualified stdlib path to its mangled name), so the
//! two producers are compared in one key space and a fixed checker
//! row would show as agreement, not as two one-sided rows. The
//! registry lists both under `placement` as legacy
//! producers; phase 1's placement lane replaces them with one table,
//! and this shadow is the divergence report that lane starts from.
//!
//! Only programs that check clean are shadowed: the compiler's rows
//! for a program it refuses decide nothing. A program is named by its
//! content (`hale_graph::shadow::program_id`), so the fixture does
//! not churn when a test file gains a literal above it.
//!
//! What this shadow cannot see: it parses each program alone, with
//! no stdlib merge, no imports and no desugar, which is not the
//! snapshot the compiler acts on. Phase 1.1's snapshot harness
//! replaces the parse.
//!
//! The gate: every divergence over the corpus is classified in
//! `fixtures/shadow_placement.txt` (known old bug, correction, spec
//! disagreement) with a note, and none is a regression. Regenerate
//! the fixture with `HALE_SHADOW_REGEN=1`, then classify by hand.

use std::collections::BTreeMap;
use std::path::PathBuf;

use hale_graph::shadow::{gate_message, parse_fixture, program_id, Report};
use hale_syntax::ast::{LocusMember, TopDecl, TypeExpr};
use hale_types::bus_graph::{collect_subscriber_placements, Placement};
use hale_types::check::{compute_pool_of_locus_type, PoolId};
use hale_types::stdlib_bodies::mangled_locus_name;
use hale_types::symbol::SourceFile;
use hale_types::Bundle;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/shadow_placement.txt")
}

/// The correspondence: both producers onto one rendering of "where
/// the type runs".
fn pool_key(p: &PoolId) -> String {
    match p {
        PoolId::Cooperative(name) if name == "main" => "main".into(),
        PoolId::Cooperative(name) => format!("pool:{name}"),
        PoolId::Pinned(_) => "pinned".into(),
    }
}
fn placement_key(p: &Placement) -> String {
    match p {
        Placement::SameThread => "main".into(),
        Placement::CrossPool(name) => format!("pool:{name}"),
        Placement::Pinned => "pinned".into(),
    }
}

/// A written locus type, as the checker names it: a qualified stdlib
/// path (`std::io::tcp::Listener`) resolves to its mangled name
/// (`__StdIoTcpListener`); a single segment is itself.
fn checker_name(segments: &[&str]) -> String {
    if segments.len() > 1 {
        if let Some(m) = mangled_locus_name(segments) {
            return m.to_string();
        }
    }
    segments.last().map(|s| s.to_string()).unwrap_or_default()
}

/// The correspondence's table for one program: the last path segment
/// of every locus-typed params field, as the bus graph keys it, to
/// the name the checker keys the same type by. Built from the AST,
/// the same fields both producers read.
fn bus_graph_to_checker_names(program: &hale_syntax::ast::Program) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for item in &program.items {
        let TopDecl::Locus(l) = item else { continue };
        for member in &l.members {
            let LocusMember::Params(pb) = member else { continue };
            for p in &pb.params {
                let Some(TypeExpr::Named { path, .. }) = &p.ty else { continue };
                let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
                let Some(last) = segs.last() else { continue };
                map.insert(last.to_string(), checker_name(&segs));
            }
        }
    }
    map
}

/// What the source says about a locus type: where fields of that
/// type are declared (and under which locus), and which placement
/// entries name those fields. Read from the text, since neither
/// producer keeps provenance on its rows.
fn witnesses(src: &str, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::from("<top level>");
    let mut fields: Vec<(String, String, usize)> = Vec::new();
    for (i, line) in src.lines().enumerate() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("main locus ").or_else(|| t.strip_prefix("locus ")) {
            current = rest.split(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or("").to_string();
        }
        // `name: Type = ...` or `name: a::b::Type = ...`
        if let Some((name, rest)) = t.split_once(':') {
            let name = name.trim();
            let ty = rest.trim().split(|c: char| c == '=' || c == ';' || c == ' ').next().unwrap_or("");
            let segs: Vec<&str> = ty.split("::").collect();
            let resolved = checker_name(&segs);
            let last = segs.last().copied().unwrap_or(ty);
            if (resolved == key || last == key) && name.chars().all(|c| c.is_alphanumeric() || c == '_') && !name.is_empty() {
                fields.push((current.clone(), name.to_string(), i + 1));
                out.push(format!("field `{name}: {ty}` declared in locus `{current}` (line {})", i + 1));
            }
        }
    }
    for (i, line) in src.lines().enumerate() {
        let t = line.trim();
        for (_, name, _) in &fields {
            if t.starts_with(&format!("{name}:")) && (t.contains("pinned") || t.contains("cooperative")) {
                out.push(format!("placement entry `{}` (line {})", t.trim_end_matches(';'), i + 1));
            }
        }
    }
    if out.is_empty() {
        out.push(format!("no field of type `{key}` is declared by name in this program (a qualified or generated type)"));
    }
    out
}

/// What depends on the row: the subjects the locus subscribes, whose
/// `direct_call_eligible` gate reads the bus graph's placement, and
/// the F.31 cross-pool check, which reads the checker's.
fn slice(src: &str, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    let mut depth = 0i32;
    for line in src.lines() {
        let t = line.trim();
        if !inside {
            if t.starts_with(&format!("locus {key} ")) || t.starts_with(&format!("locus {key}{{")) || t.starts_with(&format!("main locus {key} ")) {
                inside = true;
                depth = 0;
            } else {
                continue;
            }
        }
        depth += t.matches('{').count() as i32 - t.matches('}').count() as i32;
        if let Some(rest) = t.strip_prefix("subscribe ") {
            let subject = rest.split(|c: char| c == ' ' || c == ';').next().unwrap_or("");
            out.push(format!("dispatch gate of subject `{subject}` (direct_call_eligible reads the bus graph's placement of `{key}`)"));
        }
        if depth <= 0 && inside && t.contains('}') {
            break;
        }
    }
    out.push(format!("F.31 cross-pool method check for `{key}` (reads the checker's map)"));
    out
}

fn shadow_one(report: &mut Report, origin: &str, src: &str) {
    let Ok(program) = hale_syntax::parse_source(src) else { return };
    if hale_types::check_program(&program).iter().any(|d| d.is_error()) {
        return;
    }
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), &program);
    let mut bundle = Bundle::new(programs);
    bundle.sources = vec![SourceFile {
        id: 0,
        path: "app.hl".to_string(),
        digest: "0".to_string(),
        base: 0,
        len: src.len() as u32,
    }];
    let (top, _diags) = hale_types::resolve::build_top_scope(&bundle);
    let entry = hale_types::entry::entry_row(&bundle);
    let old: Vec<(String, String)> = compute_pool_of_locus_type(&bundle, &top, &entry)
        .iter()
        .map(|(k, v)| (k.clone(), pool_key(v)))
        .collect();
    let mut new: Vec<(String, String)> = collect_subscriber_placements(&bundle)
        .iter()
        .map(|(k, v)| (k.clone(), placement_key(v)))
        .collect();
    // The correspondence's one explicit rule: the bus graph records a
    // row only for a locus a placement entry names, and defines
    // `SameThread` as "no placement entry at all", so for every type
    // the checker knows, no row IS the bus graph's answer `main`.
    // (Keys only the bus graph has stay as they are: the checker has
    // no row for a qualified locus type, and that is a divergence.)
    let names_for_fill = bus_graph_to_checker_names(&program);
    for (k, _) in &old {
        let present = new
            .iter()
            .any(|(n, _)| names_for_fill.get(n).map(|r| r == k).unwrap_or(n == k));
        if !present {
            new.push((k.clone(), "main".to_string()));
        }
    }
    // The correspondence: the checker keys a type by its resolved name
    // (a stdlib locus by its mangled name), the bus graph by the last
    // segment of the path the field wrote. Both onto the checker's.
    let names = bus_graph_to_checker_names(&program);
    let id = program_id(origin, src);
    report.compare_rows(
        &id,
        &old,
        &new,
        |k| Some(k.clone()),
        |k| Some(names.get(k).cloned().unwrap_or_else(|| k.clone())),
        |k| witnesses(src, k),
        |k| slice(src, k),
    );
}

#[test]
fn the_two_placement_producers_agree_or_every_divergence_is_classified() {
    let mut report = Report::new("placement");
    for p in hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok()) {
        shadow_one(&mut report, &p.origin, &p.source);
    }
    assert!(report.programs > 300, "the corpus walk is vacuous ({} programs)", report.programs);
    let path = fixture_path();
    let existing = std::fs::read_to_string(&path)
        .ok()
        .map(|t| parse_fixture(&t).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
        .unwrap_or_default();
    if std::env::var("HALE_SHADOW_REGEN").as_deref() == Ok("1") {
        std::fs::write(&path, report.render_fixture(&existing)).expect("write fixture");
        eprintln!("{}", report.render());
        return;
    }
    let (unexplained, stale) = report.explain(&existing);
    assert!(
        unexplained.is_empty() && stale.is_empty(),
        "{}",
        gate_message(&report, &unexplained, &stale, "crates/hale-types/tests/fixtures/shadow_placement.txt")
    );
}
