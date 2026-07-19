//! Lotus codegen targeting LLVM (Phase 3 milestone 0).
//!
//! v0 milestone: lower a tiny subset of lotus to LLVM IR,
//! emit an object file, and link it via clang. The minimum
//! to make `lotus build hello-world/main.hl` produce a
//! runnable executable that prints "hello, world".
//!
//! Subset:
//! - `fn main()` — entry point.
//! - Locus literal `L { ... }` for an ephemeral locus with
//!   only `params` + `birth()`. State is allocated on the
//!   stack (a flat struct); birth() runs inline; dissolve
//!   is a no-op for the simplest case.
//! - String literal params; `self.X` reads.
//! - `println(...)` / `print(...)` builtins. v0 emits one
//!   printf call per argument; the last argument's printf
//!   format string carries the trailing newline for
//!   println.
//!
//! Not yet:
//! - Bus router (no `<-`)
//! - Lifecycle beyond birth()
//! - Closures, modes, accept, contract, perspectives
//! - Recovery primitives, dissolve cascade
//! - Decimal / Int arithmetic beyond literals
//! - Any non-string types past the println pipe
//!
//! These come as the codegen surface fills in. The
//! interpreter remains the reference for everything not yet
//! lowered.

pub(crate) mod bus;
pub(crate) mod channels;
pub mod codegen;
pub(crate) mod form;
pub(crate) mod locus;
pub mod mangle;
pub(crate) mod shared;
pub(crate) mod stdlib;
pub(crate) mod types;

pub use codegen::{
    stdlib_doc_source, stdlib_path_renames,
    build_executable, build_executable_with_imports,
    build_executable_with_options, BuildOptions, CodegenError, CompileTarget,
    DebugSourceFile, DebugSources, TargetCpu,
};
