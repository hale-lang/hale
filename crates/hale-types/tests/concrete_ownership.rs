//! Generic ownership is substituted from source rows and the shared request
//! inventory, before LLVM declarations or layouts exist.
#[path = "support/entries.rs"]
mod entries;
use hale_frontend::snapshot::{Config, Snapshot};

const SOURCE: &str = r#"
locus Child { }
locus Maker { fn make() { Child { }; } }
locus Owner<T> {
    params { maker: Maker = Maker { }; }
    accept(c: T) { }
}
fn main() { let owner: Owner<Child> = Owner { }; owner.maker.make(); }
"#;
fn snapshot(source: &str) -> Snapshot {
    entries::load_files(&[("main.hl", source)], Config::check(true, false))
}
#[test]
fn type_parameter_and_nominal_accepts_both_name_the_actual_emitted_owner() {
    for source in [
        SOURCE.to_string(),
        SOURCE.replace("accept(c: T)", "accept(c: Child)"),
    ] {
        let s = snapshot(&source);
        let checked = s.demand_check().unwrap();
        assert!(
            !checked.diags.iter().any(|d| d.is_error()),
            "{:?}",
            checked.diags
        );
        let view = s.demand_lowering().unwrap();
        let key = ("Maker".to_string(), "Child".to_string());
        assert_eq!(view.bubble.nonsingleton[&key], "Owner_Child");
        assert!(view.concrete_accepts["Owner_Child"].contains("Child"));
        assert!(!view.concrete_accepts.contains_key("Owner"));
        assert!(view.bubble.forwarding["Maker"].contains("Child"));
        assert_eq!(s.builds()["flow_instances"], 1);
        assert_eq!(s.builds()["ownership"], 1);
    }
}

#[test]
fn nested_generic_defaults_forward_through_their_concrete_intermediary() {
    let source = SOURCE
        .replace(
            "locus Owner<T>",
            "locus Middle<T> { params { maker: Maker = Maker { }; } }\nlocus Owner<T>",
        )
        .replace(
            "maker: Maker = Maker { }; }\n    accept",
            "middle: Middle<T> = Middle { }; }\n    accept",
        )
        .replace("owner.maker.make();", "owner.middle.maker.make();");
    let s = snapshot(&source);
    let checked = s.demand_check().unwrap();
    assert!(
        !checked.diags.iter().any(|d| d.is_error()),
        "{:?}",
        checked.diags
    );
    let view = s.demand_lowering().unwrap();
    assert_eq!(
        view.bubble.nonsingleton[&("Maker".into(), "Child".into())],
        "Owner_Child"
    );
    for name in ["Maker", "Middle_Child"] {
        assert!(view.bubble.forwarding[name].contains("Child"), "{name}");
    }
    assert!(!view.bubble.forwarding.contains_key("Middle"));
}

#[test]
fn unused_generic_templates_cannot_be_runtime_owners() {
    let source = SOURCE.replace("accept(c: T)", "accept(c: Child)").replace(
        "let owner: Owner<Child> = Owner { }; owner.maker.make();",
        "let maker = Maker { }; maker.make();",
    );
    let s = snapshot(&source);
    let view = s.demand_lowering().unwrap();
    assert!(!view.concrete_accepts.contains_key("Owner"));
    assert!(!view
        .bubble
        .nonsingleton
        .contains_key(&("Maker".into(), "Child".into())));
}

#[test]
fn owned_and_free_births_of_one_producer_retain_a_nullable_owner_choice() {
    let source = SOURCE.replace(
        "owner.maker.make();",
        "owner.maker.make(); let outside = Maker { }; outside.make();",
    );
    let s = snapshot(&source);
    let view = s.demand_lowering().unwrap();
    let key = ("Maker".to_string(), "Child".to_string());
    let choices = &view.bubble.alternatives[&key];
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].owner, "Owner_Child");
    assert_eq!(
        choices[0].class,
        hale_types::ownership_graph::EdgeClass::SameTower
    );
    assert!(view.bubble.alternative_forwarding["Maker"]
        .contains(&("Child".into(), "Owner_Child".into())));
    assert!(!view.bubble.nonsingleton.contains_key(&key));
}

#[test]
fn different_specializations_keep_distinct_owner_candidates() {
    let source = SOURCE
        .replace("accept(c: T)", "accept(c: Child)")
        .replace("locus Child { }", "locus Child { } locus Other { }")
        .replace(
            "owner.maker.make();",
            "owner.maker.make(); let second: Owner<Other> = Owner { }; second.maker.make();",
        );
    let s = snapshot(&source);
    let view = s.demand_lowering().unwrap();
    let choices = &view.bubble.alternatives[&("Maker".into(), "Child".into())];
    assert_eq!(
        choices.iter().map(|c| c.owner.as_str()).collect::<Vec<_>>(),
        ["Owner_Child", "Owner_Other"]
    );
    for owner in ["Owner_Child", "Owner_Other"] {
        assert!(view.concrete_accepts[owner].contains("Child"));
        assert!(
            view.bubble.alternative_forwarding["Maker"].contains(&("Child".into(), owner.into()))
        );
    }
}

const CROSS_POOL: &str = r#"
locus Child { params { n: Int = 1; } birth() { println("child ",self.n); } dissolve() { println("child gone"); } }
locus Maker { run() { Child { }; } }
locus Owner { params { maker: Maker = Maker { }; } accept(c: Child) { println("owner"); } }
main locus App { params { owner: Owner = Owner { }; maker: Maker = Maker { }; seen: Int = 0; } placement { maker: cooperative(pool = workers); } accept(c: Child) { self.seen = self.seen + 1; println("app"); } run() { let mut waited = 0; while self.seen < 1 && waited < 100 { std::time::sleep(50ms); waited = waited + 1; } println("completed ",self.seen); } }
fn main() { App { }; }
"#;

#[test]
fn alternative_domains_are_correlated_with_the_nearest_actual_acceptor() {
    use hale_types::ownership_graph::EdgeClass;
    let s = snapshot(CROSS_POOL);
    let checked = s.demand_check().unwrap();
    assert!(
        !checked.diags.iter().any(|d| d.is_error()),
        "{:?}",
        checked.diags
    );
    let view = s.demand_lowering().unwrap();
    let routes = &view.bubble.alternatives[&("Maker".into(), "Child".into())];
    assert_eq!(routes.len(), 2);
    let app = routes.iter().find(|r| r.owner == "App").unwrap();
    assert_eq!(app.class, EdgeClass::CrossPool);
    assert!(app.singleton);
    let owner = routes.iter().find(|r| r.owner == "Owner").unwrap();
    assert_eq!(owner.class, EdgeClass::SameTower);
    assert!(!owner.singleton);
    let defaults = s.demand_default_evaluations().unwrap();
    assert!(!defaults
        .posted
        .contains_key(&("Maker".into(), "Child".into())));
    assert_eq!(
        defaults.possible_posts[&("Maker".into(), "Child".into())],
        "App"
    );
}

#[test]
fn a_possible_owner_post_refuses_a_value_without_erasing_local_evaluation() {
    let source = CROSS_POOL.replace("run() { Child { }; }", "run() { let child = Child { }; }");
    let s = snapshot(&source);
    let checked = s.demand_check().unwrap();
    let errors: Vec<_> = checked.diags.iter().filter(|d| d.is_error()).collect();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].message.contains("fire-and-forget"), "{errors:?}");
    assert!(s.demand_lowering().is_err());
    assert!(s.demand_default_evaluations().unwrap().posted.is_empty());
}

#[test]
fn typed_generic_owner_defaults_admit_only_their_specialized_field_types() {
    let source = CROSS_POOL
        .replace(
            "locus Owner { params {",
            "locus Owner<T> { params { padding: T;",
        )
        .replace(
            "owner: Owner = Owner { }",
            "owner: Owner<Int> = Owner { padding: 7 }",
        );
    let s = snapshot(&source);
    let checked = s.demand_check().unwrap();
    assert!(
        !checked.diags.iter().any(|d| d.is_error()),
        "{:?}",
        checked.diags
    );
    let view = s.demand_lowering().unwrap();
    let routes = &view.bubble.alternatives[&("Maker".into(), "Child".into())];
    assert_eq!(
        routes
            .iter()
            .find(|r| r.owner == "Owner_Int")
            .unwrap()
            .class,
        hale_types::ownership_graph::EdgeClass::SameTower
    );
    let bad = source.replace("padding: 7", "padding: \"wrong\"");
    let s = snapshot(&bad);
    assert!(
        s.demand_check().unwrap().diags.iter().any(|d| d.is_error() && d.message.contains("padding")),
        "the Int specialization must reject a String field"
    );
}
