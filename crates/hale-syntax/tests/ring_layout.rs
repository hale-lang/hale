//! Parser round-trip for the `ring_layout` declaration
//! (shm-ring-interop Proposal B). Confirms members + nested
//! cursor/framing blocks land in the AST, including layout words
//! that collide with language keywords (`release`).

use hale_syntax::ast::{RingAttrValue, TopDecl};
use hale_syntax::parse_source;

#[test]
fn parses_full_ring_layout() {
    let src = r#"
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
    let prog = parse_source(src).expect("parse");
    let rl = prog
        .items
        .iter()
        .find_map(|i| match i {
            TopDecl::RingLayout(r) => Some(r),
            _ => None,
        })
        .expect("ring_layout decl present");

    assert_eq!(rl.name.name, "MagusRing");
    assert_eq!(rl.magic, Some(0x4D475348514D4B54));
    assert_eq!(rl.data_at, Some(128));
    assert_eq!(rl.overflow.as_ref().map(|i| i.name.as_str()), Some("lap_detect"));

    // scalars: version (expect 1, at 8, u32), buffer_size (at 12, u32).
    assert_eq!(rl.scalars.len(), 2);
    let version = rl.scalars.iter().find(|f| f.name.name == "version").unwrap();
    assert_eq!(version.expect, Some(1));
    assert_eq!(version.at, 8);
    assert_eq!(version.repr.name, "u32");
    let bsz = rl.scalars.iter().find(|f| f.name.name == "buffer_size").unwrap();
    assert_eq!(bsz.expect, None);
    assert_eq!(bsz.at, 12);

    // cursor: one named `published` with at=64, and the keyword
    // `release` admitted as the `store` value.
    assert_eq!(rl.cursors.len(), 1);
    let cur = &rl.cursors[0];
    assert_eq!(cur.name.as_ref().map(|i| i.name.as_str()), Some("published"));
    let at = cur.attrs.iter().find(|a| a.key.name == "at").unwrap();
    assert!(matches!(at.value, RingAttrValue::Int(64)));
    let store = cur.attrs.iter().find(|a| a.key.name == "store").unwrap();
    assert!(matches!(&store.value, RingAttrValue::Ident(i) if i.name == "release"));
    let unit = cur.attrs.iter().find(|a| a.key.name == "unit").unwrap();
    assert!(matches!(&unit.value, RingAttrValue::Ident(i) if i.name == "bytes"));

    // framing byte_records with len_prefix/align/pad_sentinel.
    let fr = rl.framing.as_ref().expect("framing");
    assert_eq!(fr.kind.name, "byte_records");
    let lp = fr.attrs.iter().find(|a| a.key.name == "len_prefix").unwrap();
    assert!(matches!(&lp.value, RingAttrValue::Ident(i) if i.name == "u32"));
    let pad = fr.attrs.iter().find(|a| a.key.name == "pad_sentinel").unwrap();
    assert!(matches!(pad.value, RingAttrValue::Int(0xFFFFFFFF)));
}

#[test]
fn bare_cursor_without_name_parses() {
    let src = r#"
ring_layout R {
    cursor { at 24; repr atomic_u64; }
    framing slots { }
}
"#;
    let prog = parse_source(src).expect("parse");
    let TopDecl::RingLayout(rl) = prog.items.iter().find(|i| matches!(i, TopDecl::RingLayout(_))).unwrap() else {
        unreachable!()
    };
    assert_eq!(rl.cursors.len(), 1);
    assert!(rl.cursors[0].name.is_none());
    assert_eq!(rl.framing.as_ref().unwrap().kind.name, "slots");
}
