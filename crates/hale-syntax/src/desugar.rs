//! Topic-reference desugaring.
//!
//! After parsing, the AST carries `BusSubject::Topic(name)` and
//! type-less `Subscribe { ty: None } / Publish { ty: None }` for
//! topic-ref forms (`subscribe Foo as h;`), plus
//! `Stmt::Send { subject: Expr::Ident("Foo"), ... }` for
//! topic-ref sends (`Foo <- expr`). Downstream stages (codegen,
//! interpreter) work against legacy literal-subject forms; this
//! pass normalizes the AST so they don't have to know topics
//! exist.
//!
//! Transformations:
//!   - `BusSubject::Topic(i)` → `BusSubject::Literal { subject:
//!     <wire_subject>, span: i.span }` and `ty: None` filled with
//!     the topic's declared payload type expression. The
//!     wire_subject is the dot-joined parent chain of own-subject
//!     segments (root-to-leaf).
//!   - `Stmt::Send { subject: Expr::Ident("Foo"), ... }` where
//!     `Foo` names a topic → `subject: Expr::Literal(String(
//!     <wire_subject>), span)`.
//!   - `desugar_intra_locus_topics` (Phase 2 closed-world): when
//!     a topic is used only intra-locus and has no binding,
//!     rewrites the publisher's `Stmt::Send` to a direct
//!     `self.handler(payload)` method call, sidestepping bus
//!     dispatch entirely. Runs BEFORE `desugar_topics` so the
//!     remaining bus refs go through the standard literal-subject
//!     rewrite.
//!
//! Type checking runs BEFORE this pass so topic-specific
//! diagnostics (handler-sig match, etc.) still see the original
//! `BusSubject::Topic` form and can cite the topic name in
//! errors. Codegen + runtime run AFTER, and see only the
//! literal-subject form (or, for optimized intra-locus topics,
//! direct method calls instead of Send statements).

use std::collections::BTreeMap;

use crate::ast::*;
use crate::Span;

/// Per-topic data the desugar pass needs: payload type (to fill
/// `ty: None` slots) and wire_subject (the literal subject string
/// that the topic ref desugars to). Wire subject is the dot-joined
/// chain of own-subject segments root-to-leaf — for a top-level
/// topic with no `subject:` field, it equals the topic name.
#[derive(Debug, Clone)]
struct TopicEntry {
    payload: TypeExpr,
    wire_subject: String,
}

/// Walk `program` and rewrite topic references into literal
/// forms in place. Caller invokes this after typecheck and
/// before codegen / interpretation. Idempotent: re-running on
/// already-desugared input is a no-op.
pub fn desugar_topics(program: &mut Program) {
    let mut topics: BTreeMap<String, TopicEntry> = BTreeMap::new();
    collect_topics(&program.items, &mut topics);
    rewrite_items(&mut program.items, &topics);
    desugar_binding_roles(program);
}

/// `--wrap-main` (browser playground): turn a bare-`main` program into a
/// wasm browser-entry program *on the AST*, span-preservingly. A wasm
/// program needs an `@export` locus entry (so codegen emits the
/// `_hale_start` persistent-arena path the JS loader drives) — a plain
/// `fn main` is not that shape. The playground used to rewrite the source
/// text (`fn main() { BODY }` → `@export locus __Tour { birth() { BODY } }`
/// + a `target wasm { }` header) with a brace-matching sed, which is not
/// lexer-aware (a `{`/`}` in a string or comment fools it) and shifts every
/// line so a parse/type error on the user's line 2 is reported on line 7.
///
/// This does the same wrap on the parsed AST instead. When the program has
/// a top-level `fn main()` and no `@export` entry it:
///   - replaces the `fn main` decl with a synthesized
///     `@export locus __Main { birth() { <main's body> } }` — routing
///     main's body through the same entry-inversion / `_hale_start` path an
///     `@export locus` birth already uses; and
///   - prepends a `target wasm { }` decl if none is present, so the
///     typechecker gates the syscall-backed stdlib (`std::io::tcp`, …).
///
/// Both synthesized nodes BORROW main's span, and main's body is MOVED
/// intact — so every statement keeps its original span and diagnostics
/// point at the user's real line/col with zero offset.
///
/// Prefer-explicit: if the program already declares an `@export` entry
/// (locus or fn) it is left untouched; no `fn main` ⇒ nothing to do. The
/// caller restricts this to wasm builds — on native there is no entry
/// inversion to wrap, so it would be meaningless. Returns whether it
/// wrapped a `main`.
pub fn wrap_main_as_wasm_export(program: &mut Program) -> bool {
    // Prefer-explicit: an existing @export entry means the program is
    // already in the wasm entry shape — don't touch it.
    let has_export_entry = program.items.iter().any(|it| match it {
        TopDecl::Locus(l) => l.export,
        TopDecl::Fn(f) => f.export,
        _ => false,
    });
    if has_export_entry {
        return false;
    }
    // Find the top-level `fn main`. No main ⇒ nothing to wrap.
    let Some(main_idx) = program.items.iter().position(
        |it| matches!(it, TopDecl::Fn(f) if f.name.name == "main"),
    ) else {
        return false;
    };
    let TopDecl::Fn(main_fn) = &program.items[main_idx] else {
        unreachable!("position matched a TopDecl::Fn")
    };
    // Borrow main's span; MOVE its body (statement spans preserved).
    let main_span = main_fn.span;
    let body = main_fn.body.clone();

    let birth = LifecycleDecl {
        kind: LifecycleKind::Birth,
        params: Vec::new(),
        ret: None,
        unbounded: false,
        body,
        span: main_span,
    };
    let locus = LocusDecl {
        name: Ident { name: "__Main".to_string(), span: main_span },
        is_main: false,
        export: true,
        generics: Vec::new(),
        annotations: Vec::new(),
        form: None,
        locality: None,
        bounded: false,
        members: vec![LocusMember::Lifecycle(birth)],
        span: main_span,
    };
    program.items[main_idx] = TopDecl::Locus(locus);

    // Inject `target wasm { }` if the program doesn't already gate.
    let has_target = program.items.iter().any(|it| {
        matches!(it, TopDecl::Target(t)
            if matches!(t.name.name.as_str(), "wasm" | "browser_js"))
    });
    if !has_target {
        program.items.insert(
            0,
            TopDecl::Target(TargetDecl {
                name: Ident { name: "wasm".to_string(), span: main_span },
                capabilities: Vec::new(),
                span: main_span,
            }),
        );
    }
    true
}

/// Fill in `TransportSpec::Unix { role: None, .. }` with the
/// role inferred from the bus block's publish/subscribe
/// declarations on the topic. Typecheck already emitted a diag
/// for the ambiguous case (both pub + sub with no explicit
/// role); here we just fill in the unambiguous cases. Anything
/// still `None` after this pass is either an error path
/// (typecheck diag fired) or an empty-binding (no pub or sub),
/// which we leave as `None` and let codegen sort out.
fn desugar_binding_roles(program: &mut Program) {
    let pubs = collect_topic_publishers(&program.items);
    let subs = collect_topic_subscribers(&program.items);
    fill_roles_in_items(&mut program.items, &pubs, &subs);
}

fn collect_topic_publishers(items: &[TopDecl]) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    fn walk(items: &[TopDecl], out: &mut std::collections::BTreeSet<String>) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    for member in &l.members {
                        if let LocusMember::Bus(bb) = member {
                            for bm in &bb.members {
                                if let BusMember::Publish { subject, .. } = bm {
                                    if let BusSubject::Topic(id) = subject {
                                        out.insert(id.name.clone());
                                    }
                                }
                            }
                        }
                    }
                }
                TopDecl::Module(m) => walk(&m.items, out),
                _ => {}
            }
        }
    }
    walk(items, &mut out);
    out
}

fn collect_topic_subscribers(items: &[TopDecl]) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    fn walk(items: &[TopDecl], out: &mut std::collections::BTreeSet<String>) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    for member in &l.members {
                        if let LocusMember::Bus(bb) = member {
                            for bm in &bb.members {
                                if let BusMember::Subscribe { subject, .. } = bm {
                                    if let BusSubject::Topic(id) = subject {
                                        out.insert(id.name.clone());
                                    }
                                }
                            }
                        }
                    }
                }
                TopDecl::Module(m) => walk(&m.items, out),
                _ => {}
            }
        }
    }
    walk(items, &mut out);
    out
}

fn fill_roles_in_items(
    items: &mut [TopDecl],
    pubs: &std::collections::BTreeSet<String>,
    subs: &std::collections::BTreeSet<String>,
) {
    for item in items {
        match item {
            TopDecl::Locus(l) => {
                for member in &mut l.members {
                    if let LocusMember::Bindings(bb) = member {
                        for entry in &mut bb.entries {
                            // Role inference only applies to substrate
                            // Unix bindings. Adapter bindings carry
                            // direction in the adapter locus's params
                            // block — opaque to the binding-spec layer.
                            if let TransportSpec::Unix { role, .. } =
                                &mut entry.transport
                            {
                                if role.is_none() {
                                    let p = pubs.contains(&entry.topic.name);
                                    let s = subs.contains(&entry.topic.name);
                                    // Typecheck emits a diag for (p && s)
                                    // and (!p && !s); here we only fill in
                                    // the unambiguous cases.
                                    if p && !s {
                                        *role = Some(TransportRole::Connect);
                                    } else if s && !p {
                                        *role = Some(TransportRole::Listen);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            TopDecl::Module(m) => fill_roles_in_items(&mut m.items, pubs, subs),
            _ => {}
        }
    }
}

fn collect_topics(items: &[TopDecl], topics: &mut BTreeMap<String, TopicEntry>) {
    // First gather raw decls (name → (payload, parent, subject))
    // by walking the program tree. Then resolve wire_subject
    // bottom-up by following parent chains.
    #[derive(Clone)]
    struct Raw {
        payload: TypeExpr,
        parent: Option<String>,
        subject: String,
    }
    let mut raw: BTreeMap<String, Raw> = BTreeMap::new();
    fn gather(items: &[TopDecl], raw: &mut BTreeMap<String, Raw>) {
        for item in items {
            match item {
                TopDecl::Topic(t) => {
                    let subject = t.subject.clone().unwrap_or_else(|| t.name.name.clone());
                    raw.insert(
                        t.name.name.clone(),
                        Raw {
                            payload: t.payload.clone(),
                            parent: t.parent.as_ref().map(|i| i.name.clone()),
                            subject,
                        },
                    );
                }
                TopDecl::Module(m) => gather(&m.items, raw),
                _ => {}
            }
        }
    }
    gather(items, &mut raw);

    // Resolve wire_subject for each. Cycles + missing parents
    // would have already been errored by the type-resolve pass;
    // here we treat them defensively (fall back to own subject).
    for (name, r) in raw.iter() {
        let mut chain: Vec<String> = vec![r.subject.clone()];
        let mut visited: Vec<String> = vec![name.clone()];
        let mut cur = r.parent.clone();
        while let Some(p) = cur {
            if visited.contains(&p) {
                // Cycle defense.
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
        topics.insert(
            name.clone(),
            TopicEntry {
                payload: r.payload.clone(),
                wire_subject: chain.join("."),
            },
        );
    }
}

fn rewrite_items(items: &mut [TopDecl], topics: &BTreeMap<String, TopicEntry>) {
    for item in items {
        match item {
            TopDecl::Locus(l) => rewrite_locus(l, topics),
            TopDecl::Fn(f) => rewrite_block(&mut f.body, topics),
            TopDecl::Module(m) => rewrite_items(&mut m.items, topics),
            _ => {}
        }
    }
}

fn rewrite_locus(l: &mut LocusDecl, topics: &BTreeMap<String, TopicEntry>) {
    for member in &mut l.members {
        match member {
            LocusMember::Bus(bb) => {
                for bm in &mut bb.members {
                    rewrite_bus_member(bm, topics);
                }
            }
            LocusMember::Lifecycle(lc) => rewrite_block(&mut lc.body, topics),
            LocusMember::Mode(md) => rewrite_block(&mut md.body, topics),
            LocusMember::Fn(fd) => rewrite_block(&mut fd.body, topics),
            _ => {}
        }
    }
}

fn rewrite_bus_member(bm: &mut BusMember, topics: &BTreeMap<String, TopicEntry>) {
    match bm {
        BusMember::Subscribe { subject, ty, .. } => {
            if let BusSubject::Topic(ident) = subject {
                let name = ident.name.clone();
                let span = ident.span;
                if let Some(entry) = topics.get(&name) {
                    if ty.is_none() {
                        *ty = Some(entry.payload.clone());
                    }
                    *subject = BusSubject::Literal {
                        subject: entry.wire_subject.clone(),
                        span,
                    };
                } else {
                    // Defensive: unresolved topic-ref keeps the
                    // ident name so a downstream "unknown subject"
                    // error has something to cite.
                    *subject = BusSubject::Literal { subject: name, span };
                }
            }
        }
        BusMember::Publish { subject, ty, .. } => {
            if let BusSubject::Topic(ident) = subject {
                let name = ident.name.clone();
                let span = ident.span;
                if let Some(entry) = topics.get(&name) {
                    if ty.is_none() {
                        *ty = Some(entry.payload.clone());
                    }
                    *subject = BusSubject::Literal {
                        subject: entry.wire_subject.clone(),
                        span,
                    };
                } else {
                    *subject = BusSubject::Literal { subject: name, span };
                }
            }
        }
    }
}

fn rewrite_block(b: &mut Block, topics: &BTreeMap<String, TopicEntry>) {
    for stmt in &mut b.stmts {
        rewrite_stmt(stmt, topics);
    }
    if let Some(tail) = &mut b.tail {
        rewrite_expr(tail, topics);
    }
}

fn rewrite_stmt(s: &mut Stmt, topics: &BTreeMap<String, TopicEntry>) {
    match s {
        Stmt::Send { subject, .. } => {
            // Rewrite `Foo <- value` to `"<wire_subject>" <- value`
            // when `Foo` is a declared topic. Subject is the only
            // place a topic ident appears in expression position
            // (typechecker rejects topic idents elsewhere).
            if let Expr::Ident(id) = subject {
                if let Some(entry) = topics.get(&id.name) {
                    let span = id.span;
                    *subject = Expr::Literal(
                        Literal::String(entry.wire_subject.clone()),
                        span,
                    );
                }
            }
        }
        Stmt::If(if_stmt) => rewrite_if(if_stmt, topics),
        Stmt::Match(m) => rewrite_match(m, topics),
        Stmt::For { body, .. } => rewrite_block(body, topics),
        Stmt::While { body, .. } => rewrite_block(body, topics),
        Stmt::Block(b) => rewrite_block(b, topics),
        Stmt::Expr(e) => rewrite_expr(e, topics),
        _ => {}
    }
}

fn rewrite_if(if_stmt: &mut IfStmt, topics: &BTreeMap<String, TopicEntry>) {
    rewrite_block(&mut if_stmt.then_block, topics);
    if let Some(else_branch) = &mut if_stmt.else_block {
        rewrite_else_branch(else_branch, topics);
    }
}

fn rewrite_else_branch(eb: &mut ElseBranch, topics: &BTreeMap<String, TopicEntry>) {
    match eb {
        ElseBranch::Else(b) => rewrite_block(b, topics),
        ElseBranch::ElseIf(if_stmt) => rewrite_if(if_stmt, topics),
    }
}

fn rewrite_match(m: &mut MatchStmt, topics: &BTreeMap<String, TopicEntry>) {
    for arm in &mut m.arms {
        match &mut arm.body {
            MatchArmBody::Block(b) => rewrite_block(b, topics),
            MatchArmBody::Expr(e) => rewrite_expr(e, topics),
        }
    }
}

// ---------------------------------------------------------------
// Proposal A′: repr-tagged field accessors.
// ---------------------------------------------------------------
//
// A struct whose fields carry `repr:"<wire-type>"` tags is a binary
// (wire) layout. For such a type `L2`, a call `L2::price(v)` reads the
// `price` field at its computed byte offset from `v` (a Bytes/BytesView),
// and `L2::set_price(w, x)` writes it into `w` (a BytesMut). This pass
// rewrites those accessor calls into the equivalent `std::bytes::read_*` /
// `write_*` calls so the rest of the pipeline (typecheck, codegen) needs
// no accessor-specific logic — they ride the existing pack primitives,
// including the `fallible(IndexError)` typing. Runs before typecheck (so
// the checker sees the desugared calls) and is idempotent.

/// A field's wire placement, parsed from its `repr:"…"` tag.
#[derive(Debug, Clone)]
struct WireField {
    offset: u64,
    /// The pack-primitive type token, e.g. `"u32_le"` — `read_`/`write_`
    /// prefix it to name the runtime call.
    repr: String,
}

type WireLayouts = BTreeMap<String, BTreeMap<String, WireField>>;

/// Extract a Go-style `key:"value"` from a struct field's backtick tag
/// string. Shared by the `repr:` accessor desugar and the typechecker's
/// accessor validation.
pub fn tag_value(tag: &str, key: &str) -> Option<String> {
    let needle = format!("{}:\"", key);
    let start = tag.find(&needle)? + needle.len();
    let rest = &tag[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Byte width of a pack-primitive type token (endianness suffix ignored).
fn repr_width(repr: &str) -> u64 {
    let base = repr
        .strip_suffix("_le")
        .or_else(|| repr.strip_suffix("_be"))
        .unwrap_or(repr);
    match base {
        "u8" | "i8" => 1,
        "u16" | "i16" => 2,
        "u32" | "i32" | "f32" => 4,
        _ => 8,
    }
}

/// Compute a struct's wire layout from its fields' `repr:` tags. Offsets
/// run in declaration order over the tagged fields (each advances the
/// cursor by its width); an explicit `,at=N` in the tag pins an offset
/// for a foreign format with padding. Returns `None` if no field is
/// repr-tagged (an ordinary struct).
fn wire_layout(fields: &[StructField]) -> Option<BTreeMap<String, WireField>> {
    let mut layout = BTreeMap::new();
    let mut cursor = 0u64;
    for f in fields {
        let Some(repr_val) = f.tag.as_deref().and_then(|t| tag_value(t, "repr")) else {
            continue;
        };
        let mut parts = repr_val.split(',');
        let repr = parts.next().unwrap_or("").trim().to_string();
        let mut at: Option<u64> = None;
        for p in parts {
            if let Some(n) = p.trim().strip_prefix("at=") {
                at = n.trim().parse().ok();
            }
        }
        let width = repr_width(&repr);
        let offset = at.unwrap_or(cursor);
        cursor = offset + width;
        layout.insert(f.name.name.clone(), WireField { offset, repr });
    }
    if layout.is_empty() {
        None
    } else {
        Some(layout)
    }
}

fn collect_wire_layouts(items: &[TopDecl], out: &mut WireLayouts) {
    for item in items {
        match item {
            TopDecl::Type(td) => {
                if let TypeDeclBody::Struct(fields) = &td.body {
                    if let Some(layout) = wire_layout(fields) {
                        out.insert(td.name.name.clone(), layout);
                    }
                }
            }
            TopDecl::Module(m) => collect_wire_layouts(&m.items, out),
            _ => {}
        }
    }
}

/// Rewrite repr-tagged field accessors into `std::bytes::*` calls.
pub fn desugar_repr_accessors(program: &mut Program) {
    let mut layouts = WireLayouts::new();
    collect_wire_layouts(&program.items, &mut layouts);
    if layouts.is_empty() {
        return;
    }
    acc_items(&mut program.items, &layouts);
}

fn std_bytes_call(name: &str, args: Vec<Expr>, span: Span) -> Expr {
    let seg = |s: &str| Ident {
        name: s.to_string(),
        span,
    };
    Expr::Call {
        callee: Box::new(Expr::Path(QualifiedName {
            segments: vec![seg("std"), seg("bytes"), seg(name)],
            span,
        })),
        args,
        span,
    }
}

/// If `callee(args)` is an accessor of a wire-tagged type, return its
/// `std::bytes::*` replacement.
fn try_accessor(
    callee: &Expr,
    args: &[Expr],
    span: Span,
    w: &WireLayouts,
) -> Option<Expr> {
    let Expr::Path(qn) = callee else { return None };
    if qn.segments.len() != 2 {
        return None;
    }
    let layout = w.get(&qn.segments[0].name)?;
    let member = &qn.segments[1].name;
    // Read: `T::field(v)` → `std::bytes::read_<repr>(v, off)`.
    if let Some(wf) = layout.get(member) {
        if args.len() != 1 {
            return None;
        }
        return Some(std_bytes_call(
            &format!("read_{}", wf.repr),
            vec![args[0].clone(), Expr::Literal(Literal::Int(wf.offset as i64), span)],
            span,
        ));
    }
    // Write: `T::set_field(w, x)` → `std::bytes::write_<repr>(w, off, x)`.
    if let Some(field) = member.strip_prefix("set_") {
        if let Some(wf) = layout.get(field) {
            if args.len() != 2 {
                return None;
            }
            return Some(std_bytes_call(
                &format!("write_{}", wf.repr),
                vec![
                    args[0].clone(),
                    Expr::Literal(Literal::Int(wf.offset as i64), span),
                    args[1].clone(),
                ],
                span,
            ));
        }
    }
    None
}

fn acc_items(items: &mut [TopDecl], w: &WireLayouts) {
    for item in items {
        match item {
            TopDecl::Fn(f) => acc_block(&mut f.body, w),
            TopDecl::Locus(l) => {
                for member in &mut l.members {
                    match member {
                        LocusMember::Lifecycle(lc) => acc_block(&mut lc.body, w),
                        LocusMember::Mode(md) => acc_block(&mut md.body, w),
                        LocusMember::Fn(fd) => acc_block(&mut fd.body, w),
                        _ => {}
                    }
                }
            }
            TopDecl::Module(m) => acc_items(&mut m.items, w),
            _ => {}
        }
    }
}

fn acc_block(b: &mut Block, w: &WireLayouts) {
    for s in &mut b.stmts {
        acc_stmt(s, w);
    }
    if let Some(t) = &mut b.tail {
        acc_expr(t, w);
    }
}

fn acc_stmt(s: &mut Stmt, w: &WireLayouts) {
    match s {
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => acc_expr(value, w),
        Stmt::Assign { target, value, .. } => {
            acc_expr(value, w);
            for seg in &mut target.tail {
                if let LValueSeg::Index(e) = seg {
                    acc_expr(e, w);
                }
            }
        }
        Stmt::If(if_stmt) => acc_if(if_stmt, w),
        Stmt::Match(m) => acc_match(m, w),
        Stmt::For { iter, body, .. } => {
            acc_expr(iter, w);
            acc_block(body, w);
        }
        Stmt::While { cond, body, .. } => {
            acc_expr(cond, w);
            acc_block(body, w);
        }
        Stmt::Return(Some(e), _) => acc_expr(e, w),
        Stmt::Fail { value, .. } => acc_expr(value, w),
        Stmt::Send { subject, value, .. } => {
            acc_expr(subject, w);
            acc_expr(value, w);
        }
        Stmt::Block(b) => acc_block(b, w),
        Stmt::Recovery { args, .. } => {
            for a in args {
                acc_expr(a, w);
            }
        }
        Stmt::Violate { payload, .. } => {
            if let Some(p) = payload {
                acc_expr(p, w);
            }
        }
        Stmt::ShmWrite { max, body, .. } => {
            acc_expr(max, w);
            acc_block(body, w);
        }
        Stmt::Expr(e) => acc_expr(e, w),
        Stmt::Return(None, _)
        | Stmt::Break(_)
        | Stmt::Continue(_)
        | Stmt::Yield(_)
        | Stmt::Terminate(_) => {}
    }
}

fn acc_if(if_stmt: &mut IfStmt, w: &WireLayouts) {
    acc_expr(&mut if_stmt.cond, w);
    acc_block(&mut if_stmt.then_block, w);
    if let Some(eb) = &mut if_stmt.else_block {
        match eb.as_mut() {
            ElseBranch::Else(b) => acc_block(b, w),
            ElseBranch::ElseIf(inner) => acc_if(inner, w),
        }
    }
}

fn acc_match(m: &mut MatchStmt, w: &WireLayouts) {
    acc_expr(&mut m.scrutinee, w);
    for arm in &mut m.arms {
        match &mut arm.body {
            MatchArmBody::Block(b) => acc_block(b, w),
            MatchArmBody::Expr(e) => acc_expr(e, w),
        }
    }
}

fn acc_expr(e: &mut Expr, w: &WireLayouts) {
    match e {
        Expr::Call { callee, args, span } => {
            acc_expr(callee, w);
            for a in args.iter_mut() {
                acc_expr(a, w);
            }
            if let Some(rewritten) = try_accessor(callee, args, *span, w) {
                *e = rewritten;
            }
        }
        Expr::Binary { left, right, .. } => {
            acc_expr(left, w);
            acc_expr(right, w);
        }
        Expr::Unary { operand, .. } => acc_expr(operand, w),
        Expr::Field { receiver, .. } => acc_expr(receiver, w),
        Expr::Index { receiver, index, .. } => {
            acc_expr(receiver, w);
            acc_expr(index, w);
        }
        Expr::Path2 { receiver, .. } => acc_expr(receiver, w),
        Expr::Tuple(es, _) | Expr::Array(es, _) => {
            for x in es {
                acc_expr(x, w);
            }
        }
        Expr::Struct { inits, .. } => {
            for i in inits {
                acc_expr(&mut i.value, w);
            }
        }
        Expr::Block(b) => acc_block(b, w),
        Expr::If(if_stmt) => acc_if(if_stmt, w),
        Expr::Match(m) => acc_match(m, w),
        Expr::Sum(inner, _) | Expr::Prod(inner, _) => acc_expr(inner, w),
        Expr::Approx { left, right, tolerance, .. } => {
            acc_expr(left, w);
            acc_expr(right, w);
            acc_expr(tolerance, w);
        }
        Expr::Range { lo, hi, .. } => {
            acc_expr(lo, w);
            acc_expr(hi, w);
        }
        Expr::ArrayRepeat { val, .. } => acc_expr(val, w),
        Expr::Or { inner, disposition, .. } => {
            acc_expr(inner, w);
            if let OrDisposition::Substitute(s) = disposition {
                acc_expr(s, w);
            }
        }
        Expr::Literal(..) | Expr::Ident(_) | Expr::Path(_) | Expr::KwSelf(_) => {}
    }
}

fn rewrite_expr(e: &mut Expr, topics: &BTreeMap<String, TopicEntry>) {
    match e {
        Expr::Block(b) => rewrite_block(b, topics),
        Expr::If(if_stmt) => rewrite_if(if_stmt, topics),
        Expr::Match(m) => rewrite_match(m, topics),
        _ => {}
    }
}

// ---------------------------------------------------------------
// Closed-world topology optimization: intra-locus + intra-tower
// direct call.
// ---------------------------------------------------------------
//
// A topic is "closed-world optimizable" when ALL of:
//   - no `bindings { Topic: ... }` entry references it
//   - exactly one locus type publishes it (call this P)
//   - exactly one locus type subscribes it (call this S)
//
// And EITHER:
//   (a) P == S — every Send happens inside an instance of the
//       same locus that hosts the handler. Rewrite to
//       `self.handler(payload)`.
//   (b) P contains exactly one direct singleton field of type S
//       (declared in P's `params` block). Every Send in P's body
//       statically routes to that one child. Rewrite to
//       `self.<field>.handler(payload)`.
//
// In both cases the publish→queue→drain→dispatch path collapses
// to a synchronous method call. The publish/subscribe entries
// stay in place so they keep type-checking; the bus runtime
// simply never sees traffic on the optimized subject.
//
// Multi-hop towers (P → I → S via two field accesses), plural
// child slots (`@form(vec) of S`), and child-publishes-parent-
// subscribes are intentionally out of scope for v1 — they each
// have their own design surface (broadcast semantics, parent
// reference mechanism). Falling through to the bus is always
// correct; we only optimize the unambiguous singleton cases.
//
// Run BEFORE desugar_topics so the Send still has its
// `Expr::Ident(Topic)` shape (post-desugar, it'd be a literal
// string and we'd lose the cheap topic-name lookup).

/// Description of how to rewrite a Send for an eligible topic.
/// `access_chain` lists the receivers to apply between `self`
/// and the final method call: a single-element chain `[H]` is
/// the same-locus case (`self.H(v)`); a two-element chain
/// `[f, H]` is the parent-publishes-child-subscribes case
/// (`self.f.H(v)`).
#[derive(Debug, Clone)]
struct EligibleRewrite {
    publisher_locus: String,
    access_chain: Vec<String>,
}

/// Intra-locus / intra-tower closed-world optimization entry
/// point. Mutates `program` in place. Idempotent: re-running on
/// already-optimized input is a no-op (rewritten Sends become
/// method-call Stmt::Expr nodes, which the rewrite step skips).
pub fn desugar_intra_locus_topics(program: &mut Program) {
    let bindings = collect_bindings(&program.items);
    let (pubs, subs) = collect_pub_sub(&program.items);
    let locus_types = collect_locus_type_names(&program.items);
    let locus_fields = collect_locus_typed_fields(&program.items, &locus_types);
    // F.31 pool-safety (2026-05-31): the set of (owner_locus,
    // field) pairs whose `placement { }` puts the field-child on
    // a thread OTHER than its owner's (a named cooperative pool
    // that isn't `main`, or a pinned thread). The intra-locus
    // rewrite below turns a publish into a *direct, synchronous*
    // method call on the publisher's own thread — correct only
    // when publisher and subscriber share an execution context.
    // For an off-thread subscriber that bypasses the bus's
    // `lotus_coop_pool_post` routing: the handler would run on the
    // publisher's thread (e.g. main), violating the
    // single-threaded-pool invariant AND dropping the pool context
    // that any locus the handler instantiates needs to inherit
    // (an accept'd child's run() would then go synchronous + its
    // subscriptions register on the global queue — observed as a
    // per-connection handler blocking the main thread in accept()).
    // Such publishes must stay on the bus dispatch path. Same-locus
    // (case a) and non-main-owner field-children (placement is a
    // main-locus-only seam, so those share the owner's pool) are
    // unaffected.
    let placed_off_owner_thread = collect_off_owner_thread_fields(&program.items);
    // Phase 3 (2026-05-25): collect the set of topic names that
    // declare `keyed_by` (or `on_unmatched`). The intra-locus
    // optimization rewrites publishes to direct method calls,
    // bypassing the bus runtime — which means the routing-key
    // filter never runs. For keyed topics the filter is the
    // whole point, so skip the optimization here and let the
    // publish fall through to the bus dispatch path.
    //
    // A keyed topic could still in principle benefit from the
    // optimization if every subscriber's filter is a literal
    // that matches every possible payload key, but enumerating
    // that is workload-driven and YAGNI for v0.1.
    let keyed_topics: std::collections::BTreeSet<String> = program
        .items
        .iter()
        .filter_map(|d| match d {
            TopDecl::Topic(t)
                if t.keyed_by.is_some() || t.on_unmatched.is_some() =>
            {
                Some(t.name.name.clone())
            }
            _ => None,
        })
        .collect();

    // Identify topic → rewrite recipe for each eligible topic.
    let mut eligible: BTreeMap<String, EligibleRewrite> = BTreeMap::new();
    for (topic, pub_loci) in &pubs {
        if bindings.contains(topic) {
            continue;
        }
        if keyed_topics.contains(topic) {
            continue;
        }
        if pub_loci.len() != 1 {
            continue;
        }
        let pub_locus = pub_loci[0].clone();
        let sub_pairs = match subs.get(topic) {
            Some(s) => s,
            None => continue,
        };
        if sub_pairs.len() != 1 {
            continue;
        }
        let (sub_locus, handler) = &sub_pairs[0];

        if sub_locus == &pub_locus {
            // (a) same-locus case
            eligible.insert(
                topic.clone(),
                EligibleRewrite {
                    publisher_locus: pub_locus,
                    access_chain: vec![handler.clone()],
                },
            );
            continue;
        }

        // (b) parent-publishes-child-subscribes case: P (the
        // publisher) must contain exactly one direct singleton
        // field whose type names S (the subscriber).
        let fields_of_s: Vec<&String> = locus_fields
            .get(&pub_locus)
            .map(|fields| {
                fields
                    .iter()
                    .filter_map(|(fname, fty)| if fty == sub_locus { Some(fname) } else { None })
                    .collect()
            })
            .unwrap_or_default();
        if fields_of_s.len() != 1 {
            // Zero matches → not a tower edge, leave to bus.
            // Multiple matches → ambiguous which child receives,
            // leave to bus (which broadcasts to all subscribers).
            continue;
        }
        let field = fields_of_s[0].clone();
        // F.31 pool-safety: if this field-child is placed on a
        // thread other than its owner's, the publish must route
        // through the bus (which posts to the child's pool), not a
        // direct same-thread call. See `placed_off_owner_thread`.
        if placed_off_owner_thread.contains(&(pub_locus.clone(), field.clone())) {
            continue;
        }
        eligible.insert(
            topic.clone(),
            EligibleRewrite {
                publisher_locus: pub_locus,
                access_chain: vec![field, handler.clone()],
            },
        );
    }

    if eligible.is_empty() {
        return;
    }

    // Walk locus methods and rewrite matching Sends.
    for item in &mut program.items {
        match item {
            TopDecl::Locus(l) => intra_rewrite_locus(l, &eligible),
            TopDecl::Module(m) => intra_rewrite_module(m, &eligible),
            _ => {}
        }
    }
}

/// Set of (owner_locus, field) pairs whose `placement { }` entry
/// puts the field-child on a thread other than the owner's: a
/// named cooperative pool that isn't `main`, or a pinned thread.
/// `cooperative` with no pool (or `pool = main`) keeps the child
/// on the owner's (main) thread, so a direct same-thread call is
/// safe and stays eligible for the intra-locus rewrite.
///
/// F.31 scopes `placement { }` to the main locus, so in practice
/// only main-locus fields appear here — but the walk is
/// locus-agnostic, so a future non-main placement seam is covered
/// automatically.
fn collect_off_owner_thread_fields(
    items: &[TopDecl],
) -> std::collections::BTreeSet<(String, String)> {
    let mut out = std::collections::BTreeSet::new();
    fn walk(
        items: &[TopDecl],
        out: &mut std::collections::BTreeSet<(String, String)>,
    ) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    for member in &l.members {
                        if let LocusMember::Placement(pb) = member {
                            for e in &pb.entries {
                                let off_thread = match &e.spec {
                                    PlacementSpec::Cooperative { pool } => pool
                                        .as_ref()
                                        .is_some_and(|p| p.name != "main"),
                                    PlacementSpec::Pinned { .. } => true,
                                };
                                if off_thread {
                                    out.insert((
                                        l.name.name.clone(),
                                        e.field.name.clone(),
                                    ));
                                }
                            }
                        }
                    }
                }
                TopDecl::Module(m) => walk(&m.items, out),
                _ => {}
            }
        }
    }
    walk(items, &mut out);
    out
}

/// Set of every declared locus type name in the program (across
/// all module nesting). Used to recognize "this field's type
/// names another locus" without consulting the typechecker.
fn collect_locus_type_names(items: &[TopDecl]) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    fn walk(items: &[TopDecl], out: &mut std::collections::BTreeSet<String>) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    out.insert(l.name.name.clone());
                }
                TopDecl::Module(m) => walk(&m.items, out),
                _ => {}
            }
        }
    }
    walk(items, &mut out);
    out
}

/// For each locus type, the list of (field_name, locus_type) for
/// each `params` field whose declared type names another locus.
/// Used to find tower edges at desugar time. Only direct fields
/// are tracked — capacity slots (`pool/heap/vec/recpool`) are
/// deliberately skipped because their semantics (plural, indexed,
/// recycled) don't match the closed-world singleton rewrite.
fn collect_locus_typed_fields(
    items: &[TopDecl],
    locus_types: &std::collections::BTreeSet<String>,
) -> BTreeMap<String, Vec<(String, String)>> {
    let mut out: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    fn walk(
        items: &[TopDecl],
        locus_types: &std::collections::BTreeSet<String>,
        out: &mut BTreeMap<String, Vec<(String, String)>>,
    ) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    let owner = l.name.name.clone();
                    for member in &l.members {
                        if let LocusMember::Params(pb) = member {
                            for p in &pb.params {
                                if let Some(ty) = &p.ty {
                                    if let Some(name) = single_named_locus(ty, locus_types) {
                                        out.entry(owner.clone())
                                            .or_default()
                                            .push((p.name.name.clone(), name));
                                    }
                                }
                            }
                        }
                    }
                }
                TopDecl::Module(m) => walk(&m.items, locus_types, out),
                _ => {}
            }
        }
    }
    walk(items, locus_types, &mut out);
    out
}

/// If `ty` names a single locus type (an unqualified or
/// last-segment-matches form like `Foo` or `pond::Foo`) declared
/// in the program, return its name. Otherwise None. Projection,
/// array, tuple, and function types are deliberately skipped —
/// they aren't singleton-locus shapes.
fn single_named_locus(
    ty: &TypeExpr,
    locus_types: &std::collections::BTreeSet<String>,
) -> Option<String> {
    match ty {
        TypeExpr::Named { path, generic_args, .. } if generic_args.is_empty() => {
            let last = path.segments.last()?.name.clone();
            if locus_types.contains(&last) {
                Some(last)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn intra_rewrite_module(
    m: &mut ModuleDecl,
    eligible: &BTreeMap<String, EligibleRewrite>,
) {
    for item in &mut m.items {
        match item {
            TopDecl::Locus(l) => intra_rewrite_locus(l, eligible),
            TopDecl::Module(inner) => intra_rewrite_module(inner, eligible),
            _ => {}
        }
    }
}

/// Rewrite all Send sites inside `l`'s method bodies. Only the
/// loci named as the publisher of an eligible topic get sends
/// rewritten — others may publish to other (unoptimized) topics
/// from the same lexical position, so we have to gate by the
/// (current locus name, topic name) pair.
fn intra_rewrite_locus(
    l: &mut LocusDecl,
    eligible: &BTreeMap<String, EligibleRewrite>,
) {
    let locus_name = l.name.name.clone();
    for member in &mut l.members {
        match member {
            LocusMember::Lifecycle(lc) => {
                intra_rewrite_block(&mut lc.body, &locus_name, eligible);
            }
            LocusMember::Mode(md) => {
                intra_rewrite_block(&mut md.body, &locus_name, eligible);
            }
            LocusMember::Fn(fd) => {
                intra_rewrite_block(&mut fd.body, &locus_name, eligible);
            }
            LocusMember::Failure(f) => {
                intra_rewrite_block(&mut f.body, &locus_name, eligible);
            }
            _ => {}
        }
    }
}

fn intra_rewrite_block(
    b: &mut Block,
    locus_name: &str,
    eligible: &BTreeMap<String, EligibleRewrite>,
) {
    for stmt in &mut b.stmts {
        intra_rewrite_stmt(stmt, locus_name, eligible);
    }
    if let Some(tail) = &mut b.tail {
        intra_rewrite_expr(tail, locus_name, eligible);
    }
}

/// Build the `self.<chain[0]>.<chain[1]>...(value)` expression
/// from an access chain. The chain's final segment is the method
/// name; all preceding segments are field accesses through which
/// the receiver is traversed.
fn build_chained_call(access_chain: &[String], value: Expr, span: Span) -> Expr {
    // Start from `self`, walk all but the last segment as field
    // accesses, then call the last segment as a method on the
    // accumulated receiver.
    let (method_name, fields) = access_chain
        .split_last()
        .expect("eligible access chain is never empty");
    let mut receiver = Expr::KwSelf(span);
    for field in fields {
        receiver = Expr::Field {
            receiver: Box::new(receiver),
            name: Ident { name: field.clone(), span },
            span,
        };
    }
    Expr::Call {
        callee: Box::new(Expr::Field {
            receiver: Box::new(receiver),
            name: Ident { name: method_name.clone(), span },
            span,
        }),
        args: vec![value],
        span,
    }
}

fn intra_rewrite_stmt(
    s: &mut Stmt,
    locus_name: &str,
    eligible: &BTreeMap<String, EligibleRewrite>,
) {
    if let Stmt::Send { subject, value, span, .. } = s {
        if let Expr::Ident(id) = subject {
            if let Some(rw) = eligible.get(&id.name) {
                if rw.publisher_locus == locus_name {
                    let span = *span;
                    let value_expr = std::mem::replace(
                        value,
                        Expr::Literal(Literal::Bool(false), span),
                    );
                    let call_expr = build_chained_call(&rw.access_chain, value_expr, span);
                    *s = Stmt::Expr(call_expr);
                    return;
                }
            }
        }
    }
    match s {
        Stmt::If(if_stmt) => intra_rewrite_if(if_stmt, locus_name, eligible),
        Stmt::Match(m) => intra_rewrite_match(m, locus_name, eligible),
        Stmt::For { body, .. } => intra_rewrite_block(body, locus_name, eligible),
        Stmt::While { body, .. } => intra_rewrite_block(body, locus_name, eligible),
        Stmt::Block(b) => intra_rewrite_block(b, locus_name, eligible),
        Stmt::Expr(e) => intra_rewrite_expr(e, locus_name, eligible),
        _ => {}
    }
}

fn intra_rewrite_if(
    if_stmt: &mut IfStmt,
    locus_name: &str,
    eligible: &BTreeMap<String, EligibleRewrite>,
) {
    intra_rewrite_block(&mut if_stmt.then_block, locus_name, eligible);
    if let Some(eb) = &mut if_stmt.else_block {
        match eb.as_mut() {
            ElseBranch::Else(b) => intra_rewrite_block(b, locus_name, eligible),
            ElseBranch::ElseIf(inner) => intra_rewrite_if(inner, locus_name, eligible),
        }
    }
}

fn intra_rewrite_match(
    m: &mut MatchStmt,
    locus_name: &str,
    eligible: &BTreeMap<String, EligibleRewrite>,
) {
    for arm in &mut m.arms {
        match &mut arm.body {
            MatchArmBody::Block(b) => intra_rewrite_block(b, locus_name, eligible),
            MatchArmBody::Expr(e) => intra_rewrite_expr(e, locus_name, eligible),
        }
    }
}

fn intra_rewrite_expr(
    e: &mut Expr,
    locus_name: &str,
    eligible: &BTreeMap<String, EligibleRewrite>,
) {
    match e {
        Expr::Block(b) => intra_rewrite_block(b, locus_name, eligible),
        Expr::If(if_stmt) => intra_rewrite_if(if_stmt, locus_name, eligible),
        Expr::Match(m) => intra_rewrite_match(m, locus_name, eligible),
        _ => {}
    }
}

/// Topology collection helpers. Walk the program tree once,
/// gathering: which topics are bound (any binding entry), which
/// loci publish each topic, and which loci subscribe each topic
/// (with the handler ident).
fn collect_bindings(items: &[TopDecl]) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    fn walk(items: &[TopDecl], out: &mut std::collections::BTreeSet<String>) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        if let LocusMember::Bindings(bb) = m {
                            for entry in &bb.entries {
                                out.insert(entry.topic.name.clone());
                            }
                        }
                    }
                }
                TopDecl::Module(m) => walk(&m.items, out),
                _ => {}
            }
        }
    }
    walk(items, &mut out);
    out
}

#[allow(clippy::type_complexity)]
fn collect_pub_sub(
    items: &[TopDecl],
) -> (
    BTreeMap<String, Vec<String>>,
    BTreeMap<String, Vec<(String, String)>>,
) {
    let mut pubs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut subs: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    fn walk(
        items: &[TopDecl],
        pubs: &mut BTreeMap<String, Vec<String>>,
        subs: &mut BTreeMap<String, Vec<(String, String)>>,
    ) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    let locus_name = l.name.name.clone();
                    for member in &l.members {
                        if let LocusMember::Bus(bb) = member {
                            for bm in &bb.members {
                                match bm {
                                    BusMember::Subscribe { subject, handler, .. } => {
                                        if let BusSubject::Topic(id) = subject {
                                            subs.entry(id.name.clone())
                                                .or_default()
                                                .push((locus_name.clone(), handler.name.clone()));
                                        }
                                    }
                                    BusMember::Publish { subject, .. } => {
                                        if let BusSubject::Topic(id) = subject {
                                            pubs.entry(id.name.clone())
                                                .or_default()
                                                .push(locus_name.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                TopDecl::Module(m) => walk(&m.items, pubs, subs),
                _ => {}
            }
        }
    }
    walk(items, &mut pubs, &mut subs);
    (pubs, subs)
}
