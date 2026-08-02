//! User-declared effect classes (#345).
//!
//! A payments system wants to say which functions move money; the
//! propagation engine is already generic — it unions a bitmask over
//! the call graph and does not care what the bits mean. What was
//! missing is a way for a program to name its own bits.
//!
//! Grounded exactly like a built-in: attached to a leaf with
//! `@effects(is: {...})` and propagated by the same engine. The
//! compiler owns propagation; the program owns classification, which
//! is the same split the stdlib registry has with a different owner.

use hale_syntax::parse_source;

fn errs(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

const MONEY: &str = "\
effect money;
@effects(is: {money})
fn charge(cents: Int) -> Int { return cents; }
fn calc(n: Int) -> Int { return n * 2; }
";

#[test]
fn a_user_effect_propagates_to_an_assertion() {
    let ds = errs(&format!(
        "{MONEY}@effects(none: {{money}})\n\
         fn price(n: Int) -> Int {{ return charge(n); }}\n\
         fn main() {{ println(price(5)); }}"
    ));
    assert!(
        ds.iter().any(|m| m.contains("effect assertion violated")),
        "reaching a fn that carries `money` must violate: {:?}",
        ds
    );
}

/// The control. Without it an over-broad attribution would satisfy the
/// test above while making every call violate everything.
#[test]
fn a_path_avoiding_the_carrier_still_certifies() {
    let ds = errs(&format!(
        "{MONEY}@effects(none: {{money}})\n\
         fn price(n: Int) -> Int {{ return calc(n); }}\n\
         fn main() {{ println(price(5)); }}"
    ));
    assert!(
        !ds.iter().any(|m| m.contains("effect assertion violated")),
        "a path that never reaches the carrier must certify: {:?}",
        ds
    );
}

/// Propagation is transitive — the whole point of using the engine
/// rather than a local check.
#[test]
fn it_propagates_through_an_intermediate() {
    let ds = errs(&format!(
        "{MONEY}fn mid(n: Int) -> Int {{ return charge(n); }}\n\
         @effects(none: {{money}})\n\
         fn price(n: Int) -> Int {{ return mid(n); }}\n\
         fn main() {{ println(price(5)); }}"
    ));
    assert!(
        ds.iter().any(|m| m.contains("effect assertion violated")),
        "a user effect must propagate through a helper: {:?}",
        ds
    );
}

/// Built-in classes must be unaffected by the user-class machinery.
#[test]
fn builtin_classes_still_work_alongside() {
    let ds = errs(
        "effect money;\n\
         @no_syscall\n\
         fn f() -> Int { return len(std::io::fs::read_file(\"/x\") or \"\"); }\n\
         fn main() { println(f()); }",
    );
    assert!(
        ds.iter().any(|m| m.contains("must not reach `syscall`")),
        "declaring a user effect must not disturb built-ins: {:?}",
        ds
    );
}

/// The diagnostic must name the class the AUTHOR declared.
///
/// `EffectClass::as_str` returns `&'static str`, so a `User(i)` — an
/// index into the seed's intern table — has no static name to give and
/// answered `<user effect>`. Every diagnostic that reached for it
/// printed that placeholder, which discards the single thing a user
/// effect exists to carry: `money` is the reason anyone declared it,
/// and a violation report that won't say `money` is barely a report.
#[test]
fn a_violation_names_the_declared_class() {
    let ds = errs(&format!(
        "{MONEY}@effects(none: {{money}})\n\
         fn price(n: Int) -> Int {{ return charge(n); }}\n\
         fn main() {{ println(price(5)); }}"
    ));
    let violation = ds
        .iter()
        .find(|m| m.contains("effect assertion violated"))
        .expect("the assertion is violated");
    assert!(
        violation.contains("money"),
        "the diagnostic must name the declared class, not a placeholder: {}",
        violation
    );
    assert!(
        !ds.iter().any(|m| m.contains("<user effect>")),
        "no diagnostic may leak the placeholder name: {:?}",
        ds
    );
}

/// Declaring a user class must not rename the built-ins — `User(i)`
/// resolves through a table the built-ins never index into.
#[test]
fn declaring_a_user_class_leaves_builtin_names_intact() {
    let ds = errs(
        "effect money;\n\
         fn leaf(n: Int) -> Int { println(\"x\"); return n; }\n\
         @no_syscall\n\
         fn price(n: Int) -> Int { return leaf(n); }\n\
         fn main() { println(price(5)); }",
    );
    assert!(
        ds.iter().any(|m| m.contains("`syscall`")),
        "a built-in class must still print its own name: {:?}",
        ds
    );
}

/// Declaring more classes than the mask holds must be an ERROR, not a
/// saturating no-op.
///
/// `class_mask` used to answer `PURE` for any class past the ceiling.
/// PURE means "reaches nothing", so `@effects(none: {overflowed})`
/// SILENTLY CERTIFIED a fn that called a declared source of it — the
/// analysis failed open. Everything else in this system fails closed
/// (an unclassified stdlib leaf is treated as doing anything), and an
/// effect certificate that is quietly false is worse than no
/// certificate: it is believed. The ceiling is now rejected at the
/// declaration, where there is a span to point at.
#[test]
fn overflowing_the_class_ceiling_is_rejected() {
    let cap = hale_syntax::ast::EffectClass::USER_CAPACITY;
    let decls: String =
        (0..=cap).map(|i| format!("effect e{};\n", i)).collect();
    let src = format!("{decls}fn main() {{ println(1); }}");
    let err = hale_syntax::parse_source(&src)
        .err()
        .expect("declaring past the ceiling must not parse");
    let rendered = format!("{:?}", err);
    assert!(
        rendered.contains("too many declared effect classes"),
        "the overflow must be diagnosed at the declaration: {}",
        rendered
    );
}

/// The last class that FITS must still work — an off-by-one here would
/// silently cost a usable class, or worse, hand out a bit that aliases.
#[test]
fn the_last_class_within_the_ceiling_still_propagates() {
    let cap = hale_syntax::ast::EffectClass::USER_CAPACITY;
    let last = cap - 1;
    let decls: String =
        (0..cap).map(|i| format!("effect e{};\n", i)).collect();
    let src = format!(
        "{decls}@effects(is: {{e{last}}})\n\
         fn leaf(n: Int) -> Int {{ return n; }}\n\
         @effects(none: {{e{last}}})\n\
         fn caller(n: Int) -> Int {{ return leaf(n); }}\n\
         fn main() {{ println(caller(1)); }}"
    );
    let program = hale_syntax::parse_source(&src).expect("parse");
    let ds: Vec<String> = hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect();
    assert!(
        ds.iter().any(|m| m.contains("effect assertion violated")
            && m.contains(&format!("e{last}"))),
        "the highest in-range class must still propagate and be named: {:?}",
        ds
    );
}

/// The manifest is a committed baseline whose DIFF is the review
/// artifact. A placeholder name there is worse than in a diagnostic:
/// every user class renders identically, so two distinct classes
/// produce the same line and a real change can diff to nothing.
#[test]
fn the_manifest_names_user_classes() {
    let src = format!(
        "{MONEY}@effects(none: {{money}})\n\
         fn price(n: Int) -> Int {{ return n; }}\n\
         fn main() {{ println(price(5)); }}"
    );
    let program = hale_syntax::parse_source(&src).expect("parse");
    let rows = hale_types::effects::effect_manifest(&[&program]);
    let rendered = format!("{:?}", rows);
    assert!(
        rendered.contains("money"),
        "the manifest must name the declared class: {}",
        rendered
    );
    assert!(
        !rendered.contains("<user effect>"),
        "the manifest must not leak the placeholder: {}",
        rendered
    );
}

/// An undeclared class must be an ERROR, not a fresh class.
///
/// Interning happens on DECLARATION (`effect money;`) and on bare
/// REFERENCE (`@effects(none: { money })`) alike, so a misspelling
/// minted a brand-new class that nothing carries — and the assertion
/// then held vacuously and reported success. `@effects(none: { monye })`
/// typechecked clean on a fn that called a declared `money` source.
///
/// This is the same failure as the mask overflow, arrived at from the
/// other side: there the class had no bit, here it has no carriers.
/// Both produce a certificate that is quietly true of nothing, which
/// is indistinguishable to the reader from one that is true of
/// something — and a typo is a far likelier way to reach it than
/// declaring 55 classes.
#[test]
fn an_undeclared_class_is_rejected() {
    let ds = errs(&format!(
        "{MONEY}@effects(none: {{monye}})\n\
         fn price(n: Int) -> Int {{ return charge(n); }}\n\
         fn main() {{ println(price(5)); }}"
    ));
    let d = ds
        .iter()
        .find(|m| m.contains("never declared"))
        .unwrap_or_else(|| panic!("undeclared class must be rejected: {:?}", ds));
    assert!(
        d.contains("monye"),
        "the diagnostic must name the undeclared class: {}",
        d
    );
    assert!(
        d.contains("Did you mean `money`?"),
        "a one-transposition miss should offer the declared neighbour: {}",
        d
    );
}

/// The guard must not fire on a legitimately declared class, or every
/// real user of the feature eats a false error.
#[test]
fn a_declared_class_is_not_flagged_undeclared() {
    let ds = errs(&format!(
        "{MONEY}@effects(none: {{money}})\n\
         fn price(n: Int) -> Int {{ return n; }}\n\
         fn main() {{ println(price(5)); }}"
    ));
    assert!(
        !ds.iter().any(|m| m.contains("never declared")),
        "a declared class must not be reported undeclared: {:?}",
        ds
    );
}

/// A fn's OWN `is: {…}` must be part of its inferred effect set.
///
/// `infer_effects` unioned `carries` for a CALLEE but never for the
/// function itself, so a fn's classification was invisible to its own
/// set. That is what made a user class silently not travel over the
/// bus: `causes_diags` infers each subscriber's effects from here, and
/// a subscriber declaring `is: {money}` contributed nothing.
#[test]
fn a_fns_own_carries_is_in_its_inferred_set() {
    let program = hale_syntax::parse_source(MONEY).expect("parse");
    let rows = hale_types::effects::effect_manifest_with_inference(&[&program]);
    let charge = rows
        .iter()
        .find(|r| r.func == "charge")
        .expect("charge is in the manifest");
    let rendered = format!("{:?}", charge);
    assert!(
        rendered.contains("money"),
        "a fn declaring `is: {{money}}` must infer `money` for itself: {}",
        rendered
    );
}
