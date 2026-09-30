//! The one desugar sequence before the check (F.40 phase 2.1b).
//!
//! [`desugar_before_check`] is what every entry point runs over the
//! program it loaded, after its own prefix passes and before it mints
//! the snapshot: the declaration-shaping rewrites that change how a
//! declaration is spelled without erasing a reference the checker's
//! laws read. The checker, the model and lowering then see one program
//! shape, so no consumer has to know the second spelling.
//!
//! In order:
//!
//! 1. unit returns: `-> ()` is "no return type" on every fn-shaped
//!    declaration.
//! 2. construction aliases: a struct literal, variant path or
//!    constructor pattern spelled with a type alias names the
//!    declaration the alias chain ends at
//!    ([`crate::mangle::resolve_construction_aliases`]), over the
//!    whole bundle at once.
//!
//! The bundled stdlib goes through the same passes ([`bundled_stdlib`])
//! before the resolved program appends it.

use std::sync::OnceLock;

use hale_syntax::ast::{LocusMember, Program, TopDecl, TypeExpr};

/// What the sequence is run with: the inputs its passes read besides
/// the programs themselves.
pub struct Sequence<'a> {
    /// The build's cross-seed rename table (`alias::Name` → mangled).
    pub import_renames: &'a [(Vec<String>, String)],
}

/// Run the sequence over every program of a bundle, in place.
///
/// Idempotent: a program the sequence already shaped comes back
/// unchanged, so a caller that cannot tell whether its program went
/// through it (the test harness) may run it again.
pub fn desugar_before_check(programs: &mut [&mut Program], seq: &Sequence<'_>) {
    // The bundled stdlib is what a bundle-wide pass reads besides the
    // bundle: the declarations an alias may end at. A stdlib that does
    // not parse is reported where it is appended (`resolve_program`);
    // here the passes run without it.
    let stdlib = bundled_stdlib().ok();
    shape(programs, seq, stdlib.as_slice());
}

/// The passes, in order. `context` is read and never rewritten.
fn shape(programs: &mut [&mut Program], seq: &Sequence<'_>, context: &[&Program]) {
    for p in programs.iter_mut() {
        normalize_unit_return_annotations(&mut p.items);
    }
    crate::mangle::resolve_construction_aliases(programs, context, seq.import_renames);
}

/// The bundled stdlib, parsed once and put through the sequence: what
/// the resolved program appends, and what the checker's analyses read
/// as the stdlib's bodies and declarations
/// ([`crate::stdlib_bodies::program`]), so both see one shape. Its own
/// declarations only: no rename table reaches it. The error is the
/// parse failure, rendered.
pub fn bundled_stdlib() -> Result<&'static Program, String> {
    static SHAPED: OnceLock<Result<Program, String>> = OnceLock::new();
    SHAPED
        .get_or_init(|| {
            let mut stdlib =
                hale_syntax::parse_source(hale_stdlib::AP_SOURCE).map_err(|diags| {
                    let summary = diags
                        .iter()
                        .map(|d| format!("{:?}", d))
                        .collect::<Vec<_>>()
                        .join("; ");
                    format!("stdlib parse: {}", summary)
                })?;
            shape(&mut [&mut stdlib], &Sequence { import_renames: &[] }, &[]);
            Ok(stdlib)
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// `-> ()` is spelled unit: rewrite an empty-tuple return annotation to
/// "no return type" on every fn-shaped declaration, so no signature
/// consumer (the checker's, lowering's) ever sees a 0-element tuple in
/// a return position. A `()` elsewhere (a fn TYPE's return, a type
/// argument) is still a type the checker resolves to `Unit`.
fn normalize_unit_return_annotations(items: &mut [TopDecl]) {
    fn norm(ret: &mut Option<TypeExpr>) {
        if matches!(ret, Some(TypeExpr::Tuple(parts, _)) if parts.is_empty()) {
            *ret = None;
        }
    }
    for item in items {
        match item {
            TopDecl::Fn(f) => norm(&mut f.ret),
            TopDecl::Interface(i) => {
                for m in &mut i.methods {
                    norm(&mut m.ret);
                }
            }
            TopDecl::Locus(l) => {
                for member in &mut l.members {
                    match member {
                        LocusMember::Fn(f) => norm(&mut f.ret),
                        LocusMember::Mode(md) => norm(&mut md.ret),
                        LocusMember::Lifecycle(lc) => norm(&mut lc.ret),
                        _ => {}
                    }
                }
            }
            TopDecl::Module(m) => normalize_unit_return_annotations(&mut m.items),
            _ => {}
        }
    }
}
