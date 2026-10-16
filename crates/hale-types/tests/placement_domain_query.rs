//! Instance-domain queries consume the snapshot's arrangement without
//! discarding concrete witnesses, missing placement, or dynamic births.

#[path = "support/entries.rs"]
mod entries;

use std::collections::BTreeSet;

use hale_frontend::snapshot::Config;
use hale_model::{ApplicationModel, LocusDeclId};
use hale_types::model_query::{instance_domains, InstanceDomains};

fn model(source: &str) -> ApplicationModel {
    let snapshot = entries::load_files(&[("main.hl", source)], Config::check(true, false));
    let diagnostics = entries::checked(&snapshot);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let model = snapshot.demand_model().expect("checked model").clone();
    assert_eq!(model.validate(), Ok(()));
    model
}

fn declarations(model: &ApplicationModel, names: &[&str]) -> BTreeSet<LocusDeclId> {
    names
        .iter()
        .map(|name| {
            LocusDeclId(
                model
                    .entities
                    .loci
                    .iter()
                    .position(|locus| locus.name == *name)
                    .unwrap_or_else(|| panic!("no declaration named {name}"))
                    as u32,
            )
        })
        .collect()
}

fn placed(model: &ApplicationModel, projection: &InstanceDomains<'_>) -> Vec<String> {
    projection
        .placed
        .iter()
        .map(|row| {
            assert_eq!(row.realization.instance, row.placement.instance);
            format!(
                "{} {}",
                model.entities.locus_instances[row.realization.instance.index()].path,
                model.entities.thread_domains[row.placement.domain.index()].name
            )
        })
        .collect()
}

#[test]
fn repeated_declarations_and_replicas_keep_their_instance_domains() {
    let model = model(
        r#"
        locus Leaf { }
        locus Worker { params { leaf: Leaf = Leaf { }; } }
        main locus App {
            params {
                a: Worker = Worker { };
                b: Worker = Worker { };
                c: Worker = Worker { };
            }
            placement {
                a: cooperative(pool = work);
                b: cooperative(pool = work);
                c: pinned(replicas = 2);
            }
        }
        fn main() { App { }; }
    "#,
    );
    let query = instance_domains(&model, &declarations(&model, &["Worker"]));
    assert_eq!(
        placed(&model, &query),
        [
            "App.a pool:work",
            "App.b pool:work",
            "App.c[0] pinned:App.c[0]",
            "App.c[1] pinned:App.c[1]",
        ]
    );
    assert_eq!(
        query.placed[0].placement.domain,
        query.placed[1].placement.domain
    );
    assert_ne!(
        query.placed[2].placement.domain,
        query.placed[3].placement.domain
    );
    assert!(query.holes.is_empty());
    assert!(query.missing_placement.is_empty());
    assert!(query.unrepresented.is_empty());
    // A declaration selection does not silently become a subtree.
    assert!(!placed(&model, &query)
        .iter()
        .any(|row| row.contains("leaf")));
    let children = instance_domains(&model, &declarations(&model, &["Leaf"]));
    assert_eq!(
        placed(&model, &children),
        [
            "App.a.leaf pool:work",
            "App.b.leaf pool:work",
            "App.c[0].leaf pinned:App.c[0]",
            "App.c[1].leaf pinned:App.c[1]",
        ]
    );
    // References retain the producer's evidence, rather than minting
    // a second account of where these instances came from.
    assert!(std::ptr::eq(
        query.placed[0].realization,
        model
            .relations
            .realizes
            .iter()
            .find(|row| row.instance == query.placed[0].placement.instance)
            .unwrap()
    ));
    assert!(std::ptr::eq(
        query.placed[0].placement,
        model
            .relations
            .placed_in
            .iter()
            .find(|row| row.instance == query.placed[0].realization.instance)
            .unwrap()
    ));
}

#[test]
fn cpu_affinity_overlap_does_not_merge_execution_domains() {
    let model = model(
        r#"
        locus Worker { }
        main locus App {
            params { a: Worker = Worker { }; b: Worker = Worker { }; }
            placement { a: pinned(cores = { 0 }); b: pinned(cores = { 0 }); }
        }
        fn main() { App { }; }
    "#,
    );
    let query = instance_domains(&model, &declarations(&model, &["Worker"]));
    assert_eq!(query.placed.len(), 2);
    assert_ne!(
        query.placed[0].placement.domain,
        query.placed[1].placement.domain
    );
    assert_eq!(
        model.relations.affined_to[0].cores,
        model.relations.affined_to[1].cores
    );
}

#[test]
fn dynamic_births_preserve_known_placements_and_scope_the_unknown() {
    let model = model(
        r#"
        locus Worker { }
        locus Stable { }
        main locus App {
            params { w: Worker = Worker { }; s: Stable = Stable { }; }
            run() { let extra = Worker { }; }
        }
        fn main() { App { }; }
    "#,
    );
    assert!(!model.capabilities.exact_placement);
    let worker = instance_domains(&model, &declarations(&model, &["Worker"]));
    assert_eq!(placed(&model, &worker), ["App.w main"]);
    assert_eq!(worker.holes.len(), 1);
    assert_eq!(
        worker.holes[0].kind,
        hale_model::HoleKind::RuntimeInheritedPlacement
    );
    assert!(worker.unrepresented.is_empty());
    let stable = instance_domains(&model, &declarations(&model, &["Stable"]));
    assert_eq!(placed(&model, &stable), ["App.s main"]);
    assert!(
        stable.holes.is_empty(),
        "another declaration's birth is irrelevant"
    );
}

#[test]
fn no_represented_deployment_is_distinct_from_missing_placement() {
    let model = model(
        r#"
        locus Unused { }
        locus Dynamic { }
        main locus App { run() { let dynamic = Dynamic { }; } }
        fn main() { App { }; }
    "#,
    );
    let selected = declarations(&model, &["Unused", "Dynamic"]);
    let query = instance_domains(&model, &selected);
    assert!(query.placed.is_empty());
    assert!(query.missing_placement.is_empty());
    assert_eq!(
        query.unrepresented,
        selected.into_iter().collect::<Vec<_>>()
    );
    assert_eq!(query.holes.len(), 1, "only Dynamic has an unmodeled birth");
    assert_eq!(
        query.holes[0].at,
        hale_model::EntityRef::LocusDecl(*declarations(&model, &["Dynamic"]).first().unwrap())
    );
}

#[test]
fn a_missing_placement_row_never_looks_like_no_instance() {
    let mut model = model(
        r#"
        locus Worker { }
        main locus App { params { w: Worker = Worker { }; } }
        fn main() { App { }; }
    "#,
    );
    let selected = declarations(&model, &["Worker"]);
    let instance = model
        .relations
        .realizes
        .iter()
        .find(|row| selected.contains(&row.decl))
        .unwrap()
        .instance;
    model
        .relations
        .placed_in
        .retain(|row| row.instance != instance);
    assert_eq!(model.validate(), Ok(()), "placement may be partial");
    let query = instance_domains(&model, &selected);
    assert!(query.placed.is_empty());
    assert_eq!(query.missing_placement.len(), 1);
    assert_eq!(query.missing_placement[0].instance, instance);
    assert!(query.unrepresented.is_empty());
    assert!(query.holes.is_empty());
}

#[test]
fn an_empty_selection_does_not_select_the_deployment() {
    let model = model(
        r#"
        locus Worker { }
        main locus App { params { w: Worker = Worker { }; } }
        fn main() { App { }; }
    "#,
    );
    let query = instance_domains(&model, &BTreeSet::new());
    assert!(query.placed.is_empty());
    assert!(query.missing_placement.is_empty());
    assert!(query.unrepresented.is_empty());
    assert!(query.holes.is_empty());
}
