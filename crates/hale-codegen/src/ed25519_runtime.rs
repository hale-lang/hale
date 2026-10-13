//! Ed25519 as part of the runtime, for a program that signs or verifies
//! with `std::crypto::ed25519_*`.
//!
//! Monocypher 4.0.3 (Loup Vaillant et al., BSD-2-Clause or CC0-1.0) is
//! vendored unmodified under `runtime/third_party/monocypher`: the core
//! (`monocypher.c` / `.h`, whose field and group arithmetic the Ed25519
//! of RFC 8032 is built on) and its optional SHA-512 Ed25519
//! (`monocypher-ed25519.c` / `.h`), from the release tarball
//! `monocypher-4.0.3.tar.gz` whose SHA-512 is the one monocypher.org
//! publishes beside it (`40904ada…880f8965`). 4.0.3 is the first release
//! without the EdDSA timing leak upstream lists for 4.0.2 and earlier
//! (a compiler that turns `fe_cswap` / `fe_ccopy`'s mask into a branch
//! or a pointer swap; fixed with a `volatile` mask and an unrolled copy,
//! monocypher.org/bugs), so a later bump keeps that floor. It is plain C99 with no
//! libc call, no thread and no I/O, and `runtime/lotus_ed25519.c` is the
//! glue: bytes in, bytes out.
//!
//! The three translation units are compiled by the runtime's cc step with
//! its flags (the sanitizers' included), each once per flag set into the
//! content-addressed object cache, and linked only into a program that
//! reaches `lotus_ed25519_*`, as nghttp2 is; a cross build compiles them
//! with the target's cc, and the wasm32 link compiles them for wasm32 (no
//! libc is reached, so the freestanding toolchain builds them as they
//! are). The sources are embedded as text, so a unit's quoted `#include`
//! of a vendored header is replaced by that header's text when the unit
//! is assembled — what the preprocessor would read, with the headers'
//! own include guards — and no include directory is staged. The core and
//! the Ed25519 file stay two units: each has `static` helpers of the same
//! names.

use crate::codegen::{compile_cached_runtime_object_with, BuildOptions, CodegenError};
use std::path::PathBuf;

const MONOCYPHER_H: &str = include_str!("../runtime/third_party/monocypher/monocypher.h");
const MONOCYPHER_C: &str = include_str!("../runtime/third_party/monocypher/monocypher.c");
const ED25519_H: &str = include_str!("../runtime/third_party/monocypher/monocypher-ed25519.h");
const ED25519_C: &str = include_str!("../runtime/third_party/monocypher/monocypher-ed25519.c");
/// The glue: the whole C surface `std::crypto::ed25519_*` lowers to.
const GLUE: &str = include_str!("../runtime/lotus_ed25519.c");

/// Whether this C symbol is the Ed25519 surface's, so the link needs the
/// objects.
pub(crate) fn is_ed25519_symbol(name: &str) -> bool {
    name.starts_with("lotus_ed25519_")
}

/// `source` with each line `#include "<header>"` of a vendored header
/// replaced by the header's text, recursively (the Ed25519 header
/// includes the core's).
fn inline_includes(source: &str) -> String {
    let headers: [(&str, &str); 2] = [("monocypher.h", MONOCYPHER_H), ("monocypher-ed25519.h", ED25519_H)];
    let mut out = String::with_capacity(source.len());
    for line in source.split_inclusive('\n') {
        let included = headers
            .iter()
            .find(|(name, _)| line.trim() == format!("#include \"{name}\""));
        match included {
            Some((_, text)) => {
                out.push_str(&inline_includes(text));
                out.push('\n');
            }
            None => out.push_str(line),
        }
    }
    out
}

/// The three translation units, assembled: `(stem, source)`.
pub(crate) fn units() -> Vec<(&'static str, String)> {
    vec![
        ("ed25519-monocypher", inline_includes(MONOCYPHER_C)),
        ("ed25519-monocypher-ed25519", inline_includes(ED25519_C)),
        ("ed25519-glue", inline_includes(GLUE)),
    ]
}

/// The objects, compiled (or taken from the cache) with `cflags`.
pub(crate) fn ed25519_objects(
    options: &BuildOptions,
    cc: &[String],
    cc_version: &str,
    cflags: &[String],
) -> Result<Vec<PathBuf>, CodegenError> {
    units()
        .iter()
        .map(|(stem, source)| compile_cached_runtime_object_with(options, cc, cc_version, source, stem, cflags))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every quoted include of a vendored header is replaced, and no other
    /// quoted include is left for the preprocessor to look for.
    #[test]
    fn every_unit_is_self_contained() {
        for (stem, source) in units() {
            for line in source.lines() {
                assert!(!line.trim_start().starts_with("#include \""), "{stem}: {line}");
            }
            assert!(source.contains("MONOCYPHER_H"), "{stem}: the core header is in");
        }
    }
}
