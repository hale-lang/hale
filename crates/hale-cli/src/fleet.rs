//! GH #408 Phase 1: compose topology artifacts into one fleet model.
//!
//! A fleet is a named deployed system of application **instances**,
//! not "every main in a repository". It composes *artifacts*, never
//! source — that distinction is the whole design, and it is worth
//! restating because the tempting implementation is a super-main
//! build that imports every application and runs ordinary claims over
//! the merged source. That would be unsound in both directions:
//!
//!  - **it invents edges.** An unbound topic is in-process by
//!    default, so merging two binaries makes matching publishers and
//!    subscribers look locally connected when no deployed route joins
//!    them;
//!  - **it erases edges.** Real routes supplied by deployment config
//!    do not appear merely because source was merged;
//!  - **it changes what a call means.** Calls cannot cross a process
//!    boundary; flattening makes ordinary method reachability look
//!    possible where only serialized messaging exists.
//!
//! So the composition unit is the artifact instance. Matching wire
//! identities establish *compatibility*; only an explicit route
//! creates a fleet edge.
//!
//! This module is deliberately a CLIENT of the artifact rather than a
//! second source analyzer. It reads exactly what a third party would
//! read, which is the only way "an outside evaluator can replay this"
//! stays true.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The plan's own schema, independent of the topology artifact's.
pub const FLEET_PLAN_SCHEMA: &str = "1.0";

/// An exact, finite deployment. Autoscaling ranges and wildcard
/// discovery are elaborator INPUTS, not sealed-plan contents: a
/// bounded range is not one truth value for a cardinality claim.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct FleetPlan {
    pub schema: String,
    pub name: String,
    pub instances: Vec<InstanceSpec>,
    #[serde(default)]
    pub routes: Vec<RouteSpec>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct InstanceSpec {
    /// `riskgw-prod-0`, not `riskgw`. An application TYPE is not a
    /// deployed instance, and everything downstream — cardinality,
    /// witnesses, several instances of one artifact — needs the
    /// distinction.
    pub id: String,
    /// Path to a topology artifact, relative to the plan file.
    pub artifact: String,
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct RouteSpec {
    pub id: String,
    pub publishers: Vec<Endpoint>,
    pub subscribers: Vec<Endpoint>,
    /// Free-form for now; transport POLICY is a later phase. Carried
    /// so the fleet model records how a hop is realized.
    #[serde(default)]
    pub transport: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub instance: String,
    /// The topic's LOCAL name in that instance's artifact.
    pub topic: String,
}

/// One loaded, validated component.
struct Component {
    id: String,
    labels: Vec<String>,
    artifact: serde_json::Value,
    path: PathBuf,
}

/// `(wire subject, payload hash)` — the cross-binary join key.
///
/// Never the local topic identifier: the same identifier can mean
/// different wire shapes in different applications, and different
/// identifiers can deliberately denote one contract.
#[derive(PartialEq, Eq, Clone, Debug)]
struct WireId {
    subject: String,
    payload_hash: String,
}

pub fn compose(plan_path: &Path) -> Result<String, Vec<String>> {
    let plan = read_plan(plan_path)?;
    let base = plan_path.parent().unwrap_or(Path::new("."));
    let mut errs: Vec<String> = Vec::new();

    // ---- step 1: validate every component artifact ----
    let mut comps: Vec<Component> = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for inst in &plan.instances {
        if !seen.insert(inst.id.as_str()) {
            errs.push(format!(
                "instance id `{}` is used twice — an instance id is \
                 how every vertex, route endpoint and witness refers \
                 to one deployed process",
                inst.id
            ));
            continue;
        }
        match load_artifact(&base.join(&inst.artifact)) {
            Ok(v) => comps.push(Component {
                id: inst.id.clone(),
                labels: inst.labels.clone(),
                artifact: v,
                path: base.join(&inst.artifact),
            }),
            Err(e) => errs.push(format!("instance `{}`: {}", inst.id, e)),
        }
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    let by_id: BTreeMap<&str, &Component> =
        comps.iter().map(|c| (c.id.as_str(), c)).collect();

    // ---- steps 2-3: namespace, retaining interiors ----
    //
    // Calls stay strictly INSIDE one instance: there is no such thing
    // as a cross-process call, and inventing one is exactly what
    // source-merging would do.
    let mut vertices: Vec<String> = Vec::new();
    let mut call_edges: Vec<(String, String)> = Vec::new();
    let mut local_bus: Vec<(String, String)> = Vec::new();
    for c in &comps {
        for f in arr(&c.artifact["sorts"]["fns"]) {
            if let Some(n) = f.as_str() {
                vertices.push(format!("{}::{}", c.id, n));
            }
        }
        for e in arr(&c.artifact["relations"]["calls"]) {
            if let (Some(f), Some(t)) =
                (e["from"].as_str(), e["to"].as_str())
            {
                call_edges.push((
                    format!("{}::{}", c.id, f),
                    format!("{}::{}", c.id, t),
                ));
            }
        }
        // A local publish→subscribe pair inside one instance is a
        // real in-process edge and stays one.
        for p in arr(&c.artifact["relations"]["publishes"]) {
            let (Some(pf), Some(subj)) =
                (p["fn"].as_str(), p["subject"].as_str())
            else {
                continue;
            };
            for s in arr(&c.artifact["relations"]["subscribes"]) {
                if s["subject"].as_str() == Some(subj) {
                    if let (Some(l), Some(h)) =
                        (s["locus"].as_str(), s["handler"].as_str())
                    {
                        local_bus.push((
                            format!("{}::{}", c.id, pf),
                            format!("{}::{}::{}", c.id, l, h),
                        ));
                    }
                }
            }
        }
    }

    // ---- steps 4-6: routes ----
    let mut routes_out: Vec<String> = Vec::new();
    let mut route_edges: Vec<(String, String, String)> = Vec::new();
    for r in &plan.routes {
        if r.publishers.is_empty() || r.subscribers.is_empty() {
            errs.push(format!(
                "route `{}` has {} publisher(s) and {} subscriber(s) — \
                 a route with an empty side connects nothing and is \
                 almost certainly a mistake in the plan",
                r.id,
                r.publishers.len(),
                r.subscribers.len()
            ));
            continue;
        }
        // step 5: every endpoint must agree on the wire identity.
        let mut wire: Option<(WireId, String)> = None;
        let mut bad = false;
        for ep in r.publishers.iter().chain(r.subscribers.iter()) {
            let Some(c) = by_id.get(ep.instance.as_str()) else {
                errs.push(format!(
                    "route `{}` names instance `{}`, which the plan \
                     does not declare",
                    r.id, ep.instance
                ));
                bad = true;
                continue;
            };
            match wire_id(&c.artifact, &ep.topic) {
                None => {
                    errs.push(format!(
                        "route `{}`: instance `{}` declares no topic \
                         `{}`",
                        r.id, ep.instance, ep.topic
                    ));
                    bad = true;
                }
                Some(w) => match &wire {
                    None => wire = Some((w, ep.instance.clone())),
                    Some((prev, prev_inst)) if *prev != w => {
                        errs.push(format!(
                            "route `{}` cannot be formed: `{}` sees \
                             subject `{}` / payload {}, `{}` sees `{}` \
                             / {}. Endpoints must agree on the WIRE \
                             identity — a shared local name is not a \
                             shared contract",
                            r.id,
                            prev_inst,
                            prev.subject,
                            prev.payload_hash,
                            ep.instance,
                            w.subject,
                            w.payload_hash
                        ));
                        bad = true;
                    }
                    Some(_) => {}
                },
            }
        }
        if bad {
            continue;
        }
        let Some((w, _)) = wire else { continue };
        if w.subject.is_empty() {
            errs.push(format!(
                "route `{}` carries a topic with no declared \
                 `subject:` — its wire name is a local, non-portable \
                 fallback, so it cannot be joined across binaries",
                r.id
            ));
            continue;
        }

        // step 6: the hop keeps its boundary as an explicit edge
        // rather than collapsing into a direct call.
        for p in &r.publishers {
            let Some(pc) = by_id.get(p.instance.as_str()) else {
                continue;
            };
            for pf in publishers_of(&pc.artifact, &p.topic) {
                for s in &r.subscribers {
                    let Some(sc) = by_id.get(s.instance.as_str()) else {
                        continue;
                    };
                    for (l, h) in subscribers_of(&sc.artifact, &s.topic) {
                        route_edges.push((
                            format!("{}::{}", p.instance, pf),
                            format!("{}::{}::{}", s.instance, l, h),
                            r.id.clone(),
                        ));
                    }
                }
            }
        }
        routes_out.push(format!(
            "    {{\"id\": {}, \"subject\": {}, \"payload_hash\": {}, \
             \"transport\": {}}}",
            q(&r.id),
            q(&w.subject),
            q(&w.payload_hash),
            match &r.transport {
                Some(t) => q(t),
                None => "null".to_string(),
            }
        ));
    }
    if !errs.is_empty() {
        return Err(errs);
    }

    // ---- step 8: unknowns propagate ----
    //
    // Uncertainty in a component stays uncertainty in the fleet. It
    // may add paths; it may never delete one and certify an absence.
    let mut unknowns: Vec<String> = Vec::new();
    for c in &comps {
        for u in arr(&c.artifact["unknowns"]) {
            unknowns.push(format!(
                "    {{\"instance\": {}, \"unknown\": {}}}",
                q(&c.id),
                serde_json::to_string(&u).unwrap_or_else(|_| "null".into())
            ));
        }
    }

    Ok(render(
        &plan, &comps, &vertices, &call_edges, &local_bus, &route_edges,
        &routes_out, &unknowns,
    ))
}

#[allow(clippy::too_many_arguments)]
fn render(
    plan: &FleetPlan,
    comps: &[Component],
    vertices: &[String],
    calls: &[(String, String)],
    local_bus: &[(String, String)],
    route_edges: &[(String, String, String)],
    routes: &[String],
    unknowns: &[String],
) -> String {
    // The MODEL half — hashed. Instance identities and cardinalities,
    // route hyperedges, wire identities, component shape hashes.
    // Claim results and provenance stay out of it, mirroring the
    // application artifact's split.
    let mut model = String::new();
    model.push_str("  \"instances\": [\n");
    for (i, c) in comps.iter().enumerate() {
        model.push_str(&format!(
            "    {{\"id\": {}, \"app_shape_hash\": {}, \"labels\": [{}]}}{}\n",
            q(&c.id),
            q(c.artifact["shape_hash"].as_str().unwrap_or("")),
            c.labels
                .iter()
                .map(|l| q(l))
                .collect::<Vec<_>>()
                .join(", "),
            if i + 1 == comps.len() { "" } else { "," }
        ));
    }
    model.push_str("  ],\n  \"routes\": [\n");
    model.push_str(&routes.join(",\n"));
    model.push_str("\n  ],\n  \"vertices\": [");
    model.push_str(
        &vertices.iter().map(|v| q(v)).collect::<Vec<_>>().join(", "),
    );
    model.push_str("],\n  \"relations\": {\n    \"calls\": [\n");
    model.push_str(
        &calls
            .iter()
            .map(|(f, t)| {
                format!("      {{\"from\": {}, \"to\": {}}}", q(f), q(t))
            })
            .collect::<Vec<_>>()
            .join(",\n"),
    );
    model.push_str("\n    ],\n    \"local_bus\": [\n");
    model.push_str(
        &local_bus
            .iter()
            .map(|(f, t)| {
                format!("      {{\"from\": {}, \"to\": {}}}", q(f), q(t))
            })
            .collect::<Vec<_>>()
            .join(",\n"),
    );
    model.push_str("\n    ],\n    \"routed\": [\n");
    model.push_str(
        &route_edges
            .iter()
            .map(|(f, t, r)| {
                format!(
                    "      {{\"from\": {}, \"to\": {}, \"route\": {}}}",
                    q(f),
                    q(t),
                    q(r)
                )
            })
            .collect::<Vec<_>>()
            .join(",\n"),
    );
    model.push_str("\n    ]\n  }");

    let fleet_shape_hash = fnv(&model);
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("  \"schema\": {},\n", q(FLEET_PLAN_SCHEMA)));
    out.push_str(&format!("  \"name\": {},\n", q(&plan.name)));
    out.push_str(&format!(
        "  \"fleet_shape_hash\": {},\n",
        q(&fleet_shape_hash)
    ));
    out.push_str(&model);
    // Unhashed: provenance and uncertainty, like the application
    // artifact's results half.
    out.push_str(",\n  \"unknowns\": [\n");
    out.push_str(&unknowns.join(",\n"));
    out.push_str("\n  ],\n  \"components\": [\n");
    out.push_str(
        &comps
            .iter()
            .map(|c| {
                format!(
                    "    {{\"id\": {}, \"artifact\": {}}}",
                    q(&c.id),
                    q(&c.path.display().to_string())
                )
            })
            .collect::<Vec<_>>()
            .join(",\n"),
    );
    out.push_str("\n  ]\n}\n");
    out
}

fn read_plan(p: &Path) -> Result<FleetPlan, Vec<String>> {
    let src = std::fs::read_to_string(p)
        .map_err(|e| vec![format!("read {}: {}", p.display(), e)])?;
    let plan: FleetPlan = serde_json::from_str(&src)
        .map_err(|e| vec![format!("parse {}: {}", p.display(), e)])?;
    if plan.schema != FLEET_PLAN_SCHEMA {
        return Err(vec![format!(
            "{}: plan schema `{}`, this build understands `{}`",
            p.display(),
            plan.schema,
            FLEET_PLAN_SCHEMA
        )]);
    }
    if plan.instances.is_empty() {
        return Err(vec![format!(
            "{}: a fleet with no instances composes nothing",
            p.display()
        )]);
    }
    Ok(plan)
}

/// Load one component artifact and refuse anything a composition
/// cannot honestly build on.
fn load_artifact(p: &Path) -> Result<serde_json::Value, String> {
    let src = std::fs::read_to_string(p)
        .map_err(|e| format!("read {}: {}", p.display(), e))?;
    // Integrity BEFORE meaning. `shape_hash` is an identity covering
    // the model half only, so it cannot vouch for the `topics` rows a
    // composition joins on; the whole-body digest can.
    match hale_types::topology::verify_artifact_digest(&src) {
        Some(true) => {}
        Some(false) => {
            return Err(format!(
                "{}: artifact_digest does not match its contents",
                p.display()
            ))
        }
        None => {
            return Err(format!(
                "{}: no artifact_digest — this predates schema 1.3 and \
                 cannot be verified, and an unverifiable component is \
                 not a foundation for a certificate",
                p.display()
            ))
        }
    }
    let v: serde_json::Value = serde_json::from_str(&src)
        .map_err(|e| format!("parse {}: {}", p.display(), e))?;

    let sem = v["semantics"].as_u64();
    if sem != Some(hale_types::topology::MODEL_SEMANTICS as u64) {
        return Err(format!(
            "{}: model semantics {} — this build speaks {}. The rows \
             may share a shape and mean different things, so composing \
             them would build a model neither compiler would certify",
            p.display(),
            sem.map(|s| s.to_string())
                .unwrap_or_else(|| "absent".into()),
            hale_types::topology::MODEL_SEMANTICS
        ));
    }
    match v["verdict"].as_str() {
        Some("clean") => {}
        Some(other) => {
            return Err(format!(
                "{}: component verdict `{}` — a component whose own law \
                 fails is not admissible; fix it before composing",
                p.display(),
                other
            ))
        }
        None => {
            return Err(format!(
                "{}: no verdict field (pre-1.4 artifact)",
                p.display()
            ))
        }
    }
    Ok(v)
}

fn wire_id(a: &serde_json::Value, local: &str) -> Option<WireId> {
    arr(&a["topics"]).iter().find_map(|t| {
        if t["name"].as_str() == Some(local) {
            Some(WireId {
                subject: t["subject"].as_str().unwrap_or("").to_string(),
                payload_hash: t["payload_hash"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
            })
        } else {
            None
        }
    })
}

fn publishers_of(a: &serde_json::Value, local: &str) -> Vec<String> {
    arr(&a["relations"]["publishes"])
        .iter()
        .filter(|p| p["subject"].as_str() == Some(local))
        .filter_map(|p| p["fn"].as_str().map(str::to_string))
        .collect()
}

fn subscribers_of(
    a: &serde_json::Value,
    local: &str,
) -> Vec<(String, String)> {
    arr(&a["relations"]["subscribes"])
        .iter()
        .filter(|s| s["subject"].as_str() == Some(local))
        .filter_map(|s| {
            Some((
                s["locus"].as_str()?.to_string(),
                s["handler"].as_str()?.to_string(),
            ))
        })
        .collect()
}

fn arr(v: &serde_json::Value) -> Vec<serde_json::Value> {
    v.as_array().cloned().unwrap_or_default()
}

fn q(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

fn fnv(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:016x}", h)
}
