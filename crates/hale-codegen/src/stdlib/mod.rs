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

pub(crate) mod decimal;
