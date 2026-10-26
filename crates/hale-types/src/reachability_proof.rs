//! Proof-carrying reachability evidence. The judgment supplies its authentic
//! projection; this module checks finite certificates without running a graph
//! search. Lean independently checks the corresponding mathematical statement.
//! Extraction, completeness and runtime correspondence remain trusted.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use hale_model::{ApplicationModel, ClaimIr, ClaimIrTable, Provenance, RelationSet};
use serde_json::{json, Value};

use crate::verdict::Verdict;

#[derive(Clone, Debug)]
pub struct Vertex {
    pub id: u32,
    pub identity: String,
    pub display: String,
    /// Index in the model's provenance table; stdlib interiors have no bundle span.
    pub provenance: Option<u32>,
    pub expanded: bool,
}

#[derive(Clone, Debug)]
pub enum Certificate {
    Closed {
        closed: Vec<u32>,
    },
    /// `path` excludes the source, so an empty path is reflexive reachability.
    Path {
        source: u32,
        path: Vec<u32>,
    },
    /// The judgment's empty-target shortcut, valid over ANY edge relation.
    NoTargets,
}

#[derive(Clone, Debug)]
pub struct GraphProof {
    pub vertices: Vec<Vertex>,
    pub sources: Vec<u32>,
    pub targets: Vec<u32>,
    /// Every selected outgoing edge returned for every expanded vertex. For an
    /// absence these rows form a closed reachable cone, not the entire model.
    pub edges: Vec<(u32, u32)>,
    pub certificate: Certificate,
}

impl GraphProof {
    /// Local certificate validation, deliberately independent of BFS bookkeeping.
    pub fn validate(&self) -> Result<(), String> {
        let ids: BTreeSet<_> = self.vertices.iter().map(|v| v.id).collect();
        if ids.len() != self.vertices.len() {
            return Err("duplicate proof vertex identity".into());
        }
        if self
            .sources
            .iter()
            .chain(&self.targets)
            .any(|v| !ids.contains(v))
            || self
                .edges
                .iter()
                .any(|(a, b)| !ids.contains(a) || !ids.contains(b))
        {
            return Err("dangling proof vertex".into());
        }
        let expanded: BTreeSet<_> = self
            .vertices
            .iter()
            .filter(|v| v.expanded)
            .map(|v| v.id)
            .collect();
        if self.edges.iter().any(|(from, _)| !expanded.contains(from)) {
            return Err("edge leaves an unexpanded vertex".into());
        }
        match &self.certificate {
            Certificate::Closed { closed } => {
                let set: BTreeSet<_> = closed.iter().copied().collect();
                if set.len() != closed.len() || set != expanded {
                    return Err("closed set differs from expanded vertices".into());
                }
                if self.sources.iter().any(|v| !set.contains(v)) {
                    return Err("closed set omits a source".into());
                }
                if self.targets.iter().any(|v| set.contains(v)) {
                    return Err("closed set contains a target".into());
                }
                if self
                    .edges
                    .iter()
                    .any(|(a, b)| set.contains(a) && !set.contains(b))
                {
                    return Err("closed set omits an outgoing successor".into());
                }
            }
            Certificate::Path { source, path } => {
                if !self.sources.contains(source) {
                    return Err("path does not start in the source set".into());
                }
                let edges: BTreeSet<_> = self.edges.iter().copied().collect();
                let mut at = *source;
                for next in path {
                    if !edges.contains(&(at, *next)) {
                        return Err("path uses an absent edge".into());
                    }
                    at = *next;
                }
                if !self.targets.contains(&at) {
                    return Err("path does not end in the target set".into());
                }
            }
            Certificate::NoTargets => {
                if !self.targets.is_empty() {
                    return Err("empty-target certificate has targets".into());
                }
            }
        }
        Ok(())
    }

    fn json(&self) -> Value {
        let certificate = match &self.certificate {
            Certificate::Closed { closed } => json!({"kind": "closed", "closed": closed}),
            Certificate::Path { source, path } => {
                json!({"kind": "path", "source": source, "path": path})
            }
            Certificate::NoTargets => json!({"kind": "no_targets"}),
        };
        json!({
            "vertices": self.vertices.iter().map(|v| json!({
                "id": v.id, "identity": v.identity, "display": v.display,
                "provenance": v.provenance, "expanded": v.expanded,
            })).collect::<Vec<_>>(),
            "sources": self.sources, "targets": self.targets, "edges": self.edges,
            "certificate": certificate,
        })
    }
}

/// Canonical, source-bound report. It accounts for EVERY selected forbid law,
/// including rows for which no proof exists. Other families are explicitly out
/// of scope. This report is not topology admission or a whole-program proof.
pub struct ProofBundle {
    report: Value,
    proofs: BTreeMap<u32, GraphProof>,
    all_have_certificates: bool,
}

impl ProofBundle {
    pub fn derive(
        table: &ClaimIrTable,
        model: &ApplicationModel,
        source_bases: &[u32],
    ) -> Result<Self, String> {
        if model.header.semantics != hale_model::MODEL_SEMANTICS_V15 {
            return Err("unsupported model semantics for reachability proof extraction".into());
        }
        if table.provenance.sources != model.provenance.sources {
            return Err("law and model source identities differ".into());
        }
        if source_bases.len() != model.provenance.sources.len()
            || source_bases.windows(2).any(|bases| bases[0] >= bases[1])
        {
            return Err("source base account does not match model provenance".into());
        }
        model
            .validate()
            .map_err(|e| format!("invalid model: {e:?}"))?;
        table
            .validate(model)
            .map_err(|e| format!("invalid law table: {e:?}"))?;
        let (pre, judged, mut proofs) =
            crate::judgment::judge_forbid_reaches_with_certificates(table, model, source_bases);
        let mut all_have_certificates =
            table.issues.is_empty() && pre.is_empty() && !judged.is_empty();
        let mut rows = Vec::new();
        for result in judged {
            let row = &table.rows[result.ordinal as usize];
            let required = required_relations(&row.law);
            let completeness = model
                .capabilities
                .vouched_families()
                .into_iter()
                .filter(|(_, _, relations)| required.intersects(*relations))
                .map(|(name, exact, relations)| {
                    let gap_recorded = model.holes.iter().any(|h| h.hides.intersects(relations));
                    (name, exact, gap_recorded)
                })
                .collect::<Vec<_>>();
            let unvouched = completeness
                .iter()
                .filter(|(_, exact, gap)| !exact && !gap)
                .map(|(name, _, _)| *name)
                .collect::<Vec<_>>();
            // Holes outside the explored cone can explain global degradation:
            // the judgment already checks their relevance. A missing positive
            // capability with NO typed gap account cannot certify completeness.
            // Known paths, empty sources and empty targets need no such premise.
            let needs_completeness = proofs.get(&row.ordinal).is_some_and(|proof| {
                matches!(proof.certificate, Certificate::Closed { .. }) && !proof.sources.is_empty()
            });
            let refusal = if needs_completeness && !unvouched.is_empty() {
                proofs.remove(&row.ordinal);
                Some(format!(
                    "no typed gap account for withdrawn completeness: {}",
                    unvouched.join(", ")
                ))
            } else if !proofs.contains_key(&row.ordinal) {
                Some(
                    "invalid, incomplete, or traversal cutoff; no mathematical certificate"
                        .to_string(),
                )
            } else {
                None
            };
            let proof = proofs.get(&row.ordinal);
            if let Some(proof) = proof {
                proof.validate().map_err(|e| {
                    format!("claim {}: internal certificate rejected: {e}", row.ordinal)
                })?;
                let agrees = matches!(
                    (&proof.certificate, result.verdict),
                    (
                        Certificate::Closed { .. } | Certificate::NoTargets,
                        Verdict::Holds
                    ) | (Certificate::Path { .. }, Verdict::Violated)
                );
                if !agrees {
                    return Err(format!("claim {}: proof contradicts verdict", row.ordinal));
                }
            } else {
                all_have_certificates = false;
            }
            let law: Value =
                serde_json::from_str(&row.law_payload_json(&model.entities, &table.provenance))
                    .map_err(|e| format!("cannot encode law: {e}"))?;
            rows.push(json!({
                "ordinal": row.ordinal, "name": row.name, "law": law,
                "origin": origin_json(&row.origin), "provenance": row.provenance.0,
                "verdict": result.verdict.as_str(),
                "proof": proof.map(GraphProof::json),
                "refusal": refusal,
                "required_completeness": completeness.iter().map(|(name, exact, gap)| json!({"capability": name, "globally_exact": exact, "typed_gap_recorded": gap})).collect::<Vec<_>>(),
                "diagnostics": result.diags.iter().map(diagnostic_json).collect::<Vec<_>>(),
            }));
        }
        let hex = |n| format!("{n:016x}");
        let mut report = json!({
            "schema": "hale.reachability-proofs/2",
            "scope": "selected_forbid_reaches_only",
            "all_selected_forbid_laws_have_certificates": all_have_certificates,
            "binding": {
                "analysis_semantics": crate::evidence::ANALYSIS_SEMANTICS_VERSION,
                "certificate_semantics": 1,
                "lean_toolchain": "leanprover/lean4:v4.35.0",
                "model_semantics": model.header.semantics,
                "model_shape": hex(crate::topology_projection::project_shape_hash(model)),
                "law_digest": hex(table.semantic_digest()),
                "inputs_digest": hex(crate::evidence::analysis_inputs_digest()),
                "coverage_digest": hex(model.analysis_coverage_digest()),
                "sources": model.provenance.sources.iter().map(|s| json!({"path": s.path, "digest": s.digest})).collect::<Vec<_>>(),
            },
            "model_provenance": model.provenance.records.iter().map(provenance_json).collect::<Vec<_>>(),
            "law_provenance": table.provenance.records.iter().map(provenance_json).collect::<Vec<_>>(),
            "law_sources": table.provenance.sources.iter().map(|s| json!({"path": s.path, "digest": s.digest})).collect::<Vec<_>>(),
            "selection_issues": table.issues.iter().map(|i| json!({"message": i.message, "provenance": i.provenance.0})).collect::<Vec<_>>(),
            "prepass_diagnostics": pre.iter().map(diagnostic_json).collect::<Vec<_>>(),
            "source_bases": source_bases,
            "positive_completeness": model.capabilities.vouched_families().iter().map(|(name, exact, relations)| json!({"capability": name, "exact": exact, "relations": relations.0})).collect::<Vec<_>>(),
            "holes": model.holes.iter().map(|h| json!({"anchor": entity_json(h.at), "kind": hole_kind_json(&h.kind), "hides": h.hides.0, "reason": h.reason, "provenance": h.provenance.0})).collect::<Vec<_>>(),
            "assumptions": ["canonical model/law extraction and authentic source inputs", "selected successor rows and target membership faithfully describe the claim projection", "lowering/runtime correspondence is outside the finite graph theorem"],
            "excluded_law_ordinals": table.rows.iter().filter(|r| !matches!(r.law, ClaimIr::ForbidReaches { .. })).map(|r| r.ordinal).collect::<Vec<_>>(),
            "rows": rows,
            "trust_boundary": "compiler extraction, group/effect selection, completeness and lowering/runtime correspondence; Lean proves only the supplied finite graph proposition",
        });
        // Bind exactly the canonical evidence payload, including its
        // selected graph, law, completeness, provenance and source inputs.
        // This supplements the model shape; it is not a full-model digest.
        let bytes = serde_json::to_vec(&report).map_err(|e| e.to_string())?;
        report["binding"]["evidence_digest"] = hex(crate::topology::fnv1a64(&bytes)).into();
        Ok(Self {
            report,
            proofs,
            all_have_certificates,
        })
    }

    pub fn canonical_json(&self) -> String {
        // BTreeMap object order, deterministic lists, no duplicate JSON keys.
        format!(
            "{}\n",
            serde_json::to_string_pretty(&self.report).expect("JSON value")
        )
    }

    /// Exact replay binds the entire report to current source/model/semantics,
    /// including graph facts and provenance. It accepts the canonical encoding
    /// only, so duplicate keys and added/omitted/reordered data cannot be hidden
    /// by a permissive JSON parser. It does not invoke Lean or admit other laws.
    pub fn check_canonical(&self, supplied: &str) -> Result<(), String> {
        if supplied != self.canonical_json() {
            return Err("reachability proof report differs from the current canonical source/model/claim evidence".into());
        }
        if !self.all_have_certificates {
            return Err("not every selected forbid law has a mathematical certificate (or law selection failed)".into());
        }
        Ok(())
    }

    /// Bind the actual Lean document, including propositions, to this report.
    /// Successful replay is not a claim that the Lean kernel has run.
    pub fn check_canonical_lean(&self, supplied: &str) -> Result<(), String> {
        if supplied != self.lean_source()? {
            return Err(
                "Lean document differs from the current canonical reachability proof statements"
                    .into(),
            );
        }
        Ok(())
    }

    /// Generate only numeric Lean declarations; authored strings cannot inject
    /// Lean commands. The digest binds these declarations to canonical JSON,
    /// but is an identity, not an authenticity guarantee.
    pub fn lean_source(&self) -> Result<String, String> {
        if !self.all_have_certificates {
            return Err("cannot emit Lean proofs: not every selected forbid law has a mathematical certificate (or law selection failed)".into());
        }
        let digest = crate::topology::fnv1a64(self.canonical_json().as_bytes());
        let mut out = format!("import GraphCertificates\n\nset_option maxRecDepth 100000\nset_option maxHeartbeats 10000000\n\nnamespace Hale.Claims.Generated\ndef bundleDigest : Nat := 0x{digest:016x}\n");
        for (ordinal, proof) in &self.proofs {
            writeln!(out, "\nnamespace Claim{ordinal}").unwrap();
            writeln!(out, "def edges : Edges := {:?}", proof.edges).unwrap();
            writeln!(out, "def sources : List Nat := {:?}", proof.sources).unwrap();
            writeln!(out, "def targets : List Nat := {:?}", proof.targets).unwrap();
            match &proof.certificate {
                Certificate::Closed { closed } => {
                    writeln!(out, "def closed : List Nat := {closed:?}").unwrap();
                    out.push_str("theorem absence : ∀ source ∈ sources, ∀ target ∈ targets, ¬ Reachable edges source target :=\n  checkClosed_sound edges sources targets closed (by decide)\n");
                }
                Certificate::Path { source, path } => {
                    writeln!(
                        out,
                        "def start : Nat := {source}\ndef path : List Nat := {path:?}"
                    )
                    .unwrap();
                    out.push_str("theorem violation : ∃ target, start ∈ sources ∧ target ∈ targets ∧ Reachable edges start target :=\n  checkViolation_sound edges sources targets start path (by decide)\n");
                }
                Certificate::NoTargets => {
                    out.push_str("theorem absence (arbitraryEdges : Edges) : ∀ source ∈ sources, ∀ target ∈ targets, ¬ Reachable arbitraryEdges source target := by\n  intro source _ target selected\n  simp [targets] at selected\n");
                }
            }
            writeln!(out, "end Claim{ordinal}").unwrap();
        }
        out.push_str("end Hale.Claims.Generated\n");
        Ok(out)
    }
}

fn provenance_json(provenance: &Provenance) -> Value {
    match provenance {
        Provenance::Source { source, span } => {
            json!({"kind": "source", "source": source.0, "span": [span.0, span.1]})
        }
        Provenance::Synthetic { origin } => json!({"kind": "synthetic", "origin": origin}),
        Provenance::ForeignSpan { span } => {
            json!({"kind": "foreign_span", "span": [span.0, span.1]})
        }
    }
}

fn diagnostic_json(diag: &hale_syntax::Diag) -> Value {
    let location = |span: hale_syntax::Span, origin: hale_syntax::error::SpanOrigin| {
        json!({"space": match origin {
            hale_syntax::error::SpanOrigin::Seed => "bundle",
            hale_syntax::error::SpanOrigin::Stdlib => "stdlib",
        }, "span": [span.start.0, span.end.0]})
    };
    json!({"kind": diag.kind_str(), "message": diag.message,
        "location": location(diag.span, diag.origin),
        "related": diag.related.iter().map(|r| json!({"label": r.label, "location": location(r.span, r.origin)})).collect::<Vec<_>>(),
    })
}

fn required_relations(law: &ClaimIr) -> RelationSet {
    let mut required = RelationSet(0);
    if let ClaimIr::ForbidReaches {
        via_calls,
        via_bus,
        dst,
        ..
    } = law
    {
        if *via_calls {
            required = required.union(RelationSet::CALLS);
        }
        if *via_bus {
            required = required
                .union(RelationSet::PUBLISHES)
                .union(RelationSet::SUBSCRIBES);
        }
        if matches!(dst, hale_model::SetIr::EffectCarriers(_)) {
            required = required.union(RelationSet::EFFECTS);
        }
    }
    required
}

fn origin_json(origin: &hale_model::ClaimOrigin) -> Value {
    use hale_model::ClaimOrigin;
    match origin {
        ClaimOrigin::Main => json!({"kind": "main"}),
        ClaimOrigin::Constitution { name } => json!({"kind": "constitution", "name": name}),
        ClaimOrigin::Library { alias } => json!({"kind": "library", "alias": alias}),
        ClaimOrigin::Annotation => json!({"kind": "annotation"}),
        ClaimOrigin::FleetPlan => json!({"kind": "fleet_plan"}),
    }
}
fn entity_json(entity: hale_model::EntityRef) -> Value {
    use hale_model::EntityRef;
    let (sort, id) = match entity {
        EntityRef::Function(id) => ("function", id.0),
        EntityRef::LocusDecl(id) => ("locus_decl", id.0),
        EntityRef::LocusInstance(id) => ("locus_instance", id.0),
        EntityRef::Topic(id) => ("topic", id.0),
        EntityRef::Subject(id) => ("subject", id.0),
        EntityRef::Binding(id) => ("binding", id.0),
        EntityRef::ThreadDomain(id) => ("thread_domain", id.0),
        EntityRef::Phase(id) => ("phase", id.0),
        EntityRef::Seed(id) => ("seed", id.0),
        EntityRef::Group(id) => ("group", id.0),
        EntityRef::Type(id) => ("type", id.0),
        EntityRef::Interface(id) => ("interface", id.0),
        EntityRef::Declaration(id) => ("declaration", id.0),
    };
    json!({"sort": sort, "id": id})
}
fn hole_kind_json(kind: &hale_model::HoleKind) -> Value {
    use hale_model::HoleKind;
    let name = match kind {
        HoleKind::IndirectCall => "indirect_call",
        HoleKind::UntypedReceiver { callee } => {
            return json!({"kind": "untyped_receiver", "callee": callee})
        }
        HoleKind::ComputedSubject => "computed_subject",
        HoleKind::UnknownKeyDomain => "unknown_key_domain",
        HoleKind::OpenInterface => "open_interface",
        HoleKind::RuntimeInheritedPlacement => "runtime_inherited_placement",
        HoleKind::DynamicEndpoint => "dynamic_endpoint",
        HoleKind::ExternalOpaque => "external_opaque",
        HoleKind::UnsupportedArtifactSemantics => "unsupported_artifact_semantics",
        HoleKind::UnanalyzedBody => "unanalyzed_body",
        HoleKind::GenericFlowTemplate => "generic_flow_template",
        HoleKind::UnmodeledRecovery => "unmodeled_recovery",
        HoleKind::UnmodeledFailureDispatch => "unmodeled_failure_dispatch",
        HoleKind::UnmodeledInitialization => "unmodeled_initialization",
        HoleKind::UnmodeledLifecycleContext => "unmodeled_lifecycle_context",
        HoleKind::UnmodeledFailureContext => "unmodeled_failure_context",
    };
    json!({"kind": name})
}
