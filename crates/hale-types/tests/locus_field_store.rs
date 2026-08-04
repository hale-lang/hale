//! A locus-typed field may only be assigned a locus LITERAL
//! (GH #383 — the ownership decision that closes the factory-return
//! leak).
//!
//! `self.conn = Connection { url: next };` is a documented lifecycle
//! event: break-before-make, the new instance built directly into
//! this locus's arena, unambiguously owned by the field. But
//! `self.held = make_row(...)` stores a locus somebody else built,
//! and nothing in the language says who owns it afterwards — the
//! field, or the frame that produced it.
//!
//! Today that question is dodged by never freeing: a locus returned
//! from a free fn is routed to a program-lifetime arena (m90), so
//! the field's pointer stays valid because nothing reclaims it.
//! **The leak is the safety mechanism** — the same shape as the
//! synced-map clone-on-read and the form-vec zero-reads, and the
//! reason every attempt to give those loci a real lifetime produced
//! either a use-after-free or silently wrong values (four measured
//! attempts on #383).
//!
//! Forbidding the store settles it: with no way for a factory result
//! to escape into a field, its owner is the binding that named it,
//! and ordinary scope-exit teardown is correct. Same principle the
//! language already applies to locus returns from methods (CQRS /
//! no-locus-return): a locus is structure, not a value to hand
//! around.
//!
//! Surveyed before adopting: **zero occurrences** across 60 bundles
//! in five downstream repositories, so this forbids a pattern nobody
//! writes — which is what made the restriction (rather than a silent
//! deep copy) the right call.

use hale_syntax::parse_source;
use hale_types::check_program;

fn msgs(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

fn fires(src: &str) -> bool {
    msgs(src).iter().any(|m| m.contains("ownership would be ambiguous"))
}

#[test]
fn assigning_a_factory_result_into_a_locus_field_is_rejected() {
    let src = r#"
        @form(vec)
        locus Row { params { tag: Int = 0; } capacity { heap data of Float; } }

        fn make_row(tag: Int) -> Row {
            let r = Row { tag: tag };
            r.push(1.0);
            return r;
        }

        locus Holder {
            params { held: Row = Row { }; }
            fn fill(t: Int) { self.held = make_row(t); }
        }

        fn main() { let h = Holder { }; h.fill(1); }
    "#;
    assert!(fires(src), "expected the diagnostic:\n{:#?}", msgs(src));
    let m = msgs(src)
        .into_iter()
        .find(|m| m.contains("ownership would be ambiguous"))
        .expect("checked above");
    // The message must teach the remedy, not just refuse: this is a
    // restriction people will meet without context.
    assert!(m.contains("LITERAL"), "must name the allowed form: {}", m);
    assert!(m.contains("accept("), "must offer accept(): {}", m);
}

#[test]
fn assigning_a_let_bound_locus_into_a_field_is_rejected() {
    // Same ambiguity via a local binding rather than a direct call.
    let src = r#"
        locus Conn { params { url: String = ""; } }
        locus Server {
            params { conn: Conn = Conn { }; }
            fn swap(u: String) {
                let c = Conn { url: u };
                self.conn = c;
            }
        }
        fn main() { let s = Server { }; s.swap("x"); }
    "#;
    assert!(fires(src), "expected the diagnostic:\n{:#?}", msgs(src));
}

// === negative controls ==========================================

/// The documented reassignment lifecycle event must stay legal —
/// this is the shape `docs/services/lifecycle.md` teaches.
#[test]
fn literal_reassignment_stays_legal() {
    let src = r#"
        locus Conn { params { url: String = ""; } }
        locus Server {
            params { conn: Conn = Conn { }; n: Int = 0; }
            fn reconnect(next: String) {
                self.conn = Conn { url: next };
                self.n = self.n + 1;
            }
        }
        fn main() { let s = Server { }; s.reconnect("x"); println(s.n); }
    "#;
    assert!(!fires(src), "literal reassign must be clean:\n{:#?}", msgs(src));
}

#[test]
fn non_locus_fields_are_untouched() {
    let src = r#"
        type Rec { a: Int; }
        fn make_rec(a: Int) -> Rec { return Rec { a: a }; }
        fn label(i: Int) -> String { return "n" + to_string(i); }

        locus Holder {
            params { r: Rec = Rec { a: 0 }; s: String = ""; n: Int = 0; }
            fn fill(i: Int) {
                self.r = make_rec(i);      // struct value — fine
                self.s = label(i);         // String value — fine
                self.n = i;
            }
        }
        fn main() { let h = Holder { }; h.fill(1); println(h.n); }
    "#;
    assert!(
        !fires(src),
        "only LOCUS-typed fields are restricted:\n{:#?}",
        msgs(src)
    );
}

/// Ordinary locals of locus type are unaffected — the restriction is
/// about *field* storage, which is what creates the second owner.
#[test]
fn locus_locals_are_unaffected() {
    let src = r#"
        locus Row { params { tag: Int = 0; } }
        fn make_row(t: Int) -> Row { let r = Row { tag: t }; return r; }
        locus User {
            params { n: Int = 0; }
            fn use_it(t: Int) -> Int {
                let r = make_row(t);
                self.n = r.tag;
                return r.tag;
            }
        }
        fn main() { let u = User { }; println(u.use_it(3)); }
    "#;
    assert!(
        !fires(src),
        "a let-bound factory result is the OWNED shape and must stay \
         legal — it is what the restriction makes sound:\n{:#?}",
        msgs(src)
    );
}
