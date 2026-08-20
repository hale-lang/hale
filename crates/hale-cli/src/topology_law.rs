//! The CLOSED external law decoder + the ONE schema-1.11 law-account
//! admission routine (GH #476 Change 6, rounds 4–5).
//!
//! `validate_law_account` is shared by Track A rendering and fleet
//! composition — there is exactly one definition of "admitted
//! schema-1.11 artifact". It decodes every `law.rows[*].law`
//! payload into a typed vocabulary with EXACT object shapes
//! (unknown fields rejected), retains every operand, validates all
//! references against the artifact's own catalogs (fn universe,
//! loci, groups, topics, phases, seeds, effect classes), binds
//! annotation laws to their certificate/budget evidence, binds the
//! compatibility `claims` rows to their typed law in both
//! directions, recomputes per-row certificate verdicts and the
//! document verdict, and recomputes adequacy from the positive
//! capability account.

use serde_json::Value;
use std::collections::BTreeSet;

// ------------------------------------------------------------------
// primitive decoders (exact shapes; unknown fields rejected)
// ------------------------------------------------------------------

fn only_keys(
    v: &Value,
    what: &str,
    required: &[&str],
    optional: &[&str],
) -> Result<(), String> {
    let obj = v
        .as_object()
        .ok_or_else(|| format!("{}: must be an object", what))?;
    for k in obj.keys() {
        if !required.contains(&k.as_str())
            && !optional.contains(&k.as_str())
        {
            return Err(format!(
                "{}: unknown field `{}`",
                what, k
            ));
        }
    }
    for k in required {
        if !obj.contains_key(*k) {
            return Err(format!(
                "{}: missing field `{}`",
                what, k
            ));
        }
    }
    Ok(())
}

fn span_ok(v: &Value) -> bool {
    v.as_array().is_some_and(|a| {
        a.len() == 2
            && a[0].as_u64().is_some()
            && a[1].as_u64().is_some()
            && a[0].as_u64() <= a[1].as_u64()
    })
}

/// A decoded reference: raw canonical identity + display spelling +
/// resolution status (+ provenance when carried).
pub struct Ref {
    pub name: String,
    pub display: String,
    pub resolved: bool,
}

fn decode_ref(v: &Value, what: &str) -> Result<Ref, String> {
    only_keys(
        v,
        what,
        &["name", "display", "resolved"],
        &["file", "span"],
    )?;
    let name = v["name"]
        .as_str()
        .ok_or_else(|| format!("{}: name must be a string", what))?;
    let display = v["display"]
        .as_str()
        .ok_or_else(|| format!("{}: display must be a string", what))?;
    let resolved = v["resolved"]
        .as_bool()
        .ok_or_else(|| format!("{}: resolved must be a bool", what))?;
    if v.get("file").is_some() != v.get("span").is_some() {
        return Err(format!(
            "{}: file and span come together",
            what
        ));
    }
    if let Some(f) = v.get("file") {
        if !f.is_string() || !span_ok(&v["span"]) {
            return Err(format!(
                "{}: malformed provenance",
                what
            ));
        }
    }
    Ok(Ref {
        name: name.to_string(),
        display: display.to_string(),
        resolved,
    })
}

pub struct ClassRef {
    pub class: String,
    pub builtin: bool,
    pub resolved: bool,
}

pub struct Grant {
    pub publish: bool,
    pub topic: Ref,
}

pub enum SetRef {
    Group(Ref),
    Effects(ClassRef),
}

pub enum Dim {
    Builtin(&'static str),
    UserClass(ClassRef),
}

pub struct Selector {
    pub name: String,
    /// Candidate TOPICS the selector matched — canonical
    /// (raw, display) pairs, validated against the topic catalog
    /// and REQUIRED to equal the set recomputed from it with the
    /// compiler's own matching rule (`hale_model::bus_ref_matches`).
    pub topics: Vec<(String, String)>,
    /// Candidate wire-subject patterns, same contract against the
    /// subject catalog.
    pub subjects: Vec<String>,
}

/// The closed law vocabulary — every variant retains its operands.
pub enum Law {
    ForbidReaches {
        src: SetRef,
        dst: SetRef,
        via_calls: bool,
        via_bus: bool,
        during: Option<Ref>,
        avoiding: Option<Ref>,
    },
    OnlyEdges {
        src: Ref,
        dst: Ref,
        grants: Vec<Grant>,
    },
    Bound {
        class: ClassRef,
        limit: u64,
        from: Ref,
    },
    RequireEndpoint {
        publishers: bool,
        group: Ref,
        topic: Ref,
    },
    RequireSealed {
        group: Ref,
    },
    RequireAttributed {
        class: ClassRef,
    },
    Cover {
        seed: Ref,
        group: Ref,
    },
    Count {
        publishers: bool,
        topic: Ref,
        cmp: &'static str,
        n: u64,
    },
    EffectForbid {
        at: Ref,
        classes: Vec<ClassRef>,
    },
    EffectOnly {
        at: Ref,
        classes: Vec<ClassRef>,
    },
    EffectPublishSet {
        at: Ref,
        entries: Vec<Selector>,
    },
    EffectCauses {
        at: Ref,
        classes: Vec<ClassRef>,
    },
    NoPanic {
        at: Ref,
    },
    DependsSet {
        locus: Ref,
        entries: Vec<Selector>,
    },
    PhaseEffects {
        locus: Ref,
        phases: Vec<(String, Vec<ClassRef>)>,
    },
    AllocBudget {
        at: Ref,
        per_call: u64,
    },
    QuantBudget {
        at: Ref,
        dim: Dim,
        limit: u64,
    },
}

/// The artifact's own catalogs, against which every resolved
/// reference must exist. Round 6: entity catalogs carry CANONICAL
/// (raw name, display) pairs — the raw half is the machine join
/// key, and a resolved reference must match one exact pair
/// (cross-row consistency alone cannot anchor a singleton
/// reference).
pub struct RefContext {
    pub groups: Vec<(String, String)>,
    /// (raw, display, wire subject).
    pub topics: Vec<(String, String, String)>,
    /// The wire-subject pattern universe.
    pub subjects: Vec<String>,
    pub fn_universe: Vec<(String, String)>,
    /// Function-grain analysis coverage (rounds 10–11): the three
    /// states are TYPED — `analyzed` (body walked), `summarized`
    /// (a behavior-summary row exists; this set IS `sorts.fns`,
    /// the hashed anchor), and the failure-handler kind
    /// (executable, never walked).
    pub fn_analyzed: BTreeSet<String>,
    pub fn_summarized: BTreeSet<String>,
    pub fn_failure: BTreeSet<String>,
    /// The legacy analyzable universe (`sorts.fns`) — the old
    /// engines never saw subjects outside it, so a certificate row
    /// on such a subject carries no report and judges
    /// `uncertified` (or `invalid` when statically invalid).
    pub sorts_fns: Vec<String>,
    /// (raw, display, analyzable) — loci carry the engine-walk
    /// discriminator: module-scoped loci are in every sort but
    /// outside the legacy certificate walk.
    pub loci: Vec<(String, String, bool)>,
    pub phases: Vec<String>,
    pub seeds: Vec<String>,
    /// (name, declared, cyclic) from `law.effect_classes`.
    pub classes: Vec<(String, bool, bool)>,
    /// Raw-name <-> display consistency ledger: one canonical
    /// identity keeps ONE spelling across the whole law section,
    /// and one spelling names ONE identity — the raw half is the
    /// machine join key, so a `name` swap under an unchanged
    /// `display` must not survive admission.
    seen: std::cell::RefCell<(
        std::collections::BTreeMap<String, String>,
        std::collections::BTreeMap<String, String>,
    )>,
}

impl RefContext {
    pub fn from_artifact(v: &Value) -> Result<Self, String> {
        let pairs = |x: &Value,
                     what: &str|
         -> Result<Vec<(String, String)>, String> {
            let mut out = Vec::new();
            for e in x.as_array().ok_or_else(|| {
                format!("{} must be an array", what)
            })? {
                only_keys(e, what, &["name", "display"], &[])?;
                out.push((
                    e["name"]
                        .as_str()
                        .ok_or_else(|| {
                            format!("{}: name must be a string", what)
                        })?
                        .to_string(),
                    e["display"]
                        .as_str()
                        .ok_or_else(|| {
                            format!(
                                "{}: display must be a string",
                                what
                            )
                        })?
                        .to_string(),
                ));
            }
            Ok(out)
        };
        let mut classes = Vec::new();
        for c in v["law"]["effect_classes"]
            .as_array()
            .ok_or("law.effect_classes must be an array")?
        {
            only_keys(
                c,
                "law.effect_classes[*]",
                &["name", "declared", "cyclic"],
                &[],
            )?;
            classes.push((
                c["name"]
                    .as_str()
                    .ok_or("class name must be a string")?
                    .to_string(),
                c["declared"]
                    .as_bool()
                    .ok_or("declared must be a bool")?,
                c["cyclic"]
                    .as_bool()
                    .ok_or("cyclic must be a bool")?,
            ));
        }
        let phases: Vec<String> = v["phases"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(_, p)| {
                p["phase"].as_str().map(|s| s.to_string())
            })
            .chain(
                ["birth", "accept", "release", "run", "drain",
                 "dissolve"]
                .iter()
                .map(|s| s.to_string()),
            )
            .collect();
        // Topic catalog rows carry the wire subject too.
        let mut topics: Vec<(String, String, String)> = Vec::new();
        for t in v["law"]["topics"].as_array().ok_or(
            "law.topics must be an array of canonical topic rows",
        )? {
            only_keys(
                t,
                "law.topics[*]",
                &["name", "display", "subject"],
                &[],
            )?;
            topics.push((
                t["name"]
                    .as_str()
                    .ok_or("law.topics[*].name must be a string")?
                    .to_string(),
                t["display"]
                    .as_str()
                    .ok_or(
                        "law.topics[*].display must be a string",
                    )?
                    .to_string(),
                t["subject"]
                    .as_str()
                    .ok_or(
                        "law.topics[*].subject must be a string",
                    )?
                    .to_string(),
            ));
        }
        let subjects: Vec<String> = v["law"]["subjects"]
            .as_array()
            .ok_or("law.subjects must be an array")?
            .iter()
            .map(|x| {
                x.as_str().map(|s| s.to_string()).ok_or_else(
                    || {
                        "law.subjects entries must be strings"
                            .to_string()
                    },
                )
            })
            .collect::<Result<_, _>>()?;
        let cx = RefContext {
            groups: pairs(&v["law"]["groups"], "law.groups")?,
            topics,
            subjects,
            fn_universe: {
                let mut out = Vec::new();
                for e in v["law"]["fn_universe"].as_array().ok_or(
                    "law.fn_universe must be an array of \
                     canonical rows",
                )? {
                    only_keys(
                        e,
                        "law.fn_universe[*]",
                        &["name", "display", "analyzed",
                          "summarized", "kind"],
                        &[],
                    )?;
                    let analyzed = e["analyzed"]
                        .as_bool()
                        .ok_or("law.fn_universe[*].analyzed")?;
                    let summarized = e["summarized"]
                        .as_bool()
                        .ok_or("law.fn_universe[*].summarized")?;
                    let kind = e["kind"]
                        .as_str()
                        .ok_or("law.fn_universe[*].kind")?;
                    if !matches!(
                        kind,
                        "hook" | "method" | "free" | "mode"
                            | "failure"
                    ) {
                        return Err(format!(
                            "law.fn_universe[*].kind `{}` is \
                             outside the closed vocabulary",
                            kind
                        ));
                    }
                    // Coverage laws (rounds 11–12): a summarized
                    // body was walked — and a WALKED body is
                    // summarized (the summary enumerates exactly
                    // the walked set), which anchors the analyzed
                    // bit to the HASHED `sorts.fns`: upgrading an
                    // unanalyzed body to certifiable would have to
                    // change the hashed half.
                    if analyzed && !summarized {
                        return Err(format!(
                            "law.fn_universe: `{}` is analyzed \
                             but not summarized — the walked set \
                             is the summary set",
                            e["display"].as_str().unwrap_or("?")
                        ));
                    }
                    if summarized && !analyzed {
                        return Err(format!(
                            "law.fn_universe: `{}` is summarized \
                             but not analyzed",
                            e["display"].as_str().unwrap_or("?")
                        ));
                    }
                    if kind == "failure" && analyzed {
                        return Err(format!(
                            "law.fn_universe: failure handler \
                             `{}` marked analyzed",
                            e["display"].as_str().unwrap_or("?")
                        ));
                    }
                    out.push((
                        e["name"]
                            .as_str()
                            .ok_or("law.fn_universe[*].name")?
                            .to_string(),
                        e["display"]
                            .as_str()
                            .ok_or("law.fn_universe[*].display")?
                            .to_string(),
                    ));
                }
                out
            },
            fn_analyzed: v["law"]["fn_universe"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|e| e["analyzed"] == true)
                .filter_map(|e| {
                    e["display"].as_str().map(|s| s.to_string())
                })
                .collect(),
            fn_summarized: v["law"]["fn_universe"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|e| e["summarized"] == true)
                .filter_map(|e| {
                    e["display"].as_str().map(|s| s.to_string())
                })
                .collect(),
            fn_failure: v["law"]["fn_universe"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|e| e["kind"] == "failure")
                .filter_map(|e| {
                    e["display"].as_str().map(|s| s.to_string())
                })
                .collect(),
            sorts_fns: v["sorts"]["fns"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect(),
            loci: {
                let mut out = Vec::new();
                for e in v["law"]["loci"].as_array().ok_or(
                    "law.loci must be an array of canonical rows",
                )? {
                    only_keys(
                        e,
                        "law.loci[*]",
                        &["name", "display", "analyzable"],
                        &[],
                    )?;
                    out.push((
                        e["name"]
                            .as_str()
                            .ok_or("law.loci[*].name")?
                            .to_string(),
                        e["display"]
                            .as_str()
                            .ok_or("law.loci[*].display")?
                            .to_string(),
                        e["analyzable"]
                            .as_bool()
                            .ok_or("law.loci[*].analyzable")?,
                    ));
                }
                out
            },
            phases,
            seeds: v["seeds"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, _)| k.clone())
                .collect(),
            classes,
            seen: std::cell::RefCell::new((
                std::collections::BTreeMap::new(),
                std::collections::BTreeMap::new(),
            )),
        };
        // Round 7: the catalogs are CLOSED, not extensible. Each
        // must be unique in both halves and stand in exact
        // bijection with the section other consumers join on —
        // otherwise selector recomputation is circular (a ghost
        // topic appended to the catalog widens the law universe
        // underneath the certificate).
        let unique_pairs = |pairs: &[(String, String)],
                            what: &str|
         -> Result<(), String> {
            let mut raws = BTreeSet::new();
            let mut disps = BTreeSet::new();
            for (n, d) in pairs {
                if !raws.insert(n) {
                    return Err(format!(
                        "{}: duplicate raw identity `{}`",
                        what, n
                    ));
                }
                if !disps.insert(d) {
                    return Err(format!(
                        "{}: duplicate display `{}`",
                        what, d
                    ));
                }
            }
            Ok(())
        };
        unique_pairs(&cx.fn_universe, "law.fn_universe")?;
        {
            let mut raws = BTreeSet::new();
            let mut disps = BTreeSet::new();
            for (n, d, _) in &cx.loci {
                if !raws.insert(n) || !disps.insert(d) {
                    return Err(format!(
                        "law.loci: duplicate identity `{}`",
                        d
                    ));
                }
            }
        }
        unique_pairs(&cx.groups, "law.groups")?;
        {
            let mut raws = BTreeSet::new();
            let mut disps = BTreeSet::new();
            for (n, d, _) in &cx.topics {
                if !raws.insert(n) || !disps.insert(d) {
                    return Err(format!(
                        "law.topics: duplicate identity `{}`",
                        d
                    ));
                }
            }
            let mut names = BTreeSet::new();
            for (n, _, _) in &cx.classes {
                if !names.insert(n) {
                    return Err(format!(
                        "law.effect_classes: duplicate class `{}`",
                        n
                    ));
                }
            }
            let mut pats = BTreeSet::new();
            for p2 in &cx.subjects {
                if !pats.insert(p2) {
                    return Err(format!(
                        "law.subjects: duplicate pattern `{}`",
                        p2
                    ));
                }
            }
        }
        // topics ↔ topics section: exact bijection on
        // (display, subject).
        let section_topics: Vec<(&str, &str)> = v["topics"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| {
                (
                    t["name"].as_str().unwrap_or(""),
                    t["subject"].as_str().unwrap_or(""),
                )
            })
            .collect();
        if section_topics.len() != cx.topics.len() {
            return Err(format!(
                "law.topics carries {} rows, the topics section \
                 {} — the catalogs are closed",
                cx.topics.len(),
                section_topics.len()
            ));
        }
        for (name, subject) in &section_topics {
            if !cx
                .topics
                .iter()
                .any(|(_, d, su)| d == name && su == subject)
            {
                return Err(format!(
                    "topics section row `{}` has no canonical \
                     law.topics pair",
                    name
                ));
            }
        }
        // groups ↔ groups section: exact bijection on display.
        let group_keys: Vec<&String> = v["groups"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, _)| k)
            .collect();
        if group_keys.len() != cx.groups.len() {
            return Err(format!(
                "law.groups carries {} rows, the groups section \
                 {} — the catalogs are closed",
                cx.groups.len(),
                group_keys.len()
            ));
        }
        for k in &group_keys {
            if !cx.groups.iter().any(|(_, d)| &d == k) {
                return Err(format!(
                    "groups section row `{}` has no canonical \
                     law.groups pair",
                    k
                ));
            }
        }
        // loci ↔ sorts.loci: exact bijection on display.
        let loci_sort: Vec<&str> = v["sorts"]["loci"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str())
            .collect();
        if loci_sort.len() != cx.loci.len() {
            return Err(format!(
                "law.loci carries {} rows, sorts.loci {} — the \
                 catalogs are closed",
                cx.loci.len(),
                loci_sort.len()
            ));
        }
        for name in &loci_sort {
            if !cx.loci.iter().any(|(_, d, _)| d == name) {
                return Err(format!(
                    "sorts.loci row `{}` has no canonical \
                     law.loci pair",
                    name
                ));
            }
        }
        // Rounds 10–11: the coverage account is validated at
        // FUNCTION grain against the hashed summary universe — the
        // SUMMARIZED subset of `law.fn_universe` must equal
        // `sorts.fns` exactly (the legacy fn sort IS the
        // behavior-summary key set; "walked" and "summarized" are
        // distinct typed states). No name-prefix inference.
        let sorts_set: BTreeSet<&String> =
            cx.sorts_fns.iter().collect();
        let summarized_set: BTreeSet<&String> =
            cx.fn_summarized.iter().collect();
        if sorts_set != summarized_set {
            let extra: Vec<_> =
                summarized_set.difference(&sorts_set).collect();
            let missing: Vec<_> =
                sorts_set.difference(&summarized_set).collect();
            return Err(format!(
                "the summarized function coverage does not equal \
                 the hashed summary universe (extra: {:?}, \
                 missing: {:?})",
                extra, missing
            ));
        }
        // Locus-grain: `analyzable` must agree with the member
        // coverage. `on_failure` handlers are executable but never
        // analyzed even on analyzable loci, so they are exempt; a
        // memberless locus has no member evidence (and no code, so
        // both phase shapes are vacuously truthful).
        for (_, disp, analyzable) in &cx.loci {
            let prefix = format!("{}::", disp);
            let members: Vec<&(String, String)> = cx
                .fn_universe
                .iter()
                .filter(|(_, d)| {
                    d.starts_with(&prefix)
                        && !cx.fn_failure.contains(d)
                })
                .collect();
            if members.is_empty() {
                // Memberless ⇒ VACUOUSLY analyzable (no body to
                // walk): the flag is fully recomputable, so a
                // module-scoped memberless contract cannot be
                // flipped in either direction.
                if !analyzable {
                    return Err(format!(
                        "law.loci: `{}` marks analyzable=false \
                         but it has no executable members — a \
                         memberless locus is vacuously analyzable",
                        disp
                    ));
                }
                continue;
            }
            let all_analyzed = members
                .iter()
                .all(|(_, d)| cx.fn_analyzed.contains(d));
            if all_analyzed != *analyzable {
                return Err(format!(
                    "law.loci: `{}` marks analyzable={} but its \
                     member coverage says {}",
                    disp, analyzable, all_analyzed
                ));
            }
        }
        // Round 8: the subject universe is validated against the
        // model's OWN typed endpoint projection — the `endpoints`
        // section carries every bus endpoint at wire-subject
        // grain, including a declared publisher with no send site,
        // which the narrower V1 site relations never show.
        // `law.subjects` must equal exactly the subjects the
        // endpoint and topic sections carry, in both directions —
        // an appended ghost pattern has no endpoint row, and a
        // deleted pattern orphans one.
        let mut model_subjects: BTreeSet<String> = cx
            .topics
            .iter()
            .map(|(_, _, su)| su.clone())
            .collect();
        // Round 9/10: the endpoint section is NOT a second
        // authority, and endpoint identity is TYPED — each row
        // carries its wire subject AND (when a declared topic
        // covers the end) the topic identity. A literal address
        // whose text collides with a topic display stays a
        // literal; declaredness is never inferred from strings.
        // Site rows must project exactly onto the V1 relations:
        // a topic-covered end appears there under the topic
        // display, a literal end under its own text.
        let decode_endpoint = |e: &Value,
                               what: &str|
         -> Result<(String, Option<String>), String> {
            let su = e["subject"].as_str().ok_or_else(|| {
                format!("{}: subject must be a string", what)
            })?;
            let topic = match e.get("topic") {
                None => None,
                Some(t) => {
                    let td = t.as_str().ok_or_else(|| {
                        format!(
                            "{}: topic must be a string",
                            what
                        )
                    })?;
                    let hit = cx
                        .topics
                        .iter()
                        .find(|(_, d, _)| d == td);
                    let Some((_, _, wire)) = hit else {
                        return Err(format!(
                            "{}: topic `{}` is not in this \
                             artifact",
                            what, td
                        ));
                    };
                    if wire != su {
                        return Err(format!(
                            "{}: subject `{}` disagrees with \
                             topic `{}`'s wire subject `{}`",
                            what, su, td, wire
                        ));
                    }
                    Some(td.to_string())
                }
            };
            Ok((su.to_string(), topic))
        };
        // Typed declares rows: (locus, subject, topic) — the
        // OWNER stays in the compared identity (round 12: a
        // declaration cannot move between loci while a
        // `require publishes(some G, topic T)` verdict rides on
        // the original owner).
        let mut declares: BTreeSet<(
            String,
            String,
            Option<String>,
        )> = BTreeSet::new();
        for (i, d) in v["declares_publish"]
            .as_array()
            .ok_or(
                "declares_publish must be an array of typed \
                 relation rows",
            )?
            .iter()
            .enumerate()
        {
            only_keys(
                d,
                "declares_publish[*]",
                &["locus", "subject"],
                &["topic", "file", "span"],
            )
            .map_err(|x| {
                format!("declares_publish[{}]: {}", i, x)
            })?;
            let locus = d["locus"].as_str().ok_or_else(|| {
                format!(
                    "declares_publish[{}]: locus must be a string",
                    i
                )
            })?;
            if !cx.loci.iter().any(|(_, disp, _)| disp == locus) {
                return Err(format!(
                    "declares_publish[{}]: locus `{}` is not in \
                     this artifact",
                    i, locus
                ));
            }
            let (su, topic) = decode_endpoint(
                d,
                &format!("declares_publish[{}]", i),
            )?;
            declares.insert((locus.to_string(), su, topic));
        }
        // Endpoint rows, split by (verb, via).
        let source_paths_cx: BTreeSet<String> = v["sources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| {
                s["path"].as_str().map(|x| x.to_string())
            })
            .collect();
        let source_by_id: std::collections::BTreeMap<
            i64,
            String,
        > = v["sources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| {
                Some((
                    s["id"].as_i64()?,
                    s["path"].as_str()?.to_string(),
                ))
            })
            .collect();
        // Round 11: site rows are LOSSLESS — (owner fn, site
        // ordinal, subject, topic). No two typed rows collapse
        // under the legacy display projection, and each must be
        // unique.
        let mut site_pub: BTreeSet<(
            String,
            u64,
            String,
            Option<String>,
        )> = BTreeSet::new();
        let mut decl_pub: BTreeSet<(
            String,
            String,
            Option<String>,
        )> = BTreeSet::new();
        let mut subs: BTreeSet<(
            String,
            u64,
            String,
            Option<String>,
        )> = BTreeSet::new();
        // (verb, owner, site) — exactly one occupant each; and
        // every site row's authored span, tied below to the
        // span-grained provenance account.
        let mut occupancy: BTreeSet<(bool, String, u64)> =
            BTreeSet::new();
        let mut pub_spans: Vec<(String, String, u64, u64)> =
            Vec::new();
        let mut sub_spans: Vec<(String, String, u64, u64)> =
            Vec::new();
        for (i, e) in v["endpoints"]
            .as_array()
            .ok_or(
                "endpoints must be an array of typed endpoint \
                 rows",
            )?
            .iter()
            .enumerate()
        {
            let what = format!("endpoints[{}]", i);
            let is_site_grain = matches!(
                (e["verb"].as_str(), e["via"].as_str()),
                (Some("publish"), Some("site"))
                    | (Some("subscribe"), Some("declaration"))
            );
            if is_site_grain {
                only_keys(
                    e,
                    "endpoints[*]",
                    &["verb", "subject", "via", "fn", "site"],
                    &["topic", "file", "span"],
                )
                .map_err(|x| format!("{}: {}", what, x))?;
            } else {
                only_keys(
                    e,
                    "endpoints[*]",
                    &["verb", "subject", "via", "locus"],
                    &["topic", "file", "span"],
                )
                .map_err(|x| format!("{}: {}", what, x))?;
            }
            if e.get("file").is_some() != e.get("span").is_some()
            {
                return Err(format!(
                    "{}: file and span come together",
                    what
                ));
            }
            if let Some(f) = e.get("file") {
                if !f.as_str().is_some_and(|p2| {
                    source_paths_cx.contains(p2)
                }) || !span_ok(&e["span"])
                {
                    return Err(format!(
                        "{}: location does not resolve to a \
                         known source",
                        what
                    ));
                }
            }
            let row = decode_endpoint(e, &what)?;
            model_subjects.insert(row.0.clone());
            match (e["verb"].as_str(), e["via"].as_str()) {
                (Some("publish"), Some("site"))
                | (Some("subscribe"), Some("declaration")) => {
                    let owner = e["fn"]
                        .as_str()
                        .ok_or_else(|| {
                            format!(
                                "{}: fn must be a string",
                                what
                            )
                        })?
                        .to_string();
                    if !cx
                        .fn_universe
                        .iter()
                        .any(|(_, d)| *d == owner)
                    {
                        return Err(format!(
                            "{}: fn `{}` is not in this artifact",
                            what, owner
                        ));
                    }
                    let site =
                        e["site"].as_u64().ok_or_else(|| {
                            format!(
                                "{}: site must be a number",
                                what
                            )
                        })?;
                    let is_pub = e["verb"] == "publish";
                    if !occupancy.insert((
                        is_pub,
                        owner.clone(),
                        site,
                    )) {
                        return Err(format!(
                            "{}: (verb, owner, site) already \
                             occupied",
                            what
                        ));
                    }
                    if let (Some(f), Some(sp)) = (
                        e["file"].as_str(),
                        e["span"].as_array(),
                    ) {
                        let rec = (
                            owner.clone(),
                            f.to_string(),
                            sp[0].as_u64().unwrap_or(0),
                            sp[1].as_u64().unwrap_or(0),
                        );
                        if is_pub {
                            pub_spans.push(rec);
                        } else {
                            sub_spans.push(rec);
                        }
                    }
                    let full =
                        (owner, site, row.0.clone(), row.1);
                    let fresh = if is_pub {
                        site_pub.insert(full)
                    } else {
                        subs.insert(full)
                    };
                    if !fresh {
                        return Err(format!(
                            "{}: duplicate typed endpoint row",
                            what
                        ));
                    }
                }
                (Some("publish"), Some("declaration")) => {
                    let locus = e["locus"]
                        .as_str()
                        .ok_or_else(|| {
                            format!(
                                "{}: locus must be a string",
                                what
                            )
                        })?
                        .to_string();
                    if !cx
                        .loci
                        .iter()
                        .any(|(_, disp, _)| *disp == locus)
                    {
                        return Err(format!(
                            "{}: locus `{}` is not in this \
                             artifact",
                            what, locus
                        ));
                    }
                    decl_pub.insert((locus, row.0, row.1));
                }
                _ => {
                    return Err(format!(
                        "{}: verb/via outside the closed \
                         vocabulary",
                        what
                    ));
                }
            }
        }
        // Declaration-publish endpoints ≡ the typed relation.
        if decl_pub != declares {
            return Err(format!(
                "the declaration-publish endpoints do not equal \
                 the declares_publish relation (endpoints: {:?}, \
                 relation: {:?})",
                decl_pub, declares
            ));
        }
        // Site endpoints project onto the V1 relations under
        // their OWN declared identity: topic-covered ends by the
        // topic display, literal ends by their text.
        // The image under the legacy projection must match the V1
        // relations at (owner, name) grain — AND the row COUNTS
        // must match the site-grained provenance section, which is
        // what makes the projection lossless: a typed site row
        // cannot disappear behind a colliding display (the V1
        // rows dedup; the provenance spans do not).
        let v1_name = |su: &str, topic: &Option<String>|
         -> String {
            topic.clone().unwrap_or_else(|| su.to_string())
        };
        let derived_pub: BTreeSet<(String, String)> = site_pub
            .iter()
            .map(|(f, _, su, t)| (f.clone(), v1_name(su, t)))
            .collect();
        let rel_pub: BTreeSet<(String, String)> = v["relations"]
            ["publishes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| {
                Some((
                    r["fn"].as_str()?.to_string(),
                    r["subject"].as_str()?.to_string(),
                ))
            })
            .collect();
        if derived_pub != rel_pub {
            return Err(format!(
                "the endpoints section does not project from the \
                 artifact's relations (publish endpoints: {:?}, \
                 relations: {:?})",
                derived_pub, rel_pub
            ));
        }
        let derived_sub: BTreeSet<(String, String)> = subs
            .iter()
            .map(|(f, _, su, t)| (f.clone(), v1_name(su, t)))
            .collect();
        let rel_sub: BTreeSet<(String, String)> = v["relations"]
            ["subscribes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| {
                Some((
                    format!(
                        "{}::{}",
                        r["locus"].as_str()?,
                        r["handler"].as_str()?
                    ),
                    r["subject"].as_str()?.to_string(),
                ))
            })
            .collect();
        if derived_sub != rel_sub {
            return Err(format!(
                "the endpoints section does not project from the \
                 artifact's relations (subscribe endpoints: {:?}, \
                 relations: {:?})",
                derived_sub, rel_sub
            ));
        }
        // Round 12: SPAN-multiset tie against the span-grained
        // provenance section — each typed site row is anchored to
        // its authored span, one-to-one, not merely counted.
        // Deleting or duplicating one of two colliding site rows
        // leaves the provenance spans behind.
        let prov_multiset = |rows: &Value,
                             owner: &dyn Fn(
            &Value,
        )
            -> Option<String>|
         -> Vec<(String, String, u64, u64)> {
            let mut out: Vec<(String, String, u64, u64)> = rows
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| {
                    let sp = r["span"].as_array()?;
                    Some((
                        owner(r)?,
                        source_by_id
                            .get(&r["source"].as_i64()?)?
                            .clone(),
                        sp[0].as_u64()?,
                        sp[1].as_u64()?,
                    ))
                })
                .collect();
            out.sort();
            out
        };
        let prov_pub = prov_multiset(
            &v["provenance"]["publishes"],
            &|r| r["fn"].as_str().map(|s| s.to_string()),
        );
        let mut ep_pub = pub_spans.clone();
        ep_pub.sort();
        if ep_pub != prov_pub {
            return Err(format!(
                "the site-publish endpoints do not match the \
                 provenance span account (endpoints: {:?}, \
                 provenance: {:?})",
                ep_pub, prov_pub
            ));
        }
        let prov_sub = prov_multiset(
            &v["provenance"]["subscribes"],
            &|r| {
                Some(format!(
                    "{}::{}",
                    r["locus"].as_str()?,
                    r["handler"].as_str()?
                ))
            },
        );
        let mut ep_sub = sub_spans.clone();
        ep_sub.sort();
        if ep_sub != prov_sub {
            return Err(format!(
                "the subscribe endpoints do not match the \
                 provenance span account (endpoints: {:?}, \
                 provenance: {:?})",
                ep_sub, prov_sub
            ));
        }
        let law_subjects: BTreeSet<String> =
            cx.subjects.iter().cloned().collect();
        if law_subjects != model_subjects {
            let extra: Vec<&String> =
                law_subjects.difference(&model_subjects).collect();
            let missing: Vec<&String> =
                model_subjects.difference(&law_subjects).collect();
            return Err(format!(
                "law.subjects does not equal the model's typed \
                 endpoint universe (extra: {:?}, missing: {:?})",
                extra, missing
            ));
        }
        Ok(cx)
    }

    fn exists_pair(
        &self,
        catalog: &[(String, String)],
        r: &Ref,
        what: &str,
    ) -> Result<(), String> {
        if r.resolved
            && !catalog
                .iter()
                .any(|(n, d)| *n == r.name && *d == r.display)
        {
            return Err(format!(
                "{}: resolved `{}` (raw `{}`) does not match any canonical pair in this artifact",
                what, r.display, r.name
            ));
        }
        self.bind(r, what)
    }

    /// Phases and seeds live in a raw==display domain — a resolved
    /// reference must be self-consistent AND cataloged.
    fn exists_flat(
        &self,
        catalog: &[String],
        r: &Ref,
        what: &str,
    ) -> Result<(), String> {
        if r.resolved
            && (r.name != r.display
                || !catalog.iter().any(|n| *n == r.display))
        {
            return Err(format!(
                "{}: resolved `{}` is not in this artifact",
                what, r.display
            ));
        }
        self.bind(r, what)
    }

    /// Record (and enforce) the raw<->display pairing for every
    /// decoded reference.
    fn bind(&self, r: &Ref, what: &str) -> Result<(), String> {
        let mut seen = self.seen.borrow_mut();
        match seen.0.get(&r.name) {
            Some(d) if d != &r.display => {
                return Err(format!(
                    "{}: raw identity `{}` appears under two spellings (`{}` and `{}`)",
                    what, r.name, d, r.display
                ));
            }
            Some(_) => {}
            None => {
                seen.0
                    .insert(r.name.clone(), r.display.clone());
            }
        }
        match seen.1.get(&r.display) {
            Some(n) if n != &r.name => {
                return Err(format!(
                    "{}: spelling `{}` names two raw identities (`{}` and `{}`)",
                    what, r.display, n, r.name
                ));
            }
            Some(_) => {}
            None => {
                seen.1
                    .insert(r.display.clone(), r.name.clone());
            }
        }
        Ok(())
    }
    fn group(&self, v: &Value, what: &str) -> Result<Ref, String> {
        let r = decode_ref(v, what)?;
        self.exists_pair(&self.groups, &r, what)?;
        Ok(r)
    }
    fn topic(&self, v: &Value, what: &str) -> Result<Ref, String> {
        let r = decode_ref(v, what)?;
        if r.resolved
            && !self
                .topics
                .iter()
                .any(|(n, d, _)| *n == r.name && *d == r.display)
        {
            return Err(format!(
                "{}: resolved `{}` (raw `{}`) does not match any canonical pair in this artifact",
                what, r.display, r.name
            ));
        }
        self.bind(&r, what)?;
        Ok(r)
    }
    fn function(
        &self,
        v: &Value,
        what: &str,
    ) -> Result<Ref, String> {
        let r = decode_ref(v, what)?;
        self.exists_pair(&self.fn_universe, &r, what)?;
        Ok(r)
    }
    fn locus(&self, v: &Value, what: &str) -> Result<Ref, String> {
        let r = decode_ref(v, what)?;
        if r.resolved
            && !self
                .loci
                .iter()
                .any(|(n, d, _)| *n == r.name && *d == r.display)
        {
            return Err(format!(
                "{}: resolved `{}` (raw `{}`) does not match any \
                 canonical pair in this artifact",
                what, r.display, r.name
            ));
        }
        self.bind(&r, what)?;
        Ok(r)
    }
    fn phase(&self, v: &Value, what: &str) -> Result<Ref, String> {
        let r = decode_ref(v, what)?;
        self.exists_flat(&self.phases, &r, what)?;
        Ok(r)
    }
    fn seed(&self, v: &Value, what: &str) -> Result<Ref, String> {
        let r = decode_ref(v, what)?;
        self.exists_flat(&self.seeds, &r, what)?;
        Ok(r)
    }
    fn class(
        &self,
        v: &Value,
        what: &str,
    ) -> Result<ClassRef, String> {
        only_keys(
            v,
            what,
            &["class", "builtin", "resolved"],
            &["file", "span"],
        )?;
        let class = v["class"].as_str().ok_or_else(|| {
            format!("{}: class must be a string", what)
        })?;
        let builtin = v["builtin"].as_bool().ok_or_else(|| {
            format!("{}: builtin must be a bool", what)
        })?;
        let resolved = v["resolved"].as_bool().ok_or_else(|| {
            format!("{}: resolved must be a bool", what)
        })?;
        if builtin != hale_model::is_builtin_effect_class(class) {
            return Err(format!(
                "{}: class `{}` mislabels its builtin status",
                what, class
            ));
        }
        if builtin && !resolved {
            return Err(format!(
                "{}: builtin class `{}` must be resolved",
                what, class
            ));
        }
        // A resolved USER class must exist in the class catalog.
        if !builtin
            && resolved
            && !self.classes.iter().any(|(n, _, _)| n == class)
        {
            return Err(format!(
                "{}: resolved class `{}` is not in this \
                 artifact's class catalog",
                what, class
            ));
        }
        Ok(ClassRef {
            class: class.to_string(),
            builtin,
            resolved,
        })
    }
    fn class_list(
        &self,
        v: &Value,
        what: &str,
    ) -> Result<Vec<ClassRef>, String> {
        v.as_array()
            .ok_or_else(|| format!("{}: must be an array", what))?
            .iter()
            .map(|c| self.class(c, what))
            .collect()
    }
    fn set(&self, v: &Value, what: &str) -> Result<SetRef, String> {
        only_keys(v, what, &[], &["group", "effects"])?;
        if v["group"].is_object() == v["effects"].is_object() {
            return Err(format!(
                "{}: exactly one of group|effects",
                what
            ));
        }
        if v["group"].is_object() {
            Ok(SetRef::Group(self.group(&v["group"], what)?))
        } else {
            Ok(SetRef::Effects(
                self.class(&v["effects"], what)?,
            ))
        }
    }
    fn selectors(
        &self,
        v: &Value,
        what: &str,
    ) -> Result<Vec<Selector>, String> {
        let mut out = Vec::new();
        for (i, s) in v
            .as_array()
            .ok_or_else(|| format!("{}: must be an array", what))?
            .iter()
            .enumerate()
        {
            let w = format!("{}[{}]", what, i);
            only_keys(
                s,
                &w,
                &["name", "topics", "subjects"],
                &["file", "span"],
            )?;
            let name = s["name"].as_str().ok_or_else(|| {
                format!("{}: name must be a string", w)
            })?;
            let mut cand_topics: Vec<(String, String)> = Vec::new();
            for t in s["topics"]
                .as_array()
                .ok_or_else(|| format!("{}: topics array", w))?
            {
                only_keys(t, &w, &["name", "display"], &[])?;
                let raw = t["name"].as_str().ok_or_else(|| {
                    format!("{}: candidate name", w)
                })?;
                let disp =
                    t["display"].as_str().ok_or_else(|| {
                        format!("{}: candidate display", w)
                    })?;
                // Candidate topics must be canonical pairs.
                if !self
                    .topics
                    .iter()
                    .any(|(n, d, _)| n == raw && d == disp)
                {
                    return Err(format!(
                        "{}: candidate topic `{}` does not match \
                         any canonical pair in this artifact",
                        w, disp
                    ));
                }
                cand_topics.push(
                    (raw.to_string(), disp.to_string()),
                );
            }
            let mut cand_subjects: Vec<String> = Vec::new();
            for x in s["subjects"]
                .as_array()
                .ok_or_else(|| format!("{}: subjects array", w))?
            {
                let pat = x.as_str().ok_or_else(|| {
                    format!("{}: subjects must be strings", w)
                })?;
                if !self.subjects.iter().any(|p| p == pat) {
                    return Err(format!(
                        "{}: candidate subject `{}` is not in \
                         this artifact's subject universe",
                        w, pat
                    ));
                }
                cand_subjects.push(pat.to_string());
            }
            // The candidate sets ARE the selector's normalized
            // meaning — and they are NOT free: recompute them from
            // the catalogs with the compiler's own matching rule
            // and require agreement. A candidate swap under an
            // unchanged selector name cannot survive.
            let sel = Selector {
                name: name.to_string(),
                topics: cand_topics,
                subjects: cand_subjects,
            };
            let mut expect_topics: Vec<(String, String)> = self
                .topics
                .iter()
                .filter(|(n, _, _)| {
                    hale_model::bus_ref_matches(&sel.name, n)
                })
                .map(|(n, d, _)| (n.clone(), d.clone()))
                .collect();
            expect_topics.sort();
            expect_topics.dedup();
            let mut got_topics = sel.topics.clone();
            got_topics.sort();
            got_topics.dedup();
            if got_topics != expect_topics {
                return Err(format!(
                    "{}: candidate topics do not match the set \
                     recomputed from the catalog for selector \
                     `{}`",
                    w, sel.name
                ));
            }
            let mut expect_subjects: Vec<String> = self
                .subjects
                .iter()
                .filter(|p| {
                    hale_model::bus_ref_matches(&sel.name, p)
                })
                .cloned()
                .collect();
            expect_subjects.sort();
            expect_subjects.dedup();
            let mut got_subjects = sel.subjects.clone();
            got_subjects.sort();
            got_subjects.dedup();
            if got_subjects != expect_subjects {
                return Err(format!(
                    "{}: candidate subjects do not match the set \
                     recomputed from the catalog for selector \
                     `{}`",
                    w, sel.name
                ));
            }
            out.push(sel);
        }
        Ok(out)
    }
}

/// Does the decoded law contain any UNRESOLVED reference? An
/// unresolved operand is lawful residue — but the judgment refuses
/// such a law (`invalid` / `uncertified`), so admission binds the
/// resolution state to the verdict: a `holds` row cannot carry an
/// unresolved operand (round 5 — flipping `resolved` to false to
/// bypass existence checks flips the verdict contract instead).
pub fn has_unresolved(law: &Law) -> bool {
    let r = |x: &Ref| !x.resolved;
    let c = |x: &ClassRef| !x.resolved;
    let s = |x: &SetRef| match x {
        SetRef::Group(g) => r(g),
        SetRef::Effects(e) => c(e),
    };
    match law {
        Law::ForbidReaches {
            src,
            dst,
            during,
            avoiding,
            ..
        } => {
            s(src)
                || s(dst)
                || during.as_ref().is_some_and(&r)
                || avoiding.as_ref().is_some_and(&r)
        }
        Law::OnlyEdges { src, dst, grants } => {
            r(src)
                || r(dst)
                || grants.iter().any(|g| r(&g.topic))
        }
        Law::Bound { class, from, .. } => c(class) || r(from),
        Law::RequireEndpoint { group, topic, .. } => {
            r(group) || r(topic)
        }
        Law::RequireSealed { group } => r(group),
        Law::RequireAttributed { class } => c(class),
        Law::Cover { seed, group } => r(seed) || r(group),
        Law::Count { topic, .. } => r(topic),
        Law::EffectForbid { at, classes }
        | Law::EffectOnly { at, classes }
        | Law::EffectCauses { at, classes } => {
            r(at) || classes.iter().any(&c)
        }
        Law::EffectPublishSet { at, .. } => r(at),
        Law::NoPanic { at } => r(at),
        Law::DependsSet { locus, .. } => r(locus),
        Law::PhaseEffects { locus, phases } => {
            r(locus)
                || phases
                    .iter()
                    .any(|(_, cs)| cs.iter().any(&c))
        }
        Law::AllocBudget { at, .. } => r(at),
        Law::QuantBudget { at, dim, .. } => {
            r(at)
                || matches!(dim, Dim::UserClass(u) if c(u))
        }
    }
}

/// Does the decoded law name a statically INVALID effect class —
/// an undeclared or cyclic user class, per the artifact's own
/// class catalog? Round 7: static invalidity DOMINATES every
/// replayed engine result; a law over an invalid class can only
/// truthfully judge `invalid`.
pub fn law_class_invalid(
    law: &Law,
    classes: &[(String, bool, bool)],
) -> bool {
    let bad = |c: &ClassRef| -> bool {
        !c.builtin
            && classes
                .iter()
                .find(|(n, _, _)| *n == c.class)
                .map_or(true, |(_, d, cy)| !d || *cy)
    };
    let set_bad = |s: &SetRef| match s {
        SetRef::Group(_) => false,
        SetRef::Effects(c) => bad(c),
    };
    match law {
        Law::ForbidReaches { src, dst, .. } => {
            set_bad(src) || set_bad(dst)
        }
        Law::Bound { class, .. }
        | Law::RequireAttributed { class } => bad(class),
        Law::EffectForbid { classes: cs, .. }
        | Law::EffectOnly { classes: cs, .. }
        | Law::EffectCauses { classes: cs, .. } => {
            cs.iter().any(&bad)
        }
        Law::PhaseEffects { phases, .. } => phases
            .iter()
            .flat_map(|(_, cs)| cs.iter())
            .any(&bad),
        Law::QuantBudget { dim, .. } => {
            matches!(dim, Dim::UserClass(c) if bad(c))
        }
        _ => false,
    }
}

/// Decode one payload against the closed vocabulary with EXACT
/// object shapes.
pub fn decode_law(
    law: &Value,
    cx: &RefContext,
) -> Result<Law, String> {
    match law["kind"].as_str() {
        Some("forbid_reaches") => {
            only_keys(
                law,
                "forbid_reaches",
                &["kind", "src", "dst", "via"],
                &["during", "avoiding"],
            )?;
            let via =
                law["via"].as_array().ok_or("via missing")?;
            if via.is_empty() {
                return Err("via must not be empty".to_string());
            }
            let mut via_calls = false;
            let mut via_bus = false;
            for v in via {
                match v.as_str() {
                    Some("calls") => via_calls = true,
                    Some("bus") => via_bus = true,
                    other => {
                        return Err(format!(
                            "via edge `{:?}` is not calls|bus",
                            other
                        ))
                    }
                }
            }
            let during = law
                .get("during")
                .map(|d| cx.phase(d, "during"))
                .transpose()?;
            let avoiding = law
                .get("avoiding")
                .map(|d| cx.group(d, "avoiding"))
                .transpose()?;
            Ok(Law::ForbidReaches {
                src: cx.set(&law["src"], "src")?,
                dst: cx.set(&law["dst"], "dst")?,
                via_calls,
                via_bus,
                during,
                avoiding,
            })
        }
        Some("only_edges") => {
            only_keys(
                law,
                "only_edges",
                &["kind", "src", "dst", "grants"],
                &[],
            )?;
            let mut grants = Vec::new();
            for (i, g) in law["grants"]
                .as_array()
                .ok_or("grants missing")?
                .iter()
                .enumerate()
            {
                only_keys(
                    g,
                    "grant",
                    &["verb", "topic"],
                    &[],
                )?;
                let publish = match g["verb"].as_str() {
                    Some("publish") => true,
                    Some("subscribe") => false,
                    other => {
                        return Err(format!(
                            "grants[{}]: verb `{:?}` is not \
                             publish|subscribe",
                            i, other
                        ))
                    }
                };
                grants.push(Grant {
                    publish,
                    topic: cx
                        .topic(&g["topic"], "grant topic")?,
                });
            }
            Ok(Law::OnlyEdges {
                src: cx.group(&law["src"], "src")?,
                dst: cx.group(&law["dst"], "dst")?,
                grants,
            })
        }
        Some("bound") => {
            only_keys(
                law,
                "bound",
                &["kind", "class", "limit", "from"],
                &[],
            )?;
            Ok(Law::Bound {
                class: cx.class(&law["class"], "class")?,
                limit: law["limit"]
                    .as_u64()
                    .ok_or("limit missing")?,
                from: cx.group(&law["from"], "from")?,
            })
        }
        Some("require_endpoint") => {
            only_keys(
                law,
                "require_endpoint",
                &["kind", "publishers", "group", "topic"],
                &[],
            )?;
            Ok(Law::RequireEndpoint {
                publishers: law["publishers"]
                    .as_bool()
                    .ok_or("publishers missing")?,
                group: cx.group(&law["group"], "group")?,
                topic: cx.topic(&law["topic"], "topic")?,
            })
        }
        Some("require_sealed") => {
            only_keys(
                law,
                "require_sealed",
                &["kind", "group"],
                &[],
            )?;
            Ok(Law::RequireSealed {
                group: cx.group(&law["group"], "group")?,
            })
        }
        Some("require_attributed") => {
            only_keys(
                law,
                "require_attributed",
                &["kind", "class"],
                &[],
            )?;
            Ok(Law::RequireAttributed {
                class: cx.class(&law["class"], "class")?,
            })
        }
        Some("cover") => {
            only_keys(
                law,
                "cover",
                &["kind", "seed", "group"],
                &[],
            )?;
            Ok(Law::Cover {
                seed: cx.seed(&law["seed"], "seed")?,
                group: cx.group(&law["group"], "group")?,
            })
        }
        Some("count") => {
            only_keys(
                law,
                "count",
                &["kind", "publishers", "topic", "cmp", "n"],
                &[],
            )?;
            let cmp = match law["cmp"].as_str() {
                Some("==") => "==",
                Some("<=") => "<=",
                Some(">=") => ">=",
                other => {
                    return Err(format!(
                        "cmp `{:?}` is not ==|<=|>=",
                        other
                    ))
                }
            };
            Ok(Law::Count {
                publishers: law["publishers"]
                    .as_bool()
                    .ok_or("publishers missing")?,
                topic: cx.topic(&law["topic"], "topic")?,
                cmp,
                n: law["n"].as_u64().ok_or("n missing")?,
            })
        }
        Some(k @ ("effect_forbid" | "effect_only" | "effect_causes")) => {
            only_keys(
                law,
                k,
                &["kind", "at", "classes"],
                &[],
            )?;
            let at = cx.function(&law["at"], "at")?;
            let classes =
                cx.class_list(&law["classes"], "classes")?;
            Ok(match k {
                "effect_forbid" => {
                    Law::EffectForbid { at, classes }
                }
                "effect_only" => Law::EffectOnly { at, classes },
                _ => Law::EffectCauses { at, classes },
            })
        }
        Some("effect_publish_set") => {
            only_keys(
                law,
                "effect_publish_set",
                &["kind", "at", "entries"],
                &[],
            )?;
            Ok(Law::EffectPublishSet {
                at: cx.function(&law["at"], "at")?,
                entries: cx
                    .selectors(&law["entries"], "entries")?,
            })
        }
        Some("no_panic") => {
            only_keys(law, "no_panic", &["kind", "at"], &[])?;
            Ok(Law::NoPanic {
                at: cx.function(&law["at"], "at")?,
            })
        }
        Some("depends_set") => {
            only_keys(
                law,
                "depends_set",
                &["kind", "locus", "entries"],
                &[],
            )?;
            Ok(Law::DependsSet {
                locus: cx.locus(&law["locus"], "locus")?,
                entries: cx
                    .selectors(&law["entries"], "entries")?,
            })
        }
        Some("phase_effects") => {
            only_keys(
                law,
                "phase_effects",
                &["kind", "locus", "phases"],
                &[],
            )?;
            let mut phases = Vec::new();
            for (i, p) in law["phases"]
                .as_array()
                .ok_or("phases missing")?
                .iter()
                .enumerate()
            {
                only_keys(
                    p,
                    "phase entry",
                    &["phase", "allowed"],
                    &[],
                )?;
                let name =
                    p["phase"].as_str().ok_or_else(|| {
                        format!(
                            "phases[{}]: phase must be a string",
                            i
                        )
                    })?;
                phases.push((
                    name.to_string(),
                    cx.class_list(&p["allowed"], "allowed")?,
                ));
            }
            Ok(Law::PhaseEffects {
                locus: cx.locus(&law["locus"], "locus")?,
                phases,
            })
        }
        Some("alloc_budget") => {
            only_keys(
                law,
                "alloc_budget",
                &["kind", "at", "per_call"],
                &[],
            )?;
            Ok(Law::AllocBudget {
                at: cx.function(&law["at"], "at")?,
                per_call: law["per_call"]
                    .as_u64()
                    .ok_or("per_call missing")?,
            })
        }
        Some("quant_budget") => {
            only_keys(
                law,
                "quant_budget",
                &["kind", "at", "dim", "limit"],
                &[],
            )?;
            let dimv = &law["dim"];
            let dim = if let Some(b) = dimv["builtin"].as_str() {
                only_keys(dimv, "dim", &["builtin"], &[])?;
                match b {
                    "stack_bytes" => Dim::Builtin("stack_bytes"),
                    "block_points" => {
                        Dim::Builtin("block_points")
                    }
                    "publish" => Dim::Builtin("publish"),
                    "fanout" => Dim::Builtin("fanout"),
                    other => {
                        return Err(format!(
                            "dim `{}` is not a quantitative \
                             dimension",
                            other
                        ))
                    }
                }
            } else if dimv["user_class"].is_object() {
                only_keys(dimv, "dim", &["user_class"], &[])?;
                Dim::UserClass(
                    cx.class(&dimv["user_class"], "dim")?,
                )
            } else {
                return Err(
                    "dim must be a builtin tag or a user-class \
                     reference"
                        .to_string(),
                );
            };
            Ok(Law::QuantBudget {
                at: cx.function(&law["at"], "at")?,
                dim,
                limit: law["limit"]
                    .as_u64()
                    .ok_or("limit missing")?,
            })
        }
        Some(
            "fleet_forbid_reaches"
            | "fleet_only_edges"
            | "fleet_require_endpoint"
            | "fleet_count_instances",
        ) => Err(
            "fleet law rows are not admissible in an application \
             artifact (the fleet account is Change 7's)"
                .to_string(),
        ),
        other => Err(format!(
            "law kind `{:?}` is not in the closed vocabulary",
            other
        )),
    }
}

pub fn family_of(law: &Law) -> &'static str {
    match law {
        Law::ForbidReaches { .. } => "reachability",
        Law::OnlyEdges { .. } => "boundary",
        Law::RequireEndpoint { .. }
        | Law::RequireSealed { .. }
        | Law::RequireAttributed { .. }
        | Law::Cover { .. }
        | Law::Count { .. } => "endpoint",
        Law::Bound { .. } => "bound",
        Law::EffectForbid { .. }
        | Law::EffectOnly { .. }
        | Law::EffectPublishSet { .. }
        | Law::NoPanic { .. }
        | Law::PhaseEffects { .. } => "certificate",
        Law::EffectCauses { .. }
        | Law::DependsSet { .. }
        | Law::AllocBudget { .. }
        | Law::QuantBudget { .. } => "unmigrated",
    }
}

/// Canonically re-render the compatibility `claims` form.
pub fn render_claims_form(law: &Law) -> Option<String> {
    let set = |s: &SetRef| -> String {
        match s {
            SetRef::Group(g) => g.display.clone(),
            SetRef::Effects(c) => format!("effects({})", c.class),
        }
    };
    Some(match law {
        Law::ForbidReaches {
            src,
            dst,
            via_calls,
            via_bus,
            during,
            avoiding,
        } => {
            let mut out = format!(
                "forbid reaches({}, {})",
                set(src),
                set(dst)
            );
            match (via_calls, via_bus) {
                (true, true) => {}
                (true, false) => out.push_str(" via { calls }"),
                (false, true) => out.push_str(" via { bus }"),
                (false, false) => {}
            }
            if let Some(p) = during {
                out.push_str(&format!(" during {}", p.display));
            }
            if let Some(a) = avoiding {
                out.push_str(&format!(" avoiding {}", a.display));
            }
            out
        }
        Law::OnlyEdges { src, dst, grants } => {
            let gs: Vec<String> = grants
                .iter()
                .map(|g| {
                    format!(
                        "{} {}",
                        if g.publish {
                            "publish"
                        } else {
                            "subscribe"
                        },
                        g.topic.display
                    )
                })
                .collect();
            format!(
                "only edges {} -> {} {{ {} }}",
                src.display,
                dst.display,
                gs.join("; ")
            )
        }
        Law::Bound { class, limit, from } => format!(
            "bound {} <= {} on paths from {}",
            class.class, limit, from.display
        ),
        Law::RequireEndpoint {
            publishers,
            group,
            topic,
        } => format!(
            "require {}(some {}, topic {})",
            if *publishers { "publishes" } else { "subscribes" },
            group.display,
            topic.display
        ),
        Law::RequireSealed { group } => {
            format!("require sealed(all {})", group.display)
        }
        Law::RequireAttributed { class } => {
            format!("require attributed(all {})", class.class)
        }
        Law::Cover { seed, group } => format!(
            "cover topic in seed({}): subscribed_by(some {})",
            seed.display, group.display
        ),
        Law::Count {
            publishers,
            topic,
            cmp,
            n,
        } => format!(
            "count {}(topic {}) {} {}",
            if *publishers {
                "publishers"
            } else {
                "subscribers"
            },
            topic.display,
            cmp,
            n
        ),
        _ => return None,
    })
}

/// The certificate forms the row's typed law generates — MUST
/// match the row's `certs[*].form` exactly (round 5: an operand
/// mutation cannot keep the old evidence).
pub fn expected_cert_forms(law: &Law) -> Option<Vec<String>> {
    Some(match law {
        Law::EffectForbid { at, classes } => classes
            .iter()
            .map(|c| {
                format!(
                    "forbid reaches({{{}}}, effects({}))",
                    at.display, c.class
                )
            })
            .collect(),
        Law::EffectOnly { at, classes } => {
            let s = classes
                .iter()
                .map(|c| c.class.clone())
                .collect::<Vec<_>>()
                .join(", ");
            vec![format!(
                "only effects {{{}}} on {{{}}}",
                s, at.display
            )]
        }
        Law::EffectPublishSet { at, entries } => {
            let s = entries
                .iter()
                .map(|e| e.name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            vec![format!(
                "only publishes {{{}}} from {{{}}}",
                s, at.display
            )]
        }
        Law::NoPanic { at } => vec![format!(
            "forbid reaches({{{}}}, panic)",
            at.display
        )],
        Law::PhaseEffects { locus, phases } => phases
            .iter()
            .map(|(ph, allowed)| {
                let s = allowed
                    .iter()
                    .map(|c| c.class.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "only effects {{{}}} on {{{}}} during {}",
                    s, locus.display, ph
                )
            })
            .collect(),
        _ => return None,
    })
}

/// The compatibility `lowered` form a BUDGET law generates —
/// binding `per_call` / `limit` and the dimension to the
/// legacy-engine evidence row.
pub fn expected_budget_form(law: &Law) -> Option<String> {
    match law {
        Law::AllocBudget { at, per_call } => Some(format!(
            "bound alloc <= {} on paths from {{{}}}",
            per_call, at.display
        )),
        Law::QuantBudget { at, dim, limit } => {
            let d = match dim {
                Dim::Builtin(b) => (*b).to_string(),
                Dim::UserClass(c) => c.class.clone(),
            };
            Some(format!(
                "bound {} <= {} on paths from {{{}}}",
                d, limit, at.display
            ))
        }
        _ => None,
    }
}

/// Round 6: the LEGACY-report fingerprint the unmigrated
/// non-budget families generate — must byte-match
/// `ClaimRow::legacy_form` (the emitter's spelling).
pub fn expected_legacy_form(law: &Law) -> Option<String> {
    match law {
        Law::EffectCauses { at, classes } => {
            let cs: Vec<&str> = classes
                .iter()
                .map(|c| c.class.as_str())
                .collect();
            Some(format!(
                "causes {{{}}} from {{{}}}",
                cs.join(", "),
                at.display
            ))
        }
        Law::DependsSet { locus, entries } => {
            let es: Vec<&str> = entries
                .iter()
                .map(|e| e.name.as_str())
                .collect();
            Some(format!(
                "depends {{{}}} on {{{}}}",
                es.join(", "),
                locus.display
            ))
        }
        _ => None,
    }
}

/// The subject spelling a row's lowered evidence carries.
fn expected_subject(law: &Law) -> Option<String> {
    match law {
        Law::EffectForbid { at, .. }
        | Law::EffectOnly { at, .. }
        | Law::EffectPublishSet { at, .. }
        | Law::NoPanic { at }
        | Law::AllocBudget { at, .. }
        | Law::QuantBudget { at, .. } => Some(at.display.clone()),
        Law::PhaseEffects { locus, .. } => {
            Some(locus.display.clone())
        }
        _ => None,
    }
}

fn sev(v: &str) -> u8 {
    match v {
        "holds" => 0,
        "uncertified" => 1,
        "violated" => 2,
        _ => 3,
    }
}

/// THE schema-1.11 law-account admission — shared by Track A and
/// fleet composition; `label` names the artifact in errors.
pub fn validate_law_account(
    v: &Value,
    label: &str,
) -> Result<(), String> {
    const FAMILIES: &[&str] = &[
        "reachability",
        "boundary",
        "endpoint",
        "bound",
        "certificate",
        "unmigrated",
    ];
    const VERDICTS: &[&str] =
        &["holds", "violated", "uncertified", "invalid"];
    if !v["law"].is_object()
        || !v["law"]["law_digest"].is_string()
        || !v["law"]["inputs_digest"].is_string()
        || !v["law"]["rows"].is_array()
        || !v["law"]["issues"].is_array()
    {
        return Err(format!(
            "{}: malformed artifact — law section incomplete",
            label
        ));
    }
    // Round 7: the digests are RECOMPUTED, never shape-checked.
    // `law_digest` is the canonical-JSON fingerprint over the law
    // rows (serde_json's BTreeMap rendering, fnv1a64) — editing a
    // row while keeping a stale digest refuses. `inputs_digest`
    // must match THIS binary's analysis-inputs digest: evidence
    // produced under a different stdlib/analysis snapshot is
    // refused, not silently trusted (re-dump with the current
    // compiler).
    fn fnv1a64(bytes: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }
    let canon = serde_json::to_string(&serde_json::json!({
        "issues": v["law"]["issues"],
        "rows": v["law"]["rows"],
    }))
    .map_err(|e| format!("{}: {}", label, e))?;
    let expect_law_digest =
        format!("{:016x}", fnv1a64(canon.as_bytes()));
    if v["law"]["law_digest"].as_str()
        != Some(expect_law_digest.as_str())
    {
        return Err(format!(
            "{}: malformed artifact — law_digest does not \
             recompute from the law rows",
            label
        ));
    }
    let expect_inputs = format!(
        "{:016x}",
        hale_types::evidence::analysis_inputs_digest()
    );
    if v["law"]["inputs_digest"].as_str()
        != Some(expect_inputs.as_str())
    {
        return Err(format!(
            "{}: this artifact's evidence was produced under a \
             different analysis-inputs snapshot \
             (inputs_digest {} != current {}); re-dump with the \
             current compiler",
            label,
            v["law"]["inputs_digest"].as_str().unwrap_or("?"),
            expect_inputs
        ));
    }
    let cx = RefContext::from_artifact(v)
        .map_err(|e| format!("{}: {}", label, e))?;
    let origin_ok = |origin: &str, family: &str| -> bool {
        match family {
            "certificate" | "unmigrated" => origin == "annotation",
            _ => {
                origin == "main"
                    || origin == "library"
                    || origin.starts_with("library:")
                    || origin.starts_with("constitution:")
            }
        }
    };
    // Round 7: evidence objects are VALIDATED, not
    // presence-checked — every diagnostic must carry a non-empty
    // message, paired file/span provenance, and a file the
    // artifact's sources section knows.
    let source_paths: BTreeSet<String> = v["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            s["path"].as_str().map(|x| x.to_string())
        })
        .collect();
    let check_evidence = |ev: &Value,
                          what: &str|
     -> Result<usize, String> {
        let Some(items) = ev.as_array() else {
            return Err(format!("{}: must be an array", what));
        };
        for (k, d) in items.iter().enumerate() {
            only_keys(
                d,
                what,
                &["message"],
                &["file", "span"],
            )
            .map_err(|e| format!("{}[{}]: {}", what, k, e))?;
            if !d["message"]
                .as_str()
                .is_some_and(|m| !m.is_empty())
            {
                return Err(format!(
                    "{}[{}]: message must be a non-empty string",
                    what, k
                ));
            }
            if d.get("file").is_some() != d.get("span").is_some()
            {
                return Err(format!(
                    "{}[{}]: file and span come together",
                    what, k
                ));
            }
            if let Some(f) = d.get("file") {
                if !f
                    .as_str()
                    .is_some_and(|p2| source_paths.contains(p2))
                    || !span_ok(&d["span"])
                {
                    return Err(format!(
                        "{}[{}]: location does not resolve to a \
                         known source",
                        what, k
                    ));
                }
            }
        }
        Ok(items.len())
    };
    // Round 9: the LAW-SELECTION account. Issues are validated
    // like every diagnostic, and a non-empty account fails the
    // document — a duplicate-name or constitution failure cannot
    // disappear between checking and projection.
    let issue_count =
        check_evidence(&v["law"]["issues"], "law issue")
            .map_err(|e| format!("{}: {}", label, e))?;
    let mut prev_ordinal: Option<u64> = None;
    let mut law_all_pass = true;
    let mut claims_tier_ordinals: Vec<u64> = Vec::new();
    // Round 6: the exact multiset of `lowered` evidence rows the
    // typed law account generates — keyed (law ordinal, cert
    // ordinal). The lowered section must project one-to-one from
    // it: an orphan on either side refuses.
    // Values: (form, result, subject, required). Round 7: a row
    // whose machine verdict is `invalid` may still carry the OLD
    // engine's compatibility evidence (the cyclic-class artifacts
    // preserve legacy holds under machine invalid) — such entries
    // are OPTIONAL: bound by fingerprint if present, never
    // demanded, and never allowed to override the machine verdict.
    let mut expected_lowered: std::collections::BTreeMap<
        (u64, Option<u64>),
        (String, String, String, bool),
    > = std::collections::BTreeMap::new();
    // …and the exact set of `law.legacy` report entries the
    // unmigrated non-budget rows require: ordinal -> (fingerprint,
    // verdict).
    let mut expected_legacy: std::collections::BTreeMap<
        u64,
        (String, String, bool),
    > = std::collections::BTreeMap::new();
    for (i, row) in v["law"]["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        only_keys(
            row,
            "law row",
            &["ordinal", "name", "origin", "family", "verdict",
              "law"],
            &["certs", "evidence", "file", "span"],
        )
        .map_err(|e| format!("{}: law.rows[{}]: {}", label, i, e))?;
        // Fleet rows are refused BY NAME: the application
        // artifact does not own a fleet account (Change 7 does),
        // so a fleet-family row can never be excluded from the
        // document verdict — it is inadmissible outright.
        if row["family"] == "fleet"
            || row["origin"] == "fleet"
        {
            return Err(format!(
                "{}: malformed artifact — law.rows[{}] carries a \
                 fleet row, which an application artifact does \
                 not own",
                label, i
            ));
        }
        let ok = row["ordinal"].is_u64()
            && row["name"].as_str().is_some_and(|n| !n.is_empty())
            && row["origin"].is_string()
            && row["family"]
                .as_str()
                .is_some_and(|f| FAMILIES.contains(&f))
            && row["verdict"]
                .as_str()
                .is_some_and(|x| VERDICTS.contains(&x));
        if !ok {
            return Err(format!(
                "{}: malformed artifact — law.rows[{}] must carry \
                 ordinal/name/origin/family/verdict",
                label, i
            ));
        }
        let decoded = decode_law(&row["law"], &cx).map_err(|e| {
            format!(
                "{}: malformed artifact — law.rows[{}] (`{}`) has \
                 an unrecognized or incomplete law payload: {}",
                label,
                i,
                row["name"].as_str().unwrap_or("?"),
                e
            )
        })?;
        let fam = family_of(&decoded);
        if row["family"] != fam {
            return Err(format!(
                "{}: malformed artifact — law.rows[{}] declares \
                 family `{}` but its kind belongs to `{}`",
                label,
                i,
                row["family"].as_str().unwrap_or("?"),
                fam
            ));
        }
        let origin = row["origin"].as_str().unwrap_or_default();
        if !origin_ok(origin, fam) {
            return Err(format!(
                "{}: malformed artifact — law.rows[{}] origin \
                 `{}` does not fit family `{}`",
                label, i, origin, fam
            ));
        }
        let verdict = row["verdict"].as_str().unwrap_or("?");
        // Round 7: static invalidity DOMINATES. An unresolved
        // operand or an undeclared/cyclic effect class makes
        // `invalid` the ONLY truthful verdict — a replayed engine
        // result is not an alternative, and `violated` /
        // `uncertified` cannot dress up a malformed law either.
        let static_invalid = has_unresolved(&decoded)
            || law_class_invalid(&decoded, &cx.classes);
        if static_invalid && verdict != "invalid" {
            return Err(format!(
                "{}: malformed artifact — law.rows[{}] carries an \
                 unresolved operand or an invalid effect class; \
                 only `invalid` is truthful (got `{}`)",
                label, i, verdict
            ));
        }
        // A row whose law generates no certificates must not
        // carry any — unvalidated baggage is not admitted.
        if expected_cert_forms(&decoded).is_none()
            && row.get("certs").is_some()
        {
            return Err(format!(
                "{}: malformed artifact — law.rows[{}] carries \
                 certificates its law does not generate",
                label, i
            ));
        }
        // Certificate rows: evidence binding + verdict recompute.
        if let Some(expected) = expected_cert_forms(&decoded) {
            // A subject outside the legacy analyzable universe
            // (`sorts.fns` — e.g. a module-scoped fn) has NO
            // engine report: the row must carry no certificates
            // and only `invalid` is truthful. Everything inside
            // the universe binds its full evidence.
            let analyzable = match &decoded {
                Law::PhaseEffects { locus, .. } => cx
                    .loci
                    .iter()
                    .any(|(_, d, a)| *d == locus.display && *a),
                Law::EffectForbid { at, .. }
                | Law::EffectOnly { at, .. }
                | Law::EffectPublishSet { at, .. }
                | Law::NoPanic { at } => {
                    // Round 11: "the engines can report on this
                    // subject" is the WALKED state, not summary
                    // membership — the three coverage states are
                    // typed and distinct.
                    cx.fn_analyzed.contains(&at.display)
                }
                _ => true,
            };
            let certs: Vec<&Value> = row["certs"]
                .as_array()
                .into_iter()
                .flatten()
                .collect();
            if !analyzable {
                if !certs.is_empty() {
                    return Err(format!(
                        "{}: malformed artifact — law.rows[{}] \
                         carries certificates no engine produced \
                         (subject outside the analyzable \
                         universe)",
                        label, i
                    ));
                }
                // Round 8: an unanalyzed body is RESIDUE, not
                // invalidity — `uncertified`, carrying its reason
                // — unless the law is statically invalid, which
                // dominates.
                let expect = if static_invalid {
                    "invalid"
                } else {
                    "uncertified"
                };
                if verdict != expect {
                    return Err(format!(
                        "{}: malformed artifact — law.rows[{}] \
                         has no engine evidence (subject outside \
                         the analyzable universe); only `{}` is \
                         truthful (got `{}`)",
                        label, i, expect, verdict
                    ));
                }
            } else {
                if certs.len() != expected.len() {
                    return Err(format!(
                        "{}: malformed artifact — law.rows[{}] \
                         carries {} certificates, its law \
                         generates {}",
                        label,
                        i,
                        certs.len(),
                        expected.len()
                    ));
                }
                let mut recomputed = "holds";
                for (k, (cert, form)) in
                    certs.iter().zip(expected.iter()).enumerate()
                {
                    only_keys(
                        cert,
                        "certificate",
                        &["ordinal", "form", "result"],
                        &["evidence"],
                    )
                    .map_err(|e| {
                        format!(
                            "{}: law.rows[{}] certs[{}]: {}",
                            label, i, k, e
                        )
                    })?;
                    let cert_ev = match cert.get("evidence") {
                        Some(ev) => check_evidence(
                            ev,
                            "certificate evidence",
                        )
                        .map_err(|e| {
                            format!(
                                "{}: law.rows[{}] certs[{}]: {}",
                                label, i, k, e
                            )
                        })?,
                        None => 0,
                    };
                    if cert["result"] == "violated" && cert_ev == 0
                    {
                        return Err(format!(
                            "{}: malformed artifact — \
                             law.rows[{}] certs[{}] is violated \
                             but retains no diagnostics",
                            label, i, k
                        ));
                    }
                    if cert["ordinal"].as_u64()
                        != Some(k as u64)
                        || cert["form"].as_str() != Some(form)
                        || !cert["result"]
                            .as_str()
                            .is_some_and(|r| {
                                VERDICTS.contains(&r)
                            })
                    {
                        return Err(format!(
                            "{}: malformed artifact — \
                             law.rows[{}] certs[{}] does not \
                             match its typed law (expected form \
                             `{}`)",
                            label, i, k, form
                        ));
                    }
                    let r =
                        cert["result"].as_str().unwrap_or("?");
                    // Round 10: an IMPLICIT lifecycle phase (no
                    // hook fn in the function universe) has a
                    // SYNTHETIC certificate — no hook body
                    // performs no effects, so the only truthful
                    // result is `holds`, with no diagnostics.
                    if let Law::PhaseEffects { locus, phases } =
                        &decoded
                    {
                        if let Some((ph, _)) = phases.get(k) {
                            let hook = format!(
                                "{}::{}",
                                locus.display, ph
                            );
                            let explicit = cx
                                .fn_universe
                                .iter()
                                .any(|(_, d)| *d == hook);
                            if !explicit
                                && (r != "holds" || cert_ev != 0)
                            {
                                return Err(format!(
                                    "{}: malformed artifact — \
                                     law.rows[{}] certs[{}] \
                                     covers the implicit phase \
                                     `{}`, whose only truthful \
                                     certificate is a synthetic \
                                     holds",
                                    label, i, k, ph
                                ));
                            }
                        }
                    }
                    if sev(r) > sev(recomputed) {
                        recomputed = r;
                    }
                    expected_lowered.insert(
                        (
                            row["ordinal"].as_u64().unwrap_or(0),
                            Some(k as u64),
                        ),
                        (
                            form.clone(),
                            r.to_string(),
                            expected_subject(&decoded)
                                .unwrap_or_default(),
                            true,
                        ),
                    );
                }
                // Round 7: static invalidity DOMINATES the
                // replayed engine result — when the class catalog
                // says a named class is undeclared or cyclic, the
                // old engine's vacuous `holds` is not an
                // alternative; otherwise the verdict is EXACTLY
                // the recomputed evidence result.
                let expect = if static_invalid {
                    "invalid"
                } else {
                    recomputed
                };
                if verdict != expect {
                    return Err(format!(
                        "{}: malformed artifact — law.rows[{}] \
                         verdict `{}` disagrees with its bound \
                         certificate evidence (expected `{}`)",
                        label, i, verdict, expect
                    ));
                }
            }
        }
        // Budget rows: bound to their compatibility `lowered`
        // evidence — an operand mutation (per_call 4 → 0) cannot
        // keep the old passing row. The entry joins the exact
        // bijection below, so it must exist with the re-rendered
        // form AND nothing else may claim this ordinal.
        if let Some(form) = expected_budget_form(&decoded) {
            if verdict == "holds" || verdict == "violated" {
                expected_lowered.insert(
                    (row["ordinal"].as_u64().unwrap_or(0), None),
                    (
                        form,
                        verdict.to_string(),
                        expected_subject(&decoded)
                            .unwrap_or_default(),
                        true,
                    ),
                );
            } else if verdict == "invalid" {
                // The old engine may still have produced a report
                // (a cycle-resolved-empty class counts zero and
                // holds) — preserved as OPTIONAL evidence, bound
                // by fingerprint, never overriding the machine
                // verdict.
                expected_lowered.insert(
                    (row["ordinal"].as_u64().unwrap_or(0), None),
                    (
                        form,
                        String::new(),
                        expected_subject(&decoded)
                            .unwrap_or_default(),
                        false,
                    ),
                );
            }
        }
        // Unmigrated non-budget rows (`causes:` / `depends:`):
        // their imported old-engine verdict must be keyed to the
        // exact law by a `law.legacy` report entry whose
        // fingerprint re-renders from the typed operands.
        if let Some(form) = expected_legacy_form(&decoded) {
            // (Class validity is enforced above: static_invalid
            // forces the verdict to exactly `invalid`.)
            if verdict == "holds" || verdict == "violated" {
                expected_legacy.insert(
                    row["ordinal"].as_u64().unwrap_or(0),
                    (form, verdict.to_string(), true),
                );
            } else if verdict == "invalid" {
                expected_legacy.insert(
                    row["ordinal"].as_u64().unwrap_or(0),
                    (form, String::new(), false),
                );
            }
        }
        // Row-level evidence: structurally valid always; and a
        // migrated non-holds verdict must RETAIN the judgment's
        // evidence — `violated` means "here is the countermodel",
        // `uncertified` means "here is the residue", never bare
        // labels.
        let row_ev = match row.get("evidence") {
            Some(ev) => check_evidence(ev, "row evidence")
                .map_err(|e| {
                    format!("{}: law.rows[{}]: {}", label, i, e)
                })?,
            None => 0,
        };
        if matches!(
            fam,
            "reachability" | "boundary" | "endpoint" | "bound"
        ) && matches!(verdict, "violated" | "uncertified")
            && row_ev == 0
        {
            return Err(format!(
                "{}: malformed artifact — law.rows[{}] is `{}` \
                 but retains none of its judgment's evidence",
                label, i, verdict
            ));
        }
        // Round 8: `invalid` needs its reason. The claims
        // evaluator has legitimate invalidity beyond references
        // and classes (an operand outside a verb's domain,
        // projection vacuity, an empty `during` slice, `avoiding`
        // overlap, …) — each such judgment carries its
        // explanation, and the row must RETAIN it. A statically
        // decodable invalidity (unresolved operand, invalid
        // class) needs no prose; a bare `invalid` with neither is
        // refused.
        if verdict == "invalid"
            && !matches!(fam, "certificate")
            && !static_invalid
            && row_ev == 0
        {
            return Err(format!(
                "{}: malformed artifact — law.rows[{}] asserts \
                 `invalid` with neither a decodable invalidity \
                 nor its judgment's explanation",
                label, i
            ));
        }
        // An unanalyzed certificate row's `uncertified` carries
        // its residue too.
        if matches!(fam, "certificate")
            && verdict == "uncertified"
            && row_ev == 0
        {
            return Err(format!(
                "{}: malformed artifact — law.rows[{}] is \
                 `uncertified` but carries no residue explanation",
                label, i
            ));
        }
        if matches!(
            fam,
            "reachability" | "boundary" | "endpoint" | "bound"
        ) {
            claims_tier_ordinals
                .push(row["ordinal"].as_u64().unwrap_or(0));
        }
        if row["verdict"] != "holds" {
            law_all_pass = false;
        }
        let ord = row["ordinal"].as_u64().unwrap_or(0);
        let expect = prev_ordinal.map_or(0, |o| o + 1);
        if ord != expect {
            return Err(format!(
                "{}: malformed artifact — law.rows[{}] ordinal {} \
                 breaks the contiguous sequence (expected {})",
                label, i, ord, expect
            ));
        }
        prev_ordinal = Some(ord);
    }
    // Round 6: the lowered section projects ONE-TO-ONE from the
    // typed law account. Every lowered row must be claimed by
    // exactly one (law ordinal, cert ordinal) expectation, and
    // every expectation must be met — deleting law rows orphans
    // their evidence instead of passing vacuously.
    {
        let mut unmet = expected_lowered;
        for (i, r) in v["lowered"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            only_keys(
                r,
                "lowered row",
                &["subject", "form", "result", "ordinal"],
                &["cert"],
            )
            .map_err(|e| {
                format!("{}: lowered[{}]: {}", label, i, e)
            })?;
            let Some(ord) = r["ordinal"].as_u64() else {
                return Err(format!(
                    "{}: malformed artifact — lowered[{}] carries \
                     no law ordinal",
                    label, i
                ));
            };
            let cert = r["cert"].as_u64();
            let Some((form, result, subject, required)) =
                unmet.remove(&(ord, cert))
            else {
                return Err(format!(
                    "{}: malformed artifact — lowered[{}] (`{}`) \
                     is not claimed by any typed law row",
                    label,
                    i,
                    r["form"].as_str().unwrap_or("?")
                ));
            };
            // Required entries bind result exactly; OPTIONAL
            // entries (machine-invalid rows preserving the old
            // engine's report) bind the fingerprint and subject,
            // with the old result constrained to the old engine's
            // vocabulary.
            let result_ok = if required {
                r["result"].as_str() == Some(&result)
            } else {
                matches!(
                    r["result"].as_str(),
                    Some("holds") | Some("violated")
                )
            };
            if r["form"].as_str() != Some(&form)
                || !result_ok
                || r["subject"].as_str() != Some(&subject)
            {
                return Err(format!(
                    "{}: malformed artifact — lowered[{}] does \
                     not match its typed law (expected `{}` on \
                     `{}`)",
                    label, i, form, subject
                ));
            }
        }
        if let Some(((ord, cert), (form, _, _, _))) = unmet
            .into_iter()
            .find(|(_, (_, _, _, required))| *required)
        {
            return Err(format!(
                "{}: malformed artifact — law ordinal {}{} has no \
                 lowered evidence row matching `{}`",
                label,
                ord,
                cert.map(|c| format!(" cert {}", c))
                    .unwrap_or_default(),
                form
            ));
        }
    }
    // Round 6: the `law.legacy` report projects one-to-one from
    // the unmigrated non-budget rows with imported verdicts.
    {
        let mut unmet = expected_legacy;
        for (i, e) in v["law"]["legacy"]
            .as_array()
            .ok_or_else(|| {
                format!(
                    "{}: malformed artifact — law.legacy must be \
                     an array",
                    label
                )
            })?
            .iter()
            .enumerate()
        {
            only_keys(
                e,
                "law.legacy entry",
                &["ordinal", "form", "result"],
                &[],
            )
            .map_err(|x| {
                format!("{}: law.legacy[{}]: {}", label, i, x)
            })?;
            let Some(ord) = e["ordinal"].as_u64() else {
                return Err(format!(
                    "{}: malformed artifact — law.legacy[{}] \
                     ordinal must be a number",
                    label, i
                ));
            };
            let Some((form, result, required)) =
                unmet.remove(&ord)
            else {
                return Err(format!(
                    "{}: malformed artifact — law.legacy[{}] is \
                     not claimed by any unmigrated law row",
                    label, i
                ));
            };
            let result_ok = if required {
                e["result"].as_str() == Some(&result)
            } else {
                matches!(
                    e["result"].as_str(),
                    Some("holds") | Some("violated")
                )
            };
            if e["form"].as_str() != Some(&form) || !result_ok {
                return Err(format!(
                    "{}: malformed artifact — law.legacy[{}] does \
                     not re-render from the typed law at ordinal \
                     {} (expected `{}`)",
                    label, i, ord, form
                ));
            }
        }
        if let Some((ord, (form, _, _))) = unmet
            .into_iter()
            .find(|(_, (_, _, required))| *required)
        {
            return Err(format!(
                "{}: malformed artifact — law ordinal {} imports \
                 a legacy verdict with no report entry matching \
                 `{}`",
                label, ord, form
            ));
        }
    }
    // Capabilities: the EXACT flag set; adequacy recomputed.
    const CAP_FLAGS: &[(&str, u32)] = &[
        ("exact_calls", 1 << 0),
        ("exact_publishes", 1 << 1),
        ("exact_subscribes", 1 << 2),
        ("exact_key_filters", 1 << 9),
        ("exact_ownership", 1 << 3),
        ("exact_placement", 1 << 4),
        ("exact_routes", 1 << 8),
        ("exact_effects", 1 << 7),
        ("exact_cardinality", 1 << 10),
        ("exact_delivery_guarantees", 1 << 11),
    ];
    let caps = v["capabilities"].as_object().ok_or_else(|| {
        format!("{}: capabilities must be an object", label)
    })?;
    if caps.len() != CAP_FLAGS.len()
        || CAP_FLAGS.iter().any(|(k, _)| {
            !caps.get(*k).is_some_and(|x| x.is_boolean())
        })
    {
        return Err(format!(
            "{}: malformed artifact — capabilities must carry \
             exactly the {} known boolean flags",
            label,
            CAP_FLAGS.len()
        ));
    }
    let mut vouched: u32 = 0;
    for (k, bit) in CAP_FLAGS {
        if caps[*k] == true {
            vouched |= bit;
        }
    }
    use hale_model::JudgmentFamily as JF;
    const MIGRATED: &[(&str, JF)] = &[
        ("reachability", JF::Reachability),
        ("boundary", JF::Boundary),
        ("endpoint", JF::Endpoint),
        ("bound", JF::Bound),
        ("certificate", JF::Certificate),
    ];
    let adequacy = v["adequacy"].as_object().ok_or_else(|| {
        format!("{}: adequacy must be an object", label)
    })?;
    if adequacy.len() != MIGRATED.len() {
        return Err(format!(
            "{}: malformed artifact — adequacy must carry exactly \
             the {} migrated families",
            label,
            MIGRATED.len()
        ));
    }
    for (name, fam) in MIGRATED {
        let required = fam.required_relations().0;
        let expect = if vouched & required == required {
            "exact"
        } else {
            "degraded"
        };
        if adequacy.get(*name).and_then(|x| x.as_str())
            != Some(expect)
        {
            return Err(format!(
                "{}: malformed artifact — adequacy.{} disagrees \
                 with the positive capability account (expected \
                 {})",
                label, name, expect
            ));
        }
    }
    // Claims ↔ law, both directions, with form/source binding.
    let mut claimed_ordinals: Vec<u64> = Vec::new();
    for (i, c) in
        v["claims"].as_array().into_iter().flatten().enumerate()
    {
        let Some(ord) = c["ordinal"].as_u64() else {
            return Err(format!(
                "{}: malformed artifact — claims[{}] carries no \
                 law ordinal",
                label, i
            ));
        };
        claimed_ordinals.push(ord);
        let matching: Vec<&Value> = v["law"]["rows"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| r["ordinal"] == ord)
            .collect();
        if matching.len() != 1 {
            return Err(format!(
                "{}: malformed artifact — claims[{}] does not \
                 project one-to-one from law ordinal {}",
                label, i, ord
            ));
        }
        let law_row = matching[0];
        let decoded = decode_law(&law_row["law"], &cx)
            .expect("validated above");
        let rendered = render_claims_form(&decoded);
        if rendered.as_deref() != c["form"].as_str() {
            return Err(format!(
                "{}: malformed artifact — claims[{}] (`{}`) form \
                 does not render from the typed law at ordinal {}",
                label,
                i,
                c["name"].as_str().unwrap_or("?"),
                ord
            ));
        }
        let origin =
            law_row["origin"].as_str().unwrap_or_default();
        let source_ok = match c["source"].as_str() {
            Some(src) => {
                origin == format!("constitution:{}", src)
            }
            None => !origin.starts_with("constitution:"),
        };
        let ok = source_ok
            && law_row["name"] == c["name"]
            && law_row["verdict"] == c["result"];
        if !ok {
            return Err(format!(
                "{}: malformed artifact — claims[{}] (`{}`) \
                 disagrees with law ordinal {} on \
                 name/verdict/source",
                label,
                i,
                c["name"].as_str().unwrap_or("?"),
                ord
            ));
        }
    }
    // The judgment pre-pass is RECOMPUTABLE for duplicates: two
    // claims-tier rows sharing a name is exactly the
    // contract-of-record failure the evaluator refuses, so the
    // account must say so.
    {
        let mut names = BTreeSet::new();
        let mut dup = None;
        for r in v["law"]["rows"].as_array().into_iter().flatten()
        {
            let fam = r["family"].as_str().unwrap_or("");
            if matches!(
                fam,
                "reachability" | "boundary" | "endpoint" | "bound"
            ) {
                let name = r["name"].as_str().unwrap_or("");
                if !names.insert(name.to_string()) {
                    dup = Some(name.to_string());
                }
            }
        }
        if let Some(name) = dup {
            if issue_count == 0 {
                return Err(format!(
                    "{}: malformed artifact — duplicate claim \
                     name `{}` with an empty law-selection \
                     account",
                    label, name
                ));
            }
        }
    }
    claimed_ordinals.sort_unstable();
    let mut tier: Vec<u64> = claims_tier_ordinals;
    tier.sort_unstable();
    if claimed_ordinals != tier {
        return Err(format!(
            "{}: malformed artifact — the claims rows and the \
             claims-tier law rows are not one-to-one (claims: \
             {:?}, law: {:?})",
            label, claimed_ordinals, tier
        ));
    }
    // The document verdict is RECOMPUTED, never trusted.
    let claims_pass = v["claims"]
        .as_array()
        .into_iter()
        .flatten()
        .all(|c| c["result"] == "holds");
    let lowered_pass = v["lowered"]
        .as_array()
        .into_iter()
        .flatten()
        .all(|r| r["result"] == "holds");
    let expect_verdict = if claims_pass
        && lowered_pass
        && law_all_pass
        && issue_count == 0
    {
        "clean"
    } else {
        "law_failed"
    };
    if v["verdict"] != expect_verdict {
        return Err(format!(
            "{}: malformed artifact — document verdict `{}` \
             disagrees with its own law rows (recomputed: {})",
            label,
            v["verdict"].as_str().unwrap_or("?"),
            expect_verdict
        ));
    }
    let _ = BTreeSet::<u8>::new();
    Ok(())
}
