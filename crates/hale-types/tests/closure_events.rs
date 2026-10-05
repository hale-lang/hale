//! A closure's recovery events are judged (F.40 phase 4, W3).
//!
//! `persists_through(...)` and `resets_on(...)` name recovery events
//! from a closed alphabet (`restart`, `restart_in_place`, `quarantine`).
//! Each law here is pinned by its message, the source text at its span,
//! and its witness.

use hale_syntax::{parse_source, Diag, DiagKind};
use hale_types::check_program;

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

#[test]
fn the_alphabet_is_accepted() {
    let src = tracker("persists_through(restart, restart_in_place, quarantine);");
    let all = diags(&src);
    assert!(all.iter().all(|d| !d.message.contains("recovery event") && !d.message.contains("dissolve")), "{all:?}");
}
