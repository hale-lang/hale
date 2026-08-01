//! One locus instance shared by two differently-placed towers (#333,
//! the `InstanceId` half of #334).
//!
//! F.31 keeps a locus's methods on its own pool's thread, but reasons
//! per FIELD DECLARATION: for `self.s.bump()` inside `WorkerA`, `s`
//! inherits WorkerA's pool, and WorkerB reasons identically about its
//! own `s`. Neither is wrong about its own field — they are wrong
//! about each other, and nothing related the two declarations back to
//! the single object they both name.
//!
//! Measured consequence before this: two pinned workers each doing
//! 100k increments on one shared locus produced ~140k of 200k, with
//! `hale check` reporting `ok`.

use hale_syntax::error::DiagKind;
use hale_syntax::parse_source;

fn diags(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

const SHARED: &str = "\
locus Shared { params { n: Int = 0; } fn bump() { self.n = self.n + 1; } }
locus A { params { s: Shared = Shared { }; } run() { self.s.bump(); } }
locus B { params { s: Shared = Shared { }; } run() { self.s.bump(); } }
";

#[test]
fn aliasing_into_two_pools_is_reported() {
    let src = format!(
        "{SHARED}
main locus App {{
    params {{ sh: Shared = Shared {{ }};
             a: A = A {{ s: self.sh }};
             b: B = B {{ s: self.sh }}; }}
    placement {{ a: pinned(core = 0); b: pinned(core = 1); }}
}}
fn main() {{ App {{ }}; }}"
    );
    let ds = diags(&src);
    let hit = ds
        .iter()
        .find(|m| m.contains("is shared by"))
        .unwrap_or_else(|| panic!("cross-pool aliasing must be reported: {:?}", ds));
    assert!(
        hit.contains("self.sh") && hit.contains('a') && hit.contains('b'),
        "the report must name the instance and both holders: {}",
        hit
    );
}

/// Sharing within ONE pool is ordinary composition — handler
/// serialization already orders the accesses.
#[test]
fn aliasing_within_one_pool_is_fine() {
    let src = format!(
        "{SHARED}
main locus App {{
    params {{ sh: Shared = Shared {{ }};
             a: A = A {{ s: self.sh }};
             b: B = B {{ s: self.sh }}; }}
}}
fn main() {{ App {{ }}; }}"
    );
    assert!(
        !diags(&src).iter().any(|m| m.contains("is shared by")),
        "same-pool sharing must not be reported"
    );
}

#[test]
fn a_single_holder_is_fine_however_it_is_placed() {
    let src = format!(
        "{SHARED}
main locus App {{
    params {{ sh: Shared = Shared {{ }}; a: A = A {{ s: self.sh }}; }}
    placement {{ a: pinned(core = 0); }}
}}
fn main() {{ App {{ }}; }}"
    );
    assert!(
        !diags(&src).iter().any(|m| m.contains("is shared by")),
        "one holder cannot race with itself"
    );
}

/// A WARNING, not an error. The sanctioned way to share across pools
/// is a `@form(..., sync = ...)` locus, and a plain locus whose
/// mutable state sits entirely behind such fields is a legitimate
/// design — two applications in a downstream fleet do exactly that.
/// Distinguishing those needs the declared shared-locus surface
/// discussed on #333; until then this reports without failing builds.
#[test]
fn the_report_does_not_fail_the_build() {
    let src = format!(
        "{SHARED}
main locus App {{
    params {{ sh: Shared = Shared {{ }};
             a: A = A {{ s: self.sh }};
             b: B = B {{ s: self.sh }}; }}
    placement {{ a: pinned(core = 0); b: pinned(core = 1); }}
}}
fn main() {{ App {{ }}; }}"
    );
    let program = parse_source(&src).expect("parse");
    let ds = hale_types::check_program(&program);
    assert!(
        ds.iter().any(|d| d.message.contains("is shared by")),
        "the aliasing should still be surfaced"
    );
    assert!(
        !ds.iter().any(|d| {
            d.message.contains("is shared by")
                && matches!(d.kind, DiagKind::Type)
        }),
        "but as a warning, not a type error"
    );
}
