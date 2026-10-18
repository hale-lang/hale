//! GH #1417 (R2a): `api::serve` builds an exposure.
//!
//! A serve site is read by the check as the call it is (`surfaces.rs`
//! names it, the serve-site laws judge it); this pass, which runs in the
//! desugar sequence before the check (`desugar_sequence`), is what makes
//! a program that holds one buildable. For each well-formed site it adds,
//! as `api_gen` does for the structural path, source the compiler writes:
//!
//! - the exposure param itself (GH #1565: `web: api::Exposure =
//!   api::serve(…);`, a param of the locus that holds it) gets the
//!   exposure as its init, `std::api::Exposure { … }` built from the site
//!   (its surface, digest, bound, transport instance, the rows adapter
//!   below, the bearer and role sources copied from the transport's
//!   fields); the call as written is kept beside it (`ParamDecl::serve`)
//!   for the serve-site laws and the description;
//! - for a socket transport, its listener: a param of the main locus,
//!   born just before the exposure when main holds it, just after the
//!   param a server below main sits under otherwise (reading its address
//!   off the server as built, `address_for_main`), placed by the
//!   exposure's own row when the
//!   main locus holds it, by the row of the param that holds its holder
//!   otherwise, when either puts it on a pool `where async_io`, and on the
//!   transport's shared pool (`__api_http`, `__api_unix`, `__api_grpc`)
//!   when neither does; and, for a main locus that writes no `run()`, the
//!   run that waits until its exposures stop;
//! - a top-level locus `__RpcSurface_<n>` implementing
//!   `std::api::Surface`: the rows as if-chains (member lookup, `requires`,
//!   receiver slot, decoding by shape, the roles that confer a role);
//! - on each receiver type, the plumbing of spec/api.md § Receiver
//!   failure and generations: a number per exposure that binds it (a
//!   param, written into the literal that builds the bound instance), an
//!   incarnation stamp set at every `birth()`, a keyed subscription to the
//!   exposure's calls and hello, and the thunk that runs a call through
//!   the fallible ABI and publishes the outcome;
//! - once, the five topics the runtime's wire subjects name, and the JSON
//!   codecs of the types the rows carry.
//!
//! The pass is idempotent (a site whose param is there is left alone) and
//! conservative: a site it cannot resolve is left as written, for the
//! serve-site laws to report.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::api_names::API_SYNTH_BASE;
use hale_syntax::ast::{
    flat_decls, Block, Expr, Ident, LifecycleKind, Literal, LocusDecl, LocusMember, ParamDecl, ParamInit,
    Program, Stmt, StructInit, TopDecl, TypeExpr,
};
use hale_syntax::json_gen::Conv;
use hale_syntax::{parse_source_at, Span};

use crate::topic_identity::{shape_hash, ShapeForm, Shapes, TypeClass};

/// What the pass made of one serve site.
#[derive(Debug, Clone)]
pub struct Expansion {
    /// The exposure's number: unique in the program, from 1.
    pub id: i64,
    /// `as:`.
    pub name: String,
    pub surface: String,
    pub digest: u64,
    /// The locus whose body holds the site.
    pub serving: String,
    /// The param of the serving locus that is the exposure.
    pub param: String,
}

/// One row of a surface, as the pass reads it.
#[derive(Debug, Clone)]
struct Row {
    member: String,
    locus: String,
    method: String,
    requires: Vec<String>,
    written_at: usize,
    takes_ctx: bool,
    served_ctx: bool,
    request: Option<TypeExpr>,
    response: Option<TypeExpr>,
    error: Option<TypeExpr>,
    server_error: bool,
}

/// A serve site, owned.
struct Site {
    program: usize,
    serving: String,
    /// The exposure param that declares it.
    param: String,
    surface: String,
    transport: Option<Expr>,
    name: String,
    bound: i64,
    /// For a holder that is not a main locus and serves over a socket: each
    /// main locus that holds it, the chain, and the listener's address as
    /// that main locus evaluates it.
    chains: Vec<(Chain, Expr)>,
}

fn lit_int(n: i64, span: Span) -> Expr {
    Expr::Literal(Literal::Int(n), span)
}

fn fresh(e: &Expr) -> Expr {
    let mut e = e.clone();
    hale_syntax::sites::clear_ids_in_expr(&mut e);
    e
}

fn locus_param_ty(l: &LocusDecl, field: &str) -> Option<String> {
    crate::surfaces::param_types(l).get(field).cloned()
}

fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The rows of `surface`: the members `surfaces::select_members` selects
/// (the one rule, shared with `surface_rows`), read as the adapter needs
/// them. A free fn is no receiver's; an `@rpc` row the pass cannot read is
/// left out; an `api` row it cannot read leaves the whole surface
/// unexpanded (the admission law reports it). In canonical order: member
/// as bytes, then as written.
fn rows_of_surface(
    programs: &[&Program],
    surface: &str,
    renames: &[(Vec<String>, String)],
    default_surface: &str,
) -> Vec<Row> {
    let mut loci: BTreeMap<&str, &LocusDecl> = BTreeMap::new();
    for p in programs {
        for d in flat_decls(&p.items) {
            match d {
                TopDecl::Locus(l) => {
                    loci.insert(l.name.name.as_str(), l);
                }
                _ => {}
            }
        }
    }
    let row_of = |member: String, locus: &str, method: &str, requires: Vec<String>, at: usize| -> Option<Row> {
        let l = loci.get(locus)?;
        let f = l.members.iter().find_map(|m| match m {
            LocusMember::Fn(f) if f.name.name == method => Some(f),
            _ => None,
        })?;
        let served_ctx = f.params.last().is_some_and(|p| hale_syntax::api_names::is_served_context_type(&p.ty));
        let takes_ctx = served_ctx || f.params.last().is_some_and(|p| hale_syntax::api_names::is_context_type(&p.ty));
        let value: Vec<&hale_syntax::ast::Param> =
            f.params.iter().take(f.params.len() - usize::from(takes_ctx)).collect();
        if value.len() > 1 {
            return None;
        }
        let response = match &f.ret {
            None => None,
            Some(TypeExpr::Tuple(parts, _)) if parts.is_empty() => None,
            Some(t) => Some(t.clone()),
        };
        let server_error = matches!(&f.fallible, Some(TypeExpr::Named { path, .. })
            if path.segments.last().is_some_and(|s| s.name == "ClosureViolation"));
        Some(Row {
            member,
            locus: locus.to_string(),
            method: method.to_string(),
            requires,
            written_at: at,
            takes_ctx,
            served_ctx,
            request: value.first().map(|p| p.ty.clone()),
            response,
            error: f.fallible.clone(),
            server_error,
        })
    };
    let mut rows: Vec<Row> = Vec::new();
    for m in crate::surfaces::select_members(programs, Some(surface), renames, default_surface) {
        if m.locus.is_empty() {
            continue;
        }
        let requires = m.requires.iter().map(|(n, _)| n.clone()).collect();
        match row_of(m.member, &m.locus, &m.method, requires, m.written_at) {
            Some(row) => rows.push(row),
            None if m.origin == crate::surfaces::MemberOrigin::Block => return Vec::new(),
            None => {}
        }
    }
    rows.sort_by(|a, b| (a.member.as_bytes(), a.written_at).cmp(&(b.member.as_bytes(), b.written_at)));
    rows
}

/// The members and the contract digest of `surface`, as the adapter a
/// serve site of it is built from reads them: the pass's own selection,
/// for the agreement law with `surfaces::surface_rows` to pin.
pub fn selection(
    programs: &[&Program],
    surface: &str,
    renames: &[(Vec<String>, String)],
    default_surface: &str,
) -> (Vec<String>, u64) {
    let rows = rows_of_surface(programs, surface, renames, default_surface);
    let slices: Vec<&[TopDecl]> = programs.iter().map(|p| p.items.as_slice()).collect();
    let shapes = Shapes::of_all(&slices);
    (rows.iter().map(|r| r.member.clone()).collect(), digest_of(&shapes, &rows))
}

fn digest_of(shapes: &Shapes<'_>, rows: &[Row]) -> u64 {
    let hash = |te: &TypeExpr| shape_hash(&shapes.shape(te, ShapeForm::Contract).unwrap_or_default());
    let reqs: Vec<&Vec<String>> = rows.iter().map(|r| &r.requires).collect();
    let lines: Vec<hale_model::surface::DigestLine<'_>> = rows
        .iter()
        .zip(&reqs)
        .map(|(r, req)| hale_model::surface::DigestLine {
            member: &r.member,
            request: r.request.as_ref().map(hash),
            response: r.response.as_ref().map(hash),
            error: r.error.as_ref().map(hash),
            requires: req,
        })
        .collect();
    hale_model::surface::surface_digest(&lines)
}

/// The roles that confer `role`: itself and every role that includes it,
/// transitively (`role owner includes support;`).
pub(crate) fn conferring(programs: &[&Program]) -> BTreeMap<String, Vec<String>> {
    let mut includes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in programs {
        for d in flat_decls(&p.items) {
            if let TopDecl::Role(r) = d {
                includes.entry(r.name.name.clone()).or_default().extend(r.includes.iter().map(|i| i.name.clone()));
            }
        }
    }
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for role in includes.keys() {
        let mut set: BTreeSet<String> = BTreeSet::new();
        let mut stack = vec![role.clone()];
        while let Some(r) = stack.pop() {
            if !set.insert(r.clone()) {
                continue;
            }
            for (holder, inc) in &includes {
                if inc.contains(&r) {
                    stack.push(holder.clone());
                }
            }
        }
        out.insert(role.clone(), set.into_iter().collect());
    }
    out
}

/// The struct names a type reaches, for the codecs: declared structs, and
/// builtin records (`IndexError`, …) with the primitive fields the schema
/// and the contract shape give them (`Shapes::builtin_fields`).
pub(crate) fn struct_closure(
    shapes: &Shapes<'_>,
    te: &TypeExpr,
    out: &mut BTreeSet<String>,
    builtins: &mut BTreeMap<String, Vec<(String, hale_syntax::ast::PrimType)>>,
) {
    let te = shapes.dealias(te);
    // a list reaches what its elements reach
    if let Some((elem, _, _)) = hale_syntax::json_gen::wire_list(&te) {
        struct_closure(shapes, elem, out, builtins);
        return;
    }
    match shapes.classify(&te) {
        TypeClass::Struct { name, fields } => {
            if out.insert(name.to_string()) {
                for f in fields {
                    struct_closure(shapes, &f.ty, out, builtins);
                }
            }
        }
        // an enum's codecs are generated beside the records', and its values
        // reach what they reach
        TypeClass::Enum { name, variants } => {
            if out.insert(name.to_string()) {
                for v in variants {
                    for t in &v.fields {
                        struct_closure(shapes, t, out, builtins);
                    }
                }
            }
        }
        TypeClass::Builtin(name) => {
            if out.insert(name.to_string()) {
                let fields = shapes
                    .builtin_fields(&te)
                    .into_iter()
                    .filter_map(|(f, ty)| match ty {
                        TypeExpr::Primitive(p, _) => Some((f, p)),
                        _ => None,
                    })
                    .collect();
                builtins.insert(name.to_string(), fields);
            }
        }
        _ => {}
    }
}

/// How a row's value crosses the wire at the top level: the expression
/// that renders `v` as JSON, and what decodes `body`.
pub(crate) struct Codec<'a> {
    shapes: &'a Shapes<'a>,
    convs: &'a BTreeMap<String, Conv>,
}

impl<'a> Codec<'a> {
    pub(crate) fn new(shapes: &'a Shapes<'a>, convs: &'a BTreeMap<String, Conv>) -> Self {
        Codec { shapes, convs }
    }

    /// The Hale expression rendering `v` (of type `te`) as JSON text.
    pub(crate) fn encode(&self, te: &TypeExpr, v: &str) -> String {
        use hale_syntax::ast::PrimType;
        let te = &self.shapes.dealias(te);
        match self.shapes.classify(te) {
            TypeClass::Prim(PrimType::String) => format!("\"\\\"\" + std::json::escape_string({v}) + \"\\\"\""),
            TypeClass::Prim(PrimType::Time) => format!("\"\\\"\" + std::time::iso8601({v}) + \"\\\"\""),
            TypeClass::Prim(PrimType::Duration) => format!("to_string({v} / 1ns)"),
            TypeClass::Prim(_) => format!("to_string({v})"),
            TypeClass::Struct { name, .. } | TypeClass::Builtin(name) | TypeClass::Enum { name, .. } => {
                format!("__api_encode_{name}({v})")
            }
            TypeClass::Named { name, base, .. } => match self.convs.get(name) {
                Some(c) => format!("to_string({})", c.to_int(v)),
                None if base == PrimType::String => format!("\"\\\"\" + std::json::escape_string({v}) + \"\\\"\""),
                None => format!("to_string({v})"),
            },
            _ => "\"null\"".to_string(),
        }
    }

    /// Statements that decode `body` as `te` into `let <var>`, running
    /// `bail` (a statement list ending in `return`) when it does not.
    pub(crate) fn decode(&self, te: &TypeExpr, body: &str, var: &str, bail: &str) -> String {
        use hale_syntax::ast::PrimType;
        // a plain alias decodes as what it stands for
        let te = &self.shapes.dealias(te);
        match self.shapes.classify(te) {
            TypeClass::Struct { name, .. } | TypeClass::Builtin(name) => {
                format!("let {var} = __api_decode_{name}({body}) or {{ {bail} }};\n")
            }
            // an enum's decoder names the field it reads: a row's is the payload
            TypeClass::Enum { name, .. } => {
                format!("let {var} = __api_decode_{name}({body}, \"payload\") or {{ {bail} }};\n")
            }
            // a scalar payload is its complete JSON token of the type's kind
            // (the runtime's `__api_json_is_*`), checked before it is converted
            TypeClass::Prim(PrimType::Int) => format!(
                "if !__api_json_is_int(std::str::trim({body})) {{ {bail} }}\nlet {var} = std::str::parse_int(std::str::trim({body})) or {{ {bail} }};\n"
            ),
            TypeClass::Prim(PrimType::Float) => format!(
                "if !__api_json_is_float(std::str::trim({body})) {{ {bail} }}\nlet {var} = std::str::parse_float(std::str::trim({body})) or {{ {bail} }};\n"
            ),
            TypeClass::Prim(PrimType::Bool) => format!(
                "if !__api_json_is_bool(std::str::trim({body})) {{ {bail} }}\nlet {var} = std::str::trim({body}) == \"true\";\n"
            ),
            TypeClass::Prim(PrimType::String) => format!(
                "let __t_{var} = std::str::trim({body});\nif !__api_json_is_string(__t_{var}) {{ {bail} }}\nlet {var} = std::json::unescape_string(__t_{var}[1..(len(__t_{var}) - 1)]);\n"
            ),
            // an instant is one string, RFC 3339 in UTC: the `Z` required, an
            // instant outside `Time`'s range refused by `parse_time`
            TypeClass::Prim(PrimType::Time) => format!(
                "let __t_{var} = std::str::trim({body});\nif !__api_json_is_string(__t_{var}) {{ {bail} }}\nlet __u_{var} = std::json::unescape_string(__t_{var}[1..(len(__t_{var}) - 1)]);\nif !std::str::ends_with(__u_{var}, \"Z\") {{ {bail} }}\nlet {var} = std::time::parse_time(__u_{var}) or {{ {bail} }};\n"
            ),
            TypeClass::Prim(PrimType::Duration) => format!(
                "if !__api_json_is_int(std::str::trim({body})) {{ {bail} }}\nlet __n_{var} = std::str::parse_int(std::str::trim({body})) or {{ {bail} }};\nlet {var} = __n_{var} * 1ns;\n"
            ),
            TypeClass::Named { name, .. } if self.convs.contains_key(name) => {
                let c = &self.convs[name];
                let n = format!("__n_{var}");
                let ins = format!(
                    "if !__api_json_is_int(std::str::trim({body})) {{ {bail} }}\nlet {n} = std::str::parse_int(std::str::trim({body})) or {{ {bail} }};\n"
                );
                match c {
                    Conv::Range(..) => format!("{ins}let {var} = {name}({n}) or {{ {bail} }};\n"),
                    Conv::Identity(_) => format!("{ins}let {var} = {name}({n});\n"),
                    Conv::Quantity(m, u) => format!("{ins}let {var} = {n} * {m}{u};\n"),
                }
            }
            _ => format!("let {var} = {body};\n"),
        }
    }
}

/// How a row's value crosses a gRPC exposure as protobuf (R8b): the
/// expression that writes `v` as the message `<T>` or the one-field message
/// of a scalar (`<rpc>Request`, …, `value = 1`), and what reads `bytes` as
/// one. The records are `json_gen`'s `__api_pb_*` codecs.
#[derive(Clone, Copy)]
pub(crate) struct PbCodec<'a> {
    shapes: &'a Shapes<'a>,
    convs: &'a BTreeMap<String, Conv>,
    schemas: &'a crate::surfaces::Schemas<'a>,
    /// The surface whose file names the messages (`proto_gen::message_name`).
    surface: &'a str,
}

impl PbCodec<'_> {
    /// The Hale expression (of type `Bytes`) that writes `v` (of type `te`).
    pub(crate) fn encode(&self, te: &TypeExpr, v: &str) -> String {
        use hale_syntax::ast::PrimType;
        let te = &self.shapes.dealias(te);
        let put = |p: PrimType, v: &str| match p {
            PrimType::Int => format!("__api_pb_put_int(1, {v})"),
            PrimType::Float => format!("__api_pb_put_f64(1, {v})"),
            PrimType::Bool => format!("__api_pb_put_bool(1, {v})"),
            _ => format!("__api_pb_put_str(1, {v})"),
        };
        match self.shapes.classify(te) {
            TypeClass::Struct { name, .. } | TypeClass::Builtin(name) => format!("__api_pb_encode_{name}({v})"),
            // an enum with payloads is its message; a payload-free one is
            // its number, in the one-field message a scalar row has
            TypeClass::Enum { name, variants } if variants.iter().any(|v| !v.fields.is_empty()) => {
                format!("__api_pb_encode_{name}({v})")
            }
            TypeClass::Enum { name, .. } => format!("__api_pb_put_int(1, __api_pb_enum_no_{name}({v}))"),
            // an instant is its nanoseconds since the epoch, a `Duration` its count of them
            TypeClass::Prim(PrimType::Time) => format!("__api_pb_put_int(1, std::time::nanos({v}))"),
            TypeClass::Prim(PrimType::Duration) => format!("__api_pb_put_int(1, {v} / 1ns)"),
            TypeClass::Prim(p) => put(p, v),
            TypeClass::Named { name, base, .. } => match self.convs.get(name) {
                Some(c) => format!("__api_pb_put_int(1, {})", c.to_int(v)),
                None => put(base, v),
            },
            _ => "std::bytes::from_string(\"\")".to_string(),
        }
    }

    /// Statements that decode `bytes` (a `Bytes`) as `te` into `let <var>`,
    /// running `bail` (a statement list ending in `return`) when it does not.
    pub(crate) fn decode(&self, te: &TypeExpr, bytes: &str, var: &str, bail: &str) -> String {
        use hale_syntax::ast::PrimType;
        let te = &self.shapes.dealias(te);
        let boxed = |p: PrimType| match p {
            PrimType::Int => "__api_pb_box_int",
            PrimType::Float => "__api_pb_box_float",
            PrimType::Bool => "__api_pb_box_bool",
            _ => "__api_pb_box_str",
        };
        match self.shapes.classify(te) {
            TypeClass::Struct { name, .. } | TypeClass::Builtin(name) => {
                format!("let {var} = __api_pb_decode_{name}({bytes}) or {{ {bail} }};\n")
            }
            TypeClass::Enum { name, variants } if variants.iter().any(|v| !v.fields.is_empty()) => {
                format!("let {var} = __api_pb_decode_{name}({bytes}, \"payload\") or {{ {bail} }};\n")
            }
            TypeClass::Enum { name, .. } => format!(
                "let __n_{var} = __api_pb_box_int({bytes}) or {{ {bail} }};\nlet {var} = __api_pb_enum_{name}(__n_{var}, \"payload\") or {{ {bail} }};\n"
            ),
            TypeClass::Prim(PrimType::Time) => {
                format!("let __n_{var} = __api_pb_box_int({bytes}) or {{ {bail} }};\nlet {var} = std::time::from_nanos(__n_{var});\n")
            }
            TypeClass::Prim(PrimType::Duration) => {
                format!("let __n_{var} = __api_pb_box_int({bytes}) or {{ {bail} }};\nlet {var} = __n_{var} * 1ns;\n")
            }
            TypeClass::Prim(p) => format!("let {var} = {}({bytes}) or {{ {bail} }};\n", boxed(p)),
            TypeClass::Named { name, .. } if self.convs.contains_key(name) => {
                let c = &self.convs[name];
                let n = format!("__n_{var}");
                let ins = format!("let {n} = __api_pb_box_int({bytes}) or {{ {bail} }};\n");
                match c {
                    Conv::Range(..) => format!("{ins}let {var} = {name}({n}) or {{ {bail} }};\n"),
                    Conv::Identity(_) => format!("{ins}let {var} = {name}({n});\n"),
                    Conv::Quantity(m, u) => format!("{ins}let {var} = {n} * {m}{u};\n"),
                }
            }
            TypeClass::Named { base, .. } => format!("let {var} = {}({bytes}) or {{ {bail} }};\n", boxed(base)),
            _ => format!("let {var} = {bytes};\n"),
        }
    }

    /// The message that carries `te`, as the `.proto` names it.
    pub(crate) fn message(&self, member: &str, what: &str, te: &TypeExpr) -> String {
        let mut scratch = BTreeMap::new();
        match self.schemas.type_ref(te, &mut scratch) {
            crate::surfaces::FieldSchema::Ref(n) => crate::proto_gen::message_name(&n, self.surface),
            crate::surfaces::FieldSchema::Enum { name, variants } if variants.iter().any(|(_, v)| !v.is_empty()) => {
                crate::proto_gen::message_name(&name, self.surface)
            }
            _ => crate::proto_gen::wrapper_name(member, what),
        }
    }
}

/// The descriptors a gRPC exposure reflects, one file a line: its name, the
/// symbols it declares (comma separated) and its `FileDescriptorProto` in
/// base64, tab separated. The surface's file is the one `hale api export`
/// writes (`proto_gen::from_model`); the other two are the reserved
/// description method's and the reflection service's own. Empty when a row
/// has no proto3 encoding (the export refuses it by name).
fn grpc_files(rows: &[Row], schemas: &crate::surfaces::Schemas<'_>, surface: &str, digest: u64) -> String {
    use crate::surface_doc::{ClientError, ClientMember, ClientModel};
    let mut types = BTreeMap::new();
    let members: Vec<ClientMember> = rows
        .iter()
        .map(|r| ClientMember {
            name: r.member.clone(),
            request: r.request.as_ref().map(|te| schemas.type_ref(te, &mut types)),
            response: r.response.as_ref().map(|te| schemas.type_ref(te, &mut types)),
            error: match (&r.error, r.server_error) {
                (_, true) => ClientError::Server,
                (Some(e), false) => ClientError::Type(schemas.type_ref(e, &mut types)),
                (None, false) => ClientError::None,
            },
            requires: r.requires.clone(),
        })
        .collect();
    let model = ClientModel {
        surface: surface.to_string(),
        digest: hale_model::surface::digest_text(digest),
        members,
        types,
        streams: Vec::new(),
    };
    let Ok(file) = crate::proto_gen::from_model(&model) else { return String::new() };
    let mut out = String::new();
    for f in [file, crate::proto_gen::description_file(), crate::proto_gen::reflection_file()] {
        out.push_str(&format!("{}\t{}\t{}\n", f.name, f.symbols().join(","), crate::proto_gen::base64(&f.descriptor())));
    }
    out
}

/// What of `std::api` the runtime declares (`hale_stdlib::API_RUNTIME_SOURCE`).
const RUNTIME_NAMES: &[&str] = &[
    "Grants",
    "RevisedRoleSource",
    "RevisedStaticRoles",
    "ServedContext",
    "Revision",
    "ExpiringBearerSource",
    "Request",
    "Outcome",
    "Rpc",
    "Handle",
    "Surface",
    "Exposure",
    "RpcIngress",
    "RpcLost",
    "RpcCall",
    "RpcHello",
    "RpcEvent",
    "test",
    "unix",
    "http",
    "mcp",
    "grpc",
];

/// Whether the programs serve a surface (`api::serve`) or spell a name of
/// the runtime (`std::api::Request`, `std::api::test::Rpc`, …): the
/// runtime joins a program only then.
fn mentions_runtime(programs: &[&mut Program]) -> bool {
    spells(programs, &|w| {
        (w[2] == "api" && w[3] == "serve")
            || (w[1] == "std" && w[2] == "api" && RUNTIME_NAMES.contains(&w[3]))
            || (w[0] == "std" && w[1] == "io" && w[2] == "h2" && H2_NAMES.contains(&w[3]))
    })
}

/// The names of `std::io::h2` a program may spell: its listener, its
/// connection and the two messages between a connection and a transport.
const H2_NAMES: &[&str] = &["Listener", "Conn", "Event", "Cmd"];

/// Whether any name sequence the programs spell satisfies `pred`, given the
/// last four names of the walk, the oldest first.
fn spells(programs: &[&mut Program], pred: &dyn Fn(&[&str; 4]) -> bool) -> bool {
    programs.iter().any(|p| p.items.iter().any(|item| item_spells(item, pred)))
}

/// [`spells`], over one item.
fn item_spells(item: &TopDecl, pred: &dyn Fn(&[&str; 4]) -> bool) -> bool {
    use hale_syntax::names::{for_each_spelled_in_item, Spelled};
    let mut found = false;
    let mut window: [&str; 4] = ["", "", "", ""];
    for_each_spelled_in_item(item, &mut |s| {
        if let Spelled::Name(n) = s {
            window = [window[1], window[2], window[3], n];
            if pred(&window) {
                found = true;
            }
        }
    });
    found
}

/// Whether the programs need the HTTP/2 server: a `grpc::Rpc` transport (at
/// a serve site, or held as a param), or a name of `std::io::h2`.
fn mentions_h2(programs: &[&mut Program]) -> bool {
    spells(programs, &|w| {
        (w[2] == "grpc" && w[3] == "Rpc") || (w[0] == "std" && w[1] == "io" && w[2] == "h2" && H2_NAMES.contains(&w[3]))
    })
}

/// Where the appended runtime parses: its own window of the generated
/// space, from here up to the topics'.
pub(crate) const RUNTIME_BASE: u32 = API_SYNTH_BASE + 0x0400_0000;

/// Whether an offset is in the appended runtime's source. The runtime is
/// stdlib source in every judgment (the analysis copy's rows are not the
/// program's own); this is how a pass that partitions program from stdlib
/// knows it.
pub fn is_runtime_pos(pos: u32) -> bool {
    (RUNTIME_BASE..API_SYNTH_BASE + 0x0500_0000).contains(&pos)
}

/// The stdlib file an offset of the appended runtime lies in: the file's
/// name as `crates/hale-stdlib/hl/` spells it, its text, and the offset
/// within it. Each of the runtime's six files parses in a window of its
/// own ([`inject_runtime`], [`inject_hub`], [`inject_h2`],
/// [`inject_grpc`], [`inject_tcp_serve`], [`inject_tcp_listener`]);
/// `None` outside them.
pub fn runtime_source_at(pos: u32) -> Option<(&'static str, &'static str, usize)> {
    let windows: [(u32, &'static str, &'static str); 6] = [
        (RUNTIME_BASE, "api_rpc.hl", hale_stdlib::API_RUNTIME_SOURCE),
        (RUNTIME_BASE + 0x0080_0000, "api_hub.hl", hale_stdlib::API_HUB_SOURCE),
        (RUNTIME_BASE + 0x00C0_0000, "io_h2.hl", hale_stdlib::API_H2_SOURCE),
        (RUNTIME_BASE + 0x00E0_0000, "api_grpc.hl", hale_stdlib::API_GRPC_SOURCE),
        (TCP_SERVE_BASE, "io_tcp_serve.hl", hale_stdlib::TCP_SERVE_SOURCE),
        (TCP_LISTENER_BASE, "io_tcp_listener.hl", hale_stdlib::TCP_LISTENER_SOURCE),
    ];
    windows.iter().find_map(|(base, name, src)| {
        let local = pos.checked_sub(*base)? as usize;
        (local < src.len()).then_some((*name, *src, local))
    })
}

/// Append the runtime to the first program, once.
fn inject_runtime(programs: &mut [&mut Program]) {
    let have = programs.iter().any(|p| {
        flat_decls(&p.items).any(|d| matches!(d, TopDecl::Locus(l) if l.name.name == "__StdApiExposure"))
    });
    if have || programs.is_empty() {
        return;
    }
    match parse_source_at(hale_stdlib::API_RUNTIME_SOURCE, RUNTIME_BASE) {
        Ok(rt) => {
            for mut item in rt.items {
                if let TopDecl::Type(t) = &mut item {
                    t.synthetic = true;
                }
                programs[0].items.push(item);
            }
        }
        Err(ds) => eprintln!(
            "rpc_expand: the runtime did not parse: {}",
            ds.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; ")
        ),
    }
}

/// The five topics the runtime's wire subjects name, once: a keyed topic's
/// routing (and the `or` of a send whose delivery no subscriber took)
/// belongs to the program's topic rows, and the runtime's sends are by
/// subject.
fn inject_topics(programs: &mut [&mut Program]) {
    let have = programs
        .iter()
        .any(|p| flat_decls(&p.items).any(|d| matches!(d, TopDecl::Topic(t) if t.name.name == "__ApiRpcCallT")));
    if have || programs.is_empty() {
        return;
    }
    match parse_source_at(TOPICS_SRC, API_SYNTH_BASE + 0x0500_0000) {
        Ok(t) => programs[0].items.extend(t.items),
        Err(ds) => eprintln!(
            "rpc_expand: the runtime's topics did not parse: {}",
            ds.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; ")
        ),
    }
}

/// The hub's runtime and its three topics, once, for a program that binds a
/// topic to a hub: kept out of `API_RUNTIME_SOURCE` so a program that serves
/// a surface and binds no hub carries neither.
fn inject_hub(programs: &mut [&mut Program]) {
    let have = programs
        .iter()
        .any(|p| flat_decls(&p.items).any(|d| matches!(d, TopDecl::Locus(l) if l.name.name == "__StdApiWsHub")));
    if have || programs.is_empty() {
        return;
    }
    match parse_source_at(hale_stdlib::API_HUB_SOURCE, RUNTIME_BASE + 0x0080_0000) {
        Ok(rt) => {
            for mut item in rt.items {
                if let TopDecl::Type(t) = &mut item {
                    t.synthetic = true;
                }
                programs[0].items.push(item);
            }
        }
        Err(ds) => eprintln!(
            "rpc_expand: the hub runtime did not parse: {}",
            ds.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; ")
        ),
    }
    match parse_source_at(HUB_TOPICS_SRC, API_SYNTH_BASE + 0x0500_0000 + 0x0010_0000) {
        Ok(t) => programs[0].items.extend(t.items),
        Err(ds) => eprintln!(
            "rpc_expand: the hub's topics did not parse: {}",
            ds.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; ")
        ),
    }
}

/// The HTTP/2 server's loci and its two topics, once, for a program that
/// serves over `grpc::Rpc` or names a type of `std::io::h2`: kept out of
/// `API_RUNTIME_SOURCE` so a program that serves a surface over another
/// transport carries no HTTP/2.
fn inject_h2(programs: &mut [&mut Program]) {
    let have = programs
        .iter()
        .any(|p| flat_decls(&p.items).any(|d| matches!(d, TopDecl::Locus(l) if l.name.name == "__StdIoH2Conn")));
    if have || programs.is_empty() {
        return;
    }
    match parse_source_at(hale_stdlib::API_H2_SOURCE, RUNTIME_BASE + 0x00C0_0000) {
        Ok(rt) => {
            for mut item in rt.items {
                if let TopDecl::Type(t) = &mut item {
                    t.synthetic = true;
                }
                programs[0].items.push(item);
            }
        }
        Err(ds) => eprintln!(
            "rpc_expand: the HTTP/2 server did not parse: {}",
            ds.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; ")
        ),
    }
    match parse_source_at(H2_TOPICS_SRC, API_SYNTH_BASE + 0x0500_0000 + 0x0020_0000) {
        Ok(t) => programs[0].items.extend(t.items),
        Err(ds) => eprintln!(
            "rpc_expand: the HTTP/2 server's topics did not parse: {}",
            ds.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; ")
        ),
    }
}

/// Whether the programs name the gRPC transport (`grpc::Rpc` at a serve
/// site, held as a param, or by its stdlib path).
fn mentions_grpc(programs: &[&mut Program]) -> bool {
    spells(programs, &|w| w[2] == "grpc" && w[3] == "Rpc")
}

/// The gRPC transport's locus, once, for a program that names `grpc::Rpc`:
/// it speaks to the HTTP/2 server of [`inject_h2`], which is injected first.
fn inject_grpc(programs: &mut [&mut Program]) {
    let have = programs
        .iter()
        .any(|p| flat_decls(&p.items).any(|d| matches!(d, TopDecl::Locus(l) if l.name.name == "__StdApiGrpcRpc")));
    if have || programs.is_empty() {
        return;
    }
    match parse_source_at(hale_stdlib::API_GRPC_SOURCE, RUNTIME_BASE + 0x00E0_0000) {
        Ok(rt) => {
            for mut item in rt.items {
                if let TopDecl::Type(t) = &mut item {
                    t.synthetic = true;
                }
                programs[0].items.push(item);
            }
        }
        Err(ds) => eprintln!(
            "rpc_expand: the gRPC transport did not parse: {}",
            ds.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; ")
        ),
    }
}

const H2_TOPICS_SRC: &str = "topic __ApiH2EventT { payload: __StdIoH2Event; subject: \"__api.h2.event\"; keyed_by key; }
topic __ApiH2CmdT { payload: __StdIoH2Cmd; subject: \"__api.h2.cmd\"; keyed_by key; }
";

const HUB_TOPICS_SRC: &str = "topic __ApiHubEventT { payload: __StdApiHubEvent; subject: \"__api.hub.event\"; keyed_by key; }
topic __ApiHubUpT { payload: __StdApiHubUp; subject: \"__api.hub.up\"; keyed_by key; }
topic __ApiHubOutT { payload: __StdApiHubOut; subject: \"__api.hub.out\"; keyed_by key; }
";

/// The names of the TCP connection substrate a program may spell under
/// `std::io::tcp`.
const TCP_SERVE_NAMES: &[&str] = &["Listener", "Acceptor", "Connections", "Accepted", "ConnHandler"];

/// Whether a name sequence names the TCP connection substrate:
/// `std::http::Server` (or `std::http::Responder`), or one of
/// `std::io::tcp`'s substrate names (its `Listener` among them).
fn names_tcp_serve(w: &[&str; 4]) -> bool {
    (w[1] == "std" && w[2] == "http" && (w[3] == "Server" || w[3] == "Responder"))
        || (w[0] == "std" && w[1] == "io" && w[2] == "tcp" && TCP_SERVE_NAMES.contains(&w[3]))
}

/// Whether the programs name the TCP connection substrate.
fn mentions_tcp_serve(programs: &[&mut Program]) -> bool {
    spells(programs, &names_tcp_serve)
}

/// Whether a declaration names the TCP connection substrate: one that
/// brings it into the program, so the substrate's declarations
/// ([`is_tcp_serve_appended_pos`]) are there because it, or another
/// declaration that names it, is.
pub fn item_names_tcp_serve(item: &TopDecl) -> bool {
    item_spells(item, &names_tcp_serve)
}

/// Where the substrate parses: the last window of the runtime's space.
pub const TCP_SERVE_BASE: u32 = RUNTIME_BASE + 0x00F0_0000;

/// Where `std::io::tcp::Listener` parses: past the substrate, in the same
/// last window of the runtime's space.
const TCP_LISTENER_BASE: u32 = TCP_SERVE_BASE + 0x0008_0000;

/// Whether an offset is in the TCP substrate's appended source, or the
/// `std::io::tcp::Listener` built on it.
pub fn is_tcp_serve_pos(pos: u32) -> bool {
    (TCP_SERVE_BASE..TCP_SERVE_BASE + hale_stdlib::TCP_SERVE_SOURCE.len() as u32).contains(&pos)
        || (TCP_LISTENER_BASE..TCP_LISTENER_BASE + hale_stdlib::TCP_LISTENER_SOURCE.len() as u32).contains(&pos)
}

/// The TCP substrate's declarations in the programs, by name: its loci and
/// its free fns. Appended to the program's own seed, they are stdlib source
/// the program names by its paths (`std::io::tcp::Listener`,
/// `std::http::Server`); a judgment that leaves the stdlib out leaves
/// them out too.
#[derive(Debug, Default, Clone)]
pub struct TcpServeDecls {
    pub loci: BTreeSet<String>,
    pub fns: BTreeSet<String>,
}

impl TcpServeDecls {
    pub fn of(programs: &[&Program]) -> Self {
        let mut out = TcpServeDecls::default();
        for p in programs {
            for item in flat_decls(&p.items) {
                match item {
                    TopDecl::Locus(l) if is_tcp_serve_pos(l.name.span.start.0) => {
                        out.loci.insert(l.name.name.clone());
                    }
                    TopDecl::Fn(f) if is_tcp_serve_pos(f.name.span.start.0) => {
                        out.fns.insert(f.name.name.clone());
                    }
                    _ => {}
                }
            }
        }
        out
    }

    /// Whether a fn row (`locus` its locus, `None` for a free fn) is one of
    /// the substrate's.
    pub fn holds(&self, locus: Option<&str>, fn_name: &str) -> bool {
        match locus {
            Some(l) => self.loci.contains(l),
            None => self.fns.contains(fn_name),
        }
    }
}

/// Where the substrate's topic parses.
pub const TCP_TOPICS_BASE: u32 = API_SYNTH_BASE + 0x0500_0000 + 0x0030_0000;

/// Whether an offset is in the TCP substrate's appended source or its
/// topic's: a declaration [`inject_tcp_serve`] appended.
pub fn is_tcp_serve_appended_pos(pos: u32) -> bool {
    is_tcp_serve_pos(pos) || (TCP_TOPICS_BASE..TCP_TOPICS_BASE + TCP_TOPICS_SRC.len() as u32).contains(&pos)
}

const TCP_TOPICS_SRC: &str = "topic __TcpAcceptedT { payload: __StdIoTcpAccepted; subject: \"__tcp.accepted\"; keyed_by route; on_unmatched: fallback; }
";

/// The TCP connection substrate and its topic, once, for a program that
/// names it (`hale_stdlib::TCP_SERVE_SOURCE`). Independent of the api
/// runtime: a program with a `std::http::Server` and no `api::serve`
/// carries the substrate alone.
fn inject_tcp_serve(programs: &mut [&mut Program]) {
    let have = programs
        .iter()
        .any(|p| flat_decls(&p.items).any(|d| matches!(d, TopDecl::Locus(l) if l.name.name == "__StdIoTcpRelay")));
    if have || programs.is_empty() {
        return;
    }
    match parse_source_at(hale_stdlib::TCP_SERVE_SOURCE, TCP_SERVE_BASE) {
        Ok(rt) => {
            for mut item in rt.items {
                if let TopDecl::Type(t) = &mut item {
                    t.synthetic = true;
                }
                programs[0].items.push(item);
            }
        }
        Err(ds) => eprintln!(
            "rpc_expand: the TCP substrate did not parse: {}",
            ds.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; ")
        ),
    }
    match parse_source_at(TCP_TOPICS_SRC, TCP_TOPICS_BASE) {
        Ok(t) => programs[0].items.extend(t.items),
        Err(ds) => eprintln!(
            "rpc_expand: the TCP substrate's topic did not parse: {}",
            ds.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; ")
        ),
    }
}

/// Whether the programs name `std::io::tcp::Listener`.
fn mentions_tcp_listener(programs: &[&mut Program]) -> bool {
    spells(programs, &|w| w[0] == "std" && w[1] == "io" && w[2] == "tcp" && w[3] == "Listener")
}

/// `std::io::tcp::Listener`, once, for a program that names it
/// (`hale_stdlib::TCP_LISTENER_SOURCE`): built on the substrate of
/// [`inject_tcp_serve`], which is injected first.
fn inject_tcp_listener(programs: &mut [&mut Program]) {
    let have = programs
        .iter()
        .any(|p| flat_decls(&p.items).any(|d| matches!(d, TopDecl::Locus(l) if l.name.name == "__StdIoTcpListener")));
    if have || programs.is_empty() {
        return;
    }
    match parse_source_at(hale_stdlib::TCP_LISTENER_SOURCE, TCP_LISTENER_BASE) {
        Ok(rt) => programs[0].items.extend(rt.items),
        Err(ds) => eprintln!(
            "rpc_expand: the TCP Listener did not parse: {}",
            ds.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; ")
        ),
    }
}

/// The surface `alias::name` names: the block the import table's row for
/// `alias::api:name` identifies. A surface keeps its name through the
/// mangler (it is the description's), so its row lives in the surfaces'
/// own namespace (`mangle::API_KEY`), apart from a same-named locus or type
/// of the seed, and maps to the identity the loader stamps on the block
/// (`ApiDecl::origin`, `mangle::api_origin`). The row is the provenance: a
/// block of another seed, the importer's own among them, has another
/// origin or none, so the alias never reaches it.
fn imported_surface(programs: &[&Program], renames: &[(Vec<String>, String)], alias: &str, name: &str) -> Option<String> {
    let key = format!("{}{name}", crate::mangle::API_KEY);
    let (_, origin) = renames.iter().find(|(path, _)| path.len() == 2 && path[0] == alias && path[1] == key)?;
    programs.iter().flat_map(|p| flat_decls(&p.items)).find_map(|d| match d {
        TopDecl::Api(a) if a.name.name == name && a.origin.as_ref() == Some(origin) => Some(a.name.name.clone()),
        _ => None,
    })
}

/// A serve site whose surface is written `alias::Name`, for a surface the
/// imported seed `alias` declares, names the surface as declared (`Name`),
/// so every reader of the site (the serve-site laws, this pass, the
/// description) reads one spelling. No rename pass reaches it: the surface
/// argument is a value-position path. A path that names no surface of the
/// aliased seed is left as written, for the laws to refuse in the author's
/// spelling.
fn resolve_imported_surfaces(programs: &mut [&mut Program], renames: &[(Vec<String>, String)]) {
    let mut resolved: BTreeMap<(String, String), String> = BTreeMap::new();
    {
        let ro: Vec<&Program> = programs.iter().map(|p| &**p).collect();
        for p in &ro {
            for d in flat_decls(&p.items) {
                let TopDecl::Locus(l) = d else { continue };
                for (_, site) in crate::surfaces::locus_serve_sites(l) {
                    let Expr::Path(qn) = site.surface else { continue };
                    let [alias, name] = qn.segments.as_slice() else { continue };
                    if let Some(s) = imported_surface(&ro, renames, &alias.name, &name.name) {
                        resolved.insert((alias.name.clone(), name.name.clone()), s);
                    }
                }
            }
        }
    }
    if resolved.is_empty() {
        return;
    }
    let mut rewrite = |e: &mut Expr| {
        let Expr::Path(qn) = e else { return };
        let [alias, name] = qn.segments.as_slice() else { return };
        if let Some(s) = resolved.get(&(alias.name.clone(), name.name.clone())) {
            *e = Expr::Ident(Ident { name: s.clone(), span: qn.span, id: hale_syntax::ast::NodeId::NONE });
        }
    };
    for p in programs.iter_mut() {
        hale_syntax::ast::for_each_decl_mut(&mut p.items, &mut |d| {
            let TopDecl::Locus(l) = d else { return };
            for m in &mut l.members {
                if let LocusMember::Params(pb) = m {
                    for p in &mut pb.params {
                        if let ParamInit::Value(e) = &mut p.init {
                            crate::surfaces::serve_surface_mut(e, &mut rewrite);
                        }
                    }
                }
            }
        });
    }
}

/// An exposure param is typed `api::Exposure` as its author writes it, the
/// runtime's `std::api::Exposure`: spelled so here, whether or not the site
/// is built, so a site the laws refuse is refused for what it is.
fn type_exposure_params(programs: &mut [&mut Program]) {
    for p in programs.iter_mut() {
        hale_syntax::ast::for_each_decl_mut(&mut p.items, &mut |d| {
            let TopDecl::Locus(l) = d else { return };
            for m in &mut l.members {
                let LocusMember::Params(pb) = m else { continue };
                for p in &mut pb.params {
                    if crate::surfaces::param_serve_site(p).is_none() {
                        continue;
                    }
                    if let Some(TypeExpr::Named { path, .. }) = &mut p.ty {
                        let short = path.segments.len() == 2 && path.segments[0].name == "api" && path.segments[1].name == "Exposure";
                        if short {
                            let sp = path.segments[0].span;
                            path.segments.insert(0, Ident::new("std", sp));
                        }
                    }
                }
            }
        });
    }
}

/// Run the pass over a bundle's programs.
pub fn expand(
    programs: &mut [&mut Program],
    renames: &[(Vec<String>, String)],
    default_surface: &str,
) -> Vec<Expansion> {
    resolve_imported_surfaces(programs, renames);
    type_exposure_params(programs);
    if mentions_tcp_serve(programs) {
        inject_tcp_serve(programs);
    }
    if mentions_tcp_listener(programs) {
        inject_tcp_listener(programs);
    }
    let hubs = crate::hub_expand::binds_a_hub(programs);
    if !mentions_runtime(programs) && !hubs {
        return Vec::new();
    }
    inject_runtime(programs);
    inject_topics(programs);
    // the runtime's socket listeners (`http::Rpc`, `mcp::Rpc`, `unix::Rpc`)
    // are listeners of the TCP connection substrate: the runtime carries it
    inject_tcp_serve(programs);
    if mentions_h2(programs) {
        inject_h2(programs);
    }
    if mentions_grpc(programs) {
        inject_grpc(programs);
    }
    if hubs {
        inject_hub(programs);
        crate::hub_expand::expand(programs);
    }
    // ---- the immutable phase: rows, digests, sites ----
    struct Plan {
        site: Site,
        rows: Vec<Row>,
        digest: u64,
        slots: Vec<String>,
        /// per slot: the instance's field of the serving locus.
        fields: Vec<String>,
        principals: Option<Expr>,
        roles: Option<Expr>,
        expiring: bool,
        revised: bool,
        /// served over `grpc::Rpc`: its rows get protobuf codecs beside the JSON ones.
        grpc: bool,
        /// a gRPC exposure's descriptors for server reflection (see `grpc_files`).
        files: String,
    }
    let mut plans: Vec<Plan> = Vec::new();
    let confer;
    let convs;
    let all_names: BTreeSet<String>;
    let all_builtins;
    let pb_names: BTreeSet<String>;
    let pb_builtins;
    {
        let ro: Vec<&Program> = programs.iter().map(|p| &**p).collect();
        let slices: Vec<&[TopDecl]> = ro.iter().map(|p| p.items.as_slice()).collect();
        let shapes = Shapes::of_all(&slices);
        confer = conferring(&ro);
        convs = hale_syntax::json_gen::scalar_convs(&ro);
        let mut names = BTreeSet::new();
        let mut builtins = BTreeMap::new();
        let mut names_pb = BTreeSet::new();
        let mut builtins_pb = BTreeMap::new();
        let schemas = crate::surfaces::Schemas::of(&ro);
        let mut loci: BTreeMap<&str, &LocusDecl> = BTreeMap::new();
        for p in &ro {
            for d in flat_decls(&p.items) {
                if let TopDecl::Locus(l) = d {
                    loci.insert(l.name.name.as_str(), l);
                }
            }
        }
        let mut seen_names: BTreeSet<String> = BTreeSet::new();
        for (pi, p) in ro.iter().enumerate() {
            for d in flat_decls(&p.items) {
                let TopDecl::Locus(l) = d else { continue };
                for (param, site) in crate::surfaces::locus_serve_sites(l) {
                    let surface = match site.surface {
                        Expr::Ident(i) => i.name.clone(),
                        Expr::Path(qn) => qn.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::"),
                        _ => continue,
                    };
                    let Some(Expr::Literal(Literal::String(name), _)) = site.option("as") else { continue };
                    let Some(Expr::Literal(Literal::Int(bound), _)) = site.option("bound") else { continue };
                    if !matches!(site.option("on_full"), Some(Expr::Ident(i)) if i.name == "refuse") {
                        continue;
                    }
                    let Some(transport) = site.transport else { continue };
                    // a transport the pass can build: a locus the program or
                    // the standard library declares, or an instance the
                    // serving locus holds (`http::Rpc` and the socket
                    // transports follow in R2b and R3)
                    let buildable = match transport {
                        Expr::Struct { path, .. } => {
                            let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
                            segs.first() == Some(&"std") || is_unix_rpc(path) || is_http_rpc(path) || is_mcp_rpc(path) || is_grpc_rpc(path) || loci.contains_key(segs.join("::").as_str())
                        }
                        e => crate::surfaces::self_field(e).is_some(),
                    };
                    if !buildable {
                        continue;
                    }
                    // the exposure is named once; a second site of the name is the law's
                    if !seen_names.insert(name.clone()) {
                        continue;
                    }
                    // built already (the sequence runs again over its own output)
                    if param.serve.is_some() {
                        continue;
                    }
                    // the param is the exposure: typed as one, or the law says so
                    let typed = match &param.ty {
                        None => true,
                        Some(TypeExpr::Named { path, .. }) => {
                            let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
                            segs == ["std", "api", "Exposure"]
                        }
                        Some(_) => false,
                    };
                    if !typed {
                        continue;
                    }
                    let rows = rows_of_surface(&ro, &surface, renames, default_surface);
                    if rows.is_empty() {
                        continue;
                    }
                    // the receivers: each locus type the rows name, once, in
                    // the order the rows were written
                    let mut by_written: Vec<&Row> = rows.iter().collect();
                    by_written.sort_by_key(|r| r.written_at);
                    let mut slots: Vec<String> = Vec::new();
                    for r in by_written {
                        if !slots.contains(&r.locus) {
                            slots.push(r.locus.clone());
                        }
                    }
                    if slots.len() > 16 {
                        continue;
                    }
                    let params = crate::surfaces::param_types(l);
                    let mut fields: Vec<String> = Vec::new();
                    let mut ok = true;
                    for ty in &slots {
                        let bound_here = site.receivers().iter().find(|i| crate::surfaces::binds_type(i, ty, &params));
                        let field = match bound_here {
                            Some(i) => crate::surfaces::self_field(&i.value).map(str::to_string),
                            None => {
                                let holders: Vec<&String> =
                                    params.iter().filter(|(_, t)| t.as_str() == ty).map(|(n, _)| n).collect();
                                match holders.as_slice() {
                                    [one] => Some((*one).clone()),
                                    _ => None,
                                }
                            }
                        };
                        // the instance is a param of the serving locus built
                        // by a literal: the number rides in the literal
                        let literal = field.as_ref().is_some_and(|f| {
                            l.members.iter().any(|m| {
                                matches!(m, LocusMember::Params(pb) if pb.params.iter().any(|p|
                                    p.name.name == *f && matches!(&p.init, ParamInit::Value(Expr::Struct { .. }))))
                            })
                        });
                        match field {
                            Some(f) if literal => fields.push(f),
                            _ => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    if !ok {
                        continue;
                    }
                    // the exposure is born after the receivers it binds: each
                    // is a param declared before it (the law reports one after)
                    let order: Vec<&str> = params_in_order(l).into_iter().map(|(p, _)| p.name.name.as_str()).collect();
                    let at = |f: &str| order.iter().position(|p| *p == f);
                    let me = at(&param.name.name);
                    if fields.iter().any(|f| at(f) >= me) {
                        continue;
                    }
                    // a socket's listener is the main locus's: a holder that is
                    // not one is held by exactly one chain of params from each
                    // main locus that holds it, its address readable there
                    let mut chains: Vec<(Chain, Expr)> = Vec::new();
                    if !l.is_main && has_listener(l, Some(transport)) {
                        let found = holder_chains(&loci, &l.name.name);
                        let mains: BTreeSet<&str> = found.iter().map(|c| c.main.as_str()).collect();
                        let once = mains.len() == found.len()
                            && found.iter().all(|c| {
                                loci.get(c.main.as_str())
                                    .and_then(|m| placement_entry(m, &c.fields[0].1))
                                    .is_none_or(|e| entry_replicas(e) <= 1)
                            });
                        if found.is_empty() || !once {
                            continue;
                        }
                        let Some(addr) = listener_address(l, Some(transport)) else { continue };
                        let mut ok = true;
                        for c in found {
                            match address_for_main(&c, &addr) {
                                Ok(a) => chains.push((c, a)),
                                Err(_) => ok = false,
                            }
                        }
                        if !ok {
                            continue;
                        }
                    }
                    // the sources: the transport literal's fields, or the
                    // literal of the param the transport names
                    let source_of = |field: &str| -> Option<Expr> {
                        let lit: Option<&Expr> = match transport {
                            Expr::Struct { .. } => Some(transport),
                            e => crate::surfaces::self_field(e).and_then(|f| {
                                l.members.iter().find_map(|m| match m {
                                    LocusMember::Params(pb) => pb.params.iter().find_map(|p| {
                                        (p.name.name == f).then_some(match &p.init {
                                            ParamInit::Value(v) => Some(v),
                                            _ => None,
                                        })
                                    }),
                                    _ => None,
                                })
                                .flatten()
                            }),
                        };
                        match lit {
                            Some(Expr::Struct { inits, .. }) => inits.iter().find(|i| i.name.name == field).map(|i| fresh(&i.value)),
                            _ => None,
                        }
                    };
                    let principals = source_of("principals");
                    let roles = source_of("roles");
                    let has_fn = |src: &Option<Expr>, want: &str| -> bool {
                        let Some(e) = src else { return false };
                        let ty = match e {
                            Expr::Struct { path, .. } => path.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::"),
                            e => match crate::surfaces::self_field(e).and_then(|f| locus_param_ty(l, f)) {
                                Some(t) => t,
                                None => return false,
                            },
                        };
                        loci.get(ty.as_str()).is_some_and(|d| {
                            d.members.iter().any(|m| matches!(m, LocusMember::Fn(f) if f.name.name == want))
                        })
                    };
                    let expiring = has_fn(&principals, "expiry");
                    let revised = has_fn(&roles, "grants");
                    let digest = digest_of(&shapes, &rows);
                    let grpc = is_grpc_site(l, Some(transport));
                    for r in &rows {
                        for te in r.request.iter().chain(r.response.iter()).chain(r.error.iter().filter(|_| !r.server_error)) {
                            struct_closure(&shapes, te, &mut names, &mut builtins);
                            if grpc {
                                struct_closure(&shapes, te, &mut names_pb, &mut builtins_pb);
                            }
                        }
                    }
                    let files = if grpc { grpc_files(&rows, &schemas, &surface, digest) } else { String::new() };
                    plans.push(Plan {
                        site: Site {
                            program: pi,
                            serving: l.name.name.clone(),
                            param: param.name.name.clone(),
                            surface,
                            transport: Some(fresh(transport)),
                            name: name.clone(),
                            bound: *bound,
                            chains,
                        },
                        rows,
                        digest,
                        slots,
                        fields,
                        principals,
                        roles,
                        expiring,
                        revised,
                        grpc,
                        files,
                    });
                }
            }
        }
        all_names = names;
        all_builtins = builtins;
        pb_names = names_pb;
        pb_builtins = builtins_pb;
    }
    if plans.is_empty() {
        return Vec::new();
    }

    // ---- the codecs of what the rows carry ----
    {
        let names: Vec<String> = all_names.iter().cloned().collect();
        let first = plans[0].site.program;
        let builtins: Vec<(String, Vec<(String, hale_syntax::ast::PrimType)>)> =
            all_builtins.iter().map(|(n, f)| (n.clone(), f.clone())).collect();
        hale_syntax::json_gen::generate_rpc_codecs(programs, first, &names, &builtins);
        // a gRPC exposure's rows are also carried as protobuf
        if plans.iter().any(|p| p.grpc) {
            let names: Vec<String> = pb_names.iter().cloned().collect();
            let builtins: Vec<(String, Vec<(String, hale_syntax::ast::PrimType)>)> =
                pb_builtins.iter().map(|(n, f)| (n.clone(), f.clone())).collect();
            hale_syntax::json_gen::generate_rpc_pb_codecs(programs, first, &names, &builtins);
        }
    }

    // ---- per exposure ----
    let mut out: Vec<Expansion> = Vec::new();
    let mut generated = String::new();
    let mut bindings: BTreeMap<String, Vec<Binding>> = BTreeMap::new();
    let ro_snapshot: Vec<Program> = programs.iter().map(|p| (**p).clone()).collect();
    let ro_refs: Vec<&Program> = ro_snapshot.iter().collect();
    let slices: Vec<&[TopDecl]> = ro_refs.iter().map(|p| p.items.as_slice()).collect();
    let shapes = Shapes::of_all(&slices);
    let codec = Codec { shapes: &shapes, convs: &convs };
    let schemas = crate::surfaces::Schemas::of(&ro_refs);
    let pb = PbCodec { shapes: &shapes, convs: &convs, schemas: &schemas, surface: "" };
    for (i, plan) in plans.iter().enumerate() {
        let id = i as i64 + 1;
        let site = &plan.site;
        let pb = PbCodec { surface: &site.surface, ..pb };
        generated.push_str(&surface_src(id, &plan.rows, &plan.slots, &confer, &codec, plan.grpc.then_some(&pb), &schemas));
        for (slot, ty) in plan.slots.iter().enumerate() {
            let members: Vec<(usize, &Row)> = plan
                .rows
                .iter()
                .enumerate()
                .filter(|(_, r)| &r.locus == ty)
                .collect();
            bindings.entry(ty.clone()).or_default().push(Binding {
                exposure: id,
                slot,
                members: members.iter().map(|(m, r)| (*m, (*r).clone())).collect(),
                grpc: plan.grpc,
                surface: site.surface.clone(),
            });
        }
        out.push(Expansion {
            id,
            name: site.name.clone(),
            surface: site.surface.clone(),
            digest: plan.digest,
            serving: site.serving.clone(),
            param: site.param.clone(),
        });
    }

    // ---- mutate: the holders ----
    // the main loci a listener was added to, each with the exposures (by
    // their path from it) a main that writes no `run()` serves until stopped
    let mut waits: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (i, plan) in plans.iter().enumerate() {
        let id = i as i64 + 1;
        let site = &plan.site;
        let exposure = out[i].clone();
        let program = &mut *programs[site.program];
        let Some(l) = find_locus_mut(&mut program.items, &site.serving) else { continue };
        let span = l.name.span;
        let transport = site.transport.as_ref();
        let unix_path = unix_path_of(l, transport);
        let http_bind = if unix_path.is_none() { http_bind_of(l, transport) } else { None };
        let grpc = is_grpc_site(l, transport);
        let mcp = is_mcp_site(l, transport);
        let is_main = l.is_main;
        // A TCP listener the compiler adds (over http / mcp / grpc) states
        // the address it was bound to, its name, `host:<bound port>`, so a
        // `bind:` of port 0 is described by the port the kernel gave it. The
        // main locus's own exposure reads it off its listener, born before
        // it. A holder below main is built before its listener (the listener
        // reads the address off the holder as built), so its exposure starts
        // from the `bind:` as written and is told the bound name by its
        // transport when the listener first asks for it (`RpcEvent` kind 7).
        // A Unix path is bound as written.
        let kind = if grpc { "g" } else { "h" };
        let bound_listener =
            (http_bind.is_some() && is_main).then(|| format!(", address: std::str::clone(self.__rpc_{kind}_{id}.listener)"));
        // the exposure: the param as declared, its init the exposure it builds
        let src = format!(
            "main locus __Tmp {{ params {{ {param}: std::api::Exposure = std::api::Exposure {{ id: {id}, name: {name}, surface: {surface}, digest: {digest}, bound: {bound}, rows: __RpcSurface_{id} {{ }}{address} }}; }} }}\n",
            address = bound_listener.as_deref().unwrap_or(""),
            param = exposure.param,
            id = id,
            name = q(&site.name),
            surface = q(&site.surface),
            digest = q(&hale_model::surface::digest_text(plan.digest)),
            bound = site.bound,
        );
        let Some(mut generated) = parse_params(&src) else { continue };
        if let ParamInit::Value(Expr::Struct { inits, .. }) = &mut generated.init {
            let mut push = |name: &str, value: Expr| {
                inits.push(StructInit { name: Ident::new(name, span), value, span });
            };
            if let Some(t) = transport {
                push("transport", as_stdlib_unix(t, id, &site.surface, &plan.files));
                if bound_listener.is_none() {
                    if let Some(addr) = unix_path.clone().or_else(|| http_bind.clone()) {
                        push("address", addr);
                    }
                }
            }
            if let Some(p) = &plan.principals {
                push("bearer", p.clone());
                if plan.expiring {
                    push("expiring", p.clone());
                    push("has_expiry", Expr::Literal(Literal::Bool(true), span));
                }
            }
            if let Some(r) = &plan.roles {
                push("roles", r.clone());
                if plan.revised {
                    push("revised", r.clone());
                    push("has_revised", Expr::Literal(Literal::Bool(true), span));
                }
            }
        }
        for m in l.members.iter_mut() {
            let LocusMember::Params(pb) = m else { continue };
            for p in pb.params.iter_mut().filter(|p| p.name.name == exposure.param) {
                let ParamInit::Value(call) = std::mem::replace(&mut p.init, generated.init.clone()) else { continue };
                p.serve = Some(Box::new(call));
                p.ty = generated.ty.clone();
            }
        }
        // the listener of a socket transport: a param of the main locus,
        // born after the authored params its address may read and before
        // what serves through it (so it is subscribed when the exposure
        // attaches), on the pool its placement row names
        let listener_for = |bind: Expr| -> Option<(ParamDecl, hale_syntax::ast::PlacementEntry)> {
            match (&unix_path, &http_bind) {
                (Some(_), _) => parse_unix_listener(id, bind, span),
                (None, Some(_)) if grpc => parse_grpc_listener(id, bind, span),
                (None, Some(_)) => parse_http_listener(id, bind, mcp, span),
                (None, None) => None,
            }
        };
        let field_of = |lp: &ParamDecl| lp.name.name.clone();
        if is_main {
            if let Some((lp, default_entry)) = unix_path.clone().or_else(|| http_bind.clone()).and_then(listener_for) {
                let field = field_of(&lp);
                insert_param_before(l, lp, &exposure.param);
                // the exposure's own row places its listener (and the
                // connections it starts); the exposure stays with its
                // holder, beside the sources it calls. A row that cannot
                // host a listener is left for the law to refuse.
                let row = take_listener_row(l, &exposure.param);
                push_placement(l, rename_entry(row, &field).unwrap_or(default_entry), span);
                waits.entry(l.name.name.clone()).or_default().push(exposure.param.clone());
            }
        }
        // a Unix transport the serving locus holds as a param is numbered
        // in the literal that builds it, as one written in the call is
        if let Some(f) = transport.and_then(|t| crate::surfaces::self_field(t)) {
            for m in l.members.iter_mut() {
                if let LocusMember::Params(pb) = m {
                    for p in pb.params.iter_mut().filter(|p| p.name.name == f) {
                        if let ParamInit::Value(e) = &mut p.init {
                            if matches!(e, Expr::Struct { path, .. } if is_unix_rpc(path) || is_http_rpc(path) || is_mcp_rpc(path) || is_grpc_rpc(path)) {
                                *e = as_stdlib_unix(e, id, &site.surface, &plan.files);
                            }
                        }
                    }
                }
            }
        }
        // the receivers' numbers, written into the literals that build them
        for (slot, field) in plan.fields.iter().enumerate() {
            let key = id * 100 + slot as i64;
            let key_param = format!("__rpc_s_{id}");
            inject_key(l, field, &key_param, key, span);
        }
        // a holder below a main locus: each main locus that holds it gets
        // the listener, after the param the chain starts at (so its address
        // is read off the holder as every construction built it), on the
        // pool that param's row names when a listener can run there, else
        // the transport's shared pool
        for (chain, addr) in &site.chains {
            let Some((lp, default_entry)) = listener_for(addr.clone()) else { continue };
            let field = field_of(&lp);
            let root = chain.fields[0].1.clone();
            let mut placed = false;
            for p in programs.iter_mut() {
                let Some(main) = find_locus_mut(&mut p.items, &chain.main) else { continue };
                let entry = placement_entry(main, &root)
                    .filter(|e| hosts_listener(e))
                    .and_then(|e| rename_entry(Some(e.clone()), &field))
                    .unwrap_or(default_entry.clone());
                // as early in main's birth as the address allows: before the
                // param the server sits under when the address reads nothing
                // of the server (no run of a child born before it can hold
                // it back), just after it when it reads the server as built
                if reads_holder(addr) {
                    insert_param_after(main, lp.clone(), &root);
                } else {
                    insert_param_before(main, lp.clone(), &root);
                }
                let mspan = main.name.span;
                push_placement(main, entry, mspan);
                placed = true;
                break;
            }
            if placed {
                let path = format!("{}.{}", chain.path(), exposure.param);
                waits.entry(chain.main.clone()).or_default().push(path);
            }
        }
    }

    // ---- a main locus that writes no `run()` serves until its exposures stop ----
    // (spec/api.md § Serving): the run it is given waits, as `h.wait()` does,
    // until every exposure whose listener it holds has stopped or the
    // process drains
    for (main, paths) in &waits {
        for p in programs.iter_mut() {
            let Some(m) = find_locus_mut(&mut p.items, main) else { continue };
            let has_run = m.members.iter().any(|x| matches!(x, LocusMember::Lifecycle(lc) if lc.kind == LifecycleKind::Run));
            if !has_run {
                let open: Vec<String> = paths.iter().map(|x| format!("!self.{x}.stopped")).collect();
                let src = format!(
                    "main locus __Tmp {{ run() {{ while !self.draining && ({}) {{ std::time::__idle_wait(100ms); }} }} }}\n",
                    open.join(" || ")
                );
                m.members.extend(parse_members(&src));
            }
            break;
        }
    }

    // ---- the receivers, numbered where a literal of the serving locus builds them ----
    for (i, plan) in plans.iter().enumerate() {
        let id = i as i64 + 1;
        for (slot, field) in plan.fields.iter().enumerate() {
            let key = id * 100 + slot as i64;
            let key_param = format!("__rpc_s_{id}");
            for p in programs.iter_mut() {
                number_constructions(&mut p.items, &plan.site.serving, field, &key_param, key);
            }
        }
    }

    // ---- mutate: the receiver types ----
    for (ty, bs) in &bindings {
        for p in programs.iter_mut() {
            if let Some(l) = find_locus_mut(&mut p.items, ty) {
                extend_receiver(l, bs, &codec, &pb);
                break;
            }
        }
    }

    // ---- the generated top-level items, beside the first serving locus ----
    let first = plans[0].site.program;
    match parse_source_at(&generated, API_SYNTH_BASE + 0x0100_0000) {
        Ok(g) => programs[first].items.extend(g.items),
        Err(ds) => eprintln!(
            "rpc_expand: generated source did not parse: {}\n{}",
            ds.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; "),
            generated
        ),
    }
    out
}

/// A receiver type's binding to one exposure.
#[derive(Clone)]
struct Binding {
    exposure: i64,
    slot: usize,
    /// (member index in the surface's canonical order, the row).
    members: Vec<(usize, Row)>,
    /// the exposure is served over `grpc::Rpc`: its calls may arrive as protobuf.
    grpc: bool,
    /// the exposure's surface, whose `.proto` names the messages.
    surface: String,
}

/// Whether a transport literal's path is the stdlib's Unix transport,
/// spelled `unix::Rpc` (the serve site's spelling) or `std::api::unix::Rpc`.
pub(crate) fn is_unix_rpc(path: &hale_syntax::ast::QualifiedName) -> bool {
    let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
    matches!(segs.as_slice(), ["unix", "Rpc"] | ["std", "api", "unix", "Rpc"])
}

/// Whether a transport literal's path is the stdlib's HTTP transport,
/// spelled `http::Rpc` (the serve site's spelling) or `std::api::http::Rpc`.
pub(crate) fn is_http_rpc(path: &hale_syntax::ast::QualifiedName) -> bool {
    let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
    matches!(segs.as_slice(), ["http", "Rpc"] | ["std", "api", "http", "Rpc"])
}

/// Whether a transport literal's path is the stdlib's MCP transport,
/// spelled `mcp::Rpc` (the serve site's spelling) or `std::api::mcp::Rpc`.
pub(crate) fn is_mcp_rpc(path: &hale_syntax::ast::QualifiedName) -> bool {
    let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
    matches!(segs.as_slice(), ["mcp", "Rpc"] | ["std", "api", "mcp", "Rpc"])
}

/// Whether a transport literal's path is the stdlib's gRPC transport,
/// spelled `grpc::Rpc` (the serve site's spelling) or `std::api::grpc::Rpc`.
pub(crate) fn is_grpc_rpc(path: &hale_syntax::ast::QualifiedName) -> bool {
    let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
    matches!(segs.as_slice(), ["grpc", "Rpc"] | ["std", "api", "grpc", "Rpc"])
}

/// The transport literal as the exposure holds it: a socket transport is
/// spelled by its stdlib path and carries the number of the exposure it
/// serves (`tid`), which keys the bus subjects its connections share. The
/// HTTP transport's `codec: json` is the one codec v1 has, and is not a
/// field of the locus (nor is the gRPC transport's). The gRPC transport
/// also carries the name of the surface it serves, which a call's path names.
fn as_stdlib_unix(t: &Expr, id: i64, surface: &str, files: &str) -> Expr {
    let mut e = fresh(t);
    if let Expr::Struct { path, inits, span, .. } = &mut e {
        let kind = if is_unix_rpc(path) {
            Some("unix")
        } else if is_http_rpc(path) {
            Some("http")
        } else if is_mcp_rpc(path) {
            Some("mcp")
        } else if is_grpc_rpc(path) {
            Some("grpc")
        } else {
            None
        };
        if let Some(kind) = kind {
            let sp = *span;
            path.segments = ["std", "api", kind, "Rpc"].iter().map(|n| Ident::new(*n, sp)).collect();
            inits.retain(|i| {
                i.name.name != "tid"
                    && i.name.name != "surface"
                    && i.name.name != "files"
                    && !((kind == "http" || kind == "grpc") && i.name.name == "codec" && matches!(&i.value, Expr::Ident(c) if c.name == "json"))
            });
            inits.push(StructInit { name: Ident::new("tid", sp), value: lit_int(id, sp), span: sp });
            if kind == "grpc" {
                inits.push(StructInit { name: Ident::new("surface", sp), value: Expr::Literal(Literal::String(surface.to_string()), sp), span: sp });
                inits.push(StructInit { name: Ident::new("files", sp), value: Expr::Literal(Literal::String(files.to_string()), sp), span: sp });
            }
        }
    }
    e
}

/// The `path:` a Unix transport is built with, at a serve site or in the
/// param the site names; `None` for any other transport.
fn unix_path_of(l: &LocusDecl, transport: Option<&Expr>) -> Option<Expr> {
    let lit: &Expr = match transport? {
        e @ Expr::Struct { .. } => e,
        e => {
            let f = crate::surfaces::self_field(e)?;
            l.members.iter().find_map(|m| match m {
                LocusMember::Params(pb) => pb.params.iter().find_map(|p| match &p.init {
                    ParamInit::Value(v) if p.name.name == f => Some(v),
                    _ => None,
                }),
                _ => None,
            })?
        }
    };
    match lit {
        Expr::Struct { path, inits, .. } if is_unix_rpc(path) => {
            inits.iter().find(|i| i.name.name == "path").map(|i| fresh(&i.value))
        }
        _ => None,
    }
}

/// The `bind:` an HTTP transport is built with, at a serve site or in the
/// param the site names; `None` for any other transport.
fn http_bind_of(l: &LocusDecl, transport: Option<&Expr>) -> Option<Expr> {
    let lit: &Expr = match transport? {
        e @ Expr::Struct { .. } => e,
        e => {
            let f = crate::surfaces::self_field(e)?;
            l.members.iter().find_map(|m| match m {
                LocusMember::Params(pb) => pb.params.iter().find_map(|p| match &p.init {
                    ParamInit::Value(v) if p.name.name == f => Some(v),
                    _ => None,
                }),
                _ => None,
            })?
        }
    };
    match lit {
        Expr::Struct { path, inits, .. } if is_http_rpc(path) || is_mcp_rpc(path) || is_grpc_rpc(path) => {
            inits.iter().find(|i| i.name.name == "bind").map(|i| fresh(&i.value))
        }
        _ => None,
    }
}

/// Whether the transport a serve site names is `mcp::Rpc`, written at the
/// site or held as a param of the serving locus.
fn is_mcp_site(l: &LocusDecl, transport: Option<&Expr>) -> bool {
    site_literal_is(l, transport, is_mcp_rpc)
}

/// Whether the transport a serve site names is `grpc::Rpc`, written at the
/// site or held as a param of the serving locus.
fn is_grpc_site(l: &LocusDecl, transport: Option<&Expr>) -> bool {
    site_literal_is(l, transport, is_grpc_rpc)
}

fn site_literal_is(l: &LocusDecl, transport: Option<&Expr>, is: fn(&hale_syntax::ast::QualifiedName) -> bool) -> bool {
    let lit: Option<&Expr> = match transport {
        Some(e @ Expr::Struct { .. }) => Some(e),
        Some(e) => crate::surfaces::self_field(e).and_then(|f| {
            l.members.iter().find_map(|m| match m {
                LocusMember::Params(pb) => pb.params.iter().find_map(|p| match &p.init {
                    ParamInit::Value(v) if p.name.name == f => Some(v),
                    _ => None,
                }),
                _ => None,
            })
        }),
        None => None,
    };
    matches!(lit, Some(Expr::Struct { path, .. }) if is(path))
}

/// The listener of gRPC exposure `id` and its placement: the HTTP/2
/// server's (`std::io::h2`), a param of the serving main locus on an
/// `async_io` pool of its own that every gRPC exposure of the program shares.
fn parse_grpc_listener(id: i64, bind: Expr, span: Span) -> Option<(ParamDecl, hale_syntax::ast::PlacementEntry)> {
    let field = format!("__rpc_g_{id}");
    let src = format!(
        "main locus __Tmp {{ params {{ {field}: __StdIoH2Listener = __StdIoH2Listener {{ tid: {id}, what: \"grpc::Rpc\" }}; }} placement {{ {field}: cooperative(pool = __api_grpc) where async_io; }} }}\n"
    );
    let prog = parse_source_at(&src, API_SYNTH_BASE + 0x0800_0000 + id as u32 * 0x100).ok()?;
    let mut param = None;
    let mut entry = None;
    for item in prog.items {
        if let TopDecl::Locus(l) = item {
            for m in l.members {
                match m {
                    LocusMember::Params(pb) => param = pb.params.into_iter().next(),
                    LocusMember::Placement(pl) => entry = pl.entries.into_iter().next(),
                    _ => {}
                }
            }
        }
    }
    let mut param = param?;
    if let ParamInit::Value(Expr::Struct { inits, .. }) = &mut param.init {
        inits.push(StructInit { name: Ident::new("bind", span), value: bind, span });
    }
    Some((param, entry?))
}

/// The listener of HTTP exposure `id` and its placement: as the Unix one,
/// a param of the serving main locus on a pool of its own.
fn parse_http_listener(id: i64, bind: Expr, mcp: bool, span: Span) -> Option<(ParamDecl, hale_syntax::ast::PlacementEntry)> {
    let field = format!("__rpc_h_{id}");
    let src = format!(
        "main locus __Tmp {{ params {{ {field}: __StdApiHttpListener = __StdApiHttpListener {{ tid: {id} }}; }} placement {{ {field}: cooperative(pool = __api_http) where async_io; }} }}\n"
    );
    let prog = parse_source_at(&src, API_SYNTH_BASE + 0x0700_0000 + id as u32 * 0x100).ok()?;
    let mut param = None;
    let mut entry = None;
    for item in prog.items {
        if let TopDecl::Locus(l) = item {
            for m in l.members {
                match m {
                    LocusMember::Params(pb) => param = pb.params.into_iter().next(),
                    LocusMember::Placement(pl) => entry = pl.entries.into_iter().next(),
                    _ => {}
                }
            }
        }
    }
    let mut param = param?;
    if let ParamInit::Value(Expr::Struct { inits, .. }) = &mut param.init {
        inits.push(StructInit { name: Ident::new("bind", span), value: bind, span });
        if mcp {
            let what = Expr::Literal(Literal::String("mcp::Rpc".to_string()), span);
            inits.push(StructInit { name: Ident::new("what", span), value: what, span });
        }
    }
    Some((param, entry?))
}

/// The listener of Unix exposure `id` and its placement: a param of the
/// serving locus on a pool of its own (a socket's accept parks a
/// coroutine, so it needs an `async_io` pool, which only the main locus
/// places).
fn parse_unix_listener(id: i64, path: Expr, span: Span) -> Option<(ParamDecl, hale_syntax::ast::PlacementEntry)> {
    let field = format!("__rpc_u_{id}");
    let src = format!(
        "main locus __Tmp {{ params {{ {field}: __StdApiUnixListener = __StdApiUnixListener {{ tid: {id} }}; }} placement {{ {field}: cooperative(pool = __api_unix) where async_io; }} }}\n"
    );
    let prog = parse_source_at(&src, API_SYNTH_BASE + 0x0600_0000 + id as u32 * 0x100).ok()?;
    let mut param = None;
    let mut entry = None;
    for item in prog.items {
        if let TopDecl::Locus(l) = item {
            for m in l.members {
                match m {
                    LocusMember::Params(pb) => param = pb.params.into_iter().next(),
                    LocusMember::Placement(pl) => entry = pl.entries.into_iter().next(),
                    _ => {}
                }
            }
        }
    }
    let mut param = param?;
    if let ParamInit::Value(Expr::Struct { inits, .. }) = &mut param.init {
        inits.push(StructInit { name: Ident::new("path", span), value: path, span });
    }
    Some((param, entry?))
}

/// Insert `param` into `l`'s params at the param `at` names, in whichever
/// of the locus's params blocks declares it (offset 0 before it, 1 after
/// it), or at the end of the first block when none does.
fn insert_param_at(l: &mut LocusDecl, param: ParamDecl, name: &str, offset: usize) {
    let span = l.name.span;
    let holder = l.members.iter().position(|m| matches!(m, LocusMember::Params(pb) if pb.params.iter().any(|p| p.name.name == name)));
    let first = l.members.iter().position(|m| matches!(m, LocusMember::Params(_)));
    match holder.or(first) {
        Some(i) => {
            let LocusMember::Params(pb) = &mut l.members[i] else { return };
            let at = pb.params.iter().position(|p| p.name.name == name).map_or(pb.params.len(), |i| i + offset);
            pb.params.insert(at, param);
        }
        None => l.members.push(LocusMember::Params(hale_syntax::ast::ParamsBlock { params: vec![param], span })),
    }
}

/// Insert `param` into `l`'s params just before the param `before`.
fn insert_param_before(l: &mut LocusDecl, param: ParamDecl, before: &str) {
    insert_param_at(l, param, before, 0);
}

/// Insert `param` into `l`'s params just after the param `after`.
fn insert_param_after(l: &mut LocusDecl, param: ParamDecl, after: &str) {
    insert_param_at(l, param, after, 1);
}

/// Take `field`'s placement row out of `l`'s placement block when it puts
/// the field where a listener can run; a row that cannot is left for the
/// serve-site law to refuse.
fn take_listener_row(l: &mut LocusDecl, field: &str) -> Option<hale_syntax::ast::PlacementEntry> {
    for m in l.members.iter_mut() {
        let LocusMember::Placement(pb) = m else { continue };
        let at = pb.entries.iter().position(|e| e.field.name == field && hosts_listener(e))?;
        return Some(pb.entries.remove(at));
    }
    None
}

/// A placement row moved to another field: its spec and constraints, the
/// field's name.
fn rename_entry(e: Option<hale_syntax::ast::PlacementEntry>, field: &str) -> Option<hale_syntax::ast::PlacementEntry> {
    let mut e = e?;
    e.field = Ident::new(field, e.field.span);
    e.id = hale_syntax::ast::NodeId::NONE;
    Some(e)
}

/// Append a placement row to `l`'s placement block, making one if it has none.
fn push_placement(l: &mut LocusDecl, entry: hale_syntax::ast::PlacementEntry, span: Span) {
    if let Some(LocusMember::Placement(pl)) = l.members.iter_mut().find(|m| matches!(m, LocusMember::Placement(_))) {
        pl.entries.push(entry);
    } else {
        l.members.push(LocusMember::Placement(hale_syntax::ast::PlacementBlock { entries: vec![entry], span }));
    }
}

fn parse_params(src: &str) -> Option<ParamDecl> {
    let prog = parse_source_at(src, API_SYNTH_BASE + 0x0200_0000).ok()?;
    for item in prog.items {
        if let TopDecl::Locus(l) = item {
            for m in l.members {
                if let LocusMember::Params(pb) = m {
                    return pb.params.into_iter().next();
                }
            }
        }
    }
    None
}

fn parse_members(src: &str) -> Vec<LocusMember> {
    match parse_source_at(src, API_SYNTH_BASE + 0x0300_0000) {
        Ok(prog) => {
            for item in prog.items {
                if let TopDecl::Locus(l) = item {
                    return l.members;
                }
            }
            Vec::new()
        }
        Err(ds) => {
            eprintln!(
                "rpc_expand: generated receiver source did not parse: {}\n{}",
                ds.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; "),
                src
            );
            Vec::new()
        }
    }
}

fn find_locus_mut<'a>(items: &'a mut [TopDecl], name: &str) -> Option<&'a mut LocusDecl> {
    for item in items {
        match item {
            TopDecl::Locus(l) if l.name.name == name => return Some(l),
            TopDecl::Module(m) => {
                if let Some(l) = find_locus_mut(&mut m.items, name) {
                    return Some(l);
                }
            }
            _ => {}
        }
    }
    None
}

/// Write `key_param: key` into the literal that builds `self.<field>`: the
/// param's own, and the right-hand side of each `self.<field> = T { … }`.
fn inject_key(l: &mut LocusDecl, field: &str, key_param: &str, key: i64, span: Span) {
    let init = |inits: &mut Vec<StructInit>| {
        if !inits.iter().any(|i| i.name.name == key_param) {
            inits.push(StructInit { name: Ident::new(key_param, span), value: lit_int(key, span), span });
        }
    };
    for m in &mut l.members {
        match m {
            LocusMember::Params(pb) => {
                for p in &mut pb.params {
                    if p.name.name == field {
                        if let ParamInit::Value(Expr::Struct { inits, .. }) = &mut p.init {
                            init(inits);
                        }
                    }
                }
            }
            LocusMember::Lifecycle(lc) => inject_block(&mut lc.body, field, &init),
            LocusMember::Failure(fd) => inject_block(&mut fd.body, field, &init),
            LocusMember::Fn(f) => inject_block(&mut f.body, field, &init),
            LocusMember::Mode(md) => inject_block(&mut md.body, field, &init),
            _ => {}
        }
    }
}

fn inject_block(b: &mut Block, field: &str, init: &dyn Fn(&mut Vec<StructInit>)) {
    for s in &mut b.stmts {
        match s {
            Stmt::Assign { target, value, .. } => {
                let is_field = target.head.name == "self"
                    && matches!(target.tail.as_slice(), [hale_syntax::ast::LValueSeg::Field(f)] if f.name == field);
                if is_field {
                    if let Expr::Struct { inits, .. } = value {
                        init(inits);
                    }
                }
            }
            Stmt::If(i) => inject_if(i, field, init),
            Stmt::For { body, .. } | Stmt::While { body, .. } => inject_block(body, field, init),
            Stmt::Block(b) => inject_block(b, field, init),
            _ => {}
        }
    }
}

fn inject_if(i: &mut hale_syntax::ast::IfStmt, field: &str, init: &dyn Fn(&mut Vec<StructInit>)) {
    inject_block(&mut i.then_block, field, init);
    match i.else_block.as_deref_mut() {
        Some(hale_syntax::ast::ElseBranch::Else(b)) => inject_block(b, field, init),
        Some(hale_syntax::ast::ElseBranch::ElseIf(e)) => inject_if(e, field, init),
        None => {}
    }
}

const TOPICS_SRC: &str = "topic __ApiRpcIngressT { payload: __StdApiRpcIngress; subject: \"__api.rpc.ingress\"; keyed_by exposure; }
topic __ApiRpcLostT { payload: __StdApiRpcLost; subject: \"__api.rpc.lost\"; keyed_by exposure; }
topic __ApiRpcCallT { payload: __StdApiRpcCall; subject: \"__api.rpc.call\"; keyed_by key; on_unmatched: fail; }
topic __ApiRpcHelloT { payload: __StdApiRpcHello; subject: \"__api.rpc.hello\"; keyed_by key; on_unmatched: fail; }
topic __ApiRpcEventT { payload: __StdApiRpcEvent; subject: \"__api.rpc.event\"; keyed_by exposure; }
topic __ApiUnixOutT { payload: __StdApiUnixOut; subject: \"__api.unix.out\"; keyed_by key; }
topic __ApiHttpOutT { payload: __StdApiHttpOut; subject: \"__api.http.out\"; keyed_by key; }
";

/// The rows adapter of exposure `id`.
fn surface_src(
    id: i64,
    rows: &[Row],
    slots: &[String],
    confer: &BTreeMap<String, Vec<String>>,
    codec: &Codec<'_>,
    pb: Option<&PbCodec<'_>>,
    schemas: &crate::surfaces::Schemas<'_>,
) -> String {
    let mut s = String::new();
    s.push_str(&format!("locus __RpcSurface_{id} {{\n"));
    s.push_str(&format!("    fn count() -> Int {{ return {}; }}\n", rows.len()));
    s.push_str(&format!("    fn slots() -> Int {{ return {}; }}\n", slots.len()));
    s.push_str("    fn member(name: String) -> Int {\n");
    for (i, r) in rows.iter().enumerate() {
        s.push_str(&format!("        if name == {} {{ return {i}; }}\n", q(&r.member)));
    }
    s.push_str("        return -1;\n    }\n    fn name_of(i: Int) -> String {\n");
    for (i, r) in rows.iter().enumerate() {
        s.push_str(&format!("        if i == {i} {{ return {}; }}\n", q(&r.member)));
    }
    s.push_str("        return \"\";\n    }\n    fn requires(i: Int) -> String {\n");
    for (i, r) in rows.iter().enumerate() {
        if !r.requires.is_empty() {
            s.push_str(&format!("        if i == {i} {{ return {}; }}\n", q(&r.requires.join(","))));
        }
    }
    s.push_str("        return \"\";\n    }\n    fn receiver(i: Int) -> Int {\n");
    for (i, r) in rows.iter().enumerate() {
        let slot = slots.iter().position(|t| t == &r.locus).unwrap_or(0);
        if slot != 0 {
            s.push_str(&format!("        if i == {i} {{ return {slot}; }}\n"));
        }
    }
    s.push_str("        return 0;\n    }\n    fn check(i: Int, body: String) -> String {\n");
    for (i, r) in rows.iter().enumerate() {
        if let Some(te) = &r.request {
            let bail = "return \"wrong_type: payload\";";
            let is_struct = matches!(
                codec.shapes.classify(&codec.shapes.dealias(te)),
                TypeClass::Struct { .. } | TypeClass::Builtin(_)
            );
            let bail = if is_struct { "return err.kind + \": \" + err.field;" } else { bail };
            // a protobuf call (`pb::` and the message in base64) is checked by
            // its own codec: the same words for the same faults
            let pb_check = pb.map_or(String::new(), |pb| {
                format!(
                    "            if std::str::starts_with(body, \"pb:\") {{\n                let __raw = std::text::base64::decode(body[4..len(body)]);\n                {}                return \"\";\n            }}\n",
                    pb.decode(te, "__raw", "__v", bail)
                )
            });
            s.push_str(&format!(
                "        if i == {i} {{\n{pb_check}            {}            return \"\";\n        }}\n",
                codec.decode(te, "body", "__v", bail)
            ));
        }
    }
    s.push_str("        return \"\";\n    }\n    fn grants(role: String) -> String {\n");
    for (role, who) in confer {
        if who.len() > 1 {
            s.push_str(&format!("        if role == {} {{ return {}; }}\n", q(role), q(&who.join(","))));
        }
    }
    s.push_str("        return role;\n    }\n");
    s.push_str(&description_src(rows, schemas));
    s.push_str("}\n");
    s
}

/// A string as a JSON string, compact.
fn jstr(v: &str) -> String {
    let mut o = String::from("\"");
    for c in v.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// A schema as compact JSON (the form `hale check --api` prints, pretty).
fn field_json(f: &crate::surfaces::FieldSchema) -> String {
    use crate::surfaces::FieldSchema;
    match f {
        FieldSchema::Scalar { json, hale_type, unit } => {
            let mut o = format!("{{\"type\":{}", jstr(json));
            if let Some(t) = hale_type {
                o.push_str(&format!(",\"x-hale-type\":{}", jstr(t)));
            }
            if let Some(u) = unit {
                o.push_str(&format!(",\"x-hale-unit\":{}", jstr(u)));
            }
            o.push('}');
            o
        }
        FieldSchema::Ref(name) => format!("{{\"$ref\":{}}}", jstr(&format!("#/schemas/{name}"))),
        FieldSchema::Time => "{\"type\":\"string\",\"format\":\"date-time\",\"x-hale-type\":\"Time\"}".to_string(),
        FieldSchema::List { items, max, exact } => {
            let min = if *exact { format!(",\"minItems\":{max}") } else { String::new() };
            format!("{{\"type\":\"array\",\"items\":{}{min},\"maxItems\":{max}}}", field_json(items))
        }
        FieldSchema::Nullable { inner, .. } => format!("{{\"anyOf\":[{},{{\"type\":\"null\"}}]}}", field_json(inner)),
        // no key of its own: `type_json` leaves it out
        FieldSchema::Presence { .. } | FieldSchema::Unformed => "{}".to_string(),
        FieldSchema::Enum { name, variants } => {
            let units: Vec<String> = variants.iter().filter(|(_, v)| v.is_empty()).map(|(n, _)| jstr(n)).collect();
            let unit = format!("{{\"type\":\"string\",\"enum\":[{}]", units.join(","));
            if units.len() == variants.len() {
                return format!("{unit},\"x-hale-type\":{}}}", jstr(name));
            }
            let mut one: Vec<String> = Vec::new();
            if !units.is_empty() {
                one.push(format!("{unit}}}"));
            }
            for (v, values) in variants.iter().filter(|(_, v)| !v.is_empty()) {
                let value = match values.as_slice() {
                    [only] => field_json(only),
                    many => format!(
                        "{{\"type\":\"array\",\"prefixItems\":[{}],\"minItems\":{n},\"maxItems\":{n}}}",
                        many.iter().map(field_json).collect::<Vec<_>>().join(","),
                        n = many.len()
                    ),
                };
                one.push(format!(
                    "{{\"type\":\"object\",\"properties\":{{{}:{value}}},\"required\":[{}],\"additionalProperties\":false}}",
                    jstr(v),
                    jstr(v)
                ));
            }
            format!("{{\"oneOf\":[{}],\"x-hale-type\":{}}}", one.join(","), jstr(name))
        }
        FieldSchema::Map { key, value, max, .. } => format!(
            "{{\"type\":\"object\",\"additionalProperties\":{},\"maxProperties\":{max},\"x-hale-key\":{}}}",
            type_json(value),
            jstr(key)
        ),
    }
}

fn type_json(t: &crate::surfaces::TypeSchema) -> String {
    let props: Vec<String> = t
        .properties
        .iter()
        .filter(|(_, f)| !matches!(f, crate::surfaces::FieldSchema::Presence { .. }))
        .map(|(k, f)| format!("{}:{}", jstr(k), field_json(f)))
        .collect();
    let req: Vec<String> = t.required.iter().map(|r| jstr(r)).collect();
    format!("{{\"type\":\"object\",\"properties\":{{{}}},\"required\":[{}]}}", props.join(","), req.join(","))
}

/// The static parts of a description (the rows adapter's `member_doc`,
/// `refs`, `schema_*` and `roles`): each member as a JSON object, the
/// schemas it reaches by name, the schemas of the surface in name order and
/// the roles its rows require. The runtime filters these by caller.
fn description_src(rows: &[Row], schemas: &crate::surfaces::Schemas<'_>) -> String {
    let mut all: BTreeMap<String, crate::surfaces::TypeSchema> = BTreeMap::new();
    let mut docs: Vec<String> = Vec::new();
    let mut refs: Vec<String> = Vec::new();
    for r in rows {
        let mut mine: BTreeMap<String, crate::surfaces::TypeSchema> = BTreeMap::new();
        let mut ty = |t: &Option<TypeExpr>| match t {
            Some(te) => field_json(&schemas.type_ref(te, &mut mine)),
            None => "null".to_string(),
        };
        let request = ty(&r.request);
        let response = ty(&r.response);
        let error = if r.server_error { jstr("ClosureViolation") } else { ty(&r.error) };
        let req: Vec<String> = r.requires.iter().map(|x| jstr(x)).collect();
        docs.push(format!(
            "{{\"name\":{},\"request\":{request},\"response\":{response},\"error\":{error},\"requires\":[{}]}}",
            jstr(&r.member),
            req.join(",")
        ));
        refs.push(mine.keys().cloned().collect::<Vec<_>>().join(","));
        all.extend(mine);
    }
    let roles: BTreeSet<&str> = rows.iter().flat_map(|r| r.requires.iter().map(String::as_str)).collect();
    let mut s = String::new();
    s.push_str("    fn member_doc(i: Int) -> String {\n");
    for (i, d) in docs.iter().enumerate() {
        s.push_str(&format!("        if i == {i} {{ return {}; }}\n", q(d)));
    }
    s.push_str("        return \"\";\n    }\n    fn refs(i: Int) -> String {\n");
    for (i, r) in refs.iter().enumerate() {
        if !r.is_empty() {
            s.push_str(&format!("        if i == {i} {{ return {}; }}\n", q(r)));
        }
    }
    s.push_str(&format!("        return \"\";\n    }}\n    fn schema_count() -> Int {{ return {}; }}\n", all.len()));
    s.push_str("    fn schema_name(k: Int) -> String {\n");
    for (k, name) in all.keys().enumerate() {
        s.push_str(&format!("        if k == {k} {{ return {}; }}\n", q(name)));
    }
    s.push_str("        return \"\";\n    }\n    fn schema_doc(k: Int) -> String {\n");
    for (k, t) in all.values().enumerate() {
        s.push_str(&format!("        if k == {k} {{ return {}; }}\n", q(&type_json(t))));
    }
    s.push_str(&format!(
        "        return \"\";\n    }}\n    fn roles() -> String {{ return {}; }}\n",
        q(&roles.into_iter().collect::<Vec<_>>().join(","))
    ));
    s
}

/// Add the plumbing of spec/api.md § Receiver failure and generations to a
/// receiver type, for each exposure that binds it.
fn extend_receiver(l: &mut LocusDecl, bs: &[Binding], codec: &Codec<'_>, pb: &PbCodec<'_>) {
    let span = l.name.span;
    if l.members.iter().any(|m| {
        matches!(m, LocusMember::Fn(f) if f.name.name == "__rpc_ev")
    }) {
        return;
    }
    let mut params = String::from("__rpc_stamp: Int = 0;\n");
    let mut bus = String::from("publish __ApiRpcEventT;\n");
    let mut fns = String::from(
        "fn __rpc_ev(exposure: Int, rid: Int, slot: Int, kind: Int, body: String) {\n    __ApiRpcEventT <- std::api::RpcEvent { exposure: exposure, rid: rid, slot: slot, kind: kind, stamp: self.__rpc_stamp, body: body };\n}\n",
    );
    let mut birth = String::from("self.__rpc_stamp = std::time::monotonic_ns();\n");
    let mut dissolve = String::new();
    for b in bs {
        let n = b.exposure;
        let slot = b.slot;
        params.push_str(&format!("__rpc_s_{n}: Int = 0;\n"));
        bus.push_str(&format!(
            "subscribe __ApiRpcCallT as __rpc_call_{n} where key == self.__rpc_s_{n};\nsubscribe __ApiRpcHelloT as __rpc_hello_{n} where key == self.__rpc_s_{n};\n"
        ));
        birth.push_str(&format!(
            "if self.__rpc_s_{n} != 0 {{ self.__rpc_ev({n}, 0, {slot}, 5, \"\"); }}\n"
        ));
        dissolve.push_str(&format!(
            "if self.__rpc_s_{n} != 0 {{ self.__rpc_ev({n}, 0, {slot}, 6, \"\"); }}\n"
        ));
        fns.push_str(&format!(
            "fn __rpc_hello_{n}(h: std::api::RpcHello) {{ self.__rpc_ev(h.exposure, 0, h.slot, 5, \"\"); }}\n"
        ));
        fns.push_str(&format!(
            "fn __rpc_call_{n}(c: std::api::RpcCall) {{\n    if c.stamp != self.__rpc_stamp {{\n        self.__rpc_ev(c.exposure, c.rid, {slot}, 3, \"\");\n        return;\n    }}\n    self.__rpc_ev(c.exposure, c.rid, {slot}, 4, \"\");\n"
        ));
        for (m, row) in &b.members {
            fns.push_str(&format!("    if c.member == {m} {{\n"));
            if b.grpc {
                // the call arrived as protobuf: the same steps, the other codec
                fns.push_str("        if std::str::starts_with(c.body, \"pb:\") {\n");
                fns.push_str(&thunk_body_pb(row, slot, &PbCodec { surface: &b.surface, ..*pb }));
                fns.push_str("        return;\n        }\n");
            }
            fns.push_str(&thunk_body(row, slot, codec));
            fns.push_str("        return;\n    }\n");
        }
        fns.push_str("}\n");
    }
    let src = format!(
        "locus __Tmp {{\n params {{ {params} }}\n bus {{ {bus} }}\n {fns}\n birth() {{ {birth} }}\n dissolve() {{ {dissolve} }}\n}}\n"
    );
    let parsed = parse_members(&src);
    for m in parsed {
        match m {
            LocusMember::Params(pb) => {
                if let Some(LocusMember::Params(have)) = l.members.iter_mut().find(|m| matches!(m, LocusMember::Params(_))) {
                    have.params.extend(pb.params);
                } else {
                    l.members.push(LocusMember::Params(pb));
                }
            }
            LocusMember::Bus(bb) => {
                if let Some(LocusMember::Bus(have)) = l.members.iter_mut().find(|m| matches!(m, LocusMember::Bus(_))) {
                    for member in bb.members {
                        let dup = matches!(&member, hale_syntax::ast::BusMember::Publish { subject, .. }
                            if have.members.iter().any(|h| matches!(h, hale_syntax::ast::BusMember::Publish { subject: s, .. } if s.canonical() == subject.canonical())));
                        if !dup {
                            have.members.push(member);
                        }
                    }
                } else {
                    l.members.push(LocusMember::Bus(bb));
                }
            }
            LocusMember::Lifecycle(lc) if matches!(lc.kind, LifecycleKind::Birth | LifecycleKind::Dissolve) => {
                let existing = l.members.iter_mut().find_map(|m| match m {
                    LocusMember::Lifecycle(have) if have.kind == lc.kind => Some(have),
                    _ => None,
                });
                match existing {
                    Some(have) => {
                        // the plumbing first: the stamp is set and the
                        // announcement made before the author's body runs
                        let mut stmts = lc.body.stmts;
                        stmts.extend(std::mem::take(&mut have.body.stmts));
                        have.body.stmts = stmts;
                    }
                    None => l.members.push(LocusMember::Lifecycle(lc)),
                }
            }
            other => l.members.push(other),
        }
    }
    let _ = span;
}

/// The statements that run one call of `row` on its receiver: decode the
/// request, call the handler through the fallible ABI, publish the outcome.
fn thunk_body(row: &Row, slot: usize, codec: &Codec<'_>) -> String {
    thunk_steps(row, slot, Wire::Json(codec))
}

/// The same steps for a call that arrived as protobuf.
fn thunk_body_pb(row: &Row, slot: usize, pb: &PbCodec<'_>) -> String {
    thunk_steps(row, slot, Wire::Pb(pb))
}

/// Which codec a call crosses by: the row's JSON, or its protobuf.
#[derive(Clone, Copy)]
enum Wire<'a> {
    Json(&'a Codec<'a>),
    Pb(&'a PbCodec<'a>),
}

impl Wire<'_> {
    /// Statements that decode the call's body as the request into `__req`.
    fn decode(&self, row: &Row, te: &TypeExpr, bail: &str) -> String {
        let _ = row;
        match self {
            Wire::Json(c) => c.decode(te, "c.body", "__req", bail),
            Wire::Pb(p) => format!(
                "let __raw = std::text::base64::decode(c.body[4..len(c.body)]);\n{}",
                p.decode(te, "__raw", "__req", bail)
            ),
        }
    }
    /// The text of a result body.
    fn ok(&self, te: &TypeExpr, v: &str) -> String {
        match self {
            Wire::Json(c) => c.encode(te, v),
            Wire::Pb(p) => format!("\"pb::\" + std::text::base64::encode({})", p.encode(te, v)),
        }
    }
    /// The body of a result of nothing.
    fn null(&self) -> &'static str {
        match self {
            Wire::Json(_) => "\"null\"",
            Wire::Pb(_) => "\"pb::\"",
        }
    }
    /// The text of a handler error's body: a protobuf one names its message.
    fn err(&self, row: &Row, te: &TypeExpr) -> String {
        match self {
            Wire::Json(c) => c.encode(te, "err"),
            Wire::Pb(p) => format!(
                "\"pb:{}:\" + std::text::base64::encode({})",
                p.message(&row.member, "Error", te),
                p.encode(te, "err")
            ),
        }
    }
}

fn thunk_steps(row: &Row, slot: usize, wire: Wire<'_>) -> String {
    let mut s = String::new();
    let bail = format!("self.__rpc_ev(c.exposure, c.rid, {slot}, 3, \"\"); return;");
    let mut args = String::new();
    if let Some(te) = &row.request {
        s.push_str(&wire.decode(row, te, &bail));
        args.push_str("__req");
    }
    if row.served_ctx {
        s.push_str("let __ctx = std::api::ServedContext { caller: c.caller, role: c.role, request_id: c.rid, via: c.via, exposure: c.exposure_name, generation: c.generation };\n");
    } else if row.takes_ctx {
        s.push_str("let __ctx = std::api::Context { caller: c.caller, role: c.role, request_id: c.rid, via: c.via };\n");
    }
    if row.takes_ctx {
        if !args.is_empty() {
            args.push_str(", ");
        }
        args.push_str("__ctx");
    }
    let call = format!("self.{}({})", row.method, args);
    let fail_branch = if row.error.is_none() {
        None
    } else if row.server_error {
        Some(format!("self.__rpc_ev(c.exposure, c.rid, {slot}, 2, \"\"); return;"))
    } else {
        let e = row.error.as_ref().expect("an error type");
        Some(format!(
            "self.__rpc_ev(c.exposure, c.rid, {slot}, 1, {}); return;",
            wire.err(row, e)
        ))
    };
    match (&row.response, fail_branch) {
        (Some(resp), Some(fb)) => {
            s.push_str(&format!("let __v = {call} or {{ {fb} }};\n"));
            s.push_str(&format!(
                "self.__rpc_ev(c.exposure, c.rid, {slot}, 0, {});\n",
                wire.ok(resp, "__v")
            ));
        }
        (Some(resp), None) => {
            s.push_str(&format!("let __v = {call};\n"));
            s.push_str(&format!(
                "self.__rpc_ev(c.exposure, c.rid, {slot}, 0, {});\n",
                wire.ok(resp, "__v")
            ));
        }
        (None, Some(fb)) => {
            s.push_str(&format!("{call} or {{ {fb} }};\n"));
            s.push_str(&format!("self.__rpc_ev(c.exposure, c.rid, {slot}, 0, {});\n", wire.null()));
        }
        (None, None) => {
            s.push_str(&format!("{call};\n"));
            s.push_str(&format!("self.__rpc_ev(c.exposure, c.rid, {slot}, 0, {});\n", wire.null()));
        }
    }
    s
}


// ---- the holder of an exposure, and the main locus that places its listener (GH #1565) ----

/// One way a main locus holds a locus: the params from the main locus down
/// to it, each the locus that declares the param and the param.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Chain {
    pub main: String,
    /// `(owner, field)`, the main locus's first; the last field is the held
    /// locus.
    pub fields: Vec<(String, String)>,
}

impl Chain {
    /// The fields as a path from the main locus: `admin.server`.
    pub(crate) fn path(&self) -> String {
        self.fields.iter().map(|(_, f)| f.as_str()).collect::<Vec<_>>().join(".")
    }
}

/// Every locus the programs declare, by name.
pub(crate) fn loci_by_name<'a>(programs: &[&'a Program]) -> BTreeMap<&'a str, &'a LocusDecl> {
    let mut loci = BTreeMap::new();
    for p in programs {
        for d in flat_decls(&p.items) {
            if let TopDecl::Locus(l) = d {
                loci.insert(l.name.name.as_str(), l);
            }
        }
    }
    loci
}

/// The params of a locus in declaration order, each its name and its type
/// as written (the declared type, or the path of its literal).
fn params_in_order(l: &LocusDecl) -> Vec<(&ParamDecl, Option<String>)> {
    let types = crate::surfaces::param_types(l);
    l.members
        .iter()
        .filter_map(|m| match m {
            LocusMember::Params(pb) => Some(pb.params.iter()),
            _ => None,
        })
        .flatten()
        .map(|p| (p, types.get(&p.name.name).cloned()))
        .collect()
}

/// Every chain by which a main locus holds `holder` through its params and
/// theirs: the placed tree, which only a main locus roots.
pub(crate) fn holder_chains(loci: &BTreeMap<&str, &LocusDecl>, holder: &str) -> Vec<Chain> {
    fn walk<'a>(
        loci: &BTreeMap<&str, &'a LocusDecl>,
        l: &'a LocusDecl,
        holder: &str,
        main: &str,
        path: &mut Vec<(String, String)>,
        on: &mut BTreeSet<String>,
        out: &mut Vec<Chain>,
    ) {
        if path.len() >= 16 {
            return;
        }
        for (p, ty) in params_in_order(l) {
            let Some(ty) = ty else { continue };
            path.push((l.name.name.clone(), p.name.name.clone()));
            if ty == holder {
                out.push(Chain { main: main.to_string(), fields: path.clone() });
            } else if let Some(child) = loci.get(ty.as_str()) {
                if !child.is_main && on.insert(ty.clone()) {
                    walk(loci, child, holder, main, path, on, out);
                    on.remove(&ty);
                }
            }
            path.pop();
        }
    }
    let mut out = Vec::new();
    for (name, l) in loci {
        if !l.is_main || *name == holder {
            continue;
        }
        let mut on = BTreeSet::from([name.to_string()]);
        walk(loci, l, holder, name, &mut Vec::new(), &mut on, &mut out);
    }
    out
}

/// The placement entry a main locus writes for `field`.
pub(crate) fn placement_entry<'a>(main: &'a LocusDecl, field: &str) -> Option<&'a hale_syntax::ast::PlacementEntry> {
    main.members.iter().find_map(|m| match m {
        LocusMember::Placement(pb) => pb.entries.iter().find(|e| e.field.name == field),
        _ => None,
    })
}

/// Whether a placement entry puts its field where a listener can run: a
/// named cooperative pool `where async_io`, the one place an accept or a
/// read parks a coroutine instead of the thread.
pub(crate) fn hosts_listener(e: &hale_syntax::ast::PlacementEntry) -> bool {
    use hale_syntax::ast::{PlacementConstraint, PlacementSpec};
    matches!(&e.spec, PlacementSpec::Cooperative { pool: Some(p), .. } if p.name != "main")
        && e.constraints.iter().any(|c| matches!(c.kind, PlacementConstraint::AsyncIo))
}

/// How many instances a placement entry fans its field into.
pub(crate) fn entry_replicas(e: &hale_syntax::ast::PlacementEntry) -> i64 {
    match &e.spec {
        hale_syntax::ast::PlacementSpec::Pinned { replicas: Some(k), .. } => *k,
        _ => 1,
    }
}

/// Whether an address written in a holder reads the holder (`self.bind`):
/// then the main locus can read it only once the holder is built.
pub(crate) fn reads_holder(e: &Expr) -> bool {
    let mut e = e.clone();
    let mut found = false;
    walk_expr_mut(&mut e, &mut |x| {
        if matches!(x, Expr::KwSelf(_)) {
            found = true;
        }
    });
    found
}

/// An exposure's listener address (`bind:`, `path:`), written in its
/// holder, as the main locus reads it: the listener is a param of the main
/// locus born after the param the chain starts at, so each `self.<param>`
/// the address reads is the holder's field as its construction built it,
/// read down the chain (`self.server.bind`), whatever literal built it (a
/// default, or an override at a construction of the main locus). A method
/// of `self` or `self` itself cannot be read there; the error names it.
pub(crate) fn address_for_main(chain: &Chain, e: &Expr) -> Result<Expr, String> {
    const MAIN_SELF: &str = "__rpc_main_self";
    let mut out = fresh(e);
    let mut err: Option<String> = None;
    walk_expr_mut(&mut out, &mut |x| {
        if err.is_some() {
            return;
        }
        match x {
            Expr::Call { callee, .. } if matches!(callee.as_ref(), Expr::Field { receiver, .. } if matches!(receiver.as_ref(), Expr::KwSelf(_))) => {
                if let Expr::Field { name, .. } = callee.as_ref() {
                    err = Some(format!("`self.{}(…)`, a method of the holder", name.name));
                }
            }
            Expr::Field { receiver, .. } if matches!(receiver.as_ref(), Expr::KwSelf(_)) => {
                let sp = receiver.span();
                let mut base = Expr::Ident(Ident::new(MAIN_SELF, sp));
                for (_, f) in &chain.fields {
                    base = Expr::Field { receiver: Box::new(base), name: Ident::new(f, sp), span: sp };
                }
                *receiver = Box::new(base);
            }
            Expr::KwSelf(_) => err = Some("`self` itself".to_string()),
            _ => {}
        }
    });
    if let Some(e) = err {
        return Err(e);
    }
    walk_expr_mut(&mut out, &mut |x| {
        if let Expr::Ident(i) = x {
            if i.name == MAIN_SELF {
                *x = Expr::KwSelf(i.span);
            }
        }
    });
    Ok(out)
}

/// Whether a serve site's transport has a listener the compiler adds: a
/// socket transport (`unix::Rpc`, `http::Rpc`, `mcp::Rpc`, `grpc::Rpc`),
/// written at the site or held as a param of the holder.
pub(crate) fn has_listener(l: &LocusDecl, transport: Option<&Expr>) -> bool {
    unix_path_of(l, transport).is_some() || http_bind_of(l, transport).is_some()
}

/// The address a site's listener binds, as written in its holder.
pub(crate) fn listener_address(l: &LocusDecl, transport: Option<&Expr>) -> Option<Expr> {
    unix_path_of(l, transport).or_else(|| http_bind_of(l, transport))
}

// ---- the literals that build a serving locus (a construction site) ----

/// Every expression of `e`, pre-order, `f` first on each.
fn walk_expr_mut(e: &mut Expr, f: &mut dyn FnMut(&mut Expr)) {
    f(e);
    match e {
        Expr::Binary { left, right, .. } => {
            walk_expr_mut(left, f);
            walk_expr_mut(right, f);
        }
        Expr::Unary { operand, .. } => walk_expr_mut(operand, f),
        Expr::Call { callee, args, .. } => {
            walk_expr_mut(callee, f);
            args.iter_mut().for_each(|a| walk_expr_mut(a, f));
        }
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => walk_expr_mut(receiver, f),
        Expr::Index { receiver, index, .. } => {
            walk_expr_mut(receiver, f);
            walk_expr_mut(index, f);
        }
        Expr::Tuple(es, _) | Expr::Array(es, _) => es.iter_mut().for_each(|a| walk_expr_mut(a, f)),
        Expr::Struct { inits, .. } => inits.iter_mut().for_each(|i| walk_expr_mut(&mut i.value, f)),
        Expr::Block(b) => walk_block_mut(b, f),
        Expr::If(i) => walk_if_mut(i, f),
        Expr::Match(m) => walk_match_mut(m, f),
        Expr::Sum(a, _) | Expr::Prod(a, _) => walk_expr_mut(a, f),
        Expr::Approx { left, right, tolerance, .. } => {
            walk_expr_mut(left, f);
            walk_expr_mut(right, f);
            walk_expr_mut(tolerance, f);
        }
        Expr::Range { lo, hi, .. } => {
            walk_expr_mut(lo, f);
            walk_expr_mut(hi, f);
        }
        Expr::ArrayRepeat { val, .. } => walk_expr_mut(val, f),
        Expr::Or { inner, disposition, .. } => {
            walk_expr_mut(inner, f);
            match disposition {
                hale_syntax::ast::OrDisposition::Substitute(s) | hale_syntax::ast::OrDisposition::Fail(s, _) => walk_expr_mut(s, f),
                _ => {}
            }
        }
        Expr::Literal(..) | Expr::Ident(_) | Expr::Path(_) | Expr::KwSelf(_) => {}
    }
}

fn walk_if_mut(i: &mut hale_syntax::ast::IfStmt, f: &mut dyn FnMut(&mut Expr)) {
    walk_expr_mut(&mut i.cond, f);
    walk_block_mut(&mut i.then_block, f);
    match i.else_block.as_deref_mut() {
        Some(hale_syntax::ast::ElseBranch::Else(b)) => walk_block_mut(b, f),
        Some(hale_syntax::ast::ElseBranch::ElseIf(e)) => walk_if_mut(e, f),
        None => {}
    }
}

fn walk_match_mut(m: &mut hale_syntax::ast::MatchStmt, f: &mut dyn FnMut(&mut Expr)) {
    walk_expr_mut(&mut m.scrutinee, f);
    for arm in &mut m.arms {
        if let Some(g) = &mut arm.guard {
            walk_expr_mut(g, f);
        }
        match &mut arm.body {
            hale_syntax::ast::MatchArmBody::Expr(e) => walk_expr_mut(e, f),
            hale_syntax::ast::MatchArmBody::Block(b) => walk_block_mut(b, f),
        }
    }
}

pub(crate) fn walk_block_mut(b: &mut Block, f: &mut dyn FnMut(&mut Expr)) {
    for s in &mut b.stmts {
        match s {
            Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } | Stmt::Assign { value, .. } => walk_expr_mut(value, f),
            Stmt::If(i) => walk_if_mut(i, f),
            Stmt::Match(m) => walk_match_mut(m, f),
            Stmt::For { iter, body, .. } => {
                walk_expr_mut(iter, f);
                walk_block_mut(body, f);
            }
            Stmt::While { cond, body, .. } => {
                walk_expr_mut(cond, f);
                walk_block_mut(body, f);
            }
            Stmt::Return(Some(e), _) => walk_expr_mut(e, f),
            Stmt::Fail { value, .. } => walk_expr_mut(value, f),
            Stmt::Block(b) => walk_block_mut(b, f),
            Stmt::Recovery { args, .. } => args.iter_mut().for_each(|a| walk_expr_mut(a, f)),
            Stmt::Violate { payload: Some(p), .. } => walk_expr_mut(p, f),
            Stmt::Send { subject, value, .. } => {
                walk_expr_mut(subject, f);
                walk_expr_mut(value, f);
            }
            Stmt::ShmWrite { max, body, .. } => {
                walk_expr_mut(max, f);
                walk_block_mut(body, f);
            }
            Stmt::Expr(e) => walk_expr_mut(e, f),
            _ => {}
        }
    }
    if let Some(t) = &mut b.tail {
        walk_expr_mut(t, f);
    }
}

fn walk_items_exprs_mut(items: &mut [TopDecl], f: &mut dyn FnMut(&mut Expr)) {
    for d in items {
        match d {
            TopDecl::Fn(func) => walk_block_mut(&mut func.body, f),
            TopDecl::Const(c) => walk_expr_mut(&mut c.value, f),
            TopDecl::Module(m) => walk_items_exprs_mut(&mut m.items, f),
            TopDecl::Locus(l) => {
                for m in &mut l.members {
                    match m {
                        LocusMember::Params(pb) => {
                            for p in &mut pb.params {
                                if let ParamInit::Value(e) = &mut p.init {
                                    walk_expr_mut(e, f);
                                }
                            }
                        }
                        LocusMember::Lifecycle(lc) => walk_block_mut(&mut lc.body, f),
                        LocusMember::Failure(fd) => walk_block_mut(&mut fd.body, f),
                        LocusMember::Mode(md) => walk_block_mut(&mut md.body, f),
                        LocusMember::Fn(func) => walk_block_mut(&mut func.body, f),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

/// Write `key_param: key` into the literal that builds `field` wherever a
/// literal of the serving locus `serving` is written: a program that
/// builds its serving locus from another fn (`Head { commands: Commands {
/// … } }`) overrides the param's default literal, and the instance is
/// numbered in the one that builds it.
fn number_constructions(items: &mut [TopDecl], serving: &str, field: &str, key_param: &str, key: i64) {
    walk_items_exprs_mut(items, &mut |e| {
        let Expr::Struct { path, inits, .. } = e else { return };
        if path.segments.last().map(|s| s.name.as_str()) != Some(serving) {
            return;
        }
        for i in inits.iter_mut().filter(|i| i.name.name == field) {
            if let Expr::Struct { inits: inner, span, .. } = &mut i.value {
                if !inner.iter().any(|x| x.name.name == key_param) {
                    let sp = *span;
                    inner.push(StructInit { name: Ident::new(key_param, sp), value: lit_int(key, sp), span: sp });
                }
            }
        }
    });
}
