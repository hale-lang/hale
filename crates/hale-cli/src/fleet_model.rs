//! GH #476 Change 7 — the typed component decode behind
//! `FleetModel`.
//!
//! Composition is the one consumer that reads artifacts it did not
//! produce, across a trust boundary. Everything it needs is decoded
//! ONCE, here, into typed rows — after signature, integrity,
//! semantics, verdict, and the shared law-account admission have
//! all passed — and the rest of fleet composition never touches
//! generic JSON for a modeled fact again (the #476 architecture
//! canary: "artifact/fleet code cannot walk source or generic JSON
//! for a modeled fact after cutover").
//!
//! The decode reads the SAME sections admission validated:
//! `sorts.fns`, both call relations, the V1 publish/subscribe
//! relations, the `topics` join surface, `unknowns`, the decl
//! provenance, and the hashed `endpoint_identity` (schema 1.12) —
//! interiors preserved exactly, at the grain composition composes.

use std::collections::BTreeMap;

use serde_json::Value;

/// One declared topic's join surface: the wire subject is the
/// cross-binary identity, the payload hash the compatibility check.
#[derive(Clone, Debug)]
pub struct TopicRow {
    pub name: String,
    pub subject: String,
    pub payload_hash: String,
}

/// One row of the HASHED endpoint identity (schema 1.12).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EndpointIdentity {
    pub publish: bool,
    /// Owning fn/handler for site rows; owning locus for
    /// declaration rows.
    pub owner: String,
    /// `None` for declared-publisher rows.
    pub site: Option<u64>,
    pub wire: String,
    pub topic: Option<String>,
}

/// The typed interior of ONE admitted component artifact — exactly
/// the facts fleet composition consumes, decoded once.
#[derive(Clone, Debug)]
pub struct ComponentModel {
    pub shape_hash: String,
    /// `sorts.fns` — the component's vertex universe.
    pub fns: Vec<String>,
    /// `calls` ∪ `calls_via_stdlib`, (from, to) — the union the
    /// component's own checker walks.
    pub calls: Vec<(String, String)>,
    /// V1 publish relation, (fn, subject-name).
    pub publishes: Vec<(String, String)>,
    /// V1 subscribe relation, (subject-name, locus, handler).
    pub subscribes: Vec<(String, String, String)>,
    pub topics: Vec<TopicRow>,
    /// `unknowns` — (fn, reasons): the component's own residue,
    /// surfaced as fleet holes.
    pub unknowns: Vec<(String, Vec<String>)>,
    /// decl name → source path (for witness locations).
    pub decl_sources: BTreeMap<String, String>,
    /// The hashed endpoint identity rows (schema 1.12).
    pub endpoints: Vec<EndpointIdentity>,
}

impl ComponentModel {
    /// Decode an ADMITTED artifact (the caller has already run
    /// signature/integrity/semantics/verdict checks and the shared
    /// law-account admission — this is a projection, not a
    /// validation pass).
    pub fn decode(v: &Value, label: &str) -> Result<Self, String> {
        // STRICT and FALLIBLE (round 3, #490): a malformed
        // semantic row is REFUSED, never filtered — a dropped
        // call edge or hole reason is exactly the silent-omission
        // failure #476 exists to eliminate. Only genuinely
        // optional facts (provenance, unplaceable decl sources)
        // may be absent.
        let req_str = |x: &Value,
                       what: &str|
         -> Result<String, String> {
            x.as_str().map(str::to_string).ok_or_else(|| {
                format!("{}: {} must be a string", label, what)
            })
        };
        let req_arr = |x: &Value,
                       what: &str|
         -> Result<Vec<Value>, String> {
            x.as_array().cloned().ok_or_else(|| {
                format!("{}: {} must be an array", label, what)
            })
        };
        let shape_hash =
            req_str(&v["shape_hash"], "shape_hash")?;
        let mut fns: Vec<String> = Vec::new();
        for (i, f) in
            req_arr(&v["sorts"]["fns"], "sorts.fns")?
                .iter()
                .enumerate()
        {
            fns.push(req_str(
                f,
                &format!("sorts.fns[{}]", i),
            )?);
        }
        let mut calls: Vec<(String, String)> = Vec::new();
        for rel in ["calls", "calls_via_stdlib"] {
            for (i, e) in req_arr(
                &v["relations"][rel],
                &format!("relations.{}", rel),
            )?
            .iter()
            .enumerate()
            {
                calls.push((
                    req_str(
                        &e["from"],
                        &format!(
                            "relations.{}[{}].from",
                            rel, i
                        ),
                    )?,
                    req_str(
                        &e["to"],
                        &format!(
                            "relations.{}[{}].to",
                            rel, i
                        ),
                    )?,
                ));
            }
        }
        let mut publishes: Vec<(String, String)> = Vec::new();
        for (i, p) in req_arr(
            &v["relations"]["publishes"],
            "relations.publishes",
        )?
        .iter()
        .enumerate()
        {
            publishes.push((
                req_str(
                    &p["fn"],
                    &format!("relations.publishes[{}].fn", i),
                )?,
                req_str(
                    &p["subject"],
                    &format!(
                        "relations.publishes[{}].subject",
                        i
                    ),
                )?,
            ));
        }
        let mut subscribes: Vec<(String, String, String)> =
            Vec::new();
        for (i, r) in req_arr(
            &v["relations"]["subscribes"],
            "relations.subscribes",
        )?
        .iter()
        .enumerate()
        {
            let at = format!("relations.subscribes[{}]", i);
            subscribes.push((
                req_str(
                    &r["subject"],
                    &format!("{}.subject", at),
                )?,
                req_str(&r["locus"], &format!("{}.locus", at))?,
                req_str(
                    &r["handler"],
                    &format!("{}.handler", at),
                )?,
            ));
        }
        let mut topics: Vec<TopicRow> = Vec::new();
        for (i, t) in
            req_arr(&v["topics"], "topics")?.iter().enumerate()
        {
            let at = format!("topics[{}]", i);
            topics.push(TopicRow {
                name: req_str(
                    &t["name"],
                    &format!("{}.name", at),
                )?,
                subject: req_str(
                    &t["subject"],
                    &format!("{}.subject", at),
                )?,
                payload_hash: req_str(
                    &t["payload_hash"],
                    &format!("{}.payload_hash", at),
                )?,
            });
        }
        let mut unknowns: Vec<(String, Vec<String>)> =
            Vec::new();
        for (i, u) in req_arr(&v["unknowns"], "unknowns")?
            .iter()
            .enumerate()
        {
            let at = format!("unknowns[{}]", i);
            let mut reasons: Vec<String> = Vec::new();
            for (k, r) in req_arr(
                &u["reasons"],
                &format!("{}.reasons", at),
            )?
            .iter()
            .enumerate()
            {
                reasons.push(req_str(
                    r,
                    &format!("{}.reasons[{}]", at, k),
                )?);
            }
            unknowns.push((
                req_str(&u["fn"], &format!("{}.fn", at))?,
                reasons,
            ));
        }
        // decl → source path, resolved through the sources table.
        let mut source_path: BTreeMap<i64, String> =
            BTreeMap::new();
        for (i, srow) in
            req_arr(&v["sources"], "sources")?.iter().enumerate()
        {
            let at = format!("sources[{}]", i);
            let id =
                srow["id"].as_i64().ok_or_else(|| {
                    format!(
                        "{}: {}.id must be a number",
                        label, at
                    )
                })?;
            source_path.insert(
                id,
                req_str(&srow["path"], &format!("{}.path", at))?,
            );
        }
        let mut decl_sources: BTreeMap<String, String> =
            BTreeMap::new();
        if let Some(decls) =
            v["provenance"]["decls"].as_object()
        {
            for (decl, row) in decls {
                let sid =
                    row["source"].as_i64().ok_or_else(|| {
                        format!(
                            "{}: provenance.decls.{}.source                              must be a number",
                            label, decl
                        )
                    })?;
                if sid < 0 {
                    // Unplaceable (foreign/synthetic) — a
                    // legitimate absence, not a defect.
                    continue;
                }
                let p2 =
                    source_path.get(&sid).ok_or_else(|| {
                        format!(
                            "{}: provenance.decls.{} names                              source {}, which is not in the                              sources table",
                            label, decl, sid
                        )
                    })?;
                decl_sources.insert(decl.clone(), p2.clone());
            }
        }
        // The hashed endpoint identity (schema 1.12) — admission
        // has already proven the unhashed sections agree with it.
        let mut endpoints: Vec<EndpointIdentity> = Vec::new();
        // Absent = a bus-free program (the section is emitted
        // only when endpoints exist); present-but-not-an-array =
        // malformed, refused.
        let ep_rows: Vec<Value> =
            match v.get("endpoint_identity") {
                None | Some(Value::Null) => Vec::new(),
                Some(x) => {
                    req_arr(x, "endpoint_identity")?
                }
            };
        for (i, e) in ep_rows.iter().enumerate() {
            let at = format!("endpoint_identity[{}]", i);
            let publish = match e["verb"].as_str() {
                Some("publish") => true,
                Some("subscribe") => false,
                _ => {
                    return Err(format!(
                        "{}: {}.verb outside the closed                          vocabulary",
                        label, at
                    ))
                }
            };
            let owner = e["fn"]
                .as_str()
                .or_else(|| e["locus"].as_str())
                .ok_or_else(|| {
                    format!(
                        "{}: {} carries no owner",
                        label, at
                    )
                })?
                .to_string();
            endpoints.push(EndpointIdentity {
                publish,
                owner,
                site: e["site"].as_u64(),
                wire: req_str(
                    &e["wire"],
                    &format!("{}.wire", at),
                )?,
                topic: match e.get("topic") {
                    None => None,
                    Some(t) => Some(req_str(
                        t,
                        &format!("{}.topic", at),
                    )?),
                },
            });
        }
        endpoints.sort();
        Ok(ComponentModel {
            shape_hash,
            fns,
            calls,
            publishes,
            subscribes,
            topics,
            unknowns,
            decl_sources,
            endpoints,
        })
    }

    /// The wire identity a LOCAL topic name resolves to — the
    /// cross-binary join key.
    pub fn wire_of(
        &self,
        local: &str,
    ) -> Option<(&str, &str)> {
        self.topics.iter().find_map(|t| {
            if t.name == local {
                Some((
                    t.subject.as_str(),
                    t.payload_hash.as_str(),
                ))
            } else {
                None
            }
        })
    }

    /// Route-role check at TOPIC grain (round 2, #490): the plan
    /// endpoint names a local topic, so the role must be judged
    /// against endpoint rows carrying THAT topic identity — a
    /// literal end on the same wire is a different endpoint and
    /// must not satisfy it. A publisher role needs a send site;
    /// a subscriber role is a declaration by nature.
    pub fn has_topic_endpoint(
        &self,
        topic: &str,
        publishing: bool,
    ) -> bool {
        self.endpoints.iter().any(|e| {
            e.publish == publishing
                && e.topic.as_deref() == Some(topic)
                && (!publishing || e.site.is_some())
        })
    }

    /// Route-edge owners at the SAME topic grain the role check
    /// used: publishing fns whose site rows carry this topic
    /// identity.
    pub fn topic_publishers(&self, topic: &str) -> Vec<&str> {
        self.endpoints
            .iter()
            .filter(|e| {
                e.publish
                    && e.site.is_some()
                    && e.topic.as_deref() == Some(topic)
            })
            .map(|e| e.owner.as_str())
            .collect()
    }

    /// Subscribing handlers (full `Locus::handler` displays) whose
    /// rows carry this topic identity.
    pub fn topic_subscribers(&self, topic: &str) -> Vec<&str> {
        self.endpoints
            .iter()
            .filter(|e| {
                !e.publish
                    && e.topic.as_deref() == Some(topic)
            })
            .map(|e| e.owner.as_str())
            .collect()
    }

    /// Does the component actually USE this wire subject in the
    /// given direction? Declaring a topic is not using it: a
    /// publisher end needs a SEND SITE (an identity row with a
    /// site ordinal); a subscriber end is a declaration by
    /// nature. Reads the hashed endpoint identity — byte-exact
    /// wire subjects, collision-proof (schema 1.12).
    pub fn has_endpoint(
        &self,
        wire: &str,
        publishing: bool,
    ) -> bool {
        self.endpoints.iter().any(|e| {
            e.publish == publishing
                && e.wire == wire
                && (!publishing || e.site.is_some())
        })
    }

    /// The source path of a symbol's owning declaration, when the
    /// artifact placed it.
    pub fn decl_location(&self, local: &str) -> Option<&str> {
        let decl = local.split("::").next().unwrap_or(local);
        self.decl_sources.get(decl).map(String::as_str)
    }
}
