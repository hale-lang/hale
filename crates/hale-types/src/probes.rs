//! GH #1477 S1: the runtime's probes, a main-locus param of type
//! `std::observe::Probes` (spec/semantics.md § Probes). The compiler
//! recognizes the type: one such param, on the main locus, built by its
//! default; a construction of the main locus does not supply it.

/// The stdlib locus `std::observe::Probes` is.
pub const PROBES_LOCUS: &str = "__StdObserveProbes";

/// Whether a declared type names it, spelled either way.
pub fn is_probes_type(te: &hale_syntax::ast::TypeExpr) -> bool {
    match te {
        hale_syntax::ast::TypeExpr::Named { path, .. } => {
            let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
            segs == ["std", "observe", "Probes"] || segs == [PROBES_LOCUS]
        }
        _ => false,
    }
}

/// Whether a param is the probes: declared of the type, or (its type
/// left to its default) built by a literal of it.
pub fn is_probes_param(p: &hale_syntax::ast::ParamDecl) -> bool {
    use hale_syntax::ast::{Expr, ParamInit};
    match (&p.ty, &p.init) {
        (Some(t), _) => is_probes_type(t),
        (None, ParamInit::Value(Expr::Struct { path, .. })) => {
            let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
            segs == ["std", "observe", "Probes"] || segs == [PROBES_LOCUS]
        }
        _ => false,
    }
}
