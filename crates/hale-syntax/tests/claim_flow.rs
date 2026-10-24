use hale_syntax::ast::{ClaimForm, LocusMember, TopDecl};
#[test]
fn flow_policy_preserves_its_group_operand() {
    let p = hale_syntax::parse_source(
        "main locus App { claims { lifetime: require flow(all jobs); } }",
    )
    .unwrap();
    let TopDecl::Locus(app) = &p.items[0] else {
        panic!()
    };
    let LocusMember::Claims(claims) = &app.members[0] else {
        panic!()
    };
    assert!(
        matches!(&claims.entries[0].form, ClaimForm::RequireFlow { group } if group.name == "jobs")
    );
}
#[test]
fn flow_policy_requires_the_universal_quantifier() {
    for quantifier in ["some jobs", "jobs", "all"] {
        assert!(hale_syntax::parse_source(&format!(
            "main locus App {{ claims {{ lifetime: require flow({quantifier}); }} }}"
        ))
        .is_err());
    }
}
