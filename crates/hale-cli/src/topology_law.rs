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
        // Retained for the closed decode (an unknown or malformed
        // entry refuses); the depends family is unmigrated, so no
        // downstream binding consumes the selectors yet.
        #[allow(dead_code)]
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
    Fleet,
}

/// The artifact's own catalogs, against which every resolved
/// reference must exist.
pub struct RefContext {
    pub groups: Vec<String>,
    pub topics: Vec<String>,
    pub fn_universe: Vec<String>,
    pub loci: Vec<String>,
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
        let strings = |x: &Value| -> Vec<String> {
            x.as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| s.as_str().map(|s| s.to_string()))
                .collect()
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
        Ok(RefContext {
            groups: v["groups"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, _)| k.clone())
                .collect(),
            topics: v["topics"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| {
                    t["name"].as_str().map(|s| s.to_string())
                })
                .collect(),
            fn_universe: strings(&v["law"]["fn_universe"]),
            loci: strings(&v["sorts"]["loci"]),
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
        })
    }

    fn exists(
        &self,
        catalog: &[String],
        r: &Ref,
        what: &str,
    ) -> Result<(), String> {
        if r.resolved
            && !catalog.iter().any(|n| *n == r.display)
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
                    "{}: raw identity `{}` appears under two                      spellings (`{}` and `{}`)",
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
                    "{}: spelling `{}` names two raw identities                      (`{}` and `{}`)",
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
        self.exists(&self.groups, &r, what)?;
        Ok(r)
    }
    fn topic(&self, v: &Value, what: &str) -> Result<Ref, String> {
        let r = decode_ref(v, what)?;
        self.exists(&self.topics, &r, what)?;
        Ok(r)
    }
    fn function(
        &self,
        v: &Value,
        what: &str,
    ) -> Result<Ref, String> {
        let r = decode_ref(v, what)?;
        self.exists(&self.fn_universe, &r, what)?;
        Ok(r)
    }
    fn locus(&self, v: &Value, what: &str) -> Result<Ref, String> {
        let r = decode_ref(v, what)?;
        self.exists(&self.loci, &r, what)?;
        Ok(r)
    }
    fn phase(&self, v: &Value, what: &str) -> Result<Ref, String> {
        let r = decode_ref(v, what)?;
        self.exists(&self.phases, &r, what)?;
        Ok(r)
    }
    fn seed(&self, v: &Value, what: &str) -> Result<Ref, String> {
        let r = decode_ref(v, what)?;
        self.exists(&self.seeds, &r, what)?;
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
            for t in s["topics"]
                .as_array()
                .ok_or_else(|| format!("{}: topics array", w))?
            {
                only_keys(t, &w, &["name", "display"], &[])?;
                let disp =
                    t["display"].as_str().ok_or_else(|| {
                        format!("{}: candidate display", w)
                    })?;
                // Candidate topics must exist in the artifact.
                if !self.topics.iter().any(|n| n == disp) {
                    return Err(format!(
                        "{}: candidate topic `{}` is not in this \
                         artifact",
                        w, disp
                    ));
                }
            }
            if !s["subjects"].as_array().is_some_and(|ss| {
                ss.iter().all(|x| x.is_string())
            }) {
                return Err(format!(
                    "{}: subjects must be strings",
                    w
                ));
            }
            out.push(Selector {
                name: name.to_string(),
            });
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
        Law::Fleet => false,
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
        ) => Ok(Law::Fleet),
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
        Law::Fleet => "fleet",
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
        "fleet",
    ];
    const VERDICTS: &[&str] =
        &["holds", "violated", "uncertified", "invalid"];
    if !v["law"].is_object()
        || !v["law"]["law_digest"].is_string()
        || !v["law"]["inputs_digest"].is_string()
        || !v["law"]["rows"].is_array()
    {
        return Err(format!(
            "{}: malformed artifact — law section incomplete",
            label
        ));
    }
    let cx = RefContext::from_artifact(v)
        .map_err(|e| format!("{}: {}", label, e))?;
    let origin_ok = |origin: &str, family: &str| -> bool {
        match family {
            "certificate" | "unmigrated" => origin == "annotation",
            "fleet" => origin == "fleet",
            _ => {
                origin == "main"
                    || origin == "library"
                    || origin.starts_with("library:")
                    || origin.starts_with("constitution:")
            }
        }
    };
    let mut prev_ordinal: Option<u64> = None;
    let mut law_all_pass = true;
    let mut claims_tier_ordinals: Vec<u64> = Vec::new();
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
        // Resolution ↔ verdict binding: a passing row cannot
        // carry an unresolved operand (the judgment refuses such
        // laws), so flipping `resolved` to dodge existence checks
        // flips this contract instead.
        if has_unresolved(&decoded) && verdict == "holds" {
            return Err(format!(
                "{}: malformed artifact — law.rows[{}] holds over \
                 an unresolved operand",
                label, i
            ));
        }
        // Certificate rows: evidence binding + verdict recompute.
        if let Some(expected) = expected_cert_forms(&decoded) {
            let certs: Vec<&Value> = row["certs"]
                .as_array()
                .into_iter()
                .flatten()
                .collect();
            if verdict != "invalid" || !certs.is_empty() {
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
                    if sev(r) > sev(recomputed) {
                        recomputed = r;
                    }
                }
                // The aggregate verdict is the max certificate
                // severity — or `invalid` when justified by the
                // class catalog (undeclared / cyclic user class).
                let class_invalid = match &decoded {
                    Law::EffectForbid { classes, .. }
                    | Law::EffectOnly { classes, .. }
                    | Law::EffectCauses { classes, .. } => {
                        classes.iter().any(|c| {
                            !c.builtin
                                && cx
                                    .classes
                                    .iter()
                                    .find(|(n, _, _)| {
                                        *n == c.class
                                    })
                                    .map_or(true, |(_, d, cy)| {
                                        !d || *cy
                                    })
                        })
                    }
                    Law::PhaseEffects { phases, .. } => phases
                        .iter()
                        .flat_map(|(_, cs)| cs.iter())
                        .any(|c| {
                            !c.builtin
                                && cx
                                    .classes
                                    .iter()
                                    .find(|(n, _, _)| {
                                        *n == c.class
                                    })
                                    .map_or(true, |(_, d, cy)| {
                                        !d || *cy
                                    })
                        }),
                    _ => false,
                };
                let verdict_ok = verdict == recomputed
                    || (verdict == "invalid" && class_invalid);
                if !verdict_ok {
                    return Err(format!(
                        "{}: malformed artifact — law.rows[{}] \
                         verdict `{}` disagrees with its bound \
                         certificate evidence (recomputed: {})",
                        label, i, verdict, recomputed
                    ));
                }
            }
        }
        // Budget rows: bound to their compatibility `lowered`
        // evidence — an operand mutation (per_call 4 → 0) cannot
        // keep the old passing row.
        if let Some(form) = expected_budget_form(&decoded) {
            if verdict == "holds" || verdict == "violated" {
                let hit = v["lowered"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|r| {
                        r["form"].as_str() == Some(&form)
                            && r["result"] == verdict
                    });
                if !hit {
                    return Err(format!(
                        "{}: malformed artifact — law.rows[{}] \
                         (`{}`) has no lowered evidence row \
                         matching `{}` with result `{}`",
                        label,
                        i,
                        row["name"].as_str().unwrap_or("?"),
                        form,
                        verdict
                    ));
                }
            }
        }
        if matches!(
            fam,
            "reachability" | "boundary" | "endpoint" | "bound"
        ) {
            claims_tier_ordinals
                .push(row["ordinal"].as_u64().unwrap_or(0));
        }
        if fam != "fleet" && row["verdict"] != "holds" {
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
    let expect_verdict =
        if claims_pass && lowered_pass && law_all_pass {
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
