//! The clients `hale api client` generates from a surface's rows
//! (GH #1417, R8a; spec/api.md § The clients).
//!
//! A generator is a pure function of `surface_doc::ClientModel`: it reads
//! no path, no clock and no environment, so two runs, and two checkouts of
//! one program, generate the same bytes.

pub(crate) mod hale_client;
pub(crate) mod js_client;
pub(crate) mod ts_client;

/// An enum a client declares: its variants, each with its values' schemas.
pub(crate) type ClientEnum = Vec<(String, Vec<hale_types::surfaces::FieldSchema>)>;

/// Every enum the model's shapes name, by name: a client declares each once,
/// beside the records (an enum is inline in the description, so the model's
/// types do not list it).
pub(crate) fn model_enums(model: &hale_types::surface_doc::ClientModel) -> std::collections::BTreeMap<String, ClientEnum> {
    model_shapes(model).0
}

/// Whether the model names a wire map: a client carries the helpers that
/// lift and restore a map's keys only when one does.
pub(crate) fn model_has_map(model: &hale_types::surface_doc::ClientModel) -> bool {
    model_shapes(model).1
}

fn model_shapes(model: &hale_types::surface_doc::ClientModel) -> (std::collections::BTreeMap<String, ClientEnum>, bool) {
    use hale_types::surface_doc::ClientError;
    use hale_types::surfaces::FieldSchema;
    type Out = (std::collections::BTreeMap<String, ClientEnum>, bool);
    fn walk(f: &FieldSchema, out: &mut Out) {
        match f {
            FieldSchema::Enum { name, variants, .. } => {
                if !out.0.contains_key(name) {
                    out.0.insert(name.clone(), variants.clone());
                    for (_, values) in variants {
                        values.iter().for_each(|v| walk(v, out));
                    }
                }
            }
            FieldSchema::List { items, .. } => walk(items, out),
            FieldSchema::Nullable { inner, .. } => walk(inner, out),
            FieldSchema::Map { value, .. } => {
                out.1 = true;
                value.properties.iter().for_each(|(_, v)| walk(v, out));
            }
            _ => {}
        }
    }
    let mut out: Out = (std::collections::BTreeMap::new(), false);
    for t in model.types.values() {
        t.properties.iter().for_each(|(_, f)| walk(f, &mut out));
    }
    for m in &model.members {
        m.request.iter().chain(m.response.iter()).for_each(|f| walk(f, &mut out));
        if let ClientError::Type(e) = &m.error {
            walk(e, &mut out);
        }
    }
    for s in &model.streams {
        s.payload.iter().for_each(|f| walk(f, &mut out));
    }
    out
}

/// The generators' refusal of a shape the codec does not carry: reported,
/// never approximated.
pub(crate) fn unformed(what: &str) -> String {
    format!(
        "the surface names a shape a client cannot express (`{what}`): the JSON codec does not carry it, so the \
         client is not generated rather than approximated"
    )
}

/// A name the lexer reads as one plain identifier (not a keyword).
fn is_plain_ident(name: &str) -> bool {
    use hale_syntax::lexer::{lex, TokenKind};
    match lex(name) {
        Ok(tokens) => {
            let words: Vec<_> = tokens.iter().filter(|t| !matches!(t.kind, TokenKind::Eof)).collect();
            words.len() == 1 && matches!(words[0].kind, TokenKind::Ident(_))
        }
        Err(_) => false,
    }
}

/// A document type name as a Hale type name: `lib::Item` is `lib_Item`.
pub(crate) fn type_ident(name: &str) -> String {
    hale_types::api_names::type_ident(name)
}

/// A JSON key as a Hale field name: anything an identifier cannot hold
/// becomes `_`, a leading digit gets a `f_`, and a reserved word a trailing `_`.
pub(crate) fn field_ident(key: &str) -> String {
    let mut out: String = key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert_str(0, "f_");
    }
    if !is_plain_ident(&out) {
        out.push('_');
    }
    out
}

/// `Orders::place` as `orders_place`; `LedgerBook::do_it` as `ledger_book_do_it`.
pub(crate) fn snake(member: &str) -> String {
    hale_types::api_names::snake(member)
}

/// A string as a JSON token: quoted, escaped as `serde_json` escapes.
pub(crate) fn json_quote(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// A string as a Hale string literal.
pub(crate) fn hale_lit(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
