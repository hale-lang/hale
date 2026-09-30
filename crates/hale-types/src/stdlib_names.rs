//! A top-level name the bundled stdlib declares is declared once.
//!
//! The stdlib's loci, types and fns are merged into every program under
//! their internal names (`std::http::Server` is `__StdHttpServer`), so a
//! second declaration of one of those names — a user seed spelling it,
//! or a second stdlib file declaring it — meets the first only in the
//! merged program. Nothing compared them: the build kept one of the two
//! and then failed on the other (a codegen panic, "method declared in
//! pass A2", when the HTTP server's per-connection locus reused the
//! client's `__StdHttpConn`). The duplicate is refused here, located,
//! naming both declarations (spec/semantics.md § "Declarations inside `module { }`").

use std::collections::BTreeMap;

use hale_syntax::ast::{Program, TopDecl};
use hale_syntax::error::{Diag, SpanOrigin};
use hale_syntax::Span;

/// A declaration's text without its positions (`Pos(12)`) or the
/// snapshot's identities (`NodeId(7)`): two declarations with the
/// same text compare equal wherever they sit.
fn shape(d: &TopDecl) -> String {
    let mut text = format!("{:?}", d);
    for open in ["Pos(", "NodeId("] {
        let mut out = String::with_capacity(text.len());
        let mut rest = text.as_str();
        while let Some(at) = rest.find(open) {
            let digits = &rest[at + open.len()..];
            let n = digits.bytes().take_while(u8::is_ascii_digit).count();
            out.push_str(&rest[..at + open.len()]);
            rest = &digits[n..];
        }
        out.push_str(rest);
        text = out;
    }
    text
}

/// Every top-level declaration, the kinds the program's own duplicate
/// rule covers (resolve.rs, "duplicate top-level name"): one namespace.
fn declared<'p>(items: &'p [TopDecl], out: &mut Vec<(String, Span, &'static str, &'p TopDecl)>) {
    for item in items {
        match item {
            TopDecl::Module(m) => declared(&m.items, out),
            TopDecl::Locus(l) => out.push((l.name.name.clone(), l.name.span, "locus", item)),
            TopDecl::Type(t) => out.push((t.name.name.clone(), t.name.span, "type", item)),
            TopDecl::Interface(i) => out.push((i.name.name.clone(), i.name.span, "interface", item)),
            TopDecl::Perspective(p) => out.push((p.name.name.clone(), p.name.span, "perspective", item)),
            TopDecl::Fn(f) => out.push((f.name.name.clone(), f.name.span, "fn", item)),
            TopDecl::Const(c) => out.push((c.name.name.clone(), c.name.span, "constant", item)),
            TopDecl::Topic(t) => out.push((t.name.name.clone(), t.name.span, "topic", item)),
            TopDecl::RingLayout(r) => out.push((r.name.name.clone(), r.name.span, "ring layout", item)),
            _ => {}
        }
    }
}

/// The bundled stdlib's own duplicates, and every user declaration that
/// reuses a name the stdlib declares.
pub fn stdlib_name_diags(programs: &BTreeMap<String, &Program>) -> Vec<Diag> {
    match crate::stdlib_bodies::program() {
        Some(std_prog) => name_diags(std_prog, programs),
        None => Vec::new(),
    }
}

/// [`stdlib_name_diags`] against a given stdlib program: `std_prog`'s own
/// duplicates, and each declaration in `programs` that reuses one of its
/// names (one identical to the stdlib's is the stdlib's own seed).
pub fn name_diags(std_prog: &Program, programs: &BTreeMap<String, &Program>) -> Vec<Diag> {
    let mut std_decls = Vec::new();
    declared(&std_prog.items, &mut std_decls);
    let mut diags = Vec::new();
    let mut first: BTreeMap<String, (Span, &'static str, &TopDecl)> = BTreeMap::new();
    for (name, span, kind, decl) in std_decls {
        match first.get(&name) {
            Some((at, _, _)) => diags.push(Diag {
                origin: SpanOrigin::Stdlib,
                ..Diag::ty(
                    span,
                    format!(
                        "the stdlib declares `{name}` twice: this {kind} and an earlier declaration share one name, and a program can hold only one of them. Rename one"
                    ),
                )
                .with_stdlib_related(*at, "the earlier declaration")
            }),
            None => {
                first.insert(name, (span, kind, decl));
            }
        }
    }
    for program in programs.values() {
        let mut mine = Vec::new();
        declared(&program.items, &mut mine);
        for (name, span, kind, decl) in mine {
            if let Some((at, std_kind, std_decl)) = first.get(&name) {
                // the stdlib's own seed, checked as a program (the corpus
                // harvests them): the same declaration, not a second one
                if shape(decl) == shape(std_decl) {
                    continue;
                }
                diags.push(
                    Diag::ty(
                        span,
                        format!(
                            "this {kind}'s name is the stdlib's internal name for its {std_kind} `{name}`, and the stdlib's declarations are merged into every program under those names: the two cannot share one. Rename this {kind}"
                        ),
                    )
                    .with_stdlib_related(*at, "the stdlib's declaration"),
                );
            }
        }
    }
    diags
}
