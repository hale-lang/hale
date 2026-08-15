//! GH #476 Change 1 — architecture canaries for the model schema.
//!
//! These tests are the epic's review laws made executable:
//!   1. `hale-model` depends on NOTHING — in particular never on
//!      `hale-syntax` (the crate is source-independent by law) and
//!      never on serde (no external serialization promise before the
//!      Change-3 projection).
//!   2. Every row carries provenance (by construction — exercised by
//!      building a small valid model) and dangling provenance is a
//!      validation error, not a tolerated quirk.
//!   3. A hole must hide at least one relation family.
//!   4. A capability cannot claim exactness while a hole hides that
//!      family — the positive and negative completeness accounts
//!      cannot drift.
//!   5. Canonical order is a law: an unsorted table is not a model.

use hale_model::*;

// -----------------------------------------------------------------
// 1. Dependency direction
// -----------------------------------------------------------------

#[test]
fn the_model_crate_depends_on_nothing() {
    let manifest = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/Cargo.toml"
    ))
    .expect("read own manifest");
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .expect("dependencies section exists");
    // Everything after [dependencies] must be empty (comments and
    // whitespace aside) — no hale-syntax, no hale-types, no serde,
    // no anything. The law is stronger than a denylist: source
    // independence is guaranteed by having zero deps at all.
    for line in deps.lines() {
        let l = line.trim();
        assert!(
            l.is_empty() || l.starts_with('#') || l.starts_with('['),
            "hale-model must stay dependency-free; found dependency \
             line: {}",
            l
        );
    }
}

// -----------------------------------------------------------------
// A minimal valid model the law tests perturb.
// -----------------------------------------------------------------

fn tiny_model() -> ApplicationModel {
    let mut prov = ProvenanceTable::default();
    prov.sources.push(provenance_source());
    prov.records.push(Provenance::Source {
        source: SourceId(0),
        span: (0, 10),
    });
    let p = ProvenanceId(0);

    ApplicationModel {
        header: ModelHeader {
            semantics: MODEL_SEMANTICS_V1,
            entrypoint: "App".to_string(),
        },
        entities: Entities {
            functions: vec![
                Function {
                    name: "App::run".to_string(),
                    display: "App::run".to_string(),
                    kind: FunctionKind::Hook,
                    effects: vec!["publish".to_string()],
                    provenance: p,
                },
                Function {
                    name: "Worker::on_r".to_string(),
                    display: "Worker::on_r".to_string(),
                    kind: FunctionKind::Method,
                    effects: vec![],
                    provenance: p,
                },
            ],
            loci: vec![
                LocusDecl {
                    name: "App".to_string(),
                    display: "App".to_string(),
                    sealed: false,
                    provenance: p,
                },
                LocusDecl {
                    name: "Worker".to_string(),
                    display: "Worker".to_string(),
                    sealed: false,
                    provenance: p,
                },
            ],
            locus_instances: vec![LocusInstance {
                path: "App.w".to_string(),
                decl: LocusDeclId(1),
                replica: Some(0),
                provenance: p,
            }],
            topics: vec![Topic {
                name: "Readings".to_string(),
                subject: SubjectId(0),
                payload: PayloadContractId(0),
                key: Some(TopicKey {
                    field: "sensor".to_string(),
                }),
                provenance: p,
            }],
            subjects: vec![Subject {
                pattern: "sense.reading".to_string(),
                exact: true,
                provenance: p,
            }],
            payloads: vec![PayloadContract {
                shape: "sensor:i;v:i".to_string(),
                hash: 0xfb9f,
                provenance: p,
            }],
            phases: vec![Phase {
                name: "run".to_string(),
                provenance: p,
            }],
            seeds: vec![Seed {
                name: "app".to_string(),
                provenance: p,
            }],
            thread_domains: vec![ThreadDomain {
                name: "main".to_string(),
                provenance: p,
            }],
            bindings: vec![],
        },
        relations: Relations {
            member_of: vec![
                MemberOf {
                    function: FunctionId(0),
                    locus: LocusDeclId(0),
                    provenance: p,
                },
                MemberOf {
                    function: FunctionId(1),
                    locus: LocusDeclId(1),
                    provenance: p,
                },
            ],
            publishes: vec![Publish {
                function: FunctionId(0),
                topic: TopicId(0),
                key_domain: KeyDomain::Exact(vec![KeyValue::Int(1)]),
                disposition: PublishDisposition::Default,
                provenance: p,
            }],
            subscribes: vec![Subscribe {
                topic: TopicId(0),
                handler: FunctionId(1),
                key_predicate: KeyPredicate::EqReplica,
                capacity: Capacity::Bounded(16),
                shed: ShedPolicy::DropOld,
                provenance: p,
            }],
            ..Relations::default()
        },
        labels: vec![],
        weights: vec![],
        holes: vec![],
        capabilities: Capabilities {
            exact_calls: true,
            exact_bus_endpoints: true,
            ..Capabilities::default()
        },
        provenance: prov,
    }
}

fn provenance_source() -> hale_model::provenance::SourceUnit {
    hale_model::provenance::SourceUnit {
        path: "app.hl".to_string(),
        digest: 0xec85,
    }
}

#[test]
fn a_well_formed_model_validates() {
    tiny_model().validate().expect("tiny model is lawful");
}

// -----------------------------------------------------------------
// 2. Provenance cannot dangle
// -----------------------------------------------------------------

#[test]
fn dangling_provenance_is_rejected() {
    let mut m = tiny_model();
    m.entities.functions[0].provenance = ProvenanceId(99);
    assert_eq!(
        m.validate(),
        Err(ModelError::DanglingProvenance {
            table: "functions",
            index: 0
        })
    );
}

#[test]
fn dangling_entity_ids_are_rejected() {
    let mut m = tiny_model();
    m.relations.subscribes[0].handler = FunctionId(41);
    assert_eq!(
        m.validate(),
        Err(ModelError::DanglingId {
            table: "subscribes",
            index: 0
        })
    );
}

// -----------------------------------------------------------------
// 3. A hole must hide something
// -----------------------------------------------------------------

#[test]
fn an_empty_hole_is_not_a_hole() {
    let mut m = tiny_model();
    m.holes.push(Hole {
        at: EntityRef::Function(FunctionId(1)),
        kind: HoleKind::IndirectCall,
        hides: RelationSet(0),
        reason: "hides nothing".to_string(),
        provenance: ProvenanceId(0),
    });
    assert_eq!(m.validate(), Err(ModelError::EmptyHole { index: 0 }));
}

// -----------------------------------------------------------------
// 4. Capabilities cannot contradict holes
// -----------------------------------------------------------------

#[test]
fn a_capability_cannot_claim_exactness_over_a_hole() {
    let mut m = tiny_model();
    // The model claims exact_calls, then grows a hole hiding CALLS:
    // the two completeness accounts drifted — refuse the value.
    m.holes.push(Hole {
        at: EntityRef::Function(FunctionId(1)),
        kind: HoleKind::IndirectCall,
        hides: RelationSet::CALLS,
        reason: "call through fn param `f`".to_string(),
        provenance: ProvenanceId(0),
    });
    assert_eq!(
        m.validate(),
        Err(ModelError::CapabilityContradiction {
            capability: "exact_calls"
        })
    );
    // Withdrawing the claim makes the same value lawful — holes and
    // capabilities agree that calls are inexact.
    m.capabilities.exact_calls = false;
    m.validate().expect("hole + withdrawn capability is lawful");
}

// -----------------------------------------------------------------
// 5. Canonical order is a law
// -----------------------------------------------------------------

#[test]
fn unsorted_tables_are_not_models() {
    let mut m = tiny_model();
    m.entities.functions.swap(0, 1);
    // Fix up the rows that index functions so ONLY order is wrong.
    m.relations.member_of[0].function = FunctionId(1);
    m.relations.member_of[1].function = FunctionId(0);
    m.relations.member_of.swap(0, 1);
    m.relations.publishes[0].function = FunctionId(1);
    m.relations.subscribes[0].handler = FunctionId(0);
    assert_eq!(
        m.validate(),
        Err(ModelError::NotCanonical {
            table: "functions",
            index: 1
        })
    );
}

#[test]
fn duplicate_rows_are_not_canonical() {
    let mut m = tiny_model();
    let dup = m.relations.member_of[0].clone();
    m.relations.member_of.insert(1, dup);
    assert_eq!(
        m.validate(),
        Err(ModelError::NotCanonical {
            table: "member_of",
            index: 1
        })
    );
}
