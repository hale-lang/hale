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
use hale_syntax::Span;

/// Every declared topic's wire subject: parent-joined `subject:`
/// dot-path (fallback: the declared name), child-last, cycle-safe.
/// Moved from codegen's `collect_topic_wire_subjects` — bus
/// routing, observer registration, and the artifact all read THIS
/// rule.
pub fn topic_wire_subjects(
    items: &[TopDecl],
) -> BTreeMap<String, String> {
    let decls = declared_topics(&[items]);
    decls
        .keys()
        .map(|name| (name.clone(), wire_of(name, &decls).0))
        .collect()
}

/// Every topic declaration by name, modules included; a second
/// declaration of a name replaces the first (the resolver reports the
/// duplicate).
fn declared_topics<'a>(slices: &[&'a [TopDecl]]) -> BTreeMap<String, &'a TopicDecl> {
    fn walk<'a>(items: &'a [TopDecl], out: &mut BTreeMap<String, &'a TopicDecl>) {
        for item in items {
            match item {
                TopDecl::Topic(t) => {
                    out.insert(t.name.name.clone(), t);
                }
                TopDecl::Module(m) => walk(&m.items, out),
                _ => {}
            }
        }
    }
    let mut out = BTreeMap::new();
    for items in slices {
        walk(items, &mut out);
    }
    out
}

/// A topic's declared segment: its `subject:`, else its name.
fn own_segment(t: &TopicDecl) -> String {
    t.subject.clone().unwrap_or_else(|| t.name.name.clone())
}

/// The wire subject of `name`, and whether its parent chain is broken
/// (it reaches an undeclared parent or a cycle). A broken chain still
/// joins the segments it reached, as the rule always has.
fn wire_of(name: &str, decls: &BTreeMap<String, &TopicDecl>) -> (String, bool) {
    let t = decls[name];
    let mut chain: Vec<String> = vec![own_segment(t)];
    let mut visited: Vec<&str> = vec![name];
    let mut cur = t.parent.as_ref().map(|i| i.name.as_str());
    let mut broken = false;
    while let Some(p) = cur {
        if visited.contains(&p) {
            broken = true;
            break;
        }
        visited.push(p);
        match decls.get(p) {
            Some(pt) => {
                chain.push(own_segment(pt));
                cur = pt.parent.as_ref().map(|i| i.name.as_str());
            }
            None => {
                broken = true;
                break;
            }
        }
    }
    chain.reverse();
    (chain.join("."), broken)
}

/// One declared topic: what it is on the wire and the policies a send
/// or a subscription to it is held to (F.40 phase 2.1b).
#[derive(Debug, Clone)]
pub struct TopicRow {
    pub name: String,
    pub parent: Option<String>,
    /// The declared `subject:` segment, or the name when the topic
    /// declares none.
    pub subject: String,
    /// The wire subject, by [`topic_wire_subjects`]' rule.
    pub wire: String,
    /// The parent chain reaches an undeclared parent or a cycle. The
    /// resolver reports it; a broken row answers no subject by its wire.
    pub broken: bool,
    pub payload: TypeExpr,
    pub keyed_by: Option<String>,
    pub on_unmatched: Option<UnmatchedPolicy>,
    pub bounded: Option<i64>,
    pub on_full_fail: bool,
    pub span: Span,
}

/// The bundle's topic rows, built once per bundle (on the `TopScope`),
/// and the one answer to "which topic does this subject name".
#[derive(Debug, Clone, Default)]
pub struct TopicRows {
    rows: BTreeMap<String, TopicRow>,
    /// Wire subject → the unbroken topics carrying it, in name order.
    by_wire: BTreeMap<String, Vec<String>>,
}

impl TopicRows {
    /// The rows of every topic the programs declare. A parent may be
    /// declared in another program of the bundle.
    pub fn of<'a>(programs: impl IntoIterator<Item = &'a Program>) -> TopicRows {
        let slices: Vec<&[TopDecl]> = programs.into_iter().map(|p| p.items.as_slice()).collect();
        let decls = declared_topics(&slices);
        let mut out = TopicRows::default();
        for (name, t) in &decls {
            let (wire, broken) = wire_of(name, &decls);
            if !broken {
                out.by_wire.entry(wire.clone()).or_default().push(name.clone());
            }
            out.rows.insert(
                name.clone(),
                TopicRow {
                    name: name.clone(),
                    parent: t.parent.as_ref().map(|i| i.name.clone()),
                    subject: own_segment(t),
                    wire,
                    broken,
                    payload: t.payload.clone(),
                    keyed_by: t.keyed_by.as_ref().map(|i| i.name.clone()),
                    on_unmatched: t.on_unmatched,
                    bounded: t.bounded.map(|(n, _)| n),
                    on_full_fail: t.on_full_fail.is_some(),
                    span: t.span,
                },
            );
        }
        out
    }

    /// The topic declared under `name`.
    pub fn named(&self, name: &str) -> Option<&TopicRow> {
        self.rows.get(name)
    }

    /// Every row, in name order.
    pub fn iter(&self) -> impl Iterator<Item = &TopicRow> {
        self.rows.values()
    }

    /// The topic that OWNS a wire subject: the one delivery identity
    /// (spec/model.md rule 8). A literal subject at a delivery site (a
    /// literal subscription, a literal send) is matched by this and
    /// nothing else: a literal that spells a topic's declared segment or
    /// its name while its wire differs names no topic, since lowering
    /// delivers on the bytes as written. Two topics carrying one wire
    /// own neither.
    pub fn by_wire(&self, wire: &str) -> Option<&TopicRow> {
        match self.by_wire.get(wire).map(Vec::as_slice) {
            Some([one]) => self.rows.get(one),
            _ => None,
        }
    }

    /// Every wire subject more than one topic carries, with those
    /// topics in name order: the collision the resolver reports.
    pub fn shared_wires(&self) -> impl Iterator<Item = (&str, &[String])> {
        self.by_wire
            .iter()
            .filter(|(_, names)| names.len() > 1)
            .map(|(w, names)| (w.as_str(), names.as_slice()))
    }
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
    let mut h = hale_graph::identity::Fnv64::new();
    h.write(subject.as_bytes());
    h.write(b":");
    h.write(shape.as_bytes());
    h.finish()
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

    fn rows(src: &str) -> TopicRows {
        let p = parse_source(src).expect("parse");
        TopicRows::of([&p])
    }

    /// One rule: the wire identity, then the declared segment, then
    /// the name, each answering only for exactly one topic.
    /// A wire subject names the topic that owns it and nothing else: a
    /// declared segment or a topic name that is not the wire names no
    /// topic (delivery is on the wire, spec/model.md rule 8).
    #[test]
    fn a_wire_subject_names_its_owner_and_nothing_else_does() {
        let r = rows(
            r#"
            type M { n: Int; }
            topic Org { payload: M; subject: "org"; }
            topic Metrics : Org { payload: M; subject: "metrics"; }
            topic Plain { payload: M; }
        "#,
        );
        let of = |s: &str| r.by_wire(s).map(|t| t.name.as_str());
        assert_eq!(of("org.metrics"), Some("Metrics"));
        assert_eq!(of("metrics"), None, "a segment is not a wire");
        assert_eq!(of("Metrics"), None, "a name is not a wire");
        assert_eq!(of("org"), Some("Org"));
        assert_eq!(of("Plain"), Some("Plain"), "a top-level topic's wire is its name");
        assert_eq!(of("nothing"), None);
        assert_eq!(r.named("Metrics").map(|t| t.name.as_str()), Some("Metrics"));
        assert_eq!(r.named("Metrics").map(|t| t.wire.as_str()), Some("org.metrics"));
        assert_eq!(r.shared_wires().count(), 0);
    }

    /// Two topics carrying one wire subject is a collision the rows
    /// report once, and the subject names neither; a segment two
    /// topics declare names neither either.
    #[test]
    fn a_shared_subject_names_no_topic() {
        let r = rows(
            r#"
            type M { n: Int; }
            topic A { payload: M; subject: "same"; }
            topic B { payload: M; subject: "same"; }
            topic P { payload: M; subject: "p"; }
            topic Q { payload: M; subject: "q"; }
            topic X : P { payload: M; subject: "leaf"; }
            topic Y : Q { payload: M; subject: "leaf"; }
        "#,
        );
        assert!(r.by_wire("same").is_none());
        let shared: Vec<(&str, &[String])> = r.shared_wires().collect();
        assert_eq!(shared, vec![("same", &["A".to_string(), "B".to_string()][..])]);
        assert!(r.by_wire("leaf").is_none());
        assert_eq!(r.by_wire("p.leaf").map(|t| t.name.as_str()), Some("X"));
    }

    /// A broken parent chain keeps the rule's joined wire subject
    /// (codegen reads it) but answers no subject by it.
    #[test]
    fn a_broken_chain_answers_no_subject_by_its_wire() {
        let r = rows(
            r#"
            type M { n: Int; }
            topic Orphan : Missing { payload: M; subject: "o"; }
        "#,
        );
        let t = r.named("Orphan").unwrap();
        assert!(t.broken);
        assert_eq!(t.wire, "o");
        assert!(r.shared_wires().next().is_none());
        // Not by the wire subject; only by its name.
        assert!(r.by_wire("o").is_none());
        assert!(r.named("Orphan").is_some());
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
