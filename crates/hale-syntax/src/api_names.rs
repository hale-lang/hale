//! GH #1417: the names the surface path recognises by spelling, and the
//! parse space the compiler's generated source lives in.

use crate::ast::TypeExpr;

/// The parse space of generated source (rpc and hub expansion): every
/// span of a generated item starts here, past any file of a bundle, so a
/// diagnostic about it is rendered as such rather than at a position in
/// whatever file the bundle lists first.
pub const API_SYNTH_BASE: u32 = 0x7000_0000;

/// `std::api::ServedContext`, in either of its spellings: the context a
/// handler of a served surface may declare instead of `Context`, which
/// also names the exposure and the generation the call was admitted under
/// (the runtime's `api_rpc.hl`).
pub fn is_served_context_type(te: &TypeExpr) -> bool {
    match te {
        TypeExpr::Named { path, generic_args, .. } if generic_args.is_empty() => {
            let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
            segs == ["std", "api", "ServedContext"] || segs == ["__StdApiServedContext"]
        }
        _ => false,
    }
}

/// `std::api::Context`, in either of its spellings.
pub fn is_context_type(te: &TypeExpr) -> bool {
    match te {
        TypeExpr::Named { path, generic_args, .. } if generic_args.is_empty() => {
            let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
            segs == ["std", "api", "Context"] || segs == ["__StdApiContext"]
        }
        _ => false,
    }
}
