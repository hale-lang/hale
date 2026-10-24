use hale_frontend::snapshot::{Config, Snapshot};
use hale_model::{FlowClass, FlowOwner, LocusDeclId, Provenance};
use hale_types::lifecycle::{ObligationKind as K, PathGuard, Spine};

#[path = "support/entries.rs"]
mod entries;

const DECLS: &str = r#"
locus Kid { dissolve() { println("kid gone"); } }
locus Other { }
locus Manager<T> { accept(k: T) { } release(k: T) { } }
"#;

fn snapshot(source: &str) -> Snapshot {
    let program = hale_syntax::parse_source(source).expect("parse");
    Snapshot::from_program(program, Vec::new(), Config::check(false, false))
        .unwrap_or_else(|_| panic!("load"))
}

#[test]
fn generic_release_classifies_the_plan_and_is_demanded_once() {
    let s = snapshot(&format!(
        "{DECLS} fn main() {{ let m: Manager<Kid> = Manager {{ }}; Kid {{ }}; }}"
    ));
    let flows = s.demand_flow_instances().expect("concrete flows");
    assert!(flows.is_flow("Kid"));
    assert!(!flows.is_flow("Other"));
    assert!(flows
        .sources
        .iter()
        .any(|r| r.child == "Kid" && r.specialization.as_deref() == Some("Manager_Kid")));
    let plan = s.demand_lifecycle().expect("plan");
    for kind in [K::Run, K::Drain, K::Dissolve, K::Reclaim] {
        let rows: Vec<_> = plan
            .obligations
            .iter()
            .filter(|o| {
                o.kind == kind
                    && o.guard == PathGuard::Normal
                    && o.site.as_ref().is_some_and(|s| s.decl.lowered == "Kid")
            })
            .collect();
        assert_eq!(rows.len(), 1, "{kind:?}");
        assert_eq!(
            rows[0].holder.spine,
            if kind == K::Run {
                Spine::PoolRun
            } else {
                Spine::Reclaim
            }
        );
    }
    assert!(std::ptr::eq(flows, s.demand_flow_instances().unwrap()));
    assert_eq!(s.builds()["flow_instances"], 1);
    assert_eq!(s.builds()["flows"], 1);

    let model = s.demand_model().expect("model");
    model.validate().unwrap();
    let id = |name| {
        LocusDeclId(
            model
                .entities
                .loci
                .iter()
                .position(|l| l.name == name)
                .unwrap() as u32,
        )
    };
    let selected = [id("Kid"), id("Other"), id("Manager")]
        .into_iter()
        .collect();
    let classes = hale_types::model_query::flow_classes(model, &selected);
    assert_eq!(classes.residents, vec![id("Other")]);
    assert_eq!(classes.templates, vec![id("Manager")]);
    assert_eq!(classes.flows.len(), 1);
    assert_eq!(classes.flows[0].0, id("Kid"));
    let releases = classes.flows[0].1;
    assert_eq!(releases.len(), 1);
    assert_eq!(releases[0].owner, FlowOwner::Declared(id("Manager")));
    assert_eq!(releases[0].specialization.as_deref(), Some("Manager_Kid"));
    // The model reuses the completed family rather than deriving it.
    assert_eq!(s.builds()["flow_instances"], 1);
}

#[test]
fn uninstantiated_generic_clause_does_not_classify_every_locus() {
    let s = snapshot(&format!("{DECLS} fn main() {{ Kid {{ }}; }}"));
    let flows = s.demand_flow_instances().unwrap();
    assert!(!flows.is_flow("Kid") && !flows.is_flow("Other"));
    assert!(!s
        .demand_lifecycle()
        .unwrap()
        .obligations
        .iter()
        .any(|o| o.kind == K::Run && o.site.as_ref().is_some_and(|s| s.decl.lowered == "Kid")));
}

#[test]
fn typed_generic_function_requests_are_included() {
    let s = snapshot(&format!("{DECLS} fn install<T>(value: T) {{ let m: Manager<T> = Manager {{ }}; }} fn main() {{ install(Kid {{ }}); }}"));
    let diags = &s.demand_check().unwrap().diags;
    assert!(diags.is_empty(), "{diags:?}");
    let flows = s.demand_flow_instances().unwrap();
    assert!(flows.is_flow("Kid"), "{:?}", flows.instantiations);
}

#[test]
fn nested_specializations_close_the_inventory_and_resolve_aliases() {
    let s = snapshot(&format!("{DECLS} type Job = Kid; locus Outer<T> {{ params {{ value: T; }} fn make() {{ let m: Manager<T> = Manager {{ }}; }} }} fn main() {{ let o: Outer<Job> = Outer {{ value: Kid {{ }} }}; }}"));
    let flows = s.demand_flow_instances().unwrap();
    let diags = &s.demand_check().unwrap().diags;
    assert!(diags.is_empty(), "{diags:?}");
    assert!(flows.is_flow("Kid"), "{:?}", flows.instantiations);
    assert!(flows
        .instantiations
        .iter()
        .any(|i| i.template == "Manager" && i.locus));
}

#[test]
fn optional_stdlib_clauses_are_visible_in_the_seed_identity_universe() {
    let s = snapshot("fn main() { std::http::Server { }; }");
    let flows = s.demand_flow_instances().unwrap();
    assert!(flows.is_flow("__StdIoTcpConnFlow"));
    assert!(flows.sources.iter().any(|r| r.child == "__StdIoTcpConnFlow"
        && r.universe == hale_types::placement::SiteUniverse::User));
}

#[test]
fn removing_the_release_clause_removes_the_flow_fact() {
    let source = format!("{DECLS} fn main() {{ let m: Manager<Kid> = Manager {{ }}; Kid {{ }}; }}");
    let s = snapshot(&source.replace("release(k: T) { }", ""));
    assert!(!s.demand_flow_instances().unwrap().is_flow("Kid"));
    let model = s.demand_model().unwrap();
    assert_eq!(
        model
            .entities
            .loci
            .iter()
            .find(|l| l.name == "Kid")
            .unwrap()
            .flow,
        FlowClass::Resident
    );
}

#[test]
fn model_flow_witnesses_keep_the_release_file_and_template_identity() {
    let child = "locus Kid { } locus Unused { }";
    let owners = "locus Left<T> { accept(k: T) { } release(k: T) { } } locus Right { accept(k: Kid) { } release(k: Kid) { } }";
    let main = "fn main() { let m: Left<Kid> = Left { }; Kid { }; }";
    let s = entries::load_files(
        &[
            ("child.hl", child),
            ("owners.hl", owners),
            ("main.hl", main),
        ],
        Config::check(true, false),
    );
    let model = s.demand_model().unwrap();
    model.validate().unwrap();
    let kid = model
        .entities
        .loci
        .iter()
        .find(|l| l.name == "Kid")
        .unwrap();
    let FlowClass::Flow { releases } = &kid.flow else {
        panic!("Kid must be a flow")
    };
    assert_eq!(
        releases.len(),
        2,
        "both the specialized and unused concrete owner classify Kid"
    );
    for release in releases {
        let FlowOwner::Declared(owner) = release.owner else {
            panic!("user owner must be declared")
        };
        let owner = &model.entities.loci[owner.index()];
        assert!(matches!(owner.name.as_str(), "Left" | "Right"));
        assert_eq!(
            release.specialization.as_deref(),
            (owner.name == "Left").then_some("Left_Kid")
        );
        let Provenance::Source { source, span } =
            model.provenance.records[release.provenance.index()]
        else {
            panic!("a loaded user release must keep source-backed provenance")
        };
        assert!(model.provenance.sources[source.index()]
            .path
            .ends_with("owners.hl"));
        assert!(owners[span.0 as usize..span.1 as usize].contains("release"));
    }
    let left = model
        .entities
        .loci
        .iter()
        .find(|l| l.name == "Left")
        .unwrap();
    assert_eq!(left.flow, FlowClass::GenericTemplate);
    assert_eq!(
        model
            .entities
            .loci
            .iter()
            .find(|l| l.name == "Unused")
            .unwrap()
            .flow,
        FlowClass::Resident
    );
    // Classification includes an uninstantiated owner. It is not a
    // statement that any arranged owner accepts a particular child.
    assert!(!model
        .entities
        .locus_instances
        .iter()
        .any(|i| model.entities.loci[i.decl.index()].name == "Right"));

    // Mutation control at the canonical input seam: the same numeric
    // offsets and owner spelling in the stdlib analysis copy must not
    // alias the application's source or declaration universe.
    let mut flows = s.demand_flow_instances().unwrap().clone();
    for source in &mut flows.sources {
        source.universe = hale_types::placement::SiteUniverse::StdlibAnalysis;
    }
    let inputs = hale_types::model_builder::ModelInputs {
        lifecycle_contexts: s.demand_lifecycle_contexts().unwrap_or_else(|_| panic!("source contexts")),
        top: s.demand_scope().unwrap(),
        bus_graph: s.demand_bus_graph().unwrap(),
        ownership: s.demand_ownership_graph().unwrap(),
        handlers: s.demand_handlers().unwrap(),
        typed: s.demand_typed_bodies().unwrap(),
        stdlib_recoveries: hale_types::stdlib_bodies::recovery_rows(),
        flows: &flows,
        effects: s.demand_effects().unwrap(),
        forms: s.demand_forms().unwrap(),
        bindings: s.demand_bindings().unwrap(),
        placement: s.demand_placement().unwrap(),
        arrangement: s.demand_arrangement().unwrap(),
        dispatch_plan: s.demand_dispatch_plan().unwrap(),
        surfaces: s.demand_surface_rows().unwrap(),
    };
    let foreign_model =
        hale_types::model_builder::derive_application_model_over(&s.bundle(), &inputs);
    foreign_model.validate().unwrap();
    let kid = foreign_model
        .entities
        .loci
        .iter()
        .find(|l| l.name == "Kid")
        .unwrap();
    let FlowClass::Flow { releases } = &kid.flow else {
        panic!("Kid must stay a flow")
    };
    assert_eq!(releases.len(), 2);
    for release in releases {
        assert!(matches!(release.owner, FlowOwner::External(_)));
        assert!(matches!(
            foreign_model.provenance.records[release.provenance.index()],
            Provenance::ForeignSpan { .. }
        ));
    }
}
