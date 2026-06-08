//! `shm_ring(..., layout: Name)` binding kwarg (shm-ring-interop
//! Proposal B, PR2): the layout reference must resolve to a declared
//! `ring_layout`. Absent layout = the native ring (back-compat).

use hale_syntax::parse_source;
use hale_types::check_program;

fn check(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

const LAYOUT: &str = r#"
ring_layout ForeignRing {
    magic 0x52494E47464D5431;
    version 1 at 8 : u32;
    buffer_size at 12 : u32;
    data_at 128;
    cursor published { at 64; repr atomic_u64; load acquire; unit bytes; }
    framing byte_records { len_prefix u32; align 8; }
    overflow lap_detect;
}
"#;

fn program(binding: &str) -> String {
    format!(
        r#"
{LAYOUT}
type T {{ x: Int; }}
topic Foo {{ payload: T; }}
locus Producer {{
    bus {{ publish Foo; }}
    birth() {{ Foo <- T {{ x: 1 }}; }}
}}
main locus App {{
    bindings {{
        {binding}
    }}
}}
fn main() {{ App {{ }}; Producer {{ }}; }}
"#
    )
}

#[test]
fn layout_binding_resolves_clean() {
    let msgs = check(&program(
        r#"Foo: shm_ring("/foreign.ticks", on_overflow: drop, layout: ForeignRing) where zero_copy;"#,
    ));
    assert!(
        !msgs.iter().any(|m| m.contains("ring_layout")),
        "a binding to a declared ring_layout must resolve clean; got: {:?}",
        msgs
    );
}

#[test]
fn unknown_layout_errors() {
    let msgs = check(&program(
        r#"Foo: shm_ring("/foreign.ticks", on_overflow: drop, layout: Nonexistent) where zero_copy;"#,
    ));
    assert!(
        msgs.iter().any(|m| m.contains("unknown ring_layout `Nonexistent`")),
        "got: {:?}",
        msgs
    );
}

#[test]
fn layout_naming_a_non_layout_errors() {
    // `Foo` is a topic, not a ring_layout.
    let msgs = check(&program(
        r#"Foo: shm_ring("/foreign.ticks", on_overflow: drop, layout: Foo) where zero_copy;"#,
    ));
    assert!(
        msgs.iter().any(|m| m.contains("is not a `ring_layout`")),
        "got: {:?}",
        msgs
    );
}

#[test]
fn layout_less_binding_is_back_compat() {
    // No `layout:` → native ring; must typecheck without any
    // ring_layout diagnostic.
    let msgs = check(&program(
        r#"Foo: shm_ring("/lotus.ticks", slot_count: 4, on_overflow: drop) where zero_copy;"#,
    ));
    assert!(
        !msgs.iter().any(|m| m.contains("ring_layout")),
        "a layout-less shm_ring binding must stay valid; got: {:?}",
        msgs
    );
}

#[test]
fn layout_binding_with_nonflat_payload_errors() {
    // A foreign ring record is read by direct cast / written by memcpy,
    // so a layout-bound topic needs a flat-shapeable payload regardless
    // of whether `where zero_copy` is also asserted. `T` here has a
    // String field — not flat-shapeable.
    let src = format!(
        r#"
{LAYOUT}
type T {{ name: String; }}
topic Foo {{ payload: T; }}
locus Producer {{
    bus {{ publish Foo; }}
    birth() {{ Foo <- T {{ name: "x" }}; }}
}}
main locus App {{
    bindings {{
        Foo: shm_ring("/foreign.ticks", on_overflow: drop, layout: ForeignRing);
    }}
}}
fn main() {{ App {{ }}; Producer {{ }}; }}
"#
    );
    let msgs = check(&src);
    assert!(
        msgs.iter().any(|m| m.contains("requires a flat-shapeable payload")),
        "a layout binding with a non-flat payload must error; got: {:?}",
        msgs
    );
}

#[test]
fn buffer_size_not_multiple_of_align_errors() {
    // The producer's compile-time `buffer_size:` must be a multiple of
    // the layout's record `align` (the ForeignRing layout uses align 8),
    // else a record header can straddle the wrap → OOB.
    let msgs = check(&program(
        r#"Foo: shm_ring("/foreign.ticks", on_overflow: drop, layout: ForeignRing, buffer_size: 4094);"#,
    ));
    assert!(
        msgs.iter().any(|m| m.contains("must be a multiple of") && m.contains("align")),
        "a buffer_size not a multiple of align must error; got: {:?}",
        msgs
    );
}

#[test]
fn buffer_size_multiple_of_align_is_clean() {
    let msgs = check(&program(
        r#"Foo: shm_ring("/foreign.ticks", on_overflow: drop, layout: ForeignRing, buffer_size: 4096) where zero_copy;"#,
    ));
    assert!(
        !msgs.iter().any(|m| m.contains("multiple of")),
        "a buffer_size that IS a multiple of align must be clean; got: {:?}",
        msgs
    );
}

#[test]
fn bytesview_payload_on_layout_binding_is_clean() {
    // Raw-frame mode: a BytesView payload on a layout binding is the
    // path for heterogeneous / variable-length rings — accepted (not
    // required to be flat-shapeable).
    let src = format!(
        r#"
{LAYOUT}
topic Recs {{ payload: BytesView; }}
locus Sub {{
    bus {{ subscribe Recs as on_rec; }}
    fn on_rec(v: BytesView) {{ let _ = std::bytes::read_u8(v, 0) or 0; }}
}}
main locus App {{
    bindings {{
        Recs: shm_ring("/foreign.ticks", on_overflow: drop, layout: MagusRing);
    }}
}}
fn main() {{ App {{ }}; Sub {{ }}; }}
"#
    );
    let msgs = check(&src);
    assert!(
        !msgs.iter().any(|m| m.contains("flat-shapeable")),
        "a BytesView payload on a layout binding must be accepted (raw-frame \
         mode); got: {:?}",
        msgs
    );
}
