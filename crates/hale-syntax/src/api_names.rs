//! GH #1417: the names the surface path recognises by spelling, and the
//! parse space the compiler's generated source lives in.

use crate::ast::TypeExpr;

/// The parse space of generated source (rpc and hub expansion): every
/// span of a generated item starts here, past any file of a bundle, so a
/// diagnostic about it is rendered as such rather than at a position in
/// whatever file the bundle lists first.
pub const API_SYNTH_BASE: u32 = 0x7000_0000;

/// The parse space of the JSON codecs `json_gen` writes (a record's
/// `__json_parse_<T>` / `__json_to_json_<T>`, an api surface's
/// `__api_decode_<T>` / `__api_encode_<T>` and their protobuf twins): past
/// any file of a bundle and below [`API_SYNTH_BASE`], each generated
/// chunk at its own offset beyond the last one (`json_gen`'s
/// `next_synth_base`).
///
/// They were parsed at offset 0, where their spans coincided with the
/// first files' offsets without being their text. Every table keyed by a
/// span (the owned-view pass's copies and sites, the allocation survey's
/// lookup of them) then answered for a generated statement with what it
/// had found at the user's statement of the same offsets: a store in one
/// file, written once, was a copy "in a loop" of a generated decoder, and
/// whether it was depended on the length of an unrelated constant
/// earlier in the bundle.
pub const JSON_SYNTH_BASE: u32 = 0x6000_0000;

/// Whether `off` lies in the JSON codecs' parse space
/// ([`JSON_SYNTH_BASE`]).
pub fn is_json_synth_pos(off: u32) -> bool {
    (JSON_SYNTH_BASE..API_SYNTH_BASE).contains(&off)
}

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
