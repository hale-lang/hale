//! `@effects(only: {…})` — the closed dual of `none:` (#354).
//!
//! `none:` is open-ended, so it rots. Expressing "this handler only
//! allocates" means enumerating every other class, and adding a class
//! to the language silently widens every such contract: the annotation
//! still reads "only alloc" and no longer means it. Nothing fails; the
//! certificate just quietly weakens.
//!
//! `only:` is checked as `none:` over the COMPLEMENT, computed at
//! check time from the live class universe — the built-ins plus every
//! declared user class. Nothing is written down that could go stale,
//! which is the entire difference.
//!
//! The load-bearing test here is the last one: a class the contract
//! never mentions, declared after the contract was written, is caught
//! anyway. That is the property a hand-enumerated `none:` list cannot
//! have.

use hale_syntax::parse_source;

fn errs(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

#[test]
fn a_fn_within_its_declared_set_passes() {
    let ds = errs(
        "@effects(only: { alloc })\n\
         fn ok(n: Int) -> String { return \"a\" + \"b\"; }\n\
         fn main() { println(ok(1)); }",
    );
    assert!(
        !ds.iter().any(|m| m.contains("closed effect contract")),
        "allocating under `only: {{ alloc }}` must be allowed: {:?}",
        ds
    );
}

#[test]
fn a_class_outside_the_declared_set_violates() {
    let ds = errs(
        "@effects(only: { alloc })\n\
         fn bad(n: Int) -> String { println(\"x\"); return \"a\" + \"b\"; }\n\
         fn main() { println(bad(1)); }",
    );
    let d = ds
        .iter()
        .find(|m| m.contains("closed effect contract"))
        .unwrap_or_else(|| panic!("syscall is outside `only: {{alloc}}`: {:?}", ds));
    assert!(
        d.contains("syscall"),
        "the diagnostic must name the class reached: {}",
        d
    );
    assert!(
        d.contains("only: { alloc }"),
        "and the contract that forbids it by OMISSION — otherwise a \
         reader hunts for a `none:` that was never written: {}",
        d
    );
}

/// The point of the feature. `money` is declared, carried by a leaf,
/// and never named in the contract — and is caught regardless, because
/// the complement is computed rather than enumerated.
#[test]
fn a_class_the_contract_never_mentions_is_still_caught() {
    let ds = errs(
        "effect money;\n\
         @effects(is: { money })\n\
         fn charge(n: Int) -> Int { return n; }\n\
         @effects(only: { alloc })\n\
         fn quote(n: Int) -> Int { return charge(n); }\n\
         fn main() { println(quote(1)); }",
    );
    assert!(
        ds.iter().any(|m| m.contains("closed effect contract") && m.contains("money")),
        "a user class outside the declared set must be caught without \
         the contract naming it — this is what `none:` cannot do: {:?}",
        ds
    );
}

/// A user class INSIDE the set is permitted — the complement must be
/// a real complement, not "everything user-declared".
#[test]
fn a_user_class_inside_the_declared_set_passes() {
    let ds = errs(
        "effect money;\n\
         @effects(is: { money })\n\
         fn charge(n: Int) -> Int { return n; }\n\
         @effects(only: { money })\n\
         fn quote(n: Int) -> Int { return charge(n); }\n\
         fn main() { println(quote(1)); }",
    );
    assert!(
        !ds.iter().any(|m| m.contains("closed effect contract")),
        "`money` is declared allowed and must not be reported: {:?}",
        ds
    );
}

// ---- composed classes (#354 part 2) --------------------------------
//
// `effect io = { syscall, block };` gives `io` no bit of its own — its
// mask is the union of its members'. That one fact yields both useful
// directions with no new analysis, which is what these pin.

/// Downward: forbidding the composed name forbids every member.
#[test]
fn forbidding_a_composed_class_catches_a_member() {
    let ds = errs(
        "effect io = { syscall, block };\n\
         @effects(none: { io })\n\
         fn f(n: Int) -> Int { println(\"x\"); return n; }\n\
         fn main() { println(f(1)); }",
    );
    assert!(
        ds.iter().any(|m| m.contains("effect assertion violated") && m.contains("io")),
        "a syscall is a member of `io`, so `none: {{io}}` must catch it: {:?}",
        ds
    );
}

/// A composed class must not forbid classes it does NOT list.
#[test]
fn a_composed_class_does_not_over_forbid() {
    let ds = errs(
        "effect io = { syscall };\n\
         @effects(none: { io })\n\
         fn f(n: Int) -> String { return \"a\" + \"b\"; }\n\
         fn main() { println(f(1)); }",
    );
    assert!(
        !ds.iter().any(|m| m.contains("effect assertion violated")),
        "`io` is only syscall here; allocating must stay legal: {:?}",
        ds
    );
}

/// The payoff. `@deterministic` is a hardcoded blacklist of
/// {time, entropy, env} and cannot see a user class — the fail-open
/// this issue was filed for. Composition fixes it with no new
/// mechanism: defining the class in terms of `time` puts the time bit
/// in its mask, so the existing contract catches it.
#[test]
fn a_composed_class_is_visible_to_deterministic() {
    let ds = errs(
        "effect wallclock = { time };\n\
         @effects(is: { wallclock })\n\
         fn read_clock(n: Int) -> Int { return n; }\n\
         @deterministic\n\
         fn pure_ish(n: Int) -> Int { return read_clock(n); }\n\
         fn main() { println(pure_ish(1)); }",
    );
    assert!(
        ds.iter().any(|m| m.contains("effect assertion violated")),
        "a user class defined as `{{ time }}` must be caught by \
         @deterministic without touching the parser's hardcoded list: {:?}",
        ds
    );
}

/// An ATOMIC user class is still invisible to `@deterministic` — that
/// is correct, not a bug. `money` is not a clock read. The fix is
/// opt-in by definition, which is the principled form.
#[test]
fn an_atomic_user_class_is_not_swept_into_deterministic() {
    let ds = errs(
        "effect money;\n\
         @effects(is: { money })\n\
         fn charge(n: Int) -> Int { return n; }\n\
         @deterministic\n\
         fn quote(n: Int) -> Int { return charge(n); }\n\
         fn main() { println(quote(1)); }",
    );
    assert!(
        !ds.iter().any(|m| m.contains("effect assertion violated")),
        "`money` is unrelated to determinism and must not be swept in: {:?}",
        ds
    );
}

/// A cyclic definition resolves to no effect at all, so every contract
/// naming it would hold vacuously — a silently-inert class is the same
/// failure mode as the mask overflow in #348.
#[test]
fn a_cyclic_definition_is_rejected() {
    let ds = errs(
        "effect a = { b };\n\
         effect b = { a };\n\
         @effects(none: { a })\n\
         fn f(n: Int) -> Int { return n; }\n\
         fn main() { println(f(1)); }",
    );
    assert!(
        ds.iter().any(|m| m.contains("defined in terms of itself")),
        "a definition cycle must be rejected, not silently inert: {:?}",
        ds
    );
}
