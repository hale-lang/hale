//! The client binding (spec/api.md § Calling a surface): `api::client`
//! builds a client of a surface.
//!
//! A client is a param of the locus that holds it, recognized by its call
//! as an exposure is by `api::serve`:
//!
//! ```text
//! shelf: api::Client = api::client(Shelf, http::Client { base: "http://127.0.0.1:9000", principals: self.creds });
//! ```
//!
//! This pass, which runs in the desugar sequence before the check and
//! before the serve expansion (whose codec pass then gives the rows' types
//! their api codecs), writes, for each surface a well-formed site names:
//!
//! - a locus `__ApiClient_<Surface>` with one method per row, named as the
//!   row (`create_item`; `orders_place` for a Hale row `Orders::place`),
//!   taking the row's request and answering the enum
//!   `<Surface><Row>Outcome`: `Result(T)`, `HandlerError(E)` (a Hale row's
//!   own error type, an imported row's `std::api::Failure`), `Refusal`,
//!   `ServerError`, `Lost`. Each method carries the effect
//!   `client(<Surface>)`, so `forbid reaches(g, effects(client(Shelf)))`
//!   names its calls with nothing new in the claims;
//! - the effect family that class belongs to, over a domain of the
//!   surfaces the program calls: `domain __ApiClientSurfaces = { … };
//!   effect client(__ApiClientSurfaces);`, interned through the load's one
//!   class table so a claim the program wrote names the same class;
//! - the param, typed as that locus and built holding the transport
//!   instance as written (`http::Client` and `mcp::Client` are the
//!   runtime's `__StdApiHttpClient` and `__StdApiMcpClient`).
//!
//! The runtime the methods call (`api_client.hl`) is appended to the first
//! program, once. A site the pass cannot build is left as written, its
//! param typed as the transport interface, for [`client_laws`] to report.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::api_names::API_SYNTH_BASE;
use hale_syntax::ast::{
    flat_decls, Expr, Ident, LocusMember, ParamDecl, ParamInit, Program, QualifiedName, StructInit,
    TopDecl, TypeExpr,
};
use hale_syntax::{Diag, Span};

use crate::topic_identity::{Shapes, TypeClass};

/// Where the generated client loci parse.
pub const CLIENT_BASE: u32 = API_SYNTH_BASE + 0x0E00_0000;

/// Where the client runtime (`api_client.hl`) parses: a window of the
/// appended runtime's space, so it is stdlib source in every judgment.
pub const CLIENT_RUNTIME_BASE: u32 = crate::rpc_expand::RUNTIME_BASE + 0x0060_0000;

/// Whether an offset is in the client runtime's appended source
/// (`api_client.hl`): a `std::api` path to one of its loci
/// (`std::api::VaultPrincipals`) names the program's declaration.
pub fn is_client_runtime_pos(pos: u32) -> bool {
    (CLIENT_RUNTIME_BASE..CLIENT_RUNTIME_BASE + hale_stdlib::API_CLIENT_SOURCE.len() as u32)
        .contains(&pos)
}

/// Whether an offset is in the generated client loci.
pub fn is_client_pos(pos: u32) -> bool {
    (CLIENT_BASE..CLIENT_BASE + 0x0100_0000).contains(&pos)
}

/// The name of the locus a client of `surface` is.
pub fn client_locus(surface: &str) -> String {
    format!("__ApiClient_{surface}")
}

/// The interface a client's transport implements, and the type a site the
/// pass did not build is left with.
const TRANSPORT_IFACE: &str = "__StdApiClientTransport";

/// A row a client calls: what its method writes.
#[derive(Debug, Clone)]
struct ClientRow {
    /// The method's name.
    method: String,
    /// The outcome enum's name.
    outcome: String,
    member: String,
    /// An imported row's HTTP operation (`GET /items/{id}`), "" for a Hale
    /// row (or a row whose wire is no HTTP operation).
    wire: String,
    /// The MCP tool.
    tool: String,
    /// The request is no JSON object (a Hale row's enum), which no MCP
    /// tool's input can be.
    not_object: bool,
    /// A Hale row's member digest, "" for an imported row.
    digest: String,
    /// The roles the row requires, joined by `,`.
    roles: String,
    request: Option<Carry>,
    response: Option<Carry>,
    error: Failure,
    /// A signature row (of a surface the program consumes).
    signature: bool,
}

/// How a whole value crosses: by the type's api codec, or as free-form JSON.
#[derive(Debug, Clone)]
enum Carry {
    Codec(String),
    Json,
}

impl Carry {
    fn ty(&self) -> String {
        match self {
            Carry::Codec(t) => t.clone(),
            Carry::Json => "std::json::Value".to_string(),
        }
    }
}

/// What a row's handler error is to a caller.
#[derive(Debug, Clone)]
enum Failure {
    /// A Hale row that declares none, or `ClosureViolation`.
    None,
    /// A Hale row's own error type.
    Type(Carry),
    /// An imported row: `std::api::Failure`.
    Imported,
}

/// The two transports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Http,
    Mcp,
}

/// A lifecycle name a row's method may not take.
const RESERVED: &[&str] = &[
    "birth",
    "run",
    "drain",
    "dissolve",
    "accept",
    "release",
    "on_failure",
    "bulk",
    "harmonic",
    "resolution",
    "closure",
];

/// `e` as an `api::client(…)` call: its arguments.
pub fn client_call(e: &Expr) -> Option<&[Expr]> {
    let Expr::Call { callee, args, .. } = e else {
        return None;
    };
    let Expr::Path(qn) = callee.as_ref() else {
        return None;
    };
    let segs: Vec<&str> = qn.segments.iter().map(|s| s.name.as_str()).collect();
    (segs == ["api", "client"]).then_some(args.as_slice())
}

fn transport_kind(e: &Expr) -> Option<Kind> {
    let Expr::Struct { path, .. } = e else {
        return None;
    };
    let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
    match segs.as_slice() {
        ["http", "Client"] | ["std", "api", "http", "Client"] | ["__StdApiHttpClient"] => {
            Some(Kind::Http)
        }
        ["mcp", "Client"] | ["std", "api", "mcp", "Client"] | ["__StdApiMcpClient"] => {
            Some(Kind::Mcp)
        }
        _ => None,
    }
}

/// Whether a wire names an HTTP operation: `METHOD /path`.
fn http_operation(wire: &str) -> bool {
    match wire.split_once(' ') {
        Some((m, p)) => {
            !m.is_empty() && m.chars().all(|c| c.is_ascii_uppercase()) && p.starts_with('/')
        }
        None => false,
    }
}

fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// How a row's whole request, response or error crosses, or why it cannot.
fn carry(shapes: &Shapes<'_>, te: &TypeExpr, signature: bool) -> Result<Carry, String> {
    let d = shapes.dealias(te);
    if signature && hale_syntax::json_gen::is_json_value(&d) {
        return Ok(Carry::Json);
    }
    match shapes.classify(&d) {
        TypeClass::Struct { name, .. } | TypeClass::Enum { name, .. } => Ok(Carry::Codec(name.to_string())),
        _ => Err(format!(
            "`{}` is no record or enum, and a client carries a row's whole value by a record's or an enum's codec",
            crate::surfaces::type_text(te)
        )),
    }
}

/// The surface a site names, as declared.
fn surface_of(ro: &[&Program], renames: &[(Vec<String>, String)], e: &Expr) -> Option<String> {
    match e {
        Expr::Ident(i) => Some(i.name.clone()),
        Expr::Path(qn) => match qn.segments.as_slice() {
            [one] => Some(one.name.clone()),
            [alias, name] => {
                crate::rpc_expand::imported_surface(ro, renames, &alias.name, &name.name)
            }
            _ => None,
        },
        _ => None,
    }
}

fn surface_text(e: &Expr) -> String {
    match e {
        Expr::Ident(i) => i.name.clone(),
        Expr::Path(qn) => qn
            .segments
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join("::"),
        _ => "?".to_string(),
    }
}

/// The rows a client of `surface` calls, or why it cannot call them.
fn client_rows(
    ro: &[&Program],
    shapes: &Shapes<'_>,
    surface: &str,
    renames: &[(Vec<String>, String)],
    default_surface: &str,
) -> Result<Vec<ClientRow>, String> {
    let mut out: Vec<ClientRow> = Vec::new();
    let outcome = |method: &str| format!("{surface}{}Outcome", crate::api_names::camel(method));
    // a surface the program consumes: its signature rows, as written
    let mut consumed = false;
    for p in ro {
        for d in flat_decls(&p.items) {
            let TopDecl::Api(a) = d else { continue };
            if a.name.name != surface {
                continue;
            }
            for r in &a.rows {
                let Some(sig) = &r.signature else { continue };
                consumed = true;
                let name = r.method.name.clone();
                let what = |slot: &str, why: String| format!("row `{name}`'s {slot}: {why}");
                let request = match &sig.request {
                    Some(t) => Some(carry(shapes, t, true).map_err(|w| what("request", w))?),
                    None => None,
                };
                let response = match &sig.response {
                    None => None,
                    Some(TypeExpr::Tuple(parts, _)) if parts.is_empty() => None,
                    Some(t) => Some(carry(shapes, t, true).map_err(|w| what("response", w))?),
                };
                let wire = sig.wire.clone().unwrap_or_default();
                let (http, tool) = if http_operation(&wire) {
                    (wire.clone(), String::new())
                } else if wire.is_empty() {
                    (String::new(), name.clone())
                } else {
                    (String::new(), wire.clone())
                };
                out.push(ClientRow {
                    outcome: outcome(&name),
                    method: name.clone(),
                    member: name,
                    wire: http,
                    tool,
                    not_object: false,
                    digest: String::new(),
                    roles: r
                        .requires
                        .iter()
                        .map(|i| i.name.as_str())
                        .collect::<Vec<_>>()
                        .join(","),
                    request,
                    response,
                    error: Failure::Imported,
                    signature: true,
                });
            }
        }
    }
    if !consumed {
        let rows = crate::rpc_expand::rows_of_surface(ro, surface, renames, default_surface);
        let digests = crate::rpc_expand::member_digests(shapes, &rows);
        for (r, digest) in rows.iter().zip(digests) {
            let what = |slot: &str, why: String| format!("`{}`'s {slot}: {why}", r.member);
            let request = match &r.request {
                Some(t) => Some(carry(shapes, t, false).map_err(|w| what("request", w))?),
                None => None,
            };
            let response = match &r.response {
                Some(t) => Some(carry(shapes, t, false).map_err(|w| what("response", w))?),
                None => None,
            };
            let error = match &r.error {
                Some(_) if r.server_error => Failure::None,
                Some(t) => Failure::Type(carry(shapes, t, false).map_err(|w| what("error", w))?),
                None => Failure::None,
            };
            let method = crate::api_names::snake(&r.member);
            let not_object = r.request.as_ref().is_some_and(|t| {
                !matches!(
                    shapes.classify(&shapes.dealias(t)),
                    TypeClass::Struct { .. }
                )
            });
            out.push(ClientRow {
                outcome: outcome(&r.member),
                method,
                member: r.member.clone(),
                wire: String::new(),
                tool: crate::surface_doc::tool_name(&r.member),
                not_object,
                digest,
                roles: r.requires.join(","),
                request,
                response,
                error,
                signature: false,
            });
        }
    }
    if out.is_empty() {
        return Err(format!("`{surface}` has no row a client can call"));
    }
    if let Some(r) = out.iter().find(|r| RESERVED.contains(&r.method.as_str())) {
        return Err(format!(
            "row `{}` would be the client's method `{}`, a locus's lifecycle name; rename the row",
            r.member, r.method
        ));
    }
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    if let Some(r) = out.iter().find(|r| !seen.insert(r.method.as_str())) {
        return Err(format!(
            "two rows of `{surface}` would be the client's method `{}`",
            r.method
        ));
    }
    Ok(out)
}

/// What a site is, or why it is refused.
struct Plan {
    surface: String,
    kind: Kind,
    rows: Vec<ClientRow>,
}

fn plan(
    ro: &[&Program],
    shapes: &Shapes<'_>,
    renames: &[(Vec<String>, String)],
    default_surface: &str,
    param: &ParamDecl,
    args: &[Expr],
) -> Result<Plan, String> {
    let pname = &param.name.name;
    let [surface_e, transport] = args else {
        return Err(format!(
            "client `{pname}`: `api::client` takes the surface and the transport: `api::client(Surface, http::Client {{ base: \"http://…\", principals: self.creds }})`"
        ));
    };
    let written = surface_text(surface_e);
    let typed = match &param.ty {
        None => true,
        Some(TypeExpr::Named { path, .. }) => {
            let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
            matches!(
                segs.as_slice(),
                ["api", "Client"] | ["std", "api", "Client"] | [TRANSPORT_IFACE]
            )
        }
        Some(_) => false,
    };
    if !typed {
        return Err(format!("client `{pname}`: `api::client(…)` builds an `api::Client`; declare `{pname}: api::Client`"));
    }
    let Some(surface) = surface_of(ro, renames, surface_e) else {
        return Err(format!(
            "client of `{written}`: no surface `{written}` is declared"
        ));
    };
    let declared = ro
        .iter()
        .flat_map(|p| flat_decls(&p.items))
        .any(|d| matches!(d, TopDecl::Api(a) if a.name.name == surface))
        || !crate::rpc_expand::rows_of_surface(ro, &surface, renames, default_surface).is_empty();
    if !declared {
        return Err(format!(
            "client of `{written}`: no surface `{written}` is declared"
        ));
    }
    let Some(kind) = transport_kind(transport) else {
        return Err(format!(
            "client of `{written}`: the transport is a literal of `http::Client {{ base, principals }}` or `mcp::Client {{ url, principals }}`"
        ));
    };
    let rows = client_rows(ro, shapes, &surface, renames, default_surface)
        .map_err(|w| format!("client of `{written}`: {w}"))?;
    for r in &rows {
        match kind {
            Kind::Http if r.signature && r.wire.is_empty() => {
                return Err(format!(
                    "client of `{written}` over `http::Client`: row `{}` names no HTTP operation (`wire: \"GET /path\"`); a surface imported from MCP is called over `mcp::Client`",
                    r.member
                ));
            }
            Kind::Mcp if r.not_object => {
                return Err(format!(
                    "client of `{written}` over `mcp::Client`: `{}` takes a request that is no JSON object, and an MCP tool's input is one, so `mcp::Rpc` serves it as no tool; call it over `http::Client`",
                    r.member
                ));
            }
            Kind::Mcp if r.signature && !r.wire.is_empty() => {
                return Err(format!(
                    "client of `{written}` over `mcp::Client`: row `{}` is the HTTP operation `{}`; a surface imported from OpenAPI is called over `http::Client`",
                    r.member, r.wire
                ));
            }
            _ => {}
        }
    }
    Ok(Plan {
        surface,
        kind,
        rows,
    })
}

/// One row's method.
fn method_src(surface: &str, r: &ClientRow) -> String {
    let o = &r.outcome;
    let (param, payload) = match &r.request {
        Some(Carry::Codec(t)) => (format!("request: {t}"), format!("{t}::api_encode(request)")),
        Some(Carry::Json) => (
            "request: std::json::Value".to_string(),
            "request.text".to_string(),
        ),
        None => (
            String::new(),
            if r.signature {
                "\"\"".to_string()
            } else {
                "\"{}\"".to_string()
            },
        ),
    };
    let mut s = format!(
        "    @effects(is: {{ client({surface}) }})\n    fn {}({param}) -> {o} {{\n        let raw = self.__transport.call({}, {}, {}, {}, {}, {payload});\n",
        r.method,
        q(&r.member),
        q(&r.wire),
        q(&r.tool),
        q(&r.digest),
        q(&r.roles)
    );
    let decode = |c: &Carry, var: &str, part: &str| {
        match c {
        Carry::Codec(t) => format!(
            "            let {var} = {t}::api_decode(raw.body) or {{ return {o}::Lost(\"the {part} did not decode: \" + err.kind + \": \" + err.field); }};\n"
        ),
        Carry::Json => format!(
            "            let {var} = std::json::Value::parse(raw.body) or {{ return {o}::Lost(\"the {part} did not decode: \" + err.kind); }};\n"
        ),
    }
    };
    match &r.response {
        Some(c) => {
            s.push_str("        if raw.outcome == 0 {\n");
            s.push_str(&decode(c, "value", "result"));
            s.push_str(&format!(
                "            return {o}::Result(value);\n        }}\n"
            ));
        }
        None => s.push_str(&format!(
            "        if raw.outcome == 0 {{\n            return {o}::Result;\n        }}\n"
        )),
    }
    match &r.error {
        Failure::Type(c) => {
            s.push_str("        if raw.outcome == 1 {\n");
            s.push_str(&decode(c, "error", "handler error"));
            s.push_str(&format!("            return {o}::HandlerError(error);\n        }}\n"));
        }
        Failure::Imported => {
            s.push_str(&format!("        if raw.outcome == 1 {{\n            return {o}::HandlerError(__api_client_failure(raw));\n        }}\n"));
        }
        Failure::None => s.push_str(&format!(
            "        if raw.outcome == 1 {{\n            return {o}::Lost(\"the member has no handler error, and one came: \" + raw.body);\n        }}\n"
        )),
    }
    s.push_str(&format!("        if raw.outcome == 2 {{\n            return {o}::Refusal(__api_client_refusal(raw));\n        }}\n"));
    s.push_str(&format!(
        "        if raw.outcome == 3 {{\n            return {o}::ServerError;\n        }}\n"
    ));
    s.push_str(&format!("        return {o}::Lost(raw.why);\n    }}\n"));
    s
}

/// The outcome enum of one row.
fn outcome_src(r: &ClientRow) -> String {
    let mut variants = vec![match &r.response {
        Some(c) => format!("Result({})", c.ty()),
        None => "Result".to_string(),
    }];
    match &r.error {
        Failure::Type(c) => variants.push(format!("HandlerError({})", c.ty())),
        Failure::Imported => variants.push("HandlerError(std::api::Failure)".to_string()),
        Failure::None => {}
    }
    variants.push("Refusal(std::api::Refusal)".to_string());
    variants.push("ServerError".to_string());
    variants.push("Lost(String)".to_string());
    format!("type {} = enum {{ {} }};\n", r.outcome, variants.join(", "))
}

/// Whether any program spells an `api::client(…)` param.
fn client_params(programs: &[&mut Program]) -> bool {
    programs.iter().any(|p| {
        flat_decls(&p.items).any(|d| match d {
            TopDecl::Locus(l) => l.members.iter().any(|m| match m {
                LocusMember::Params(pb) => pb
                    .params
                    .iter()
                    .any(|p| matches!(&p.init, ParamInit::Value(e) if client_call(e).is_some())),
                _ => false,
            }),
            _ => false,
        })
    })
}

/// Run the pass over a bundle's programs.
pub fn expand(
    programs: &mut [&mut Program],
    renames: &[(Vec<String>, String)],
    default_surface: &str,
) {
    if programs.is_empty() || !client_params(programs) {
        return;
    }
    inject_runtime(programs);
    // ---- the immutable phase: each site's plan ----
    // (program, locus, param) -> surface and transport kind
    let mut built: BTreeMap<(usize, String, String), (String, Kind)> = BTreeMap::new();
    let mut surfaces: BTreeMap<String, Vec<ClientRow>> = BTreeMap::new();
    {
        let ro: Vec<&Program> = programs.iter().map(|p| &**p).collect();
        let slices: Vec<&[TopDecl]> = ro.iter().map(|p| p.items.as_slice()).collect();
        let shapes = Shapes::of_all(&slices);
        for (pi, p) in ro.iter().enumerate() {
            for d in flat_decls(&p.items) {
                let TopDecl::Locus(l) = d else { continue };
                for m in &l.members {
                    let LocusMember::Params(pb) = m else { continue };
                    for param in &pb.params {
                        let ParamInit::Value(e) = &param.init else {
                            continue;
                        };
                        let Some(args) = client_call(e) else { continue };
                        if let Ok(plan) = plan(&ro, &shapes, renames, default_surface, param, args)
                        {
                            built.insert(
                                (pi, l.name.name.clone(), param.name.name.clone()),
                                (plan.surface.clone(), plan.kind),
                            );
                            surfaces.entry(plan.surface).or_insert(plan.rows);
                        }
                    }
                }
            }
        }
    }
    // ---- the generated source: the family, the outcomes, the loci ----
    let have: BTreeSet<String> = programs
        .iter()
        .flat_map(|p| flat_decls(&p.items))
        .filter_map(|d| match d {
            TopDecl::Locus(l) => Some(l.name.name.clone()),
            _ => None,
        })
        .collect();
    let fresh: Vec<(&String, &Vec<ClientRow>)> = surfaces
        .iter()
        .filter(|(s, _)| !have.contains(&client_locus(s)))
        .collect();
    if !fresh.is_empty() {
        let mut src = format!(
            "domain __ApiClientSurfaces = {{ {} }};\neffect client(__ApiClientSurfaces);\n\n",
            surfaces.keys().cloned().collect::<Vec<_>>().join(", ")
        );
        for (surface, rows) in &fresh {
            for r in rows.iter() {
                src.push_str(&outcome_src(r));
            }
            src.push_str(&format!(
                "\nlocus {} {{\n    params {{\n        __transport: {TRANSPORT_IFACE} = __StdApiNoClient {{ }};\n    }}\n",
                client_locus(surface)
            ));
            for r in rows.iter() {
                src.push_str(&method_src(surface, r));
            }
            src.push_str("}\n\n");
        }
        // through the load's one class table: `client(<Surface>)` is the
        // index a claim the program wrote interned for it
        match crate::effect_classes::parse_generated(programs, &src, CLIENT_BASE) {
            Ok(generated) => programs[0].items.extend(generated.items),
            Err(ds) => {
                eprintln!(
                    "client_expand: the generated client did not parse: {}",
                    ds.iter()
                        .map(|d| d.message.clone())
                        .collect::<Vec<_>>()
                        .join("; ")
                );
                return;
            }
        }
    }
    // ---- the params: built as the client locus, or left for the law ----
    for (pi, p) in programs.iter_mut().enumerate() {
        hale_syntax::ast::for_each_decl_mut(&mut p.items, &mut |d| {
            let TopDecl::Locus(l) = d else { return };
            let locus = l.name.name.clone();
            for m in &mut l.members {
                let LocusMember::Params(pb) = m else { continue };
                for param in &mut pb.params {
                    let ParamInit::Value(e) = &param.init else {
                        continue;
                    };
                    let Some(args) = client_call(e) else { continue };
                    let sp = param.span;
                    match built.get(&(pi, locus.clone(), param.name.name.clone())) {
                        Some((surface, kind)) => {
                            let mut transport = args[1].clone();
                            if let Expr::Struct { path, .. } = &mut transport {
                                let name = match kind {
                                    Kind::Http => "__StdApiHttpClient",
                                    Kind::Mcp => "__StdApiMcpClient",
                                };
                                *path = QualifiedName {
                                    segments: vec![Ident::new(name, path.span)],
                                    span: path.span,
                                };
                            }
                            let name = client_locus(surface);
                            param.ty = Some(named(&name, sp));
                            param.init = ParamInit::Value(Expr::Struct {
                                path: QualifiedName {
                                    segments: vec![Ident::new(name, sp)],
                                    span: sp,
                                },
                                inits: vec![StructInit {
                                    name: Ident::new("__transport", sp),
                                    value: transport,
                                    span: sp,
                                }],
                                span: sp,
                                id: hale_syntax::ast::NodeId::NONE,
                            });
                        }
                        None => {
                            // typed as what the runtime declares, so the
                            // law's refusal is the one diagnostic
                            if matches!(&param.ty, Some(TypeExpr::Named { path, .. }) if path.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>() == ["api", "Client"])
                            {
                                param.ty = Some(named(TRANSPORT_IFACE, sp));
                            }
                        }
                    }
                }
            }
        });
    }
}

fn named(name: &str, sp: Span) -> TypeExpr {
    TypeExpr::Named {
        path: QualifiedName {
            segments: vec![Ident::new(name, sp)],
            span: sp,
        },
        generic_args: Vec::new(),
        span: sp,
    }
}

/// Append the client runtime to the first program, once.
fn inject_runtime(programs: &mut [&mut Program]) {
    let have = programs.iter().any(|p| {
        flat_decls(&p.items)
            .any(|d| matches!(d, TopDecl::Locus(l) if l.name.name == "__StdApiHttpClient"))
    });
    if have {
        return;
    }
    match hale_syntax::parse_source_at(hale_stdlib::API_CLIENT_SOURCE, CLIENT_RUNTIME_BASE) {
        Ok(rt) => {
            for mut item in rt.items {
                if let TopDecl::Type(t) = &mut item {
                    t.synthetic = true;
                }
                programs[0].items.push(item);
            }
        }
        Err(ds) => eprintln!(
            "client_expand: the client runtime did not parse: {}",
            ds.iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("; ")
        ),
    }
}

/// The client sites the pass left as written, each refused with why, and
/// every `api::client` call anywhere else: a client is a param's literal
/// and nothing else (spec/api.md § Calling a surface).
pub fn client_laws(
    programs: &[&Program],
    renames: &[(Vec<String>, String)],
    default_surface: &str,
) -> Vec<Diag> {
    let slices: Vec<&[TopDecl]> = programs.iter().map(|p| p.items.as_slice()).collect();
    let shapes = Shapes::of_all(&slices);
    let mut diags = Vec::new();
    // the callee of each call a param's literal is, by where it starts
    let mut sites: BTreeSet<u32> = BTreeSet::new();
    for p in programs {
        for d in flat_decls(&p.items) {
            let TopDecl::Locus(l) = d else { continue };
            for m in &l.members {
                let LocusMember::Params(pb) = m else { continue };
                for param in &pb.params {
                    let ParamInit::Value(e) = &param.init else {
                        continue;
                    };
                    let Some(args) = client_call(e) else { continue };
                    if let Expr::Call { callee, .. } = e {
                        if let Expr::Path(qn) = callee.as_ref() {
                            if let Some(first) = qn.segments.first() {
                                sites.insert(first.span.start.0);
                            }
                        }
                    }
                    let why = match plan(programs, &shapes, renames, default_surface, param, args) {
                        Err(why) => why,
                        Ok(plan) => {
                            format!("client of `{}`: the client was not built", plan.surface)
                        }
                    };
                    diags.push(Diag::ty(
                        e_span(e),
                        format!("{why} (spec/api.md § Calling a surface)"),
                    ));
                }
            }
        }
    }
    for p in programs {
        hale_syntax::names::for_each_spelled(&p.items, &mut |s| {
            let hale_syntax::names::Spelled::Path(segs) = s else {
                return;
            };
            let [a, c] = segs else { return };
            if a.name != "api" || c.name != "client" || sites.contains(&a.span.start.0) {
                return;
            }
            let span = Span {
                start: a.span.start,
                end: c.span.end,
            };
            diags.push(Diag::ty(
                span,
                "`api::client(…)` builds a client, which is a param of the locus that calls it: write \
                 `NAME: api::Client = api::client(Surface, http::Client { … });` in its params and call \
                 `self.NAME`'s methods (spec/api.md § Calling a surface)"
                    .to_string(),
            ));
        });
    }
    diags
}

fn e_span(e: &Expr) -> Span {
    match e {
        Expr::Call { span, .. } => *span,
        _ => Span::new(0, 0),
    }
}
