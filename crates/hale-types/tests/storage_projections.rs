//! The analysis, model and lowering view share effective storage facts.
use hale_frontend::snapshot::{Config, Snapshot, Target};
use hale_model::{LocusDeclId, RecognitionMode as M, StorageProjection as P};
use hale_syntax::ast::{ProjectionClass as A, RecognitionSubMode as R};

const SOURCE: &str = r#"
locus Child { }
locus Plain { }
locus Accepting { accept(c: Child) { } }
locus ForcedRich : projection rich { accept(c: Child) { } }
locus Fixed : projection recognition(cap = 2, fixed_cell) { accept(c: Child) { } }
locus Slab : projection recognition(cap = 3, shared_slab) { accept(c: Child) { } }
locus Owner<T> : projection recognition(cap = 4, fixed_cell) { accept(c: T) { } }
fn main() { let owner: Owner<Child> = Owner { }; }
"#;
fn snapshot(source: &str) -> Snapshot {
    Snapshot::from_program(
        hale_syntax::parse_source(source).unwrap(),
        Vec::new(),
        Config::harness(Target::host()),
    )
    .unwrap_or_else(|_| panic!("load"))
}
#[test]
fn effective_defaults_and_modes_are_shared_with_the_model_and_view() {
    let s = snapshot(SOURCE);
    assert!(s.demand_check().unwrap().diags.is_empty());
    let summary = s.demand_alloc_summary().unwrap();
    let model = s.demand_model().unwrap();
    model.validate().unwrap();
    let view = s.demand_lowering().unwrap();
    let rows = view.storage_projections.as_ref().unwrap();
    for (name, class, policy, accepts) in [
        ("Plain", A::Rich, P::Rich, false),
        ("Accepting", A::Chunked, P::Chunked, true),
        ("ForcedRich", A::Rich, P::Rich, true),
        (
            "Fixed",
            A::Recognition(Some(hale_syntax::ast::RecognitionParams {
                cap: 2,
                sub_mode: R::FixedCell,
            })),
            P::Recognition {
                cap: 2,
                mode: M::FixedCell,
            },
            true,
        ),
        (
            "Slab",
            A::Recognition(Some(hale_syntax::ast::RecognitionParams {
                cap: 3,
                sub_mode: R::SharedSlab,
            })),
            P::Recognition {
                cap: 3,
                mode: M::SharedSlab,
            },
            true,
        ),
    ] {
        assert_eq!(summary.locus_shapes[name].projection.class, class, "{name}");
        assert_eq!(
            summary.locus_shapes[name].projection.accepts_children,
            accepts
        );
        assert_eq!(rows[name].class, class);
        let locus = model.entities.loci.iter().find(|l| l.name == name).unwrap();
        let storage = locus.storage.as_ref().unwrap();
        assert_eq!(storage.projection, policy);
        assert_eq!(storage.accepts_children, accepts);
    }
    assert_eq!(summary.locus_shapes["Fixed"].recognition_cap(), Some(2));
    assert_eq!(summary.locus_shapes["Slab"].recognition_cap(), Some(3));
    assert_eq!(
        rows["Owner_Child"],
        summary.locus_shapes["Owner"].projection
    );
    assert_eq!(s.builds()["alloc_summary"], 1);
    assert!(std::ptr::eq(view, s.demand_lowering().unwrap()));
}
#[test]
fn missing_model_policy_stays_unknown_and_unshipped_modes_stay_distinct() {
    let s = snapshot(SOURCE);
    let mut model = s.demand_model().unwrap().clone();
    let index = model
        .entities
        .loci
        .iter()
        .position(|l| l.name == "Fixed")
        .unwrap();
    let id = LocusDeclId(index as u32);
    model.entities.loci[index].storage = None;
    model.validate().unwrap();
    let selected = [id].into_iter().collect();
    assert!(
        hale_types::model_query::storage_policies(&model, &selected)[0]
            .1
            .is_none()
    );
    for (spelling, mode) in [
        ("spillover", R::Spillover),
        ("summary_only", R::SummaryOnly),
    ] {
        let source = SOURCE.replace("fixed_cell", spelling);
        let program = hale_syntax::parse_source(&source).unwrap();
        let fixed = hale_syntax::ast::flat_decls(&program.items)
            .find_map(|d| match d {
                hale_syntax::ast::TopDecl::Locus(l) if l.name.name == "Fixed" => Some(l),
                _ => None,
            })
            .unwrap();
        assert!(fixed.annotations.iter().any(|a| matches!(a,
            hale_syntax::ast::LocusAnnotation::Projection(A::Recognition(Some(p))) if p.cap == 2 && p.sub_mode == mode)));
        let s = snapshot(&source);
        assert!(s.demand_model().is_err());
    }
}
#[test]
fn a_missing_template_policy_cannot_be_filled_from_a_default() {
    let s = snapshot(SOURCE);
    let mut summary = s.demand_alloc_summary().unwrap().clone();
    summary.locus_shapes.remove("Owner");
    let error = summary
        .storage_projections(&s.demand_flow_instances().unwrap().instantiations)
        .unwrap_err();
    assert!(
        error.contains("Owner") && error.contains("required alloc_summary"),
        "{error}"
    );
}
