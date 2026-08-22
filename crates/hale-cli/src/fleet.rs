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
/// 1.1 (GH #408 Phase 7): `binary` / `binary_sha256` on an instance,
/// for `hale fleet attest`. A 1.0 plan still reads; the artifact
/// emits 1.1.
pub const FLEET_PLAN_SCHEMA: &str = "1.1";

/// Plan schemas this build reads. Equality was right when there was
/// one; a set keeps "your plan is newer than your compiler" a real
/// refusal without making every older plan one too.
const READABLE_PLAN_SCHEMAS: [&str; 2] = ["1.0", "1.1"];

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
    /// Fleet groups quantify over INSTANCES, by id or by label. A
    /// group's vertices are every vertex of its instances, which is
    /// the same projection an application-tier group makes from a
    /// locus to its methods — one altitude up.
    #[serde(default)]
    pub groups: BTreeMap<String, GroupSpec>,
    #[serde(default)]
    pub claims: Vec<ClaimSpec>,
}

#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct GroupSpec {
    #[serde(default)]
    pub instances: Vec<String>,
    #[serde(default)]
    pub labels: Vec<String>,
}

/// A fleet claim, as normalized rows rather than source grammar —
/// the plan is an IR, so an external generator can produce one
/// without Hale syntax committing to a deployment format.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ClaimSpec {
    pub name: String,
    #[serde(default)]
    pub forbid_reaches: Option<ForbidReaches>,
    #[serde(default)]
    pub require_subscribes: Option<RequireEndpoint>,
    #[serde(default)]
    pub require_publishes: Option<RequireEndpoint>,
    #[serde(default)]
    pub count_publisher_instances: Option<CountSpec>,
    #[serde(default)]
    pub count_subscriber_instances: Option<CountSpec>,
    #[serde(default)]
    pub only_edges: Option<OnlyEdges>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ForbidReaches {
    pub from: String,
    pub to: String,
    /// Masking a group's vertices makes this the interposition form:
    /// "every path passes through the gate" is "no path avoids it".
    #[serde(default)]
    pub avoiding: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct RequireEndpoint {
    pub group: String,
    pub subject: String,
}

/// Fleet cardinality counts INSTANCE-qualified endpoints, not source
/// declarations — a different sort from the application tier, which
/// is why it gets a different spelling.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct CountSpec {
    pub subject: String,
    #[serde(default)]
    pub eq: Option<usize>,
    #[serde(default)]
    pub max: Option<usize>,
    #[serde(default)]
    pub min: Option<usize>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct OnlyEdges {
    pub from: String,
    pub to: String,
    /// Wire subjects that may cross this boundary. A grant names a
    /// typed topic identity, not a transport address.
    #[serde(default)]
    pub grant_subjects: Vec<String>,
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
    /// GH #408 Phase 7 (`attest`): path to this instance's built
    /// executable, relative to the plan file. The artifact certifies
    /// the model; this row is what lets a deployment answer for the
    /// bytes it actually runs.
    #[serde(default)]
    pub binary: Option<String>,
    /// Expected SHA-256 of `binary`, hex. Cryptographic where the
    /// in-band `artifact_digest` (FNV, a tripwire) deliberately is
    /// not: this hash is the thing an operator asserts across a
    /// trust boundary.
    #[serde(default)]
    pub binary_sha256: Option<String>,
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
    /// The typed interior (GH #476 Change 7) — decoded once after
    /// admission; composition never crawls generic JSON again.
    model: crate::fleet_model::ComponentModel,
    path: PathBuf,
    /// SHA-256 of the artifact bytes that were ADMITTED — recorded
    /// in the fleet artifact so an auditor can re-check exactly what
    /// the composition read, independent of the in-band FNV digest.
    sha256: String,
    /// key_id that verified this component's sidecar signature, when
    /// trust roots were declared. `None` means composition ran
    /// without trust — recorded rather than omitted, so a reader can
    /// tell "unsigned admission" from "signed and verified".
    signed_by: Option<String>,
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

pub fn compose(
    plan_path: &Path,
    trust: &crate::sign::Trust,
) -> Result<String, Vec<String>> {
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
        match load_artifact(&base.join(&inst.artifact), trust) {
            Ok((model, sha256, signed_by)) => comps.push(Component {
                id: inst.id.clone(),
                labels: inst.labels.clone(),
                model,
                path: base.join(&inst.artifact),
                sha256,
                signed_by,
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
        for n in &c.model.fns {
            vertices.push(format!("{}::{}", c.id, n));
        }
        // BOTH call relations. `calls_via_stdlib` holds user→user
        // paths whose interior is stdlib code, contracted to their
        // user endpoints — the application checker walks the union,
        // so a fleet that read only `calls` would lose a path the
        // component's own claims can see. An intermediate service
        // whose routed handler reaches its routed publisher through
        // `std::http::Router` is invisible in `calls` alone, and a
        // prohibition spanning it would report a false absence.
        // (The typed decode already unions them.)
        for (f, t) in &c.model.calls {
            call_edges.push((
                format!("{}::{}", c.id, f),
                format!("{}::{}", c.id, t),
            ));
        }
        // A local publish→subscribe pair inside one instance is a
        // real in-process edge and stays one.
        for (pf, subj) in &c.model.publishes {
            for (ssubj, l, h) in &c.model.subscribes {
                if ssubj == subj {
                    local_bus.push((
                        format!("{}::{}", c.id, pf),
                        format!("{}::{}::{}", c.id, l, h),
                    ));
                }
            }
        }
    }

    // ---- steps 4-6: routes ----
    let mut routes_out: Vec<String> = Vec::new();
    let mut route_edges: Vec<(String, String, String)> = Vec::new();
    // (id, wire subject, publisher instances, subscriber instances) —
    // what a `require_*` claim needs to ask whether the plan actually
    // DELIVERS a subject, not merely whether somebody could receive it.
    let mut resolved_routes: Vec<(String, String, Vec<String>, Vec<String>)> =
        Vec::new();
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
        // step 5: every endpoint must agree on the wire identity AND
        // actually hold the role the plan gives it.
        //
        // Declaring a topic is not the same as using it: a component
        // that imports the topic module has every topic in its table
        // whether or not any of its code publishes or subscribes one.
        // Checking only the table admits a route with a phantom
        // producer, and a `require_subscribes` law then holds with
        // nothing on the other end — the plan vouching for behavior
        // the code does not have. The artifact is the authority here,
        // never the plan.
        let mut wire: Option<(WireId, String)> = None;
        let mut bad = false;
        let endpoints = r
            .publishers
            .iter()
            .map(|e| (e, true))
            .chain(r.subscribers.iter().map(|e| (e, false)));
        for (ep, publishing) in endpoints {
            let Some(c) = by_id.get(ep.instance.as_str()) else {
                errs.push(format!(
                    "route `{}` names instance `{}`, which the plan \
                     does not declare",
                    r.id, ep.instance
                ));
                bad = true;
                continue;
            };
            match c.model.wire_of(&ep.topic).map(|(s, h)| {
                WireId {
                    subject: s.to_string(),
                    payload_hash: h.to_string(),
                }
            }) {
                None => {
                    errs.push(format!(
                        "route `{}`: instance `{}` declares no topic \
                         `{}`",
                        r.id, ep.instance, ep.topic
                    ));
                    bad = true;
                }
                Some(w) if !c
                    .model
                    .has_topic_endpoint(&ep.topic, publishing) =>
                {
                    errs.push(format!(
                        "route `{}`: instance `{}` is named as a {} of \
                         `{}` (subject `{}`), but nothing in that \
                         component {} it — declaring a topic is not \
                         using it",
                        r.id,
                        ep.instance,
                        if publishing { "publisher" } else { "subscriber" },
                        ep.topic,
                        w.subject,
                        if publishing { "publishes" } else { "subscribes" },
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
            for pf in pc.model.topic_publishers(&p.topic) {
                for s in &r.subscribers {
                    let Some(sc) = by_id.get(s.instance.as_str()) else {
                        continue;
                    };
                    for handler in
                        sc.model.topic_subscribers(&s.topic)
                    {
                        route_edges.push((
                            format!("{}::{}", p.instance, pf),
                            format!(
                                "{}::{}",
                                s.instance, handler
                            ),
                            r.id.clone(),
                        ));
                    }
                }
            }
        }
        resolved_routes.push((
            r.id.clone(),
            w.subject.clone(),
            r.publishers.iter().map(|e| e.instance.clone()).collect(),
            r.subscribers.iter().map(|e| e.instance.clone()).collect(),
        ));
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
    // Instance-qualified vertex -> why its out-edges are incomplete.
    // Serializing these and then evaluating without them is how an
    // indirect call used to remove the only modeled path to a target
    // and leave `forbid_reaches` reporting `holds`.
    let mut holes: Vec<(String, String)> = Vec::new();
    for c in &comps {
        for (f, reasons) in &c.model.unknowns {
            unknowns.push(format!(
                "    {{\"instance\": {}, \"unknown\": {{\"fn\": {}, \"reasons\": [{}]}}}}",
                q(&c.id),
                q(f),
                reasons
                    .iter()
                    .map(|r| q(r))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            for kind in reasons {
                if hale_types::model_graph::kind_hides_edges(kind) {
                    holes.push((
                        format!("{}::{}", c.id, f),
                        kind.to_string(),
                    ));
                }
            }
        }
    }

    // ---- step 9-10: fleet claims over the composed model ----
    let groups = resolve_groups(&plan, &comps, &mut errs);
    if !errs.is_empty() {
        return Err(errs);
    }
    let claim_rows = evaluate_claims(
        &plan, &groups, &comps, &by_id, &call_edges, &local_bus,
        &route_edges, &resolved_routes, &holes, &mut errs,
    );
    if !errs.is_empty() {
        return Err(errs);
    }

    Ok(render(
        &plan, &comps, &vertices, &call_edges, &local_bus, &route_edges,
        &routes_out, &unknowns, &claim_rows,
    ))
}

/// A group's vertices: every vertex of every instance it names, by id
/// or by label. An unknown name is an error, never an empty set — the
/// application tier's rule, and for the same reason: a `forbid`
/// satisfied by an empty quantification domain is a fail-open wearing
/// formal clothing.
fn resolve_groups(
    plan: &FleetPlan,
    comps: &[Component],
    errs: &mut Vec<String>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (name, g) in &plan.groups {
        let mut insts: BTreeSet<String> = BTreeSet::new();
        for id in &g.instances {
            if comps.iter().any(|c| &c.id == id) {
                insts.insert(id.clone());
            } else {
                errs.push(format!(
                    "group `{}` names instance `{}`, which the plan does \
                     not declare",
                    name, id
                ));
            }
        }
        for l in &g.labels {
            let hit: Vec<&Component> =
                comps.iter().filter(|c| c.labels.contains(l)).collect();
            if hit.is_empty() {
                errs.push(format!(
                    "group `{}` names label `{}`, which no instance \
                     carries",
                    name, l
                ));
            }
            for c in hit {
                insts.insert(c.id.clone());
            }
        }
        if insts.is_empty() {
            errs.push(format!(
                "group `{}` resolves to no instances — a claim \
                 quantifying over an empty group holds vacuously",
                name
            ));
        }
        out.insert(name.clone(), insts);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn evaluate_claims(
    plan: &FleetPlan,
    groups: &BTreeMap<String, BTreeSet<String>>,
    comps: &[Component],
    by_id: &BTreeMap<&str, &Component>,
    calls: &[(String, String)],
    local_bus: &[(String, String)],
    routed: &[(String, String, String)],
    resolved_routes: &[(String, String, Vec<String>, Vec<String>)],
    holes: &[(String, String)],
    errs: &mut Vec<String>,
) -> Vec<String> {
    let inst_of = |v: &str| -> String {
        v.split("::").next().unwrap_or("").to_string()
    };
    let group = |n: &str, claim: &str, errs: &mut Vec<String>| {
        match groups.get(n) {
            Some(g) => Some(g.clone()),
            None => {
                errs.push(format!(
                    "claim `{}` names group `{}`, which the plan does \
                     not declare",
                    claim, n
                ));
                None
            }
        }
    };
    // The composed edge set: interior calls, interior bus hops, and
    // the explicit routes. Nothing else — a route is the ONLY way a
    // vertex in one instance reaches one in another.
    let mut graph = hale_types::model_graph::ModelGraph::new();
    for (f, t) in calls.iter().chain(local_bus.iter()) {
        graph.add_edge(f.as_str(), t.as_str(), None);
    }
    for (f, t, r) in routed {
        graph.add_edge(f.as_str(), t.as_str(), Some(r.clone()));
    }
    // Uncertainty is part of the model, not a footnote beside it.
    for (v, why) in holes {
        graph.add_hole(v.as_str(), why.as_str());
    }

    // Fleet groups name INSTANCES; the graph is over the vertices
    // those instances contain, which is the same projection the
    // application tier makes from a locus to its methods.
    let all_vertices: Vec<String> = comps
        .iter()
        .flat_map(|c| {
            c.model
                .fns
                .iter()
                .map(|n| format!("{}::{}", c.id, n))
                .collect::<Vec<_>>()
        })
        .collect();
    let expand = |insts: &BTreeSet<String>| -> BTreeSet<String> {
        all_vertices
            .iter()
            .filter(|v| insts.contains(&inst_of(v)))
            .cloned()
            .collect()
    };

    // GH #476 Change 4: the plan's law rows lower to ClaimIr
    // alongside the evaluation (lowering only — this evaluator
    // stays authoritative until Change 7's FleetModel). The trace
    // line is the same demand-proof surface the model builder uses.
    if std::env::var("HALE_MODEL_TRACE").as_deref() == Ok("1") {
        let lowered = lower_plan_claims(&plan.claims);
        eprintln!(
            "[hale-model] lowered {} fleet claim row(s)",
            lowered.rows.len()
        );
    }

    let mut rows: Vec<String> = Vec::new();
    for c in &plan.claims {
        // A claim is ONE sentence, exactly as it is in source.
        //
        // Every verb is an `Option`, so a plan could set several and
        // the evaluator — an if/else chain producing one row — would
        // silently judge whichever came first in this file and ignore
        // the rest. A claim pairing a holding `require_subscribes`
        // with an impossible `count ... eq: 999` passed. Worse, the
        // name that survived into the artifact was the whole claim's,
        // so the record said a sentence held when half of it was
        // never read.
        //
        // Refusing the shape is better than evaluating each verb: one
        // name must mean one normalized form, or the claim name stops
        // being the contract of record.
        let named: Vec<&str> = [
            ("forbid_reaches", c.forbid_reaches.is_some()),
            ("require_subscribes", c.require_subscribes.is_some()),
            ("require_publishes", c.require_publishes.is_some()),
            (
                "count_publisher_instances",
                c.count_publisher_instances.is_some(),
            ),
            (
                "count_subscriber_instances",
                c.count_subscriber_instances.is_some(),
            ),
            ("only_edges", c.only_edges.is_some()),
        ]
        .iter()
        .filter(|(_, set)| *set)
        .map(|(n, _)| *n)
        .collect();
        if named.len() > 1 {
            errs.push(format!(
                "claim `{}` names {} verbs ({}) — a claim is one \
                 sentence; split it into one claim per verb so each \
                 has its own name and verdict",
                c.name,
                named.len(),
                named.join(", ")
            ));
            continue;
        }
        let (result, witness) = if let Some(fr) = &c.forbid_reaches {
            let (Some(src), Some(dst)) = (
                group(&fr.from, &c.name, errs),
                group(&fr.to, &c.name, errs),
            ) else {
                continue;
            };
            let masked = match &fr.avoiding {
                Some(m) => match group(m, &c.name, errs) {
                    Some(g) => g,
                    None => continue,
                },
                None => BTreeSet::new(),
            };
            // Masking an endpoint deletes the quantification domain,
            // and an empty domain makes a prohibition hold over
            // nothing. The application tier rejects this rather than
            // reporting a green vacuous law; so does this one.
            let mask_hits: Vec<&str> = masked
                .iter()
                .filter(|i| src.contains(*i) || dst.contains(*i))
                .map(|s| s.as_str())
                .collect();
            if !mask_hits.is_empty() {
                errs.push(format!(
                    "claim `{}`: `avoiding` group `{}` contains {}, \
                     which is already an endpoint of this claim — \
                     masking an endpoint removes what the claim \
                     quantifies over and would make it hold over \
                     nothing",
                    c.name,
                    fr.avoiding.as_deref().unwrap_or(""),
                    mask_hits.join(", ")
                ));
                continue;
            }
            // An instance in BOTH groups already reaches the
            // forbidden set by standing still. The shared engine
            // tests roots, so it would now return a one-vertex path
            // on its own — correctness no longer depends on this
            // check. It stays because a plan-level message naming
            // both group names is more useful here than a witness
            // consisting of a single vertex.
            let both: Vec<&str> = src
                .intersection(&dst)
                .map(|s| s.as_str())
                .collect();
            if !both.is_empty() {
                (
                    "violated",
                    format!(
                        "{} is in both `{}` and `{}` — a zero-length \
                         path: the source already IS the forbidden \
                         destination",
                        both.join(", "),
                        fr.from,
                        fr.to
                    ),
                )
            } else {
                use hale_types::model_graph::Reach;
                match graph.reaches(
                    &expand(&src),
                    &expand(&dst),
                    &expand(&masked),
                ) {
                    Reach::None => ("holds", String::new()),
                    Reach::Path(path) => (
                        "violated",
                        render_witness(&path, comps, by_id),
                    ),
                    // An absence nobody could see is not an absence.
                    Reach::Uncertified(h) => (
                        "uncertified",
                        format!(
                            "cannot certify this absence: `{}` has \
                             outgoing edges this model cannot see \
                             ({}), and it is reachable from `{}` — \
                             the missing edges could lead to `{}`",
                            h.at, h.why, fr.from, fr.to
                        ),
                    ),
                }
            }
        } else if let Some(r) = &c.require_subscribes {
            endpoint_claim(r, groups, comps, resolved_routes, false, &c.name, errs)
        } else if let Some(r) = &c.require_publishes {
            endpoint_claim(r, groups, comps, resolved_routes, true, &c.name, errs)
        } else if let Some(k) = &c.count_publisher_instances {
            if !bounded(k, &c.name, errs) {
                continue;
            }
            count_claim(k, comps, true)
        } else if let Some(k) = &c.count_subscriber_instances {
            if !bounded(k, &c.name, errs) {
                continue;
            }
            count_claim(k, comps, false)
        } else if let Some(oe) = &c.only_edges {
            let (Some(src), Some(dst)) = (
                group(&oe.from, &c.name, errs),
                group(&oe.to, &c.name, errs),
            ) else {
                continue;
            };
            let granted: BTreeSet<&str> =
                oe.grant_subjects.iter().map(String::as_str).collect();
            let mut bad = Vec::new();
            for (f, t, rid) in routed {
                if src.contains(&inst_of(f)) && dst.contains(&inst_of(t)) {
                    let subj = plan
                        .routes
                        .iter()
                        .find(|r| &r.id == rid)
                        .and_then(|r| {
                            r.publishers.first().and_then(|ep| {
                                by_id.get(ep.instance.as_str()).and_then(
                                    |c| c.model.wire_of(&ep.topic),
                                )
                            })
                        })
                        .map(|(subject, _)| subject.to_string())
                        .unwrap_or_default();
                    if !granted.contains(subj.as_str()) {
                        bad.push(format!(
                            "{} -(route `{}`, subject `{}`)-> {}",
                            f, rid, subj, t
                        ));
                    }
                }
            }
            if bad.is_empty() {
                ("holds", String::new())
            } else {
                ("violated", bad.join("; "))
            }
        } else {
            errs.push(format!(
                "claim `{}` names no verb — one of forbid_reaches, \
                 require_subscribes, require_publishes, \
                 count_publisher_instances, count_subscriber_instances, \
                 only_edges",
                c.name
            ));
            continue;
        };
        // Round 5–6 (#490): the component's positive completeness
        // account is HONORED — with the POLARITY of the law.
        // `forbid_reaches` / `only_edges` certify an ABSENCE, so
        // incomplete knowledge in an involved component prevents
        // the proof (holds → uncertified). The endpoint and count
        // forms handle completeness INSIDE their evaluators: a
        // known routed witness is a positive fact no incomplete
        // set can erase, and counts evaluate over a
        // [known, known+hidden] interval. The scoping mirrors the
        // unreachable-unknown rule. Serialized AFTER this final
        // verdict (round 6: the row previously recorded the
        // pre-rewrite result).
        let negative_family = if c.forbid_reaches.is_some() {
            Some("reachability")
        } else if c.only_edges.is_some() {
            Some("boundary")
        } else {
            None
        };
        let (result, witness) = if let (Some(family), "holds") =
            (negative_family, result)
        {
            let involved = |k: &Component| -> bool {
                if let Some(fr) = &c.forbid_reaches {
                    let (Some(src), Some(dst)) = (
                        groups.get(&fr.from),
                        groups.get(&fr.to),
                    ) else {
                        return true;
                    };
                    if src.contains(&k.id)
                        || dst.contains(&k.id)
                    {
                        return true;
                    }
                    let masked = fr
                        .avoiding
                        .as_ref()
                        .and_then(|m| groups.get(m))
                        .cloned()
                        .unwrap_or_default();
                    let mine: BTreeSet<String> = k
                        .model
                        .fns
                        .iter()
                        .map(|n| format!("{}::{}", k.id, n))
                        .collect();
                    !matches!(
                        graph.reaches(
                            &expand(src),
                            &mine,
                            &expand(&masked),
                        ),
                        hale_types::model_graph::Reach::None
                    )
                } else if let Some(oe) = &c.only_edges {
                    [&oe.from, &oe.to].iter().any(|g| {
                        groups
                            .get(*g)
                            .is_some_and(|g| g.contains(&k.id))
                    })
                } else {
                    true
                }
            };
            let degraded: Vec<String> = comps
                .iter()
                .filter(|k| {
                    k.model
                        .adequacy
                        .get(family)
                        .map(String::as_str)
                        == Some("degraded")
                        && involved(k)
                })
                .map(|k| {
                    let withdrawn: Vec<&str> = k
                        .model
                        .capabilities
                        .iter()
                        .filter(|(_, on)| !**on)
                        .map(|(f, _)| f.as_str())
                        .collect();
                    format!(
                        "`{}` (withdraws {})",
                        k.id,
                        withdrawn.join(", ")
                    )
                })
                .collect();
            if degraded.is_empty() {
                (result, witness)
            } else {
                (
                    "uncertified",
                    format!(
                        "cannot certify this absence: \
                         instance(s) {} carry a degraded `{}` \
                         adequacy — the required relations are \
                         not vouched by their own artifacts",
                        degraded.join(", "),
                        family
                    ),
                )
            }
        } else {
            (result, witness)
        };
        rows.push(format!(
            "    {{\"name\": {}, \"result\": {}, \"witness\": {}}}",
            q(&c.name),
            q(result),
            q(&witness)
        ));
        // `uncertified` fails too. A law that could not be checked
        // has not been satisfied — the distinction from `violated`
        // is recorded because the REPAIR differs (resolve the
        // unknown edge vs. fix the program), not because one of them
        // passes.
        if result == "violated" || result == "uncertified" {
            errs.push(format!(
                "fleet claim `{}` {}{}{}",
                c.name,
                result,
                if witness.is_empty() { "" } else { " — witness:\n  " },
                witness
            ));
        }
    }
    rows
}

/// A witness that crosses artifacts, naming the source file of each
/// hop. Phase 0's source maps are what make this renderable: a bare
/// bundle-global offset could not be turned into a location by
/// anything outside the process that produced it.
fn render_witness(
    path: &[(String, Option<String>)],
    _comps: &[Component],
    by_id: &BTreeMap<&str, &Component>,
) -> String {
    let mut out = String::new();
    for (i, (v, via)) in path.iter().enumerate() {
        if i > 0 {
            match via {
                Some(r) => {
                    out.push_str(&format!("\n  -(route `{}`)->\n  ", r))
                }
                None => out.push_str("\n  ->\n  "),
            }
        }
        out.push_str(v);
        // the file this vertex lives in, from its component's map
        let inst = v.split("::").next().unwrap_or("");
        let local = v.strip_prefix(&format!("{}::", inst)).unwrap_or(v);
        if let Some(c) = by_id.get(inst) {
            if let Some(loc) = c.model.decl_location(local) {
                out.push_str(&format!("  [{}]", loc));
            }
        }
    }
    out
}


/// `require subscribes/publishes` is a STRUCTURAL DEPLOYMENT
/// statement: some instance in the group exposes the endpoint **and
/// the plan connects it**.
///
/// Checking only that the endpoint exists was a fail-open, and a real
/// deployment found it: a slice where the ledger subscribes
/// `exec.fill` but nothing in the plan publishes it reported `holds`.
/// The law "fills must reach the ledger" then cannot catch a missing
/// route, which is the one thing it is for. A synthetic fixture hides
/// this because whoever writes it routes everything they assert.
fn endpoint_claim(
    r: &RequireEndpoint,
    groups: &BTreeMap<String, BTreeSet<String>>,
    comps: &[Component],
    resolved_routes: &[(String, String, Vec<String>, Vec<String>)],
    publishing: bool,
    claim: &str,
    errs: &mut Vec<String>,
) -> (&'static str, String) {
    let Some(g) = groups.get(&r.group) else {
        errs.push(format!(
            "claim `{}` names group `{}`, which the plan does not \
             declare",
            claim, r.group
        ));
        return ("invalid", String::new());
    };
    let verb = if publishing { "publishes" } else { "subscribes" };
    let exposing: Vec<&str> = comps
        .iter()
        .filter(|c| {
            g.contains(&c.id)
                && c.model.has_endpoint(&r.subject, publishing)
        })
        .map(|c| c.id.as_str())
        .collect();
    if exposing.is_empty() {
        return (
            "violated",
            format!("no instance in `{}` {} `{}`", r.group, verb, r.subject),
        );
    }
    // …and a route must actually carry it to (or from) one of them.
    let connected = resolved_routes.iter().any(|(_, subj, pubs, subs)| {
        subj == &r.subject
            && {
                let side = if publishing { pubs } else { subs };
                side.iter().any(|i| exposing.contains(&i.as_str()))
            }
            // a route with an empty other side delivers nothing
            && !pubs.is_empty()
            && !subs.is_empty()
    });
    if connected {
        ("holds", String::new())
    } else {
        (
            "violated",
            format!(
                "`{}` {} `{}`, but no route in this plan carries it {} \
                 them — the endpoint exists and nothing connects it, so \
                 the traffic this claim is about does not flow",
                exposing.join(", "),
                verb,
                r.subject,
                if publishing { "from" } else { "to" }
            ),
        )
    }
}

/// A cardinality claim must actually bound something.
///
/// With `eq`/`min`/`max` all absent every comparison defaults to
/// true, so the claim held against any fleet whatsoever — a
/// `count_publisher_instances` naming only a subject read like a
/// real law and asserted nothing. A claim naming NO verb was already
/// refused; a verb naming no bound is the same emptiness one level
/// down, and gets the same answer rather than a silent pass.
fn bounded(k: &CountSpec, name: &str, errs: &mut Vec<String>) -> bool {
    if k.eq.is_none() && k.min.is_none() && k.max.is_none() {
        errs.push(format!(
            "claim `{}` counts `{}` but names no bound — give it \
             one of eq, min, max",
            name, k.subject
        ));
        return false;
    }
    true
}

fn count_claim(
    k: &CountSpec,
    comps: &[Component],
    publishing: bool,
) -> (&'static str, String) {
    // Round 6 (#490): counts are evaluated over an INTERVAL, with
    // the canonical monotone rule. Known endpoint rows are a lower
    // bound; an uncounted component that withdraws the RELEVANT
    // completeness (publisher counts consult publish +
    // cardinality; subscriber counts subscribe + cardinality)
    // could hide another endpoint, so it raises only the upper
    // bound. A `min` already met by known rows holds regardless
    // of hidden candidates; a `max` already exceeded by known
    // rows violates regardless; everything the interval cannot
    // decide is uncertified.
    let hits: Vec<&str> = comps
        .iter()
        .filter(|c| c.model.has_endpoint(&k.subject, publishing))
        .map(|c| c.id.as_str())
        .collect();
    let relevant = if publishing {
        "exact_publishes"
    } else {
        "exact_subscribes"
    };
    let hidden: Vec<&str> = comps
        .iter()
        .filter(|c| {
            !hits.contains(&c.id.as_str())
                && (!c
                    .model
                    .capabilities
                    .get(relevant)
                    .copied()
                    .unwrap_or(true)
                    || !c
                        .model
                        .capabilities
                        .get("exact_cardinality")
                        .copied()
                        .unwrap_or(true))
        })
        .map(|c| c.id.as_str())
        .collect();
    let lo = hits.len();
    let hi = lo + hidden.len();
    // Per bound: Ok(true) definite pass, Ok(false) definite
    // violation, Err(()) undecidable on this interval.
    let bounds: Vec<(&str, Result<bool, ()>)> = [
        ("eq", k.eq.map(|e| {
            if lo > e || hi < e {
                Ok(false)
            } else if lo == e && hi == e {
                Ok(true)
            } else {
                Err(())
            }
        })),
        ("max", k.max.map(|m| {
            if lo > m {
                Ok(false)
            } else if hi <= m {
                Ok(true)
            } else {
                Err(())
            }
        })),
        ("min", k.min.map(|m| {
            if lo >= m {
                Ok(true)
            } else if hi < m {
                Ok(false)
            } else {
                Err(())
            }
        })),
    ]
    .into_iter()
    .filter_map(|(n, v)| v.map(|v| (n, v)))
    .collect();
    let counted = format!(
        "counted {} deployed {} endpoint(s) of `{}`{}{}",
        lo,
        if publishing { "publisher" } else { "subscriber" },
        k.subject,
        if hits.is_empty() {
            String::new()
        } else {
            format!(": {}", hits.join(", "))
        },
        if hidden.is_empty() {
            String::new()
        } else {
            format!(
                " (up to {} more possible — {} withdraw(s) the \
                 relevant completeness)",
                hidden.len(),
                hidden.join(", ")
            )
        }
    );
    // Conjunctive fleet form: any definite violation violates;
    // all definite passes hold; otherwise uncertified.
    if bounds.iter().any(|(_, v)| *v == Ok(false)) {
        ("violated", counted)
    } else if bounds.iter().all(|(_, v)| *v == Ok(true)) {
        ("holds", String::new())
    } else {
        ("uncertified", counted)
    }
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
    claims: &[String],
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
            q(&c.model.shape_hash),
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
    out.push_str(",\n  \"claims\": [\n");
    out.push_str(&claims.join(",\n"));
    out.push_str("\n  ],\n  \"unknowns\": [\n");
    out.push_str(&unknowns.join(",\n"));
    out.push_str("\n  ],\n  \"components\": [\n");
    out.push_str(
        &comps
            .iter()
            .map(|c| {
                // Unhashed provenance, like everything else in this
                // section. `sha256` is what was ADMITTED; `signed_by`
                // distinguishes "verified under this key" from
                // "composed without trust roots" — null is a fact
                // here, not an omission.
                format!(
                    "    {{\"id\": {}, \"artifact\": {}, \"sha256\": {}, \
                     \"signed_by\": {}}}",
                    q(&c.id),
                    q(&c.path.display().to_string()),
                    q(&c.sha256),
                    c.signed_by
                        .as_ref()
                        .map(|k| q(k))
                        .unwrap_or_else(|| "null".into())
                )
            })
            .collect::<Vec<_>>()
            .join(",\n"),
    );
    out.push_str("\n  ]\n}\n");
    out
}

/// `hale fleet attest <plan>` — the binary-digest half of Phase 7.
///
/// The composition certifies the model; attest answers a different
/// question: are the executables this plan deploys the ones the
/// operator hashed? All-or-nothing over the plan's instances — an
/// attestation that skips an instance it cannot check is a partial
/// answer wearing a full answer's exit code, so a missing `binary`
/// or `binary_sha256` row is a refusal, not a skip.
///
/// Out of scope, on purpose: whether a RUNNING process still is
/// that binary. This checks bytes at rest at deploy time; runtime
/// self-attestation is obs territory (7b).
pub fn attest(plan_path: &Path) -> Result<String, Vec<String>> {
    let plan = read_plan(plan_path)?;
    let base = plan_path.parent().unwrap_or(Path::new("."));
    let mut errs: Vec<String> = Vec::new();
    let mut ok: usize = 0;
    for inst in &plan.instances {
        let (Some(bin), Some(expect)) =
            (&inst.binary, &inst.binary_sha256)
        else {
            errs.push(format!(
                "instance `{}` declares no {} — every instance must \
                 carry `binary` and `binary_sha256` for the plan to be \
                 attestable; a partial attestation would report \
                 coverage it does not have",
                inst.id,
                match (&inst.binary, &inst.binary_sha256) {
                    (None, None) => "`binary` / `binary_sha256`",
                    (None, _) => "`binary`",
                    _ => "`binary_sha256`",
                }
            ));
            continue;
        };
        let path = base.join(bin);
        match crate::sign::sha256_file(&path) {
            Err(e) => errs.push(format!("instance `{}`: {}", inst.id, e)),
            Ok(actual) => {
                if actual.eq_ignore_ascii_case(expect) {
                    ok += 1;
                } else {
                    errs.push(format!(
                        "instance `{}`: {} is not the binary the plan \
                         names — sha256 {} where the plan says {}",
                        inst.id,
                        path.display(),
                        actual,
                        expect
                    ));
                }
            }
        }
    }
    if errs.is_empty() {
        Ok(format!(
            "ok: fleet `{}` attested — {} binary(ies) match the plan",
            plan.name, ok
        ))
    } else {
        Err(errs)
    }
}

fn read_plan(p: &Path) -> Result<FleetPlan, Vec<String>> {
    let src = std::fs::read_to_string(p)
        .map_err(|e| vec![format!("read {}: {}", p.display(), e)])?;
    let plan: FleetPlan = serde_json::from_str(&src)
        .map_err(|e| vec![format!("parse {}: {}", p.display(), e)])?;
    if !READABLE_PLAN_SCHEMAS.contains(&plan.schema.as_str()) {
        return Err(vec![format!(
            "{}: plan schema `{}`, this build understands `{}`",
            p.display(),
            plan.schema,
            READABLE_PLAN_SCHEMAS.join("`, `")
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
/// cannot honestly build on. Returns the parsed artifact, the
/// SHA-256 of the admitted bytes, and the key_id that signed them
/// (when trust roots are declared).
fn load_artifact(
    p: &Path,
    trust: &crate::sign::Trust,
) -> Result<
    (crate::fleet_model::ComponentModel, String, Option<String>),
    String,
> {
    let src = std::fs::read_to_string(p)
        .map_err(|e| format!("read {}: {}", p.display(), e))?;
    // Provenance BEFORE integrity BEFORE meaning. The signature is
    // checked first because it covers the exact bytes everything
    // after it reads — and because trust roots, once declared, make
    // an unsigned component inadmissible outright (GH #408 Phase 7):
    // declaring a trust set and then quietly composing unsigned
    // artifacts would be the vacuity this system exists to refuse.
    let signed_by = if trust.is_empty() {
        None
    } else {
        Some(
            trust
                .verify(src.as_bytes(), &crate::sign::sidecar(p))
                .map_err(|e| e.to_string())?,
        )
    };
    let sha256 = {
        use std::fmt::Write as _;
        let d = openssl::sha::sha256(src.as_bytes());
        let mut s = String::with_capacity(64);
        for b in d {
            let _ = write!(s, "{:02x}", b);
        }
        s
    };
    // Integrity BEFORE meaning. `shape_hash` is an identity covering
    // the model half only, so it cannot vouch for the `topics` rows a
    // composition joins on; the whole-body digest can.
    // One unambiguous value per key (round 3, #490): serde's
    // last-wins map parse must not be able to shadow what the raw
    // verifiers below check.
    match hale_types::topology::scan_top_level(&src) {
        Err(e) => {
            return Err(format!(
                "{}: {} — the verified and consumed values \
                 could disagree",
                p.display(),
                e
            ));
        }
        Ok(top) => {
            if let Err(e) =
                hale_types::topology::verify_top_level_order(
                    &top,
                )
            {
                return Err(format!("{}: {}", p.display(), e));
            }
        }
    }
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
    // Identity BEFORE meaning too (round 2, #490): the declared
    // shape_hash must recompute from the hashed model half, or the
    // wire identity could drift under a stale identity — the exact
    // residual schema 1.12 closes.
    match hale_types::topology::verify_shape_hash(&src) {
        Some(true) => {}
        Some(false) => {
            return Err(format!(
                "{}: shape_hash does not recompute from the \
                 model half — the declared identity is stale",
                p.display()
            ))
        }
        None => {
            return Err(format!(
                "{}: no recomputable shape_hash",
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
    // GH #476 Change 6 (rounds 4–5): the ONE shared law-account
    // admission — the same routine Track A runs, so there is
    // exactly one definition of "admitted schema-1.11 artifact".
    // It decodes every payload, binds evidence, joins claims↔law
    // in both directions, and RECOMPUTES the document verdict.
    crate::topology_law::validate_law_account(
        &v,
        &p.display().to_string(),
    )?;
    // GH #476 Change 7: decode the typed interior ONCE — the last
    // time this artifact's JSON is touched.
    let model = crate::fleet_model::ComponentModel::decode(
        &v,
        &p.display().to_string(),
    )?;
    Ok((model, sha256, signed_by))
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

/// GH #476 Change 4 — lower the plan's claim rows to `ClaimIr`.
/// ONE row per `ClaimSpec`, mirroring the evaluator's
/// one-name/one-sentence rule (a claim name identifies one sentence
/// and one verdict): a spec with zero or several verbs set lowers to
/// a structured [`hale_model::LoweringIssue`], never to a split or
/// silently dropped law, and a `CountSpec`'s eq/max/min bounds stay
/// ONE conjunctive law. Fleet targets are PLAN-level names
/// (instances, plan groups, wire subjects); they resolve against a
/// typed `FleetModel` at Change 7, so every row lowers name-level
/// and the old evaluator stays authoritative.
pub fn lower_plan_claims(
    claims: &[ClaimSpec],
) -> hale_model::ClaimIrTable {
    use hale_model::{
        ClaimIr, ClaimIrTable, ClaimOrigin, ClaimRow, ProvenanceId,
    };
    let mut table = ClaimIrTable::default();
    let prov = |table: &mut ClaimIrTable, name: &str| {
        let pid = ProvenanceId(table.provenance.records.len() as u32);
        table.provenance.records.push(
            hale_model::Provenance::Synthetic {
                origin: format!("deployment plan claim `{}`", name),
            },
        );
        pid
    };
    for c in claims {
        let verbs = [
            c.forbid_reaches.is_some(),
            c.require_subscribes.is_some(),
            c.require_publishes.is_some(),
            c.count_publisher_instances.is_some(),
            c.count_subscriber_instances.is_some(),
            c.only_edges.is_some(),
        ]
        .iter()
        .filter(|v| **v)
        .count();
        if verbs != 1 {
            // The evaluator's one-name/one-sentence rule: this spec
            // is not one law. Structured invalidity, not a split.
            let pid = prov(&mut table, &c.name);
            table.issues.push(hale_model::LoweringIssue {
                family: Some(hale_model::JudgmentFamily::Fleet),
                message: format!(
                    "plan claim `{}` sets {} verbs — a claim is ONE \
                     sentence with one verdict",
                    c.name, verbs
                ),
                provenance: pid,
            });
            continue;
        }
        let law = if let Some(fr) = &c.forbid_reaches {
            ClaimIr::FleetForbidReaches {
                from: fr.from.clone(),
                to: fr.to.clone(),
                avoiding: fr.avoiding.clone(),
            }
        } else if let Some(r) = &c.require_subscribes {
            ClaimIr::FleetRequireEndpoint {
                publishers: false,
                target: r.group.clone(),
                topic: r.subject.clone(),
            }
        } else if let Some(r) = &c.require_publishes {
            ClaimIr::FleetRequireEndpoint {
                publishers: true,
                target: r.group.clone(),
                topic: r.subject.clone(),
            }
        } else if let Some(spec) = &c.count_publisher_instances {
            ClaimIr::FleetCountInstances {
                publishers: true,
                topic: spec.subject.clone(),
                eq: spec.eq.map(|n| n as u64),
                max: spec.max.map(|n| n as u64),
                min: spec.min.map(|n| n as u64),
            }
        } else if let Some(spec) = &c.count_subscriber_instances {
            ClaimIr::FleetCountInstances {
                publishers: false,
                topic: spec.subject.clone(),
                eq: spec.eq.map(|n| n as u64),
                max: spec.max.map(|n| n as u64),
                min: spec.min.map(|n| n as u64),
            }
        } else {
            let Some(oe) = &c.only_edges else { unreachable!() };
            ClaimIr::FleetOnlyEdges {
                src: oe.from.clone(),
                dst: oe.to.clone(),
                grants: oe.grant_subjects.clone(),
            }
        };
        let pid = prov(&mut table, &c.name);
        let ordinal = table.rows.len() as u32;
        table.rows.push(ClaimRow {
            ordinal,
            name: c.name.clone(),
            origin: ClaimOrigin::FleetPlan,
            law,
            provenance: pid,
        });
    }
    table
}

#[cfg(test)]
mod claim_ir_tests {
    use super::*;
    use hale_model::{ClaimIr, ClaimOrigin};

    fn empty_spec(name: &str) -> ClaimSpec {
        ClaimSpec {
            name: name.to_string(),
            forbid_reaches: None,
            require_subscribes: None,
            require_publishes: None,
            count_publisher_instances: None,
            count_subscriber_instances: None,
            only_edges: None,
        }
    }

    /// GH #476 Change 4 (round 15): ONE row per ClaimSpec — the
    /// evaluator's one-name/one-sentence rule. A multi-bound count
    /// stays one conjunctive law; a multi-verb or verbless spec
    /// lowers to a structured issue, never a split or a silent drop.
    #[test]
    fn plan_claims_lower_one_row_per_sentence() {
        let mut iso = empty_spec("iso");
        iso.forbid_reaches = Some(ForbidReaches {
            from: "gw-0".to_string(),
            to: "risk-0".to_string(),
            avoiding: Some("audit".to_string()),
        });
        let mut writers = empty_spec("writers");
        writers.count_publisher_instances = Some(CountSpec {
            subject: "orders".to_string(),
            eq: None,
            max: Some(3),
            min: Some(1),
        });
        let mut broken = empty_spec("broken");
        broken.require_publishes = Some(RequireEndpoint {
            group: "gw".to_string(),
            subject: "orders".to_string(),
        });
        broken.only_edges = Some(OnlyEdges {
            from: "gw".to_string(),
            to: "risk".to_string(),
            grant_subjects: vec![],
        });
        let hollow = empty_spec("hollow");
        let t = lower_plan_claims(&[iso, writers, broken, hollow]);
        assert_eq!(t.rows.len(), 2, "one row per well-formed sentence");
        assert!(t
            .rows
            .iter()
            .all(|r| r.origin == ClaimOrigin::FleetPlan));
        assert!(matches!(
            &t.rows[0].law,
            ClaimIr::FleetForbidReaches { avoiding: Some(a), .. } if a == "audit"
        ));
        // The two-bound count is ONE conjunctive law.
        assert!(matches!(
            &t.rows[1].law,
            ClaimIr::FleetCountInstances {
                eq: None,
                max: Some(3),
                min: Some(1),
                ..
            }
        ));
        // Multi-verb and verbless specs are structured invalidity.
        assert_eq!(t.issues.len(), 2);
        assert!(t.issues[0].message.contains("`broken` sets 2 verbs"));
        assert!(t.issues[1].message.contains("`hollow` sets 0 verbs"));
    }
}

