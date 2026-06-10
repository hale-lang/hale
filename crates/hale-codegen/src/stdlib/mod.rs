//! Stdlib path-call lowering, organized by `std::*` namespace.
//!
//! Each submodule defines a `<Namespace>Stdlib` trait extension on
//! [`crate::codegen::Cx`] carrying the `lower_std_<ns>_*` functions for
//! that namespace. The top-level dispatch in `codegen.rs` brings these
//! traits into scope so call sites keep the `self.lower_std_*(...)`
//! shape.
//!
//! This module exists to keep the codegen monolith navigable; see
//! `notes/refactor-codegen-model-org.md`.

pub(crate) mod bus;
pub(crate) mod bytes;
pub(crate) mod crypto;
pub(crate) mod decimal;
pub(crate) mod env;
pub(crate) mod io_file;
pub(crate) mod io_fs;
pub(crate) mod io_stdin;
pub(crate) mod io_tcp;
pub(crate) mod io_tls;
pub(crate) mod io_udp;
pub(crate) mod math;
pub(crate) mod process;
pub(crate) mod rand;
pub(crate) mod sockopt;
pub(crate) mod str;
pub(crate) mod term;
pub(crate) mod text;
pub(crate) mod time;
