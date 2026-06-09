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

/// The Hale expression that parses a scalar value span `[__vs, __ve)` into
/// the field's type. Strings slice + unescape (the one real allocation);
/// ints/bools read the range allocation-free.
fn scalar_value_expr(s: ScalarTy) -> &'static str {
    match s {
        ScalarTy::Int => "std::str::range_parse_int(__s, __vs, __ve) or 0",
        ScalarTy::Float => "std::str::parse_float(__s[__vs..__ve]) or 0.0",
        ScalarTy::Bool => "std::str::range_eq(__s, __vs, __ve, \"true\")",
        ScalarTy::Str => "std::json::unescape_string(__s[(__vs + 1)..(__ve - 1)])",
    }
}

fn generate_parser_src(t: &JsonType) -> String {
    let mut b = String::new();
    b.push_str(&format!(
        "fn __json_parse_{}(__s: String) -> {} fallible(JsonError) {{\n",
        t.name, t.name
    ));
    b.push_str("    let __total = len(__s);\n");
    // Field accumulators + presence flags. Scalars hold the parsed value;
    // nested fields hold the matched object's raw text (parsed at
    // construction). A presence flag is omitted when a scalar default
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
    // Inlined single-pass scan — no cursor structs, only Int offsets +
    // the leaf SIMD/range primitives. Skip to `{`, then walk members.
    b.push_str("    let mut __p = std::json::next_non_ws(__s, 0, __total);\n");
    b.push_str("    if __p < __total && std::str::byte_at_unchecked(__s, __p) == 123 { __p = __p + 1; }\n");
    b.push_str("    let mut __more = true;\n");
    b.push_str("    while __more {\n");
    b.push_str("        __p = std::json::next_non_ws(__s, __p, __total);\n");
    b.push_str("        while __p < __total && std::str::byte_at_unchecked(__s, __p) == 44 {\n");
    b.push_str("            __p = std::json::next_non_ws(__s, __p + 1, __total);\n");
    b.push_str("        }\n");
    b.push_str("        if __p >= __total || std::str::byte_at_unchecked(__s, __p) != 34 {\n");
    b.push_str("            __more = false;\n");
    b.push_str("        } else {\n");
    // Key span [__ks, __ke).
    b.push_str("            let __ks = __p + 1;\n");
    b.push_str("            let mut __ke = std::json::next_quote_or_bs(__s, __ks, __total);\n");
    b.push_str("            while __ke < __total && std::str::byte_at_unchecked(__s, __ke) == 92 {\n");
    b.push_str("                __ke = std::json::next_quote_or_bs(__s, __ke + 2, __total);\n");
    b.push_str("            }\n");
    // Whitespace, `:`, whitespace.
    b.push_str("            let mut __q = std::json::next_non_ws(__s, __ke + 1, __total);\n");
    b.push_str("            if __q >= __total || std::str::byte_at_unchecked(__s, __q) != 58 {\n");
    b.push_str("                __more = false;\n");
    b.push_str("            } else {\n");
    b.push_str("                __q = std::json::next_non_ws(__s, __q + 1, __total);\n");
    b.push_str("                let __vs = __q;\n");
    // Track whether a string value contained a backslash escape — lets a
    // String field skip the unescape copy entirely in the common
    // escape-free case (it falls out of the value scan for free).
    b.push_str("                let mut __esc = false;\n");
    // Value end: jump structural-to-structural, skipping strings + nesting.
    b.push_str("                let mut __depth = 0;\n");
    b.push_str("                let mut __vscan = true;\n");
    b.push_str("                while __vscan {\n");
    b.push_str("                    __q = std::json::next_struct_or_quote(__s, __q, __total);\n");
    b.push_str("                    if __q >= __total { __q = __total; __vscan = false; }\n");
    b.push_str("                    else {\n");
    b.push_str("                        let __c = std::str::byte_at_unchecked(__s, __q);\n");
    b.push_str("                        if __c == 34 {\n");
    b.push_str("                            let mut __sp = std::json::next_quote_or_bs(__s, __q + 1, __total);\n");
    b.push_str("                            while __sp < __total && std::str::byte_at_unchecked(__s, __sp) == 92 {\n");
    b.push_str("                                __esc = true;\n");
    b.push_str("                                __sp = std::json::next_quote_or_bs(__s, __sp + 2, __total);\n");
    b.push_str("                            }\n");
    b.push_str("                            if __sp >= __total { __q = __total; __vscan = false; } else { __q = __sp + 1; }\n");
    b.push_str("                        } else if __c == 123 || __c == 91 { __depth = __depth + 1; __q = __q + 1; }\n");
    b.push_str("                        else if __c == 125 || __c == 93 { if __depth == 0 { __vscan = false; } else { __depth = __depth - 1; __q = __q + 1; } }\n");
    b.push_str("                        else { if __depth == 0 { __vscan = false; } else { __q = __q + 1; } }\n");
    b.push_str("                    }\n");
    b.push_str("                }\n");
    // Trim trailing whitespace so the value span ends at the value's last
    // byte (number digit / closing quote), not on the delimiter's run of
    // whitespace — range_parse_int and the string-slice both need this.
    b.push_str("                let mut __ve = __q;\n");
    b.push_str("                while __ve > __vs {\n");
    b.push_str("                    let __tc = std::str::byte_at_unchecked(__s, __ve - 1);\n");
    b.push_str("                    if __tc == 32 || __tc == 9 || __tc == 10 || __tc == 13 { __ve = __ve - 1; } else { break; }\n");
    b.push_str("                }\n");
    // Per-field dispatch on the key span.
    let mut first = true;
    for f in &t.fields {
        let kw = if first { "if" } else { "} else if" };
        first = false;
        b.push_str(&format!(
            "                {} (__ke - __ks) == {} && std::str::range_eq(__s, __ks, __ke, \"{}\") {{\n",
            kw,
            f.key.len(),
            f.key
        ));
        match &f.kind {
            // String: slice directly when escape-free (the common case),
            // only unescape-copy when the scan saw a backslash.
            FieldKind::Scalar(ScalarTy::Str) => b.push_str(&format!(
                "                    if __esc {{ __f_{} = std::json::unescape_string(__s[(__vs + 1)..(__ve - 1)]); }} \
                 else {{ __f_{} = __s[(__vs + 1)..(__ve - 1)]; }}\n",
                f.name, f.name
            )),
            FieldKind::Scalar(s) => b.push_str(&format!(
                "                    __f_{} = {};\n",
                f.name,
                scalar_value_expr(*s)
            )),
            FieldKind::Nested(_) => b.push_str(&format!(
                "                    __raw_{} = __s[__vs..__ve];\n",
                f.name
            )),
        }
        if f.default_src.is_none() {
            b.push_str(&format!("                    __seen_{} = true;\n", f.name));
        }
    }
    if !first {
        b.push_str("                }\n");
    }
    b.push_str("                __p = __q;\n");
    b.push_str("            }\n"); // close the `:` else
    b.push_str("        }\n"); // close the key else
    b.push_str("    }\n"); // close while __more
    // Presence checks.
    for f in &t.fields {
        if f.default_src.is_none() {
            b.push_str(&format!(
                "    if !__seen_{} {{ fail JsonError {{ kind: \"missing_field\", field: \"{}\" }}; }}\n",
                f.name, f.key
            ));
        }
    }
    // Recurse into nested fields (their JsonError propagates via `or raise`).
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

/// The Hale string-literal `"<sep>\"<key>\":"` — the member prefix for the
/// emitted object (sep is "" for the first field, "," after).
fn prefix_lit(sep: &str, key: &str) -> String {
    format!("\"{}\\\"{}\\\":\"", sep, key)
}

/// Emit-side: `__json_to_json_<T>(v: T) -> String`, building the object by
/// string concat. Numbers/bools via `to_string`, strings quoted +
/// escaped, nested structs recurse. Not fallible — serialization always
/// succeeds.
fn generate_emit_src(t: &JsonType) -> String {
    let mut b = String::new();
    b.push_str(&format!(
        "fn __json_to_json_{}(__v: {}) -> String {{\n",
        t.name, t.name
    ));
    b.push_str("    let mut __b: String = \"{\";\n");
    for (i, f) in t.fields.iter().enumerate() {
        let sep = if i == 0 { "" } else { "," };
        let prefix = prefix_lit(sep, &f.key);
        let value = match &f.kind {
            FieldKind::Scalar(ScalarTy::Str) => format!(
                "\"\\\"\" + std::json::escape_string(__v.{}) + \"\\\"\"",
                f.name
            ),
            FieldKind::Scalar(_) => format!("to_string(__v.{})", f.name),
            FieldKind::Nested(tn) => format!("__json_to_json_{}(__v.{})", tn, f.name),
        };
        b.push_str(&format!("    __b = __b + {} + {};\n", prefix, value));
    }
    b.push_str("    __b = __b + \"}\";\n");
    b.push_str("    return __b;\n}\n");
    b
}

/// Synthesize `__json_parse_<T>` / `__json_to_json_<T>` / `JsonError`,
/// inject them, then rewrite every `T::from_json(s)` and `T::to_json(o)`
/// call to the generated function.
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
        if !existing_fns.contains(&format!("__json_to_json_{}", t.name)) {
            src.push_str(&generate_emit_src(t));
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
            // `T::from_json(s)` / `T::to_json(o)` with T a generated type
            // -> `__json_parse_T(s)` / `__json_to_json_T(o)`.
            if args.len() == 1 {
                if let Expr::Path(qn) = callee.as_ref() {
                    if qn.segments.len() == 2 && names.contains(&qn.segments[0].name) {
                        let prefix = match qn.segments[1].name.as_str() {
                            "from_json" => Some("__json_parse_"),
                            "to_json" => Some("__json_to_json_"),
                            _ => None,
                        };
                        if let Some(prefix) = prefix {
                            let fname = format!("{}{}", prefix, qn.segments[0].name);
                            *callee = Box::new(Expr::Ident(Ident {
                                name: fname,
                                span: *span,
                            }));
                        }
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
