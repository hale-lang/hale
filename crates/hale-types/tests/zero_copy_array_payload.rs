//! `where zero_copy` + fixed-size array payload fields.
//!
//! 2026-07-01 inline fixed arrays: codegen lays a SCALAR-element
//! `[T; N]` field out INLINE in its containing struct
//! (`llvm_field_storage_type` → `[N x T]`), so the element bytes are
//! part of the value's own layout and a raw memcpy across a
//! zero-copy / shm boundary carries them correctly. The
//! `is_flat_shapeable` predicate accepts scalar arrays to match —
//! the bench `xproc` large-payload can be the idiomatic
//! `type Blob { tag: Int; data: [Int; 511]; }` instead of 512
//! hand-spelled scalar fields (verified cross-process 2026-07-01).
//!
//! NON-scalar element arrays keep the out-of-line pointer layout and
//! must still be REJECTED — a memcpy would share a pointer that
//! dangles in the reader's address space (the original cross-process
//! segfault this test was born from).

use hale_syntax::parse_source;
use hale_types::check_program;

fn check(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

fn program(payload_fields: &str) -> String {
    format!(
        r#"
type Blob {{ {payload_fields} }}
topic Frame {{ payload: Blob; subject: "frame"; }}
locus Reader {{
    bus {{ subscribe Frame as on_frame; }}
    fn on_frame(b: Blob) {{ println(b.tag); }}
}}
main locus App {{
    bindings {{
        Frame: shm_ring("/zc-array", slot_count: 16, on_overflow: drop) where zero_copy;
    }}
}}
fn main() {{ App {{ }}; Reader {{ }}; }}
"#
    )
}

#[test]
fn zero_copy_scalar_array_field_payload_is_accepted() {
    // Scalar-element fixed arrays are inline (2026-07-01) — flat.
    let msgs = check(&program("tag: Int; data: [Int; 8];"));
    assert!(
        !msgs.iter().any(|m| m.contains("flat-shapeable")),
        "a scalar [Int; 8] field in a zero_copy payload is inline and \
         must be accepted; got: {:?}",
        msgs
    );
}

#[test]
fn zero_copy_string_array_field_payload_is_rejected() {
    // Non-scalar elements stay out-of-line — still rejected.
    let msgs = check(&program("tag: Int; names: [String; 4];"));
    assert!(
        msgs.iter().any(|m| m.contains("zero_copy")
            && m.contains("not flat-shapeable")),
        "a [String; 4] field in a zero_copy payload keeps the \
         out-of-line layout and must be rejected; got: {:?}",
        msgs
    );
}

#[test]
fn zero_copy_scalar_only_payload_is_accepted() {
    // The always-valid shape — only fixed-size scalar fields — must NOT
    // trip the flat-shapeable constraint (guard against over-rejecting).
    let msgs = check(&program("tag: Int; a: Int; b: Float;"));
    assert!(
        !msgs.iter().any(|m| m.contains("flat-shapeable")),
        "a scalar-only zero_copy payload must be accepted; got: {:?}",
        msgs
    );
}
