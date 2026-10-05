use std::collections::BTreeMap;
use std::path::PathBuf;

// The renderers live in `hale-frontend` (F.40 phase 2.1a), which the
// LSP shares and which links no LLVM; only the one that renders a
// `CodegenError` stays here, beside the crate that defines it.
pub(crate) use hale_frontend::diag::*;

/// GH #848: one spelling of a failed compile, for every command that
/// compiles.
///
/// A `CodegenError` that carries a span renders like any other
/// finding — `path:line:col: codegen error: message` with the
/// offending source line and a caret — through the same
/// `render_located` the checker's diagnostics take, so the position
/// is un-shifted by the file's virtual base and the path is the
/// canonical one (GH #775, GH #822). One that carries no span renders
/// as the bare `codegen error: <message>` line.
///
/// Only `build` did this (GH #241, extended to the missing-shim
/// refusal by GH #808). `run`, `test`, `bench` and `replay` printed
/// the error with `{:?}`, so the span reached the user as
/// `Span { start: Pos(55), end: Pos(60) }` and the message arrived
/// wrapped in a Rust variant name. Text only: these commands have no
/// machine-readable channel — `check --json` is its own reporting
/// path and is unaffected.
pub(crate) fn render_codegen_error(
    e: &hale_codegen::CodegenError,
    file_bases: &[(u32, PathBuf, u32)],
    sources: &BTreeMap<PathBuf, String>,
) -> String {
    // GH #808: the missing-tree-sitter-shim refusal carries a span
    // only when the program reached `std::ts::*` through a path call
    // we could point at; otherwise it renders as a bare line like the
    // rest.
    let located = match e {
        hale_codegen::CodegenError::UnsupportedAt(msg, span) => {
            Some((msg.clone(), *span))
        }
        hale_codegen::CodegenError::MissingTsShim(msg, Some(span)) => {
            Some((msg.clone(), *span))
        }
        hale_codegen::CodegenError::CapabilityRefused(msg, Some(span)) => {
            Some((msg.clone(), *span))
        }
        _ => None,
    };
    match located {
        Some((msg, span)) => render_located(
            &hale_syntax::Diag::codegen(span, msg),
            file_bases,
            sources,
        ),
        None => format!("codegen error: {}", e),
    }
}

/// A snapshot family a build demanded and did not get, rendered as a
/// compiling command reports it: the errors that blocked it, located,
/// or its producer's own refusal as the codegen error it is (the
/// lowering view's `resolve_rewritten`). One line each, joined.
pub(crate) fn render_blocked(
    b: &hale_frontend::snapshot::Blocked,
    file_bases: &[(u32, PathBuf, u32)],
    sources: &BTreeMap<PathBuf, String>,
) -> String {
    let mut lines: Vec<String> =
        b.because.iter().map(|d| render_located(d, file_bases, sources)).collect();
    if let Some(msg) = &b.refused {
        lines.push(render_codegen_error(
            &hale_codegen::CodegenError::Unsupported(msg.clone()),
            file_bases,
            sources,
        ));
    }
    lines.join("\n")
}
