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
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
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
                    on_unmatched: KeyOnUnmatched::Swallow,
                }),
                bound: Some(TopicBound {
                    capacity: 64,
                    on_full: TopicOnFull::Fail,
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
            groups: vec![],
            types: vec![],
            interfaces: vec![],
            declarations: vec![],
        },
        relations: Relations {
            realizes: vec![Realizes {
                instance: LocusInstanceId(0),
                decl: LocusDeclId(1),
                provenance: p,
            }],
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
                subject: SubjectId(0),
                declared_topic: Some(TopicId(0)),
                payload: PayloadContractId(0),
                site: 0,
                key_domain: Some(KeyDomain::Exact(vec![KeyValue::Int(1)])),
                disposition: PublishDisposition::Default,
                provenance: p,
            }],
            subscribes: vec![Subscribe {
                subject: SubjectId(0),
                declared_topic: Some(TopicId(0)),
                payload: PayloadContractId(0),
                handler: FunctionId(1),
                site: 0,
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

// -----------------------------------------------------------------
// Review round 1 — the laws validate() previously stated but did
// not enforce.
// -----------------------------------------------------------------

/// The review's exact counterexample: a duplicated relation row
/// near the END of the schema must be rejected, so a newly added
/// table cannot accidentally miss the canonical law.
#[test]
fn duplicate_publishes_are_not_canonical() {
    let mut m = tiny_model();
    m.relations.publishes.push(m.relations.publishes[0].clone());
    assert_eq!(
        m.validate(),
        Err(ModelError::NotCanonical {
            table: "publishes",
            index: 1
        })
    );
}

#[test]
fn unsorted_supervises_are_not_canonical() {
    let mut m = tiny_model();
    let p = ProvenanceId(0);
    m.relations.supervises = vec![
        Supervises {
            parent: LocusDeclId(1),
            child: LocusDeclId(0),
            error_type: "IoError".to_string(),
            policy: SupervisionPolicy {
                ops: vec!["restart".to_string()],
                retry_bound: Some(3),
            },
            provenance: p,
        },
        Supervises {
            parent: LocusDeclId(0),
            child: LocusDeclId(1),
            error_type: "IoError".to_string(),
            policy: SupervisionPolicy {
                ops: vec!["restart".to_string()],
                retry_bound: None,
            },
            provenance: p,
        },
    ];
    assert_eq!(
        m.validate(),
        Err(ModelError::NotCanonical {
            table: "supervises",
            index: 1
        })
    );
}

#[test]
fn nested_key_sets_must_be_canonical() {
    let mut m = tiny_model();
    m.relations.publishes[0].key_domain =
        Some(KeyDomain::Exact(vec![KeyValue::Int(2), KeyValue::Int(1)]));
    assert_eq!(
        m.validate(),
        Err(ModelError::NotCanonical {
            table: "publishes.key_domain",
            index: 0
        })
    );
}

/// The review's second counterexample: an inline Unknown may not
/// hide inside an otherwise resolved row — it needs typed residue,
/// and the residue then drives the capability law.
#[test]
fn inline_unknown_key_domain_requires_a_hole() {
    let mut m = tiny_model();
    m.capabilities.exact_key_filters = true;
    m.relations.publishes[0].key_domain = Some(KeyDomain::Unknown);
    // No hole at all: the unknown is unrepresented.
    assert_eq!(
        m.validate(),
        Err(ModelError::UnrepresentedUnknown {
            table: "publishes",
            index: 0
        })
    );
    // With the hole, the capability contradiction fires instead.
    m.holes.push(Hole {
        at: EntityRef::Function(FunctionId(0)),
        kind: HoleKind::UnknownKeyDomain,
        hides: RelationSet::KEY_FILTERS,
        reason: "computed shard key".to_string(),
        provenance: ProvenanceId(0),
    });
    assert_eq!(
        m.validate(),
        Err(ModelError::CapabilityContradiction {
            capability: "exact_key_filters"
        })
    );
    // Withdrawing the claim makes the honest value lawful.
    m.capabilities.exact_key_filters = false;
    m.validate().expect("unknown + hole + no claim is lawful");
}

#[test]
fn zero_bounds_are_invalid() {
    let mut m = tiny_model();
    m.relations.subscribes[0].capacity = Capacity::Bounded(0);
    assert_eq!(
        m.validate(),
        Err(ModelError::InvalidBound {
            table: "subscribes",
            index: 0
        })
    );
    let mut m = tiny_model();
    m.entities.topics[0].bound = Some(TopicBound {
        capacity: 0,
        on_full: TopicOnFull::Fail,
    });
    assert_eq!(
        m.validate(),
        Err(ModelError::InvalidBound {
            table: "topics",
            index: 0
        })
    );
}

/// One fact, two access paths: the instance's `decl` field and its
/// `realizes` row must exist (totality), be unique, and agree.
#[test]
fn realizes_must_be_total_unique_and_agree() {
    // Missing row.
    let mut m = tiny_model();
    m.relations.realizes.clear();
    assert_eq!(
        m.validate(),
        Err(ModelError::RealizesIncomplete { instance: 0 })
    );
    // Disagreeing row (instance says Worker, relation says App).
    let mut m = tiny_model();
    m.relations.realizes[0].decl = LocusDeclId(0);
    assert_eq!(
        m.validate(),
        Err(ModelError::RealizesDisagrees { index: 0 })
    );
    // Duplicate rows: not unique (tripped as incomplete-≠-1).
    let mut m = tiny_model();
    let mut dup = m.relations.realizes[0].clone();
    dup.decl = LocusDeclId(0);
    m.relations.realizes.insert(0, dup);
    assert!(m.validate().is_err());
}

#[test]
fn binds_subject_agreement_is_checked() {
    let mut m = tiny_model();
    let p = ProvenanceId(0);
    // A second subject the binding points at while the topic keeps
    // the first — the repeated fact drifted.
    m.entities.subjects.push(Subject {
        pattern: "z.other".to_string(),
        exact: true,
        provenance: p,
    });
    m.entities.bindings.push(Binding {
        subject: SubjectId(1),
        transport: TransportKind::Unix,
        role: BindingRole::Listen,
        loss: BindingLossBehavior::Fail,
        provenance: p,
    });
    m.relations.binds.push(TopicBinding {
        topic: TopicId(0),
        binding: BindingId(0),
        provenance: p,
    });
    assert_eq!(
        m.validate(),
        Err(ModelError::BindingSubjectDisagrees { index: 0 })
    );
    // Agreeing subjects are lawful.
    m.entities.bindings[0].subject = SubjectId(0);
    m.validate().expect("agreeing binding subject is lawful");
}

#[test]
fn provenance_record_contents_must_resolve() {
    // Dangling SourceId inside an otherwise-indexed record.
    let mut m = tiny_model();
    m.provenance.records[0] = Provenance::Source {
        source: SourceId(999),
        span: (0, 10),
    };
    assert_eq!(
        m.validate(),
        Err(ModelError::InvalidProvenanceRecord { index: 0 })
    );
    // Inverted span.
    let mut m = tiny_model();
    m.provenance.records[0] = Provenance::Source {
        source: SourceId(0),
        span: (10, 4),
    };
    assert_eq!(
        m.validate(),
        Err(ModelError::InvalidProvenanceRecord { index: 0 })
    );
}

/// Every capability flag participates in the contradiction law — an
/// unmapped flag would be unfalsifiable.
#[test]
fn every_capability_flag_is_mapped_to_a_family() {
    let all = Capabilities {
        exact_calls: true,
        exact_bus_endpoints: true,
        exact_key_filters: true,
        exact_ownership: true,
        exact_placement: true,
        exact_routes: true,
        exact_effects: true,
        exact_cardinality: true,
        exact_delivery_guarantees: true,
    };
    let vouched = all.vouched_families();
    assert_eq!(vouched.len(), 9, "every flag appears exactly once");
    for (name, claimed, family) in vouched {
        assert!(claimed, "{} must carry its flag", name);
        assert!(!family.is_empty(), "{} must vouch a real family", name);
    }
}

// -----------------------------------------------------------------
// Review round 2 — site grain, supervision error types, and the
// full routing contract.
// -----------------------------------------------------------------

/// The review's motivating program: one function publishing one
/// topic twice with different dispositions
/// (`Orders <- a or wait; Orders <- b or discard;`) — two rows at
/// site grain, each keeping its own disposition and provenance.
#[test]
fn two_publish_sites_on_one_topic_are_representable() {
    let mut m = tiny_model();
    m.relations.publishes.push(Publish {
        function: FunctionId(0),
        subject: SubjectId(0),
        declared_topic: Some(TopicId(0)),
        payload: PayloadContractId(0),
        site: 1,
        key_domain: Some(KeyDomain::Exact(vec![KeyValue::Int(2)])),
        disposition: PublishDisposition::Wait,
        provenance: ProvenanceId(0),
    });
    m.validate()
        .expect("two sites with different dispositions are lawful");
    // The SAME site twice is still a duplicate.
    m.relations.publishes[1].site = 0;
    m.relations.publishes[1].key_domain = Some(KeyDomain::Exact(vec![KeyValue::Int(1)]));
    m.relations.publishes[1].disposition = PublishDisposition::Default;
    assert_eq!(
        m.validate(),
        Err(ModelError::NotCanonical {
            table: "publishes",
            index: 1
        })
    );
}

/// Two call sites sharing endpoints with different loop facts are
/// two rows; the endpoint merge is a projection concern, never a
/// schema collapse.
#[test]
fn call_sites_keep_their_own_loop_facts() {
    let mut m = tiny_model();
    let p = ProvenanceId(0);
    m.relations.calls = vec![
        Call {
            from: FunctionId(0),
            to: FunctionId(1),
            dispatch: DispatchKind::Direct,
            site: 0,
            in_loop: false,
            unbounded: false,
            provenance: p,
        },
        Call {
            from: FunctionId(0),
            to: FunctionId(1),
            dispatch: DispatchKind::Direct,
            site: 1,
            in_loop: true,
            unbounded: true,
            provenance: p,
        },
    ];
    m.validate().expect("site-grained call rows are lawful");
}

/// Two on_failure handlers for the same child, different error
/// types: distinct policies, both representable (the schema-1.10
/// supervision section is per-handler).
#[test]
fn supervision_is_per_error_type() {
    let mut m = tiny_model();
    let p = ProvenanceId(0);
    m.relations.supervises = vec![
        Supervises {
            parent: LocusDeclId(0),
            child: LocusDeclId(1),
            error_type: "ClosureViolation".to_string(),
            policy: SupervisionPolicy {
                ops: vec!["restart".to_string()],
                retry_bound: Some(3),
            },
            provenance: p,
        },
        Supervises {
            parent: LocusDeclId(0),
            child: LocusDeclId(1),
            error_type: "IoError".to_string(),
            policy: SupervisionPolicy {
                ops: vec!["replace".to_string()],
                retry_bound: None,
            },
            provenance: p,
        },
    ];
    m.validate().expect("per-error-type supervision is lawful");
    // Same (parent, child, error_type) twice is a duplicate.
    m.relations.supervises[1].error_type = "ClosureViolation".to_string();
    assert_eq!(
        m.validate(),
        Err(ModelError::NotCanonical {
            table: "supervises",
            index: 1
        })
    );
}

/// The fallback contract, both directions: `where key == _` is
/// legal only on `on_unmatched: fallback` topics, and a fallback
/// topic must have its catch.
#[test]
fn fallback_contract_is_validated_both_ways() {
    // `_` on a swallow topic: illegal.
    let mut m = tiny_model();
    m.relations.subscribes[0].key_predicate = KeyPredicate::Fallback;
    assert_eq!(m.validate(), Err(ModelError::IllegalFallback { index: 0 }));

    // Fallback topic without a catch: uncovered.
    let mut m = tiny_model();
    m.entities.topics[0].key = Some(TopicKey {
        field: "sensor".to_string(),
        on_unmatched: KeyOnUnmatched::Fallback,
    });
    assert_eq!(
        m.validate(),
        Err(ModelError::FallbackUncovered { topic: 0 })
    );

    // Properly paired: lawful.
    m.relations.subscribes[0].key_predicate = KeyPredicate::Fallback;
    m.validate().expect("fallback topic + catch is lawful");
}

/// Every shipped key-eligible type has a KeyValue variant, and
/// mixed-variant canonical sets stay ordered.
#[test]
fn key_values_cover_the_shipped_routing_types() {
    let mut m = tiny_model();
    m.relations.publishes[0].key_domain = Some(KeyDomain::Exact(vec![
        KeyValue::Bool(true),
        KeyValue::Int(3),
        KeyValue::Time(1_000),
        KeyValue::Duration(5_000),
        KeyValue::EnumTag("Buy".to_string()),
        KeyValue::Decimal { lo: 1, hi: 2 },
        KeyValue::Str("sym".to_string()),
    ]));
    m.validate().expect("all key-type variants are usable");
}

// -----------------------------------------------------------------
// Review round 3 — groups and the unkeyed key contract.
// -----------------------------------------------------------------

/// Groups are model rows (claims resolve selectors through them and
/// the artifact shape-hashes them) — with membership as a typed
/// relation over EntityRefs, covering loci AND free fns.
#[test]
fn groups_are_typed_model_rows() {
    let mut m = tiny_model();
    let p = ProvenanceId(0);
    m.entities.groups = vec![
        Group {
            name: "probes".to_string(),
            may_be_empty: true,
            provenance: p,
        },
        Group {
            name: "workers".to_string(),
            may_be_empty: false,
            provenance: p,
        },
    ];
    m.relations.group_members = vec![
        GroupMember {
            group: GroupId(1),
            member: EntityRef::LocusDecl(LocusDeclId(1)),
            provenance: p,
        },
        GroupMember {
            group: GroupId(1),
            member: EntityRef::Function(FunctionId(0)),
            provenance: p,
        },
    ];
    // Members sort by (group, member); the rows above are reversed.
    assert_eq!(
        m.validate(),
        Err(ModelError::NotCanonical {
            table: "group_members",
            index: 1
        })
    );
    m.relations.group_members.swap(0, 1);
    m.validate().expect("sorted group membership is lawful");

    // Dangling group id refused (perturb the LAST row so the table
    // stays sorted and the reference check is what fires).
    m.relations.group_members[1].group = GroupId(9);
    assert_eq!(
        m.validate(),
        Err(ModelError::DanglingId {
            table: "group_members",
            index: 1
        })
    );
}

/// The unkeyed contract, all four directions: an unkeyed publish
/// carries NO key domain; a keyed publish carries one; an unkeyed
/// topic admits only the plain subscription; keyed predicates need
/// keyed topics.
#[test]
fn keyedness_must_match_the_topic_both_ways() {
    // Keyed topic (tiny_model default), publish without a domain.
    let mut m = tiny_model();
    m.relations.publishes[0].key_domain = None;
    assert_eq!(
        m.validate(),
        Err(ModelError::KeyContract {
            table: "publishes",
            index: 0
        })
    );

    // Unkeyed topic, publish WITH a domain: inventing a meaning.
    let mut m = tiny_model();
    m.entities.topics[0].key = None;
    m.relations.subscribes[0].key_predicate = KeyPredicate::Any;
    assert_eq!(
        m.validate(),
        Err(ModelError::KeyContract {
            table: "publishes",
            index: 0
        })
    );

    // Fully unkeyed: publish None + Any subscription is lawful.
    m.relations.publishes[0].key_domain = None;
    m.validate().expect("unkeyed topic, unkeyed rows: lawful");

    // A keyed predicate on the unkeyed topic is refused.
    m.relations.subscribes[0].key_predicate = KeyPredicate::EqLiteral(KeyValue::Int(1));
    assert_eq!(
        m.validate(),
        Err(ModelError::KeyContract {
            table: "subscribes",
            index: 0
        })
    );
}

// -----------------------------------------------------------------
// Review round 4 — dead dispatches, literal/wildcard endpoints, and
// the full declaration universe.
// -----------------------------------------------------------------

/// A call through an uninhabited interface is a DEAD site, not a
/// hole: representable without hiding CALLS, so `exact_calls` stays
/// claimable — precisely the artifact's closed-world rule.
#[test]
fn dead_interface_calls_are_not_holes() {
    let mut m = tiny_model();
    m.relations.dead_interface_calls = vec![DeadInterfaceCall {
        from: FunctionId(1),
        site: 0,
        interface: "Notifier".to_string(),
        method: "notify".to_string(),
        provenance: ProvenanceId(0),
    }];
    // exact_calls is TRUE in tiny_model — the dead site must not
    // contradict it (it is not residue).
    assert!(m.capabilities.exact_calls);
    m.validate()
        .expect("a dead dispatch coexists with exact_calls");
    // Same (from, site) twice is a duplicate.
    m.relations
        .dead_interface_calls
        .push(m.relations.dead_interface_calls[0].clone());
    assert_eq!(
        m.validate(),
        Err(ModelError::NotCanonical {
            table: "dead_interface_calls",
            index: 1
        })
    );
}

/// Literal and wildcard subjects are real endpoints WITHOUT topic
/// declarations: subject-grained rows carry them, and no fake Topic
/// enters the declared sort.
#[test]
fn literal_and_wildcard_endpoints_are_representable() {
    let mut m = tiny_model();
    let p = ProvenanceId(0);
    m.entities.subjects.push(Subject {
        pattern: "z.orders.**".to_string(),
        exact: false,
        provenance: p,
    });
    // A wildcard subscription with no declared topic.
    m.relations.subscribes.push(Subscribe {
        subject: SubjectId(1),
        declared_topic: None,
        payload: PayloadContractId(0),
        handler: FunctionId(1),
        site: 1,
        key_predicate: KeyPredicate::Any,
        capacity: Capacity::Unbounded,
        shed: ShedPolicy::None,
        provenance: p,
    });
    // A literal publish with no declared topic (unkeyed: None).
    m.relations.publishes.push(Publish {
        function: FunctionId(0),
        subject: SubjectId(1),
        declared_topic: None,
        payload: PayloadContractId(0),
        site: 1,
        key_domain: None,
        disposition: PublishDisposition::Default,
        provenance: p,
    });
    m.validate()
        .expect("undeclared endpoints are lawful subject rows");
    assert_eq!(
        m.entities.topics.len(),
        1,
        "no fake Topic was needed — the declared sort is untouched"
    );

    // A keyed predicate on an undeclared endpoint is refused.
    m.relations.subscribes[1].key_predicate = KeyPredicate::EqReplica;
    assert_eq!(
        m.validate(),
        Err(ModelError::KeyContract {
            table: "subscribes",
            index: 1
        })
    );
}

/// A declared_topic whose subject disagrees with the row's subject
/// is one endpoint with two addresses — refused.
#[test]
fn declared_topic_subject_must_agree() {
    let mut m = tiny_model();
    m.entities.subjects.push(Subject {
        pattern: "z.other".to_string(),
        exact: true,
        provenance: ProvenanceId(0),
    });
    m.relations.publishes[0].subject = SubjectId(1);
    assert_eq!(
        m.validate(),
        Err(ModelError::DeclaredTopicDisagrees {
            table: "publishes",
            index: 0
        })
    );
}

/// The declaration universe covers the seed sort: types,
/// interfaces, and groups participate in `declared_in` alongside
/// loci, fns, and topics — the full rename table, no side channel.
#[test]
fn declared_in_covers_the_full_declaration_universe() {
    let mut m = tiny_model();
    let p = ProvenanceId(0);
    m.entities.types = vec![TypeDecl {
        name: "Reading".to_string(),
        display: "Reading".to_string(),
        provenance: p,
    }];
    m.entities.interfaces = vec![InterfaceDecl {
        name: "Notifier".to_string(),
        display: "Notifier".to_string(),
        provenance: p,
    }];
    m.entities.groups = vec![Group {
        name: "workers".to_string(),
        may_be_empty: false,
        provenance: p,
    }];
    m.relations.declared_in = vec![
        DeclaredIn {
            entity: EntityRef::Function(FunctionId(1)),
            seed: SeedId(0),
            provenance: p,
        },
        DeclaredIn {
            entity: EntityRef::LocusDecl(LocusDeclId(1)),
            seed: SeedId(0),
            provenance: p,
        },
        DeclaredIn {
            entity: EntityRef::Topic(TopicId(0)),
            seed: SeedId(0),
            provenance: p,
        },
        DeclaredIn {
            entity: EntityRef::Group(GroupId(0)),
            seed: SeedId(0),
            provenance: p,
        },
        DeclaredIn {
            entity: EntityRef::Type(TypeDeclId(0)),
            seed: SeedId(0),
            provenance: p,
        },
        DeclaredIn {
            entity: EntityRef::Interface(InterfaceDeclId(0)),
            seed: SeedId(0),
            provenance: p,
        },
    ];
    m.validate()
        .expect("the full declaration universe joins the seed sort");

    // Dangling type id in an EntityRef is still refused.
    m.relations.declared_in[4].entity = EntityRef::Type(TypeDeclId(7));
    assert!(m.validate().is_err());
}

// -----------------------------------------------------------------
// Review round 5 — the COMPLETE nameable universe + endpoint
// payloads.
// -----------------------------------------------------------------

/// The declaration-universe canary, covering every variant the
/// compiler's `top_decl_name` names — not a hand-picked subset:
/// locus, fn, topic, type, interface, group (specialized sorts)
/// PLUS perspective, const, ring layout, target (opaque
/// Declaration rows). Module/Claims/Constitution are deliberately
/// nameless there and absent here. A new nameable TopDecl variant
/// must extend this test alongside the schema.
#[test]
fn the_nameable_declaration_universe_is_complete() {
    let mut m = tiny_model();
    let p = ProvenanceId(0);
    m.entities.types = vec![TypeDecl {
        name: "Reading".to_string(),
        display: "Reading".to_string(),
        provenance: p,
    }];
    m.entities.interfaces = vec![InterfaceDecl {
        name: "Notifier".to_string(),
        display: "Notifier".to_string(),
        provenance: p,
    }];
    m.entities.groups = vec![Group {
        name: "workers".to_string(),
        may_be_empty: false,
        provenance: p,
    }];
    m.entities.declarations = vec![
        Declaration {
            kind: DeclKind::Const,
            name: "MAX_DEPTH".to_string(),
            display: "MAX_DEPTH".to_string(),
            provenance: p,
        },
        Declaration {
            kind: DeclKind::RingLayout,
            name: "TickRing".to_string(),
            display: "TickRing".to_string(),
            provenance: p,
        },
        Declaration {
            kind: DeclKind::Target,
            name: "edge_node".to_string(),
            display: "edge_node".to_string(),
            provenance: p,
        },
        Declaration {
            kind: DeclKind::Perspective,
            name: "public_api".to_string(),
            display: "public_api".to_string(),
            provenance: p,
        },
    ];
    m.entities
        .declarations
        .sort_by(|a, b| (&a.name, a.kind).cmp(&(&b.name, b.kind)));
    // ALL ten nameable kinds join the seed sort.
    m.relations.declared_in = vec![
        DeclaredIn {
            entity: EntityRef::Function(FunctionId(1)),
            seed: SeedId(0),
            provenance: p,
        },
        DeclaredIn {
            entity: EntityRef::LocusDecl(LocusDeclId(1)),
            seed: SeedId(0),
            provenance: p,
        },
        DeclaredIn {
            entity: EntityRef::Topic(TopicId(0)),
            seed: SeedId(0),
            provenance: p,
        },
        DeclaredIn {
            entity: EntityRef::Group(GroupId(0)),
            seed: SeedId(0),
            provenance: p,
        },
        DeclaredIn {
            entity: EntityRef::Type(TypeDeclId(0)),
            seed: SeedId(0),
            provenance: p,
        },
        DeclaredIn {
            entity: EntityRef::Interface(InterfaceDeclId(0)),
            seed: SeedId(0),
            provenance: p,
        },
        DeclaredIn {
            entity: EntityRef::Declaration(DeclarationId(0)),
            seed: SeedId(0),
            provenance: p,
        },
        DeclaredIn {
            entity: EntityRef::Declaration(DeclarationId(1)),
            seed: SeedId(0),
            provenance: p,
        },
        DeclaredIn {
            entity: EntityRef::Declaration(DeclarationId(2)),
            seed: SeedId(0),
            provenance: p,
        },
        DeclaredIn {
            entity: EntityRef::Declaration(DeclarationId(3)),
            seed: SeedId(0),
            provenance: p,
        },
    ];
    m.validate()
        .expect("all ten nameable declaration kinds join the seed sort");
    // Kind is part of the DeclKind vocabulary — all four opaque
    // kinds are present in this model.
    let kinds: std::collections::BTreeSet<_> =
        m.entities.declarations.iter().map(|d| d.kind).collect();
    assert_eq!(
        kinds.len(),
        4,
        "perspective, const, ring layout, target all representable"
    );
    // Dangling opaque-declaration ref is refused.
    m.relations.declared_in[9].entity = EntityRef::Declaration(DeclarationId(9));
    assert!(m.validate().is_err());
}

/// Literal/wildcard endpoints keep their `of type T` payload — the
/// checked BusGraph fact — and a declared endpoint's payload must
/// agree with its topic's.
#[test]
fn endpoint_payloads_are_kept_and_must_agree() {
    // Undeclared endpoint with its own payload: lawful, payload
    // reachable straight off the row.
    let mut m = tiny_model();
    let p = ProvenanceId(0);
    m.entities.subjects.push(Subject {
        pattern: "z.orders.**".to_string(),
        exact: false,
        provenance: p,
    });
    m.entities.payloads.push(PayloadContract {
        shape: "z_op:i".to_string(),
        hash: 0x828a,
        provenance: p,
    });
    m.relations.subscribes.push(Subscribe {
        subject: SubjectId(1),
        declared_topic: None,
        payload: PayloadContractId(1),
        handler: FunctionId(1),
        site: 1,
        key_predicate: KeyPredicate::Any,
        capacity: Capacity::Unbounded,
        shed: ShedPolicy::None,
        provenance: p,
    });
    m.validate().expect("undeclared endpoint keeps its payload");

    // Declared endpoint whose payload disagrees with the topic's:
    // one endpoint, two shapes — refused.
    m.relations.publishes[0].payload = PayloadContractId(1);
    assert_eq!(
        m.validate(),
        Err(ModelError::EndpointPayloadDisagrees {
            table: "publishes",
            index: 0
        })
    );

    // Dangling payload id refused.
    m.relations.publishes[0].payload = PayloadContractId(9);
    assert_eq!(
        m.validate(),
        Err(ModelError::DanglingId {
            table: "publishes",
            index: 0
        })
    );
}

// -----------------------------------------------------------------
// Review round 6 — authored selectors alongside resolved members.
// -----------------------------------------------------------------

/// The legacy hash covers group selectors AS AUTHORED — `{ lib::* }`
/// and `{ lib::A, lib::B }` may resolve identically but must keep
/// distinct shapes. Both grains coexist: GroupSelector (authored,
/// ordered, globs unexpanded) and GroupMember (resolved).
#[test]
fn authored_selectors_are_kept_alongside_resolved_members() {
    let mut m = tiny_model();
    let p = ProvenanceId(0);
    m.entities.groups = vec![Group {
        name: "workers".to_string(),
        may_be_empty: false,
        provenance: p,
    }];
    // Resolved membership: one locus (as if lib::* enumerated it).
    m.relations.group_members = vec![GroupMember {
        group: GroupId(0),
        member: EntityRef::LocusDecl(LocusDeclId(1)),
        provenance: p,
    }];
    // Authored grain, variant A: a glob, UNEXPANDED.
    m.relations.group_selectors = vec![GroupSelector {
        group: GroupId(0),
        ordinal: 0,
        selector: SelectorForm::SeedGlob {
            seed: SeedId(0),
            display: "lib::*".to_string(),
        },
        provenance: p,
    }];
    m.validate().expect("glob selector + resolved member coexist");
    let glob_selectors = m.relations.group_selectors.clone();

    // Authored grain, variant B: the same membership spelled as a
    // named selector — the RESOLVED rows are identical, the
    // AUTHORED rows differ, which is exactly what keeps the two
    // programs' shapes distinct.
    m.relations.group_selectors = vec![GroupSelector {
        group: GroupId(0),
        ordinal: 0,
        selector: SelectorForm::Named {
            member: EntityRef::LocusDecl(LocusDeclId(1)),
            display: "lib::Worker".to_string(),
        },
        provenance: p,
    }];
    m.validate().expect("named selector variant is lawful");
    assert_ne!(
        glob_selectors, m.relations.group_selectors,
        "identical membership, distinct authored shapes"
    );

    // A zero-member glob is still a selector row: authored shape
    // survives even when resolution contributes nothing.
    m.relations.group_members.clear();
    m.entities.groups[0].may_be_empty = true;
    m.relations.group_selectors = vec![GroupSelector {
        group: GroupId(0),
        ordinal: 0,
        selector: SelectorForm::SeedGlob {
            seed: SeedId(0),
            display: "lib::*".to_string(),
        },
        provenance: p,
    }];
    m.validate().expect("zero-member glob keeps its authored row");

    // Ordinals are ordered facts; a duplicate ordinal is refused.
    m.relations.group_selectors.push(GroupSelector {
        group: GroupId(0),
        ordinal: 0,
        selector: SelectorForm::Named {
            member: EntityRef::Function(FunctionId(0)),
            display: "App::run".to_string(),
        },
        provenance: p,
    });
    assert_eq!(
        m.validate(),
        Err(ModelError::NotCanonical {
            table: "group_selectors",
            index: 1
        })
    );
    // Dangling glob seed is refused.
    m.relations.group_selectors = vec![GroupSelector {
        group: GroupId(0),
        ordinal: 0,
        selector: SelectorForm::SeedGlob {
            seed: SeedId(7),
            display: "ghost::*".to_string(),
        },
        provenance: p,
    }];
    assert_eq!(
        m.validate(),
        Err(ModelError::DanglingId {
            table: "group_selectors",
            index: 0
        })
    );
}
