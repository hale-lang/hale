//! A locus holding a `sync`-bearing form is an effect surface
//! (#340). No annotation: the lock is a property of the form's own
//! declaration, so holding one is inferred from structure.
//!
//! `sync = serialized` is a per-map mutex. Any call reaching it can
//! take that lock — regardless of placement, and regardless of
//! whether anyone ever shares the containing locus. Three contracts
//! were false across that boundary.

use hale_syntax::parse_source;

fn diags(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

const REG: &str = "\
type E { k: Int; v: Int; }
@form(hashmap, sync = serialized)
locus Counts { capacity { pool entries of E indexed_by k; } }
locus Registry {
    params { store: Counts = Counts { }; }
    fn record(e: E) { self.store.set(e); }
    fn read(k: Int) -> Int { let e = self.store.get(k) or E { k: 0, v: 0 }; return e.v; }
}
";

/// `sync = serialized` is a per-map mutex. Acquiring one waits on
/// another thread, which is what `block` means — so certifying it as
/// non-blocking was a false hot-path certificate.
#[test]
fn no_block_catches_reaching_a_shared_locus() {
    let src = format!(
        "{REG}
locus W {{ params {{ r: Registry = Registry {{ }}; }}
    @no_block
    fn hot() {{ self.r.record(E {{ k: 1, v: 1 }}); }} }}
main locus App {{ params {{ r: Registry = Registry {{ }}; w: W = W {{ r: self.r }}; }} }}
fn main() {{ App {{ }}; }}"
    );
    let ds = diags(&src);
    assert!(
        ds.iter().any(|m| m.contains("must not reach `block`")),
        "a mutex acquisition must not certify as non-blocking: {:?}",
        ds
    );
    assert!(
        ds.iter().any(|m| m.contains("sync")),
        "and the witness should name why: {:?}",
        ds
    );
}

/// Another pool can change the value between two calls with identical
/// arguments, so the result is not a function of the inputs. Same
/// distinction the docs draw between `monotonic_ns()` and
/// `time_from_unix(n)`.
#[test]
fn deterministic_catches_a_shared_read() {
    let src = format!(
        "{REG}
locus W {{ params {{ r: Registry = Registry {{ }}; }}
    @deterministic
    fn pure_ish(k: Int) -> Int {{ return self.r.read(k); }} }}
main locus App {{ params {{ r: Registry = Registry {{ }}; w: W = W {{ r: self.r }}; }} }}
fn main() {{ App {{ }}; }}"
    );
    let ds = diags(&src);
    assert!(
        ds.iter()
            .any(|m| m.contains("not a function of this call's inputs")),
        "a shared read must defeat @deterministic: {:?}",
        ds
    );
}

/// A shared locus is an input channel with no bus edge, so a
/// `depends:` closure over the message graph cannot be complete.
#[test]
fn depends_reports_the_shared_channel_it_cannot_close_over() {
    let src = format!(
        "{REG}
topic Ask {{ payload: E; }}
@effects(depends: {{Ask}})
locus Reader {{ params {{ reg: Counts = Counts {{ }}; seen: Int = 0; }}
    bus {{ subscribe Ask as on_ask; }}
    fn on_ask(e: E) {{ self.reg.set(e); self.seen = e.v; }} }}
locus P {{ bus {{ publish Ask; }} fn go() {{ Ask <- E {{ k: 1, v: 1 }}; }} }}
main locus App {{ params {{
    d: Reader = Reader {{ }}; p: P = P {{ }}; }} }}
fn main() {{ App {{ }}; }}"
    );
    let ds = diags(&src);
    assert!(
        ds.iter().any(|m| m.contains("outside the bus graph")),
        "depends: must not claim completeness it cannot have: {:?}",
        ds
    );
}

/// The attribution must be specific to `@shared`, or every ordinary
/// locus method becomes blocking and nothing can be certified.
#[test]
fn an_ordinary_locus_call_is_still_non_blocking() {
    let ds = diags(
        "locus Plain { params { n: Int = 0; } fn get() -> Int { return self.n; } }\n\
         locus W { params { p: Plain = Plain { }; }\n\
           @no_block @deterministic\n\
           fn hot() -> Int { return self.p.get(); } }\n\
         main locus App { params { w: W = W { }; } }\n\
         fn main() { App { }; }",
    );
    assert!(
        !ds.iter().any(|m| m.contains("must not reach")),
        "ordinary locus calls must stay certifiable: {:?}",
        ds
    );
}
