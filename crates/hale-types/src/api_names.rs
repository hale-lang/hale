//! The names a surface's generated forms declare for themselves, in one
//! place: the `.proto`, the Hale, TypeScript and JavaScript clients, and
//! the description.
//!
//! Each generator names its own declarations through these functions and
//! lists, never by a format of its own, and the naming of imported types
//! (`topic_identity`, `imported_names`) reserves [`reserved_for`] over the
//! program: the union of everything any generator may emit apart from the
//! schema's own type names (and the codec functions named after them). An
//! imported type whose declared name is one of them is qualified, as on a
//! collision with another type, so no generator declares a name twice.
//! `hale-cli`'s `every_generated_name_is_reserved_or_a_schema_name` test
//! generates every form of real surfaces and checks that each name they declare is reserved here or is
//! derived from a schema, which catches a kind of declaration added to a
//! generator and forgotten here.

use std::collections::BTreeSet;

use hale_syntax::ast::{LocusMember, TopDecl, TypeDecl, TypeDeclBody};

/// The Hale client's own declarations: its constants and its preamble's.
pub const HALE_CLIENT_NAMES: &[&str] = &[
    "SURFACE", "SURFACE_DIGEST", "ApiRefusal", "ApiRaw", "ApiWs", "api_lost", "api_refusal", "api_str", "api_scalar_str",
    "api_scalar_int", "api_scalar_bool", "api_scalar_float", "api_roles", "api_refused", "api_http_send", "api_http",
    "api_unix_exchange", "api_unix", "api_call", "api_get", "api_describe", "api_lists", "API_WAIT_NS", "API_ID",
    "API_WS_KEY", "API_MASK",
];
/// The JavaScript client's own declarations.
pub const JS_CLIENT_NAMES: &[&str] = &[
    "SURFACE", "SURFACE_DIGEST", "ClientOptions", "StreamOptions", "Refusal", "TransportError", "Outcome", "describe",
    "StreamEvent", "Subscription", "apiCall", "apiRefusal", "apiSubscribe", "apiMapEncode", "apiMapDecode",
];
/// The TypeScript client's own declarations.
pub const TS_CLIENT_NAMES: &[&str] = &[
    "SURFACE", "SURFACE_DIGEST", "ClientOptions", "StreamOptions", "Refusal", "TransportError", "Outcome", "describe",
    "StreamEvent", "Subscription", "apiCall", "apiRefusal", "apiSubscribe", "apiMapEncode", "apiMapDecode", "__Record",
    "__Promise", "__Array",
    "__AsyncIterable", "__AsyncGenerator", "__Response", "__MessageEvent",
];

/// `Orders::place` as `OrdersPlace`, `lib::Fills` as `LibFills`.
pub fn camel(member: &str) -> String {
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

/// `Orders::place` as `ordersPlace`.
pub fn lower_camel(name: &str) -> String {
    let c = camel(name);
    let mut cs = c.chars();
    match cs.next() {
        Some(f) => f.to_ascii_lowercase().to_string() + cs.as_str(),
        None => String::new(),
    }
}

/// `Orders::place` as `orders_place`; `LedgerBook::do_it` as `ledger_book_do_it`.
pub fn snake(member: &str) -> String {
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

/// A member's outcome type, in every client: `OrdersPlaceOutcome`.
pub fn outcome(member: &str) -> String {
    format!("{}Outcome", camel(member))
}

/// A stream's event type, in every client: `FillsEvent`.
pub fn event(topic: &str) -> String {
    format!("{}Event", camel(topic))
}

/// A stream's subscription locus, in the Hale client: `FillsSubscription`.
pub fn subscription(topic: &str) -> String {
    format!("{}Subscription", camel(topic))
}

/// A stream's subscribe fn, in the TypeScript and JavaScript clients:
/// `subscribeFills`.
pub fn subscribe_fn(topic: &str) -> String {
    format!("subscribe{}", camel(topic))
}

/// A document type name as the generated code spells it, in the `.proto`
/// and every client: `lib.Item` (an imported type qualified by its
/// importer's path) is `lib_Item`.
pub fn type_ident(doc: &str) -> String {
    doc.replace("::", "_").replace('.', "_")
}

/// The names the forms generate after one schema, by its final document
/// name `doc`: the spelling of a qualified name (`lib_Item`, which no other
/// type may then be named), the TypeScript and JavaScript clients' codecs
/// (`encode_`/`decode_`; the Hale client's are the type's own
/// `T::api_encode`/`T::api_decode`, no name of their own), and an enum's
/// `.proto` variant names (`lib_Shape_Pair`). They depend on the names the
/// naming finally gives (`topic_identity::Shapes::of_all` reserves them to
/// a fixpoint), so they are not in [`reserved_for`].
pub fn schema_names(doc: &str, t: &TypeDecl, out: &mut BTreeSet<String>) {
    let ident = type_ident(doc);
    if ident != doc {
        out.insert(ident.clone());
    }
    for p in ["encode_", "decode_"] {
        out.insert(format!("{p}{ident}"));
    }
    match &t.body {
        TypeDeclBody::Enum(vs) => {
            for v in vs {
                out.insert(proto_variant(&ident, &v.name.name));
            }
        }
        _ => {}
    }
}

/// A record of several values of an enum's variant, and a payload-free
/// enum's value, in the `.proto`: `Shape_Circle`.
pub fn proto_variant(enum_name: &str, variant: &str) -> String {
    format!("{enum_name}_{variant}")
}

/// What a generator may declare for one member: the `.proto`'s one-field
/// wrappers and rpc, a client's outcome and member fn.
fn member_names(m: &str, out: &mut BTreeSet<String>) {
    for w in ["Request", "Response", "Error"] {
        out.insert(crate::proto_gen::wrapper_name(m, w));
    }
    out.insert(crate::proto_gen::rpc_name(m));
    out.insert(outcome(m));
    out.insert(snake(m));
    out.insert(lower_camel(m));
}

/// What a generator may declare for one stream topic.
fn topic_names(t: &str, out: &mut BTreeSet<String>) {
    out.insert(event(t));
    out.insert(subscription(t));
    out.insert(subscribe_fn(t));
}

/// Every name a generated form may declare for the programs `slices`,
/// apart from the schema's own type names: the fixed names (each client's
/// preamble and constants, the `.proto`'s refusal and empty messages, the
/// builtin records a description renders), and, for every member of every
/// surface the programs declare (an `api` block's rows, an `@rpc` fn) and
/// every topic (a hub's stream is one), its own, and every `api` block's
/// `.proto` service. What a form generates after a schema's own name is
/// [`schema_names`].
pub fn reserved_for(slices: &[&[TopDecl]]) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = HALE_CLIENT_NAMES
        .iter()
        .chain(TS_CLIENT_NAMES)
        .chain(JS_CLIENT_NAMES)
        .map(|s| s.to_string())
        .collect();
    out.insert(crate::proto_gen::REFUSAL.to_string());
    out.insert(crate::proto_gen::EMPTY.to_string());
    out.extend(crate::builtin_types::BUILTIN_TYPES.iter().map(|b| b.name.to_string()));
    fn walk(items: &[TopDecl], out: &mut BTreeSet<String>) {
        for item in items {
            match item {
                TopDecl::Api(a) => {
                    // the `.proto`'s service, and the message a record named
                    // as the surface is (`proto_gen::message_name`)
                    out.insert(a.name.name.clone());
                    out.insert(crate::proto_gen::message_name(&a.name.name, &a.name.name));
                    for r in &a.rows {
                        member_names(&format!("{}::{}", r.written, r.method.name), out);
                    }
                }
                TopDecl::Locus(l) => {
                    let shown = l.display.clone().unwrap_or_else(|| l.name.name.clone());
                    for m in &l.members {
                        if let LocusMember::Fn(f) = m {
                            if f.rpc.is_some() {
                                member_names(&format!("{shown}::{}", f.name.name), out);
                            }
                        }
                    }
                }
                TopDecl::Topic(t) => {
                    topic_names(&t.name.name, out);
                    if let Some(d) = &t.display {
                        topic_names(d, out);
                    }
                }
                TopDecl::Module(m) => walk(&m.items, out),
                _ => {}
            }
        }
    }
    for items in slices {
        walk(items, &mut out);
    }
    out
}
