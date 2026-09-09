//! `hale model diff` — the semantic difference between two topology
//! artifacts (GH #527 B4, part of #521 Hale DNA).
//!
//! The diff is over ARTIFACTS, not in-process models: both inputs are
//! byte-reproducible, digest-verified documents a caller can store
//! as evidence, so the diff is replayable by anyone holding them.
//! Every input is admitted through the same gates the renderer
//! uses (top-level order, `artifact_digest`, `semantics`,
//! recomputed `shape_hash`, exact schema) — an edited or foreign
//! artifact is refused, never diffed.
//!
//! Matching is deterministic and NAMES what it compared. Two
//! declarations pair by (kind, name); a removed and an added
//! declaration of one kind pair as a rename only when each is the
//! other's UNIQUE shape-signature match; a split or join is
//! reported only for an exact partition of a locus's methods.
//! Anything with more than one candidate is reported as
//! `ambiguous` with its candidates, never guessed — INSPECTOR §8's
//! warning about joins that make wrong hypotheses feel evidenced.
//!
//! Output: one versioned document (`TOPOLOGY_DIFF_SCHEMA`) with
//! sections `declarations`, `contracts`, `effects`, `law` and a
//! `classification`; or a text rendering of the same rows.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Map, Value};

use crate::topology::{
    scan_top_level, verify_artifact_digest, verify_shape_hash,
    verify_top_level_order, MODEL_SEMANTICS, TOPOLOGY_SCHEMA,
};

/// The diff document's decoding contract. Moves when a section
/// changes meaning or a field becomes required.
pub const TOPOLOGY_DIFF_SCHEMA: &str = "1.0";

/// A topology artifact that passed every admission gate.
pub struct Admitted {
    pub path: String,
    pub v: Value,
    pub artifact_digest: String,
    pub shape_hash: String,
    /// Source-unit paths relative to the artifact's common source
    /// root, indexed like `sources`.
    pub units: Vec<String>,
}

fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

/// Admit one artifact: the renderer's gates, plus the sections this
/// diff reads.
pub fn admit(path: &str, raw: &str) -> Result<Admitted, String> {
    let top = scan_top_level(raw)
        .map_err(|e| format!("{path}: {e} — the verified and consumed values could disagree"))?;
    verify_top_level_order(&top).map_err(|e| format!("{path}: {e}"))?;
    match verify_artifact_digest(raw) {
        Some(true) => {}
        Some(false) => {
            return Err(format!(
                "{path}: artifact_digest does not match its contents — refusing to diff an edited or corrupted artifact"
            ))
        }
        None => return Err(format!("{path}: no artifact_digest — an unverifiable artifact is not diffable")),
    }
    let v: Value = serde_json::from_str(raw).map_err(|e| format!("{path}: not a JSON artifact: {e}"))?;
    let sem = v["semantics"].as_u64();
    if sem != Some(MODEL_SEMANTICS as u64) {
        return Err(format!(
            "{path}: model semantics {} — this build speaks {}; the rows may share a shape and mean different things",
            sem.map(|s| s.to_string()).unwrap_or_else(|| "absent".into()),
            MODEL_SEMANTICS
        ));
    }
    match verify_shape_hash(raw) {
        Some(true) => {}
        Some(false) => {
            return Err(format!("{path}: shape_hash does not recompute from the model half — the declared identity is stale"))
        }
        None => return Err(format!("{path}: no recomputable shape_hash")),
    }
    let schema = str_of(&v["schema"]);
    if schema != TOPOLOGY_SCHEMA {
        return Err(format!(
            "{path}: unsupported topology artifact schema `{schema}` — this build diffs exactly schema {TOPOLOGY_SCHEMA}; re-dump with the current compiler"
        ));
    }
    for (sect, ok) in [
        ("sorts.loci", v["sorts"]["loci"].is_array()),
        ("sorts.fns", v["sorts"]["fns"].is_array()),
        ("sorts.topics", v["sorts"]["topics"].is_array()),
        ("contracts", v["contracts"].is_array()),
        ("effects", v["effects"].is_object()),
        ("claims", v["claims"].is_array()),
        ("law.rows", v["law"]["rows"].is_array()),
        ("adequacy", v["adequacy"].is_object()),
        ("sources", v["sources"].is_array()),
        ("provenance.decls", v["provenance"]["decls"].is_object()),
    ] {
        if !ok {
            return Err(format!(
                "{path}: malformed artifact — {sect} missing or mistyped (digest verified, so this is a schema drift, not corruption)"
            ));
        }
    }
    // Source units, relative to the common root. Absolute paths
    // differ between two checkouts of one program; what "moved"
    // means is a different unit, so compare the part that names
    // the unit.
    let paths: Vec<String> = v["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| str_of(&s["path"]))
        .collect();
    let units = relative_units(&paths);
    Ok(Admitted {
        path: path.to_string(),
        artifact_digest: str_of(&v["artifact_digest"]),
        shape_hash: str_of(&v["shape_hash"]),
        v,
        units,
    })
}

fn relative_units(paths: &[String]) -> Vec<String> {
    if paths.is_empty() {
        return Vec::new();
    }
    let split: Vec<Vec<&str>> = paths.iter().map(|p| p.split('/').collect()).collect();
    // Common directory prefix, never eating the final component.
    let mut common = 0usize;
    'outer: loop {
        let Some(first) = split[0].get(common) else { break };
        for s in &split {
            if s.len() <= common + 1 || s[common] != *first {
                break 'outer;
            }
        }
        common += 1;
    }
    split.iter().map(|s| s[common..].join("/")).collect()
}

// ---------------------------------------------------------------
// Views over one admitted artifact
// ---------------------------------------------------------------

/// Where a declaration is: (unit, span) when the artifact places it.
fn decl_site(a: &Admitted, name: &str) -> Option<(String, [u64; 2])> {
    let d = a.v["provenance"]["decls"].get(name)?;
    let src = d["source"].as_i64()?;
    if src < 0 {
        return None;
    }
    let unit = a.units.get(src as usize)?.clone();
    let span = d["span"].as_array()?;
    Some((unit, [span.first()?.as_u64()?, span.get(1)?.as_u64()?]))
}

fn site_json(s: &Option<(String, [u64; 2])>) -> Value {
    match s {
        Some((u, sp)) => json!({"unit": u, "span": sp}),
        None => Value::Null,
    }
}

fn names(v: &Value) -> BTreeSet<String> {
    v.as_array().map(|a| a.iter().map(str_of).collect()).unwrap_or_default()
}

fn contract_of<'a>(a: &'a Admitted, locus: &str) -> Option<&'a Value> {
    a.v["contracts"].as_array()?.iter().find(|c| c["locus"] == locus)
}

fn local_method(full: &str) -> String {
    match full.rsplit_once("::") {
        Some((_, m)) => m.to_string(),
        None => full.to_string(),
    }
}

/// Locus renames accepted so far, a-name -> b-name. Facets and
/// signatures on the A side are read THROUGH it, so a rename does
/// not echo as a contract delta in every locus that mentions the
/// renamed one (a param type, a supervised child, a method owner,
/// a group member) — and a fn `Old::m` pairs with `New::m`.
pub type RenameMap = BTreeMap<String, String>;

fn via<'a>(map: &'a RenameMap, name: &'a str) -> &'a str {
    map.get(name).map(String::as_str).unwrap_or(name)
}

/// Rewrite every identifier token of `text` that names a renamed
/// declaration (locus or group) — for rendered claim forms, which
/// spell the names they quantify over.
fn rewrite_words(text: &str, map: &RenameMap) -> String {
    if map.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if !word.is_empty() {
            out.push_str(via(map, word));
            word.clear();
        }
    };
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            word.push(ch);
        } else {
            flush(&mut word, &mut out);
            out.push(ch);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// The name-independent contract facets of a locus, each rendered
/// as a sorted set of strings, keyed by facet name. Locus names
/// inside facets are read through `map`.
fn locus_facets(a: &Admitted, locus: &str, map: &RenameMap) -> BTreeMap<&'static str, BTreeSet<String>> {
    let mut out: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
    let Some(c) = contract_of(a, locus) else { return out };
    out.insert("sealed", [c["sealed"].as_bool().unwrap_or(false).to_string()].into());
    out.insert(
        "params",
        c["params"]
            .as_array()
            .map(|ps| ps.iter().map(|p| format!("{}: {}", str_of(&p["name"]), via(map, &str_of(&p["type"])))).collect())
            .unwrap_or_default(),
    );
    out.insert("methods", names(&c["methods"]).iter().map(|m| local_method(m)).collect());
    out.insert("publishes", names(&c["publishes"]));
    out.insert(
        "subscribes",
        c["subscribes"]
            .as_array()
            .map(|ss| {
                ss.iter()
                    .map(|s| {
                        let cap = match s["capacity"].as_u64() {
                            Some(n) => format!(" bounded({n}) {}", str_of(&s["shed"])),
                            None => String::new(),
                        };
                        format!("{} as {}{}", str_of(&s["topic"]), local_method(&str_of(&s["handler"])), cap)
                    })
                    .collect()
            })
            .unwrap_or_default(),
    );
    out.insert(
        "supervises",
        c["supervises"]
            .as_array()
            .map(|ss| {
                ss.iter()
                    .map(|s| {
                        let ops: Vec<String> = names(&s["ops"]).into_iter().collect();
                        let retry = match s["retry"].as_i64() {
                            Some(n) => format!(" retry {n}"),
                            None => String::new(),
                        };
                        format!("{} on {}: {}{}", via(map, &str_of(&s["child"])), str_of(&s["error"]), ops.join(" "), retry)
                    })
                    .collect()
            })
            .unwrap_or_default(),
    );
    let insts = c["instances"].as_array().cloned().unwrap_or_default();
    // Ownership and placement as WHO owns / WHERE, not by instance
    // path: the path is the owner's param name, which the owner's own
    // params facet already reports — carrying it here made a param
    // rename echo as two more rows (the kill-test reviewers read them
    // as noise).
    out.insert(
        "ownership",
        insts.iter().map(|i| format!("owned by {}", i["owner"].as_str().unwrap_or("nobody"))).collect(),
    );
    out.insert(
        "placement",
        insts.iter().map(|i| format!("on {}", i["domain"].as_str().unwrap_or("unplaced"))).collect(),
    );
    out
}

/// Rename signature: what a declaration IS apart from its name.
/// Locus names inside it are read through `map`.
fn signature(a: &Admitted, kind: &str, name: &str, map: &RenameMap) -> String {
    match kind {
        "locus" => {
            let f = locus_facets(a, name, map);
            // Ownership and placement carry instance PATHS, which
            // embed the name; a renamed locus keeps neither.
            f.iter()
                .filter(|(k, _)| **k != "ownership" && **k != "placement")
                .map(|(k, v)| format!("{k}={}", v.iter().cloned().collect::<Vec<_>>().join(",")))
                .collect::<Vec<_>>()
                .join(";")
        }
        "topic" => a.v["topics"]
            .as_array()
            .and_then(|ts| ts.iter().find(|t| t["name"] == name))
            .map(|t| format!("subject={} shape={}", str_of(&t["subject"]), str_of(&t["shape"])))
            .unwrap_or_default(),
        "fn" => {
            let owner = name.rsplit_once("::").map(|(o, _)| o).unwrap_or("");
            let eff: Vec<String> = names(&a.v["effects"][name]).into_iter().collect();
            let kind = str_of(&a.v["phases"][name]["kind"]);
            format!("owner={} kind={kind} effects={}", via(map, owner), eff.join(","))
        }
        "group" => names(&a.v["groups"][name])
            .iter()
            .map(|m| via(map, m).to_string())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(","),
        "claim" => a.v["claims"]
            .as_array()
            .and_then(|cs| cs.iter().find(|c| c["name"] == name))
            .map(|c| rewrite_words(&str_of(&c["form"]), map))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn universe(a: &Admitted, kind: &str) -> BTreeSet<String> {
    match kind {
        "locus" => names(&a.v["sorts"]["loci"]),
        "fn" => names(&a.v["sorts"]["fns"]),
        "topic" => names(&a.v["sorts"]["topics"]),
        "group" => a.v["groups"].as_object().map(|g| g.keys().cloned().collect()).unwrap_or_default(),
        "claim" => a.v["claims"]
            .as_array()
            .map(|cs| cs.iter().map(|c| str_of(&c["name"])).collect())
            .unwrap_or_default(),
        _ => BTreeSet::new(),
    }
}

const KINDS: &[&str] = &["locus", "topic", "fn", "group", "claim"];

// ---------------------------------------------------------------
// The diff
// ---------------------------------------------------------------

/// Pairs of (a-name, b-name) the contract/effect/law sections
/// compare: persisted names plus accepted renames.
struct Pairing {
    pairs: Vec<(String, String)>,
}

fn set_delta(a: &BTreeSet<String>, b: &BTreeSet<String>) -> (Vec<String>, Vec<String>) {
    (
        a.difference(b).cloned().collect(),
        b.difference(a).cloned().collect(),
    )
}

fn declarations(a: &Admitted, b: &Admitted) -> (Value, BTreeMap<&'static str, Pairing>, RenameMap) {
    let mut rows: Vec<Value> = Vec::new();
    let mut pairings: BTreeMap<&'static str, Pairing> = BTreeMap::new();
    // Loci are matched first; their accepted renames feed every
    // later kind (see `RenameMap`).
    let mut map: RenameMap = RenameMap::new();
    for kind in KINDS {
        let ua = universe(a, kind);
        let ub = universe(b, kind);
        let mut pairs: Vec<(String, String)> = Vec::new();
        // A method of a renamed locus pairs with its counterpart
        // under the new owner: `Old::m` -> `New::m`, when it exists.
        let mut carried_a: BTreeSet<String> = BTreeSet::new();
        let mut carried_b: BTreeSet<String> = BTreeSet::new();
        if *kind == "fn" {
            for n in &ua {
                let Some((owner, m)) = n.rsplit_once("::") else { continue };
                let Some(new_owner) = map.get(owner) else { continue };
                let cand = format!("{new_owner}::{m}");
                if ub.contains(&cand) && !ua.contains(&cand) {
                    rows.push(json!({
                        "kind": kind, "name": cand, "change": "renamed", "from": n,
                        "signature": format!("owner renamed {owner} -> {new_owner}"),
                        "a": site_json(&decl_site(a, n)), "b": site_json(&decl_site(b, &cand)),
                    }));
                    pairs.push((n.clone(), cand.clone()));
                    carried_a.insert(n.clone());
                    carried_b.insert(cand);
                }
            }
        }
        // persisted / moved
        for n in ua.intersection(&ub) {
            let sa = decl_site(a, n);
            let sb = decl_site(b, n);
            let moved = match (&sa, &sb) {
                (Some((ua_, _)), Some((ub_, _))) => ua_ != ub_,
                _ => false,
            };
            rows.push(json!({
                "kind": kind, "name": n,
                "change": if moved { "moved" } else { "persisted" },
                "a": site_json(&sa), "b": site_json(&sb),
            }));
            pairs.push((n.clone(), n.clone()));
        }
        let removed: BTreeSet<String> = ua.difference(&ub).filter(|n| !carried_a.contains(*n)).cloned().collect();
        let added: BTreeSet<String> = ub.difference(&ua).filter(|n| !carried_b.contains(*n)).cloned().collect();
        // renames: unique mutual signature matches. The A side is
        // read through the locus rename map; the B side as is.
        let none = RenameMap::new();
        let sig_a: BTreeMap<&String, String> = removed.iter().map(|n| (n, signature(a, kind, n, &map))).collect();
        let sig_b: BTreeMap<&String, String> = added.iter().map(|n| (n, signature(b, kind, n, &none))).collect();
        let mut renamed_from: BTreeSet<String> = BTreeSet::new();
        let mut renamed_to: BTreeSet<String> = BTreeSet::new();
        let mut ambiguous: BTreeSet<String> = BTreeSet::new();
        for r in &removed {
            let sr = &sig_a[r];
            if sr.is_empty() {
                continue;
            }
            let cands: Vec<&String> = added.iter().filter(|x| &sig_b[x] == sr).collect();
            match cands.len() {
                0 => {}
                1 => {
                    let x = cands[0];
                    let back: Vec<&String> = removed.iter().filter(|y| &sig_a[y] == sr).collect();
                    if back.len() == 1 {
                        rows.push(json!({
                            "kind": kind, "name": x, "change": "renamed", "from": r,
                            "signature": sr,
                            "a": site_json(&decl_site(a, r)), "b": site_json(&decl_site(b, x)),
                        }));
                        pairs.push((r.clone(), x.clone()));
                        renamed_from.insert(r.clone());
                        renamed_to.insert(x.clone());
                        if *kind == "locus" || *kind == "group" {
                            map.insert(r.clone(), x.clone());
                        }
                    } else {
                        rows.push(json!({
                            "kind": kind, "name": r, "change": "ambiguous",
                            "question": "rename", "candidates": cands,
                            "also_matching": back.iter().filter(|y| *y != &r).collect::<Vec<_>>(),
                            "a": site_json(&decl_site(a, r)),
                        }));
                        ambiguous.insert(r.clone());
                    }
                }
                _ => {
                    rows.push(json!({
                        "kind": kind, "name": r, "change": "ambiguous",
                        "question": "rename", "candidates": cands,
                        "a": site_json(&decl_site(a, r)),
                    }));
                    ambiguous.insert(r.clone());
                }
            }
        }
        // splits / joins: exact partitions of a locus's local methods
        let mut split_from: BTreeSet<String> = BTreeSet::new();
        let mut split_to: BTreeSet<String> = BTreeSet::new();
        if *kind == "locus" {
            let methods = |art: &Admitted, n: &str| -> BTreeSet<String> {
                locus_facets(art, n, &none).remove("methods").unwrap_or_default()
            };
            let free_a: Vec<&String> = removed.iter().filter(|n| !renamed_from.contains(*n) && !ambiguous.contains(*n)).collect();
            let free_b: Vec<&String> = added.iter().filter(|n| !renamed_to.contains(*n)).collect();
            let partition = |whole: &BTreeSet<String>, parts: &[(&String, BTreeSet<String>)]| -> bool {
                if parts.len() < 2 || whole.is_empty() {
                    return false;
                }
                let mut seen: BTreeSet<String> = BTreeSet::new();
                for (_, p) in parts {
                    if p.is_empty() || !p.is_subset(whole) || p.iter().any(|m| seen.contains(m)) {
                        return false;
                    }
                    seen.extend(p.iter().cloned());
                }
                seen == *whole
            };
            for r in &free_a {
                let whole = methods(a, r);
                let parts: Vec<(&String, BTreeSet<String>)> = free_b
                    .iter()
                    .map(|x| (*x, methods(b, x)))
                    .filter(|(_, m)| !m.is_empty() && m.is_subset(&whole))
                    .collect();
                if partition(&whole, &parts) {
                    let into: Vec<&String> = parts.iter().map(|(n, _)| *n).collect();
                    rows.push(json!({
                        "kind": kind, "name": r, "change": "split", "into": into,
                        "methods": whole,
                        "a": site_json(&decl_site(a, r)),
                        "b": into.iter().map(|n| site_json(&decl_site(b, n))).collect::<Vec<_>>(),
                    }));
                    split_from.insert(r.to_string());
                    split_to.extend(into.iter().map(|n| n.to_string()));
                }
            }
            for x in &free_b {
                if split_to.contains(*x) {
                    continue;
                }
                let whole = methods(b, x);
                let parts: Vec<(&String, BTreeSet<String>)> = free_a
                    .iter()
                    .filter(|n| !split_from.contains(**n))
                    .map(|n| (*n, methods(a, n)))
                    .filter(|(_, m)| !m.is_empty() && m.is_subset(&whole))
                    .collect();
                if partition(&whole, &parts) {
                    let from: Vec<&String> = parts.iter().map(|(n, _)| *n).collect();
                    rows.push(json!({
                        "kind": kind, "name": x, "change": "joined", "from": from,
                        "methods": whole,
                        "a": from.iter().map(|n| site_json(&decl_site(a, n))).collect::<Vec<_>>(),
                        "b": site_json(&decl_site(b, x)),
                    }));
                    split_to.insert(x.to_string());
                    split_from.extend(from.iter().map(|n| n.to_string()));
                }
            }
        }
        for r in &removed {
            if renamed_from.contains(r) || ambiguous.contains(r) || split_from.contains(r) {
                continue;
            }
            rows.push(json!({"kind": kind, "name": r, "change": "removed", "a": site_json(&decl_site(a, r))}));
        }
        for x in &added {
            if renamed_to.contains(x) || split_to.contains(x) {
                continue;
            }
            rows.push(json!({"kind": kind, "name": x, "change": "added", "b": site_json(&decl_site(b, x))}));
        }
        pairings.insert(kind, Pairing { pairs });
    }
    (Value::Array(rows), pairings, map)
}

fn topic_row<'a>(art: &'a Admitted, name: &str) -> Option<&'a Value> {
    art.v["topics"].as_array()?.iter().find(|t| t["name"] == name)
}

fn contracts(a: &Admitted, b: &Admitted, loci: &Pairing, topics: &Pairing, map: &RenameMap) -> Value {
    let none = RenameMap::new();
    let mut rows: Vec<Value> = Vec::new();
    // A paired topic whose wire subject or payload shape moved is a
    // contract change for every locus on it (the kill test's QueueStat
    // gained a field and the view said nothing).
    for (na, nb) in &topics.pairs {
        let (Some(ta), Some(tb)) = (topic_row(a, na), topic_row(b, nb)) else { continue };
        for facet in ["subject", "shape"] {
            if ta[facet] != tb[facet] {
                let mut row = Map::new();
                row.insert("topic".into(), json!(nb));
                if na != nb {
                    row.insert("from".into(), json!(na));
                }
                row.insert("facet".into(), json!(facet));
                row.insert("removed".into(), json!([str_of(&ta[facet])]));
                row.insert("added".into(), json!([str_of(&tb[facet])]));
                row.insert("a".into(), site_json(&decl_site(a, na)));
                row.insert("b".into(), site_json(&decl_site(b, nb)));
                rows.push(Value::Object(row));
            }
        }
    }
    for (na, nb) in &loci.pairs {
        let fa = locus_facets(a, na, map);
        let fb = locus_facets(b, nb, &none);
        for facet in ["sealed", "params", "methods", "publishes", "subscribes", "supervises", "ownership", "placement"] {
            let empty = BTreeSet::new();
            let (removed, added) = set_delta(fa.get(facet).unwrap_or(&empty), fb.get(facet).unwrap_or(&empty));
            if removed.is_empty() && added.is_empty() {
                continue;
            }
            let mut row = Map::new();
            row.insert("locus".into(), json!(nb));
            if na != nb {
                row.insert("from".into(), json!(na));
            }
            row.insert("facet".into(), json!(facet));
            row.insert("removed".into(), json!(removed));
            row.insert("added".into(), json!(added));
            row.insert("a".into(), site_json(&decl_site(a, na)));
            row.insert("b".into(), site_json(&decl_site(b, nb)));
            rows.push(Value::Object(row));
        }
    }
    Value::Array(rows)
}

fn effects(a: &Admitted, b: &Admitted, fns: &Pairing) -> Value {
    let mut rows: Vec<Value> = Vec::new();
    for (na, nb) in &fns.pairs {
        let ea = names(&a.v["effects"][na]);
        let eb = names(&b.v["effects"][nb]);
        let (dropped, gained) = set_delta(&ea, &eb);
        if dropped.is_empty() && gained.is_empty() {
            continue;
        }
        let mut row = Map::new();
        row.insert("fn".into(), json!(nb));
        if na != nb {
            row.insert("from".into(), json!(na));
        }
        row.insert("gained".into(), json!(gained));
        row.insert("dropped".into(), json!(dropped));
        rows.push(Value::Object(row));
    }
    // fn-grained certificates (`@effects` asserts, `only:`
    // complements, `@budget`): keyed by (subject, form).
    let certs = |art: &Admitted| -> BTreeMap<(String, String), String> {
        art.v["lowered"]
            .as_array()
            .map(|ls| {
                ls.iter()
                    .map(|l| ((str_of(&l["subject"]), str_of(&l["form"])), str_of(&l["result"])))
                    .collect()
            })
            .unwrap_or_default()
    };
    let ca = certs(a);
    let cb = certs(b);
    let mut cert_rows: Vec<Value> = Vec::new();
    for (k, ra) in &ca {
        match cb.get(k) {
            None => cert_rows.push(json!({"subject": k.0, "form": k.1, "change": "removed", "a": ra})),
            Some(rb) if rb != ra => {
                cert_rows.push(json!({"subject": k.0, "form": k.1, "change": "result", "a": ra, "b": rb}))
            }
            _ => {}
        }
    }
    for (k, rb) in &cb {
        if !ca.contains_key(k) {
            cert_rows.push(json!({"subject": k.0, "form": k.1, "change": "added", "b": rb}));
        }
    }
    json!({"classes": rows, "certificates": cert_rows})
}

fn law(a: &Admitted, b: &Admitted, claims: &Pairing, map: &RenameMap) -> Value {
    let claim = |art: &Admitted, n: &str| -> Option<Value> {
        art.v["claims"].as_array()?.iter().find(|c| c["name"] == n).cloned()
    };
    let law_row = |art: &Admitted, n: &str| -> Option<Value> {
        art.v["law"]["rows"].as_array()?.iter().find(|r| r["name"] == n).cloned()
    };
    let mut rows: Vec<Value> = Vec::new();
    for (na, nb) in &claims.pairs {
        let (Some(ca), Some(cb)) = (claim(a, na), claim(b, nb)) else { continue };
        let mut changes = Map::new();
        // A form that differs only by a renamed locus or group is
        // the same law.
        if rewrite_words(&str_of(&ca["form"]), map) != str_of(&cb["form"]) {
            changes.insert("form".into(), json!({"a": ca["form"], "b": cb["form"]}));
        }
        if ca["result"] != cb["result"] {
            changes.insert("result".into(), json!({"a": ca["result"], "b": cb["result"]}));
        }
        if let (Some(la), Some(lb)) = (law_row(a, na), law_row(b, nb)) {
            // the typed verdict; only its own row when it differs from
            // the surface result (they move together almost always)
            if la["verdict"] != lb["verdict"] && (la["verdict"] != ca["result"] || lb["verdict"] != cb["result"]) {
                changes.insert("verdict".into(), json!({"a": la["verdict"], "b": lb["verdict"]}));
            }
            if la["family"] != lb["family"] {
                changes.insert("family".into(), json!({"a": la["family"], "b": lb["family"]}));
            }
        }
        if changes.is_empty() {
            continue;
        }
        let mut row = Map::new();
        row.insert("claim".into(), json!(nb));
        if na != nb {
            row.insert("from".into(), json!(na));
        }
        row.insert("changes".into(), Value::Object(changes));
        rows.push(Value::Object(row));
    }
    let ua = universe(a, "claim");
    let ub = universe(b, "claim");
    let paired_a: BTreeSet<&String> = claims.pairs.iter().map(|(x, _)| x).collect();
    let paired_b: BTreeSet<&String> = claims.pairs.iter().map(|(_, y)| y).collect();
    for n in ua.iter().filter(|n| !paired_a.contains(n)) {
        let c = claim(a, n).unwrap_or(Value::Null);
        rows.push(json!({"claim": n, "change": "removed", "form": c["form"], "result": c["result"]}));
    }
    for n in ub.iter().filter(|n| !paired_b.contains(n)) {
        let c = claim(b, n).unwrap_or(Value::Null);
        rows.push(json!({"claim": n, "change": "added", "form": c["form"], "result": c["result"]}));
    }
    let mut adequacy: Vec<Value> = Vec::new();
    let aa = a.v["adequacy"].as_object().cloned().unwrap_or_default();
    let ab = b.v["adequacy"].as_object().cloned().unwrap_or_default();
    for fam in aa.keys().chain(ab.keys()).collect::<BTreeSet<_>>() {
        let x = aa.get(fam).cloned().unwrap_or(Value::Null);
        let y = ab.get(fam).cloned().unwrap_or(Value::Null);
        if x != y {
            adequacy.push(json!({"family": fam, "a": x, "b": y}));
        }
    }
    let verdict = if a.v["verdict"] != b.v["verdict"] {
        json!({"a": a.v["verdict"], "b": b.v["verdict"]})
    } else {
        Value::Null
    };
    json!({"claims": rows, "adequacy": adequacy, "verdict": verdict})
}

/// Diff two admitted artifacts into the versioned diff document.
pub fn diff(a: &Admitted, b: &Admitted) -> Value {
    let (decls, pairings, map) = declarations(a, b);
    let empty = Pairing { pairs: Vec::new() };
    let contracts = contracts(a, b, pairings.get("locus").unwrap_or(&empty), pairings.get("topic").unwrap_or(&empty), &map);
    let effects = effects(a, b, pairings.get("fn").unwrap_or(&empty));
    let law = law(a, b, pairings.get("claim").unwrap_or(&empty), &map);
    let identical = a.artifact_digest == b.artifact_digest;
    let shape_changed = a.shape_hash != b.shape_hash;
    let count = |v: &Value, key: &str, val: &str| -> usize {
        v.as_array().map(|r| r.iter().filter(|x| x[key] == val).count()).unwrap_or(0)
    };
    // `shape_hash` covers the model HALF (sorts, relations, endpoint
    // identity); payload shapes, params and the other contract facets
    // ride unhashed. A payload that gained a field with the hash
    // unmoved is a contract change, not "source-only" — the blind
    // round's frozen-schema case read as harmless under that label.
    let contract_changed = contracts.as_array().map(|r| !r.is_empty()).unwrap_or(false)
        || effects["classes"].as_array().map(|r| !r.is_empty()).unwrap_or(false);
    let classification = if identical {
        "identical"
    } else if shape_changed {
        "model-shape"
    } else if contract_changed {
        "contract"
    } else {
        "source-only"
    };
    json!({
        "schema": TOPOLOGY_DIFF_SCHEMA,
        "a": {"path": a.path, "artifact_digest": a.artifact_digest, "shape_hash": a.shape_hash, "schema": a.v["schema"]},
        "b": {"path": b.path, "artifact_digest": b.artifact_digest, "shape_hash": b.shape_hash, "schema": b.v["schema"]},
        "classification": classification,
        "renames": map,
        "matching": {
            "declarations": "by (kind, name); moved = a different source unit (paths relative to each artifact's common source root)",
            "order": "loci, then topics, fns, groups, claims; an accepted locus or group rename is read through wherever the name recurs (fn owners, group members, param types, supervised children, claim forms)",
            "renames": "a removed and an added declaration of one kind whose shape signatures match each other uniquely; more than one candidate is reported as ambiguous",
            "splits_joins": "loci only: an exact partition of the local method set across two or more added (split) or removed (joined) loci",
            "contracts": "per paired locus, facet by facet, as set differences",
        },
        "summary": {
            "added": count(&decls, "change", "added"),
            "removed": count(&decls, "change", "removed"),
            "renamed": count(&decls, "change", "renamed"),
            "moved": count(&decls, "change", "moved"),
            "split": count(&decls, "change", "split"),
            "joined": count(&decls, "change", "joined"),
            "ambiguous": count(&decls, "change", "ambiguous"),
            "contract_deltas": contracts.as_array().map(|r| r.len()).unwrap_or(0),
            "effect_deltas": effects["classes"].as_array().map(|r| r.len()).unwrap_or(0),
            "certificate_deltas": effects["certificates"].as_array().map(|r| r.len()).unwrap_or(0),
            "law_deltas": law["claims"].as_array().map(|r| r.len()).unwrap_or(0),
        },
        "declarations": decls,
        "contracts": contracts,
        "effects": effects,
        "law": law,
    })
}

fn site_text(v: &Value) -> String {
    match (v["unit"].as_str(), v["span"].as_array()) {
        (Some(u), Some(sp)) if sp.len() == 2 => format!("  ({u} bytes {}..{})", sp[0], sp[1]),
        _ => String::new(),
    }
}

fn list(v: &Value) -> String {
    names(v).into_iter().collect::<Vec<_>>().join(", ")
}

/// The text rendering: one line per row, the `+ / - / ~ / ! / ?`
/// vocabulary of a review view.
pub fn render_text(d: &Value) -> String {
    let mut o = String::new();
    o.push_str(&format!("hale model diff  {}  ->  {}\n", str_of(&d["a"]["path"]), str_of(&d["b"]["path"])));
    o.push_str(&format!(
        "classification: {}  (shape_hash {} -> {})\n",
        str_of(&d["classification"]),
        str_of(&d["a"]["shape_hash"]),
        str_of(&d["b"]["shape_hash"])
    ));
    o.push_str("legend: + added  - removed  ~ renamed  > moved  * split/joined  ? ambiguous  ! changed in place\n");
    o.push_str("classes: identical · source-only (no model or contract change) · contract (a contract or effect moved, shape_hash unmoved) · model-shape\n");
    let mut decl_lines: Vec<String> = Vec::new();
    for r in d["declarations"].as_array().map(|x| x.as_slice()).unwrap_or(&[]) {
        let kind = str_of(&r["kind"]);
        let name = str_of(&r["name"]);
        match str_of(&r["change"]).as_str() {
            "added" => decl_lines.push(format!("  + {kind} {name}{}", site_text(&r["b"]))),
            "removed" => decl_lines.push(format!("  - {kind} {name}{}", site_text(&r["a"]))),
            "renamed" => decl_lines.push(format!("  ~ {kind} {} -> {name}  (renamed; shape unchanged){}", str_of(&r["from"]), site_text(&r["b"]))),
            "moved" => decl_lines.push(format!(
                "  > {kind} {name}  moved {} -> {}",
                str_of(&r["a"]["unit"]),
                str_of(&r["b"]["unit"])
            )),
            "split" => decl_lines.push(format!("  * {kind} {name}  split into {}", list(&r["into"]))),
            "joined" => decl_lines.push(format!("  * {kind} {name}  joined from {}", list(&r["from"]))),
            "ambiguous" => decl_lines.push(format!(
                "  ? {kind} {name}  {} ambiguous: candidates {}",
                str_of(&r["question"]),
                list(&r["candidates"])
            )),
            _ => {}
        }
    }
    if !decl_lines.is_empty() {
        o.push_str("declarations:\n");
        for l in decl_lines {
            o.push_str(&l);
            o.push('\n');
        }
    }
    let contracts = d["contracts"].as_array().cloned().unwrap_or_default();
    if !contracts.is_empty() {
        o.push_str("contracts:\n");
        for r in &contracts {
            let mut parts: Vec<String> = Vec::new();
            for x in names(&r["removed"]) {
                parts.push(format!("-{x}"));
            }
            for x in names(&r["added"]) {
                parts.push(format!("+{x}"));
            }
            if r["topic"].is_string() {
                o.push_str(&format!("  ! topic {} {}: {}\n", str_of(&r["topic"]), str_of(&r["facet"]), parts.join("; ")));
            } else {
                o.push_str(&format!("  ! locus {} {}: {}\n", str_of(&r["locus"]), str_of(&r["facet"]), parts.join("; ")));
            }
        }
    }
    let classes = d["effects"]["classes"].as_array().cloned().unwrap_or_default();
    let certs = d["effects"]["certificates"].as_array().cloned().unwrap_or_default();
    if !classes.is_empty() || !certs.is_empty() {
        o.push_str("effects:\n");
        for r in &classes {
            let mut parts: Vec<String> = Vec::new();
            let g = list(&r["gained"]);
            if !g.is_empty() {
                parts.push(format!("gains {g}"));
            }
            let dr = list(&r["dropped"]);
            if !dr.is_empty() {
                parts.push(format!("drops {dr}"));
            }
            o.push_str(&format!("  ! fn {} {}\n", str_of(&r["fn"]), parts.join("; ")));
        }
        for r in &certs {
            match str_of(&r["change"]).as_str() {
                "added" => o.push_str(&format!("  + certificate {} {}  [{}]\n", str_of(&r["subject"]), str_of(&r["form"]), str_of(&r["b"]))),
                "removed" => o.push_str(&format!("  - certificate {} {}  [{}]\n", str_of(&r["subject"]), str_of(&r["form"]), str_of(&r["a"]))),
                _ => o.push_str(&format!(
                    "  ! certificate {} {}: {} -> {}\n",
                    str_of(&r["subject"]),
                    str_of(&r["form"]),
                    str_of(&r["a"]),
                    str_of(&r["b"])
                )),
            }
        }
    }
    let claims = d["law"]["claims"].as_array().cloned().unwrap_or_default();
    let adequacy = d["law"]["adequacy"].as_array().cloned().unwrap_or_default();
    if !claims.is_empty() || !adequacy.is_empty() || !d["law"]["verdict"].is_null() {
        o.push_str("law:\n");
        for r in &claims {
            match str_of(&r["change"]).as_str() {
                "added" => o.push_str(&format!("  + claim {}: {}  [{}]\n", str_of(&r["claim"]), str_of(&r["form"]), str_of(&r["result"]))),
                "removed" => o.push_str(&format!("  - claim {}: {}  [{}]\n", str_of(&r["claim"]), str_of(&r["form"]), str_of(&r["result"]))),
                _ => {
                    let mut parts: Vec<String> = Vec::new();
                    if let Some(ch) = r["changes"].as_object() {
                        for (k, v) in ch {
                            parts.push(format!("{k} {} -> {}", str_of(&v["a"]), str_of(&v["b"])));
                        }
                    }
                    o.push_str(&format!("  ! claim {}: {}\n", str_of(&r["claim"]), parts.join("; ")));
                }
            }
        }
        for r in &adequacy {
            o.push_str(&format!("  ! adequacy {}: {} -> {}\n", str_of(&r["family"]), str_of(&r["a"]), str_of(&r["b"])));
        }
        if !d["law"]["verdict"].is_null() {
            o.push_str(&format!("  ! verdict: {} -> {}\n", str_of(&d["law"]["verdict"]["a"]), str_of(&d["law"]["verdict"]["b"])));
        }
    }
    if d["summary"].as_object().map(|s| s.values().all(|v| v.as_u64() == Some(0))).unwrap_or(false) {
        o.push_str(if str_of(&d["classification"]) == "identical" { "no differences\n" } else { "no semantic differences\n" });
    }
    o
}

#[cfg(test)]
mod tests {
    use super::relative_units;

    #[test]
    fn units_are_relative_to_the_common_root() {
        let u = relative_units(&["/a/b/x/main.hl".into(), "/a/b/y/lib.hl".into()]);
        assert_eq!(u, vec!["x/main.hl", "y/lib.hl"]);
        let u = relative_units(&["/tmp/q/main.hl".into()]);
        assert_eq!(u, vec!["main.hl"]);
        let u = relative_units(&["/tmp/q/main.hl".into(), "/tmp/q/lib.hl".into()]);
        assert_eq!(u, vec!["main.hl", "lib.hl"]);
    }
}
