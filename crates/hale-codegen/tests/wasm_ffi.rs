//! The `wasm_ffi` integration-test binary: 5 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "ffi_basic.rs"]
mod ffi_basic;
#[path = "ffi_string_return.rs"]
mod ffi_string_return;
#[path = "shadow_capability_lowering.rs"]
mod shadow_capability_lowering;
#[path = "wasm_target.rs"]
mod wasm_target;
#[path = "ws1_ffi_handle_reassign.rs"]
mod ws1_ffi_handle_reassign;
