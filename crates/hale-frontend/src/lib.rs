//! The shared frontend (F.40 phase 2, RFC #1212): which source units
//! form a program — the entry, every seed its `import`s reach, their
//! merge order and the spans' virtual bases — for every entry point.
//!
//! The CLI's verbs and the LSP load through this crate. It links no
//! LLVM: it depends on the parser and the checker's crate, never on
//! `hale-codegen` or on its consumer `hale-lsp`
//! (`crates/hale-graph/tests/architecture.rs` holds that).
//!
//! - [`frontend`]: the loaders — [`frontend::parse_with_imports`] for
//!   a file entry, [`frontend::collect_checkable`] for a check target,
//!   the file collection and merge beneath them, the source map.
//! - [`imports`]: import resolution, the seed cache, the mangling
//!   drivers and alias scoping.
//! - [`workspace`]: the workspace root and the seed walk.
//! - [`diag`]: the located and `--json` renderers the loaders' failures
//!   travel through.
//! - [`source`]: where the loaders read from — the disk for the CLI,
//!   the editor's buffers over the disk for the LSP.

pub mod diag;
pub mod frontend;
pub mod imports;
pub mod source;
pub mod workspace;
