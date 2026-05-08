//! Lotus runtime — Phase 2.
//!
//! v0 cut: a tree-walking interpreter that runs parsed +
//! typechecked Lotus programs. Region allocator, cooperative
//! scheduler, and bus router come later in Phase 2; the
//! interpreter is the "is the language semantically real"
//! check that doesn't wait on codegen.
//!
//! Public surface:
//! - [`run_program`] / [`run_bundle`] — execute a parsed Program
//!   (or set of programs) starting from `fn main()`.

pub mod builtins;
pub mod bus;
pub mod env;
pub mod eval;
pub mod value;

pub use eval::{run_bundle, run_program};
pub use value::Value;
