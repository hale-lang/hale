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
ring_layout MagusRing {
    magic 0x4D475348514D4B54;
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
        r#"Foo: shm_ring("/magus.ticks", on_overflow: drop, layout: MagusRing) where zero_copy;"#,
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
        r#"Foo: shm_ring("/magus.ticks", on_overflow: drop, layout: Nonexistent) where zero_copy;"#,
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
        r#"Foo: shm_ring("/magus.ticks", on_overflow: drop, layout: Foo) where zero_copy;"#,
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
