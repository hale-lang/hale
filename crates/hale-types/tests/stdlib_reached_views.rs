//! The stdlib copy's `views` column where a program reaches it
//! (`stdlib_bodies::view_types_within`), on its incremental path: each
//! item walked once per process, the first time a reach asks for it, by a
//! walk that stands the items already walked in. This file holds one test,
//! so its process runs no whole walk before it: the whole walk, once run,
//! answers every reach, and the incremental path would go unexercised.

use hale_types::stdlib_bodies::{mono_shapes, program, view_types, view_types_within};
use hale_types::stdlib_reach::StdlibReach;

/// Two reaches in sequence, the second walking only what the first did
/// not: every item each reaches has the table the whole copy's walk gives
/// it, and every monomorph shape they hold is the whole walk's.
#[test]
fn two_reaches_in_sequence_are_the_whole_walk() {
    let hello = hale_syntax::parse_source("fn main() { println(\"hi\"); }").expect("parses");
    let client = hale_syntax::parse_source(
        "fn main() { let c = std::http::Client { }; let r = c.get(\"http://x\"); println(r.body); }",
    )
    .expect("parses");
    let first = StdlibReach::of(&[&hello]);
    let second = StdlibReach::of(&[&client]);
    assert!(second.items().any(|i| !first.contains(i)), "the second reach walks items the first did not");
    let a = view_types_within(&first).expect("the stdlib parses");
    let b = view_types_within(&second).expect("the stdlib parses");
    let n = program().expect("the stdlib parses").items.len();
    assert!(!first.is_whole() && !second.is_whole());
    let whole = view_types().expect("the stdlib parses");
    let whole_monos = mono_shapes().expect("the stdlib parses");
    for (reach, got, which) in [(&first, &a, "first"), (&second, &b, "second")] {
        for i in 0..n {
            if reach.contains(i) {
                assert!(got.views[i] == whole[i], "{which} reach: item {i}'s table is not the whole walk's");
            } else {
                assert!(*got.views[i] == Default::default(), "{which} reach: item {i} is outside it");
            }
        }
        for (name, shape) in got.monos.iter() {
            assert_eq!(whole_monos.get(name), Some(shape), "{which} reach: monomorph `{name}`");
        }
    }
}
