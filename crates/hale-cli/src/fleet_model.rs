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
        let arr = |x: &Value| -> Vec<Value> {
            x.as_array().cloned().unwrap_or_default()
        };
        let shape_hash = v["shape_hash"]
            .as_str()
            .ok_or_else(|| {
                format!("{}: shape_hash must be a string", label)
            })?
            .to_string();
        let fns: Vec<String> = arr(&v["sorts"]["fns"])
            .iter()
            .filter_map(|f| f.as_str().map(str::to_string))
            .collect();
        let mut calls: Vec<(String, String)> = Vec::new();
        for rel in ["calls", "calls_via_stdlib"] {
            for e in arr(&v["relations"][rel]) {
                if let (Some(f), Some(t)) =
                    (e["from"].as_str(), e["to"].as_str())
                {
                    calls.push((f.to_string(), t.to_string()));
                }
            }
        }
        let publishes: Vec<(String, String)> = arr(
            &v["relations"]["publishes"],
        )
        .iter()
        .filter_map(|p| {
            Some((
                p["fn"].as_str()?.to_string(),
                p["subject"].as_str()?.to_string(),
            ))
        })
        .collect();
        let subscribes: Vec<(String, String, String)> = arr(
            &v["relations"]["subscribes"],
        )
        .iter()
        .filter_map(|s| {
            Some((
                s["subject"].as_str()?.to_string(),
                s["locus"].as_str()?.to_string(),
                s["handler"].as_str()?.to_string(),
            ))
        })
        .collect();
        let topics: Vec<TopicRow> = arr(&v["topics"])
            .iter()
            .filter_map(|t| {
                Some(TopicRow {
                    name: t["name"].as_str()?.to_string(),
                    subject: t["subject"].as_str()?.to_string(),
                    payload_hash: t["payload_hash"]
                        .as_str()?
                        .to_string(),
                })
            })
            .collect();
        let unknowns: Vec<(String, Vec<String>)> =
            arr(&v["unknowns"])
                .iter()
                .filter_map(|u| {
                    Some((
                        u["fn"].as_str()?.to_string(),
                        arr(&u["reasons"])
                            .iter()
                            .filter_map(|r| {
                                r.as_str().map(str::to_string)
                            })
                            .collect(),
                    ))
                })
                .collect();
        // decl → source path, resolved through the sources table.
        let source_path: BTreeMap<i64, String> = arr(&v["sources"])
            .iter()
            .filter_map(|s| {
                Some((
                    s["id"].as_i64()?,
                    s["path"].as_str()?.to_string(),
                ))
            })
            .collect();
        let mut decl_sources: BTreeMap<String, String> =
            BTreeMap::new();
        if let Some(decls) =
            v["provenance"]["decls"].as_object()
        {
            for (decl, row) in decls {
                let Some(sid) = row["source"].as_i64() else {
                    continue;
                };
                if sid < 0 {
                    continue;
                }
                if let Some(p) = source_path.get(&sid) {
                    decl_sources
                        .insert(decl.clone(), p.clone());
                }
            }
        }
        // The hashed endpoint identity (schema 1.12) — admission
        // has already proven the unhashed sections agree with it.
        let mut endpoints: Vec<EndpointIdentity> = Vec::new();
        for e in arr(&v["endpoint_identity"]) {
            let publish = match e["verb"].as_str() {
                Some("publish") => true,
                Some("subscribe") => false,
                _ => {
                    return Err(format!(
                        "{}: endpoint_identity verb outside the \
                         closed vocabulary",
                        label
                    ))
                }
            };
            let owner = e["fn"]
                .as_str()
                .or_else(|| e["locus"].as_str())
                .ok_or_else(|| {
                    format!(
                        "{}: endpoint_identity row without an \
                         owner",
                        label
                    )
                })?
                .to_string();
            endpoints.push(EndpointIdentity {
                publish,
                owner,
                site: e["site"].as_u64(),
                wire: e["wire"]
                    .as_str()
                    .ok_or_else(|| {
                        format!(
                            "{}: endpoint_identity row without a \
                             wire subject",
                            label
                        )
                    })?
                    .to_string(),
                topic: e["topic"]
                    .as_str()
                    .map(str::to_string),
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
