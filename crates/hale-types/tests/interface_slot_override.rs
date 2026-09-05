//! GH #533 (DNA F.11, 2026-09-05): a call through an interface-typed
//! field followed the field's DECLARATION DEFAULT, not the impl the
//! constructor stored. `Holder { dep: Real { } }` over a
//! `dep: Gate = Noop { }` default ran `Real::apply` and
//! `forbid reaches(.., effects(apply_it))` passed — fail-open on the
//! constructor-shaped assembly #521 is built from; the mirror was a
//! false positive. The slot now keeps its declared interface and the
//! dispatch rewrite fans to every conformer, the rule a one-hop slot
//! and an interface-typed fn param already followed.

use hale_syntax::parse_source;
use hale_types::check_program;

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

/// Default the carrier, override harmless: the conservative answer is
/// STILL a refusal — every conformer is a possible callee of the slot
/// and the engine never claims absence it cannot prove. What changes
/// is that the verdict no longer depends on which literal happens to
/// be the default.
#[test]
fn default_carrier_with_harmless_override_is_still_refused_conservatively() {
    let ds = diags(&program("Real", "Noop"));
    assert!(
        ds.iter().any(|m| m.contains("claim `gated` violated")),
        "conformer fan-out keeps the carrier reachable: {:?}",
        ds
    );
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
