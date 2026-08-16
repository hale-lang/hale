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
    Binding, Declaration, Function, Group, InterfaceDecl, LocusDecl, LocusInstance,
    PayloadContract, Phase, Seed, Subject, ThreadDomain, Topic, TypeDecl,
};
use crate::hole::Hole;
use crate::ids::{EntityRef, FunctionId, ProvenanceId};
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
    /// A `binds` row joins a topic and a binding whose subjects
    /// disagree — the entity attribute and the relation repeat one
    /// fact and drifted.
    BindingSubjectDisagrees { index: usize },
    /// A `where key == _` (Fallback) subscription on a topic whose
    /// `on_unmatched` policy is not `fallback` — the `_` sentinel
    /// is legal only on fallback topics (resolve-time law,
    /// mirrored in the schema).
    IllegalFallback { index: usize },
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
    /// | `declares_publish` | (locus, subject)               |
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
    pub fn validate(&self) -> Result<(), ModelError> {
        let e = &self.entities;
        let prov_len = self.provenance.records.len();

        // --- provenance record contents resolve.
        let src_len = self.provenance.sources.len();
        for (i, rec) in self.provenance.records.iter().enumerate() {
            if let Provenance::Source { source, span } = rec {
                if source.index() >= src_len || span.0 > span.1 {
                    return Err(ModelError::InvalidProvenanceRecord { index: i });
                }
            }
        }

        // --- canonical order: entity tables sort by canonical name.
        check_sorted_keys("functions", e.functions.iter().map(|f| &f.name))?;
        check_sorted_keys("loci", e.loci.iter().map(|l| &l.name))?;
        check_sorted_keys("locus_instances", e.locus_instances.iter().map(|i| &i.path))?;
        check_sorted_keys("topics", e.topics.iter().map(|t| &t.name))?;
        check_sorted_keys("subjects", e.subjects.iter().map(|s| &s.pattern))?;
        check_sorted_keys("payloads", e.payloads.iter().map(|p| (&p.shape, p.hash)))?;
        check_sorted_keys("phases", e.phases.iter().map(|p| &p.name))?;
        check_sorted_keys("seeds", e.seeds.iter().map(|s| &s.name))?;
        check_sorted_keys("thread_domains", e.thread_domains.iter().map(|d| &d.name))?;
        check_sorted_keys("groups", e.groups.iter().map(|g| &g.name))?;
        check_sorted_keys("types", e.types.iter().map(|t| &t.name))?;
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
            r.declares_publish.iter().map(|x| (x.locus, x.subject)),
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
            r.supervises
                .iter()
                .map(|x| (x.parent, x.child.clone(), &x.error_type)),
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
        for (name, claimed, family) in self.capabilities.vouched_families() {
            if !claimed {
                continue;
            }
            if self.holes.iter().any(|h| h.hides.intersects(family)) {
                return Err(ModelError::CapabilityContradiction { capability: name });
            }
        }
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
