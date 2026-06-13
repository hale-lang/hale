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
ring_layout ForeignRing {
    magic 0x52494E47464D5431;
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
fn record_header_clean_and_validated() {
    // A valid record_header layout (fixed-header: 32-byte header, kind-pad,
    // post_copy recheck) typechecks clean.
    let valid = r#"
ring_layout FixedHdrRing {
    magic 0x52494E47464D5431;
    version 1 at 8 : u32;
    buffer_size at 12 : u32;
    data_at 128;
    cursor published { at 64; repr atomic_u64; load acquire; unit bytes; }
    framing byte_records {
        len_prefix u32; align 8;
        record_header_bytes 32;
        pad_field_offset 4; pad_field_width 1; pad_field_value 1;
        seq_offset 8; seq_width 8;
        kernel_ns_offset 16; kernel_ns_width 8;
        recheck post_copy;
    }
    overflow lap_detect;
}
"#;
    assert!(check(valid).is_empty(), "valid record_header should be clean; got: {:?}", check(valid));

    // A declared header field beyond the record header → error.
    let bad_field = valid.replace("seq_offset 8;", "seq_offset 40;");
    assert!(
        check(&bad_field).iter().any(|m| m.contains("outside the")),
        "got: {:?}", check(&bad_field)
    );

    // record_header_bytes not a multiple of align → error (the stride
    // formula header + align(len) would not hold).
    let bad_align = valid.replace("record_header_bytes 32;", "record_header_bytes 30;");
    assert!(
        check(&bad_align).iter().any(|m| m.contains("must be a multiple of align")),
        "got: {:?}", check(&bad_align)
    );

    // pad_field beyond the header → error.
    let bad_pad = valid.replace("pad_field_offset 4;", "pad_field_offset 40;");
    assert!(
        check(&bad_pad).iter().any(|m| m.contains("outside the")),
        "got: {:?}", check(&bad_pad)
    );

    // unknown recheck mode → error.
    let bad_recheck = valid.replace("recheck post_copy;", "recheck seqlock;");
    assert!(
        check(&bad_recheck).iter().any(|m| m.contains("unknown recheck mode")),
        "got: {:?}", check(&bad_recheck)
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
// the foreign format is fixed, so a mis-transcribed layout is our bug and
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

// --- Frontend hardening (2026-06-08) ---

#[test]
fn len_prefix_wider_than_align_errors() {
    // len_prefix u64 (8) with align 4 → a record header could straddle
    // the wrap. Must error.
    let src = r#"
ring_layout R {
    magic 0x1;
    buffer_size at 12 : u32;
    data_at 128;
    cursor published { at 64; repr atomic_u64; load acquire; unit bytes; }
    framing byte_records { len_prefix u64; align 4; }
}
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("exceeds `align`")),
        "len_prefix wider than align must error; got: {:?}",
        msgs
    );
}

#[test]
fn unaligned_atomic_cursor_errors() {
    let src = r#"
ring_layout R {
    magic 0x1;
    buffer_size at 12 : u32;
    data_at 128;
    cursor published { at 60; repr atomic_u64; load acquire; unit bytes; }
    framing byte_records { len_prefix u32; align 8; }
}
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("8-byte aligned")),
        "an atomic_u64 cursor at a non-8-aligned offset must error; got: {:?}",
        msgs
    );
}

#[test]
fn missing_magic_and_buffer_size_error() {
    // No magic, no buffer_size scalar, no data_at — each is required.
    let src = r#"
ring_layout R {
    cursor published { at 64; repr atomic_u64; load acquire; unit bytes; }
    framing byte_records { len_prefix u32; align 8; }
}
"#;
    let msgs = check(src);
    assert!(msgs.iter().any(|m| m.contains("needs a `magic`")), "got: {:?}", msgs);
    assert!(msgs.iter().any(|m| m.contains("needs a `buffer_size` scalar")), "got: {:?}", msgs);
    assert!(msgs.iter().any(|m| m.contains("needs `data_at`")), "got: {:?}", msgs);
}

#[test]
fn cursor_unit_must_match_framing() {
    let src = r#"
ring_layout R {
    magic 0x1;
    buffer_size at 12 : u32;
    data_at 128;
    cursor published { at 64; repr atomic_u64; load acquire; unit slots; }
    framing byte_records { len_prefix u32; align 8; }
}
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("doesn't match `framing")),
        "cursor unit/framing mismatch must error; got: {:?}",
        msgs
    );
}

#[test]
fn u64_magic_with_top_bit_set_is_expressible() {
    // A full-width u64 magic (top bit set) must lex + typecheck — it
    // can't be written as a positive i64. Must be clean.
    let src = r#"
ring_layout R {
    magic 0xFFFFFFFFFFFFFFFF;
    version 1 at 8 : u32;
    buffer_size at 12 : u32;
    data_at 128;
    cursor published { at 64; repr atomic_u64; load acquire; unit bytes; }
    framing byte_records { len_prefix u32; align 8; pad_sentinel 0xFFFFFFFF; }
    overflow lap_detect;
}
"#;
    let msgs = check(src);
    assert!(
        msgs.is_empty(),
        "a u64 magic with the top bit set must be expressible + clean; got: {:?}",
        msgs
    );
}

/// A LotusRing-shaped `slots` layout (the native LRSRNG1 format) is clean:
/// slot geometry comes from `slot_size`/`slot_count` scalars, the cursor
/// is the seqno, and there's no buffer_size/len_prefix.
#[test]
fn valid_slots_layout_is_clean() {
    let src = r#"
ring_layout LotusRing {
    magic 0x4C5253524E4731;
    slot_size  at 8  : u64;
    slot_count at 16 : u64;
    data_at 128;
    cursor published { at 24; repr atomic_u64; load acquire; unit slots; }
    framing slots { }
    overflow lap_detect;
}
"#;
    let msgs = check(src);
    assert!(
        msgs.is_empty(),
        "a well-formed slots ring_layout must typecheck clean; got: {:?}",
        msgs
    );
}

/// `framing slots` without the slot geometry is rejected — the consumer
/// reads slot_size/slot_count from the header, so the scalars are required.
#[test]
fn slots_framing_requires_slot_geometry() {
    let src = r#"
ring_layout Bad {
    magic 0x4C5253524E4731;
    data_at 128;
    cursor published { at 24; repr atomic_u64; load acquire; unit slots; }
    framing slots { }
    overflow lap_detect;
}
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("slot_size"))
            && msgs.iter().any(|m| m.contains("slot_count")),
        "framing slots without slot_size/slot_count must be diagnosed; got: {:?}",
        msgs
    );
}
