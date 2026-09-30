//! Code more than one verb runs: what `main.rs` used to hold beside the
//! verbs (C5 of the refactor review). Each module here is a pure move of
//! functions that keep their bodies; a verb reaches them by path.
//!
//! The loaders — `frontend` and `imports`, and the LLVM-free halves of
//! `diag` and `workspace` — live in `hale-frontend` since F.40 phase
//! 2.1a, shared with the LSP; they are re-exported here under the paths
//! the verbs already use.

pub(crate) mod diag;
pub(crate) use hale_frontend::frontend;
pub(crate) use hale_frontend::imports;
pub(crate) mod options;
pub(crate) mod process;
pub(crate) use hale_frontend::source;
pub(crate) mod stale;
pub(crate) mod workspace;
