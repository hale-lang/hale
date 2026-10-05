//! A closure's recovery events are judged (F.40 phase 4, W3).
//!
//! `persists_through(...)` and `resets_on(...)` name recovery events
//! from a closed alphabet (`restart`, `restart_in_place`, `quarantine`).
//! Each law here is pinned by its message, the source text at its span,
//! and its witness.

use hale_syntax::{parse_source, Diag, DiagKind};
#[path = "support/entries.rs"]
mod entries;
use entries::check_program;

fn diags(src: &str) -> Vec<Diag> {
    check_program(&parse_source(src).expect("parses"))
}

/// The source text a diagnostic points at.
fn at<'s>(src: &'s str, d: &Diag) -> &'s str {
    &src[d.span.start.0 as usize..d.span.end.0 as usize]
}

/// The diagnostics whose message starts with `closure `{name}``.
fn of<'d>(all: &'d [Diag], closure: &str) -> Vec<&'d Diag> {
    let head = format!("closure `{closure}`:");
    all.iter().filter(|d| d.message.starts_with(&head)).collect()
}

/// A locus whose closure accumulates, with `clauses` in its body.
fn tracker(clauses: &str) -> String {
    format!(
        "locus Tracker {{
    params {{ delta: Int = 0; }}
    closure band {{
        sum(self.delta) ~~ 0 within 100;
        epoch tick;
        {clauses}
    }}
}}
fn main() {{ Tracker {{ }}; }}
"
    )
}

// Ruling 2: a name outside the alphabet is an error, at the name.

#[test]
fn a_misspelled_event_is_refused_at_the_name_and_the_event_suggested() {
    let src = tracker("persists_through(quarantin);");
    let all = diags(&src);
    let found = of(&all, "band");
    assert_eq!(found.len(), 1, "{all:?}");
    let d = found[0];
    assert_eq!(d.kind, DiagKind::Type);
    assert_eq!(
        d.message,
        "closure `band`: `quarantin` is not a recovery event: `persists_through(...)` names `restart`, \
         `restart_in_place` or `quarantine`; did you mean `quarantine`?"
    );
    assert_eq!(at(&src, d), "quarantin");
    assert!(d.related.is_empty());
}

#[test]
fn a_name_far_from_every_event_names_the_alphabet_only() {
    let src = tracker("resets_on(replace);");
    let all = diags(&src);
    let found = of(&all, "band");
    assert_eq!(found.len(), 1, "{all:?}");
    assert_eq!(
        found[0].message,
        "closure `band`: `replace` is not a recovery event: `resets_on(...)` names `restart`, \
         `restart_in_place` or `quarantine`"
    );
    assert_eq!(at(&src, found[0]), "replace");
}

#[test]
fn each_name_outside_the_alphabet_is_its_own_error() {
    let src = tracker("persists_through(restart, restat, quarantine, rstart_in_place);");
    let all = diags(&src);
    let found: Vec<&str> = of(&all, "band").into_iter().map(|d| at(&src, d)).collect();
    assert_eq!(found, ["restat", "rstart_in_place"], "{all:?}");
    let suggested: Vec<bool> =
        of(&all, "band").into_iter().map(|d| d.message.ends_with("did you mean `restart`?")).collect();
    assert_eq!(suggested, [true, false], "a misspelling two edits away suggests nothing");
}

#[test]
fn dissolve_in_resets_on_is_outside_the_alphabet() {
    let src = tracker("resets_on(dissolve);");
    let all = diags(&src);
    let found = of(&all, "band");
    assert_eq!(found.len(), 1, "{all:?}");
    assert!(found[0].message.starts_with("closure `band`: `dissolve` is not a recovery event"), "{all:?}");
    assert_eq!(at(&src, found[0]), "dissolve");
}

// Ruling 3: `dissolve` in `persists_through(...)` is an error.

#[test]
fn persisting_through_dissolve_is_refused_and_says_why() {
    let src = tracker("persists_through(quarantine, dissolve);");
    let all = diags(&src);
    let found = of(&all, "band");
    assert_eq!(found.len(), 1, "{all:?}");
    let d = found[0];
    assert_eq!(d.kind, DiagKind::Type);
    assert_eq!(
        d.message,
        "closure `band`: an accumulator does not outlive its locus's dissolve, so \
         `persists_through(dissolve)` can mean nothing: `persists_through(...)` names `restart`, \
         `restart_in_place` or `quarantine`"
    );
    assert_eq!(at(&src, d), "dissolve");
}

// Ruling 4: `resets_on` states the default; a name in both clauses of
// one closure is an error.

#[test]
fn resets_on_states_the_default_and_is_accepted() {
    let src = tracker("persists_through(quarantine); resets_on(restart, restart_in_place);");
    let all = diags(&src);
    assert!(of(&all, "band").is_empty(), "{all:?}");
}

#[test]
fn an_event_in_both_clauses_is_refused_with_the_other_clause_as_witness() {
    let src = tracker("persists_through(restart_in_place, quarantine);\n        resets_on(restart, quarantine);");
    let all = diags(&src);
    let found = of(&all, "band");
    assert_eq!(found.len(), 1, "{all:?}");
    let d = found[0];
    assert_eq!(d.kind, DiagKind::Type);
    assert_eq!(
        d.message,
        "closure `band`: `quarantine` is in both `persists_through(...)` and `resets_on(...)`, which \
         contradict each other: its accumulators either survive `quarantine` or reset on it"
    );
    // At the `resets_on` name: the second `quarantine` in the source.
    let second = src.rfind("quarantine").unwrap();
    assert_eq!((d.span.start.0 as usize, at(&src, d)), (second, "quarantine"));
    let witness: Vec<(&str, usize, &str)> = d
        .related
        .iter()
        .map(|r| (r.label.as_str(), r.span.start.0 as usize, &src[r.span.start.0 as usize..r.span.end.0 as usize]))
        .collect();
    let first = src.find("quarantine").unwrap();
    assert_eq!(witness, [("`persists_through` names `quarantine` here", first, "quarantine")]);
}

#[test]
fn a_contradiction_is_reported_once_per_event_and_only_within_one_closure() {
    let src = "locus Tracker {
    params { delta: Int = 0; }
    closure a {
        sum(self.delta) ~~ 0 within 100;
        epoch tick;
        persists_through(restart);
        resets_on(restart, restart);
        resets_on(restart);
    }
    closure b {
        sum(self.delta) ~~ 0 within 100;
        epoch tick;
        resets_on(restart);
    }
}
fn main() { Tracker { }; }
";
    let all = diags(src);
    assert_eq!(of(&all, "a").len(), 1, "{all:?}");
    assert!(of(&all, "b").is_empty(), "{all:?}");
}

// Ruling 5: an event the closed world never applies to the locus is a
// warning, with who handles it and what they apply.

/// A closed world: `App` holds a `Tracker`; `clause` is the tracker's
/// clause and `handler` the body of `App`'s `on_failure` for it (`None`:
/// no handler); `extra` goes in `App`'s body.
fn world(clause: &str, handler: Option<&str>, extra: &str) -> String {
    let handler = handler
        .map(|body| format!("    on_failure(t: Tracker, err: ClosureViolation) {{ {body} }}\n"))
        .unwrap_or_default();
    format!(
        "locus Tracker {{
    params {{ delta: Int = 0; }}
    closure band {{
        sum(self.delta) ~~ 0 within 100;
        epoch tick;
        {clause}
    }}
}}
main locus App {{
    params {{ t: Tracker = Tracker {{ }}; }}
{handler}{extra}}}
fn main() {{ App {{ }}; }}
"
    )
}

/// The warnings of closure `band`.
fn warnings(all: &[Diag]) -> Vec<&Diag> {
    of(all, "band").into_iter().filter(|d| d.kind == DiagKind::Warn).collect()
}

/// A diagnostic's witness: each note, with the text at its span.
fn witness<'s>(src: &'s str, d: &'s Diag) -> Vec<(&'s str, &'s str)> {
    d.related
        .iter()
        .map(|r| (r.label.as_str(), &src[r.span.start.0 as usize..r.span.end.0 as usize]))
        .collect()
}

#[test]
fn an_event_a_handler_applies_is_silent() {
    for (clause, handler) in [
        ("persists_through(quarantine);", "quarantine(t);"),
        ("persists_through(restart);", "restart(t);"),
        ("resets_on(restart_in_place);", "restart_in_place(t);"),
        // A spent `for` bound quarantines.
        ("persists_through(quarantine, restart);", "restart(t) for 3;"),
    ] {
        let src = world(clause, Some(handler), "");
        let all = diags(&src);
        assert!(of(&all, "band").is_empty(), "{clause} / {handler}: {all:?}");
    }
}

#[test]
fn an_event_no_handler_applies_is_a_warning_with_the_handler_as_witness() {
    let src = world("persists_through(quarantine);", Some("restart(t);"), "");
    let all = diags(&src);
    let found = warnings(&all);
    assert_eq!(found.len(), 1, "{all:?}");
    let d = found[0];
    assert_eq!(
        d.message,
        "closure `band`: no recovery in this program applies `quarantine` to a `Tracker`, so \
         `persists_through(quarantine)` never takes effect"
    );
    assert_eq!(at(&src, d), "quarantine");
    assert_eq!(
        witness(&src, d),
        [(
            "`App` handles a failing `Tracker` here and applies `restart`",
            "on_failure(t: Tracker, err: ClosureViolation) { restart(t); }"
        )]
    );
    assert!(!all.iter().any(|d| d.is_error()), "a warning, never an error: {all:?}");
}

#[test]
fn a_handler_that_absorbs_is_named_as_applying_nothing() {
    let src = world("resets_on(restart);", Some("println(\"absorbed\");"), "");
    let all = diags(&src);
    let found = warnings(&all);
    assert_eq!(found.len(), 1, "{all:?}");
    assert!(found[0].message.contains("so `resets_on(restart)` never takes effect"), "{all:?}");
    assert_eq!(witness(&src, found[0])[0].0, "`App` handles a failing `Tracker` here and applies no recovery event");
}

#[test]
fn with_no_handler_the_witness_is_the_locus() {
    let src = world("persists_through(restart_in_place);", None, "");
    let all = diags(&src);
    let found = warnings(&all);
    assert_eq!(found.len(), 1, "{all:?}");
    assert_eq!(
        witness(&src, found[0]),
        [(
            "`Tracker` is declared here; no `on_failure` in the program handles one, and no recovery statement \
             is applied to one",
            "Tracker"
        )]
    );
}

#[test]
fn a_recovery_statement_outside_a_handler_applies_its_event() {
    // `restart_in_place(self.t)` in a method: the rows name its child by
    // the field's declared type.
    let method = "    fn again() { restart_in_place(self.t); }\n";
    let src = world("persists_through(restart_in_place);", Some("restart(t);"), method);
    let all = diags(&src);
    assert!(of(&all, "band").is_empty(), "{all:?}");
    // And it is in the witness of an event nothing applies.
    let src = world("persists_through(quarantine);", Some("restart(t);"), method);
    let all = diags(&src);
    let found = warnings(&all);
    assert_eq!(found.len(), 1, "{all:?}");
    assert_eq!(
        witness(&src, found[0]),
        [
            (
                "`App` handles a failing `Tracker` here and applies `restart`",
                "on_failure(t: Tracker, err: ClosureViolation) { restart(t); }"
            ),
            ("`App` applies `restart_in_place` to a `Tracker` here", "restart_in_place(self.t);"),
        ]
    );
}

#[test]
fn a_bounded_restart_outside_a_handler_applies_quarantine() {
    let src = world("persists_through(quarantine);", None, "    fn again(t: Tracker) { restart(t) for 2; }\n");
    let all = diags(&src);
    assert!(of(&all, "band").is_empty(), "{all:?}");
}

#[test]
fn an_open_world_is_not_judged() {
    // No entry: a library checked alone has no parents.
    let src = "locus Tracker {
    params { delta: Int = 0; }
    closure band { sum(self.delta) ~~ 0 within 100; epoch tick; persists_through(restart); }
}
";
    let all = diags(src);
    assert!(of(&all, "band").is_empty(), "{all:?}");
}

#[test]
fn an_imported_locus_is_not_judged() {
    let src = world("persists_through(restart); resets_on(restart_in_place);", None, "");
    let mut prog = parse_source(&src).expect("parses");
    for item in &mut prog.items {
        if let hale_syntax::ast::TopDecl::Locus(l) = item {
            if l.name.name == "Tracker" {
                l.imported = true;
            }
        }
    }
    let all = check_program(&prog);
    assert!(of(&all, "band").is_empty(), "{all:?}");
}

#[test]
fn an_event_a_generic_supervisor_applies_is_not_judged() {
    // `Sup<T>` restarts a `T`: the rows cannot name which loci, so no
    // `restart` is reported unreached; `quarantine` still is.
    let src = world(
        "persists_through(restart, quarantine);",
        None,
        "    params { s: Sup<Tracker> = Sup<Tracker> { }; }\n",
    )
    .replace("main locus App", "locus Sup<T> {\n    on_failure(c: T, err: ClosureViolation) { restart(c); }\n}\nmain locus App");
    let all = diags(&src);
    let found: Vec<&str> = warnings(&all).into_iter().map(|d| at(&src, d)).collect();
    assert_eq!(found, ["quarantine"], "{all:?}");
}

#[test]
fn a_receiver_the_rows_cannot_name_suspends_its_event() {
    // `restart(x)` on a local: its child type is not a declared param's,
    // so whether it restarts a `Tracker` is not judged.
    let method = "    fn again() { let x = self.t; restart(x); }\n";
    let src = world("persists_through(restart, quarantine);", None, method);
    let all = diags(&src);
    let found: Vec<&str> = warnings(&all).into_iter().map(|d| at(&src, d)).collect();
    assert_eq!(found, ["quarantine"], "{all:?}");
}

#[test]
fn example_41s_quarantine_is_reached() {
    // The corpus's one `persists_through`: `Coordinator` quarantines its
    // `Tracker`. As written the program's entry is `fn main` (no `main
    // locus`, so rule 9's world is open); as the entry it is judged, and
    // the event is reached.
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../hale-codegen/tests/fixtures/examples/41-closure-accumulator/main.hl");
    let src = std::fs::read_to_string(path).expect("example 41");
    assert!(src.contains("persists_through (quarantine);"));
    for src in [src.clone(), src.replace("locus Coordinator", "main locus Coordinator")] {
        let all = diags(&src);
        assert!(of(&all, "within_band").is_empty(), "{all:?}");
        assert!(!all.iter().any(|d| d.is_error()), "{all:?}");
    }
}

// Ruling 6: `persists_through` on a closure with no accumulator.

#[test]
fn persisting_with_no_accumulator_is_a_warning_at_the_clause() {
    let src = "locus Gauge {
    params { x: Int = 0; y: Int = 0; }
    closure level {
        self.x ~~ self.y within 1;
        epoch tick;
        persists_through(restart);
    }
}
main locus App {
    params { g: Gauge = Gauge { }; }
    on_failure(g: Gauge, err: ClosureViolation) { restart(g); }
}
fn main() { App { }; }
";
    let all = diags(src);
    let found: Vec<&Diag> = of(&all, "level");
    assert_eq!(found.len(), 1, "{all:?}");
    let d = found[0];
    assert_eq!(d.kind, DiagKind::Warn);
    assert_eq!(
        d.message,
        "closure `level`: `persists_through(...)` keeps a closure's accumulators through a recovery, and this \
         closure has none, so the clause keeps nothing"
    );
    assert_eq!(at(src, d), "persists_through(restart);");
    assert_eq!(
        witness(src, d),
        [("the assertion accumulates nothing: no `sum`, `count` or `mean`", "self.x ~~ self.y within 1;")]
    );
}

#[test]
fn an_inline_closure_persisting_through_is_a_warning_with_no_witness() {
    let src = "locus L {
    params { e: String = \"\"; }
    closure fatal { captures: e; epoch inline; persists_through(quarantine); }
}
";
    let all = diags(src);
    let found: Vec<&Diag> = of(&all, "fatal");
    assert_eq!(found.len(), 1, "{all:?}");
    assert_eq!(found[0].kind, DiagKind::Warn);
    assert!(found[0].related.is_empty());
}

#[test]
fn the_alphabet_is_accepted() {
    let src = tracker("persists_through(restart, restart_in_place, quarantine);");
    let all = diags(&src);
    assert!(all.iter().all(|d| !d.message.contains("recovery event") && !d.message.contains("dissolve")), "{all:?}");
}
