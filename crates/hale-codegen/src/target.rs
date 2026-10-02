//! The target model lives in `hale_types::target` (F.40 phase 3, P3):
//! it is plain data with no LLVM in it, and the capability matrix
//! (`hale_types::capability`) derives its target classes from it, so
//! it sits below the checker rather than inside the backend. Every
//! `hale_codegen::target::*` path keeps compiling through this
//! re-export; `TargetSpec` is the same type on both sides.

pub use hale_types::target::*;
