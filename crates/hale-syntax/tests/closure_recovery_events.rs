//! A closure's recovery events are a typed list (F.40 phase 4, W3).
//!
//! The parser types each name of a `persists_through(...)` or
//! `resets_on(...)` list once: the event it is when the name is in the
//! alphabet (`restart`, `restart_in_place`, `quarantine`), and the name
//! as written either way, with its span, for the check to refuse. A name
//! outside the alphabet is not a parse error: the check's message names
//! the alphabet at the name.

use hale_syntax::ast::{ClosureClause, ClosureDecl, LocusMember, RecoveryEvent, TopDecl};
use hale_syntax::parse_source;

const SRC: &str = "locus Tracker {
    params { delta: Int = 0; }
    closure band {
        sum(self.delta) ~~ 0 within 100;
        epoch tick;
        persists_through (quarantine, restart_in_place, dissolve, quarantin);
        resets_on (restart);
    }
}
";

fn closure(src: &str) -> ClosureDecl {
    let prog = parse_source(src).expect("parses");
    prog.items
        .iter()
        .find_map(|i| match i {
            TopDecl::Locus(l) => l.members.iter().find_map(|m| match m {
                LocusMember::Closure(c) => Some(c.clone()),
                _ => None,
            }),
            _ => None,
        })
        .expect("a closure")
}

#[test]
fn each_name_is_typed_once_and_kept_as_written() {
    let c = closure(SRC);
    let ClosureClause::PersistsThrough(persists) = &c.clauses[1] else { panic!("{:?}", c.clauses[1]) };
    let typed: Vec<(&str, Option<RecoveryEvent>, &str)> = persists
        .names
        .iter()
        .map(|n| (n.name.name.as_str(), n.event, &SRC[n.name.span.start.0 as usize..n.name.span.end.0 as usize]))
        .collect();
    assert_eq!(
        typed,
        [
            ("quarantine", Some(RecoveryEvent::Quarantine), "quarantine"),
            ("restart_in_place", Some(RecoveryEvent::RestartInPlace), "restart_in_place"),
            ("dissolve", None, "dissolve"),
            ("quarantin", None, "quarantin"),
        ]
    );
    // The clause's span runs from its keyword to its `;`.
    let clause = &SRC[persists.span.start.0 as usize..persists.span.end.0 as usize];
    assert_eq!(clause, "persists_through (quarantine, restart_in_place, dissolve, quarantin);");
    // What lowering reads: the events, in the alphabet only.
    assert_eq!(
        c.persists_through().collect::<Vec<_>>(),
        [RecoveryEvent::Quarantine, RecoveryEvent::RestartInPlace]
    );
    let ClosureClause::ResetsOn(resets) = &c.clauses[2] else { panic!("{:?}", c.clauses[2]) };
    assert_eq!(resets.events().collect::<Vec<_>>(), [RecoveryEvent::Restart]);
}

#[test]
fn the_alphabet_is_the_recovery_statements_names() {
    let names: Vec<&str> = RecoveryEvent::ALL.iter().map(|e| e.name()).collect();
    assert_eq!(names, ["restart", "restart_in_place", "quarantine"]);
    for e in RecoveryEvent::ALL {
        assert_eq!(RecoveryEvent::named(e.name()), Some(e));
    }
    assert_eq!(RecoveryEvent::named("dissolve"), None);
    assert_eq!(RecoveryEvent::named("replace"), None);
}

#[test]
fn an_empty_list_parses() {
    let c = closure("locus L { params { n: Int = 0; } closure c { self.n ~~ self.n within 0; persists_through (); } }");
    let ClosureClause::PersistsThrough(persists) = &c.clauses[0] else { panic!("{:?}", c.clauses[0]) };
    assert!(persists.names.is_empty());
}
