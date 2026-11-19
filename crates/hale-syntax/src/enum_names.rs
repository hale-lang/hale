//! RFC #1995 E1 / E1b: an enum's external names.
//!
//! A variant has one external name (F.53): its `json:` tag's key when it
//! has one, else the name its enum's `@wire_case(...)` gives it, else its
//! declared name. The JSON codecs read it (`json_gen::variant_wire_name`),
//! and so do the four functions the compiler writes for an enum `E`:
//!
//! - `E::name(v) -> String`: `v`'s external name, payloads ignored; a
//!   lossless catch-all (``Other(String) `json:",other"` ``) names the
//!   string it holds.
//! - `E::parse(s) -> E fallible(ParseError)`: the variant `s` names, or
//!   `ParseError { kind: "unknown_variant", input: s }`. With a catch-all
//!   it is not fallible: an unknown name is the catch-all, holding the
//!   string when it is lossless, as the JSON codec reads one.
//! - `E::variants`: a value, `[E; N]`, the variants in declaration order
//!   (a lossless catch-all, which needs a string, left out).
//! - `E::index(v) -> Int`: `v`'s declaration ordinal.
//!
//! `parse` and `variants` apply to an enum whose variants carry no
//! values, but for one lossless catch-all; `name` and `index` to every
//! enum but a generic one. The names are contextual (F.43 discipline): an
//! enum that declares a variant of the name keeps meaning its variant.
//!
//! As `T::from_json` is, each is a type-path call the desugar sequence
//! rewrites to a function it writes (`__enum_name_<E>`, ...), so the
//! checker and lowering see an ordinary function and agree on it. A call
//! the pass does not rewrite (a payload enum's `parse`, `E::variants()`,
//! the wrong arity) is left as written, and the checker refuses it with
//! the reason ([`op_refusal`], [`call_refusal`]).
//!
//! `@wire_case(snake | screaming_snake | kebab | lower)` on the enum's
//! declaration is applied where it is parsed ([`apply_wire_case`]): every
//! variant without a `json:` key of its own takes the cased name as one, so
//! every reader of a variant's wire name reads the one spelling.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::ast::{EnumVariant, Expr, Ident, NodeId, Program, QualifiedName, TopDecl, TypeDeclBody};
use crate::json_gen::{hale_str, json_tag_key, json_tag_options, next_synth_base, rewrite_items, variant_wire_name};

/// The four names, in the order the RFC lists them.
pub const OPS: [&str; 4] = ["name", "parse", "variants", "index"];

/// One of the four functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Op {
    Name,
    Parse,
    Variants,
    Index,
}

impl Op {
    pub fn of(word: &str) -> Option<Op> {
        match word {
            "name" => Some(Op::Name),
            "parse" => Some(Op::Parse),
            "variants" => Some(Op::Variants),
            "index" => Some(Op::Index),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Op::Name => "name",
            Op::Parse => "parse",
            Op::Variants => "variants",
            Op::Index => "index",
        }
    }

    /// The prefix of the function the desugar writes for it.
    fn prefix(self) -> &'static str {
        match self {
            Op::Name => "__enum_name_",
            Op::Parse => "__enum_parse_",
            Op::Variants => "__enum_variants_",
            Op::Index => "__enum_index_",
        }
    }

    /// The function the desugar writes for it on the enum `e`.
    pub fn fn_name(self, e: &str) -> String {
        format!("{}{e}", self.prefix())
    }
}

/// `E::name` for a function the desugar wrote (`__enum_name_E`), the way
/// the program spells it; `None` for any other name.
pub fn generated_display(f: &str) -> Option<String> {
    [Op::Name, Op::Parse, Op::Variants, Op::Index]
        .into_iter()
        .find_map(|op| f.strip_prefix(op.prefix()).filter(|e| !e.is_empty()).map(|e| format!("{e}::{}", op.word())))
}

/// `@wire_case(...)`: how an enum's variants are spelled on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireCase {
    /// `InProgress` -> `in_progress`
    Snake,
    /// `InProgress` -> `IN_PROGRESS`
    ScreamingSnake,
    /// `InProgress` -> `in-progress`
    Kebab,
    /// `InProgress` -> `inprogress`
    Lower,
}

impl WireCase {
    pub const WORDS: [&'static str; 4] = ["snake", "screaming_snake", "kebab", "lower"];

    pub fn of(word: &str) -> Option<WireCase> {
        match word {
            "snake" => Some(WireCase::Snake),
            "screaming_snake" => Some(WireCase::ScreamingSnake),
            "kebab" => Some(WireCase::Kebab),
            "lower" => Some(WireCase::Lower),
            _ => None,
        }
    }

    /// A declared name in this case. Words break at `_` and `-`, before an
    /// upper-case letter that follows a lower-case one or a digit, and
    /// before the last capital of a run followed by a lower-case letter
    /// (`HTTPError` is `http` `error`); a digit stays with its word
    /// (`Http2Error` is `http2` `error`). `lower` is the name in lower case.
    pub fn apply(self, name: &str) -> String {
        match self {
            WireCase::Snake => words(name).iter().map(|w| w.to_lowercase()).collect::<Vec<_>>().join("_"),
            WireCase::ScreamingSnake => words(name).iter().map(|w| w.to_uppercase()).collect::<Vec<_>>().join("_"),
            WireCase::Kebab => words(name).iter().map(|w| w.to_lowercase()).collect::<Vec<_>>().join("-"),
            WireCase::Lower => name.to_lowercase(),
        }
    }
}

/// The words of an identifier ([`WireCase::apply`]).
fn words(name: &str) -> Vec<String> {
    let cs: Vec<char> = name.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for (i, &c) in cs.iter().enumerate() {
        if c == '_' || c == '-' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if c.is_uppercase() && !cur.is_empty() {
            let prev = cs[i - 1];
            let next_lower = cs.get(i + 1).is_some_and(|n| n.is_lowercase());
            if prev.is_lowercase() || prev.is_ascii_digit() || (prev.is_uppercase() && next_lower) {
                out.push(std::mem::take(&mut cur));
            }
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `@wire_case(case)` on an enum: every variant without a `json:` key of
/// its own takes its name in `case` as one, so the codecs and `E::name` /
/// `E::parse` read one spelling. An explicit key wins, and a lossless
/// catch-all keeps none (it writes the string it holds).
pub fn apply_wire_case(variants: &mut [EnumVariant], case: WireCase) {
    for v in variants {
        if v.member || json_tag_key(v.tag.as_deref()).is_some() {
            continue;
        }
        let other = json_tag_options(v.tag.as_deref()).iter().any(|o| o == "other");
        if other && !v.fields.is_empty() {
            continue;
        }
        v.tag = Some(with_json_key(v.tag.as_deref(), &case.apply(&v.name.name)));
    }
}

/// A metadata tag with its `json:` key set to `key`, its options and every
/// other key kept.
fn with_json_key(tag: Option<&str>, key: &str) -> String {
    let Some(t) = tag.filter(|t| !t.trim().is_empty()) else { return format!("json:\"{key}\"") };
    let needle = "json:\"";
    let Some(at) = t.find(needle) else { return format!("{t} json:\"{key}\"") };
    let start = at + needle.len();
    let end = t[start..].find('"').map_or(t.len(), |e| start + e);
    let value = &t[start..end];
    let opts = value.find(',').map_or("", |c| &value[c..]);
    format!("{}{key}{opts}{}", &t[..start], &t[end..])
}

/// A variant as [`op_refusal`] reads it: its name, how many values it
/// carries, and its tag.
pub struct VariantShape<'a> {
    pub name: &'a str,
    pub fields: usize,
    pub tag: Option<&'a str>,
}

impl VariantShape<'_> {
    fn other(&self) -> bool {
        json_tag_options(self.tag).iter().any(|o| o == "other")
    }

    /// The catch-all that holds the string it caught.
    fn lossless(&self) -> bool {
        self.other() && self.fields > 0
    }
}

/// Why `E::<op>` does not apply to the enum `shown` of these variants, or
/// `None` when it does: `parse` and `variants` need a variant that carries
/// no values, but for a lossless catch-all, and the four are not written
/// for a generic enum.
pub fn op_refusal(shown: &str, vs: &[VariantShape<'_>], generic: bool, op: Op) -> Option<String> {
    let w = op.word();
    if generic {
        return Some(format!(
            "`{shown}::{w}`: `{shown}` is generic, and hale can't write `name`, `parse`, `variants` or `index` for a \
             generic enum yet; match on the value"
        ));
    }
    if !matches!(op, Op::Parse | Op::Variants) {
        return None;
    }
    let (what, instead) = if op == Op::Parse {
        ("builds a variant from its name alone", "match on the name and build the variant yourself")
    } else {
        ("lists every value of the enum", "list the values you need yourself")
    };
    if let Some(v) = vs.iter().find(|v| v.fields > 0 && !v.lossless()) {
        let n = v.fields;
        return Some(format!(
            "`{shown}::{w}` {what}, so it applies to an enum whose variants carry no values (and at most one \
             catch-all, `Other(String) `json:\",other\"``), and `{shown}::{}` carries {n} value{}; {instead}",
            v.name,
            if n == 1 { "" } else { "s" }
        ));
    }
    if op == Op::Variants && vs.iter().all(VariantShape::lossless) {
        return Some(format!(
            "`{shown}::variants` lists the variants that carry no values, and `{shown}` has none: its one variant is \
             the catch-all, which needs the string it holds"
        ));
    }
    None
}

/// Why a call of `E::<op>` with `args` arguments (`None`: not called) is
/// refused by its shape alone, or `None`: `variants` is a value, the other
/// three are called with one argument.
pub fn call_refusal(shown: &str, op: Op, args: Option<usize>) -> Option<String> {
    let w = op.word();
    match (op, args) {
        (Op::Variants, None) => None,
        (Op::Variants, Some(_)) => Some(format!(
            "`{shown}::variants` is a value, the array of `{shown}`'s variants in declaration order, not a \
             function: write `{shown}::variants`, without the parentheses"
        )),
        (_, None) => Some(format!(
            "`{shown}::{w}` is a function the compiler writes for the enum, called as `{shown}::{w}({})`; it is no \
             value of its own",
            if op == Op::Parse { "s" } else { "v" }
        )),
        (_, Some(1)) => None,
        (_, Some(n)) => Some(format!(
            "`{shown}::{w}` takes one argument, {}, and this passes {n}",
            if op == Op::Parse {
                "the name, a `String`".to_string()
            } else {
                format!("a `{shown}`")
            }
        )),
    }
}

/// An enum of the bundle, as the pass reads it.
struct EnumDecl {
    /// The program that declares it, where its functions are written.
    program: usize,
    generic: bool,
    variants: Vec<EnumVariant>,
}

impl EnumDecl {
    fn shapes(&self) -> Vec<VariantShape<'_>> {
        self.variants
            .iter()
            .map(|v| VariantShape { name: &v.name.name, fields: v.fields.len(), tag: v.tag.as_deref() })
            .collect()
    }

    fn declares(&self, word: &str) -> bool {
        self.variants.iter().any(|v| v.name.name == word)
    }

    fn applies(&self, name: &str, op: Op) -> bool {
        op_refusal(name, &self.shapes(), self.generic, op).is_none()
    }
}

/// The enum and the function a type path names: `E::<op>`, or an imported
/// one's `alias::E::<op>` through the build's rename table. `None` when the
/// head is no enum of the bundle, the last segment is none of the four, or
/// the enum declares a variant of that name.
fn target(qn: &QualifiedName, enums: &BTreeMap<String, EnumDecl>, renames: &[(Vec<String>, String)]) -> Option<(String, Op)> {
    let n = qn.segments.len();
    if n < 2 {
        return None;
    }
    let op = Op::of(&qn.segments[n - 1].name)?;
    let name = if n == 2 {
        qn.segments[0].name.clone()
    } else {
        let head: Vec<&str> = qn.segments[..n - 1].iter().map(|s| s.name.as_str()).collect();
        renames.iter().find(|(k, _)| k.iter().map(String::as_str).eq(head.iter().copied()))?.1.clone()
    };
    let e = enums.get(&name)?;
    (!e.declares(op.word())).then_some((name, op))
}

/// Write the functions a bundle's programs call (`E::name(v)`,
/// `E::parse(s)`, `E::variants`, `E::index(v)`, each through an import as
/// well) into the program that declares the enum, and rewrite each use to
/// call its function. A use the functions do not apply to is left as
/// written, for the checker to refuse. Idempotent: a rewritten use names
/// no type path, and a function a program already has is not written
/// again.
pub fn generate_enum_names_in(programs: &mut [&mut Program], renames: &[(Vec<String>, String)]) {
    let mut enums: BTreeMap<String, EnumDecl> = BTreeMap::new();
    for (i, p) in programs.iter().enumerate() {
        for d in p.items.iter() {
            let TopDecl::Type(td) = d else { continue };
            let TypeDeclBody::Enum(vs) = &td.body else { continue };
            enums.entry(td.name.name.clone()).or_insert_with(|| EnumDecl {
                program: i,
                generic: !td.generics.is_empty(),
                variants: vs.clone(),
            });
        }
    }
    if enums.is_empty() {
        return;
    }
    let mut wanted: BTreeSet<(String, Op)> = BTreeSet::new();
    for p in programs.iter_mut() {
        rewrite_items(&mut p.items, &mut |e: &mut Expr| match e {
            Expr::Call { callee, args, span, .. } => {
                let Expr::Path(qn) = callee.as_ref() else { return };
                let Some((name, op)) = target(qn, &enums, renames) else { return };
                if call_refusal(&name, op, Some(args.len())).is_some() || !enums[&name].applies(&name, op) {
                    return;
                }
                *callee = Box::new(Expr::Ident(Ident::new(op.fn_name(&name), *span)));
                wanted.insert((name, op));
            }
            Expr::Path(qn) => {
                let Some((name, Op::Variants)) = target(qn, &enums, renames) else { return };
                if !enums[&name].applies(&name, Op::Variants) {
                    return;
                }
                let span = qn.span;
                *e = Expr::Call {
                    callee: Box::new(Expr::Ident(Ident::new(Op::Variants.fn_name(&name), span))),
                    args: Vec::new(),
                    span,
                    id: NodeId::NONE,
                };
                wanted.insert((name, Op::Variants));
            }
            _ => {}
        });
    }
    if wanted.is_empty() {
        return;
    }
    let mut by_program: BTreeMap<usize, String> = BTreeMap::new();
    let mut existing: HashSet<String> = HashSet::new();
    for p in programs.iter() {
        for d in p.items.iter() {
            if let TopDecl::Fn(f) = d {
                existing.insert(f.name.name.clone());
            }
        }
    }
    for (name, op) in &wanted {
        let e = &enums[name];
        if existing.insert(op.fn_name(name)) {
            by_program.entry(e.program).or_default().push_str(&generate(name, e, *op));
        }
    }
    for (i, src) in by_program {
        let base = next_synth_base(programs.iter().map(|p| &**p));
        match crate::parse_source_at(&src, base) {
            Ok(generated) => programs[i].items.extend(generated.items),
            // The generator writes well-formed source; a parse failure is
            // its bug. The rewritten calls then name nothing, which the
            // checker reports.
            Err(ds) => eprintln!(
                "enum_names: generated source did not parse: {}",
                ds.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; ")
            ),
        }
    }
}

/// The source of `E::<op>`'s function.
fn generate(n: &str, e: &EnumDecl, op: Op) -> String {
    let lossless = |v: &EnumVariant| {
        json_tag_options(v.tag.as_deref()).iter().any(|o| o == "other") && !v.fields.is_empty()
    };
    let pattern = |v: &EnumVariant| {
        if v.fields.is_empty() {
            format!("{n}::{}", v.name.name)
        } else {
            format!("{n}::{}({})", v.name.name, vec!["_"; v.fields.len()].join(", "))
        }
    };
    let mut b = String::new();
    match op {
        Op::Name => {
            b.push_str(&format!("fn {}(__v: {n}) -> String {{\n    match __v {{\n", op.fn_name(n)));
            for v in &e.variants {
                if lossless(v) {
                    b.push_str(&format!("        {n}::{}(__s) -> {{ return __s; }},\n", v.name.name));
                } else {
                    b.push_str(&format!("        {} -> {{ return \"{}\"; }},\n", pattern(v), hale_str(&variant_wire_name(v))));
                }
            }
            b.push_str("    }\n}\n");
        }
        Op::Index => {
            b.push_str(&format!("fn {}(__v: {n}) -> Int {{\n    match __v {{\n", op.fn_name(n)));
            for (i, v) in e.variants.iter().enumerate() {
                b.push_str(&format!("        {} -> {{ return {i}; }},\n", pattern(v)));
            }
            b.push_str("    }\n}\n");
        }
        Op::Variants => {
            let vs: Vec<String> = e.variants.iter().filter(|v| !lossless(v)).map(|v| format!("{n}::{}", v.name.name)).collect();
            b.push_str(&format!(
                "fn {}() -> [{n}; {}] {{\n    return [{}];\n}}\n",
                op.fn_name(n),
                vs.len(),
                vs.join(", ")
            ));
        }
        Op::Parse => {
            let other = e.variants.iter().find(|v| json_tag_options(v.tag.as_deref()).iter().any(|o| o == "other"));
            let ret = if other.is_some() { n.to_string() } else { format!("{n} fallible(ParseError)") };
            b.push_str(&format!("fn {}(__s: String) -> {ret} {{\n", op.fn_name(n)));
            for v in e.variants.iter().filter(|v| !lossless(v)) {
                b.push_str(&format!(
                    "    if __s == \"{}\" {{ return {n}::{}; }}\n",
                    hale_str(&variant_wire_name(v)),
                    v.name.name
                ));
            }
            match other {
                Some(v) if lossless(v) => b.push_str(&format!("    return {n}::{}(__s);\n", v.name.name)),
                Some(v) => b.push_str(&format!("    return {n}::{};\n", v.name.name)),
                None => b.push_str("    fail ParseError { kind: \"unknown_variant\", input: __s };\n"),
            }
            b.push_str("}\n");
        }
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cases() {
        let c = |case, n| WireCase::apply(case, n);
        assert_eq!(c(WireCase::Snake, "InProgress"), "in_progress");
        assert_eq!(c(WireCase::Snake, "HTTPError"), "http_error");
        assert_eq!(c(WireCase::Snake, "Http2Error"), "http2_error");
        assert_eq!(c(WireCase::Snake, "already_snake"), "already_snake");
        assert_eq!(c(WireCase::Snake, "A"), "a");
        assert_eq!(c(WireCase::ScreamingSnake, "InProgress"), "IN_PROGRESS");
        assert_eq!(c(WireCase::Kebab, "InProgress"), "in-progress");
        assert_eq!(c(WireCase::Lower, "InProgress"), "inprogress");
    }

    #[test]
    fn json_key_is_set_and_options_kept() {
        assert_eq!(with_json_key(None, "a"), "json:\"a\"");
        assert_eq!(with_json_key(Some("json:\",other\""), "a"), "json:\"a,other\"");
        assert_eq!(with_json_key(Some("db:\"x\""), "a"), "db:\"x\" json:\"a\"");
        assert_eq!(with_json_key(Some("db:\"x\" json:\",other\""), "a"), "db:\"x\" json:\"a,other\"");
    }

    #[test]
    fn generated_names_display_as_written() {
        assert_eq!(generated_display("__enum_parse_Color").as_deref(), Some("Color::parse"));
        assert_eq!(generated_display("__enum_variants_Color").as_deref(), Some("Color::variants"));
        assert_eq!(generated_display("__json_parse_Color"), None);
    }
}
