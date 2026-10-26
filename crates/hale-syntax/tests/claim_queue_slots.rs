use hale_syntax::ast::{ClaimForm, TopDecl};
use hale_syntax::parse_source;

#[test]
fn queue_reservation_bound_parses_and_formats() {
    let source = "claims { slots: bound queue_slots <= 24 on instances workers; }";
    let program = parse_source(source).unwrap();
    let TopDecl::Claims(block) = &program.items[0] else {
        panic!("claim block")
    };
    let ClaimForm::QueueSlots { group, limit } = &block.entries[0].form else {
        panic!("queue-slot claim")
    };
    assert_eq!(group.name, "workers");
    assert_eq!(*limit, 24);
    let formatted = hale_syntax::fmt::format_source(source).unwrap();
    parse_source(&formatted).unwrap();
    assert_eq!(
        hale_syntax::fmt::format_source(&formatted).unwrap(),
        formatted
    );
}

#[test]
fn queue_slots_stays_available_as_a_user_effect_on_paths() {
    let source =
        "effect queue_slots; claims { calls: bound queue_slots <= 1 on paths from workers; }";
    let program = parse_source(source).unwrap();
    let TopDecl::Claims(block) = &program.items[0] else {
        panic!("claim block")
    };
    assert!(matches!(block.entries[0].form, ClaimForm::Bound { .. }));
}

#[test]
fn malformed_reservations_are_refused() {
    for source in [
        "claims { slots: bound queue_slots <= -1 on instances workers; }",
        "claims { slots: bound queue_slots <= 1 on instances; }",
        "claims { slots: bound other <= 1 on instances workers; }",
    ] {
        assert!(parse_source(source).is_err(), "{source}");
    }
}
