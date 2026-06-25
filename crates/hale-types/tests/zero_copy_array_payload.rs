//! Regression: a `where zero_copy` binding whose topic payload contains a
//! fixed-size array field must be REJECTED at typecheck — codegen lays an
//! array field out-of-line (the field is a pointer, not inline bytes), so a
//! raw memcpy of the value across a zero-copy / shm boundary shares a
//! pointer that dangles in the reader's address space (a real cross-process
//! segfault, surfaced by the bench `xproc` large-payload work). The
//! `is_flat_shapeable` predicate now matches that layout reality, so the
//! binding errors with a clear diagnostic instead of compiling-and-crashing.

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
fn zero_copy_array_field_payload_is_rejected() {
    let msgs = check(&program("tag: Int; data: [Int; 8];"));
    assert!(
        msgs.iter().any(|m| m.contains("zero_copy")
            && m.contains("not flat-shapeable")
            && m.contains("array")),
        "a fixed-size array field in a zero_copy payload must be rejected \
         with a flat-shapeable diagnostic naming the array; got: {:?}",
        msgs
    );
}

#[test]
fn zero_copy_scalar_only_payload_is_accepted() {
    // The valid shape — only fixed-size scalar fields — must NOT trip the
    // flat-shapeable constraint (regression guard against over-rejecting).
    let msgs = check(&program("tag: Int; a: Int; b: Float;"));
    assert!(
        !msgs.iter().any(|m| m.contains("flat-shapeable")),
        "a scalar-only zero_copy payload must be accepted; got: {:?}",
        msgs
    );
}
