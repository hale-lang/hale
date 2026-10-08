//! GH #1417 (R2a): `api::serve` builds an exposure.
//!
//! A serve site is read by the check as the call it is (`surfaces.rs`
//! names it, the serve-site laws judge it); this pass, which runs in the
//! desugar sequence before the check (`desugar_sequence`), is what makes
//! a program that holds one buildable. For each well-formed site it adds,
//! as `api_gen` does for the structural path, source the compiler writes:
//!
//! - on the serving locus, a param `__rpc_x_<as>: std::api::Exposure`
//!   built from the site (its surface, digest, bound, transport instance,
//!   the rows adapter below, the bearer and role sources copied from the
//!   transport's fields), declared last, so the exposure is born with the
//!   locus after the receivers and the transport it reads. The
//!   `api::serve(…)` call itself stays in the body: the check types it as
//!   that exposure and lowering reads the param (`lower_serve_site`);
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

use hale_syntax::api_gen::API_SYNTH_BASE;
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

/// The param a serve site's exposure is, by its `as:`.
pub fn exposure_param(name: &str) -> String {
    let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    format!("__rpc_x_{safe}")
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
    surface: String,
    transport: Option<Expr>,
    name: String,
    bound: i64,
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
        let served_ctx = f.params.last().is_some_and(|p| hale_syntax::api_gen::is_served_context_type(&p.ty));
        let takes_ctx = served_ctx || f.params.last().is_some_and(|p| hale_syntax::api_gen::is_context_type(&p.ty));
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
    match shapes.classify(&te) {
        TypeClass::Struct { name, fields } => {
            if out.insert(name.to_string()) {
                for f in fields {
                    struct_closure(shapes, &f.ty, out, builtins);
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
            TypeClass::Prim(_) => format!("to_string({v})"),
            TypeClass::Struct { name, .. } | TypeClass::Builtin(name) => format!("__api_encode_{name}({v})"),
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
            TypeClass::Named { name, .. } if self.convs.contains_key(name) => {
                let c = &self.convs[name];
                let n = format!("__n_{var}");
                let ins = format!(
                    "if !__api_json_is_int(std::str::trim({body})) {{ {bail} }}\nlet {n} = std::str::parse_int(std::str::trim({body})) or {{ {bail} }};\n"
                );
                match c {
                    Conv::Range(_) => format!("{ins}let {var} = {name}({n}) or {{ {bail} }};\n"),
                    Conv::Identity(_) => format!("{ins}let {var} = {name}({n});\n"),
                    Conv::Quantity(m, u) => format!("{ins}let {var} = {n} * {m}{u};\n"),
                }
            }
            _ => format!("let {var} = {body};\n"),
        }
    }
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
];

/// Whether the programs serve a surface (`api::serve`) or spell a name of
/// the runtime (`std::api::Request`, `std::api::test::Rpc`, …): the
/// runtime joins a program only then.
fn mentions_runtime(programs: &[&mut Program]) -> bool {
    use hale_syntax::names::{for_each_spelled_in_item, Spelled};
    let mut found = false;
    for p in programs {
        for item in &p.items {
            let mut window: [&str; 3] = ["", "", ""];
            for_each_spelled_in_item(item, &mut |s| {
                if let Spelled::Name(n) = s {
                    window = [window[1], window[2], n];
                    if (window[1] == "api" && window[2] == "serve")
                        || (window[0] == "std" && window[1] == "api" && RUNTIME_NAMES.contains(&window[2]))
                    {
                        found = true;
                    }
                }
            });
            if found {
                return true;
            }
        }
    }
    false
}

/// Where the appended runtime parses: its own window of the generated
/// space, from here up to the topics'.
const RUNTIME_BASE: u32 = API_SYNTH_BASE + 0x0400_0000;

/// Whether an offset is in the appended runtime's source. The runtime is
/// stdlib source in every judgment (the analysis copy's rows are not the
/// program's own); this is how a pass that partitions program from stdlib
/// knows it.
pub fn is_runtime_pos(pos: u32) -> bool {
    (RUNTIME_BASE..API_SYNTH_BASE + 0x0500_0000).contains(&pos)
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

const HUB_TOPICS_SRC: &str = "topic __ApiHubEventT { payload: __StdApiHubEvent; subject: \"__api.hub.event\"; keyed_by key; }
topic __ApiHubUpT { payload: __StdApiHubUp; subject: \"__api.hub.up\"; keyed_by key; }
topic __ApiHubOutT { payload: __StdApiHubOut; subject: \"__api.hub.out\"; keyed_by key; }
";

/// Run the pass over a bundle's programs.
pub fn expand(
    programs: &mut [&mut Program],
    renames: &[(Vec<String>, String)],
    default_surface: &str,
) -> Vec<Expansion> {
    let hubs = crate::hub_expand::binds_a_hub(programs);
    if !mentions_runtime(programs) && !hubs {
        return Vec::new();
    }
    inject_runtime(programs);
    inject_topics(programs);
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
    }
    let mut plans: Vec<Plan> = Vec::new();
    let confer;
    let convs;
    let all_names: BTreeSet<String>;
    let all_builtins;
    {
        let ro: Vec<&Program> = programs.iter().map(|p| &**p).collect();
        let slices: Vec<&[TopDecl]> = ro.iter().map(|p| p.items.as_slice()).collect();
        let shapes = Shapes::of_all(&slices);
        confer = conferring(&ro);
        convs = hale_syntax::json_gen::scalar_convs(&ro);
        let mut names = BTreeSet::new();
        let mut builtins = BTreeMap::new();
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
                for site in crate::surfaces::locus_serve_sites(l) {
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
                            segs.first() == Some(&"std") || is_unix_rpc(path) || is_http_rpc(path) || is_mcp_rpc(path) || loci.contains_key(segs.join("::").as_str())
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
                    if l.members.iter().any(|m| {
                        matches!(m, LocusMember::Params(pb)
                            if pb.params.iter().any(|p| p.name.name == exposure_param(name)))
                    }) {
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
                        let bound_here = site.receivers().iter().find(|i| {
                            i.name.name == *ty || i.name.name.rsplit("::").next().is_some_and(|t| t == ty.as_str())
                        });
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
                    for r in &rows {
                        for te in r.request.iter().chain(r.response.iter()).chain(r.error.iter().filter(|_| !r.server_error)) {
                            struct_closure(&shapes, te, &mut names, &mut builtins);
                        }
                    }
                    plans.push(Plan {
                        site: Site {
                            program: pi,
                            serving: l.name.name.clone(),
                            surface,
                            transport: Some(fresh(transport)),
                            name: name.clone(),
                            bound: *bound,
                        },
                        rows,
                        digest,
                        slots,
                        fields,
                        principals,
                        roles,
                        expiring,
                        revised,
                    });
                }
            }
        }
        all_names = names;
        all_builtins = builtins;
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
    for (i, plan) in plans.iter().enumerate() {
        let id = i as i64 + 1;
        let site = &plan.site;
        generated.push_str(&surface_src(id, &plan.rows, &plan.slots, &confer, &codec, &schemas));
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
            });
        }
        out.push(Expansion {
            id,
            name: site.name.clone(),
            surface: site.surface.clone(),
            digest: plan.digest,
            serving: site.serving.clone(),
            param: exposure_param(&site.name),
        });
    }

    // ---- mutate: the serving loci ----
    for (i, plan) in plans.iter().enumerate() {
        let id = i as i64 + 1;
        let site = &plan.site;
        let exposure = out[i].clone();
        let program = &mut *programs[site.program];
        let Some(l) = find_locus_mut(&mut program.items, &site.serving) else { continue };
        let span = l.name.span;
        // the exposure param
        let src = format!(
            "main locus __Tmp {{ params {{ {param}: std::api::Exposure = std::api::Exposure {{ id: {id}, name: {name}, surface: {surface}, digest: {digest}, bound: {bound}, rows: __RpcSurface_{id} {{ }} }}; }} }}\n",
            param = exposure.param,
            id = id,
            name = q(&site.name),
            surface = q(&site.surface),
            digest = q(&hale_model::surface::digest_text(plan.digest)),
            bound = site.bound,
        );
        let Some(mut param) = parse_params(&src) else { continue };
        if let ParamInit::Value(Expr::Struct { inits, .. }) = &mut param.init {
            let mut push = |name: &str, value: Expr| {
                inits.push(StructInit { name: Ident::new(name, span), value, span });
            };
            if let Some(t) = &site.transport {
                push("transport", as_stdlib_unix(t, id));
                if let Some(addr) = http_bind_of(l, site.transport.as_ref()) {
                    push("address", addr);
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
        if let Some(LocusMember::Params(pb)) = l.members.iter_mut().find(|m| matches!(m, LocusMember::Params(_))) {
            pb.params.push(param);
        } else {
            l.members.push(LocusMember::Params(hale_syntax::ast::ParamsBlock { params: vec![param], span }));
        }
        // a Unix transport's listener: a param of the serving locus born
        // after the authored params its path may read, and before the
        // exposure (so it is subscribed when the exposure attaches), placed on a
        // pool of its own
        if l.is_main {
            let unix_path = unix_path_of(l, site.transport.as_ref());
            let http_bind = if unix_path.is_none() { http_bind_of(l, site.transport.as_ref()) } else { None };
            let listener = match (unix_path, http_bind) {
                (Some(path), _) => parse_unix_listener(id, path, span),
                (None, Some(bind)) => parse_http_listener(id, bind, is_mcp_site(l, site.transport.as_ref()), span),
                (None, None) => None,
            };
            {
                if let Some((lp, entry)) = listener {
                    if let Some(LocusMember::Params(pb)) = l.members.iter_mut().find(|m| matches!(m, LocusMember::Params(_))) {
                        // after every authored param (the address may read one),
                        // before the exposure just pushed
                        let at = pb.params.iter().position(|p| p.name.name == exposure_param(&site.name)).unwrap_or(pb.params.len());
                        pb.params.insert(at, lp);
                    }
                    if let Some(LocusMember::Placement(pl)) = l.members.iter_mut().find(|m| matches!(m, LocusMember::Placement(_))) {
                        pl.entries.push(entry);
                    } else {
                        l.members.push(LocusMember::Placement(hale_syntax::ast::PlacementBlock { entries: vec![entry], span }));
                    }
                }
            }
        }
        // a Unix transport the serving locus holds as a param is numbered
        // in the literal that builds it, as one written in the call is
        if let Some(f) = site.transport.as_ref().and_then(|t| crate::surfaces::self_field(t)) {
            for m in l.members.iter_mut() {
                if let LocusMember::Params(pb) = m {
                    for p in pb.params.iter_mut().filter(|p| p.name.name == f) {
                        if let ParamInit::Value(e) = &mut p.init {
                            if matches!(e, Expr::Struct { path, .. } if is_unix_rpc(path) || is_http_rpc(path) || is_mcp_rpc(path)) {
                                *e = as_stdlib_unix(e, id);
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
                extend_receiver(l, bs, &codec);
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

/// The transport literal as the exposure holds it: a socket transport is
/// spelled by its stdlib path and carries the number of the exposure it
/// serves (`tid`), which keys the bus subjects its connections share. The
/// HTTP transport's `codec: json` is the one codec v1 has, and is not a
/// field of the locus.
fn as_stdlib_unix(t: &Expr, id: i64) -> Expr {
    let mut e = fresh(t);
    if let Expr::Struct { path, inits, span, .. } = &mut e {
        let kind = if is_unix_rpc(path) {
            Some("unix")
        } else if is_http_rpc(path) {
            Some("http")
        } else if is_mcp_rpc(path) {
            Some("mcp")
        } else {
            None
        };
        if let Some(kind) = kind {
            let sp = *span;
            path.segments = ["std", "api", kind, "Rpc"].iter().map(|n| Ident::new(*n, sp)).collect();
            inits.retain(|i| {
                i.name.name != "tid" && !(kind == "http" && i.name.name == "codec" && matches!(&i.value, Expr::Ident(c) if c.name == "json"))
            });
            inits.push(StructInit { name: Ident::new("tid", sp), value: lit_int(id, sp), span: sp });
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
        Expr::Struct { path, inits, .. } if is_http_rpc(path) || is_mcp_rpc(path) => {
            inits.iter().find(|i| i.name.name == "bind").map(|i| fresh(&i.value))
        }
        _ => None,
    }
}

/// Whether the transport a serve site names is `mcp::Rpc`, written at the
/// site or held as a param of the serving locus.
fn is_mcp_site(l: &LocusDecl, transport: Option<&Expr>) -> bool {
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
    matches!(lit, Some(Expr::Struct { path, .. }) if is_mcp_rpc(path))
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
            s.push_str(&format!(
                "        if i == {i} {{\n            {}            return \"\";\n        }}\n",
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
        FieldSchema::Unformed => "{}".to_string(),
    }
}

fn type_json(t: &crate::surfaces::TypeSchema) -> String {
    let props: Vec<String> = t.properties.iter().map(|(k, f)| format!("{}:{}", jstr(k), field_json(f))).collect();
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
fn extend_receiver(l: &mut LocusDecl, bs: &[Binding], codec: &Codec<'_>) {
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
    let mut s = String::new();
    let bail = format!("self.__rpc_ev(c.exposure, c.rid, {slot}, 3, \"\"); return;");
    let mut args = String::new();
    if let Some(te) = &row.request {
        s.push_str(&codec.decode(te, "c.body", "__req", &bail));
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
            codec.encode(e, "err")
        ))
    };
    match (&row.response, fail_branch) {
        (Some(resp), Some(fb)) => {
            s.push_str(&format!("let __v = {call} or {{ {fb} }};\n"));
            s.push_str(&format!(
                "self.__rpc_ev(c.exposure, c.rid, {slot}, 0, {});\n",
                codec.encode(resp, "__v")
            ));
        }
        (Some(resp), None) => {
            s.push_str(&format!("let __v = {call};\n"));
            s.push_str(&format!(
                "self.__rpc_ev(c.exposure, c.rid, {slot}, 0, {});\n",
                codec.encode(resp, "__v")
            ));
        }
        (None, Some(fb)) => {
            s.push_str(&format!("{call} or {{ {fb} }};\n"));
            s.push_str(&format!("self.__rpc_ev(c.exposure, c.rid, {slot}, 0, \"null\");\n"));
        }
        (None, None) => {
            s.push_str(&format!("{call};\n"));
            s.push_str(&format!("self.__rpc_ev(c.exposure, c.rid, {slot}, 0, \"null\");\n"));
        }
    }
    s
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

fn walk_block_mut(b: &mut Block, f: &mut dyn FnMut(&mut Expr)) {
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
