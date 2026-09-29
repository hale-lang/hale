//! Code more than one verb runs: what `main.rs` used to hold beside the
//! verbs (C5 of the refactor review). Each module here is a pure move of
//! functions that keep their bodies; a verb reaches them by path.

pub(crate) mod diag;
pub(crate) mod options;
pub(crate) mod process;
pub(crate) mod stale;
pub(crate) mod workspace;
