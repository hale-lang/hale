//! The `numeric` integration-test binary: 13 test files of this area, kept
//! where they are (their paths, names and history unchanged) and built as
//! modules of one binary, so the crate links once for the area instead of
//! once per file. Each file is a module; a test's name is `<file>::<fn>`.
//! A new test file joins an area by a line here (and is refused by
//! `every_test_file_is_built` until it does).

#[path = "decimal_alignment.rs"]
mod decimal_alignment;
#[path = "decimal_format.rs"]
mod decimal_format;
#[path = "duration_scalar_arith.rs"]
mod duration_scalar_arith;
#[path = "float_to_int_cast.rs"]
mod float_to_int_cast;
#[path = "int_bitwise_ops.rs"]
mod int_bitwise_ops;
#[path = "k_max_builtin.rs"]
mod k_max_builtin;
#[path = "math_and_int_float.rs"]
mod math_and_int_float;
#[path = "math_nan_inf.rs"]
mod math_nan_inf;
#[path = "os_getrandom.rs"]
mod os_getrandom;
#[path = "rand_primitives.rs"]
mod rand_primitives;
#[path = "struct_layout_with_i128.rs"]
mod struct_layout_with_i128;
#[path = "time_now.rs"]
mod time_now;
#[path = "time_return.rs"]
mod time_return;
#[path = "unit_conversion_lowering.rs"]
mod unit_conversion_lowering;
#[path = "unit_quantity_lowering.rs"]
mod unit_quantity_lowering;
#[path = "unit_dialect_unsupported.rs"]
mod unit_dialect_unsupported;
#[path = "unit_fallible.rs"]
mod unit_fallible;
