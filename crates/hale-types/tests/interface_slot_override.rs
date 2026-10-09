//! GH #533 (DNA F.11, 2026-09-05): a call through an interface-typed
//! field followed the field's DECLARATION DEFAULT, not the impl the
//! constructor stored. `Holder { dep: Real { } }` over a
//! `dep: Gate = Noop { }` default ran `Real::apply` and
//! `forbid reaches(.., effects(apply_it))` passed — fail-open on the
//! constructor-shaped assembly #521 is built from; the mirror was a
//! false positive. The slot now keeps its declared interface and the
//! dispatch rewrite fans to every conformer, the rule a one-hop slot
//! and an interface-typed fn param already followed.

#[path = "support/entries.rs"]
mod entries;
use hale_syntax::parse_source;
use entries::check_program;

fn diags(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

fn program(default_impl: &str, override_impl: &str) -> String {
    format!(
        r#"
effect apply_it;
interface Gate {{ fn apply(x: String) -> Bool; }}
locus Real {{ @effects(is: {{ apply_it }}) fn apply(x: String) -> Bool {{ return true; }} }}
locus Noop {{ fn apply(x: String) -> Bool {{ return false; }} }}
locus Holder {{ params {{ dep: Gate = {default_impl} {{ }}; }} }}
group organism = {{ App }};
main locus App {{
    params {{ h: Holder = Holder {{ dep: {override_impl} {{ }} }}; }}
    claims {{ gated: forbid reaches(organism, effects(apply_it)); }}
    run() {{ let ok = self.h.dep.apply("x"); }}
}}
fn main() {{ App {{ }}; }}
"#
    )
}

/// Default harmless, override the carrier: the carrier runs, so the
/// claim must be refused with a witness through it.
#[test]
fn override_with_the_carrier_is_refused() {
    let ds = diags(&program("Noop", "Real"));
    assert!(
        ds.iter().any(|m| m.contains("claim `gated` violated") && m.contains("Real::apply")),
        "the constructor override must be reachable: {:?}",
        ds
    );
}

/// Default the carrier, override harmless, and the only site
/// overrides: GH #540 narrows the slot to what the program stores
/// into it. The default literal is never evaluated, so the carrier
/// is not stored and the claim holds — whichever literal happens to
/// be the default no longer decides the verdict.
#[test]
fn default_carrier_with_harmless_override_holds_when_no_site_stores_it() {
    let ds = diags(&program("Real", "Noop"));
    assert!(
        !violated(&ds),
        "the carrier is only the default and every site overrides it: {:?}",
        ds
    );
}

/// The default is used when a literal omits the field, so it is one
/// of the slot's stores then.
#[test]
fn default_carrier_used_by_an_omitting_site_is_reached() {
    let extra = "fn mk() -> Holder { return Holder { }; }";
    let ds = diags(&slot_program(" = Real { }", "Holder { dep: Other { } }", extra));
    assert!(violated(&ds), "{:?}", ds);
}

fn slot_program(default_impl: &str, stores: &str, extra: &str) -> String {
    format!(
        r#"
effect apply_it;
interface Gate {{ fn apply(x: String) -> Bool; }}
locus Real {{ @effects(is: {{ apply_it }}) fn apply(x: String) -> Bool {{ return true; }} }}
locus Noop {{ fn apply(x: String) -> Bool {{ return false; }} }}
locus Other {{ fn apply(x: String) -> Bool {{ return false; }} }}
locus Holder {{ params {{ dep: Gate{default_impl}; }} }}
group organism = {{ App }};
main locus App {{
    params {{ h: Holder = {stores}; }}
    claims {{ gated: forbid reaches(organism, effects(apply_it)); }}
    run() {{ let ok = self.h.dep.apply("x"); }}
}}
{extra}
fn main() {{ App {{ }}; }}
"#
    )
}

fn violated(ds: &[String]) -> bool {
    ds.iter().any(|m| m.contains("claim `gated` violated"))
}

/// The carrier conforms but is never stored: the claim holds.
#[test]
fn unstored_carrier_does_not_reach_through_the_slot() {
    let ds = diags(&slot_program(" = Noop { }", "Holder { dep: Other { } }", ""));
    assert!(!violated(&ds), "Real is never stored into the slot: {:?}", ds);
}

/// Two sites store different impls: both are callees.
#[test]
fn two_sites_keep_both_impls() {
    let extra = "fn mk() -> Holder { return Holder { dep: Real { } }; }";
    let ds = diags(&slot_program(" = Noop { }", "Holder { dep: Other { } }", extra));
    assert!(violated(&ds), "the second site stores the carrier: {:?}", ds);
}

/// A literal whose value is an interface-typed parameter is a write
/// the pre-pass cannot name: the slot keeps every conformer.
#[test]
fn slot_written_from_a_parameter_keeps_every_conformer() {
    let extra = "fn mk(g: Gate) -> Holder { return Holder { dep: g }; }";
    let ds = diags(&slot_program(" = Noop { }", "Holder { dep: Other { } }", extra));
    assert!(violated(&ds), "an unseen write puts every conformer back: {:?}", ds);
}

/// An assignment of a value the pre-pass cannot name does the same,
/// whatever the receiver.
#[test]
fn slot_assigned_an_opaque_value_keeps_every_conformer() {
    let extra = "fn swap(h: Holder, g: Gate) { h.dep = g; }";
    let ds = diags(&slot_program(" = Noop { }", "Holder { dep: Other { } }", extra));
    assert!(violated(&ds), "{:?}", ds);
}

/// A slot with no default narrows to its literals.
#[test]
fn slot_without_default_narrows_to_its_literals() {
    let ds = diags(&slot_program("", "Holder { dep: Other { } }", ""));
    assert!(!violated(&ds), "{:?}", ds);
}

/// With no carrier conforming to the interface at all, the claim holds.
#[test]
fn no_conforming_carrier_holds() {
    let src = r#"
effect apply_it;
interface Gate { fn apply(x: String) -> Bool; }
locus Noop { fn apply(x: String) -> Bool { return false; } }
locus Other { @effects(is: { apply_it }) fn other(x: String) -> Bool { return true; } }
locus Holder { params { dep: Gate = Noop { }; } }
group organism = { App };
main locus App {
    params { h: Holder = Holder { dep: Noop { } }; }
    claims { gated: forbid reaches(organism, effects(apply_it)); }
    run() { let ok = self.h.dep.apply("x"); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("claim `gated` violated")),
        "no conformer carries the effect: {:?}",
        ds
    );
}

/// The F.20 guarantee survives: an effect behind a slot is seen, and
/// the witness runs through the slot into the conformer.
#[test]
fn effect_behind_a_slot_still_witnessed_through_it() {
    let src = r#"
interface Emitter { fn emit(tag: String) -> Int; }
locus LoudEmitter { params { n: Int = 0; } fn emit(tag: String) -> Int { println("loud: ", tag); return 1; } }
locus Manifest { params { sink: Emitter = LoudEmitter { }; } fn reach(t: String) -> Int { return self.sink.emit(t); } }
@no_syscall
fn certified(m: Manifest) -> Int { return m.reach("x"); }
fn main() { let m = Manifest { }; println(certified(m)); }
"#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("Manifest::reach") && m.contains("LoudEmitter::emit")),
        "witness through the slot: {:?}",
        ds
    );
}
