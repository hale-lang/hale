//! JSON Tier 2: compiler-generated, schema-specialized parsers.
//!
//! A struct that carries at least one `json:"<key>"` field tag opts into a
//! generated `Type::from_json(s) -> Type fallible(JsonError)`. For each such
//! type we synthesize a `__json_parse_<Type>` function that drives the
//! single-pass `std::json` object cursor (see `../../hale-stdlib/hl/json.hl`),
//! dispatching each key (length-gated) and reading the value by the field's
//! declared type — one pass, no per-field rescan, unknown keys skipped.
//! `Type::from_json(s)` calls are rewritten to the generated function.
//!
//! Semantics (v1, scalar fields):
//!   - key = the `json:` tag value, else the field name.
//!   - a missing field raises `JsonError { kind: "missing_field", field }`,
//!     UNLESS the field declares a literal `= default`, in which case the
//!     default is kept (optional-with-default for free).
//!   - field types: Int / Float / Bool / String / `Decimal<S>` (a JSON
//!     number, its exact digits at the field's scale; a number with a
//!     nonzero digit past the scale is the field's `wrong_type`, never a
//!     truncation). A type with any non-scalar field is left ungenerated
//!     (nested types + arrays are a follow-up).
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
use crate::{parse_source, Span};

#[derive(Clone, Copy)]
enum ScalarTy {
    Int,
    Float,
    Bool,
    Str,
    /// `Decimal<S>`, by its scale.
    Decimal(u8),
}

impl ScalarTy {
    fn type_name(self) -> String {
        match self {
            ScalarTy::Int => "Int".into(),
            ScalarTy::Float => "Float".into(),
            ScalarTy::Bool => "Bool".into(),
            ScalarTy::Str => "String".into(),
            ScalarTy::Decimal(s) => format!("Decimal<{s}>"),
        }
    }
    fn zero(self) -> &'static str {
        match self {
            ScalarTy::Int => "0",
            ScalarTy::Float => "0.0",
            ScalarTy::Bool => "false",
            ScalarTy::Str => "\"\"",
            ScalarTy::Decimal(_) => "0d",
        }
    }
}

#[derive(Clone)]
enum FieldKind {
    Scalar(ScalarTy),
    /// A field whose type is another generated JSON struct — parsed by
    /// recursing into the nested object's raw text.
    Nested(String),
    /// A `Time` (the api codecs only): on the JSON wire its RFC 3339 text in
    /// UTC (`std::time::iso8601`: the fraction to the nanosecond, its
    /// trailing zeros dropped, none when it is zero), read back by
    /// `std::time::parse_time` with the `Z` required, an instant outside
    /// `Time`'s range `wrong_type`; as protobuf its nanoseconds since the
    /// epoch, an `int64`.
    Time,
    /// A list (the api codecs only): `bounded[T; N]` (`exact` false: up to
    /// `cap` elements, absent is empty) or `[T; N]` (`exact`: all `cap`
    /// of them). A JSON array, a protobuf `repeated` field; more elements
    /// than `cap` is `too_long` and fewer than an exact list's `too_short`,
    /// never a truncation. Decoded into the holder `__ApiList_<T>_<field>`
    /// (`{ b: bounded[<elem_ty>; cap] }`) as it is read, then moved into
    /// the record.
    List { elem: Elem, elem_ty: String, cap: u64, exact: bool, key: Option<String> },
    /// An enum (the api codecs only, spec/api.md § Codecs): on the JSON wire
    /// a payload-free variant is its name and one with values an object
    /// keyed by it; in protobuf a payload-free enum is its number and one
    /// with payloads a message of a `oneof`. Read like a record, by the
    /// enum's own decoder (`__api_decode_<E>(text, key)`).
    Enum { name: String, unit_only: bool },
}

/// A `Time` no payload has set yet: the epoch.
const EPOCH: &str = "`1970-01-01T00:00:00Z`";

/// An element of a list field: what a field of the same type would be.
#[derive(Clone)]
enum Elem {
    Scalar(ScalarTy, Option<Conv>),
    Time,
    Nested(String),
    /// An enum, and whether it has no payloads.
    Enum { name: String, unit_only: bool },
}

/// A list on the wire: `bounded[T; N]` (`N`, not exact) or `[T; N]` with a
/// literal `N` of at least one (`N`, exact); `None` for any other type,
/// `[T]` and `[T; 0]` among them (spec/api.md § Codecs).
pub fn wire_list(te: &TypeExpr) -> Option<(&TypeExpr, u64, bool)> {
    match te {
        TypeExpr::Bounded { elem, cap, .. } => Some((elem, *cap, false)),
        TypeExpr::Array { elem, size: Some(Expr::Literal(Literal::Int(n), _)), .. } if *n > 0 => {
            Some((elem, *n as u64, true))
        }
        _ => None,
    }
}

/// Whether a field of the (dealiased) type `ty` must be present in a
/// payload: a `bounded[T; N]` never is (absent, it is empty, as its
/// declaration leaves it), a fixed array always is (the codec does not read
/// its default), and any other field unless it declares a default.
pub fn wire_required(ty: &TypeExpr, has_default: bool) -> bool {
    match wire_list(ty) {
        Some((_, _, exact)) => exact,
        None => !has_default,
    }
}

#[derive(Clone)]
struct JsonField {
    name: String,           // struct field name
    key: String,            // JSON key (tag value or field name)
    kind: FieldKind,
    default_src: Option<String>, // scalar literal default; None for nested/required
    /// GH #1417 (R2a): the field's type is a scalar of the unit dialect
    /// (an identity, a range or a quantity): on the wire its `Int`, read
    /// as `Int` and converted at the struct's construction, converted back
    /// where it is emitted (spec/units.md § Layout and the wire).
    conv: Option<Conv>,
    /// GH #1417 (R8b): the field's protobuf number, its place among the
    /// struct's declared fields counted from 1 (0 where no protobuf codec
    /// is made).
    pb_no: u32,
    /// A presence pair (the api codecs only, spec/api.md § Codecs): the
    /// field is optional, or it is another field's presence flag.
    presence: Presence,
}

/// A field's part in a presence pair: `x` with a sibling `has_x: Bool`
/// is optional, present on the JSON wire as a key with a value other
/// than `null`, and `has_x` says whether it was; the flag is no JSON key.
/// Protobuf carries both fields as declared.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Presence {
    No,
    /// The optional field; its flag's name.
    Tracked(String),
    /// The flag; the optional field's name.
    Flag(String),
}

impl JsonField {
    /// Whether a payload must carry the field: every field without a
    /// default, but a `bounded[T; N]` (empty when absent) and either half
    /// of a presence pair (an absent optional field is its flag's `false`).
    fn needs_seen(&self) -> bool {
        self.presence == Presence::No
            && self.default_src.is_none()
            && !matches!(self.kind, FieldKind::List { exact: false, .. })
    }
    /// Whether the JSON decoder records that the field's key was read:
    /// a required field, and an optional one (its flag's value).
    fn tracks(&self) -> bool {
        self.needs_seen() || matches!(self.presence, Presence::Tracked(_))
    }
    /// The presence flag: no JSON key.
    fn is_flag(&self) -> bool {
        matches!(self.presence, Presence::Flag(_))
    }
}

/// The presence pairs of a struct's fields (spec/api.md § Codecs): each
/// `(x, has_x)` where `has_x` is a field of the same struct whose type
/// `is_bool` admits and `x` another field of it. `x` is optional on the
/// JSON wire, and `has_x` says whether a payload carried it.
pub fn presence_pairs<'a>(
    fields: &'a [crate::ast::StructField],
    is_bool: &dyn Fn(&TypeExpr) -> bool,
) -> Vec<(&'a str, &'a str)> {
    let mut out = Vec::new();
    for f in fields {
        let Some(of) = f.name.name.strip_prefix("has_") else { continue };
        if of.is_empty() || !is_bool(&f.ty) {
            continue;
        }
        if fields.iter().any(|g| g.name.name == of) {
            out.push((of, f.name.name.as_str()));
        }
    }
    out
}

/// How a scalar of the unit dialect crosses the JSON wire as its `Int`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Conv {
    /// `type OrderId = distinct Int;` — `OrderId(n)` and `Int(id)`.
    Identity(String),
    /// `type Money = quantity Int in cent;` — `n * 1cent` and `q / 1cent`
    /// (the multiple and the unit).
    Quantity(u64, String),
    /// A range (`Int { range: 0..10; }`, or an identity with one):
    /// narrowing from the wire's `Int` may leave the range, which is a
    /// wrong type — `Lvl(n) or fail …` and `Int(l)`. The `Int` is the
    /// range's zero, the value an absent optional field of it holds: `0`
    /// where the range holds it, its least value where it does not.
    Range(String, i64),
}

impl Conv {
    /// The expression that reads the wire's `int` as the field's type.
    pub fn from_int(&self, int: &str, key: &str) -> String {
        match self {
            Conv::Identity(n) => format!("{}({})", n, int),
            Conv::Quantity(m, u) => format!("{} * {}{}", int, m, u),
            Conv::Range(n, _) => format!(
                "{}({}) or fail JsonError {{ kind: \"wrong_type\", field: \"{}\" }}",
                n, int, key
            ),
        }
    }
    /// The expression that writes the field's value `v` as the wire's `Int`.
    pub fn to_int(&self, v: &str) -> String {
        match self {
            Conv::Identity(_) | Conv::Range(..) => format!("Int({})", v),
            Conv::Quantity(m, u) => format!("({} / {}{})", v, m, u),
        }
    }
}

/// The scalars of the unit dialect a bundle declares that cross the JSON
/// wire as their `Int`, by name (a point does not: its origin is not a
/// count).
pub fn scalar_convs(programs: &[&Program]) -> std::collections::BTreeMap<String, Conv> {
    use crate::ast::{ScalarClause, ScalarKind};
    fn walk(items: &[TopDecl], out: &mut std::collections::BTreeMap<String, Conv>) {
        for item in items {
            match item {
                TopDecl::Type(td) => {
                    let TypeDeclBody::Scalar(sd) = &td.body else { continue };
                    let name = td.name.name.clone();
                    let ranged = sd.clauses.iter().any(|c| matches!(c, ScalarClause::Range { .. }));
                    // the range's zero: 0 where it holds it, its least value otherwise
                    let lit = |e: &Expr| match e {
                        Expr::Literal(Literal::Int(n), _) => Some(*n),
                        Expr::Unary { op: crate::ast::UnaryOp::Neg, operand, .. } => match operand.as_ref() {
                            Expr::Literal(Literal::Int(n), _) => Some(-*n),
                            _ => None,
                        },
                        _ => None,
                    };
                    let zero = sd
                        .clauses
                        .iter()
                        .find_map(|c| match c {
                            ScalarClause::Range { lo, hi, inclusive, .. } => {
                                let (lo, hi) = (lit(lo)?, lit(hi)?);
                                let holds = lo <= 0 && if *inclusive { 0 <= hi } else { 0 < hi };
                                Some(if holds { 0 } else { lo })
                            }
                            _ => None,
                        })
                        .unwrap_or(0);
                    let conv = match sd.kind {
                        Some(ScalarKind::Point) => None,
                        Some(ScalarKind::Quantity) => sd
                            .denom
                            .as_ref()
                            .filter(|_| matches!(sd.base, TypeExpr::Primitive(PrimType::Int, _)))
                            .map(|d| Conv::Quantity(d.multiple, d.unit.name.clone())),
                        Some(ScalarKind::Distinct) => {
                            Some(if ranged { Conv::Range(name.clone(), zero) } else { Conv::Identity(name.clone()) })
                        }
                        None => ranged.then(|| Conv::Range(name.clone(), zero)),
                    };
                    if let Some(c) = conv {
                        out.insert(name, c);
                    }
                }
                TopDecl::Module(m) => walk(&m.items, out),
                _ => {}
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    for p in programs {
        walk(&p.items, &mut out);
    }
    out
}

struct JsonType {
    name: String,
    fields: Vec<JsonField>,
    /// A wire map's value (spec/api.md § Codecs): the record without its key
    /// field `lifted`, which the decoder takes as `__key` (the map's key) and
    /// the encoder leaves out. Its codecs are `..._<T>__by_<lifted>`.
    lifted: Option<String>,
    /// The lifted key's presence flag, when the key field is optional: the
    /// map supplied the key, so the value decoder sets it.
    lifted_flag: Option<String>,
}

fn scalar_of(te: &TypeExpr) -> Option<ScalarTy> {
    match te {
        TypeExpr::Primitive(PrimType::Int, _) => Some(ScalarTy::Int),
        TypeExpr::Primitive(PrimType::Float, _) => Some(ScalarTy::Float),
        TypeExpr::Primitive(PrimType::Bool, _) => Some(ScalarTy::Bool),
        TypeExpr::Primitive(PrimType::String, _) => Some(ScalarTy::Str),
        TypeExpr::Primitive(PrimType::Decimal(s), _) => Some(ScalarTy::Decimal(*s)),
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
        // The lexer keeps a Decimal's digits; the spelling had its `d`.
        Expr::Literal(Literal::Decimal(d), _) => Some(format!("{}d", d.trim_end_matches('d'))),
        Expr::Literal(Literal::String(s), _) => {
            Some(format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
        }
        Expr::Literal(Literal::Time(s), _) => Some(format!("`{s}`")),
        Expr::Literal(Literal::Quantity { value, unit }, _) => Some(format!("{value}{unit}")),
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
                            FieldKind::Nested(_) | FieldKind::Time | FieldKind::List { .. } | FieldKind::Enum { .. } => None,
                        };
                        jfields.push(JsonField {
                            name: f.name.name.clone(),
                            key: tag_json_key(&f.tag).unwrap_or_else(|| f.name.name.clone()),
                            kind,
                            default_src,
                            conv: None,
                            pb_no: 0,
                            presence: Presence::No,
                        });
                    }
                    if ok && !jfields.is_empty() {
                        out.push(JsonType { name: td.name.name.clone(), fields: jfields, lifted: None, lifted_flag: None });
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
        // Read strictly in either mode (`strict_scalar_value_expr`).
        ScalarTy::Decimal(_) => "std::str::range_parse_decimal(__s, __vs, __ve) or 0d",
    }
}

/// The strict reading: a number that does not parse is a wrong type,
/// and a bool is exactly `true` or `false`.
fn strict_scalar_value_expr(s: ScalarTy, key: &str) -> String {
    match s {
        ScalarTy::Int => format!(
            "std::str::range_parse_int(__s, __vs, __ve) or fail JsonError {{ kind: \"wrong_type\", field: \"{}\" }}",
            key
        ),
        ScalarTy::Float => format!(
            "std::str::parse_float(__s[__vs..__ve]) or fail JsonError {{ kind: \"wrong_type\", field: \"{}\" }}",
            key
        ),
        ScalarTy::Decimal(_) => format!(
            "std::str::range_parse_decimal(__s, __vs, __ve) or fail JsonError {{ kind: \"wrong_type\", field: \"{}\" }}",
            key
        ),
        // Handled as a statement by the caller; never reached.
        ScalarTy::Bool => format!("std::str::range_eq(__s, __vs, __ve, \"true\") /* {} */", key),
        ScalarTy::Str => "std::json::unescape_string(__s[(__vs + 1)..(__ve - 1)])".to_string(),
    }
}

fn generate_parser_src(t: &JsonType) -> String {
    generate_parser_src_mode(t, "__json_parse_", false)
}

/// `strict` refuses a value of the wrong JSON kind for its field
/// (`JsonError { kind: "wrong_type", field }`) instead of reading it
/// as the type's zero; the api binding's decoders are strict so a
/// handler only ever sees a payload the caller actually sent.
fn generate_parser_src_mode(t: &JsonType, prefix: &str, strict: bool) -> String {
    let mut b = String::new();
    match &t.lifted {
        Some(k) => b.push_str(&format!(
            "fn {prefix}{n}__by_{k}(__s: String, __key: String) -> {n} fallible(JsonError) {{\n",
            n = t.name
        )),
        None => b.push_str(&format!(
            "fn {}{}(__s: String) -> {} fallible(JsonError) {{\n",
            prefix, t.name, t.name
        )),
    }
    b.push_str("    let __total = len(__s);\n");
    // Field accumulators + presence flags. Scalars hold the parsed value;
    // nested fields hold the matched object's raw text (parsed at
    // construction). A presence flag is omitted when a scalar default
    // covers a missing key.
    for f in &t.fields {
        // a presence flag is no key: its value is whether its field was read
        if f.is_flag() {
            continue;
        }
        match &f.kind {
            FieldKind::Scalar(s) => {
                // an optional field left out holds its zero, a range's own
                let init = if matches!(f.presence, Presence::Tracked(_)) {
                    wire_zero(*s, &f.conv, &f.default_src)
                } else {
                    f.default_src.clone().unwrap_or_else(|| s.zero().to_string())
                };
                b.push_str(&format!(
                    "    let mut __f_{}: {} = {};\n",
                    f.name,
                    s.type_name(),
                    init
                ));
            }
            FieldKind::Nested(_) | FieldKind::Enum { .. } => {
                b.push_str(&format!("    let mut __raw_{}: String = \"\";\n", f.name));
            }
            FieldKind::Time => {
                let init = f.default_src.as_deref().unwrap_or(EPOCH);
                b.push_str(&format!("    let mut __f_{}: Time = {};\n", f.name, init));
            }
            FieldKind::List { .. } => {
                b.push_str(&format!("    let mut __h_{} = {} {{ }};\n", f.name, holder_name(&t.name, &f.name)));
            }
        }
        if f.tracks() {
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
    for f in t.fields.iter().filter(|f| !f.is_flag()) {
        let kw = if first { "if" } else { "} else if" };
        first = false;
        b.push_str(&format!(
            "                {} (__ke - __ks) == {} && std::str::range_eq(__s, __ks, __ke, \"{}\") {{\n",
            kw,
            f.key.len(),
            f.key
        ));
        // an optional field's `null` is its absence (the last of a key sent
        // twice wins, as for any field)
        let optional = matches!(f.presence, Presence::Tracked(_));
        if optional {
            // a `null` after an earlier occurrence leaves nothing of it: the
            // field is back at its default (or zero), a list empty
            let reset = match &f.kind {
                FieldKind::Scalar(s) => format!("__f_{} = {};", f.name, wire_zero(*s, &f.conv, &f.default_src)),
                FieldKind::Time => format!("__f_{} = {};", f.name, f.default_src.as_deref().unwrap_or(EPOCH)),
                FieldKind::Nested(_) | FieldKind::Enum { .. } => format!("__raw_{} = \"\";", f.name),
                FieldKind::List { .. } => format!("clear(__h_{}.b);", f.name),
            };
            b.push_str(&format!(
                "                if std::str::range_eq(__s, __vs, __ve, \"null\") {{ __seen_{} = false; {reset} }} else {{\n",
                f.name
            ));
        }
        if strict {
            let want = match &f.kind {
                FieldKind::Scalar(ScalarTy::Str) => "__c0 == 34",
                FieldKind::Scalar(ScalarTy::Int)
                | FieldKind::Scalar(ScalarTy::Float)
                | FieldKind::Scalar(ScalarTy::Decimal(_)) => "__c0 == 45 || (__c0 >= 48 && __c0 <= 57)",
                FieldKind::Scalar(ScalarTy::Bool) => "__c0 == 116 || __c0 == 102",
                FieldKind::Nested(_) => "__c0 == 123",
                FieldKind::Enum { .. } => "__c0 == 34 || __c0 == 123",
                FieldKind::Time => "__c0 == 34",
                FieldKind::List { key: Some(_), .. } => "__c0 == 123",
                FieldKind::List { .. } => "__c0 == 91",
            };
            b.push_str(&format!(
                "                    let __c0 = if __vs < __ve {{ std::str::byte_at_unchecked(__s, __vs) }} else {{ 0 }};\n                    if !({}) {{ fail JsonError {{ kind: \"wrong_type\", field: \"{}\" }}; }}\n",
                want, f.key
            ));
        }
        match &f.kind {
            // String: slice directly when escape-free (the common case),
            // only unescape-copy when the scan saw a backslash.
            FieldKind::Scalar(ScalarTy::Str) => b.push_str(&format!(
                "                    if __esc {{ __f_{} = std::json::unescape_string(__s[(__vs + 1)..(__ve - 1)]); }} \
                 else {{ __f_{} = __s[(__vs + 1)..(__ve - 1)]; }}\n",
                f.name, f.name
            )),
            // A strict bool is a statement, not an expression: `fail`
            // ends a block and does not produce a value.
            FieldKind::Scalar(ScalarTy::Bool) if strict => b.push_str(&format!(
                "                    if std::str::range_eq(__s, __vs, __ve, \"true\") {{ __f_{n} = true; }} \
                 else if std::str::range_eq(__s, __vs, __ve, \"false\") {{ __f_{n} = false; }} \
                 else {{ fail JsonError {{ kind: \"wrong_type\", field: \"{k}\" }}; }}\n",
                n = f.name,
                k = f.key
            )),
            // A Decimal with a nonzero digit past its field's scale is a
            // value the field cannot hold: refused in either mode, never
            // read as zero.
            FieldKind::Scalar(s) if strict || matches!(s, ScalarTy::Decimal(_)) => b.push_str(&format!(
                "                    __f_{} = {};\n",
                f.name,
                strict_scalar_value_expr(*s, &f.key)
            )),
            FieldKind::Scalar(s) => b.push_str(&format!(
                "                    __f_{} = {};\n",
                f.name,
                scalar_value_expr(*s)
            )),
            FieldKind::Nested(_) | FieldKind::Enum { .. } => b.push_str(&format!(
                "                    __raw_{} = __s[__vs..__ve];\n",
                f.name
            )),
            FieldKind::Time => {
                let wrong = format!("fail JsonError {{ kind: \"wrong_type\", field: \"{}\" }}", f.key);
                b.push_str("                    let mut __tt = __s[(__vs + 1)..(__ve - 1)];\n");
                b.push_str("                    if __esc { __tt = std::json::unescape_string(__tt); }\n");
                b.push_str(&format!("                    if !std::str::ends_with(__tt, \"Z\") {{ {wrong}; }}\n"));
                b.push_str(&format!("                    __f_{} = std::time::parse_time(__tt) or {wrong};\n", f.name));
            }
            FieldKind::List { elem: Elem::Nested(tn), key: Some(k), .. } => b.push_str(&json_map_read(f, tn, k, prefix)),
            FieldKind::List { elem, .. } => b.push_str(&json_list_read(f, elem, prefix)),
        }
        if f.tracks() {
            b.push_str(&format!("                    __seen_{} = true;\n", f.name));
        }
        if optional {
            b.push_str("                }\n");
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
        if f.needs_seen() {
            b.push_str(&format!(
                "    if !__seen_{} {{ fail JsonError {{ kind: \"missing_field\", field: \"{}\" }}; }}\n",
                f.name, f.key
            ));
        }
    }
    // Recurse into nested fields (their JsonError propagates via `or raise`);
    // an absent optional record is its type's zero.
    for f in &t.fields {
        if let FieldKind::Nested(tn) = &f.kind {
            if matches!(f.presence, Presence::Tracked(_)) {
                b.push_str(&format!("    let __p_{n} = __api_optional_{tn}(__seen_{n}, __raw_{n}) or raise;\n", n = f.name));
                continue;
            }
            b.push_str(&format!(
                "    let __p_{} = {}{}(__raw_{}) or raise;\n",
                f.name, prefix, tn, f.name
            ));
        }
        // an enum's decoder names the field it reads
        if let FieldKind::Enum { name, .. } = &f.kind {
            if matches!(f.presence, Presence::Tracked(_)) {
                // an absent optional enum is its zero, built only then
                b.push_str(&format!(
                    "    let __p_{n} = __api_optional_{name}(__seen_{n}, __raw_{n}, \"{}\") or raise;\n",
                    f.key,
                    n = f.name
                ));
            } else {
                b.push_str(&format!("    let __p_{} = {prefix}{name}(__raw_{}, \"{}\") or raise;\n", f.name, f.name, f.key));
            }
        }
    }
    b.push_str(&construct_src(t, true));
    b.push_str("}\n");
    b
}

/// The prefix of a record's zero (`__api_zero_<T>`): the value an absent
/// optional record of type `T` holds, every field its default or its
/// type's zero (spec/api.md § Codecs). Fallible only for a range a zero
/// leaves (`wrong_type`).
const ZERO: &str = "__api_zero_";

/// The wire value a scalar field holds before a payload sets it: its
/// default, else its type's zero (a range's own, spec/api.md § Codecs).
fn wire_zero(s: ScalarTy, conv: &Option<Conv>, default: &Option<String>) -> String {
    match (default, conv) {
        (Some(d), _) => d.clone(),
        (None, Some(Conv::Range(_, z))) => z.to_string(),
        (None, _) => s.zero().to_string(),
    }
}

/// A list element's zero: its scalar's (through its conversion), the
/// epoch, or its record's zero.
fn elem_zero(elem: &Elem, key: &str) -> String {
    match elem {
        Elem::Scalar(s, Some(c)) => c.from_int(&wire_zero(*s, &Some(c.clone()), &None), key),
        Elem::Scalar(s, None) => s.zero().to_string(),
        Elem::Time => EPOCH.to_string(),
        Elem::Nested(tn) | Elem::Enum { name: tn, .. } => format!("{ZERO}{tn}() or raise"),
    }
}

/// The reader of the optional fixed array `field` of `record`.
fn optional_array_name(record: &str, field: &str) -> String {
    format!("__api_optional_{record}__{field}")
}

/// The readers of an optional value, which build its zero only when it is
/// absent (a zero can fail, for a range that leaves it out, where the value
/// sent is valid): `__api_optional_<T>(present, text)` and
/// `__api_pb_optional_<T>(bytes)` for a record, and
/// `__api_optional_<T>__<field>(present, holder)` for each optional fixed
/// array of `t`.
fn generate_optional_src(t: &JsonType, protobuf: bool) -> String {
    let n = &t.name;
    let mut b = if protobuf {
        format!(
            "fn __api_pb_optional_{n}(__raw: Bytes) -> {n} fallible(JsonError) {{\n    if len(__raw) > 0 {{ return __api_pb_decode_{n}(__raw) or raise; }}\n    return {ZERO}{n}() or raise;\n}}\n"
        )
    } else {
        format!(
            "fn __api_optional_{n}(__present: Bool, __raw: String) -> {n} fallible(JsonError) {{\n    if __present {{ return __api_decode_{n}(__raw) or raise; }}\n    return {ZERO}{n}() or raise;\n}}\n"
        )
    };
    if protobuf {
        return b;
    }
    for f in &t.fields {
        let FieldKind::List { elem, elem_ty, cap, exact: true, .. } = &f.kind else { continue };
        if !matches!(f.presence, Presence::Tracked(_)) {
            continue;
        }
        let short = format!("fail JsonError {{ kind: \"too_short\", field: \"{}\" }}", f.key);
        b.push_str(&format!(
            "fn {}(__present: Bool, __h: {}) -> [{elem_ty}; {cap}] fallible(JsonError) {{\n",
            optional_array_name(n, &f.name),
            holder_name(n, &f.name)
        ));
        b.push_str(&format!("    if !__present {{ return [{}; {cap}]; }}\n", elem_zero(elem, &f.key)));
        b.push_str(&format!("    if count(__h.b) != {cap} {{ {short}; }}\n"));
        b.push_str(&format!("    let __a0 = at(__h.b, 0) or {short};\n"));
        b.push_str(&format!("    let mut __a = [{}; {cap}];\n", unheld(elem, "__a0")));
        b.push_str("    let mut __ai = 1;\n");
        b.push_str(&format!("    while __ai < {cap} {{\n"));
        b.push_str(&format!("        let __av = at(__h.b, __ai) or {short};\n"));
        b.push_str(&format!("        __a[__ai] = {};\n", unheld(elem, "__av")));
        b.push_str("        __ai = __ai + 1;\n    }\n");
        b.push_str("    return __a;\n}\n");
    }
    b
}

/// `__api_zero_<T>() -> T fallible(JsonError)`.
fn generate_zero_src(t: &JsonType) -> String {
    let mut inits = Vec::new();
    for f in &t.fields {
        let v = match &f.kind {
            FieldKind::Scalar(s) => {
                let d = wire_zero(*s, &f.conv, &f.default_src);
                match &f.conv {
                    Some(c) => c.from_int(&d, &f.key),
                    None => d,
                }
            }
            FieldKind::Time => f.default_src.clone().unwrap_or_else(|| EPOCH.to_string()),
            FieldKind::Nested(tn) | FieldKind::Enum { name: tn, .. } => format!("{ZERO}{tn}() or raise"),
            FieldKind::List { exact: false, .. } => continue,
            FieldKind::List { elem, cap, exact: true, .. } => format!("[{}; {cap}]", elem_zero(elem, &f.key)),
        };
        inits.push(format!("{}: {v}", f.name));
    }
    format!(
        "fn {ZERO}{n}() -> {n} fallible(JsonError) {{\n    return {n} {{ {} }};\n}}\n",
        inits.join(", "),
        n = t.name
    )
}

/// The name of the holder a list field is decoded into.
fn holder_name(record: &str, field: &str) -> String {
    format!("__ApiList_{record}_{field}")
}

/// The holder of the list field `field` of `t`: `type __ApiList_<T>_<f> { b:
/// bounded[<elem>; <cap>]; }`, a local the decoders push each element into
/// as it is read (a `bounded` field is a type's, never a local's).
fn holder_src(t: &JsonType, field: &str) -> String {
    let mut out = String::new();
    for f in t.fields.iter().filter(|f| f.name == field) {
        if let FieldKind::List { elem, elem_ty, cap, .. } = &f.kind {
            let held = if matches!(elem, Elem::Time) { "Int" } else { elem_ty.as_str() };
            out.push_str(&format!("type {} {{ b: bounded[{held}; {cap}]; }}\n", holder_name(&t.name, &f.name)));
        }
    }
    out
}

/// What the holder keeps of an element `v`: the element itself, but an
/// instant as its nanoseconds (a `bounded` holds no `Time`; a `[Time; N]`
/// is a record's field all the same).
fn held(elem: &Elem, v: &str) -> String {
    match elem {
        Elem::Time => format!("std::time::nanos({v})"),
        _ => v.to_string(),
    }
}

/// The element the holder's `v` is.
fn unheld(elem: &Elem, v: &str) -> String {
    match elem {
        Elem::Time => format!("std::time::from_nanos({v})"),
        _ => v.to_string(),
    }
}

/// The end of a decoder: a fixed array built from its holder (refused
/// `too_short` when the payload held fewer than its length), the record
/// constructed from the fields read, and each `bounded` field filled from
/// its holder (the record's own field starts empty, and the holder holds
/// no more than it does).
fn construct_src(t: &JsonType, json: bool) -> String {
    let mut b = String::new();
    for f in &t.fields {
        if let FieldKind::List { elem, cap, exact: true, .. } = &f.kind {
            let short = format!("fail JsonError {{ kind: \"too_short\", field: \"{}\" }}", f.key);
            // an optional fixed array the payload does not carry is its
            // elements' zeros: absent from JSON (or `null`), or no occurrence
            // in a message
            if matches!(f.presence, Presence::Tracked(_)) {
                let present = if json { format!("__seen_{}", f.name) } else { format!("count(__h_{}.b) > 0", f.name) };
                b.push_str(&format!(
                    "    let __a_{n} = {}({present}, __h_{n}) or raise;\n",
                    optional_array_name(&t.name, &f.name),
                    n = f.name
                ));
                continue;
            }
            b.push_str(&format!("    if count(__h_{n}.b) != {cap} {{ {short}; }}\n", n = f.name));
            b.push_str(&format!("    let __a0_{n} = at(__h_{n}.b, 0) or {short};\n", n = f.name));
            b.push_str(&format!("    let mut __a_{n} = [{}; {cap}];\n", unheld(elem, &format!("__a0_{}", f.name)), n = f.name));
            b.push_str(&format!("    let mut __ai_{n} = 1;\n", n = f.name));
            b.push_str(&format!("    while __ai_{n} < {cap} {{\n", n = f.name));
            b.push_str(&format!("        let __av_{n} = at(__h_{n}.b, __ai_{n}) or {short};\n", n = f.name));
            b.push_str(&format!("        __a_{n}[__ai_{n}] = {};\n", unheld(elem, &format!("__av_{}", f.name)), n = f.name));
            b.push_str(&format!("        __ai_{n} = __ai_{n} + 1;\n    }}\n", n = f.name));
        }
    }
    let lifted = t
        .lifted
        .iter()
        .map(|k| format!("{k}: __key"))
        .chain(t.lifted_flag.iter().map(|flag| format!("{flag}: true")));
    let inits: Vec<String> = lifted
        .chain(t.fields.iter().filter_map(|f| match (&f.kind, &f.conv) {
            // on the JSON wire a presence flag is whether its field was read
            _ if json && f.is_flag() => {
                let Presence::Flag(of) = &f.presence else { unreachable!("a flag") };
                Some(format!("{}: __seen_{of}", f.name))
            }
            (FieldKind::Scalar(_), Some(c)) => Some(format!("{}: {}", f.name, c.from_int(&format!("__f_{}", f.name), &f.key))),
            (FieldKind::Scalar(_), None) => Some(format!("{}: __f_{}", f.name, f.name)),
            (FieldKind::Nested(_) | FieldKind::Enum { .. }, _) => Some(format!("{}: __p_{}", f.name, f.name)),
            (FieldKind::Time, _) => Some(format!("{}: __f_{}", f.name, f.name)),
            (FieldKind::List { exact: true, .. }, _) => Some(format!("{}: __a_{}", f.name, f.name)),
            (FieldKind::List { exact: false, .. }, _) => None,
        }))
        .collect();
    let bounded: Vec<&JsonField> =
        t.fields.iter().filter(|f| matches!(f.kind, FieldKind::List { exact: false, .. })).collect();
    if bounded.is_empty() {
        b.push_str(&format!("    return {} {{ {} }};\n", t.name, inits.join(", ")));
        return b;
    }
    b.push_str(&format!("    let mut __r = {} {{ {} }};\n", t.name, inits.join(", ")));
    for f in bounded {
        let FieldKind::List { elem, .. } = &f.kind else { continue };
        b.push_str(&format!("    for __x in __h_{n}.b {{ push(__r.{n}, {}) or discard; }}\n", unheld(elem, "__x"), n = f.name));
    }
    b.push_str("    return __r;\n");
    b
}

/// The statements that read one JSON element whose text is `[{src}[__es..__ee])`
/// as `elem`, binding `__ev`; a value of the wrong kind is `wrong_type`
/// for the list's `key`.
fn json_elem_read(elem: &Elem, src: &str, key: &str, prefix: &str, ind: &str) -> String {
    let wrong = format!("fail JsonError {{ kind: \"wrong_type\", field: \"{key}\" }}");
    let mut b = format!("{ind}let __e0 = std::str::byte_at_unchecked({src}, __es);\n");
    let want = match elem {
        Elem::Scalar(ScalarTy::Str, _) => "__e0 == 34",
        Elem::Scalar(ScalarTy::Int, _) | Elem::Scalar(ScalarTy::Float, _) | Elem::Scalar(ScalarTy::Decimal(_), _) => {
            "__e0 == 45 || (__e0 >= 48 && __e0 <= 57)"
        }
        Elem::Scalar(ScalarTy::Bool, _) => "__e0 == 116 || __e0 == 102",
        Elem::Time => "__e0 == 34",
        Elem::Nested(_) => "__e0 == 123",
        Elem::Enum { .. } => "__e0 == 34 || __e0 == 123",
    };
    b.push_str(&format!("{ind}if !({want}) {{ {wrong}; }}\n"));
    match elem {
        Elem::Scalar(ScalarTy::Str, _) => {
            b.push_str(&format!("{ind}let __ev = std::json::unescape_string({src}[(__es + 1)..(__ee - 1)]);\n"));
        }
        Elem::Scalar(ScalarTy::Int, conv) => {
            b.push_str(&format!("{ind}let __ei = std::str::range_parse_int({src}, __es, __ee) or {wrong};\n"));
            match conv {
                Some(c) => b.push_str(&format!("{ind}let __ev = {};\n", c.from_int("__ei", key))),
                None => b.push_str(&format!("{ind}let __ev = __ei;\n")),
            }
        }
        Elem::Scalar(ScalarTy::Float, _) => {
            b.push_str(&format!("{ind}let __ev = std::str::parse_float({src}[__es..__ee]) or {wrong};\n"));
        }
        // a Decimal<S> element is a JSON number read at the element's scale,
        // every digit kept: the binding is declared `Decimal<S>`, so
        // `range_parse_decimal` takes that scale, and a nonzero digit past it
        // is `wrong_type`, as for a field
        Elem::Scalar(d @ ScalarTy::Decimal(_), _) => {
            b.push_str(&format!(
                "{ind}let __ev: {} = std::str::range_parse_decimal({src}, __es, __ee) or {wrong};\n",
                d.type_name()
            ));
        }
        Elem::Scalar(ScalarTy::Bool, _) => {
            b.push_str(&format!("{ind}let __ev = std::str::range_eq({src}, __es, __ee, \"true\");\n"));
            b.push_str(&format!(
                "{ind}if !__ev && !std::str::range_eq({src}, __es, __ee, \"false\") {{ {wrong}; }}\n"
            ));
        }
        Elem::Time => {
            b.push_str(&format!("{ind}let __et = std::json::unescape_string({src}[(__es + 1)..(__ee - 1)]);\n"));
            b.push_str(&format!("{ind}if !std::str::ends_with(__et, \"Z\") {{ {wrong}; }}\n"));
            b.push_str(&format!("{ind}let __ev = std::time::parse_time(__et) or {wrong};\n"));
        }
        Elem::Nested(tn) => {
            b.push_str(&format!("{ind}let __ev = {prefix}{tn}({src}[__es..__ee]) or raise;\n"));
        }
        Elem::Enum { name, .. } => {
            b.push_str(&format!("{ind}let __ev = {prefix}{name}({src}[__es..__ee], \"{key}\") or raise;\n"));
        }
    }
    b
}

/// The statements that read a wire map's value span `[__vs, __ve)`: the
/// object checked whole (a malformed one is `wrong_type`), the holder
/// cleared, and each member read as the record whose key is the member's
/// key (`<prefix><T>__by_<key>`, the value's own key field, if it sends
/// one, skipped as an unknown key); a key already read is `duplicate_key`,
/// more members than the holder holds `too_long`.
fn json_map_read(f: &JsonField, tn: &str, k: &str, prefix: &str) -> String {
    let ind = "                    ";
    let wrong = format!("fail JsonError {{ kind: \"wrong_type\", field: \"{}\" }}", f.key);
    let dup = format!("fail JsonError {{ kind: \"duplicate_key\", field: \"{}\" }}", f.key);
    let mut b = String::new();
    b.push_str(&format!("{ind}let __ma = __s[__vs..__ve];\n"));
    b.push_str(&format!("{ind}if !std::json::valid(__ma) {{ {wrong}; }}\n"));
    b.push_str(&format!("{ind}clear(__h_{}.b);\n", f.name));
    b.push_str(&format!("{ind}let mut __mi = std::json::object_first(__ma);\n"));
    b.push_str(&format!("{ind}while !__mi.done {{\n"));
    b.push_str(&format!("{ind}    let __mk = std::json::unescape_string(__ma[__mi.key_start..__mi.key_end]);\n"));
    b.push_str(&format!("{ind}    for __mx in __h_{}.b {{ if __mx.{k} == __mk {{ {dup}; }} }}\n", f.name));
    b.push_str(&format!(
        "{ind}    let __mv = {prefix}{tn}__by_{k}(__ma[__mi.val_start..__mi.val_end], __mk) or raise;\n"
    ));
    b.push_str(&format!(
        "{ind}    push(__h_{}.b, __mv) or fail JsonError {{ kind: \"too_long\", field: \"{}\" }};\n",
        f.name, f.key
    ));
    b.push_str(&format!("{ind}    __mi = std::json::object_next(__mi, __ma);\n"));
    b.push_str(&format!("{ind}}}\n"));
    b
}

/// The statements that read one entry of a wire map's `map<string, T>`
/// field: the entry message's key (1) and record (2), the record decoded
/// whole and given the entry's key (the map's key wins over the record's
/// own); a key already read is `duplicate_key`, more entries than the
/// holder holds `too_long`.
fn pb_map_read(f: &JsonField, tn: &str, k: &str, bad: &str) -> String {
    let ind = "            ";
    let wrong = format!("fail JsonError {{ kind: \"wrong_type\", field: \"{}\" }}", f.key);
    let dup = format!("fail JsonError {{ kind: \"duplicate_key\", field: \"{}\" }}", f.key);
    let mut b = String::new();
    b.push_str(&format!("{ind}if __wire != 2 {{ {wrong}; }}\n"));
    b.push_str(&format!("{ind}let __me = __api_pb_vend(__b, __p);\n"));
    b.push_str(&format!("{ind}if __me < 0 {{ {bad} }}\n"));
    b.push_str(&format!("{ind}let __ml = __api_pb_vval(__b, __p, __me);\n"));
    b.push_str(&format!("{ind}if __ml < 0 || __me + __ml > __n {{ {bad} }}\n"));
    b.push_str(&format!("{ind}let __mv = __api_pb_entry_{tn}__by_{k}(std::bytes::slice(__b, __me, __me + __ml), \"{}\") or raise;\n", f.key));
    b.push_str(&format!("{ind}for __mx in __h_{}.b {{ if __mx.{k} == __mv.{k} {{ {dup}; }} }}\n", f.name));
    b.push_str(&format!(
        "{ind}push(__h_{}.b, __mv) or fail JsonError {{ kind: \"too_long\", field: \"{}\" }};\n",
        f.name, f.key
    ));
    b.push_str(&format!("{ind}__p = __me + __ml;\n"));
    b
}

/// `__api_pb_entry_<T>__by_<k>(bytes, field)`: one entry of a
/// `map<string, T>`, its key (field 1) and its record (field 2, decoded
/// whole), the record given the entry's key; the last key sent twice
/// wins, a value message sent twice merges (its occurrences' payloads
/// concatenated, as proto3 merges a singular message), and an unknown
/// field is skipped. The key is put before the value's own fields as its
/// field `key_no`, so a value that leaves its key out is no
/// `missing_field`, then the entry's key replaces the value's (and sets
/// its presence `flag`, when the key field is optional).
fn generate_pb_entry_src(tn: &str, k: &str, key_no: u32, flag: Option<&str>) -> String {
    let wrong = "fail JsonError { kind: \"wrong_type\", field: __f };";
    let mut b = format!("fn __api_pb_entry_{tn}__by_{k}(__b: Bytes, __f: String) -> {tn} fallible(JsonError) {{\n");
    b.push_str("    let __n = len(__b);\n    let mut __p = 0;\n    let mut __key = \"\";\n");
    b.push_str("    let mut __raw = std::bytes::from_string(\"\");\n");
    b.push_str("    while __p < __n {\n");
    b.push_str("        let __te = __api_pb_vend(__b, __p);\n");
    b.push_str(&format!("        if __te < 0 {{ {wrong} }}\n"));
    b.push_str("        let __tag = __api_pb_vval(__b, __p, __te);\n");
    b.push_str("        let __no = __tag >> 3;\n        let __wire = __tag & 7;\n");
    b.push_str("        if (__no == 1 || __no == 2) && __wire == 2 {\n");
    b.push_str("            let __e = __api_pb_vend(__b, __te);\n");
    b.push_str("            let __l = __api_pb_vval(__b, __te, __e);\n");
    b.push_str(&format!("            if __e < 0 || __l < 0 || __e + __l > __n {{ {wrong} }}\n"));
    b.push_str("            if __no == 1 {\n");
    b.push_str("                __key = std::str::from_bytes(std::bytes::slice(__b, __e, __e + __l));\n");
    b.push_str(&format!("                if len(__key) != __l {{ {wrong} }}\n"));
    b.push_str("            } else {\n");
    b.push_str("                __raw = std::bytes::concat(__raw, std::bytes::slice(__b, __e, __e + __l));\n");
    b.push_str("            }\n");
    b.push_str("            __p = __e + __l;\n");
    b.push_str("        } else {\n");
    b.push_str("            let __e = __api_pb_skip(__b, __te, __wire);\n");
    b.push_str(&format!("            if __e < 0 {{ {wrong} }}\n"));
    b.push_str("            __p = __e;\n");
    b.push_str("        }\n");
    b.push_str("    }\n");
    b.push_str(&format!(
        "    let mut __v = __api_pb_decode_{tn}(std::bytes::concat(__api_pb_put_str({key_no}, __key), __raw)) or raise;\n"
    ));
    b.push_str(&format!("    __v.{k} = __key;\n"));
    if let Some(flag) = flag {
        b.push_str(&format!("    __v.{flag} = true;\n"));
    }
    b.push_str("    return __v;\n}\n");
    b
}

/// The statements that read a list field's value span `[__vs, __ve)`: the
/// array checked whole (a malformed one is `wrong_type`), the holder
/// cleared (the last of a key sent twice wins, as for any field), and
/// each element read and pushed; more than the holder holds is `too_long`.
fn json_list_read(f: &JsonField, elem: &Elem, prefix: &str) -> String {
    let ind = "                    ";
    let wrong = format!("fail JsonError {{ kind: \"wrong_type\", field: \"{}\" }}", f.key);
    let mut b = String::new();
    b.push_str(&format!("{ind}let __la = __s[__vs..__ve];\n"));
    b.push_str(&format!("{ind}if !std::json::valid(__la) {{ {wrong}; }}\n"));
    b.push_str(&format!("{ind}clear(__h_{}.b);\n", f.name));
    b.push_str(&format!("{ind}let mut __li = std::json::array_first_span(__la);\n"));
    b.push_str(&format!("{ind}while !__li.done {{\n"));
    b.push_str(&format!("{ind}    let __es = __li.cur_start;\n"));
    b.push_str(&format!("{ind}    let mut __ee = __li.cur_end;\n"));
    b.push_str(&format!("{ind}    while __ee > __es {{\n"));
    b.push_str(&format!("{ind}        let __et = std::str::byte_at_unchecked(__la, __ee - 1);\n"));
    b.push_str(&format!(
        "{ind}        if __et == 32 || __et == 9 || __et == 10 || __et == 13 {{ __ee = __ee - 1; }} else {{ break; }}\n"
    ));
    b.push_str(&format!("{ind}    }}\n"));
    b.push_str(&json_elem_read(elem, "__la", &f.key, prefix, &format!("{ind}    ")));
    b.push_str(&format!(
        "{ind}    push(__h_{}.b, {}) or fail JsonError {{ kind: \"too_long\", field: \"{}\" }};\n",
        f.name,
        held(elem, "__ev"),
        f.key
    ));
    b.push_str(&format!("{ind}    __li = std::json::array_next_span(__li, __la);\n"));
    b.push_str(&format!("{ind}}}\n"));
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
    generate_emit_src_mode(t, "__json_to_json_")
}

fn generate_emit_src_mode(t: &JsonType, fn_prefix: &str) -> String {
    let mut b = String::new();
    let suffix = t.lifted.as_ref().map_or(String::new(), |k| format!("__by_{k}"));
    b.push_str(&format!(
        "fn {}{}{suffix}(__v: {}) -> String {{\n",
        fn_prefix, t.name, t.name
    ));
    b.push_str("    let mut __b: String = \"{\";\n");
    // A record with an optional field writes a key only when its field is
    // present (and never its flag), so its separators are decided as it is
    // written; any other record's are fixed here, as they always were.
    let dynamic = t.fields.iter().any(|f| f.presence != Presence::No);
    if dynamic {
        b.push_str("    let mut __sep: String = \"\";\n");
    }
    for (i, f) in t.fields.iter().enumerate() {
        if f.is_flag() {
            continue;
        }
        let sep = if i == 0 { "" } else { "," };
        let prefix = if dynamic {
            format!("__sep + {}", prefix_lit("", &f.key))
        } else {
            prefix_lit(sep, &f.key)
        };
        if let Presence::Tracked(flag) = &f.presence {
            b.push_str(&format!("    if __v.{flag} {{\n"));
        }
        let value = match &f.kind {
            FieldKind::Scalar(ScalarTy::Str) => format!(
                "\"\\\"\" + std::json::escape_string(__v.{}) + \"\\\"\"",
                f.name
            ),
            FieldKind::Scalar(_) => match &f.conv {
                Some(c) => format!("to_string({})", c.to_int(&format!("__v.{}", f.name))),
                None => format!("to_string(__v.{})", f.name),
            },
            FieldKind::Nested(tn) => format!("{}{}(__v.{})", fn_prefix, tn, f.name),
            FieldKind::Enum { name, .. } => format!("{fn_prefix}{name}(__v.{})", f.name),
            FieldKind::Time => format!("\"\\\"\" + std::time::iso8601(__v.{}) + \"\\\"\"", f.name),
            // a wire map: an object keyed by each record's key, the record
            // without it as the value, in the list's order
            FieldKind::List { elem: Elem::Nested(tn), key: Some(k), .. } => {
                b.push_str(&format!("    __b = __b + {} + \"{{\";\n", prefix));
                b.push_str(&format!("    let mut __n_{} = 0;\n", f.name));
                b.push_str(&format!("    for __x in __v.{} {{\n", f.name));
                b.push_str(&format!("        if __n_{} > 0 {{ __b = __b + \",\"; }}\n", f.name));
                b.push_str(&format!(
                    "        __b = __b + \"\\\"\" + std::json::escape_string(__x.{k}) + \"\\\":\" + {fn_prefix}{tn}__by_{k}(__x);\n"
                ));
                b.push_str(&format!("        __n_{n} = __n_{n} + 1;\n    }}\n", n = f.name));
                b.push_str("    __b = __b + \"}\";\n");
                if dynamic {
                    b.push_str("    __sep = \",\";\n");
                }
                if matches!(f.presence, Presence::Tracked(_)) {
                    b.push_str("    }\n");
                }
                continue;
            }
            FieldKind::List { elem, elem_ty, .. } => {
                b.push_str(&format!("    __b = __b + {} + \"[\";\n", prefix));
                b.push_str(&format!("    let mut __n_{} = 0;\n", f.name));
                b.push_str(&format!("    for __x in __v.{} {{\n", f.name));
                b.push_str(&elem_bind(elem, elem_ty, "        "));
                b.push_str(&format!("        if __n_{} > 0 {{ __b = __b + \",\"; }}\n", f.name));
                b.push_str(&format!("        __b = __b + {};\n", json_elem_write(elem, "__xe", fn_prefix)));
                b.push_str(&format!("        __n_{n} = __n_{n} + 1;\n    }}\n", n = f.name));
                b.push_str("    __b = __b + \"]\";\n");
                if dynamic {
                    b.push_str("    __sep = \",\";\n");
                }
                if matches!(f.presence, Presence::Tracked(_)) {
                    b.push_str("    }\n");
                }
                continue;
            }
        };
        b.push_str(&format!("    __b = __b + {} + {};\n", prefix, value));
        if dynamic {
            b.push_str("    __sep = \",\";\n");
        }
        if matches!(f.presence, Presence::Tracked(_)) {
            b.push_str("    }\n");
        }
    }
    b.push_str("    __b = __b + \"}\";\n");
    b.push_str("    return __b;\n}\n");
    b
}

/// The loop's element `__x` bound as `__xe` of its declared type: the
/// iteration variable of a list of a unit-dialect scalar does not carry the
/// scalar's type into its conversion (`__x / 1cent` stays a quantity), the
/// annotated binding does.
fn elem_bind(elem: &Elem, elem_ty: &str, ind: &str) -> String {
    match elem {
        Elem::Scalar(_, Some(_)) | Elem::Scalar(ScalarTy::Decimal(_), _) | Elem::Time => {
            format!("{ind}let __xe: {elem_ty} = __x;\n")
        }
        _ => format!("{ind}let __xe = __x;\n"),
    }
}

/// The JSON text of the list element `v`.
fn json_elem_write(elem: &Elem, v: &str, fn_prefix: &str) -> String {
    match elem {
        Elem::Scalar(ScalarTy::Str, _) => format!("\"\\\"\" + std::json::escape_string({v}) + \"\\\"\""),
        Elem::Scalar(_, Some(c)) => format!("to_string({})", c.to_int(v)),
        Elem::Scalar(_, None) => format!("to_string({v})"),
        Elem::Time => format!("\"\\\"\" + std::time::iso8601({v}) + \"\\\"\""),
        Elem::Nested(tn) | Elem::Enum { name: tn, .. } => format!("{fn_prefix}{tn}({v})"),
    }
}

// ---- GH #1417 (R8b): the protobuf codec of a record ----------------------

/// `__api_pb_decode_<T>`: the strict reading of proto3's wire format. The
/// fields are read as the JSON decoder reads them (a field with no default
/// that is absent is `missing_field`, one of the wrong wire type is
/// `wrong_type`, the value of a unit-dialect scalar passes through its
/// conversion), by field number; an unknown field is skipped by its wire
/// type; the last of a scalar field sent twice wins, and a nested message sent
/// twice is merged (the occurrences are concatenated and decoded once, which is
/// the merge, recursively). The helpers (`__api_pb_vend`,
/// `__api_pb_vval`, `__api_pb_skip`) are `api_grpc.hl`'s.
fn generate_pb_parser_src(t: &JsonType) -> String {
    let mut b = String::new();
    let bad = "fail JsonError { kind: \"wrong_type\", field: \"payload\" };";
    b.push_str(&format!("fn __api_pb_decode_{}(__b: Bytes) -> {} fallible(JsonError) {{\n", t.name, t.name));
    b.push_str("    let __n = len(__b);\n");
    for f in &t.fields {
        match &f.kind {
            FieldKind::Scalar(s) => {
                // an optional field left out holds its zero, a range's own
                let init = if matches!(f.presence, Presence::Tracked(_)) {
                    wire_zero(*s, &f.conv, &f.default_src)
                } else {
                    f.default_src.clone().unwrap_or_else(|| s.zero().to_string())
                };
                b.push_str(&format!("    let mut __f_{}: {} = {};\n", f.name, s.type_name(), init));
            }
            FieldKind::Nested(_) | FieldKind::Enum { unit_only: false, .. } => {
                b.push_str(&format!("    let mut __raw_{}: Bytes = std::bytes::from_string(\"\");\n", f.name));
            }
            // a payload-free enum is its number
            FieldKind::Enum { unit_only: true, .. } => {
                b.push_str(&format!("    let mut __n_{}: Int = 0;\n", f.name));
            }
            FieldKind::Time => {
                let init = f.default_src.as_deref().unwrap_or(EPOCH);
                b.push_str(&format!("    let mut __f_{}: Time = {};\n", f.name, init));
            }
            FieldKind::List { .. } => {
                b.push_str(&format!("    let mut __h_{} = {} {{ }};\n", f.name, holder_name(&t.name, &f.name)));
            }
        }
        if f.needs_seen() {
            b.push_str(&format!("    let mut __seen_{}: Bool = false;\n", f.name));
        }
    }
    b.push_str("    let mut __p = 0;\n");
    b.push_str("    while __p < __n {\n");
    b.push_str("        let __te = __api_pb_vend(__b, __p);\n");
    b.push_str(&format!("        if __te < 0 {{ {bad} }}\n"));
    b.push_str("        let __tag = __api_pb_vval(__b, __p, __te);\n");
    b.push_str("        let __no = __tag >> 3;\n");
    b.push_str("        let __wire = __tag & 7;\n");
    b.push_str("        __p = __te;\n");
    for (i, f) in t.fields.iter().enumerate() {
        let kw = if i == 0 { "if" } else { "} else if" };
        b.push_str(&format!("        {kw} __no == {} {{\n", f.pb_no));
        if let FieldKind::List { elem: Elem::Nested(tn), key: Some(k), .. } = &f.kind {
            b.push_str(&pb_map_read(f, tn, k, bad));
            continue;
        }
        if let FieldKind::List { elem, .. } = &f.kind {
            b.push_str(&pb_list_read(f, elem, bad));
            if f.needs_seen() {
                b.push_str(&format!("            __seen_{} = true;\n", f.name));
            }
            continue;
        }
        let (want, what) = match &f.kind {
            FieldKind::Scalar(ScalarTy::Float) => (1, "f64"),
            FieldKind::Scalar(ScalarTy::Str)
            | FieldKind::Scalar(ScalarTy::Decimal(_))
            | FieldKind::Nested(_)
            | FieldKind::Enum { unit_only: false, .. } => (2, "len"),
            FieldKind::Scalar(_) | FieldKind::Time | FieldKind::Enum { unit_only: true, .. } => (0, "varint"),
            FieldKind::List { .. } => unreachable!("read above"),
        };
        b.push_str(&format!(
            "            if __wire != {want} {{ fail JsonError {{ kind: \"wrong_type\", field: \"{}\" }}; }}\n",
            f.key
        ));
        match what {
            "f64" => {
                b.push_str(&format!("            if __p + 8 > __n {{ {bad} }}\n"));
                b.push_str(&format!("            __f_{} = std::bytes::read_f64_le(__b, __p) or 0.0;\n", f.name));
                b.push_str("            __p = __p + 8;\n");
            }
            "len" => {
                b.push_str("            let __e = __api_pb_vend(__b, __p);\n");
                b.push_str(&format!("            if __e < 0 {{ {bad} }}\n"));
                b.push_str("            let __l = __api_pb_vval(__b, __p, __e);\n");
                b.push_str(&format!("            if __l < 0 || __e + __l > __n {{ {bad} }}\n"));
                match &f.kind {
                    // a message sent twice is merged: the occurrences are
                    // concatenated (a `oneof`'s last member then wins)
                    FieldKind::Nested(_) | FieldKind::Enum { .. } => {
                        b.push_str(&format!("            __raw_{0} = std::bytes::concat(__raw_{0}, std::bytes::slice(__b, __e, __e + __l));\n", f.name));
                    }
                    _ => {
                        b.push_str("            let __s = std::str::from_bytes(std::bytes::slice(__b, __e, __e + __l));\n");
                        b.push_str(&format!(
                            "            if len(__s) != __l {{ fail JsonError {{ kind: \"wrong_type\", field: \"{}\" }}; }}\n",
                            f.key
                        ));
                        if let FieldKind::Scalar(ScalarTy::Decimal(_)) = f.kind {
                            // A Decimal is its digits (proto3 has no decimal).
                            b.push_str(&format!(
                                "            __f_{} = std::str::parse_decimal(__s) or fail JsonError {{ kind: \"wrong_type\", field: \"{}\" }};\n",
                                f.name, f.key
                            ));
                        } else {
                            b.push_str(&format!("            __f_{} = __s;\n", f.name));
                        }
                    }
                }
                b.push_str("            __p = __e + __l;\n");
            }
            _ => {
                b.push_str("            let __e = __api_pb_vend(__b, __p);\n");
                b.push_str(&format!("            if __e < 0 {{ {bad} }}\n"));
                let value = "__api_pb_vval(__b, __p, __e)";
                if matches!(f.kind, FieldKind::Scalar(ScalarTy::Bool)) {
                    b.push_str(&format!("            __f_{} = {value} != 0;\n", f.name));
                } else if matches!(f.kind, FieldKind::Time) {
                    b.push_str(&format!("            __f_{} = std::time::from_nanos({value});\n", f.name));
                } else if matches!(f.kind, FieldKind::Enum { .. }) {
                    b.push_str(&format!("            __n_{} = {value};\n", f.name));
                } else {
                    b.push_str(&format!("            __f_{} = {value};\n", f.name));
                }
                b.push_str("            __p = __e;\n");
            }
        }
        if f.needs_seen() {
            b.push_str(&format!("            __seen_{} = true;\n", f.name));
        }
    }
    let kw = if t.fields.is_empty() { "" } else { "} else " };
    b.push_str(&format!("        {kw}{{\n"));
    b.push_str("            let __e = __api_pb_skip(__b, __p, __wire);\n");
    b.push_str(&format!("            if __e < 0 {{ {bad} }}\n"));
    b.push_str("            __p = __e;\n");
    b.push_str("        }\n");
    b.push_str("    }\n");
    for f in &t.fields {
        if f.needs_seen() {
            b.push_str(&format!(
                "    if !__seen_{} {{ fail JsonError {{ kind: \"missing_field\", field: \"{}\" }}; }}\n",
                f.name, f.key
            ));
        }
    }
    for f in &t.fields {
        if let FieldKind::Nested(tn) = &f.kind {
            // an optional record its message does not carry is its type's zero
            if matches!(f.presence, Presence::Tracked(_)) {
                b.push_str(&format!("    let __p_{n} = __api_pb_optional_{tn}(__raw_{n}) or raise;\n", n = f.name));
                continue;
            }
            b.push_str(&format!("    let __p_{} = __api_pb_decode_{}(__raw_{}) or raise;\n", f.name, tn, f.name));
        }
        match &f.kind {
            FieldKind::Enum { name, unit_only: true } => {
                b.push_str(&format!("    let __p_{} = __api_pb_enum_{name}(__n_{}, \"{}\") or raise;\n", f.name, f.name, f.key));
            }
            FieldKind::Enum { name, unit_only: false } if matches!(f.presence, Presence::Tracked(_)) => {
                b.push_str(&format!("    let __p_{} = __api_pb_optional_{name}(__raw_{}, \"{}\") or raise;\n", f.name, f.name, f.key));
            }
            FieldKind::Enum { name, unit_only: false } => {
                b.push_str(&format!("    let __p_{} = __api_pb_decode_{name}(__raw_{}, \"{}\") or raise;\n", f.name, f.name, f.key));
            }
            _ => {}
        }
    }
    b.push_str(&construct_src(t, false));
    b.push_str("}\n");
    b
}

/// The wire type of a list element as proto3 writes one: 0 a varint, 1 a
/// `double`, 2 length-delimited.
fn pb_elem_wire(elem: &Elem) -> u32 {
    match elem {
        Elem::Scalar(ScalarTy::Float, _) => 1,
        // a Decimal is its digits in a string, as a field's is
        Elem::Scalar(ScalarTy::Str, _)
        | Elem::Scalar(ScalarTy::Decimal(_), _)
        | Elem::Nested(_)
        | Elem::Enum { unit_only: false, .. } => 2,
        Elem::Scalar(_, _) | Elem::Time | Elem::Enum { unit_only: true, .. } => 0,
    }
}

/// The statements that read one element of wire type `pb_elem_wire(elem)`
/// at `__p` (no further than `end`), bind it as `__ev` and step `__p` past it.
fn pb_elem_read(elem: &Elem, key: &str, end: &str, bad: &str, ind: &str) -> String {
    let wrong = format!("fail JsonError {{ kind: \"wrong_type\", field: \"{key}\" }}");
    let mut b = String::new();
    match elem {
        Elem::Scalar(ScalarTy::Float, _) => {
            b.push_str(&format!("{ind}if __p + 8 > {end} {{ {bad} }}\n"));
            b.push_str(&format!("{ind}let __ev = std::bytes::read_f64_le(__b, __p) or 0.0;\n"));
            b.push_str(&format!("{ind}__p = __p + 8;\n"));
        }
        Elem::Scalar(ScalarTy::Str, _)
        | Elem::Scalar(ScalarTy::Decimal(_), _)
        | Elem::Nested(_)
        | Elem::Enum { unit_only: false, .. } => {
            b.push_str(&format!("{ind}let __ee = __api_pb_vend(__b, __p);\n"));
            b.push_str(&format!("{ind}if __ee < 0 {{ {bad} }}\n"));
            b.push_str(&format!("{ind}let __el = __api_pb_vval(__b, __p, __ee);\n"));
            b.push_str(&format!("{ind}if __el < 0 || __ee + __el > {end} {{ {bad} }}\n"));
            match elem {
                Elem::Nested(tn) => {
                    b.push_str(&format!(
                        "{ind}let __ev = __api_pb_decode_{tn}(std::bytes::slice(__b, __ee, __ee + __el)) or raise;\n"
                    ));
                }
                Elem::Enum { name, .. } => {
                    b.push_str(&format!(
                        "{ind}let __ev = __api_pb_decode_{name}(std::bytes::slice(__b, __ee, __ee + __el), \"{key}\") or raise;\n"
                    ));
                }
                Elem::Scalar(d @ ScalarTy::Decimal(_), _) => {
                    b.push_str(&format!(
                        "{ind}let __ed = std::str::from_bytes(std::bytes::slice(__b, __ee, __ee + __el));\n"
                    ));
                    b.push_str(&format!("{ind}if len(__ed) != __el {{ {wrong}; }}\n"));
                    b.push_str(&format!("{ind}let __ev: {} = std::str::parse_decimal(__ed) or {wrong};\n", d.type_name()));
                }
                _ => {
                    b.push_str(&format!(
                        "{ind}let __ev = std::str::from_bytes(std::bytes::slice(__b, __ee, __ee + __el));\n"
                    ));
                    b.push_str(&format!("{ind}if len(__ev) != __el {{ {wrong}; }}\n"));
                }
            }
            b.push_str(&format!("{ind}__p = __ee + __el;\n"));
        }
        Elem::Time => {
            b.push_str(&format!("{ind}let __ee = __api_pb_vend(__b, __p);\n"));
            b.push_str(&format!("{ind}if __ee < 0 || __ee > {end} {{ {bad} }}\n"));
            b.push_str(&format!("{ind}let __ev = std::time::from_nanos(__api_pb_vval(__b, __p, __ee));\n"));
            b.push_str(&format!("{ind}__p = __ee;\n"));
        }
        Elem::Scalar(s, conv) => {
            b.push_str(&format!("{ind}let __ee = __api_pb_vend(__b, __p);\n"));
            b.push_str(&format!("{ind}if __ee < 0 || __ee > {end} {{ {bad} }}\n"));
            b.push_str(&format!("{ind}let __ei = __api_pb_vval(__b, __p, __ee);\n"));
            b.push_str(&format!("{ind}__p = __ee;\n"));
            match (s, conv) {
                (ScalarTy::Bool, _) => b.push_str(&format!("{ind}let __ev = __ei != 0;\n")),
                (_, Some(c)) => b.push_str(&format!("{ind}let __ev = {};\n", c.from_int("__ei", key))),
                (_, None) => b.push_str(&format!("{ind}let __ev = __ei;\n")),
            }
        }
        // a payload-free enum is its number
        Elem::Enum { name, unit_only: true } => {
            b.push_str(&format!("{ind}let __ee = __api_pb_vend(__b, __p);\n"));
            b.push_str(&format!("{ind}if __ee < 0 || __ee > {end} {{ {bad} }}\n"));
            b.push_str(&format!("{ind}let __ev = __api_pb_enum_{name}(__api_pb_vval(__b, __p, __ee), \"{key}\") or raise;\n"));
            b.push_str(&format!("{ind}__p = __ee;\n"));
        }
    }
    b
}

/// The statements that read one occurrence of a `repeated` field: one
/// element of its own wire type, or, for a numeric element, a packed run
/// (a length-delimited field of elements, which proto3 writes by default and
/// a parser must accept either way); each pushed into the holder, more than it
/// holds `too_long`.
fn pb_list_read(f: &JsonField, elem: &Elem, bad: &str) -> String {
    let ind = "            ";
    let wire = pb_elem_wire(elem);
    let push = format!(
        "push(__h_{}.b, {}) or fail JsonError {{ kind: \"too_long\", field: \"{}\" }};",
        f.name,
        held(elem, "__ev"),
        f.key
    );
    let mut b = format!("{ind}if __wire == {wire} {{\n");
    b.push_str(&pb_elem_read(elem, &f.key, "__n", bad, &format!("{ind}    ")));
    b.push_str(&format!("{ind}    {push}\n"));
    if wire != 2 {
        b.push_str(&format!("{ind}}} else if __wire == 2 {{\n"));
        b.push_str(&format!("{ind}    let __pe0 = __api_pb_vend(__b, __p);\n"));
        b.push_str(&format!("{ind}    if __pe0 < 0 {{ {bad} }}\n"));
        b.push_str(&format!("{ind}    let __pl = __api_pb_vval(__b, __p, __pe0);\n"));
        b.push_str(&format!("{ind}    if __pl < 0 || __pe0 + __pl > __n {{ {bad} }}\n"));
        b.push_str(&format!("{ind}    let __pe = __pe0 + __pl;\n"));
        b.push_str(&format!("{ind}    __p = __pe0;\n"));
        b.push_str(&format!("{ind}    while __p < __pe {{\n"));
        b.push_str(&pb_elem_read(elem, &f.key, "__pe", bad, &format!("{ind}        ")));
        b.push_str(&format!("{ind}        {push}\n"));
        b.push_str(&format!("{ind}    }}\n"));
    }
    b.push_str(&format!(
        "{ind}}} else {{ fail JsonError {{ kind: \"wrong_type\", field: \"{}\" }}; }}\n",
        f.key
    ));
    b
}

/// `__api_pb_encode_<T>`: every field is written, in number order, whether
/// or not it equals its default (the `.proto` marks every scalar `optional`,
/// so a zero is a value).
fn generate_pb_emit_src(t: &JsonType) -> String {
    let mut b = String::new();
    b.push_str(&format!("fn __api_pb_encode_{}(__v: {}) -> Bytes {{\n", t.name, t.name));
    b.push_str("    let mut __o = std::bytes::from_string(\"\");\n");
    for f in &t.fields {
        let v = format!("__v.{}", f.name);
        let put = match &f.kind {
            FieldKind::Scalar(ScalarTy::Str) => format!("__api_pb_put_str({}, {v})", f.pb_no),
            FieldKind::Scalar(ScalarTy::Float) => format!("__api_pb_put_f64({}, {v})", f.pb_no),
            FieldKind::Scalar(ScalarTy::Bool) => format!("__api_pb_put_bool({}, {v})", f.pb_no),
            FieldKind::Scalar(ScalarTy::Decimal(_)) => format!("__api_pb_put_str({}, to_string({v}))", f.pb_no),
            FieldKind::Scalar(ScalarTy::Int) => match &f.conv {
                Some(c) => format!("__api_pb_put_int({}, {})", f.pb_no, c.to_int(&v)),
                None => format!("__api_pb_put_int({}, {v})", f.pb_no),
            },
            FieldKind::Nested(tn) => format!("__api_pb_put_msg({}, __api_pb_encode_{tn}({v}))", f.pb_no),
            FieldKind::Enum { name, unit_only: true } => format!("__api_pb_put_int({}, __api_pb_enum_no_{name}({v}))", f.pb_no),
            FieldKind::Enum { name, unit_only: false } => {
                format!("__api_pb_put_msg({}, __api_pb_encode_{name}({v}))", f.pb_no)
            }
            FieldKind::Time => format!("__api_pb_put_int({}, std::time::nanos({v}))", f.pb_no),
            // a wire map: `map<string, T>`, each entry an occurrence of a
            // message of its key (1) and the whole record (2)
            FieldKind::List { elem: Elem::Nested(tn), key: Some(k), .. } => {
                b.push_str(&format!("    for __x in {v} {{\n"));
                b.push_str(&format!(
                    "        __o = std::bytes::concat(__o, __api_pb_put_msg({}, std::bytes::concat(__api_pb_put_str(1, __x.{k}), __api_pb_put_msg(2, __api_pb_encode_{tn}(__x)))));\n",
                    f.pb_no
                ));
                b.push_str("    }\n");
                continue;
            }
            FieldKind::List { elem, elem_ty, .. } => {
                // each element its own field occurrence, which a proto3
                // parser reads as it reads the packed form
                b.push_str(&format!("    for __x in {v} {{\n"));
                b.push_str(&elem_bind(elem, elem_ty, "        "));
                b.push_str(&format!("        __o = std::bytes::concat(__o, {});\n", pb_elem_put(elem, f.pb_no, "__xe")));
                b.push_str("    }\n");
                continue;
            }
        };
        b.push_str(&format!("    __o = std::bytes::concat(__o, {put});\n"));
    }
    b.push_str("    return __o;\n}\n");
    b
}

/// The field occurrence `no` that writes the list element `v`.
fn pb_elem_put(elem: &Elem, no: u32, v: &str) -> String {
    match elem {
        Elem::Scalar(ScalarTy::Str, _) => format!("__api_pb_put_str({no}, {v})"),
        Elem::Scalar(ScalarTy::Float, _) => format!("__api_pb_put_f64({no}, {v})"),
        Elem::Scalar(ScalarTy::Bool, _) => format!("__api_pb_put_bool({no}, {v})"),
        Elem::Scalar(ScalarTy::Int, Some(c)) => format!("__api_pb_put_int({no}, {})", c.to_int(v)),
        Elem::Scalar(ScalarTy::Int, None) => format!("__api_pb_put_int({no}, {v})"),
        Elem::Scalar(ScalarTy::Decimal(_), _) => format!("__api_pb_put_str({no}, to_string({v}))"),
        Elem::Time => format!("__api_pb_put_int({no}, std::time::nanos({v}))"),
        Elem::Nested(tn) => format!("__api_pb_put_msg({no}, __api_pb_encode_{tn}({v}))"),
        Elem::Enum { name, unit_only: true } => format!("__api_pb_put_int({no}, __api_pb_enum_no_{name}({v}))"),
        Elem::Enum { name, unit_only: false } => format!("__api_pb_put_msg({no}, __api_pb_encode_{name}({v}))"),
    }
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
            Ok(generated) => program.items.extend(mark_synthetic(generated.items)),
            // The generator emits well-formed source; a parse failure is a
            // generator bug, not user error. Say so on stderr and leave the
            // program unchanged so the (un-rewritten) call surfaces a normal
            // diagnostic.
            Err(ds) => {
                eprintln!(
                    "json_gen: generated parser did not parse: {}",
                    ds.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; ")
                );
                return;
            }
        }
    }

    let names: HashSet<String> = types.iter().map(|t| t.name.clone()).collect();
    // `T::from_json(s)` / `T::to_json(o)` with T a generated type ->
    // `__json_parse_T(s)` / `__json_to_json_T(o)`.
    rewrite_items(&mut program.items, &mut |e: &mut Expr| {
        let Expr::Call { callee, args, span, .. } = e else { return };
        if args.len() != 1 {
            return;
        }
        let Expr::Path(qn) = callee.as_ref() else { return };
        if qn.segments.len() != 2 || !names.contains(&qn.segments[0].name) {
            return;
        }
        let prefix = match qn.segments[1].name.as_str() {
            "from_json" => "__json_parse_",
            "to_json" => "__json_to_json_",
            _ => return,
        };
        let fname = format!("{}{}", prefix, qn.segments[0].name);
        *callee = Box::new(Expr::Ident(Ident::new(fname, *span)));
    });
}

/// Mark the generated `JsonError` as synthesized. The generated fns
/// carry their marker in their names (`__json_parse_`, `__json_to_json_`,
/// `__api_decode_`, `__api_encode_`); `JsonError` is a name a program
/// may declare itself, so it carries a flag instead.
fn mark_synthetic(mut items: Vec<TopDecl>) -> Vec<TopDecl> {
    for item in &mut items {
        if let TopDecl::Type(t) = item {
            t.synthetic = true;
        }
    }
    items
}

// ---- `T::from_json(s)` -> `__json_parse_T(s)` rewrite (full expr walk) ----

/// Every expression a program evaluates: the bodies, and the initializers
/// (a `params` field's, a parameter's default, a struct field's default, a
/// `const`'s), where a call is as much a call as in a body.
fn rewrite_items(items: &mut [TopDecl], visit: &mut dyn FnMut(&mut Expr)) {
    fn defaults(ps: &mut [crate::ast::Param], visit: &mut dyn FnMut(&mut Expr)) {
        for p in ps {
            if let Some(d) = &mut p.default {
                rewrite_expr(d, visit);
            }
        }
    }
    fn type_defaults(t: &mut crate::ast::TypeDecl, visit: &mut dyn FnMut(&mut Expr)) {
        if let crate::ast::TypeDeclBody::Struct(fields) = &mut t.body {
            for f in fields {
                if let Some(d) = &mut f.default {
                    rewrite_expr(d, visit);
                }
            }
        }
    }
    for item in items {
        match item {
            TopDecl::Fn(f) => {
                defaults(&mut f.params, visit);
                rewrite_block(&mut f.body, visit);
            }
            TopDecl::Type(t) => type_defaults(t, visit),
            TopDecl::Const(c) => rewrite_expr(&mut c.value, visit),
            TopDecl::Locus(l) => {
                for m in &mut l.members {
                    match m {
                        LocusMember::Lifecycle(lc) => {
                            defaults(&mut lc.params, visit);
                            rewrite_block(&mut lc.body, visit);
                        }
                        LocusMember::Mode(md) => {
                            defaults(&mut md.params, visit);
                            rewrite_block(&mut md.body, visit);
                        }
                        LocusMember::Fn(fd) => {
                            defaults(&mut fd.params, visit);
                            rewrite_block(&mut fd.body, visit);
                        }
                        LocusMember::Params(pb) => {
                            for p in &mut pb.params {
                                if let crate::ast::ParamInit::Value(e) = &mut p.init {
                                    rewrite_expr(e, visit);
                                }
                            }
                        }
                        LocusMember::Const(c) => rewrite_expr(&mut c.value, visit),
                        LocusMember::Type(t) => type_defaults(t, visit),
                        _ => {}
                    }
                }
            }
            TopDecl::Module(m) => rewrite_items(&mut m.items, visit),
            _ => {}
        }
    }
}

fn rewrite_block(b: &mut Block, visit: &mut dyn FnMut(&mut Expr)) {
    for s in &mut b.stmts {
        rewrite_stmt(s, visit);
    }
    if let Some(t) = &mut b.tail {
        rewrite_expr(t, visit);
    }
}

fn rewrite_stmt(s: &mut Stmt, visit: &mut dyn FnMut(&mut Expr)) {
    match s {
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => rewrite_expr(value, visit),
        Stmt::Assign { target, value, .. } => {
            rewrite_expr(value, visit);
            for seg in &mut target.tail {
                if let crate::ast::LValueSeg::Index(e) = seg {
                    rewrite_expr(e, visit);
                }
            }
        }
        Stmt::If(if_stmt) => rewrite_if(if_stmt, visit),
        Stmt::Match(m) => rewrite_match(m, visit),
        Stmt::For { iter, body, .. } => {
            rewrite_expr(iter, visit);
            rewrite_block(body, visit);
        }
        Stmt::While { cond, body, .. } => {
            rewrite_expr(cond, visit);
            rewrite_block(body, visit);
        }
        Stmt::Return(Some(e), _) => rewrite_expr(e, visit),
        Stmt::Fail { value, .. } => rewrite_expr(value, visit),
        Stmt::Send { subject, value, .. } => {
            rewrite_expr(subject, visit);
            rewrite_expr(value, visit);
        }
        Stmt::Block(b) => rewrite_block(b, visit),
        Stmt::Recovery { args, .. } => {
            for a in args {
                rewrite_expr(a, visit);
            }
        }
        Stmt::Violate { payload, .. } => {
            if let Some(p) = payload {
                rewrite_expr(p, visit);
            }
        }
        Stmt::ShmWrite { max, body, .. } => {
            rewrite_expr(max, visit);
            rewrite_block(body, visit);
        }
        Stmt::Expr(e) => rewrite_expr(e, visit),
        Stmt::Return(None, _)
        | Stmt::Break(_)
        | Stmt::Continue(_)
        | Stmt::Yield(_)
        | Stmt::Terminate(_)
        | Stmt::Reperspective { .. } => {}
    }
}

fn rewrite_if(if_stmt: &mut IfStmt, visit: &mut dyn FnMut(&mut Expr)) {
    rewrite_expr(&mut if_stmt.cond, visit);
    rewrite_block(&mut if_stmt.then_block, visit);
    if let Some(eb) = &mut if_stmt.else_block {
        match eb.as_mut() {
            ElseBranch::Else(b) => rewrite_block(b, visit),
            ElseBranch::ElseIf(inner) => rewrite_if(inner, visit),
        }
    }
}

fn rewrite_match(m: &mut MatchStmt, visit: &mut dyn FnMut(&mut Expr)) {
    rewrite_expr(&mut m.scrutinee, visit);
    for arm in &mut m.arms {
        match &mut arm.body {
            MatchArmBody::Block(b) => rewrite_block(b, visit),
            MatchArmBody::Expr(e) => rewrite_expr(e, visit),
        }
    }
}

fn rewrite_expr(e: &mut Expr, visit: &mut dyn FnMut(&mut Expr)) {
    match e {
        Expr::Call { callee, args, .. } => {
            rewrite_expr(callee, visit);
            for a in args.iter_mut() {
                rewrite_expr(a, visit);
            }
            visit(e);
        }
        Expr::Binary { left, right, .. } => {
            rewrite_expr(left, visit);
            rewrite_expr(right, visit);
        }
        Expr::Unary { operand, .. } => rewrite_expr(operand, visit),
        Expr::Field { receiver, .. } => rewrite_expr(receiver, visit),
        Expr::Index { receiver, index, .. } => {
            rewrite_expr(receiver, visit);
            rewrite_expr(index, visit);
        }
        Expr::Path2 { receiver, .. } => rewrite_expr(receiver, visit),
        Expr::Tuple(es, _) | Expr::Array(es, _) => {
            for x in es {
                rewrite_expr(x, visit);
            }
        }
        Expr::Struct { inits, .. } => {
            for i in inits {
                rewrite_expr(&mut i.value, visit);
            }
        }
        Expr::Block(b) => rewrite_block(b, visit),
        Expr::If(if_stmt) => rewrite_if(if_stmt, visit),
        Expr::Match(m) => rewrite_match(m, visit),
        Expr::Sum(inner, _) | Expr::Prod(inner, _) => rewrite_expr(inner, visit),
        Expr::Approx { left, right, tolerance, .. } => {
            rewrite_expr(left, visit);
            rewrite_expr(right, visit);
            rewrite_expr(tolerance, visit);
        }
        Expr::Range { lo, hi, .. } => {
            rewrite_expr(lo, visit);
            rewrite_expr(hi, visit);
        }
        Expr::ArrayRepeat { val, .. } => rewrite_expr(val, visit),
        Expr::Or { inner, disposition, .. } => {
            rewrite_expr(inner, visit);
            if let OrDisposition::Substitute(s) = disposition {
                rewrite_expr(s, visit);
            }
        }
        Expr::Literal(..) | Expr::Ident(_) | Expr::Path(_) | Expr::KwSelf(_) => {}
    }
}


// ---- GH #1530 step 0: the api codec, called by a program ----

/// The types a program asks the api codec of (spec/api.md § Codecs):
/// An imported type's codec call, `alias::T::api_encode(v)`, spelled by the
/// name its declaration was merged under (`__lib_…_T::api_encode(v)`), the
/// way a construction through an import is: `renames` is the build's
/// cross-seed table (`alias::Name` → merged). A path the table does not
/// know is left as written, for the checker to refuse.
pub fn canonicalize_api_codec_paths(programs: &mut [&mut Program], renames: &[(Vec<String>, String)]) {
    if renames.is_empty() {
        return;
    }
    for p in programs.iter_mut() {
        rewrite_items(&mut p.items, &mut |e: &mut Expr| {
            let Expr::Call { callee, args, .. } = e else { return };
            if args.len() != 1 {
                return;
            }
            let Expr::Path(qn) = callee.as_mut() else { return };
            let n = qn.segments.len();
            if n < 3 || !matches!(qn.segments[n - 1].name.as_str(), "api_decode" | "api_encode") {
                return;
            }
            let head: Vec<String> = qn.segments[..n - 1].iter().map(|s| s.name.clone()).collect();
            let Some((_, merged)) = renames.iter().find(|(k, _)| *k == head) else { return };
            let span = qn.segments[n - 2].span;
            let m = qn.segments[n - 1].clone();
            qn.segments = vec![Ident::new(merged.clone(), span), m];
        });
    }
}

/// `T::api_decode(text)` and `T::api_encode(value)`, each type once, with the
/// index of the first program that asks.
pub fn api_codec_targets(programs: &mut [&mut Program]) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    for (i, p) in programs.iter_mut().enumerate() {
        rewrite_items(&mut p.items, &mut |e: &mut Expr| {
            let Expr::Call { callee, args, .. } = e else { return };
            if args.len() != 1 {
                return;
            }
            let Expr::Path(qn) = callee.as_ref() else { return };
            if let [t, m] = qn.segments.as_slice() {
                if matches!(m.name.as_str(), "api_decode" | "api_encode") && !out.iter().any(|(_, n)| *n == t.name) {
                    out.push((i, t.name.clone()));
                }
            }
        });
    }
    out
}

/// Rewrite `T::api_decode(text)` to `__api_decode_T(text)` and
/// `T::api_encode(v)` to `__api_encode_T(v)` for each `T` in `carried`, the
/// codecs `generate_rpc_codecs` wrote; an enum's decoder (one of `enums`)
/// names the field it reads, and a whole value's is `payload`, as a row's
/// is. A call of another type is left as written, for the checker to refuse.
pub fn rewrite_api_codec_calls(programs: &mut [&mut Program], carried: &HashSet<String>, enums: &HashSet<String>) {
    for p in programs.iter_mut() {
        rewrite_items(&mut p.items, &mut |e: &mut Expr| {
            let Expr::Call { callee, args, span, .. } = e else { return };
            if args.len() != 1 {
                return;
            }
            let Expr::Path(qn) = callee.as_ref() else { return };
            let [t, m] = qn.segments.as_slice() else { return };
            if !carried.contains(&t.name) {
                return;
            }
            let prefix = match m.name.as_str() {
                "api_decode" => "__api_decode_",
                "api_encode" => "__api_encode_",
                _ => return,
            };
            let fname = format!("{prefix}{}", t.name);
            if prefix == "__api_decode_" && enums.contains(&t.name) {
                args.push(Expr::Literal(Literal::String("payload".to_string()), *span));
            }
            *callee = Box::new(Expr::Ident(Ident::new(fname, *span)));
        });
    }
}

// ---- GH #1417: codecs for a surface's type set --------------------------

/// GH #1417 (R2a): the codecs of a surface's rows: strict decoders
/// `__api_decode_<T>` and encoders `__api_encode_<T>` for every struct
/// named in `names`, looked up across the bundle's programs, into
/// `programs[main_idx]`, whose fields are scalars or structs of the same
/// set (the surface laws already refused anything else; callers pass the
/// transitive closure). A field whose type is a scalar of the unit
/// dialect (an identity, a range, a quantity) is read and written as its
/// `Int`, and one whose type is a plain alias as what the alias stands for
/// (spec/api.md § Codecs). `builtins` are the builtin records in `names`
/// (`IndexError`, …), each with the primitive fields the contract shape
/// renders: no declaration carries them, so the caller names them.
pub fn generate_rpc_codecs(
    programs: &mut [&mut Program],
    main_idx: usize,
    names: &[String],
    builtins: &[(String, Vec<(String, PrimType)>)],
) {
    generate_codecs(programs, main_idx, names, true, builtins, false);
}

/// GH #1417 (R8b): the protobuf codecs of the same type set, beside the JSON
/// ones: `__api_pb_decode_<T>(Bytes) -> T fallible(JsonError)` and
/// `__api_pb_encode_<T>(T) -> Bytes`, proto3's wire format with the field
/// numbers of the struct's declaration order (spec/api.md § gRPC). The
/// helpers they call are the gRPC transport's source (`api_grpc.hl`), so a
/// program joins these only when it serves over `grpc::Rpc`.
pub fn generate_rpc_pb_codecs(
    programs: &mut [&mut Program],
    main_idx: usize,
    names: &[String],
    builtins: &[(String, Vec<(String, PrimType)>)],
) {
    generate_codecs(programs, main_idx, names, true, builtins, true);
}

/// The plain aliases a bundle declares (`type Count = Int;`), by name.
fn collect_aliases(items: &[TopDecl], out: &mut std::collections::BTreeMap<String, TypeExpr>) {
    for item in items {
        match item {
            TopDecl::Type(td) => {
                if let TypeDeclBody::Alias(t) = &td.body {
                    out.insert(td.name.name.clone(), t.clone());
                }
            }
            TopDecl::Module(m) => collect_aliases(&m.items, out),
            _ => {}
        }
    }
}

/// What an alias chain ends at: `te` when it names no alias.
fn through_aliases(te: &TypeExpr, aliases: &std::collections::BTreeMap<String, TypeExpr>) -> TypeExpr {
    let mut cur = te.clone();
    for _ in 0..=aliases.len() {
        match named_single(&cur).and_then(|n| aliases.get(n)) {
            Some(next) => cur = next.clone(),
            None => break,
        }
    }
    cur
}

fn generate_codecs(
    programs: &mut [&mut Program],
    main_idx: usize,
    names: &[String],
    units: bool,
    builtins: &[(String, Vec<(String, PrimType)>)],
    protobuf: bool,
) {
    let wanted: HashSet<String> = names.iter().cloned().collect();
    let mut types: Vec<JsonType> = Vec::new();
    let mut have_jsonerror = false;
    let mut existing_fns: HashSet<String> = HashSet::new();
    let mut declared_types: HashSet<String> = HashSet::new();
    let scalars = if units {
        let ro: Vec<&Program> = programs.iter().map(|p| &**p).collect();
        scalar_convs(&ro)
    } else {
        std::collections::BTreeMap::new()
    };
    let mut aliases = std::collections::BTreeMap::new();
    if units {
        for p in programs.iter() {
            collect_aliases(&p.items, &mut aliases);
        }
    }
    let mut enum_kinds = std::collections::BTreeMap::new();
    for p in programs.iter() {
        collect_enum_kinds(&p.items, &mut enum_kinds);
    }
    let mut enums: Vec<JsonEnum> = Vec::new();
    for p in programs.iter() {
        collect_api_types(&p.items, &wanted, &scalars, &aliases, &enum_kinds, &mut types);
        collect_api_enums(&p.items, &wanted, &scalars, &aliases, &enum_kinds, &mut enums);
        walk_fns(&p.items, &mut |name| {
            existing_fns.insert(name.to_string());
        });
        if items_declare_type(&p.items, "JsonError") {
            have_jsonerror = true;
        }
        walk_types(&p.items, &mut |name| {
            declared_types.insert(name.to_string());
        });
    }
    for (name, fields) in builtins {
        let jfields: Option<Vec<JsonField>> = fields
            .iter()
            .enumerate()
            .map(|(at, (f, p))| {
                scalar_of(&TypeExpr::Primitive(*p, Span::new(0, 0))).map(|s| JsonField {
                    name: f.clone(),
                    key: f.clone(),
                    kind: FieldKind::Scalar(s),
                    default_src: None,
                    conv: None,
                    pb_no: at as u32 + 1,
                    presence: Presence::No,
                })
            })
            .collect();
        if let (Some(fields), false) = (jfields, types.iter().any(|t| &t.name == name)) {
            types.push(JsonType { name: name.clone(), fields, lifted: None, lifted_flag: None });
        }
    }
    let mut src = String::new();
    if !have_jsonerror && !protobuf {
        src.push_str("type JsonError { kind: String; field: String; }\n");
    }
    // the zeros an absent optional record holds, made only where a field
    // is optional (every record of the set: a zero is built of its fields'
    // zeros)
    let zeros = types.iter().any(|t| t.fields.iter().any(|f| matches!(f.presence, Presence::Tracked(_))));
    if zeros {
        for t in &types {
            if existing_fns.insert(format!("{ZERO}{}", t.name)) {
                src.push_str(&generate_zero_src(t));
            }
            let reader = if protobuf { format!("__api_pb_optional_{}", t.name) } else { format!("__api_optional_{}", t.name) };
            if existing_fns.insert(reader) {
                src.push_str(&generate_optional_src(t, protobuf));
            }
        }
        for e in &enums {
            let reader = if protobuf { format!("__api_pb_optional_{}", e.name) } else { format!("__api_optional_{}", e.name) };
            if existing_fns.insert(reader) {
                src.push_str(&generate_enum_zero_src(e, protobuf));
            }
        }
    }
    for t in &types {
        // the holders of the record's lists, which both codecs decode into
        for f in &t.fields {
            if matches!(f.kind, FieldKind::List { .. }) && declared_types.insert(holder_name(&t.name, &f.name)) {
                src.push_str(&holder_src(t, &f.name));
            }
        }
        if protobuf {
            if !existing_fns.contains(&format!("__api_pb_decode_{}", t.name)) {
                src.push_str(&generate_pb_parser_src(t));
            }
            if !existing_fns.contains(&format!("__api_pb_encode_{}", t.name)) {
                src.push_str(&generate_pb_emit_src(t));
            }
            continue;
        }
        if !existing_fns.contains(&format!("__api_decode_{}", t.name)) {
            src.push_str(&generate_parser_src_mode(t, "__api_decode_", true));
        }
        if !existing_fns.contains(&format!("__api_encode_{}", t.name)) {
            src.push_str(&generate_emit_src_mode(t, "__api_encode_"));
        }
    }
    // the enums' codecs, and for protobuf the records a variant of several
    // values crosses as, with theirs
    for e in &enums {
        if protobuf {
            for (decl, rec) in variant_records(e) {
                if declared_types.insert(rec.name.clone()) {
                    src.push_str(&decl);
                }
                if !existing_fns.contains(&format!("__api_pb_decode_{}", rec.name)) {
                    src.push_str(&generate_pb_parser_src(&rec));
                    src.push_str(&generate_pb_emit_src(&rec));
                }
            }
            let probe = if e.unit_only() { format!("__api_pb_enum_{}", e.name) } else { format!("__api_pb_decode_{}", e.name) };
            if !existing_fns.contains(&probe) {
                src.push_str(&generate_enum_pb_src(e));
            }
            continue;
        }
        if !existing_fns.contains(&format!("__api_decode_{}", e.name)) {
            src.push_str(&generate_enum_json_src(e));
        }
    }
    // the wire maps' value codecs: each (record, key) a map is keyed by
    let mut lifted: Vec<(String, String)> = Vec::new();
    for t in &types {
        for f in &t.fields {
            if let FieldKind::List { elem: Elem::Nested(tn), key: Some(k), .. } = &f.kind {
                if !lifted.contains(&(tn.clone(), k.clone())) {
                    lifted.push((tn.clone(), k.clone()));
                }
            }
        }
    }
    for (tn, k) in &lifted {
        let Some(t) = types.iter().find(|t| &t.name == tn) else { continue };
        if protobuf {
            if existing_fns.insert(format!("__api_pb_entry_{tn}__by_{k}")) {
                let key_no = t.fields.iter().find(|f| &f.name == k).map_or(1, |f| f.pb_no);
                let flag = t
                    .fields
                    .iter()
                    .find(|f| matches!(&f.presence, Presence::Flag(of) if of == k))
                    .map(|f| f.name.clone());
                src.push_str(&generate_pb_entry_src(tn, k, key_no, flag.as_deref()));
            }
            continue;
        }
        // the key and its presence flag, if it has one, are the map's
        let flag = t
            .fields
            .iter()
            .find(|f| matches!(&f.presence, Presence::Flag(of) if of == k))
            .map(|f| f.name.clone());
        let value = JsonType {
            name: t.name.clone(),
            fields: t.fields.iter().filter(|f| &f.name != k && Some(&f.name) != flag.as_ref()).cloned().collect(),
            lifted: Some(k.clone()),
            lifted_flag: flag,
        };
        if existing_fns.insert(format!("__api_decode_{tn}__by_{k}")) {
            src.push_str(&generate_parser_src_mode(&value, "__api_decode_", true));
            src.push_str(&generate_emit_src_mode(&value, "__api_encode_"));
        }
    }
    if src.trim().is_empty() {
        return;
    }
    match parse_source(&src) {
        Ok(generated) => programs[main_idx].items.extend(mark_synthetic(generated.items)),
        Err(ds) => eprintln!(
            "json_gen: generated api codec did not parse: {}",
            ds.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; ")
        ),
    }
}

// ---- GH #1530 step 0: the codecs of an enum ------------------------------

/// An enum a surface's rows carry: its variants, each with the values it
/// holds (what a list element of their type is, and its spelling).
struct JsonEnum {
    name: String,
    variants: Vec<(String, Vec<(Elem, String)>)>,
}

impl JsonEnum {
    fn unit_only(&self) -> bool {
        self.variants.iter().all(|(_, v)| v.is_empty())
    }
}

/// Every enum a bundle declares, by name: whether it is payload-free.
fn collect_enum_kinds(items: &[TopDecl], out: &mut std::collections::BTreeMap<String, bool>) {
    for item in items {
        match item {
            TopDecl::Type(td) => {
                if let TypeDeclBody::Enum(vs) = &td.body {
                    out.insert(td.name.name.clone(), vs.iter().all(|v| v.fields.is_empty()));
                }
            }
            TopDecl::Module(m) => collect_enum_kinds(&m.items, out),
            _ => {}
        }
    }
}

/// The enums of the named set (the surface laws already refused a value a
/// codec does not carry: a list, an enum with payloads).
fn collect_api_enums(
    items: &[TopDecl],
    wanted: &HashSet<String>,
    scalars: &std::collections::BTreeMap<String, Conv>,
    aliases: &std::collections::BTreeMap<String, TypeExpr>,
    enums: &std::collections::BTreeMap<String, bool>,
    out: &mut Vec<JsonEnum>,
) {
    for item in items {
        match item {
            TopDecl::Type(td) => {
                let TypeDeclBody::Enum(vs) = &td.body else { continue };
                if !wanted.contains(&td.name.name) || out.iter().any(|e| e.name == td.name.name) {
                    continue;
                }
                let mut variants = Vec::new();
                for v in vs {
                    let values: Option<Vec<(Elem, String)>> =
                        v.fields.iter().map(|t| elem_of(t, scalars, aliases, enums)).collect();
                    let Some(values) = values else { continue };
                    variants.push((v.name.name.clone(), values));
                }
                out.push(JsonEnum { name: td.name.name.clone(), variants });
            }
            TopDecl::Module(m) => collect_api_enums(&m.items, wanted, scalars, aliases, enums, out),
            _ => {}
        }
    }
}

/// The key expression a value's reader names in a refusal: the field the
/// enum is read for, which its decoder takes as `__k` (spliced into the
/// readers' string literal of the key).
const RUNTIME_KEY: &str = "\" + __k + \"";

/// The name of the reader of value `i` of variant `v` of `e` from JSON.
fn json_value_reader(e: &str, v: &str, i: usize) -> String {
    format!("__api_jv_{e}_{v}_{i}")
}

/// The record a variant with several values crosses protobuf as.
fn variant_record(e: &str, v: &str) -> String {
    format!("__ApiVariant_{e}_{v}")
}

/// `__api_decode_<E>(text, key)` and `__api_encode_<E>(v)`, the JSON codec
/// (spec/api.md § Codecs): a payload-free variant is its name as a string;
/// a variant with one value `{"<Variant>": <value>}`, with several
/// `{"<Variant>": [<value>, ...]}`, exactly as many; anything else, an
/// unknown variant among it, is `wrong_type` for the field read.
fn generate_enum_json_src(e: &JsonEnum) -> String {
    let n = &e.name;
    let wrong = "fail JsonError { kind: \"wrong_type\", field: __k };";
    let mut b = String::new();
    // one reader per value: a JSON value span of `__src` read as its type
    for (v, values) in &e.variants {
        for (i, (elem, ty)) in values.iter().enumerate() {
            b.push_str(&format!(
                "fn {}(__src: String, __es: Int, __ee0: Int, __k: String) -> {ty} fallible(JsonError) {{\n",
                json_value_reader(n, v, i)
            ));
            b.push_str("    let mut __ee = __ee0;\n");
            b.push_str("    while __ee > __es {\n");
            b.push_str("        let __tc = std::str::byte_at_unchecked(__src, __ee - 1);\n");
            b.push_str("        if __tc == 32 || __tc == 9 || __tc == 10 || __tc == 13 { __ee = __ee - 1; } else { break; }\n");
            b.push_str("    }\n");
            b.push_str(&format!("    if __ee <= __es {{ {wrong} }}\n"));
            b.push_str(&json_elem_read(elem, "__src", RUNTIME_KEY, "__api_decode_", "    "));
            b.push_str("    return __ev;\n}\n");
        }
    }
    b.push_str(&format!("fn __api_decode_{n}(__s: String, __k: String) -> {n} fallible(JsonError) {{\n"));
    b.push_str("    let __t = std::str::trim(__s);\n");
    b.push_str("    let __tn = len(__t);\n");
    b.push_str(&format!("    if __tn < 2 || !std::json::valid(__t) {{ {wrong} }}\n"));
    b.push_str("    let __c = std::str::byte_at_unchecked(__t, 0);\n");
    b.push_str("    if __c == 34 {\n");
    b.push_str("        let __name = std::json::unescape_string(__t[1..(__tn - 1)]);\n");
    for (v, values) in &e.variants {
        if values.is_empty() {
            b.push_str(&format!("        if __name == \"{v}\" {{ return {n}::{v}; }}\n"));
        }
    }
    b.push_str(&format!("        {wrong}\n    }}\n"));
    if !e.unit_only() {
        b.push_str("    if __c == 123 {\n");
        b.push_str("        let __it = std::json::object_first(__t);\n");
        b.push_str(&format!("        if __it.done {{ {wrong} }}\n"));
        b.push_str(&format!("        if !std::json::object_next(__it, __t).done {{ {wrong} }}\n"));
        b.push_str("        let __vs = __it.val_start;\n");
        b.push_str("        let __ve = __it.val_end;\n");
        for (v, values) in &e.variants {
            if values.is_empty() {
                continue;
            }
            b.push_str(&format!("        if std::json::obj_key_eq(__it, __t, \"{v}\") {{\n"));
            if values.len() == 1 {
                b.push_str(&format!("            let __a0 = {}(__t, __vs, __ve, __k) or raise;\n", json_value_reader(n, v, 0)));
                b.push_str(&format!("            return {n}::{v}(__a0);\n"));
            } else {
                b.push_str(&format!("            if std::str::byte_at_unchecked(__t, __vs) != 91 {{ {wrong} }}\n"));
                b.push_str("            let __la = __t[__vs..__ve];\n");
                b.push_str("            let mut __li = std::json::array_first_span(__la);\n");
                let mut args = Vec::new();
                for i in 0..values.len() {
                    b.push_str(&format!("            if __li.done {{ {wrong} }}\n"));
                    b.push_str(&format!(
                        "            let __a{i} = {}(__la, __li.cur_start, __li.cur_end, __k) or raise;\n",
                        json_value_reader(n, v, i)
                    ));
                    b.push_str("            __li = std::json::array_next_span(__li, __la);\n");
                    args.push(format!("__a{i}"));
                }
                b.push_str(&format!("            if !__li.done {{ {wrong} }}\n"));
                b.push_str(&format!("            return {n}::{v}({});\n", args.join(", ")));
            }
            b.push_str("        }\n");
        }
        b.push_str(&format!("        {wrong}\n    }}\n"));
    }
    b.push_str(&format!("    {wrong}\n}}\n"));
    // the encoder
    b.push_str(&format!("fn __api_encode_{n}(__v: {n}) -> String {{\n    match __v {{\n"));
    for (v, values) in &e.variants {
        if values.is_empty() {
            b.push_str(&format!("        {n}::{v} -> {{ return \"\\\"{v}\\\"\"; }},\n"));
            continue;
        }
        let binds: Vec<String> = (0..values.len()).map(|i| format!("__a{i}")).collect();
        b.push_str(&format!("        {n}::{v}({}) -> {{\n", binds.join(", ")));
        let mut parts = Vec::new();
        for (i, (elem, ty)) in values.iter().enumerate() {
            b.push_str(&format!("            let __x{i}: {ty} = __a{i};\n"));
            parts.push(json_elem_write(elem, &format!("__x{i}"), "__api_encode_"));
        }
        let body = if parts.len() == 1 {
            parts[0].clone()
        } else {
            format!("\"[\" + {} + \"]\"", parts.join(" + \",\" + "))
        };
        b.push_str(&format!("            return \"{{\\\"{v}\\\":\" + {body} + \"}}\";\n        }},\n"));
    }
    b.push_str("    }\n}\n");
    b
}

/// An enum's zero and its optional readers (spec/api.md § Codecs): the
/// zero is its first variant with its values' zeros, built only when an
/// optional enum is absent: `__api_zero_<E>()`, `__api_optional_<E>(present,
/// text, key)` and, for an enum with payloads, `__api_pb_optional_<E>(bytes,
/// key)` (a payload-free one reads its number, absent its zero).
fn generate_enum_zero_src(e: &JsonEnum, protobuf: bool) -> String {
    let n = &e.name;
    if protobuf {
        if e.unit_only() {
            return String::new();
        }
        return format!(
            "fn __api_pb_optional_{n}(__raw: Bytes, __k: String) -> {n} fallible(JsonError) {{\n    if len(__raw) > 0 {{ return __api_pb_decode_{n}(__raw, __k) or raise; }}\n    return {ZERO}{n}() or raise;\n}}\n"
        );
    }
    let Some((v, values)) = e.variants.first() else { return String::new() };
    let zero = if values.is_empty() {
        format!("{n}::{v}")
    } else {
        let zs: Vec<String> = values.iter().map(|(elem, _)| elem_zero(elem, "payload")).collect();
        format!("{n}::{v}({})", zs.join(", "))
    };
    format!(
        "fn {ZERO}{n}() -> {n} fallible(JsonError) {{\n    return {zero};\n}}\nfn __api_optional_{n}(__present: Bool, __raw: String, __k: String) -> {n} fallible(JsonError) {{\n    if __present {{ return __api_decode_{n}(__raw, __k) or raise; }}\n    return {ZERO}{n}() or raise;\n}}\n"
    )
}

/// The protobuf codec of an enum. A payload-free enum is its variant's
/// number from 0 in declaration order: `__api_pb_enum_<E>(n, key)` and
/// `__api_pb_enum_no_<E>(v)`, a number no variant has `wrong_type`. An enum
/// with payloads is a message of one `oneof`, its members numbered from 1 in
/// declaration order (a payload-free variant a `bool`, a variant of one value
/// that value's field, one of several the record `__ApiVariant_<E>_<V>` of
/// fields `v0`, `v1`, ...): `__api_pb_encode_<E>(v)` and
/// `__api_pb_decode_<E>(bytes, key)`, the last member a message carries
/// winning, as proto3 merges a `oneof`; a message with none is
/// `missing_field`.
fn generate_enum_pb_src(e: &JsonEnum) -> String {
    let n = &e.name;
    let wrong = "fail JsonError { kind: \"wrong_type\", field: __k };";
    let mut b = String::new();
    if e.unit_only() {
        b.push_str(&format!("fn __api_pb_enum_{n}(__no: Int, __k: String) -> {n} fallible(JsonError) {{\n"));
        for (i, (v, _)) in e.variants.iter().enumerate() {
            b.push_str(&format!("    if __no == {i} {{ return {n}::{v}; }}\n"));
        }
        b.push_str(&format!("    {wrong}\n}}\n"));
        b.push_str(&format!("fn __api_pb_enum_no_{n}(__v: {n}) -> Int {{\n    match __v {{\n"));
        for (i, (v, _)) in e.variants.iter().enumerate() {
            b.push_str(&format!("        {n}::{v} -> {{ return {i}; }},\n"));
        }
        b.push_str("    }\n}\n");
        return b;
    }
    // the encoder
    b.push_str(&format!("fn __api_pb_encode_{n}(__v: {n}) -> Bytes {{\n    match __v {{\n"));
    for (i, (v, values)) in e.variants.iter().enumerate() {
        let no = i as u32 + 1;
        match values.len() {
            0 => b.push_str(&format!("        {n}::{v} -> {{ return __api_pb_put_bool({no}, true); }},\n")),
            1 => {
                let (elem, ty) = &values[0];
                b.push_str(&format!("        {n}::{v}(__a0) -> {{\n"));
                b.push_str(&format!("            let __x0: {ty} = __a0;\n"));
                b.push_str(&format!("            return {};\n        }},\n", pb_elem_put(elem, no, "__x0")));
            }
            k => {
                let binds: Vec<String> = (0..k).map(|i| format!("__a{i}")).collect();
                let inits: Vec<String> = (0..k).map(|i| format!("v{i}: __a{i}")).collect();
                let rec = variant_record(n, v);
                b.push_str(&format!("        {n}::{v}({}) -> {{\n", binds.join(", ")));
                b.push_str(&format!(
                    "            return __api_pb_put_msg({no}, __api_pb_encode_{rec}({rec} {{ {} }}));\n        }},\n",
                    inits.join(", ")
                ));
            }
        }
    }
    b.push_str("    }\n}\n");
    // the decoder: find the last member carried, then read it
    let bad = wrong;
    b.push_str(&format!("fn __api_pb_decode_{n}(__b: Bytes, __k: String) -> {n} fallible(JsonError) {{\n"));
    b.push_str("    let __n = len(__b);\n");
    b.push_str("    let mut __p = 0;\n");
    b.push_str("    let mut __which = 0;\n");
    b.push_str("    let mut __wire0 = 0;\n");
    b.push_str("    let mut __ws = 0;\n");
    b.push_str("    let mut __we = 0;\n");
    // the payload of a message-valued member: its occurrences merged (their
    // contents concatenated, which is the merge) while the member repeats
    b.push_str("    let mut __acc = std::bytes::from_string(\"\");\n");
    b.push_str("    while __p < __n {\n");
    b.push_str("        let __te = __api_pb_vend(__b, __p);\n");
    b.push_str(&format!("        if __te < 0 {{ {bad} }}\n"));
    b.push_str("        let __tag = __api_pb_vval(__b, __p, __te);\n");
    b.push_str("        let __no = __tag >> 3;\n");
    b.push_str("        let __wire = __tag & 7;\n");
    b.push_str("        let __e = __api_pb_skip(__b, __te, __wire);\n");
    b.push_str(&format!("        if __e < 0 {{ {bad} }}\n"));
    b.push_str(&format!("        if __no >= 1 && __no <= {} {{\n", e.variants.len()));
    // the members whose value is a message: a record, or several values
    let merged: Vec<String> = e
        .variants
        .iter()
        .enumerate()
        .filter(|(_, (_, values))| values.len() > 1 || matches!(values.as_slice(), [(Elem::Nested(_) | Elem::Enum { unit_only: false, .. }, _)]))
        .map(|(i, _)| format!("__no == {}", i + 1))
        .collect();
    let merged = if merged.is_empty() { "false".to_string() } else { merged.join(" || ") };
    b.push_str("            if __wire == 2 {\n");
    b.push_str("                let __le = __api_pb_vend(__b, __te);\n");
    b.push_str("                let __ll = __api_pb_vval(__b, __te, __le);\n");
    b.push_str(&format!("                if __le < 0 || __ll < 0 || __le + __ll > __e {{ {bad} }}\n"));
    b.push_str("                let __pl = std::bytes::slice(__b, __le, __le + __ll);\n");
    b.push_str(&format!(
        "                if __no == __which && __wire0 == 2 && ({merged}) {{ __acc = std::bytes::concat(__acc, __pl); }} else {{ __acc = __pl; }}\n"
    ));
    b.push_str("            }\n");
    b.push_str("            __which = __no;\n            __wire0 = __wire;\n            __ws = __te;\n            __we = __e;\n        }\n");
    b.push_str("        __p = __e;\n");
    b.push_str("    }\n");
    b.push_str("    if __which == 0 { fail JsonError { kind: \"missing_field\", field: __k }; }\n");
    for (i, (v, values)) in e.variants.iter().enumerate() {
        let no = i + 1;
        b.push_str(&format!("    if __which == {no} {{\n"));
        match values.len() {
            0 => {
                b.push_str(&format!("        if __wire0 != 0 {{ {wrong} }}\n"));
                b.push_str(&format!("        return {n}::{v};\n"));
            }
            1 => {
                let (elem, _) = &values[0];
                b.push_str(&format!("        if __wire0 != {} {{ {wrong} }}\n", pb_elem_wire(elem)));
                // a message value: its merged occurrences
                match elem {
                    Elem::Nested(tn) => {
                        b.push_str(&format!("        return {n}::{v}(__api_pb_decode_{tn}(__acc) or raise);\n"));
                    }
                    Elem::Enum { name, unit_only: false } => {
                        b.push_str(&format!("        return {n}::{v}(__api_pb_decode_{name}(__acc, __k) or raise);\n"));
                    }
                    _ => {
                        b.push_str("        __p = __ws;\n");
                        b.push_str(&pb_elem_read(elem, RUNTIME_KEY, "__we", bad, "        "));
                        b.push_str(&format!("        return {n}::{v}(__ev);\n"));
                    }
                }
            }
            k => {
                let rec = variant_record(n, v);
                b.push_str(&format!("        if __wire0 != 2 {{ {wrong} }}\n"));
                b.push_str(&format!("        let __r = __api_pb_decode_{rec}(__acc) or raise;\n"));
                let args: Vec<String> = (0..k).map(|i| format!("__r.v{i}")).collect();
                b.push_str(&format!("        return {n}::{v}({});\n", args.join(", ")));
            }
        }
        b.push_str("    }\n");
    }
    b.push_str(&format!("    {wrong}\n}}\n"));
    b
}

/// The records the variants with several values cross protobuf as, with
/// the codec they carry: `type __ApiVariant_<E>_<V> { v0: T0; ... }`.
fn variant_records(e: &JsonEnum) -> Vec<(String, JsonType)> {
    let mut out = Vec::new();
    for (v, values) in &e.variants {
        if values.len() < 2 {
            continue;
        }
        let rec = variant_record(&e.name, v);
        let decl = format!(
            "type {rec} {{ {} }}\n",
            values.iter().enumerate().map(|(i, (_, ty))| format!("v{i}: {ty};")).collect::<Vec<_>>().join(" ")
        );
        let fields = values
            .iter()
            .enumerate()
            .map(|(i, (elem, _))| {
                // the `.proto` declares these plain proto3 fields (implicit
                // presence: a client leaves a zero out), so an absent one is
                // its proto3 zero, never `missing_field`; a record value is a
                // message, whose presence is explicit
                let (kind, conv, default_src) = match elem {
                    Elem::Scalar(s, c) => (FieldKind::Scalar(*s), c.clone(), Some(wire_zero(*s, c, &None))),
                    Elem::Time => (FieldKind::Time, None, Some(EPOCH.to_string())),
                    Elem::Nested(tn) => (FieldKind::Nested(tn.clone()), None, None),
                    Elem::Enum { name, unit_only } => {
                        (FieldKind::Enum { name: name.clone(), unit_only: *unit_only }, None, unit_only.then(|| "0".to_string()))
                    }
                };
                JsonField {
                    name: format!("v{i}"),
                    key: format!("v{i}"),
                    kind,
                    default_src,
                    conv,
                    pb_no: i as u32 + 1,
                    presence: Presence::No,
                }
            })
            .collect();
        out.push((decl, JsonType { name: rec, fields, lifted: None, lifted_flag: None }));
    }
    out
}

fn items_declare_type(items: &[TopDecl], name: &str) -> bool {
    items.iter().any(|i| match i {
        TopDecl::Type(t) => t.name.name == name,
        TopDecl::Module(m) => items_declare_type(&m.items, name),
        _ => false,
    })
}

fn walk_types(items: &[TopDecl], f: &mut dyn FnMut(&str)) {
    for i in items {
        match i {
            TopDecl::Type(d) => f(&d.name.name),
            TopDecl::Module(m) => walk_types(&m.items, f),
            _ => {}
        }
    }
}

fn walk_fns(items: &[TopDecl], f: &mut dyn FnMut(&str)) {
    for i in items {
        match i {
            TopDecl::Fn(d) => f(&d.name.name),
            TopDecl::Module(m) => walk_fns(&m.items, f),
            _ => {}
        }
    }
}

/// What a value of `ty` is to the codec, and the type's spelling: a scalar
/// (a unit-dialect scalar with its conversion to the wire's `Int`, a plain
/// alias as what it stands for) or a record; `None` for anything else (a
/// list among them: a list's element is never a list).
fn elem_of(
    ty: &TypeExpr,
    scalars: &std::collections::BTreeMap<String, Conv>,
    aliases: &std::collections::BTreeMap<String, TypeExpr>,
    enums: &std::collections::BTreeMap<String, bool>,
) -> Option<(Elem, String)> {
    let ty = through_aliases(ty, aliases);
    if let TypeExpr::Primitive(PrimType::Time, _) = ty {
        Some((Elem::Time, "Time".to_string()))
    } else if let TypeExpr::Primitive(PrimType::Duration, _) = ty {
        // the stdlib's `quantity Int in ns`: its count of nanoseconds
        Some((Elem::Scalar(ScalarTy::Int, Some(Conv::Quantity(1, "ns".to_string()))), "Duration".to_string()))
    } else if let Some(s) = scalar_of(&ty) {
        Some((Elem::Scalar(s, None), s.type_name().to_string()))
    } else if let Some((tn, c)) = named_single(&ty).and_then(|tn| scalars.get(tn).map(|c| (tn, c))) {
        // a scalar of the unit dialect: its `Int` on the wire
        Some((Elem::Scalar(ScalarTy::Int, Some(c.clone())), tn.to_string()))
    } else if let Some((tn, unit_only)) = named_single(&ty).and_then(|tn| enums.get(tn).map(|u| (tn, *u))) {
        Some((Elem::Enum { name: tn.to_string(), unit_only }, tn.to_string()))
    } else {
        named_single(&ty).map(|tn| (Elem::Nested(tn.to_string()), tn.to_string()))
    }
}

/// Like `collect_json_types`, but for a named set and with every field
/// admitted (the api surface already classified the closure).
fn collect_api_types(
    items: &[TopDecl],
    wanted: &HashSet<String>,
    scalars: &std::collections::BTreeMap<String, Conv>,
    aliases: &std::collections::BTreeMap<String, TypeExpr>,
    enums: &std::collections::BTreeMap<String, bool>,
    out: &mut Vec<JsonType>,
) {
    for item in items {
        match item {
            TopDecl::Type(td) => {
                if !wanted.contains(&td.name.name) {
                    continue;
                }
                if out.iter().any(|t| t.name == td.name.name) {
                    continue;
                }
                let TypeDeclBody::Struct(fields) = &td.body else { continue };
                let mut jfields = Vec::new();
                for (at, f) in fields.iter().enumerate() {
                    let mut conv = None;
                    // a field typed by a plain alias is what the alias stands for
                    let ty = through_aliases(&f.ty, aliases);
                    let kind = if let Some((elem, cap, exact)) = wire_list(&ty) {
                        // a list: its elements are what a field of their type is
                        let Some((elem, elem_ty)) = elem_of(elem, scalars, aliases, enums) else { continue };
                        // `indexed_by <key>`: a wire map of the records,
                        // keyed by their `<key>` (spec/api.md § Codecs)
                        let key = f.indexed_by.as_ref().map(|k| k.name.clone()).filter(|_| matches!(elem, Elem::Nested(_)));
                        FieldKind::List { elem, elem_ty, cap, exact, key }
                    } else {
                        match elem_of(&ty, scalars, aliases, enums) {
                            Some((Elem::Scalar(s, c), _)) => {
                                conv = c;
                                FieldKind::Scalar(s)
                            }
                            Some((Elem::Time, _)) => FieldKind::Time,
                            Some((Elem::Nested(tn), _)) => FieldKind::Nested(tn),
                            Some((Elem::Enum { name, unit_only }, _)) => FieldKind::Enum { name, unit_only },
                            None => continue,
                        }
                    };
                    // a literal default; a unit scalar's is held as its wire `Int`
                    let default_src = match (&kind, &conv) {
                        (FieldKind::Scalar(_), None) | (FieldKind::Time, _) => f.default.as_ref().and_then(literal_src),
                        (FieldKind::Scalar(_), Some(c)) => f.default.as_ref().and_then(literal_src).map(|d| c.to_int(&d)),
                        _ => None,
                    };
                    jfields.push(JsonField {
                        name: f.name.name.clone(),
                        key: tag_json_key(&f.tag).unwrap_or_else(|| f.name.name.clone()),
                        kind,
                        default_src,
                        conv,
                        pb_no: at as u32 + 1,
                        presence: Presence::No,
                    });
                }
                let is_bool = |te: &TypeExpr| matches!(through_aliases(te, aliases), TypeExpr::Primitive(PrimType::Bool, _));
                for (of, flag) in presence_pairs(fields, &is_bool) {
                    for jf in jfields.iter_mut() {
                        if jf.name == of {
                            jf.presence = Presence::Tracked(flag.to_string());
                        } else if jf.name == flag {
                            jf.presence = Presence::Flag(of.to_string());
                        }
                    }
                }
                out.push(JsonType { name: td.name.name.clone(), fields: jfields, lifted: None, lifted_flag: None });
            }
            TopDecl::Module(m) => collect_api_types(&m.items, wanted, scalars, aliases, enums, out),
            _ => {}
        }
    }
}
