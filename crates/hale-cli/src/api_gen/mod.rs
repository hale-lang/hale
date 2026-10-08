//! The clients `hale api client` generates from a surface's rows
//! (GH #1417, R8a; spec/api.md § The clients).
//!
//! A generator is a pure function of `surface_doc::ClientModel`: it reads
//! no path, no clock and no environment, so two runs, and two checkouts of
//! one program, generate the same bytes.

pub(crate) mod hale_client;

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
    name.replace("::", "_")
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
    let mut out = String::new();
    let mut prev_lower = false;
    for c in member.chars() {
        match c {
            ':' => {
                if !out.ends_with('_') {
                    out.push('_');
                }
                prev_lower = false;
            }
            c if c.is_ascii_uppercase() => {
                if prev_lower {
                    out.push('_');
                }
                out.push(c.to_ascii_lowercase());
                prev_lower = false;
            }
            c => {
                out.push(c);
                prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
            }
        }
    }
    out
}

/// `Orders::place` as `OrdersPlace`; `Fills` as `Fills`; `lib::Fills` as `LibFills`.
pub(crate) fn camel(member: &str) -> String {
    member
        .split(|c: char| c == ':' || c == '_')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut cs = p.chars();
            match cs.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + cs.as_str(),
                None => String::new(),
            }
        })
        .collect()
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
