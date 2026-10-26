//! The empty stdlib recovery inventory must not force whole-stdlib typing.
//! The existing reached-view contract exposes whether the whole walk was
//! already cached, so this test needs no production instrumentation.

#[test]
fn syntax_absence_returns_empty_recoveries_without_forcing_the_full_walk() {
    use hale_types::stdlib_bodies::{identities, program, recovery_rows, view_types, view_types_within};
    use hale_types::stdlib_reach::StdlibReach;
    assert_eq!(identities().expect("stdlib identities").recovery_syntax, Some(false));
    assert!(recovery_rows().expect("inventory available").is_empty());
    let user = hale_syntax::parse_source("fn main() { }").expect("parses");
    let reach = StdlibReach::of(&[&user]);
    assert!(!reach.is_whole());
    let outside: Vec<_> = (0..program().unwrap().items.len()).filter(|i| !reach.contains(*i)).collect();
    assert!(!outside.is_empty(), "a vacuity control for the unrequested declarations");
    let partial = view_types_within(&reach).expect("partial walk");
    for i in &outside {
        assert!(*partial.views[*i] == Default::default(), "recovery emptiness forced the whole typing walk");
    }
    // Negative control: the same observation detects a deliberate full walk.
    let whole = view_types().expect("whole walk");
    assert!(outside.iter().any(|i| *whole[*i] != Default::default()));
    let after = view_types_within(&reach).expect("cached whole walk");
    assert!(outside.iter().any(|i| *after.views[*i] != Default::default()));
    assert!(recovery_rows().expect("same empty inventory").is_empty());
}
