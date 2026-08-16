//! #399 — the per-topic observation identity, in ONE place.
//!
//! The observer protocol (iris PROTOCOL.md §4) fuses topics across
//! binaries on `(name, shape_hash)` where `shape_hash` is a content
//! hash of the topic's wire subject plus its canonical payload
//! shape. Three parties must compute the SAME value: the native
//! emitter (codegen registers shapes at startup;
//! `lotus_obs.c::obs_fnv` hashes them), library emitters in other
//! languages, and — since #399 — the topology artifact, which
//! exports the identity so a recording/WAL segment can name the
//! exact checked topology it ran under.
//!
//! This module is the single Rust implementation. Codegen calls it
//! for both the wire-subject rule and the shape string, so the
//! artifact and the emitted binary cannot drift; the C hasher is a
//! byte-for-byte mirror of [`topic_shape_hash`], pinned by the
//! protocol's test vectors.
//!
//! The definition (mirrored in PROTOCOL.md §4):
//!
//!  * **wire subject** — the parent-joined dot-path of declared
//!    `subject:` values, child-last; a topic without `subject:`
//!    contributes its declared NAME, as written. Only explicitly
//!    declared subjects are stable across binaries (a name
//!    fallback carries the declaring binary's local — possibly
//!    mangled — spelling); shared topics should declare one.
//!  * **canonical shape** — for a payload written as a bare,
//!    non-generic named struct: the struct's fields in declaration
//!    order as `<field>:<tag>` joined by `;`. Tags: `i` Int/Uint,
//!    `f` Float, `b` Bool, `d` Decimal, `t` Time, `u` Duration,
//!    `s` String/StringView, `y` Bytes/BytesView/BytesMut,
//!    `struct` anything else (nested structs deliberately
//!    name-free so the hash never depends on a declaring binary's
//!    local type names). Any other payload form hashes the EMPTY
//!    shape.
//!  * **hash** — FNV-1a/64 (offset 0xcbf29ce484222325, prime
//!    0x100000001b3) over the subject bytes, one `:` byte, the
//!    shape bytes.

use std::collections::BTreeMap;

use hale_syntax::ast::*;

/// Every declared topic's wire subject: parent-joined `subject:`
/// dot-path (fallback: the declared name), child-last, cycle-safe.
/// Moved from codegen's `collect_topic_wire_subjects` — bus
/// routing, observer registration, and the artifact all read THIS
/// rule.
pub fn topic_wire_subjects(
    items: &[TopDecl],
) -> BTreeMap<String, String> {
    struct Raw {
        parent: Option<String>,
        subject: String,
    }
    let mut raw: BTreeMap<String, Raw> = BTreeMap::new();
    fn walk(items: &[TopDecl], raw: &mut BTreeMap<String, Raw>) {
        for item in items {
            match item {
                TopDecl::Topic(t) => {
                    raw.insert(
                        t.name.name.clone(),
                        Raw {
                            parent: t
                                .parent
                                .as_ref()
                                .map(|i| i.name.clone()),
                            subject: t
                                .subject
                                .clone()
                                .unwrap_or_else(|| t.name.name.clone()),
                        },
                    );
                }
                TopDecl::Module(m) => walk(&m.items, raw),
                _ => {}
            }
        }
    }
    walk(items, &mut raw);

    let mut out = BTreeMap::new();
    for (name, r) in raw.iter() {
        let mut chain: Vec<String> = vec![r.subject.clone()];
        let mut visited: Vec<String> = vec![name.clone()];
        let mut cur = r.parent.clone();
        while let Some(p) = cur {
            if visited.contains(&p) {
                break;
            }
            visited.push(p.clone());
            match raw.get(&p) {
                Some(pr) => {
                    chain.push(pr.subject.clone());
                    cur = pr.parent.clone();
                }
                None => break,
            }
        }
        chain.reverse();
        out.insert(name.clone(), chain.join("."));
    }
    out
}

/// The canonical payload shape for one topic decl, per the pinned
/// definition. Empty string for every payload form that is not a
/// bare, non-generic named struct.
pub fn canonical_topic_shape(
    items: &[TopDecl],
    topic: &TopicDecl,
) -> String {
    let TypeExpr::Named { path, generic_args, .. } = &topic.payload
    else {
        return String::new();
    };
    if path.segments.len() != 1 || !generic_args.is_empty() {
        return String::new();
    }
    canonical_type_shape(items, path.segments[0].name.as_str())
}

/// The canonical structural shape of one declared bare struct type,
/// by (post-merge) name — the type-level half of
/// [`canonical_topic_shape`], exposed for the model builder (GH #476
/// Change 2), which needs payload identity for LITERAL endpoints'
/// `of type T` too, through the exact same renderer (a second shape
/// renderer would drift). Empty string when the name is not a
/// declared bare struct.
pub fn canonical_type_shape(items: &[TopDecl], type_name: &str) -> String {
    let mut types: BTreeMap<&str, &TypeDecl> = BTreeMap::new();
    fn collect<'a>(
        items: &'a [TopDecl],
        out: &mut BTreeMap<&'a str, &'a TypeDecl>,
    ) {
        for item in items {
            match item {
                TopDecl::Type(t) => {
                    out.insert(t.name.name.as_str(), t);
                }
                TopDecl::Module(m) => collect(&m.items, out),
                _ => {}
            }
        }
    }
    collect(items, &mut types);
    let Some(td) = types.get(type_name) else {
        return String::new();
    };
    let TypeDeclBody::Struct(fields) = &td.body else {
        return String::new();
    };
    fields
        .iter()
        .map(|f| format!("{}:{}", f.name.name, field_tag(&f.ty)))
        .collect::<Vec<_>>()
        .join(";")
}

/// One field's coarse tag. Deliberately name-free for anything
/// compound (nested structs, arrays, tuples), so the hash never
/// depends on a declaring binary's local type names.
fn field_tag(ty: &TypeExpr) -> &'static str {
    match ty {
        TypeExpr::Primitive(p, _) => match p {
            PrimType::Int | PrimType::Uint => "i",
            PrimType::Float => "f",
            PrimType::Bool => "b",
            PrimType::Decimal => "d",
            PrimType::Time => "t",
            PrimType::Duration => "u",
            PrimType::String | PrimType::StringView => "s",
            PrimType::Bytes
            | PrimType::BytesView
            | PrimType::BytesMut => "y",
        },
        _ => "struct",
    }
}

/// FNV-1a/64 over `subject ++ ':' ++ shape` — the byte-for-byte
/// mirror of `lotus_obs.c::obs_fnv`. The `:` separator is hashed
/// unconditionally (an empty shape still contributes it), exactly
/// as the C does.
pub fn topic_shape_hash(subject: &str, shape: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |b: u8| {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for b in subject.bytes() {
        eat(b);
    }
    eat(b':');
    for b in shape.bytes() {
        eat(b);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use hale_syntax::parse_source;

    fn subjects_and(
        src: &str,
    ) -> (Vec<TopDecl>, BTreeMap<String, String>) {
        let p = parse_source(src).expect("parse");
        let subs = topic_wire_subjects(&p.items);
        (p.items, subs)
    }

    /// The protocol's test vectors (PROTOCOL.md §4). Changing any
    /// of these values is a wire-protocol break.
    #[test]
    fn the_protocol_vectors_hold() {
        let src = r#"
            type Task { id: Int; label: String; }
            topic Tasks { payload: Task; }
        "#;
        let (items, subs) = subjects_and(src);
        assert_eq!(subs["Tasks"], "Tasks");
        let topic = items
            .iter()
            .find_map(|i| match i {
                TopDecl::Topic(t) => Some(t),
                _ => None,
            })
            .unwrap();
        let shape = canonical_topic_shape(&items, topic);
        assert_eq!(shape, "id:i;label:s");
        assert_eq!(
            format!("{:016x}", topic_shape_hash("Tasks", &shape)),
            "f7d174542aa33437"
        );
        // The empty-shape vector: a subject with no resolvable
        // struct payload still hashes subject + ':'.
        assert_eq!(
            format!("{:016x}", topic_shape_hash("Tasks", "")),
            "f3573379dcc4dcd5"
        );
    }

    /// Parent-joined subjects: the child's wire subject is the
    /// dot-path, and THAT is what the identity hashes — the
    /// unjoined declared subject is not an identity.
    #[test]
    fn parented_subjects_join_child_last() {
        let src = r#"
            type M { n: Int; }
            topic Org { payload: M; subject: "org"; }
            topic Metrics : Org {
                payload: M;
                subject: "metrics";
            }
        "#;
        let (_items, subs) = subjects_and(src);
        assert_eq!(subs["Metrics"], "org.metrics");
        assert_eq!(subs["Org"], "org");
    }

    /// Nested structs are name-free — a compound field is `struct`
    /// regardless of what the declaring binary calls it.
    #[test]
    fn nested_structs_are_name_free() {
        let src = r#"
            type Inner { a: Bool; }
            type Payload { id: Int; body: Inner; }
            topic T { payload: Payload; }
        "#;
        let (items, _) = subjects_and(src);
        let topic = items
            .iter()
            .find_map(|i| match i {
                TopDecl::Topic(t) => Some(t),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            canonical_topic_shape(&items, topic),
            "id:i;body:struct"
        );
    }
}
