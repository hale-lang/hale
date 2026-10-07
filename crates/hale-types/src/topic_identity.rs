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
//!
//! The canonical shape is the observation form of [`Shapes`], the one
//! renderer of a type's shape (spec/model.md § The shape of a type);
//! its contract form is what a surface row's types are, for the
//! contract digest (GH #1417), and agrees with it on every flat struct.

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
/// declared bare struct. The observation form of [`Shapes`].
pub fn canonical_type_shape(items: &[TopDecl], type_name: &str) -> String {
    Shapes::of(items).named(type_name, ShapeForm::Observation).unwrap_or_default()
}

/// Which of a type's two shapes [`Shapes`] renders (spec/model.md § The
/// shape of a type). The two agree byte for byte on a flat struct, one
/// whose every field is a primitive, an identity, a range, a quantity or
/// a point, so such a type has one shape hash whichever form reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeForm {
    /// The topic's observation identity and the model's payload
    /// contract: a bare struct's fields, every compound field tagged
    /// `struct`, name-free so the hash never depends on a declaring
    /// binary's local type names. Only a bare struct has one. It is a
    /// wire identity (the runtime's `obs_fnv` mirrors it), so it never
    /// moves.
    Observation,
    /// What a surface row's request, response or error is, for the
    /// contract digest (spec/api.md § The contract digest): deep, so a
    /// field changed inside a nested struct or a variant added to an
    /// enum moves the hash. Every type has one.
    Contract,
}

/// The renderer of both shape forms over one program's declarations:
/// every `type` by its (post-merge) name, the identities and ranges (each
/// the `Int` it is) and the quantities and points (each its `q(…)` tag).
pub struct Shapes<'a> {
    types: BTreeMap<&'a str, &'a TypeDecl>,
    ints: std::collections::BTreeSet<&'a str>,
    quantities: BTreeMap<&'a str, String>,
}

/// The FNV-1a/64 fold of a shape string: a type's shape hash, in either
/// form, written as sixteen lowercase hex digits where it is shown.
pub fn shape_hash(shape: &str) -> u64 {
    hale_graph::identity::fnv64(shape.as_bytes())
}

impl<'a> Shapes<'a> {
    pub fn of(items: &'a [TopDecl]) -> Shapes<'a> {
        let mut types: BTreeMap<&str, &TypeDecl> = BTreeMap::new();
        fn collect<'a>(items: &'a [TopDecl], out: &mut BTreeMap<&'a str, &'a TypeDecl>) {
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
        let scalars: BTreeMap<&str, &hale_syntax::ast::ScalarDecl> = types
            .iter()
            .filter_map(|(n, t)| match &t.body {
                TypeDeclBody::Scalar(s) => Some((*n, s)),
                _ => None,
            })
            .collect();
        // GH #1076 (U2, decision 9): an identity or a range is tagged as
        // the `Int` it is, so declaring one changes no shape hash.
        let ints = crate::units::typed_scalar_names(&scalars);
        // GH #1076 (U3, decision 9): a quantity or a point is tagged by
        // its denomination, so two processes whose fields count in
        // different denominations disagree in the shape hash; a program
        // with neither renders exactly what it did.
        let quantities = crate::units::quantity_tags(&scalars);
        Shapes { types, ints, quantities }
    }

    /// The shape of the type `name` names, in `form`: `None` for a name
    /// with no observation shape (anything but a bare struct).
    pub fn named(&self, name: &str, form: ShapeForm) -> Option<String> {
        let te = TypeExpr::Named {
            path: hale_syntax::ast::QualifiedName {
                segments: vec![hale_syntax::ast::Ident::new(name, Span::new(0, 0))],
                span: Span::new(0, 0),
            },
            generic_args: Vec::new(),
            span: Span::new(0, 0),
        };
        self.shape(&te, form)
    }

    /// The shape of `te` in `form`. The observation form is a bare
    /// struct's alone; the contract form is every type's.
    pub fn shape(&self, te: &TypeExpr, form: ShapeForm) -> Option<String> {
        match form {
            ShapeForm::Observation => {
                // A builtin record has no observation shape: its payload
                // contract stays the opaque one it always was.
                let fields = self.struct_fields(te, false)?;
                Some(
                    fields
                        .iter()
                        .map(|(name, ty)| format!("{name}:{}", self.observation_tag(ty)))
                        .collect::<Vec<_>>()
                        .join(";"),
                )
            }
            ShapeForm::Contract => {
                // The type itself is the outermost one being rendered, so
                // a field that names it again is `rec(1)`.
                let mut stack: Vec<String> = Self::bare_name(te).map(|n| vec![n.to_string()]).unwrap_or_default();
                Some(self.contract_top(te, &mut stack))
            }
        }
    }

    /// The shape hash of `te`'s contract shape.
    pub fn contract_hash(&self, te: &TypeExpr) -> u64 {
        shape_hash(&self.shape(te, ShapeForm::Contract).unwrap_or_default())
    }

    /// The single-segment, non-generic name `te` spells.
    fn bare_name<'t>(te: &'t TypeExpr) -> Option<&'t str> {
        match te {
            TypeExpr::Named { path, generic_args, .. } if path.segments.len() == 1 && generic_args.is_empty() => {
                Some(path.segments[0].name.as_str())
            }
            _ => None,
        }
    }

    /// A bare struct's fields by name and type: a declared one's, or with
    /// `builtins` a builtin record's (`ClosureViolation`, `IndexError`, …,
    /// a struct of its primitive fields); a declaration of its name wins,
    /// as in the checker.
    fn struct_fields(&self, te: &TypeExpr, builtins: bool) -> Option<Vec<(String, TypeExpr)>> {
        let name = Self::bare_name(te)?;
        match self.types.get(name) {
            Some(td) => match &td.body {
                TypeDeclBody::Struct(fields) => {
                    Some(fields.iter().map(|f| (f.name.name.clone(), f.ty.clone())).collect())
                }
                _ => None,
            },
            None if !builtins => None,
            None => {
                let b = crate::builtin_types::BUILTIN_TYPES.iter().find(|b| b.name == name)?;
                Some(
                    b.fields
                        .iter()
                        .map(|(f, p)| (f.to_string(), TypeExpr::Primitive(*p, Span::new(0, 0))))
                        .collect(),
                )
            }
        }
    }

    /// A field's tag both forms share: a primitive's letter, an identity
    /// or range `i`, a quantity or point its `q(…)`. `None` for anything
    /// compound.
    fn flat_tag(&self, ty: &TypeExpr) -> Option<String> {
        if let Some(name) = Self::bare_name(ty) {
            if let Some(q) = self.quantities.get(name) {
                return Some(q.clone());
            }
            if self.ints.contains(name) {
                return Some("i".to_string());
            }
        }
        match ty {
            TypeExpr::Primitive(p, _) => Some(
                match p {
                    PrimType::Int | PrimType::Uint => "i",
                    PrimType::Float => "f",
                    PrimType::Bool => "b",
                    PrimType::Decimal => "d",
                    PrimType::Time => "t",
                    PrimType::Duration => "u",
                    PrimType::String | PrimType::StringView => "s",
                    PrimType::Bytes | PrimType::BytesView | PrimType::BytesMut => "y",
                }
                .to_string(),
            ),
            _ => None,
        }
    }

    /// One field's observation tag: its flat tag, or `struct` for
    /// anything compound (nested structs, arrays, tuples), name-free.
    fn observation_tag(&self, ty: &TypeExpr) -> String {
        self.flat_tag(ty).unwrap_or_else(|| "struct".to_string())
    }

    /// The contract shape of a type a row names: a struct's fields as
    /// `<field>:<tag>` joined by `;`, an enum as `=enum(<variants>)`, any
    /// other type as `=` and its tag. `stack` holds the named types being
    /// rendered, outermost first.
    fn contract_top(&self, te: &TypeExpr, stack: &mut Vec<String>) -> String {
        if let Some(fields) = self.struct_fields(te, true) {
            return fields
                .iter()
                .map(|(name, ty)| format!("{name}:{}", self.contract_tag(ty, stack)))
                .collect::<Vec<_>>()
                .join(";");
        }
        if let Some(variants) = self.enum_variants(te) {
            let vs = variants
                .iter()
                .map(|v| {
                    if v.fields.is_empty() {
                        v.name.name.clone()
                    } else {
                        let fs: Vec<String> = v.fields.iter().map(|f| self.contract_tag(f, stack)).collect();
                        format!("{}({})", v.name.name, fs.join(","))
                    }
                })
                .collect::<Vec<_>>();
            return format!("=enum({})", vs.join("|"));
        }
        if let Some(target) = self.alias_target(te) {
            return self.contract_top(target, stack);
        }
        format!("={}", self.contract_tag(te, stack))
    }

    fn enum_variants(&self, te: &TypeExpr) -> Option<&'a [hale_syntax::ast::EnumVariant]> {
        match &self.types.get(Self::bare_name(te)?)?.body {
            TypeDeclBody::Enum(vs) => Some(vs),
            _ => None,
        }
    }

    /// What an alias, or a scalar that is neither an identity, a range,
    /// a quantity nor a point (`distinct Float`), stands for.
    fn alias_target(&self, te: &TypeExpr) -> Option<&'a TypeExpr> {
        let name = Self::bare_name(te)?;
        if self.ints.contains(name) || self.quantities.contains_key(name) {
            return None;
        }
        match &self.types.get(name)?.body {
            TypeDeclBody::Alias(t) => Some(t),
            TypeDeclBody::Scalar(s) => Some(&s.base),
            _ => None,
        }
    }

    /// One field's (or element's, or variant field's) contract tag: its
    /// flat tag; `#` and the contract hash of a named struct or enum
    /// (`rec(<k>)` for one already being rendered, `k` levels out); what
    /// an alias stands for; `[<tag>]`, `[<tag>;<n>]`, `bounded[<tag>;<n>]`
    /// and `(<tag>,…)` for the compound forms; `opaque(<type>)` for a type
    /// that names nothing the program declares.
    fn contract_tag(&self, ty: &TypeExpr, stack: &mut Vec<String>) -> String {
        if let Some(t) = self.flat_tag(ty) {
            return t;
        }
        if let Some(target) = self.alias_target(ty) {
            return self.contract_tag(target, stack);
        }
        match ty {
            TypeExpr::Named { .. } if self.struct_fields(ty, true).is_some() || self.enum_variants(ty).is_some() => {
                let name = Self::bare_name(ty).unwrap_or_default().to_string();
                if let Some(at) = stack.iter().position(|n| *n == name) {
                    return format!("rec({})", stack.len() - at);
                }
                stack.push(name);
                let inner = self.contract_top(ty, stack);
                stack.pop();
                format!("#{:016x}", shape_hash(&inner))
            }
            TypeExpr::Array { elem, size, .. } => {
                let e = self.contract_tag(elem, stack);
                match size {
                    None => format!("[{e}]"),
                    Some(hale_syntax::ast::Expr::Literal(hale_syntax::ast::Literal::Int(n), _)) => format!("[{e};{n}]"),
                    Some(_) => format!("[{e};_]"),
                }
            }
            TypeExpr::Bounded { elem, cap, .. } => format!("bounded[{};{cap}]", self.contract_tag(elem, stack)),
            TypeExpr::Tuple(parts, _) => {
                let ps: Vec<String> = parts.iter().map(|p| self.contract_tag(p, stack)).collect();
                format!("({})", ps.join(","))
            }
            TypeExpr::Named { path, generic_args, .. } => {
                let base = path.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::");
                if generic_args.is_empty() {
                    format!("opaque({base})")
                } else {
                    let args: Vec<String> = generic_args.iter().map(|a| self.contract_tag(a, stack)).collect();
                    format!("opaque({base}<{}>)", args.join(","))
                }
            }
            TypeExpr::Function { .. } => "opaque(fn)".to_string(),
            TypeExpr::Projection { .. } => "opaque(projection)".to_string(),
            TypeExpr::Perspective { name, .. } => format!("opaque(perspective({}))", name.name),
            TypeExpr::Primitive(..) => unreachable!("a primitive has a flat tag"),
        }
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

    /// Both forms of `name`'s shape, and the contract shape's hash.
    fn forms(src: &str, name: &str) -> (Option<String>, String, String) {
        let p = parse_source(src).expect("parse");
        let s = Shapes::of(&p.items);
        let contract = s.named(name, ShapeForm::Contract).unwrap();
        let h = format!("{:016x}", shape_hash(&contract));
        (s.named(name, ShapeForm::Observation), contract, h)
    }

    /// spec/model.md § The shape of a type, one kind per case: the
    /// shape string each form renders, byte for byte, and the contract
    /// shape's hash. A flat struct's two forms are one string, so its
    /// hash is the one the payload contract has always carried.
    #[test]
    fn primitives_render_their_letters_in_both_forms() {
        let (obs, contract, h) = forms(
            "type P { a: Int; b: Uint; c: Float; d: Bool; e: Decimal; f: Time; g: Duration; h: String; i: Bytes; }",
            "P",
        );
        assert_eq!(contract, "a:i;b:i;c:f;d:b;e:d;f:t;g:u;h:s;i:y");
        assert_eq!(obs.as_deref(), Some(contract.as_str()), "a flat struct has one shape");
        assert_eq!(h, "de3f3333e812b5fb");
    }

    /// The unit dialect's scalars: an identity is the `Int` it is, a
    /// quantity its `q(<denomination>)`. These are the R0 fixture's
    /// values (tests/api-contract/digest.md), which no form moves.
    #[test]
    fn named_scalars_are_their_int_or_their_unit() {
        let src = "unit cent; type Money = quantity Int in cent; type OrderId = distinct Int; \
                   type PlaceOrder { symbol: String; qty: Int; limit: Money; } \
                   type OrderReceipt { order: OrderId; notional: Money; } \
                   type CancelOrder { order: OrderId; }";
        for (name, shape, hash) in [
            ("PlaceOrder", "symbol:s;qty:i;limit:q(cent)", "cb5775974312c858"),
            ("OrderReceipt", "order:i;notional:q(cent)", "bb4f99639cf069af"),
            ("CancelOrder", "order:i", "deb8489f34994e5a"),
        ] {
            let (obs, contract, h) = forms(src, name);
            assert_eq!(contract, shape, "{name}");
            assert_eq!(obs.as_deref(), Some(shape), "{name}: a flat struct has one shape");
            assert_eq!(h, hash, "{name}");
        }
        // A scalar a row names by itself is `=` and its tag.
        assert_eq!(forms(src, "OrderId").1, "=i");
        assert_eq!(forms(src, "Money").1, "=q(cent)");
        assert_eq!(forms(src, "OrderId").0, None, "only a bare struct has an observation shape");
    }

    /// A nested struct is its own contract hash in the field's slot,
    /// `#<hash>`; the observation form keeps the name-free `struct`.
    #[test]
    fn a_nested_struct_is_its_hash_in_the_contract_form() {
        let src = "type Inner { a: Bool; } type Outer { id: Int; body: Inner; }";
        let (_, inner, inner_h) = forms(src, "Inner");
        assert_eq!(inner, "a:b");
        assert_eq!(inner_h, "e661911904a01160");
        let (obs, contract, h) = forms(src, "Outer");
        assert_eq!(obs.as_deref(), Some("id:i;body:struct"));
        assert_eq!(contract, format!("id:i;body:#{inner_h}"));
        assert_eq!(h, "e94668d4adea4cba");
        // A field changed inside the nested type moves the outer hash.
        let (_, _, moved) = forms("type Inner { a: Int; } type Outer { id: Int; body: Inner; }", "Outer");
        assert_ne!(moved, h);
    }

    /// An enum is its variants in declaration order, a variant's fields
    /// tagged; as a field it is its hash, as a nested struct is.
    #[test]
    fn an_enum_is_its_variants() {
        let src = "type Side = enum { Buy, Sell(Int, String) }; type Order { side: Side; }";
        let (obs, contract, h) = forms(src, "Side");
        assert_eq!(obs, None);
        assert_eq!(contract, "=enum(Buy|Sell(i,s))");
        assert_eq!(h, "64470fcdf9853aeb");
        assert_eq!(forms(src, "Order").1, format!("side:#{h}"));
        let (_, _, added) = forms("type Side = enum { Buy, Sell(Int, String), Hold };", "Side");
        assert_ne!(added, h, "a variant added moves the hash");
    }

    /// Arrays, bounded arrays and tuples are their element tags, and an
    /// alias is what it stands for.
    #[test]
    fn compound_fields_and_aliases() {
        let src = "type Count = Int; type Inner { a: Bool; } \
                   type C { xs: [Int]; fixed: [Float; 3]; window: bounded[Inner; 8]; pair: (Int, String); n: Count; }";
        let (obs, contract, h) = forms(src, "C");
        assert_eq!(obs.as_deref(), Some("xs:struct;fixed:struct;window:struct;pair:struct;n:struct"));
        assert_eq!(contract, "xs:[i];fixed:[f;3];window:bounded[#e661911904a01160;8];pair:(i,s);n:i");
        assert_eq!(h, "62333af909327aef");
    }

    /// A type that names itself again is `rec(<k>)`, `k` levels out, so
    /// every shape is finite.
    #[test]
    fn a_recursive_type_is_finite() {
        let (_, contract, _) = forms("type Node { v: Int; kids: [Node]; }", "Node");
        assert_eq!(contract, "v:i;kids:[rec(1)]");
    }

    /// The builtin record a violation carries is its fields in the
    /// contract form (spec/api.md § The contract digest), and has no
    /// observation shape, so its payload contract stays opaque.
    #[test]
    fn closure_violation_is_its_record() {
        let (obs, contract, h) = forms("type Unrelated { a: Int; }", "ClosureViolation");
        assert_eq!(obs, None);
        assert_eq!(contract, "locus:s;closure:s;diff:i");
        assert_eq!(h, "36c7f0561125943e");
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
