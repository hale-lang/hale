//! One module per verb (C5 of the refactor review). Each is a pure move of
//! the functions `main.rs` used to hold for it; `main.rs` keeps the dispatch.

pub(crate) mod bench;
pub(crate) mod build;
pub(crate) mod check;
pub(crate) mod doc;
pub(crate) mod fleet;
pub(crate) mod fmt;
pub(crate) mod help;
pub(crate) mod init;
pub(crate) mod model;
pub(crate) mod replay;
pub(crate) mod run;
pub(crate) mod test;
