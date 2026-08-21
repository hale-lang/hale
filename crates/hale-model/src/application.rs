//! The application-horizon root: one checked, closed application.
//!
//! Determinism is a law here, not a convention: every table is
//! canonically sorted and deduplicated, checked by
//! [`ApplicationModel::validate`]. Canonical iteration is what makes
//! hashing, artifact projection, diffing, and rendering stable
//! without forcing hot in-memory operations through ordered maps —
//! sorting happens once, at the boundary where a model is finished.

use crate::capability::Capabilities;
use crate::entity::{
    Binding, Declaration, EffectClassDecl, Function, Group,
    InterfaceDecl, LocusDecl, LocusInstance, PayloadContract, Phase,
    Seed, Subject, ThreadDomain, Topic, TypeDecl,
};
use crate::hole::Hole;
use crate::ids::{EntityRef, FunctionId, ProvenanceId, TopicId};
use crate::claim_ir::{ClaimIrError, ClaimIrTable, ClaimRow};
use crate::provenance::{Provenance, ProvenanceTable};
use crate::relation::{
    AffinedTo, Call, DeadInterfaceCall, DeclaredIn, DeclaresPublish,
    GroupMember, GroupSelector, MemberOf, Owns, PhaseOf, PlacedIn,
    Publish, Realizes, SelectorForm, Subscribe, Supervises, TopicBinding,
};

/// The meaning of the model's rows. Bumped when a row's
/// interpretation changes — a schema addition is not a semantics
/// bump; a reinterpretation is.
pub const MODEL_SEMANTICS_V1: u32 = 1;

/// Named hash algorithms over a model. The replay-compatibility law
/// (crate docs): the first artifact encoder reproduces
/// `TopologyShapeV1` byte-for-byte over the corpus before any
/// cutover; richer identities are new *named* variants with a
/// recording-format transition — the existing `u64` is never
/// silently reinterpreted. (The projection itself is Change 3;
/// naming the algorithm is Change 1, so nothing can ship an unnamed
/// hash in the meantime.)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModelHashKind {
    /// The legacy topology artifact shape hash: FNV-1a 64 over the
    /// canonical serialized model half, exactly as
    /// `hale-types::topology` emits it today.
    TopologyShapeV1,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ModelHeader {
    pub semantics: u32,
    /// The application entrypoint (main locus name, or `main`).
    pub entrypoint: String,
}

/// A free-form label on an entity (`labels` in the artifact).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LabelRow {
    pub at: EntityRef,
    pub label: String,
    pub provenance: ProvenanceId,
}

/// A quantitative fact (loop weights, cost classes) attached to an
/// entity. Change 1 keeps the metric vocabulary open; the
/// quantitative-judgment migration (Change 5d) closes it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WeightRow {
    pub at: EntityRef,
    pub metric: String,
    pub value: u64,
    pub provenance: ProvenanceId,
}

#[derive(Clone, Default, Debug)]
pub struct Entities {
    pub functions: Vec<Function>,
    pub loci: Vec<LocusDecl>,
    pub locus_instances: Vec<LocusInstance>,
    pub topics: Vec<Topic>,
    pub subjects: Vec<Subject>,
    pub payloads: Vec<PayloadContract>,
    pub phases: Vec<Phase>,
    pub seeds: Vec<Seed>,
    pub thread_domains: Vec<ThreadDomain>,
    pub bindings: Vec<Binding>,
    pub groups: Vec<Group>,
    pub types: Vec<TypeDecl>,
    pub interfaces: Vec<InterfaceDecl>,
    /// Declared/referenced USER effect classes (GH #476 Change 4).
    pub effect_classes: Vec<EffectClassDecl>,
    /// Seed-membership-only declarations (perspective, const, ring
    /// layout, target) — the rest of the nameable universe.
    pub declarations: Vec<Declaration>,
}

#[derive(Clone, Default, Debug)]
pub struct Relations {
    pub member_of: Vec<MemberOf>,
    pub phase_of: Vec<PhaseOf>,
    pub declared_in: Vec<DeclaredIn>,
    pub realizes: Vec<Realizes>,
    pub owns: Vec<Owns>,
    pub calls: Vec<Call>,
    pub dead_interface_calls: Vec<DeadInterfaceCall>,
    pub publishes: Vec<Publish>,
    /// Declared publisher ends (`bus { publish T; }`) — the
    /// endpoint grain `require publishes(...)` quantifies over,
    /// distinct from the site-grained sends above.
    pub declares_publish: Vec<DeclaresPublish>,
    pub subscribes: Vec<Subscribe>,
    pub placed_in: Vec<PlacedIn>,
    pub affined_to: Vec<AffinedTo>,
    pub binds: Vec<TopicBinding>,
    pub supervises: Vec<Supervises>,
    pub group_members: Vec<GroupMember>,
    /// The AUTHORED selector lists (legacy-hash grain), alongside
    /// the resolved `group_members` (judgment grain).
    pub group_selectors: Vec<GroupSelector>,
}

/// The Change-3 bridge, quarantined in one named structure: the
/// EXACT membership of legacy serialized sorts that are narrower
/// than the model's universe, so `TopologyShapeV1` can be projected
/// from the model alone (no summary/AST side channel). Deleted when
/// One subject's dispatch-gate facts (GH #476 Change 8) — copied
/// verbatim from the BusGraph's soundness gates.
#[derive(Clone, Debug, Default)]
pub struct DispatchGate {
    /// The BusGraph's subject key (site spelling).
    pub subject: String,
    /// Static-bucket eligible (closed world).
    pub static_eligible: bool,
    /// Direct-call eligible (same-thread + quiet + closed world).
    pub direct_eligible: bool,
    /// The gate's reason when not static-eligible.
    pub ineligible_reason: Option<String>,
    /// Publisher locus displays (site grain, deduped).
    pub publisher_loci: Vec<String>,
    /// Subscriber (locus display, handler) pairs.
    pub subscribers: Vec<(String, String)>,
}

/// the legacy artifact schema is versioned past.
#[derive(Clone, Debug, Default)]
pub struct LegacyProjection {
    /// The legacy artifact's fn sort (behavior-summary keys): a
    /// strict subset of `entities.functions` — a module-scoped or
    /// empty declaration exists in the universe but not here.
    pub topology_v1_fns: Vec<FunctionId>,
    /// The legacy artifact's `calls_via_stdlib` rows, verbatim:
    /// the output of the legacy one-Boolean, no-revisit contraction
    /// walk, NOT of the model's two-component lattice. The two can
    /// legitimately disagree on a loop bit — a stdlib node first
    /// reached on a non-looped path is never revisited by the
    /// legacy walk, while the lattice strengthens it — and the
    /// loop bit is inside the hashed model half, so projecting the
    /// lattice rows would silently change `TopologyShapeV1` for
    /// unchanged source. Both endpoints are legacy fns
    /// (∈ `topology_v1_fns`).
    pub topology_v1_calls_via_stdlib: Vec<(FunctionId, FunctionId, bool)>,
    /// GH #476 Change 8: the BusGraph's per-subject dispatch gates
    /// — the trusted devirtualization analysis, bridged like every
    /// other legacy engine. `DispatchPlan::derive` combines these
    /// facts with the arrangement into the typed lowering plan.
    pub dispatch_gates: Vec<DispatchGate>,
    /// GH #476 Change 5a: what the evaluator's merged-summary walk
    /// sees INSIDE stdlib bodies reachable from a user fn — interior
    /// fail-closed holes (with the stdlib fn's display for the
    /// diagnostic), and user→user re-emergence edges with their
    /// interior witness path. The reachability judgment consumes
    /// this to reproduce the evaluator byte-for-byte; deleted at
    /// Change 9 with the rest of the projection. Sorted by
    /// (from, site).
    pub stdlib_absorption: Vec<StdlibAbsorption>,
}

/// One user call site whose callee is a stdlib body (or a stdlib
/// conformer alternative of an interface dispatch): the interior
/// GRAPH the evaluator's merged-summary walk would traverse. Kept
/// as a graph — not a flattened summary — because BFS layering
/// decides which of a hole and a hit fires first, and the judgment
/// must replay the evaluator's order exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StdlibAbsorption {
    pub from: FunctionId,
    /// The authored site ordinal within `from` (stdlib calls consume
    /// ordinals like every authored site), so the judgment can
    /// interleave the entry edge at the evaluator's BFS position.
    pub site: u32,
    /// Dispatch rendering of the ENTRY step (a stdlib conformer
    /// alternative of an interface dispatch), when it is one.
    pub entry_dispatch: Option<(String, String)>,
    /// The entry call's loop nesting — a carrier reached through a
    /// loop-nested stdlib entry repeats per iteration, and the
    /// evaluator reads it off the real edge (review: looped stdlib
    /// entries).
    pub entry_in_loop: bool,
    /// The entry call's dispatch group (shared by the alternatives
    /// of one dispatch site — `bound` folds them with MAX).
    pub entry_group: Option<u32>,
    /// The entry call's authored span.
    pub entry_provenance: ProvenanceId,
    /// Interior vertices, discovery order; node 0 is the entry.
    pub nodes: Vec<AbsorbedNode>,
}

/// One interior stdlib vertex: its display spelling and its body's
/// walk-relevant events, in body order (the evaluator collects call
/// edges and returns at the FIRST unfollowable one, then scans
/// publish sites — the judgment replays this sequence under the
/// claim's via masks).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbsorbedNode {
    /// e.g. `std::io::tcp::Stream::send` (demangled like every
    /// witness spelling).
    pub display: String,
    /// The stdlib fn's declared carrier classes (rendered) — the
    /// `bound` count's per-node contribution (GH #476 Change 5d;
    /// `std::secret` bodies carry `secret_use`).
    pub carries: Vec<String>,
    /// The stdlib fn's DIRECT effect classes (the evaluator applies
    /// `direct_effects` to every visited FnKey, stdlib included —
    /// an `effects(C)` destination can be satisfied INSIDE a
    /// stdlib body; review: stdlib effect sinks).
    pub direct_effects: Vec<String>,
    pub events: Vec<AbsorbedEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AbsorbedEvent {
    /// A resolved call edge — interior or re-emergent.
    Call {
        target: AbsorbedTarget,
        /// (interface display, method) when the edge is a dispatch
        /// alternative.
        dispatch: Option<(String, String)>,
        /// Loop-nested at the call site (a carrier past it repeats
        /// per iteration — `bound`'s unbounded rule).
        in_loop: bool,
        /// Dispatch-group id: alternatives of ONE dispatch site
        /// share it, and a `bound` count takes their MAX, not sum.
        group: Option<u32>,
    },
    /// An unfollowable call edge (fires under `via { calls }`).
    CallHole(AbsorbedHoleKind),
    /// A publish to a known subject (fans out under `via { bus }`).
    Publish {
        /// Canonical spelling for delivery matching (topic name for
        /// a declared-topic reference, wire text otherwise).
        subject: String,
        /// TYPED topic identity when the interior publish speaks a
        /// declared topic — a literal wire address whose text
        /// collides with a topic name stays `None` (round 6: hole
        /// coverage must not re-conflate the identities the model
        /// keeps apart).
        declared_topic: Option<TopicId>,
        /// The publish sits inside a loop (5d bound walks).
        in_loop: bool,
    },
    /// A publish to a computed subject (fires under `via { bus }`).
    PublishHole,
    /// The absorption walk hit its step ceiling before settling —
    /// explicit residue, never silent exhaustion (a judgment
    /// treating a truncated interior as fully explored would
    /// certify an absence it cannot see).
    Truncated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbsorbedTarget {
    /// Another interior vertex (index into `nodes`).
    Interior(u32),
    /// Re-emergence at a user fn.
    User(FunctionId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AbsorbedHoleKind {
    /// An indirect call (fn-typed value).
    IndirectCall,
    /// A method call on an untypable receiver.
    OpaqueCall { callee: String },
}

/// The verdict vocabulary, mirrored from the evaluator's
/// (`hale-types::verdict::Verdict`) — a conformance test pins the
/// two, since this crate cannot depend on it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VerdictIr {
    Holds,
    Violated,
    Uncertified,
    Invalid,
}

/// GH #476 Change 5e: one pointwise certificate's EVIDENCE — the
/// fn-grained outcome the existing certificate engines produce
/// (the artifact has carried these as lowered claim rows since
/// #392 §8). The engines stay the one analysis authority
/// (extract-and-call, like `direct_effects`); Change 6 formalizes
/// this into the typed evidence artifact rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificateEvidence {
    /// The lowered claim form, display voice — the artifact's
    /// `lowered` row spelling.
    pub form: String,
    pub result: VerdictIr,
    /// The engine's diagnostics for this certificate, in emission
    /// order: (message, span provenance).
    pub diags: Vec<(String, ProvenanceId)>,
}

/// The evidence SIDECAR for one lowered law table — deliberately
/// OUTSIDE [`ApplicationModel`] (review: the model must not carry a
/// cached prior judgment of itself). Rows key by the ClaimIr
/// ordinal and typed subject; `model_shape` ties the sidecar to the
/// exact model it was derived beside, so stale evidence is
/// structurally refusable.
#[derive(Clone, Debug, Default)]
pub struct EvidenceTable {
    /// `TopologyShapeV1` of the model this evidence was derived
    /// with — a judgment refuses evidence whose shape disagrees
    /// with the model it is asked to judge.
    pub model_shape: u64,
    /// [`ClaimIrTable::semantic_digest`] of the law table this
    /// evidence answers. The topology shape does NOT hash
    /// annotation laws — two programs with identical topology but
    /// different `@effects` classes share a `model_shape`, so the
    /// sidecar must also be tied to the LAW it certifies (review
    /// round 2).
    pub law_digest: u64,
    /// Digest of the ANALYSIS INPUTS outside the model — the
    /// stdlib source the certificate engines walk and the compiler
    /// version that produced the evidence. Engine inputs not
    /// represented in the topology hash cannot be covered by
    /// `model_shape` (review round 3); the producer computes it,
    /// and validation requires the judging toolchain to agree.
    pub inputs_digest: u64,
    /// Digest of the model's ANALYSIS COVERAGE (round 10) — the
    /// function-grain `analyzed` bits and locus-grain `analyzable`
    /// bits the producer's synthesis decisions depend on.
    /// `TopologyShapeV1` deliberately excludes coverage (recording
    /// compatibility), so two models differing only in coverage
    /// share a `model_shape` — this digest is what stops a
    /// synthetic `Holds` sidecar from validating against a model
    /// for which derivation would have produced no report.
    pub coverage_digest: u64,
    pub rows: Vec<EvidenceRow>,
    pub provenance: ProvenanceTable,
}

impl ApplicationModel {
    /// The relevant (non-failure) members a locus OWNS — from the
    /// closed `owner` account, which `member_of` must mirror
    /// exactly (validated). Shared by the model validator and the
    /// sidecar API, so coverage is judged from one relation.
    pub fn locus_members_analyzed(
        &self,
        lid: crate::ids::LocusDeclId,
    ) -> bool {
        self.entities
            .functions
            .iter()
            .filter(|f| {
                f.owner == Some(lid)
                    && !matches!(
                        f.kind,
                        crate::entity::FunctionKind::FailureHandler
                    )
            })
            .all(|f| f.analyzed)
    }

    /// The OWNERSHIP + COVERAGE laws (round 15) — one shared
    /// validator: `member_of` is a total, exclusive partition
    /// agreeing with `Function::owner` (a free fn owns nothing
    /// and appears in no row; every other kind has exactly one
    /// row, at its canonical owner), and every coverage law from
    /// rounds 10–14. `ApplicationModel::validate` calls this, and
    /// so does `EvidenceTable::validate` — a model whose
    /// ownership account is corrupted cannot certify anything,
    /// digests notwithstanding.
    pub fn validate_coverage(&self) -> Result<(), ModelError> {
        let e = &self.entities;
        // Ownership partition: rows per function.
        let mut rows_of: std::collections::BTreeMap<
            u32,
            Vec<crate::ids::LocusDeclId>,
        > = std::collections::BTreeMap::new();
        for m in &self.relations.member_of {
            rows_of
                .entry(m.function.0)
                .or_default()
                .push(m.locus);
        }
        for (i, f) in e.functions.iter().enumerate() {
            let rows = rows_of
                .get(&(i as u32))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let free =
                matches!(f.kind, crate::entity::FunctionKind::Free);
            if free != f.owner.is_none() {
                return Err(ModelError::CoverageLaw {
                    index: i,
                    law: "free functions own nothing; every \
                          other kind has a canonical owner",
                });
            }
            match f.owner {
                None => {
                    if !rows.is_empty() {
                        return Err(ModelError::CoverageLaw {
                            index: i,
                            law: "a free function appears in no \
                                  member_of row",
                        });
                    }
                }
                Some(owner) => {
                    if rows.len() != 1 || rows[0] != owner {
                        return Err(ModelError::CoverageLaw {
                            index: i,
                            law: "member_of is a total exclusive \
                                  partition agreeing with the \
                                  canonical owner",
                        });
                    }
                    // Round 16: ownership is anchored to the
                    // ENTITY IDENTITY, not merely its relational
                    // mirror — the owner named by the field must
                    // be the locus encoded in the function's own
                    // canonical name and display. A coordinated
                    // repoint (owner + row + both analyzability
                    // flags) is refused HERE: `Hidden::poke`
                    // cannot canonically be owned by `App`.
                    let Some(l) = e.loci.get(owner.index())
                    else {
                        return Err(ModelError::DanglingId {
                            table: "functions.owner",
                            index: i,
                        });
                    };
                    let raw_ok = f
                        .name
                        .strip_prefix(&l.name)
                        .is_some_and(|r| r.starts_with("::"));
                    let disp_ok = f
                        .display
                        .strip_prefix(&l.display)
                        .is_some_and(|r| r.starts_with("::"));
                    if !raw_ok || !disp_ok {
                        return Err(ModelError::CoverageLaw {
                            index: i,
                            law: "the canonical owner is the \
                                  locus encoded in the \
                                  function's own identity",
                        });
                    }
                }
            }
            if f.summarized && !f.analyzed {
                return Err(ModelError::CoverageLaw {
                    index: i,
                    law: "summarized implies analyzed",
                });
            }
            if f.analyzed && !f.summarized {
                return Err(ModelError::CoverageLaw {
                    index: i,
                    law: "analyzed implies summarized — the \
                          walked set is the summary set",
                });
            }
            if matches!(
                f.kind,
                crate::entity::FunctionKind::FailureHandler
            ) && f.analyzed
            {
                return Err(ModelError::CoverageLaw {
                    index: i,
                    law: "failure handlers are never analyzed",
                });
            }
        }
        // Unanalyzed-residue law.
        {
            let holed: std::collections::BTreeSet<u32> = self
                .holes
                .iter()
                .filter(|h| {
                    h.kind == crate::hole::HoleKind::UnanalyzedBody
                })
                .filter_map(|h| match h.at {
                    crate::ids::EntityRef::Function(f) => {
                        Some(f.0)
                    }
                    _ => None,
                })
                .collect();
            for (i, f) in e.functions.iter().enumerate() {
                let has_hole = holed.contains(&(i as u32));
                if f.analyzed && has_hole {
                    return Err(ModelError::CoverageLaw {
                        index: i,
                        law: "an analyzed body carries no \
                              UnanalyzedBody residue",
                    });
                }
                if !f.analyzed && !has_hole {
                    return Err(ModelError::CoverageLaw {
                        index: i,
                        law: "an unanalyzed body retains its \
                              UnanalyzedBody residue",
                    });
                }
            }
        }
        // Locus-grain law, from the closed owner account.
        for (i, l) in e.loci.iter().enumerate() {
            let expect = self.locus_members_analyzed(
                crate::ids::LocusDeclId(i as u32),
            );
            if l.analyzable != expect {
                return Err(ModelError::CoverageLaw {
                    index: i,
                    law: "locus analyzability derives from its \
                          member coverage",
                });
            }
        }
        // Summarized set == legacy fn sort (in-range rows only;
        // out-of-range is the DanglingId defect).
        {
            let summarized: std::collections::BTreeSet<u32> = e
                .functions
                .iter()
                .enumerate()
                .filter(|(_, f)| f.summarized)
                .map(|(i, _)| i as u32)
                .collect();
            let legacy: std::collections::BTreeSet<u32> = self
                .legacy
                .topology_v1_fns
                .iter()
                .map(|f| f.0)
                .collect();
            let in_range = legacy
                .iter()
                .all(|i| (*i as usize) < e.functions.len());
            if in_range && summarized != legacy {
                return Err(ModelError::CoverageLaw {
                    index: usize::MAX,
                    law: "the legacy fn sort is the summarized \
                          set",
                });
            }
        }
        Ok(())
    }

    /// The model's analysis-coverage identity (round 10): fnv1a64
    /// over every locus's `analyzable` bit and every function's
    /// `analyzed` bit, in canonical entity order. Evidence
    /// derivation stamps it; the judgment refuses a sidecar whose
    /// coverage disagrees with the judged model.
    pub fn analysis_coverage_digest(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut eat = |bytes: &[u8]| {
            for b in bytes {
                h ^= u64::from(*b);
                h = h.wrapping_mul(0x100000001b3);
            }
        };
        for l in &self.entities.loci {
            eat(l.name.as_bytes());
            eat(&[0, u8::from(l.analyzable)]);
        }
        for f in &self.entities.functions {
            eat(f.name.as_bytes());
            eat(&[
                1,
                u8::from(f.analyzed),
                u8::from(f.summarized),
                matches!(
                    f.kind,
                    crate::entity::FunctionKind::FailureHandler
                ) as u8,
            ]);
            // Ownership is coverage-bearing (round 15): moving a
            // member changes which locus's coverage it counts
            // toward.
            match f.owner {
                Some(l) => eat(&l.0.to_le_bytes()),
                None => eat(&[0xff; 4]),
            }
        }
        h
    }
}

impl EvidenceTable {
    /// Structural laws against the judged pair: the shape must
    /// match the model, the law digest must match the table, and
    /// ordinals must be unique and in the law table's range,
    /// subjects must agree with the ClaimIr row's resolution, and
    /// diagnostic provenance must resolve.
    pub fn validate(
        &self,
        model: &ApplicationModel,
        model_shape: u64,
        table: &ClaimIrTable,
        inputs_digest: u64,
    ) -> Result<(), ClaimIrError> {
        // Round 15: the SAME ownership/coverage validator the
        // model runs — a corrupted ownership account (a deleted or
        // moved member_of row, an upgraded coverage bit) cannot
        // certify anything, digests notwithstanding.
        if model.validate_coverage().is_err() {
            return Err(ClaimIrError::InvalidProvenanceRecord {
                index: usize::MAX,
            });
        }
        if self.model_shape != model_shape
            || self.law_digest != table.semantic_digest()
            || self.inputs_digest != inputs_digest
            || self.coverage_digest
                != model.analysis_coverage_digest()
        {
            return Err(ClaimIrError::InvalidProvenanceRecord {
                index: usize::MAX,
            });
        }
        // Source-snapshot tie: the sidecar's source units (path +
        // content digest) must exactly equal BOTH the judged
        // model's and the law table's (review rounds 3–4) — the
        // three artifacts must describe one snapshot, or stale
        // offsets render against the wrong source bases.
        let unit_mismatch =
            |a: &[crate::provenance::SourceUnit],
             b: &[crate::provenance::SourceUnit]| {
                a.len() != b.len()
                    || a.iter().zip(b.iter()).any(|(x, y)| {
                        x.path != y.path || x.digest != y.digest
                    })
            };
        if unit_mismatch(
            &self.provenance.sources,
            &model.provenance.sources,
        ) || unit_mismatch(
            &self.provenance.sources,
            &table.provenance.sources,
        ) {
            return Err(ClaimIrError::InvalidProvenanceRecord {
                index: usize::MAX,
            });
        }
        // Round 12: coverage is BINDING, not advisory — a
        // certificate payload for a subject whose typed coverage
        // says no report can exist (an unanalyzed fn, an
        // unanalyzable locus) is refused here, so the judgment can
        // never consume it. A matching digest proves the sidecar
        // repeated the model's bits; this proves its evidence
        // obeys them.
        let claim_by_ordinal: std::collections::BTreeMap<
            u32,
            &ClaimRow,
        > = table.rows.iter().map(|r| (r.ordinal, r)).collect();
        for (i, row) in self.rows.iter().enumerate() {
            if row.certs.is_empty() {
                continue;
            }
            let eligible = match claim_by_ordinal
                .get(&row.ordinal)
                .map(|c| &c.law)
            {
                Some(
                    crate::claim_ir::ClaimIr::EffectForbid {
                        at, ..
                    }
                    | crate::claim_ir::ClaimIr::EffectOnly {
                        at, ..
                    }
                    | crate::claim_ir::ClaimIr::EffectPublishSet {
                        at, ..
                    }
                    | crate::claim_ir::ClaimIr::NoPanic { at },
                ) => at.0.is_some_and(|f| {
                    // Round 14: eligibility requires the HASHED
                    // anchor too — a coverage bit upgraded on an
                    // otherwise-unvalidated model still fails
                    // here (analyzed without summarized is not a
                    // reportable subject).
                    model
                        .entities
                        .functions
                        .get(f.index())
                        .is_some_and(|f| {
                            f.analyzed && f.summarized
                        })
                }),
                Some(
                    crate::claim_ir::ClaimIr::PhaseEffects {
                        locus,
                        ..
                    },
                ) => locus.0.is_some_and(|lid| {
                    // Round 14: the locus bit alone is not
                    // trusted — the member coverage must agree
                    // (recomputed from the typed member_of
                    // relation), so flipping `analyzable` over an
                    // unanalyzed member cannot make its phases
                    // reportable.
                    let flag = model
                        .entities
                        .loci
                        .get(lid.index())
                        .is_some_and(|l| l.analyzable);
                    flag && model.locus_members_analyzed(lid)
                }),
                _ => false,
            };
            if !eligible {
                return Err(
                    ClaimIrError::InvalidProvenanceRecord {
                        index: i,
                    },
                );
            }
        }
        let law_rows = table.rows.len();
        let by_ordinal: std::collections::BTreeMap<u32, &ClaimRow> =
            table.rows.iter().map(|r| (r.ordinal, r)).collect();
        let mut seen = std::collections::BTreeSet::new();
        for (i, row) in self.rows.iter().enumerate() {
            if row.ordinal as usize >= law_rows
                || !seen.insert(row.ordinal)
            {
                return Err(ClaimIrError::NonContiguousOrdinal {
                    index: i,
                });
            }
            if row
                .subject
                .is_some_and(|f| f.index() >= model.entities.functions.len())
            {
                return Err(ClaimIrError::DanglingId {
                    index: i,
                    what: "evidence subject",
                });
            }
            // Per-row law binding (review round 3): each stored
            // certificate's form must agree POSITIONALLY with the
            // exact forms its ClaimIr row produces — a table-wide
            // digest cannot see two same-subject rows exchanging
            // their certificate payloads. A SHORT row (an
            // unresolved subject the engines never saw) stays a
            // per-row judgment concern; a LONGER row or any form
            // disagreement is a malformed sidecar.
            let expected = by_ordinal
                .get(&row.ordinal)
                .map(|r| r.certificate_forms())
                .unwrap_or_default();
            if row.certs.len() > expected.len()
                || row
                    .certs
                    .iter()
                    .zip(expected.iter())
                    .any(|(c, (_, form))| c.form != *form)
            {
                return Err(ClaimIrError::NameDisagreement {
                    index: i,
                    what: "evidence certificate form",
                });
            }
            for c in &row.certs {
                for (_, pid) in &c.diags {
                    if pid.index() >= self.provenance.records.len() {
                        return Err(
                            ClaimIrError::DanglingProvenance {
                                index: i,
                            },
                        );
                    }
                }
            }
        }
        // Record contents resolve (incl. inverted ForeignSpan).
        let src_len = self.provenance.sources.len();
        for (i, r) in self.provenance.records.iter().enumerate() {
            match r {
                Provenance::Source { source, span } => {
                    if source.index() >= src_len || span.0 > span.1 {
                        return Err(
                            ClaimIrError::InvalidProvenanceRecord {
                                index: i,
                            },
                        );
                    }
                }
                Provenance::ForeignSpan { span } => {
                    if span.0 > span.1 {
                        return Err(
                            ClaimIrError::InvalidProvenanceRecord {
                                index: i,
                            },
                        );
                    }
                }
                Provenance::Synthetic { .. } => {}
            }
        }
        Ok(())
    }
}

/// The certificates of ONE ClaimIr row (a multi-class assert has
/// one certificate per class, in class order).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceRow {
    /// The ClaimIr ordinal this evidence answers.
    pub ordinal: u32,
    /// The annotated fn, as the ClaimIr row resolves it.
    pub subject: Option<FunctionId>,
    pub certs: Vec<CertificateEvidence>,
}

#[derive(Clone, Debug)]
pub struct ApplicationModel {
    pub header: ModelHeader,
    pub entities: Entities,
    pub relations: Relations,
    pub labels: Vec<LabelRow>,
    pub weights: Vec<WeightRow>,
    pub holes: Vec<Hole>,
    pub capabilities: Capabilities,
    pub provenance: ProvenanceTable,
    pub legacy: LegacyProjection,
}

/// A violated model law. `validate` returns the FIRST violation —
/// builders fix laws one at a time; consumers treat any error as
/// "this value is not a model".
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ModelError {
    /// A table (or set-like nested vector) is not canonically
    /// sorted + deduplicated by its semantic key. Carries the table
    /// name and the first offending index.
    NotCanonical { table: &'static str, index: usize },
    /// A row references an ID outside its table.
    DanglingId { table: &'static str, index: usize },
    /// A row's provenance ID is out of range.
    DanglingProvenance { table: &'static str, index: usize },
    /// A provenance record's contents are unresolvable: a dangling
    /// `SourceId` or an inverted span.
    InvalidProvenanceRecord { index: usize },
    /// A hole that hides no relation family is not a hole.
    EmptyHole { index: usize },
    /// A capability claims exactness while a hole hides that
    /// family — the two accounts of completeness drifted.
    CapabilityContradiction { capability: &'static str },
    /// A row carries an inline `Unknown` (key domain / predicate)
    /// with no typed hole hiding KEY_FILTERS at its anchor — an
    /// unknown may not hide inside an otherwise resolved row.
    UnrepresentedUnknown { table: &'static str, index: usize },
    /// A zero capacity: `Bounded(0)` / a zero topic bound is
    /// excluded by the settled bounds design (GH #255).
    InvalidBound { table: &'static str, index: usize },
    /// A locus instance's `decl` field and its `realizes` row
    /// disagree — two authorities for one fact drifted.
    RealizesDisagrees { index: usize },
    /// A locus instance has no (or more than one) `realizes` row:
    /// the relational projection is not total.
    RealizesIncomplete { instance: usize },
    /// The replicas of one `replicas = K` field are not a
    /// contiguous, unique, 0-based index set. Replica indices are
    /// what codegen bakes and what a keyed subscriber registers
    /// under, so `[3, 3, 3]` (the count repeated) or a gap is a
    /// model that names instances the runtime does not have.
    /// Carries the field's instance-path base.
    ReplicaIndicesNotContiguous { base: String },
    /// A `binds` row joins a topic and a binding whose subjects
    /// disagree — the entity attribute and the relation repeat one
    /// fact and drifted.
    BindingSubjectDisagrees { index: usize },
    /// A `where key == _` (Fallback) subscription on a topic whose
    /// `on_unmatched` policy is not `fallback` — the `_` sentinel
    /// is legal only on fallback topics (resolve-time law,
    /// mirrored in the schema).
    IllegalFallback { index: usize },
    /// A coverage law is violated (round 11): `summarized ⇒
    /// analyzed`, `FailureHandler ⇒ ¬analyzed`, or the legacy fn
    /// sort disagrees with the summarized set.
    CoverageLaw { index: usize, law: &'static str },
    /// A topic declares `on_unmatched: fallback` but has no
    /// Fallback subscription — the policy's required catch is
    /// missing.
    FallbackUncovered { topic: usize },
    /// The keyed-ness of a publish/subscription disagrees with its
    /// topic: a key domain on an unkeyed topic (or none on a keyed
    /// one), or a keyed predicate (`EqLiteral`/`EqReplica`/
    /// `Fallback`/`Unknown`) filtering an unkeyed topic.
    KeyContract { table: &'static str, index: usize },
    /// A bus row's `declared_topic` names a topic whose subject
    /// differs from the row's own subject — one endpoint, two
    /// addresses.
    DeclaredTopicDisagrees { table: &'static str, index: usize },
    /// A bus row's payload contract differs from its declared
    /// topic's — one endpoint, two shapes.
    EndpointPayloadDisagrees { table: &'static str, index: usize },
}

impl ApplicationModel {
    /// Check every model law. A value that fails is not a model and
    /// must not be judged, hashed, projected, or composed.
    ///
    /// Canonical keys (strictly increasing — sorted AND
    /// deduplicated; attributes outside the key are merged by the
    /// builder, never duplicated into extra rows):
    ///
    /// | table            | key                              |
    /// |------------------|----------------------------------|
    /// | entity tables    | canonical name / path / pattern  |
    /// | `payloads`       | (shape, hash)                    |
    /// | `bindings`       | (subject, transport, role)       |
    /// | `member_of`      | (function, locus)                |
    /// | `phase_of`       | (function, phase)                |
    /// | `declared_in`    | (entity, seed)                   |
    /// | `realizes`       | (instance, decl)                 |
    /// | `owns`           | (parent, child)                  |
    /// | `calls`          | (from, to, dispatch, site)       |
    /// | `publishes`      | (function, subject, site)        |
    /// | `declares_publish` | (locus, subject, declared_topic) |
    /// | `subscribes`     | (subject, handler, site)         |
    /// | `dead_interface_calls` | (from, site)               |
    /// | `placed_in`      | (instance)                       |
    /// | `affined_to`     | (domain)                         |
    /// | `binds`          | (topic, binding)                 |
    /// | `supervises`     | (parent, child, error_type)      |
    /// | `group_members`  | (group, member)                  |
    /// | `group_selectors`| (group, ordinal)                 |
    /// | `labels`         | (at, label)                      |
    /// | `weights`        | (at, metric)                     |
    /// | `holes`          | (at, kind, reason)               |
    /// | nested key sets  | `KeyDomain::Exact`, `CoreSet`    |
    /// EVERY relation family some unresolved residue hides —
    /// typed hole rows AND stdlib-absorption residue (a CallHole
    /// is an unfollowable call, a PublishHole an unprovable
    /// publish, a Truncated frontier hides everything beyond it).
    /// Exactness and holes are dual accounts that may not
    /// disagree, wherever the hole lives; the capability law and
    /// per-family adequacy both read this one mask (rounds 8, 2).
    pub fn unresolved_relation_mask(
        &self,
    ) -> crate::hole::RelationSet {
        let mut m = crate::hole::RelationSet(0);
        for h in &self.holes {
            m = m.union(h.hides);
        }
        for a in &self.legacy.stdlib_absorption {
            for n in &a.nodes {
                for ev in &n.events {
                    match ev {
                        crate::AbsorbedEvent::CallHole(_) => {
                            m = m
                                .union(crate::hole::RelationSet::CALLS)
                                .union(
                                    crate::hole::RelationSet::EFFECTS,
                                );
                        }
                        crate::AbsorbedEvent::PublishHole => {
                            m = m.union(
                                crate::hole::RelationSet::PUBLISHES,
                            );
                        }
                        crate::AbsorbedEvent::Truncated => {
                            m = m
                                .union(crate::hole::RelationSet::CALLS)
                                .union(
                                    crate::hole::RelationSet::PUBLISHES,
                                )
                                .union(
                                    crate::hole::RelationSet::EFFECTS,
                                );
                        }
                        _ => {}
                    }
                }
            }
        }
        m
    }

    pub fn validate(&self) -> Result<(), ModelError> {
        let e = &self.entities;
        let prov_len = self.provenance.records.len();

        // --- provenance record contents resolve (incl. inverted
        // ForeignSpan — an accepted-but-unrenderable record is the
        // same defect as a dangling SourceId; review round 5).
        let src_len = self.provenance.sources.len();
        for (i, rec) in self.provenance.records.iter().enumerate() {
            match rec {
                Provenance::Source { source, span } => {
                    if source.index() >= src_len || span.0 > span.1 {
                        return Err(ModelError::InvalidProvenanceRecord { index: i });
                    }
                }
                Provenance::ForeignSpan { span } => {
                    if span.0 > span.1 {
                        return Err(ModelError::InvalidProvenanceRecord { index: i });
                    }
                }
                Provenance::Synthetic { .. } => {}
            }
        }

        // --- canonical order: entity tables sort by canonical name.
        check_sorted_keys("functions", e.functions.iter().map(|f| &f.name))?;
        for (i, f) in e.functions.iter().enumerate() {
            // direct_effects is a sorted set (unlike `effects`,
            // whose declaration order is semantic in the artifact).
            if f.direct_effects.windows(2).any(|w| w[0] >= w[1]) {
                return Err(ModelError::NotCanonical {
                    table: "functions.direct_effects",
                    index: i,
                });
            }
        }
        check_sorted_keys("loci", e.loci.iter().map(|l| &l.name))?;
        check_sorted_keys("locus_instances", e.locus_instances.iter().map(|i| &i.path))?;
        check_sorted_keys("topics", e.topics.iter().map(|t| &t.name))?;
        check_sorted_keys("subjects", e.subjects.iter().map(|s| &s.pattern))?;
        check_sorted_keys(
            "payloads",
            e.payloads.iter().map(|p| (&p.shape, p.hash, p.opaque)),
        )?;
        check_sorted_keys("phases", e.phases.iter().map(|p| &p.name))?;
        check_sorted_keys("seeds", e.seeds.iter().map(|s| &s.name))?;
        check_sorted_keys("thread_domains", e.thread_domains.iter().map(|d| &d.name))?;
        check_sorted_keys("groups", e.groups.iter().map(|g| &g.name))?;
        check_sorted_keys("types", e.types.iter().map(|t| &t.name))?;
        check_sorted_keys(
            "effect_classes",
            e.effect_classes.iter().map(|c| &c.name),
        )?;
        check_sorted_keys("interfaces", e.interfaces.iter().map(|t| &t.name))?;
        check_sorted_keys(
            "declarations",
            e.declarations.iter().map(|d| (&d.name, d.kind)),
        )?;
        check_sorted_keys(
            "bindings",
            e.bindings
                .iter()
                .map(|b| (b.subject, b.transport.clone(), b.role)),
        )?;

        // --- ID ranges + provenance for entities.
        let fns = e.functions.len();
        let loci = e.loci.len();
        let insts = e.locus_instances.len();
        let topics = e.topics.len();
        let subjects = e.subjects.len();
        let payloads = e.payloads.len();
        let phases = e.phases.len();
        let seeds = e.seeds.len();
        let domains = e.thread_domains.len();
        let bindings = e.bindings.len();

        let prov = |table, index, p: ProvenanceId| {
            if p.index() >= prov_len {
                Err(ModelError::DanglingProvenance { table, index })
            } else {
                Ok(())
            }
        };
        for (i, c) in e.effect_classes.iter().enumerate() {
            // A composed definition is the NORMALIZED atomic
            // expansion: sorted, deduplicated, never
            // self-referential. Atomic and InvalidCycle carry no
            // atoms by construction.
            if let crate::entity::EffectClassDefinition::Composed {
                atoms,
            } = &c.definition
            {
                if atoms.windows(2).any(|w| w[0] >= w[1])
                    || atoms.iter().any(|m| *m == c.name)
                {
                    return Err(ModelError::NotCanonical {
                        table: "effect_classes.definition",
                        index: i,
                    });
                }
            }
            prov("effect_classes", i, c.provenance)?;
        }
        for (i, f) in e.functions.iter().enumerate() {
            prov("functions", i, f.provenance)?;
        }
        for (i, l) in e.loci.iter().enumerate() {
            prov("loci", i, l.provenance)?;
        }
        for (i, inst) in e.locus_instances.iter().enumerate() {
            if inst.decl.index() >= loci {
                return Err(ModelError::DanglingId {
                    table: "locus_instances",
                    index: i,
                });
            }
            prov("locus_instances", i, inst.provenance)?;
        }
        for (i, t) in e.topics.iter().enumerate() {
            if t.subject.index() >= subjects || t.payload.index() >= payloads {
                return Err(ModelError::DanglingId {
                    table: "topics",
                    index: i,
                });
            }
            if let Some(b) = &t.bound {
                if b.capacity == 0 {
                    return Err(ModelError::InvalidBound {
                        table: "topics",
                        index: i,
                    });
                }
            }
            prov("topics", i, t.provenance)?;
        }
        for (i, s) in e.subjects.iter().enumerate() {
            prov("subjects", i, s.provenance)?;
        }
        for (i, p) in e.payloads.iter().enumerate() {
            prov("payloads", i, p.provenance)?;
        }
        for (i, p) in e.phases.iter().enumerate() {
            prov("phases", i, p.provenance)?;
        }
        for (i, s) in e.seeds.iter().enumerate() {
            prov("seeds", i, s.provenance)?;
        }
        for (i, d) in e.thread_domains.iter().enumerate() {
            prov("thread_domains", i, d.provenance)?;
        }
        for (i, g) in e.groups.iter().enumerate() {
            prov("groups", i, g.provenance)?;
        }
        for (i, t) in e.types.iter().enumerate() {
            prov("types", i, t.provenance)?;
        }
        for (i, t) in e.interfaces.iter().enumerate() {
            prov("interfaces", i, t.provenance)?;
        }
        for (i, d) in e.declarations.iter().enumerate() {
            prov("declarations", i, d.provenance)?;
        }
        for (i, b) in e.bindings.iter().enumerate() {
            if b.subject.index() >= subjects {
                return Err(ModelError::DanglingId {
                    table: "bindings",
                    index: i,
                });
            }
            prov("bindings", i, b.provenance)?;
        }

        // --- entity-ref bound check, shared.
        let ref_ok = |r: &EntityRef| -> bool {
            match r {
                EntityRef::Function(id) => id.index() < fns,
                EntityRef::LocusDecl(id) => id.index() < loci,
                EntityRef::LocusInstance(id) => id.index() < insts,
                EntityRef::Topic(id) => id.index() < topics,
                EntityRef::Subject(id) => id.index() < subjects,
                EntityRef::Binding(id) => id.index() < bindings,
                EntityRef::ThreadDomain(id) => id.index() < domains,
                EntityRef::Phase(id) => id.index() < phases,
                EntityRef::Seed(id) => id.index() < seeds,
                EntityRef::Group(id) => id.index() < e.groups.len(),
                EntityRef::Type(id) => id.index() < e.types.len(),
                EntityRef::Interface(id) => id.index() < e.interfaces.len(),
                EntityRef::Declaration(id) => id.index() < e.declarations.len(),
            }
        };
        // Does a hole hiding `family` anchor at `at`?
        let hole_at = |at: EntityRef, family: crate::hole::RelationSet| {
            self.holes
                .iter()
                .any(|h| h.at == at && h.hides.intersects(family))
        };

        // --- relations: canonical order, ranges, provenance, and
        // the cross-authority agreement laws.
        let r = &self.relations;
        check_sorted_keys(
            "member_of",
            r.member_of.iter().map(|x| (x.function, x.locus)),
        )?;
        for (i, x) in r.member_of.iter().enumerate() {
            if x.function.index() >= fns || x.locus.index() >= loci {
                return Err(ModelError::DanglingId {
                    table: "member_of",
                    index: i,
                });
            }
            prov("member_of", i, x.provenance)?;
        }
        check_sorted_keys("phase_of", r.phase_of.iter().map(|x| (x.function, x.phase)))?;
        for (i, x) in r.phase_of.iter().enumerate() {
            if x.function.index() >= fns || x.phase.index() >= phases {
                return Err(ModelError::DanglingId {
                    table: "phase_of",
                    index: i,
                });
            }
            prov("phase_of", i, x.provenance)?;
        }
        check_sorted_keys(
            "declared_in",
            r.declared_in.iter().map(|x| (x.entity, x.seed)),
        )?;
        for (i, x) in r.declared_in.iter().enumerate() {
            if !ref_ok(&x.entity) || x.seed.index() >= seeds {
                return Err(ModelError::DanglingId {
                    table: "declared_in",
                    index: i,
                });
            }
            prov("declared_in", i, x.provenance)?;
        }
        check_sorted_keys("realizes", r.realizes.iter().map(|x| (x.instance, x.decl)))?;
        for (i, x) in r.realizes.iter().enumerate() {
            if x.instance.index() >= insts || x.decl.index() >= loci {
                return Err(ModelError::DanglingId {
                    table: "realizes",
                    index: i,
                });
            }
            prov("realizes", i, x.provenance)?;
        }
        // Realizes totality + uniqueness + agreement with the
        // instance's own `decl` field: one fact, two access paths,
        // provably identical.
        for (i, inst) in e.locus_instances.iter().enumerate() {
            let mut rows = r.realizes.iter().filter(|x| x.instance.index() == i);
            match (rows.next(), rows.next()) {
                (Some(row), None) => {
                    if row.decl != inst.decl {
                        return Err(ModelError::RealizesDisagrees { index: i });
                    }
                }
                _ => return Err(ModelError::RealizesIncomplete { instance: i }),
            }
        }
        // Replica index law (Change 8): the instances of one
        // `replicas = K` field are `base[0] … base[K-1]`, each
        // carrying its OWN index. The runtime pins replica `i` to
        // one core and a keyed subscriber on that field registers
        // under `key == i`, so a model that stored the COUNT in
        // every row (or skipped an index) would name a population
        // the process does not have. Grouped by the path base, so
        // two replicated fields cannot patch each other's gaps.
        {
            let mut by_base: std::collections::BTreeMap<
                &str,
                Vec<u32>,
            > = std::collections::BTreeMap::new();
            for inst in &e.locus_instances {
                // Replica-ness is a property of the LAST path
                // component only. A replica's own children keep
                // being ordinary children — `App.workers[0].leaf`
                // is a `leaf`, not replica 0 of anything — so an
                // ancestor's bracket must not make a descendant
                // answer for it.
                let last = inst.path.rfind('.').map_or(0, |d| d + 1);
                let open = inst.path[last..]
                    .strip_suffix(']')
                    .and_then(|c| c.rfind('['))
                    .map(|i| last + i);
                let Some(open) = open else {
                    // Not a replica path: it must not claim an index.
                    if inst.replica.is_some() {
                        return Err(
                            ModelError::ReplicaIndicesNotContiguous {
                                base: inst.path.clone(),
                            },
                        );
                    }
                    continue;
                };
                let Some(k) = inst.replica else {
                    return Err(ModelError::ReplicaIndicesNotContiguous {
                        base: inst.path[..open].to_string(),
                    });
                };
                // The path and the field are two spellings of ONE
                // fact; a row whose `base[2]` claims replica 0 is
                // two answers to "which replica is this".
                let in_path = inst.path[open + 1..]
                    .strip_suffix(']')
                    .and_then(|d| d.parse::<u32>().ok());
                if in_path != Some(k) {
                    return Err(ModelError::ReplicaIndicesNotContiguous {
                        base: inst.path[..open].to_string(),
                    });
                }
                by_base.entry(&inst.path[..open]).or_default().push(k);
            }
            for (base, mut ks) in by_base {
                ks.sort_unstable();
                let contiguous =
                    ks.iter().enumerate().all(|(i, k)| *k == i as u32);
                if !contiguous {
                    return Err(ModelError::ReplicaIndicesNotContiguous {
                        base: base.to_string(),
                    });
                }
            }
        }
        check_sorted_keys("owns", r.owns.iter().map(|x| (x.parent, x.child)))?;
        for (i, x) in r.owns.iter().enumerate() {
            if x.parent.index() >= insts || x.child.index() >= insts {
                return Err(ModelError::DanglingId {
                    table: "owns",
                    index: i,
                });
            }
            prov("owns", i, x.provenance)?;
        }
        check_sorted_keys(
            "calls",
            r.calls
                .iter()
                .map(|x| (x.from, x.to, x.dispatch.clone(), x.site)),
        )?;
        for (i, x) in r.calls.iter().enumerate() {
            if x.from.index() >= fns || x.to.index() >= fns {
                return Err(ModelError::DanglingId {
                    table: "calls",
                    index: i,
                });
            }
            prov("calls", i, x.provenance)?;
        }
        check_sorted_keys(
            "dead_interface_calls",
            r.dead_interface_calls.iter().map(|x| (x.from, x.site)),
        )?;
        for (i, x) in r.dead_interface_calls.iter().enumerate() {
            if x.from.index() >= fns {
                return Err(ModelError::DanglingId {
                    table: "dead_interface_calls",
                    index: i,
                });
            }
            prov("dead_interface_calls", i, x.provenance)?;
        }
        check_sorted_keys(
            "publishes",
            r.publishes.iter().map(|x| (x.function, x.subject, x.site)),
        )?;
        for (i, x) in r.publishes.iter().enumerate() {
            if x.function.index() >= fns
                || x.subject.index() >= subjects
                || x.declared_topic
                    .map(|t| t.index() >= topics)
                    .unwrap_or(false)
            {
                return Err(ModelError::DanglingId {
                    table: "publishes",
                    index: i,
                });
            }
            if x.payload.index() >= payloads {
                return Err(ModelError::DanglingId {
                    table: "publishes",
                    index: i,
                });
            }
            // A declared topic's subject AND payload must agree
            // with the row's.
            if let Some(t) = x.declared_topic {
                if e.topics[t.index()].subject != x.subject {
                    return Err(ModelError::DeclaredTopicDisagrees {
                        table: "publishes",
                        index: i,
                    });
                }
                if e.topics[t.index()].payload != x.payload {
                    return Err(ModelError::EndpointPayloadDisagrees {
                        table: "publishes",
                        index: i,
                    });
                }
            }
            // Keyed-ness must match the declared topic, both ways;
            // an undeclared (literal/wildcard) endpoint is unkeyed.
            let topic_keyed = x
                .declared_topic
                .map(|t| e.topics[t.index()].key.is_some())
                .unwrap_or(false);
            if topic_keyed != x.key_domain.is_some() {
                return Err(ModelError::KeyContract {
                    table: "publishes",
                    index: i,
                });
            }
            if let Some(crate::keys::KeyDomain::Exact(vals)) = &x.key_domain {
                check_sorted_keys("publishes.key_domain", vals.iter()).map_err(|_| {
                    ModelError::NotCanonical {
                        table: "publishes.key_domain",
                        index: i,
                    }
                })?;
            }
            if matches!(x.key_domain, Some(crate::keys::KeyDomain::Unknown))
                && !hole_at(
                    EntityRef::Function(x.function),
                    crate::hole::RelationSet::KEY_FILTERS,
                )
            {
                return Err(ModelError::UnrepresentedUnknown {
                    table: "publishes",
                    index: i,
                });
            }
            prov("publishes", i, x.provenance)?;
        }
        check_sorted_keys(
            "declares_publish",
            r.declares_publish
                .iter()
                .map(|x| (x.locus, x.subject, x.declared_topic)),
        )?;
        for (i, x) in r.declares_publish.iter().enumerate() {
            if x.locus.index() >= loci
                || x.subject.index() >= subjects
                || x.payload.index() >= payloads
                || x.declared_topic
                    .map(|t| t.index() >= topics)
                    .unwrap_or(false)
            {
                return Err(ModelError::DanglingId {
                    table: "declares_publish",
                    index: i,
                });
            }
            if let Some(t) = x.declared_topic {
                if e.topics[t.index()].subject != x.subject {
                    return Err(ModelError::DeclaredTopicDisagrees {
                        table: "declares_publish",
                        index: i,
                    });
                }
                if e.topics[t.index()].payload != x.payload {
                    return Err(ModelError::EndpointPayloadDisagrees {
                        table: "declares_publish",
                        index: i,
                    });
                }
            }
            prov("declares_publish", i, x.provenance)?;
        }
        check_sorted_keys(
            "subscribes",
            r.subscribes.iter().map(|x| (x.subject, x.handler, x.site)),
        )?;
        for (i, x) in r.subscribes.iter().enumerate() {
            if x.subject.index() >= subjects
                || x.handler.index() >= fns
                || x.declared_topic
                    .map(|t| t.index() >= topics)
                    .unwrap_or(false)
            {
                return Err(ModelError::DanglingId {
                    table: "subscribes",
                    index: i,
                });
            }
            if x.payload.index() >= payloads {
                return Err(ModelError::DanglingId {
                    table: "subscribes",
                    index: i,
                });
            }
            if let Some(t) = x.declared_topic {
                if e.topics[t.index()].subject != x.subject {
                    return Err(ModelError::DeclaredTopicDisagrees {
                        table: "subscribes",
                        index: i,
                    });
                }
                if e.topics[t.index()].payload != x.payload {
                    return Err(ModelError::EndpointPayloadDisagrees {
                        table: "subscribes",
                        index: i,
                    });
                }
            }
            if matches!(x.capacity, crate::keys::Capacity::Bounded(0)) {
                return Err(ModelError::InvalidBound {
                    table: "subscribes",
                    index: i,
                });
            }
            // A keyed predicate needs a keyed declared topic;
            // unkeyed and undeclared endpoints admit only the plain
            // full-delivery subscription.
            let sub_keyed = x
                .declared_topic
                .map(|t| e.topics[t.index()].key.is_some())
                .unwrap_or(false);
            if !sub_keyed && !matches!(x.key_predicate, crate::keys::KeyPredicate::Any) {
                return Err(ModelError::KeyContract {
                    table: "subscribes",
                    index: i,
                });
            }
            if matches!(x.key_predicate, crate::keys::KeyPredicate::Unknown)
                && !hole_at(
                    EntityRef::Function(x.handler),
                    crate::hole::RelationSet::KEY_FILTERS,
                )
            {
                return Err(ModelError::UnrepresentedUnknown {
                    table: "subscribes",
                    index: i,
                });
            }
            prov("subscribes", i, x.provenance)?;
        }
        // Fallback contract, both directions: `_` only on fallback
        // topics; fallback topics have their catch.
        for (i, x) in r.subscribes.iter().enumerate() {
            let is_fallback_pred = matches!(x.key_predicate, crate::keys::KeyPredicate::Fallback);
            let topic_policy = x
                .declared_topic
                .and_then(|t| e.topics[t.index()].key.as_ref().map(|k| k.on_unmatched));
            if is_fallback_pred && topic_policy != Some(crate::keys::KeyOnUnmatched::Fallback) {
                return Err(ModelError::IllegalFallback { index: i });
            }
        }
        for (ti, t) in e.topics.iter().enumerate() {
            let wants_fallback = t
                .key
                .as_ref()
                .map(|k| k.on_unmatched == crate::keys::KeyOnUnmatched::Fallback)
                .unwrap_or(false);
            if wants_fallback
                && !r.subscribes.iter().any(|x| {
                    x.declared_topic.map(|t| t.index()) == Some(ti)
                        && matches!(x.key_predicate, crate::keys::KeyPredicate::Fallback)
                })
            {
                return Err(ModelError::FallbackUncovered { topic: ti });
            }
        }
        check_sorted_keys("placed_in", r.placed_in.iter().map(|x| x.instance))?;
        for (i, x) in r.placed_in.iter().enumerate() {
            if x.instance.index() >= insts || x.domain.index() >= domains {
                return Err(ModelError::DanglingId {
                    table: "placed_in",
                    index: i,
                });
            }
            prov("placed_in", i, x.provenance)?;
        }
        check_sorted_keys("affined_to", r.affined_to.iter().map(|x| x.domain))?;
        for (i, x) in r.affined_to.iter().enumerate() {
            if x.domain.index() >= domains {
                return Err(ModelError::DanglingId {
                    table: "affined_to",
                    index: i,
                });
            }
            check_sorted_keys("affined_to.cores", x.cores.0.iter()).map_err(|_| {
                ModelError::NotCanonical {
                    table: "affined_to.cores",
                    index: i,
                }
            })?;
            prov("affined_to", i, x.provenance)?;
        }
        check_sorted_keys("binds", r.binds.iter().map(|x| (x.topic, x.binding)))?;
        for (i, x) in r.binds.iter().enumerate() {
            if x.topic.index() >= topics || x.binding.index() >= bindings {
                return Err(ModelError::DanglingId {
                    table: "binds",
                    index: i,
                });
            }
            // The topic's subject and the binding's subject repeat
            // one fact — they must agree.
            if e.topics[x.topic.index()].subject != e.bindings[x.binding.index()].subject {
                return Err(ModelError::BindingSubjectDisagrees { index: i });
            }
            prov("binds", i, x.provenance)?;
        }
        check_sorted_keys(
            "supervises",
            r.supervises.iter().map(|x| {
                (
                    x.parent,
                    x.child.clone(),
                    &x.error_type,
                    x.authored_ordinal,
                )
            }),
        )?;
        for (i, x) in r.supervises.iter().enumerate() {
            let child_ok = match &x.child {
                crate::relation::SupervisedRef::Locus(id) => {
                    id.index() < loci
                }
                crate::relation::SupervisedRef::External(_) => true,
            };
            if x.parent.index() >= loci || !child_ok {
                return Err(ModelError::DanglingId {
                    table: "supervises",
                    index: i,
                });
            }
            prov("supervises", i, x.provenance)?;
        }

        check_sorted_keys(
            "group_members",
            r.group_members.iter().map(|x| (x.group, x.member)),
        )?;
        let groups_len = e.groups.len();
        for (i, x) in r.group_members.iter().enumerate() {
            if x.group.index() >= groups_len || !ref_ok(&x.member) {
                return Err(ModelError::DanglingId {
                    table: "group_members",
                    index: i,
                });
            }
            prov("group_members", i, x.provenance)?;
        }
        check_sorted_keys(
            "group_selectors",
            r.group_selectors.iter().map(|x| (x.group, x.ordinal)),
        )?;
        for (i, x) in r.group_selectors.iter().enumerate() {
            let sel_ok = match &x.selector {
                SelectorForm::Named { member, .. } => ref_ok(member),
                SelectorForm::SeedGlob { seed, .. } => {
                    seed.index() < seeds
                }
            };
            if x.group.index() >= groups_len || !sel_ok {
                return Err(ModelError::DanglingId {
                    table: "group_selectors",
                    index: i,
                });
            }
            prov("group_selectors", i, x.provenance)?;
        }

        // --- labels / weights.
        // Labels are grouped by entity in canonical entity order,
        // but WITHIN one entity the label order is semantic — the
        // declared class order the artifact hashes (`zebra` declared
        // before `alpha` renders before it) — so rows are NOT fully
        // key-sorted (review round 11; mirrors `Function.effects`,
        // whose declaration order was already preserved).
        {
            let mut prev_at: Option<crate::ids::EntityRef> = None;
            let mut seen: std::collections::BTreeSet<(
                crate::ids::EntityRef,
                &String,
            )> = std::collections::BTreeSet::new();
            for (i, l) in self.labels.iter().enumerate() {
                if let Some(p) = prev_at {
                    if l.at < p {
                        return Err(ModelError::NotCanonical {
                            table: "labels",
                            index: i,
                        });
                    }
                }
                prev_at = Some(l.at);
                if !seen.insert((l.at, &l.label)) {
                    return Err(ModelError::NotCanonical {
                        table: "labels",
                        index: i,
                    });
                }
            }
        }
        for (i, l) in self.labels.iter().enumerate() {
            if !ref_ok(&l.at) {
                return Err(ModelError::DanglingId {
                    table: "labels",
                    index: i,
                });
            }
            prov("labels", i, l.provenance)?;
        }
        check_sorted_keys("weights", self.weights.iter().map(|w| (w.at, &w.metric)))?;
        for (i, w) in self.weights.iter().enumerate() {
            if !ref_ok(&w.at) {
                return Err(ModelError::DanglingId {
                    table: "weights",
                    index: i,
                });
            }
            prov("weights", i, w.provenance)?;
        }

        // --- holes: canonical, anchored, non-empty, provenanced.
        check_sorted_keys(
            "holes",
            self.holes.iter().map(|h| (h.at, h.kind.clone(), &h.reason)),
        )?;
        for (i, h) in self.holes.iter().enumerate() {
            if !ref_ok(&h.at) {
                return Err(ModelError::DanglingId {
                    table: "holes",
                    index: i,
                });
            }
            if h.hides.is_empty() {
                return Err(ModelError::EmptyHole { index: i });
            }
            // The closed shape matrix (round 7): only KNOWN family
            // bits, only defined (anchor, kind, family)
            // combinations, and authored positions only at fn
            // grain — a hole every judgment silently ignores would
            // let the model claim its unknowns are accounted for.
            if !crate::hole::RelationSet::ALL_KNOWN
                .contains(h.hides)
            {
                return Err(ModelError::EmptyHole { index: i });
            }
            match crate::hole::allowed_hole_families(&h.at, &h.kind)
            {
                Some((required, allowed))
                    if allowed.contains(h.hides)
                        && h.hides.contains(required) => {}
                _ => {
                    return Err(ModelError::NotCanonical {
                        table: "holes.shape",
                        index: i,
                    });
                }
            }
            if !matches!(h.at, EntityRef::Function(_))
                && h.authored_site.is_some()
            {
                return Err(ModelError::NotCanonical {
                    table: "holes.shape",
                    index: i,
                });
            }
            // Site identity is EXCLUSIVE, not optional (round 8):
            // a site-shaped hole stands for one authored
            // expression and requires its ordinal; a whole-body
            // hole has no single position and must not carry one.
            if matches!(h.at, EntityRef::Function(_))
                && crate::hole::hole_site_shaped(&h.kind)
                    != h.authored_site.is_some()
            {
                return Err(ModelError::NotCanonical {
                    table: "holes.shape",
                    index: i,
                });
            }
            prov("holes", i, h.provenance)?;
        }

        // --- legacy projection: sorted ids, all in range.
        check_sorted_keys(
            "legacy.topology_v1_fns",
            self.legacy.topology_v1_fns.iter(),
        )?;
        for (i, f) in self.legacy.topology_v1_fns.iter().enumerate() {
            if f.index() >= fns {
                return Err(ModelError::DanglingId {
                    table: "legacy.topology_v1_fns",
                    index: i,
                });
            }
        }
        // NON-strict ordering: one dispatch site can fan out to
        // SEVERAL stdlib conformers — multiple entries legitimately
        // share (from, site).
        for (i, w) in
            self.legacy.stdlib_absorption.windows(2).enumerate()
        {
            if (w[0].from, w[0].site) > (w[1].from, w[1].site) {
                return Err(ModelError::NotCanonical {
                    table: "legacy.stdlib_absorption",
                    index: i + 1,
                });
            }
        }
        // One authored call site = ONE dispatch (review round 6).
        // `bound` folds a (from, site) class with MAX because the
        // schema's contract says multiple rows at one site are the
        // conformer ALTERNATIVES of one interface dispatch; two
        // direct calls are two authored sites and must SUM. Enforce
        // the shape: a class with more than one member — counting
        // non-ViaStdlib call rows and absorption entries together —
        // must be all interface alternatives (call rows of ONE
        // interface; absorption entries with a dispatch rendering).
        // ViaStdlib rows are the legacy contraction of the same
        // authored call an absorption entry represents, so they are
        // excluded from the count.
        {
            use crate::relation::DispatchKind;
            // (member count, still-lawful, the ONE dispatch pair —
            // interface DISPLAY + method — every member must share)
            struct SiteClass {
                n: usize,
                ok: bool,
                pair: Option<(String, String)>,
            }
            let iface_display = |raw: &str| -> String {
                e.interfaces
                    .iter()
                    .find(|x| x.name == raw)
                    .map(|x| x.display.clone())
                    .unwrap_or_else(|| raw.to_string())
            };
            let mut class: std::collections::BTreeMap<
                (u32, u32),
                SiteClass,
            > = std::collections::BTreeMap::new();
            let mut join = |key: (u32, u32),
                            pair: Option<(String, String)>| {
                let entry =
                    class.entry(key).or_insert(SiteClass {
                        n: 0,
                        ok: true,
                        pair: None,
                    });
                entry.n += 1;
                match (pair, &entry.pair) {
                    (None, _) => entry.ok = false,
                    (Some(p), None) => entry.pair = Some(p),
                    (Some(p), Some(prev)) => {
                        if p != *prev {
                            entry.ok = false;
                        }
                    }
                }
            };
            for c in &self.relations.calls {
                if matches!(c.dispatch, DispatchKind::ViaStdlib) {
                    continue;
                }
                let pair = match &c.dispatch {
                    DispatchKind::Interface { interface } => {
                        let method = e.functions[c.to.index()]
                            .name
                            .rsplit("::")
                            .next()
                            .unwrap_or_default()
                            .to_string();
                        Some((iface_display(interface), method))
                    }
                    _ => None,
                };
                join((c.from.0, c.site), pair);
            }
            for a in &self.legacy.stdlib_absorption {
                join(
                    (a.from.0, a.site),
                    a.entry_dispatch.clone(),
                );
            }
            for (i, (_, cl)) in class.iter().enumerate() {
                // >1 member ⇒ the alternatives of ONE dispatch:
                // interface rows with ONE (interface, method)
                // identity, absorption entries carrying the SAME
                // rendering (round 7: entry compatibility is
                // identity agreement, never mere presence).
                if cl.n > 1 && !cl.ok {
                    return Err(ModelError::NotCanonical {
                        table: "calls.dispatch_site",
                        index: i,
                    });
                }
            }
        }

        for (i, a) in self.legacy.stdlib_absorption.iter().enumerate()
        {
            if a.from.index() >= fns || a.nodes.is_empty() {
                return Err(ModelError::DanglingId {
                    table: "legacy.stdlib_absorption",
                    index: i,
                });
            }
            // The entry's dispatch label binds to the entry
            // NODE's method identity (round 8) — textual agreement
            // with user alternatives is not enough when node zero
            // is an unrelated method.
            if let Some((_, m)) = &a.entry_dispatch {
                let tail = a.nodes[0]
                    .display
                    .rsplit("::")
                    .next()
                    .unwrap_or_default();
                if tail != m {
                    return Err(ModelError::NotCanonical {
                        table:
                            "legacy.stdlib_absorption.dispatch",
                        index: i,
                    });
                }
            }
            // The reachability judgment renders entry_provenance as
            // the authored crossing span — a dangling id would
            // silently render 0..0 (review round 5).
            if a.entry_provenance.index()
                >= self.provenance.records.len()
            {
                return Err(ModelError::DanglingId {
                    table: "legacy.stdlib_absorption",
                    index: i,
                });
            }
            for n in &a.nodes {
                // Interior dispatch identity (round 7): one group
                // id inside a node is ONE dispatch — `bound` folds
                // same-group alternatives with MAX, so an
                // arbitrary bucket would absorb what must sum.
                let mut node_groups: std::collections::BTreeMap<
                    u32,
                    &(String, String),
                > = std::collections::BTreeMap::new();
                for ev in &n.events {
                    match ev {
                        crate::AbsorbedEvent::Call {
                            target,
                            dispatch,
                            group,
                            ..
                        } => {
                            let ok = match target {
                                crate::AbsorbedTarget::Interior(
                                    k,
                                ) => (*k as usize) < a.nodes.len(),
                                crate::AbsorbedTarget::User(f) => {
                                    f.index() < fns
                                }
                            };
                            if !ok {
                                return Err(ModelError::DanglingId {
                                    table:
                                        "legacy.stdlib_absorption",
                                    index: i,
                                });
                            }
                            // A dispatch label binds to its
                            // TARGET (round 8): the rendered
                            // method must be the target's own
                            // method identity, so a group can
                            // never collect calls to unrelated
                            // functions under one label.
                            if let Some((_, m)) = dispatch {
                                let tail = match target {
                                    crate::AbsorbedTarget::User(
                                        f,
                                    ) => e.functions[f.index()]
                                        .name
                                        .rsplit("::")
                                        .next()
                                        .unwrap_or_default(),
                                    crate::AbsorbedTarget::Interior(
                                        k,
                                    ) => a.nodes[*k as usize]
                                        .display
                                        .rsplit("::")
                                        .next()
                                        .unwrap_or_default(),
                                };
                                if tail != m {
                                    return Err(
                                        ModelError::NotCanonical {
                                            table:
                                                "legacy.stdlib_absorption.dispatch",
                                            index: i,
                                        },
                                    );
                                }
                            }
                            // group ⇔ dispatch, and same group ⇒
                            // identical (interface, method)
                            // (round 7).
                            let lawful = match (group, dispatch) {
                                (None, None) => true,
                                (Some(g), Some(d)) => {
                                    match node_groups.entry(*g) {
                                        std::collections::btree_map::Entry::Vacant(v) => {
                                            v.insert(d);
                                            true
                                        }
                                        std::collections::btree_map::Entry::Occupied(o) => {
                                            *o.get() == d
                                        }
                                    }
                                }
                                _ => false,
                            };
                            if !lawful {
                                return Err(
                                    ModelError::NotCanonical {
                                        table:
                                            "legacy.stdlib_absorption.dispatch",
                                        index: i,
                                    },
                                );
                            }
                        }
                        // The TYPED publish identity judgments
                        // trust (round 7): a present TopicId must
                        // be in range, and the delivery spelling
                        // must be exactly that topic's name — a
                        // disagreement would route hole coverage
                        // as one topic while known delivery looks
                        // up another.
                        crate::AbsorbedEvent::Publish {
                            subject,
                            declared_topic,
                            ..
                        } => {
                            if let Some(t) = declared_topic {
                                let ok = t.index()
                                    < e.topics.len()
                                    && e.topics[t.index()].name
                                        == *subject;
                                if !ok {
                                    return Err(
                                        ModelError::DanglingId {
                                            table:
                                                "legacy.stdlib_absorption",
                                            index: i,
                                        },
                                    );
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        check_sorted_keys(
            "legacy.topology_v1_calls_via_stdlib",
            self.legacy
                .topology_v1_calls_via_stdlib
                .iter()
                .map(|(f, t, _)| (f, t)),
        )?;
        {
            let legacy_set: std::collections::BTreeSet<&FunctionId> =
                self.legacy.topology_v1_fns.iter().collect();
            for (i, (f, t, _)) in self
                .legacy
                .topology_v1_calls_via_stdlib
                .iter()
                .enumerate()
            {
                // Endpoint law: a legacy contracted row only ever
                // connects legacy fns — the legacy walk starts and
                // ends at its own serialized sort.
                if !legacy_set.contains(f) || !legacy_set.contains(t) {
                    return Err(ModelError::DanglingId {
                        table: "legacy.topology_v1_calls_via_stdlib",
                        index: i,
                    });
                }
            }
        }

        // --- capability/hole contradiction: exactness may not be
        // claimed for a family any hole hides. Every capability is
        // mapped (an unmapped flag would be unfalsifiable).
        // The authored-site event partition (round 8): one
        // (function, site) in CALL space is occupied by resolved
        // alternatives (call rows and/or absorption entries — the
        // dispatch-identity laws bind those), OR dead interface
        // rows, OR one typed call hole — never a mixture; in
        // PUBLISH space, a known publish row and a
        // computed-subject hole are likewise exclusive. One
        // authored expression cannot be two events, and a judgment
        // must never have to invent which comes first.
        {
            use crate::relation::DispatchKind;
            #[derive(Default)]
            struct Occ {
                resolved: bool,
                dead: bool,
                holes: u8,
            }
            let mut call_sites: std::collections::BTreeMap<
                (u32, u32),
                Occ,
            > = std::collections::BTreeMap::new();
            for c in &self.relations.calls {
                if matches!(c.dispatch, DispatchKind::ViaStdlib) {
                    continue;
                }
                call_sites
                    .entry((c.from.0, c.site))
                    .or_default()
                    .resolved = true;
            }
            for a in &self.legacy.stdlib_absorption {
                call_sites
                    .entry((a.from.0, a.site))
                    .or_default()
                    .resolved = true;
            }
            for d in &self.relations.dead_interface_calls {
                call_sites
                    .entry((d.from.0, d.site))
                    .or_default()
                    .dead = true;
            }
            let mut pub_holes: std::collections::BTreeSet<(
                u32,
                u32,
            )> = std::collections::BTreeSet::new();
            for (i, h) in self.holes.iter().enumerate() {
                let EntityRef::Function(f) = h.at else {
                    continue;
                };
                let Some(site) = h.authored_site else {
                    continue;
                };
                if matches!(
                    h.kind,
                    crate::hole::HoleKind::ComputedSubject
                ) {
                    // Exactly ONE event per publish site (round
                    // 9): a second computed-subject hole in the
                    // same site is rejected, not collapsed.
                    if !pub_holes.insert((f.0, site)) {
                        return Err(ModelError::NotCanonical {
                            table: "publishes.site_partition",
                            index: i,
                        });
                    }
                } else if crate::hole::hole_site_shaped(&h.kind) {
                    let occ = call_sites
                        .entry((f.0, site))
                        .or_default();
                    occ.holes = occ.holes.saturating_add(1);
                }
            }
            for (i, (_, occ)) in call_sites.iter().enumerate() {
                // Exactly ONE event, not one event category
                // (round 9): two typed holes in one call site
                // would leave the judgment picking a diagnostic by
                // canonical kind order rather than by an authored
                // event.
                let cats = usize::from(occ.resolved)
                    + usize::from(occ.dead)
                    + usize::from(occ.holes > 0);
                if cats > 1 || occ.holes > 1 {
                    return Err(ModelError::NotCanonical {
                        table: "calls.site_partition",
                        index: i,
                    });
                }
            }
            for (i, p) in self.relations.publishes.iter().enumerate()
            {
                if pub_holes.contains(&(p.function.0, p.site)) {
                    return Err(ModelError::NotCanonical {
                        table: "publishes.site_partition",
                        index: i,
                    });
                }
            }
        }
        // The contracted relation and the absorption sidecar are
        // DUAL ACCOUNTS of the same through-stdlib paths (round 9)
        // and may not disagree: every `ViaStdlib` call row must be
        // realized by a re-emergent User target inside one of its
        // fn's absorption entries, and every re-emergence must
        // have its contracted row. The judgments walk the
        // absorption and skip the contracted rows — a row without
        // an interior would be an edge every judgment discards,
        // and an interior re-emergence without its row would be an
        // edge the legacy projection denies.
        {
            use crate::relation::DispatchKind;
            let mut contracted: std::collections::BTreeMap<
                u32,
                std::collections::BTreeSet<u32>,
            > = std::collections::BTreeMap::new();
            for c in &self.relations.calls {
                if matches!(c.dispatch, DispatchKind::ViaStdlib) {
                    contracted
                        .entry(c.from.0)
                        .or_default()
                        .insert(c.to.0);
                }
            }
            let mut absorbed: std::collections::BTreeMap<
                u32,
                std::collections::BTreeSet<u32>,
            > = std::collections::BTreeMap::new();
            for a in &self.legacy.stdlib_absorption {
                let set =
                    absorbed.entry(a.from.0).or_default();
                for n in &a.nodes {
                    for ev in &n.events {
                        if let crate::AbsorbedEvent::Call {
                            target:
                                crate::AbsorbedTarget::User(f2),
                            ..
                        } = ev
                        {
                            set.insert(f2.0);
                        }
                    }
                }
            }
            absorbed.retain(|_, v| !v.is_empty());
            contracted.retain(|_, v| !v.is_empty());
            if contracted != absorbed {
                return Err(ModelError::NotCanonical {
                    table: "calls.via_stdlib_agreement",
                    index: 0,
                });
            }
        }
        for (name, claimed, family) in self.capabilities.vouched_families() {
            if !claimed {
                continue;
            }
            if self.unresolved_relation_mask().intersects(family) {
                return Err(ModelError::CapabilityContradiction { capability: name });
            }
        }
        // --- ownership + coverage laws (rounds 10–15): one
        // shared validator, also called by the sidecar API. Runs
        // LAST so purely structural defects (non-canonical
        // ordering, dangling ids) report as themselves.
        self.validate_coverage()?;

        Ok(())
    }
}

/// Strictly-increasing check over a table's canonical keys —
/// enforces sorted AND deduplicated in one pass.
fn check_sorted_keys<K: Ord, I>(table: &'static str, keys: I) -> Result<(), ModelError>
where
    I: Iterator<Item = K>,
{
    let mut prev: Option<K> = None;
    for (i, k) in keys.enumerate() {
        if let Some(p) = &prev {
            if *p >= k {
                return Err(ModelError::NotCanonical { table, index: i });
            }
        }
        prev = Some(k);
    }
    Ok(())
}
