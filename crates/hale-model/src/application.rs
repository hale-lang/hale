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
    Binding, Function, LocusDecl, LocusInstance, PayloadContract, Phase,
    Seed, Subject, ThreadDomain, Topic,
};
use crate::hole::Hole;
use crate::ids::{EntityRef, ProvenanceId};
use crate::provenance::ProvenanceTable;
use crate::relation::{
    AffinedTo, Call, DeclaredIn, MemberOf, Owns, PhaseOf, PlacedIn,
    Publish, Realizes, Subscribe, Supervises, TopicBinding,
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
}

#[derive(Clone, Default, Debug)]
pub struct Relations {
    pub member_of: Vec<MemberOf>,
    pub phase_of: Vec<PhaseOf>,
    pub declared_in: Vec<DeclaredIn>,
    pub realizes: Vec<Realizes>,
    pub owns: Vec<Owns>,
    pub calls: Vec<Call>,
    pub publishes: Vec<Publish>,
    pub subscribes: Vec<Subscribe>,
    pub placed_in: Vec<PlacedIn>,
    pub affined_to: Vec<AffinedTo>,
    pub binds: Vec<TopicBinding>,
    pub supervises: Vec<Supervises>,
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
}

/// A violated model law. `validate` returns the FIRST violation —
/// builders fix laws one at a time; consumers treat any error as
/// "this value is not a model".
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ModelError {
    /// A table is not canonically sorted + deduplicated. Carries the
    /// table name and the first offending index.
    NotCanonical { table: &'static str, index: usize },
    /// A row references an ID outside its table.
    DanglingId { table: &'static str, index: usize },
    /// A row's provenance ID is out of range.
    DanglingProvenance { table: &'static str, index: usize },
    /// A hole that hides no relation family is not a hole.
    EmptyHole { index: usize },
    /// A capability claims exactness while a hole hides that
    /// family — the two accounts of completeness drifted.
    CapabilityContradiction { capability: &'static str },
}

impl ApplicationModel {
    /// Check every model law. A value that fails is not a model and
    /// must not be judged, hashed, projected, or composed.
    pub fn validate(&self) -> Result<(), ModelError> {
        let e = &self.entities;
        let prov_len = self.provenance.records.len();

        // --- canonical order: entity tables sort by canonical name.
        check_sorted("functions", e.functions.iter().map(|f| &f.name))?;
        check_sorted("loci", e.loci.iter().map(|l| &l.name))?;
        check_sorted(
            "locus_instances",
            e.locus_instances.iter().map(|i| &i.path),
        )?;
        check_sorted("topics", e.topics.iter().map(|t| &t.name))?;
        check_sorted("subjects", e.subjects.iter().map(|s| &s.pattern))?;
        check_sorted("phases", e.phases.iter().map(|p| &p.name))?;
        check_sorted("seeds", e.seeds.iter().map(|s| &s.name))?;
        check_sorted(
            "thread_domains",
            e.thread_domains.iter().map(|d| &d.name),
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
            if t.subject.index() >= subjects || t.payload.index() >= payloads
            {
                return Err(ModelError::DanglingId {
                    table: "topics",
                    index: i,
                });
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
            }
        };

        // --- relations: canonical order, ranges, provenance.
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
        check_sorted_keys(
            "phase_of",
            r.phase_of.iter().map(|x| (x.function, x.phase)),
        )?;
        for (i, x) in r.phase_of.iter().enumerate() {
            if x.function.index() >= fns || x.phase.index() >= phases {
                return Err(ModelError::DanglingId {
                    table: "phase_of",
                    index: i,
                });
            }
            prov("phase_of", i, x.provenance)?;
        }
        for (i, x) in r.declared_in.iter().enumerate() {
            if !ref_ok(&x.entity) || x.seed.index() >= seeds {
                return Err(ModelError::DanglingId {
                    table: "declared_in",
                    index: i,
                });
            }
            prov("declared_in", i, x.provenance)?;
        }
        check_sorted_keys(
            "realizes",
            r.realizes.iter().map(|x| (x.instance, x.decl)),
        )?;
        for (i, x) in r.realizes.iter().enumerate() {
            if x.instance.index() >= insts || x.decl.index() >= loci {
                return Err(ModelError::DanglingId {
                    table: "realizes",
                    index: i,
                });
            }
            prov("realizes", i, x.provenance)?;
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
        for (i, x) in r.calls.iter().enumerate() {
            if x.from.index() >= fns || x.to.index() >= fns {
                return Err(ModelError::DanglingId {
                    table: "calls",
                    index: i,
                });
            }
            prov("calls", i, x.provenance)?;
        }
        for (i, x) in r.publishes.iter().enumerate() {
            if x.function.index() >= fns || x.topic.index() >= topics {
                return Err(ModelError::DanglingId {
                    table: "publishes",
                    index: i,
                });
            }
            prov("publishes", i, x.provenance)?;
        }
        for (i, x) in r.subscribes.iter().enumerate() {
            if x.topic.index() >= topics || x.handler.index() >= fns {
                return Err(ModelError::DanglingId {
                    table: "subscribes",
                    index: i,
                });
            }
            prov("subscribes", i, x.provenance)?;
        }
        for (i, x) in r.placed_in.iter().enumerate() {
            if x.instance.index() >= insts || x.domain.index() >= domains {
                return Err(ModelError::DanglingId {
                    table: "placed_in",
                    index: i,
                });
            }
            prov("placed_in", i, x.provenance)?;
        }
        for (i, x) in r.affined_to.iter().enumerate() {
            if x.domain.index() >= domains {
                return Err(ModelError::DanglingId {
                    table: "affined_to",
                    index: i,
                });
            }
            prov("affined_to", i, x.provenance)?;
        }
        for (i, x) in r.binds.iter().enumerate() {
            if x.topic.index() >= topics || x.binding.index() >= bindings {
                return Err(ModelError::DanglingId {
                    table: "binds",
                    index: i,
                });
            }
            prov("binds", i, x.provenance)?;
        }
        for (i, x) in r.supervises.iter().enumerate() {
            if x.parent.index() >= loci || x.child.index() >= loci {
                return Err(ModelError::DanglingId {
                    table: "supervises",
                    index: i,
                });
            }
            prov("supervises", i, x.provenance)?;
        }

        // --- labels / weights.
        for (i, l) in self.labels.iter().enumerate() {
            if !ref_ok(&l.at) {
                return Err(ModelError::DanglingId {
                    table: "labels",
                    index: i,
                });
            }
            prov("labels", i, l.provenance)?;
        }
        for (i, w) in self.weights.iter().enumerate() {
            if !ref_ok(&w.at) {
                return Err(ModelError::DanglingId {
                    table: "weights",
                    index: i,
                });
            }
            prov("weights", i, w.provenance)?;
        }

        // --- holes: anchored, non-empty, provenanced.
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

        // --- capability/hole contradiction: exactness may not be
        // claimed for a family any hole hides.
        for (claimed, family) in self.capabilities.vouched_families() {
            if !claimed {
                continue;
            }
            if self.holes.iter().any(|h| h.hides.intersects(family)) {
                let capability = match family.0 {
                    x if x == crate::hole::RelationSet::CALLS.0 => {
                        "exact_calls"
                    }
                    x if x == crate::hole::RelationSet::OWNS.0 => {
                        "exact_ownership"
                    }
                    x if x == crate::hole::RelationSet::PLACED.0 => {
                        "exact_placement"
                    }
                    x if x == crate::hole::RelationSet::ROUTES.0 => {
                        "exact_routes"
                    }
                    x if x == crate::hole::RelationSet::EFFECTS.0 => {
                        "exact_effects"
                    }
                    _ => "exact_bus_endpoints",
                };
                return Err(ModelError::CapabilityContradiction {
                    capability,
                });
            }
        }
        Ok(())
    }
}

fn check_sorted<'a, I>(table: &'static str, names: I) -> Result<(), ModelError>
where
    I: Iterator<Item = &'a String>,
{
    let mut prev: Option<&str> = None;
    for (i, n) in names.enumerate() {
        if let Some(p) = prev {
            if p >= n.as_str() {
                return Err(ModelError::NotCanonical { table, index: i });
            }
        }
        prev = Some(n);
    }
    Ok(())
}

fn check_sorted_keys<K: Ord, I>(
    table: &'static str,
    keys: I,
) -> Result<(), ModelError>
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
