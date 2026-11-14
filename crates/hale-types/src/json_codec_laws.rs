//! The record codecs' law (spec/semantics.md § Metadata tags).
//!
//! A record with a `json:` field tag has a generated `T::from_json(s)`
//! and `T::to_json(v)`, which the desugar sequence rewrites to the
//! codecs it wrote (`json_gen`). A call the sequence left as written
//! names a codec the compiler did not write: the record declares no
//! tag, a field of it (or of a record or an enum it reaches) is a shape
//! the JSON codec does not carry, the type is no record, or the call has
//! the wrong arity. Each is refused here, naming the rule, so that a
//! program the check admits builds (downstream handoff: a record with an
//! enum field checked and then failed to lower, "`or` over unknown path
//! call").

use std::collections::BTreeMap;

use hale_syntax::ast::{flat_decls, Expr, Program, QualifiedName, TopDecl, TypeDeclBody, TypeExpr};
use hale_syntax::Diag;

use crate::listener_window::{for_each_expression_root, visit_block, visit_expr, Root, Visited};
use crate::symbol::Bundle;
use crate::topic_identity::Shapes;

/// The codec methods a record has.
const METHODS: [&str; 2] = ["from_json", "to_json"];

pub fn json_codec_laws(bundle: &Bundle<'_>) -> Vec<Diag> {
    let programs: Vec<&Program> = bundle.programs.values().copied().collect();
    // the tags' options, whether or not a call names the codec: a served
    // row's codec reads them too
    let mut diags = json_tag_laws(&programs);
    // every call left as written, before any type is classified: a
    // program with none (every program that builds) costs no `Shapes`
    let mut calls: Vec<(QualifiedName, usize)> = Vec::new();
    for p in &programs {
        for_each_expression_root(&p.items, &mut |root| {
            let mut visit = |v: Visited<'_>| {
                let Visited::Expr(Expr::Call { callee, args, .. }) = v else { return };
                let Expr::Path(qn) = callee.as_ref() else { return };
                let n = qn.segments.len();
                if (2..=3).contains(&n) && METHODS.contains(&qn.segments[n - 1].name.as_str()) {
                    calls.push((qn.clone(), args.len()));
                }
            };
            match root {
                Root::Block(b) => visit_block(b, &mut visit),
                Root::Expr(e) => visit_expr(e, &mut visit),
            }
        });
    }
    if calls.is_empty() {
        return diags;
    }
    let slices: Vec<&[TopDecl]> = programs.iter().map(|p| p.items.as_slice()).collect();
    let shapes = Shapes::of_all(&slices);
    let mut types: BTreeMap<&str, &hale_syntax::ast::TypeDecl> = BTreeMap::new();
    for p in &programs {
        for d in flat_decls(&p.items) {
            if let TopDecl::Type(td) = d {
                types.entry(td.name.name.as_str()).or_insert(td);
            }
        }
    }
    for (qn, arity) in calls.iter().map(|(q, a)| (q, *a)) {
        let n = qn.segments.len();
        let method = qn.segments[n - 1].name.as_str();
        let written: Vec<&str> = qn.segments.iter().map(|s| s.name.as_str()).collect();
        let written = written.join("::");
        // the record: the program's own, or an import's by the build's
        // rename table (`alias::T`); a head that is neither is the
        // unresolved-path rule's
        let head: Vec<&str> = qn.segments[..n - 1].iter().map(|s| s.name.as_str()).collect();
        let name: &str = if n == 2 {
            head[0]
        } else {
            match bundle.import_renames.iter().find(|(k, _)| k.iter().map(String::as_str).eq(head.iter().copied())) {
                Some((_, merged)) => merged.as_str(),
                None => continue,
            }
        };
        let Some(td) = types.get(name) else { continue };
        let shown = format!("`{written}`");
        let fields = match &td.body {
            TypeDeclBody::Struct(fields) => fields,
            TypeDeclBody::Enum(_) => {
                diags.push(Diag::ty(
                    qn.span,
                    format!(
                        "{shown}: `{name}` is an enum, and `{method}` is a record's (a record with a `json:` field \
                         tag, spec/semantics.md § Metadata tags): an enum crosses as a field of one"
                    ),
                ));
                continue;
            }
            _ => {
                diags.push(Diag::ty(
                    qn.span,
                    format!(
                        "{shown}: `{name}` is no record, and `{method}` is a record's (a record with a `json:` field \
                         tag, spec/semantics.md § Metadata tags)"
                    ),
                ));
                continue;
            }
        };
        if arity != 1 {
            let what = if method == "from_json" { "the JSON text" } else { "the value" };
            diags.push(Diag::ty(qn.span, format!("{shown} takes one argument ({what}), got {arity}")));
            continue;
        }
        // a positional record opts in as a tag does
        let tagged =
            td.positional || fields.iter().any(|f| f.tag.as_deref().and_then(|t| hale_syntax::desugar::tag_value(t, "json")).is_some());
        if !tagged {
            let hint = fields
                .first()
                .map(|f| {
                    format!(
                        "; tag a field, `{f}: {t} `json:\"{f}\"`;`",
                        f = f.name.name,
                        t = crate::surfaces::type_text(&f.ty)
                    )
                })
                .unwrap_or_default();
            diags.push(Diag::ty(
                qn.span,
                format!(
                    "{shown}: `{name}` declares no `json:` field tag, which is what gives a record its JSON codec \
                     (spec/semantics.md § Metadata tags){hint}"
                ),
            ));
            continue;
        }
        // tagged, and still no codec: a field the JSON codec does not
        // carry, named by its path (law 5's classification, spec/api.md §
        // Codecs), or one the record codecs carry no way of their own
        let te = TypeExpr::Named { path: QualifiedName { segments: vec![td.name.clone()], span: td.name.span }, generic_args: Vec::new(), span: td.name.span };
        let message = match crate::surfaces::codec_refusal(&shapes, &te) {
            Some((path, at)) if !path.is_empty() => {
                format!("{shown}: `{name}` has a field `{}: {at}`, which the JSON codec does not carry", path.join("."))
            }
            _ => format!("{shown}: the JSON codec does not carry `{name}` (a record of another seed among its fields, or a map of values it does not carry)"),
        };
        diags.push(Diag::ty(qn.span, message).with_related(td.name.span, "the record"));
    }
    diags
}

/// The `json:` tag's options (spec/semantics.md § Metadata tags,
/// spec/api.md § Codecs). After its key a tag may say `string`
/// (`json:"px,string"`, or `json:",string"` keeping the field's name): a
/// `Decimal<S>` field, or a list of them, written as a JSON string holding
/// its digits. Any other option, and `string` on a field that is no
/// Decimal, is refused at the field, as is a key on a `@positional`
/// record's field, whose place in the array is its name on the wire.
pub fn json_tag_laws(programs: &[&Program]) -> Vec<Diag> {
    let slices: Vec<&[TopDecl]> = programs.iter().map(|p| p.items.as_slice()).collect();
    let shapes = Shapes::of_all(&slices);
    let mut diags = Vec::new();
    for p in programs {
        for d in flat_decls(&p.items) {
            let TopDecl::Type(td) = d else { continue };
            let TypeDeclBody::Struct(fields) = &td.body else { continue };
            for f in fields {
                let Some(tag) = f.tag.as_deref() else { continue };
                if hale_syntax::desugar::tag_value(tag, "json").is_none() {
                    continue;
                }
                let at = format!("`{}.{}`", td.name.name, f.name.name);
                if td.positional {
                    if let Some(key) = hale_syntax::json_gen::json_tag_key(Some(tag)) {
                        diags.push(Diag::ty(
                            f.span,
                            format!(
                                "{at}: `{}` is a `@positional` record, which crosses JSON as an array of its fields \
                                 in order, so its fields have no key and `json:\"{key}\"` names none; drop the key \
                                 (an option stays: `json:\",string\"`)",
                                td.name.name
                            ),
                        ));
                    }
                }
                for opt in hale_syntax::json_gen::json_tag_options(Some(tag)) {
                    if opt != "string" {
                        diags.push(Diag::ty(
                            f.span,
                            format!(
                                "{at}: `json:` tag option `{opt}` is not one: the one option is `string`, a Decimal \
                                 written as a JSON string (`json:\"price,string\"`)"
                            ),
                        ));
                        continue;
                    }
                    let ty = shapes.dealias(&f.ty);
                    let elem = hale_syntax::json_gen::wire_list(&ty).map(|(e, _, _)| shapes.dealias(e));
                    let decimal = |te: &TypeExpr| matches!(te, TypeExpr::Primitive(hale_syntax::ast::PrimType::Decimal(_), _));
                    if !(decimal(&ty) || elem.as_ref().is_some_and(decimal)) {
                        diags.push(Diag::ty(
                            f.span,
                            format!(
                                "{at}: `json:` tag option `string` writes a Decimal as a JSON string, and the field is \
                                 `{}`: it is for a `Decimal<S>` field or a list of them",
                                crate::surfaces::type_text(&f.ty)
                            ),
                        ));
                    }
                }
            }
        }
    }
    diags
}
