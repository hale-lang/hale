//! JSON Tier 2: compiler-generated, schema-specialized parsers.
//!
//! A struct that carries at least one `json:"<key>"` field tag opts into a
//! generated `Type::from_json(s) -> Type fallible(JsonError)`. For each such
//! type we synthesize a `__json_parse_<Type>` function that drives the
//! single-pass `std::json` object cursor (see `runtime/stdlib/json.hl`),
//! dispatching each key (length-gated) and reading the value by the field's
//! declared type — one pass, no per-field rescan, unknown keys skipped.
//! `Type::from_json(s)` calls are rewritten to the generated function.
//!
//! Semantics (v1, scalar fields):
//!   - key = the `json:` tag value, else the field name.
//!   - a missing field raises `JsonError { kind: "missing_field", field }`,
//!     UNLESS the field declares a literal `= default`, in which case the
//!     default is kept (optional-with-default for free).
//!   - field types: Int / Float / Bool / String. A type with any non-scalar
//!     field is left ungenerated (nested types + arrays are a follow-up).
//!
//! The pass runs BEFORE typecheck (so the generated parser is itself
//! checked and `from_json` resolves to a real fallible function the caller
//! must address) and is idempotent.

use std::collections::HashSet;

use crate::ast::{
    Block, ElseBranch, Expr, Ident, IfStmt, Literal, LocusMember, MatchArmBody,
    MatchStmt, OrDisposition, PrimType, Program, Stmt, TopDecl, TypeDeclBody,
    TypeExpr,
};
use crate::parse_source;

#[derive(Clone, Copy)]
enum ScalarTy {
    Int,
    Float,
    Bool,
    Str,
}

impl ScalarTy {
    fn type_name(self) -> &'static str {
        match self {
            ScalarTy::Int => "Int",
            ScalarTy::Float => "Float",
            ScalarTy::Bool => "Bool",
            ScalarTy::Str => "String",
        }
    }
    fn zero(self) -> &'static str {
        match self {
            ScalarTy::Int => "0",
            ScalarTy::Float => "0.0",
            ScalarTy::Bool => "false",
            ScalarTy::Str => "\"\"",
        }
    }
    fn reader(self) -> &'static str {
        match self {
            ScalarTy::Int => "obj_value_int",
            ScalarTy::Float => "obj_value_float",
            ScalarTy::Bool => "obj_value_bool",
            ScalarTy::Str => "obj_value_string",
        }
    }
}

enum FieldKind {
    Scalar(ScalarTy),
    /// A field whose type is another generated JSON struct — parsed by
    /// recursing into the nested object's raw text.
    Nested(String),
}

struct JsonField {
    name: String,           // struct field name
    key: String,            // JSON key (tag value or field name)
    kind: FieldKind,
    default_src: Option<String>, // scalar literal default; None for nested/required
}

struct JsonType {
    name: String,
    fields: Vec<JsonField>,
}

fn scalar_of(te: &TypeExpr) -> Option<ScalarTy> {
    match te {
        TypeExpr::Primitive(PrimType::Int, _) => Some(ScalarTy::Int),
        TypeExpr::Primitive(PrimType::Float, _) => Some(ScalarTy::Float),
        TypeExpr::Primitive(PrimType::Bool, _) => Some(ScalarTy::Bool),
        TypeExpr::Primitive(PrimType::String, _) => Some(ScalarTy::Str),
        _ => None,
    }
}

/// Render a literal default expression to Hale source. Only literals are
/// supported (covers `= 0`, `= 1.5`, `= true`, `= "USD"`); anything else
/// disqualifies the field's default (treated as required).
fn literal_src(e: &Expr) -> Option<String> {
    match e {
        Expr::Literal(Literal::Int(n), _) => Some(n.to_string()),
        Expr::Literal(Literal::Bool(b), _) => Some(b.to_string()),
        Expr::Literal(Literal::Float(f), _) => Some(format!("{:?}", f)),
        Expr::Literal(Literal::String(s), _) => {
            Some(format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
        }
        _ => None,
    }
}

fn tag_json_key(tag: &Option<String>) -> Option<String> {
    tag.as_deref().and_then(|t| crate::desugar::tag_value(t, "json"))
}

/// A single-segment named type (`Inner`, not `mod::Inner` or `Box<T>`).
fn named_single(te: &TypeExpr) -> Option<&str> {
    match te {
        TypeExpr::Named { path, generic_args, .. }
            if generic_args.is_empty() && path.segments.len() == 1 =>
        {
            Some(&path.segments[0].name)
        }
        _ => None,
    }
}

/// Names of all structs that opt into JSON parsing (≥1 `json:` tag).
fn collect_json_type_names(items: &[TopDecl], out: &mut HashSet<String>) {
    for item in items {
        match item {
            TopDecl::Type(td) => {
                if let TypeDeclBody::Struct(fields) = &td.body {
                    if fields.iter().any(|f| tag_json_key(&f.tag).is_some()) {
                        out.insert(td.name.name.clone());
                    }
                }
            }
            TopDecl::Module(m) => collect_json_type_names(&m.items, out),
            _ => {}
        }
    }
}

/// Build the field schema for every opted-in struct. Each field must be a
/// scalar (Int/Float/Bool/String) or a nested field whose type is itself a
/// generated JSON struct (in `names`); a field of any other type leaves
/// the whole type ungenerated (arrays + non-JSON structs are future work).
fn collect_json_types(items: &[TopDecl], names: &HashSet<String>, out: &mut Vec<JsonType>) {
    for item in items {
        match item {
            TopDecl::Type(td) => {
                if let TypeDeclBody::Struct(fields) = &td.body {
                    if !fields.iter().any(|f| tag_json_key(&f.tag).is_some()) {
                        continue;
                    }
                    let mut jfields = Vec::new();
                    let mut ok = true;
                    for f in fields {
                        let kind = if let Some(s) = scalar_of(&f.ty) {
                            FieldKind::Scalar(s)
                        } else if let Some(tn) = named_single(&f.ty) {
                            if names.contains(tn) {
                                FieldKind::Nested(tn.to_string())
                            } else {
                                ok = false;
                                break;
                            }
                        } else {
                            ok = false;
                            break;
                        };
                        let default_src = match kind {
                            FieldKind::Scalar(_) => f.default.as_ref().and_then(literal_src),
                            FieldKind::Nested(_) => None,
                        };
                        jfields.push(JsonField {
                            name: f.name.name.clone(),
                            key: tag_json_key(&f.tag).unwrap_or_else(|| f.name.name.clone()),
                            kind,
                            default_src,
                        });
                    }
                    if ok && !jfields.is_empty() {
                        out.push(JsonType { name: td.name.name.clone(), fields: jfields });
                    }
                }
            }
            TopDecl::Module(m) => collect_json_types(&m.items, names, out),
            _ => {}
        }
    }
}

fn generate_parser_src(t: &JsonType) -> String {
    let mut b = String::new();
    b.push_str(&format!(
        "fn __json_parse_{}(__s: String) -> {} fallible(JsonError) {{\n",
        t.name, t.name
    ));
    // Locals. Scalars accumulate the parsed value; nested fields keep the
    // raw object text and are parsed at construction (no zero-value for a
    // struct local). A presence flag is needed unless a scalar default
    // covers a missing key.
    for f in &t.fields {
        match &f.kind {
            FieldKind::Scalar(s) => {
                let init = f.default_src.as_deref().unwrap_or_else(|| s.zero());
                b.push_str(&format!(
                    "    let mut __f_{}: {} = {};\n",
                    f.name,
                    s.type_name(),
                    init
                ));
            }
            FieldKind::Nested(_) => {
                b.push_str(&format!("    let mut __raw_{}: String = \"\";\n", f.name));
            }
        }
        if f.default_src.is_none() {
            b.push_str(&format!("    let mut __seen_{}: Bool = false;\n", f.name));
        }
    }
    b.push_str("    let mut __it = std::json::object_first(__s);\n");
    b.push_str("    while !__it.done {\n");
    let mut first = true;
    for f in &t.fields {
        let kw = if first { "if" } else { "} else if" };
        first = false;
        b.push_str(&format!(
            "        {} std::json::obj_key_len(__it) == {} && std::json::obj_key_eq(__it, __s, \"{}\") {{\n",
            kw,
            f.key.len(),
            f.key
        ));
        match &f.kind {
            FieldKind::Scalar(s) => b.push_str(&format!(
                "            __f_{} = std::json::{}(__it, __s);\n",
                f.name,
                s.reader()
            )),
            FieldKind::Nested(_) => b.push_str(&format!(
                "            __raw_{} = std::json::obj_value_raw(__it, __s);\n",
                f.name
            )),
        }
        if f.default_src.is_none() {
            b.push_str(&format!("            __seen_{} = true;\n", f.name));
        }
    }
    if !first {
        b.push_str("        }\n");
    }
    b.push_str("        __it = std::json::object_next(__it, __s);\n");
    b.push_str("    }\n");
    for f in &t.fields {
        if f.default_src.is_none() {
            b.push_str(&format!(
                "    if !__seen_{} {{ fail JsonError {{ kind: \"missing_field\", field: \"{}\" }}; }}\n",
                f.name, f.key
            ));
        }
    }
    // Recurse into nested fields before construction so their JsonError
    // propagates via `or raise`.
    for f in &t.fields {
        if let FieldKind::Nested(tn) = &f.kind {
            b.push_str(&format!(
                "    let __p_{} = __json_parse_{}(__raw_{}) or raise;\n",
                f.name, tn, f.name
            ));
        }
    }
    let inits: Vec<String> = t
        .fields
        .iter()
        .map(|f| match f.kind {
            FieldKind::Scalar(_) => format!("{}: __f_{}", f.name, f.name),
            FieldKind::Nested(_) => format!("{}: __p_{}", f.name, f.name),
        })
        .collect();
    b.push_str(&format!("    return {} {{ {} }};\n}}\n", t.name, inits.join(", ")));
    b
}

/// Synthesize `__json_parse_<T>` + `JsonError`, inject them, then rewrite
/// every `T::from_json(s)` call to `__json_parse_T(s)`.
pub fn generate_json_parsers(program: &mut Program) {
    let mut names = HashSet::new();
    collect_json_type_names(&program.items, &mut names);
    let mut types = Vec::new();
    collect_json_types(&program.items, &names, &mut types);
    if types.is_empty() {
        return;
    }

    let existing_fns: HashSet<String> = program
        .items
        .iter()
        .filter_map(|i| match i {
            TopDecl::Fn(f) => Some(f.name.name.clone()),
            _ => None,
        })
        .collect();
    let have_jsonerror = program.items.iter().any(
        |i| matches!(i, TopDecl::Type(t) if t.name.name == "JsonError"),
    );

    let mut src = String::new();
    if !have_jsonerror {
        src.push_str("type JsonError { kind: String; field: String; }\n");
    }
    for t in &types {
        if !existing_fns.contains(&format!("__json_parse_{}", t.name)) {
            src.push_str(&generate_parser_src(t));
        }
    }
    if !src.trim().is_empty() {
        match parse_source(&src) {
            Ok(generated) => program.items.extend(generated.items),
            // The generator emits well-formed source; a parse failure is a
            // generator bug, not user error. Leave the program unchanged so
            // the (un-rewritten) call surfaces a normal diagnostic.
            Err(_) => return,
        }
    }

    let names: HashSet<String> = types.iter().map(|t| t.name.clone()).collect();
    rewrite_items(&mut program.items, &names);
}

// ---- `T::from_json(s)` -> `__json_parse_T(s)` rewrite (full expr walk) ----

fn rewrite_items(items: &mut [TopDecl], names: &HashSet<String>) {
    for item in items {
        match item {
            TopDecl::Fn(f) => rewrite_block(&mut f.body, names),
            TopDecl::Locus(l) => {
                for m in &mut l.members {
                    match m {
                        LocusMember::Lifecycle(lc) => rewrite_block(&mut lc.body, names),
                        LocusMember::Mode(md) => rewrite_block(&mut md.body, names),
                        LocusMember::Fn(fd) => rewrite_block(&mut fd.body, names),
                        _ => {}
                    }
                }
            }
            TopDecl::Module(m) => rewrite_items(&mut m.items, names),
            _ => {}
        }
    }
}

fn rewrite_block(b: &mut Block, names: &HashSet<String>) {
    for s in &mut b.stmts {
        rewrite_stmt(s, names);
    }
    if let Some(t) = &mut b.tail {
        rewrite_expr(t, names);
    }
}

fn rewrite_stmt(s: &mut Stmt, names: &HashSet<String>) {
    match s {
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => rewrite_expr(value, names),
        Stmt::Assign { target, value, .. } => {
            rewrite_expr(value, names);
            for seg in &mut target.tail {
                if let crate::ast::LValueSeg::Index(e) = seg {
                    rewrite_expr(e, names);
                }
            }
        }
        Stmt::If(if_stmt) => rewrite_if(if_stmt, names),
        Stmt::Match(m) => rewrite_match(m, names),
        Stmt::For { iter, body, .. } => {
            rewrite_expr(iter, names);
            rewrite_block(body, names);
        }
        Stmt::While { cond, body, .. } => {
            rewrite_expr(cond, names);
            rewrite_block(body, names);
        }
        Stmt::Return(Some(e), _) => rewrite_expr(e, names),
        Stmt::Fail { value, .. } => rewrite_expr(value, names),
        Stmt::Send { subject, value, .. } => {
            rewrite_expr(subject, names);
            rewrite_expr(value, names);
        }
        Stmt::Block(b) => rewrite_block(b, names),
        Stmt::Recovery { args, .. } => {
            for a in args {
                rewrite_expr(a, names);
            }
        }
        Stmt::Violate { payload, .. } => {
            if let Some(p) = payload {
                rewrite_expr(p, names);
            }
        }
        Stmt::ShmWrite { max, body, .. } => {
            rewrite_expr(max, names);
            rewrite_block(body, names);
        }
        Stmt::Expr(e) => rewrite_expr(e, names),
        Stmt::Return(None, _)
        | Stmt::Break(_)
        | Stmt::Continue(_)
        | Stmt::Yield(_)
        | Stmt::Terminate(_) => {}
    }
}

fn rewrite_if(if_stmt: &mut IfStmt, names: &HashSet<String>) {
    rewrite_expr(&mut if_stmt.cond, names);
    rewrite_block(&mut if_stmt.then_block, names);
    if let Some(eb) = &mut if_stmt.else_block {
        match eb.as_mut() {
            ElseBranch::Else(b) => rewrite_block(b, names),
            ElseBranch::ElseIf(inner) => rewrite_if(inner, names),
        }
    }
}

fn rewrite_match(m: &mut MatchStmt, names: &HashSet<String>) {
    rewrite_expr(&mut m.scrutinee, names);
    for arm in &mut m.arms {
        match &mut arm.body {
            MatchArmBody::Block(b) => rewrite_block(b, names),
            MatchArmBody::Expr(e) => rewrite_expr(e, names),
        }
    }
}

fn rewrite_expr(e: &mut Expr, names: &HashSet<String>) {
    match e {
        Expr::Call { callee, args, span } => {
            rewrite_expr(callee, names);
            for a in args.iter_mut() {
                rewrite_expr(a, names);
            }
            // `T::from_json(s)` with T a generated type -> `__json_parse_T(s)`.
            if args.len() == 1 {
                if let Expr::Path(qn) = callee.as_ref() {
                    if qn.segments.len() == 2
                        && qn.segments[1].name == "from_json"
                        && names.contains(&qn.segments[0].name)
                    {
                        let fname = format!("__json_parse_{}", qn.segments[0].name);
                        *callee = Box::new(Expr::Ident(Ident {
                            name: fname,
                            span: *span,
                        }));
                    }
                }
            }
        }
        Expr::Binary { left, right, .. } => {
            rewrite_expr(left, names);
            rewrite_expr(right, names);
        }
        Expr::Unary { operand, .. } => rewrite_expr(operand, names),
        Expr::Field { receiver, .. } => rewrite_expr(receiver, names),
        Expr::Index { receiver, index, .. } => {
            rewrite_expr(receiver, names);
            rewrite_expr(index, names);
        }
        Expr::Path2 { receiver, .. } => rewrite_expr(receiver, names),
        Expr::Tuple(es, _) | Expr::Array(es, _) => {
            for x in es {
                rewrite_expr(x, names);
            }
        }
        Expr::Struct { inits, .. } => {
            for i in inits {
                rewrite_expr(&mut i.value, names);
            }
        }
        Expr::Block(b) => rewrite_block(b, names),
        Expr::If(if_stmt) => rewrite_if(if_stmt, names),
        Expr::Match(m) => rewrite_match(m, names),
        Expr::Sum(inner, _) | Expr::Prod(inner, _) => rewrite_expr(inner, names),
        Expr::Approx { left, right, tolerance, .. } => {
            rewrite_expr(left, names);
            rewrite_expr(right, names);
            rewrite_expr(tolerance, names);
        }
        Expr::Range { lo, hi, .. } => {
            rewrite_expr(lo, names);
            rewrite_expr(hi, names);
        }
        Expr::ArrayRepeat { val, .. } => rewrite_expr(val, names),
        Expr::Or { inner, disposition, .. } => {
            rewrite_expr(inner, names);
            if let OrDisposition::Substitute(s) = disposition {
                rewrite_expr(s, names);
            }
        }
        Expr::Literal(..) | Expr::Ident(_) | Expr::Path(_) | Expr::KwSelf(_) => {}
    }
}
