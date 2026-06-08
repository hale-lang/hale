//! Resolve + typecheck for `ring_layout` declarations
//! (shm-ring-interop Proposal B): a valid layout is clean; the
//! layout contract (known reprs, framing, cursor) is enforced.

use hale_syntax::parse_source;
use hale_types::check_program;

fn check(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

const VALID: &str = r#"
ring_layout MagusRing {
    magic 0x4D475348514D4B54;
    version 1 at 8 : u32;
    buffer_size at 12 : u32;
    data_at 128;
    cursor published {
        at 64; repr atomic_u64; load acquire; store release; unit bytes;
    }
    framing byte_records {
        len_prefix u32; align 8; pad_sentinel 0xFFFFFFFF;
    }
    overflow lap_detect;
}
"#;

#[test]
fn valid_layout_is_clean() {
    let msgs = check(VALID);
    assert!(
        msgs.is_empty(),
        "a well-formed ring_layout must typecheck clean; got: {:?}",
        msgs
    );
}

#[test]
fn unknown_scalar_repr_errors() {
    let src = r#"
ring_layout R {
    version at 8 : u99;
    cursor c { at 64; }
    framing slots { }
}
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("unknown repr `u99`")),
        "got: {:?}",
        msgs
    );
}

#[test]
fn missing_cursor_errors() {
    let src = r#"
ring_layout R {
    framing slots { }
}
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("needs at least one `cursor")),
        "got: {:?}",
        msgs
    );
}

#[test]
fn cursor_without_at_errors() {
    let src = r#"
ring_layout R {
    cursor c { repr atomic_u64; }
    framing slots { }
}
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("cursor needs an `at OFFSET")),
        "got: {:?}",
        msgs
    );
}

#[test]
fn unknown_framing_errors() {
    let src = r#"
ring_layout R {
    cursor c { at 64; }
    framing mystery { }
}
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("unknown framing `mystery`")),
        "got: {:?}",
        msgs
    );
}

#[test]
fn byte_records_without_len_prefix_errors() {
    let src = r#"
ring_layout R {
    cursor c { at 64; }
    framing byte_records { align 8; }
}
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("needs `len_prefix")),
        "got: {:?}",
        msgs
    );
}

#[test]
fn bad_memory_ordering_errors() {
    let src = r#"
ring_layout R {
    cursor c { at 64; load sideways; }
    framing slots { }
}
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("unknown memory ordering `sideways`")),
        "got: {:?}",
        msgs
    );
}

#[test]
fn ring_layout_used_as_value_errors() {
    let src = r#"
ring_layout R {
    cursor c { at 64; }
    framing slots { }
}
fn main() {
    let x = R;
    println(to_string(x));
}
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("ring_layout `R` is not a value")),
        "got: {:?}",
        msgs
    );
}

// ---- conformance: cross-field geometric consistency (2026-06-06) ----
// magus2's format is fixed, so a mis-transcribed layout is our bug and
// several of these fields silently corrupt the reader. Caught at
// compile time.

#[test]
fn cursor_past_data_at_errors() {
    let src = r#"
ring_layout R {
    magic 0x1;
    data_at 64;
    cursor published { at 64; repr atomic_u64; load acquire; unit bytes; }
    framing byte_records { len_prefix u32; align 8; }
}
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("overruns `data_at")),
        "a cursor at/after data_at must error; got: {:?}",
        msgs
    );
}

#[test]
fn overlapping_header_fields_error() {
    // version@8:u64 occupies [8,16); buffer_size@12 overlaps it.
    let src = r#"
ring_layout R {
    magic 0x1;
    version 1 at 8 : u64;
    buffer_size at 12 : u32;
    data_at 128;
    cursor published { at 64; repr atomic_u64; load acquire; unit bytes; }
    framing byte_records { len_prefix u32; align 8; }
}
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("overlaps")),
        "overlapping header fields must error; got: {:?}",
        msgs
    );
}

#[test]
fn non_power_of_two_align_errors() {
    let src = r#"
ring_layout R {
    magic 0x1;
    data_at 128;
    cursor published { at 64; repr atomic_u64; load acquire; unit bytes; }
    framing byte_records { len_prefix u32; align 6; }
}
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("must be a power")),
        "a non-power-of-two align must error; got: {:?}",
        msgs
    );
}

#[test]
fn pad_sentinel_too_wide_for_len_prefix_errors() {
    // len_prefix is u16 (max 0xFFFF) but pad_sentinel is 0xFFFFFFFF.
    let src = r#"
ring_layout R {
    magic 0x1;
    data_at 128;
    cursor published { at 64; repr atomic_u64; load acquire; unit bytes; }
    framing byte_records { len_prefix u16; align 8; pad_sentinel 0xFFFFFFFF; }
}
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("does not fit in the `len_prefix` width")),
        "a pad_sentinel wider than len_prefix must error; got: {:?}",
        msgs
    );
}
