//! GH #1951 (item 2): how a raised error becomes the enclosing fn's.
//!
//! `or raise` hands the inner call's error on as the enclosing
//! `fallible(E)` fn's own. The error `P` may differ from `E` in two ways
//! the language converts, tried in this order (spec/semantics.md § `or`
//! disposition):
//!
//! - **R2**, a set member: `E` is a set type holding `P`, and `P`'s value
//!   is the set's (the relation `fail`, `or fail <expr>`, `or raise` and a
//!   fallible handler's failure share);
//! - **R3**, a conversion: one `@raise fn conv(e: P) -> E` is in reach,
//!   and the raise is `or fail conv(err)`.
//!
//! This module holds the conversions: which `@raise` fns the bundle
//! declares that convert, and the refusals of the ones that cannot. The
//! checker reads the first to judge each `or raise`; the bundle's rules
//! report the second.

use std::collections::BTreeSet;

use hale_syntax::ast::{FnDecl, LocusMember, TopDecl};
use hale_syntax::{Diag, Span};

use crate::resolve::TopScope;
use crate::symbol::{Bundle, TopSymbol};
use crate::ty::Ty;

/// One `@raise fn` that converts: its declared (merged) name, the error
/// it takes and the error it returns.
#[derive(Debug, Clone, PartialEq)]
pub struct RaiseFn {
    pub name: String,
    pub from: Ty,
    pub to: Ty,
    pub span: Span,
}

/// Whether `fd` is written `@raise fn`.
pub fn is_raise_fn(fd: &FnDecl) -> bool {
    fd.decorators.iter().any(|d| d.name == "raise")
}

/// The library a declaration name was mangled in, by the mangler's
/// spelling (`crate::mangle::mangled`): `__lib_<lib>__<stem>__<name>`,
/// whose components never hold `__`, or `__lib_<stem>_<stem>_<name>`,
/// whose stem holds no `_`.
fn library_of(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("__lib_")?;
    Some(match rest.find("__") {
        Some(i) => &rest[..i],
        None => rest.split('_').next().unwrap_or(rest),
    })
}

/// The seed that declares a name, as the orphan rule reads it: `Some("")`
/// for the entry seed, `Some(lib)` for an imported library's, `None` for
/// a name no seed declares (a builtin or a stdlib type, a primitive).
fn seed_of_type<'t>(t: &'t Ty, user_types: &BTreeSet<&str>) -> Option<&'t str> {
    let Ty::Named(n) = t else { return None };
    if let Some(lib) = library_of(n) {
        return Some(lib);
    }
    user_types.contains(n.as_str()).then_some("")
}

/// The seed a declaring fn belongs to: an imported library's, by its
/// mangled name, else the entry seed's.
fn seed_of_fn(name: &str) -> &str {
    library_of(name).unwrap_or("")
}

/// Where a type is declared, as the orphan rule's refusal says it (of a
/// type neither seed of the conversion declares).
fn seed_shown(seed: Option<&str>) -> &'static str {
    match seed {
        None => "is a builtin type, which no seed declares",
        Some(_) => "is declared in another seed",
    }
}

/// The bundle's conversions, and the refusal of every `@raise` fn that is
/// none: a fn that is not free, takes other than one parameter, is
/// generic or fallible, returns nothing or its own parameter's type, or is
/// declared in a seed that declares neither type (the orphan rule). Its
/// effects are the effect assertion `@raise` implies (the parser's), held
/// by the effect rules as every assertion is.
pub fn raise_fns(bundle: &Bundle<'_>, top: &TopScope) -> (Vec<RaiseFn>, Vec<Diag>) {
    let mut fns = Vec::new();
    let mut diags = Vec::new();
    let user_types: BTreeSet<&str> = bundle
        .programs
        .values()
        .flat_map(|p| p.items.iter())
        .filter_map(|d| match d {
            TopDecl::Type(t) => Some(t.name.name.as_str()),
            _ => None,
        })
        .collect();
    for program in bundle.programs.values() {
        for decl in program.items.iter() {
            match decl {
                TopDecl::Fn(fd) if is_raise_fn(fd) => match judge(fd, top, &user_types) {
                    Ok(Some(f)) => fns.push(f),
                    Ok(None) => {}
                    Err(d) => diags.push(d),
                },
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        if let LocusMember::Fn(fd) = m {
                            if let Some(d) = fd.decorators.iter().find(|d| d.name == "raise") {
                                diags.push(Diag::ty(
                                    d.span,
                                    format!(
                                        "`@raise` declares a conversion, a free fn: `{}.{}` is a method of a \
                                         locus, and an `or raise` has no locus to call it on. Declare it at the \
                                         top level, `@raise fn {}(e: P) -> E {{ … }}`",
                                        l.name.name, fd.name.name, fd.name.name
                                    ),
                                ));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    (fns, diags)
}

/// One `@raise fn`: the conversion it declares, `None` for one whose
/// types the check could not name (refused elsewhere), or its refusal.
fn judge(fd: &FnDecl, top: &TopScope, user_types: &BTreeSet<&str>) -> Result<Option<RaiseFn>, Diag> {
    let name = fd.name.name.as_str();
    let at = fd.decorators.iter().find(|d| d.name == "raise").map(|d| d.span).unwrap_or(fd.name.span);
    let shape = "`@raise fn conv(e: P) -> E { … }`";
    if let Some(g) = fd.generics.first() {
        return Err(Diag::ty(
            g.name.span,
            format!(
                "a `@raise` fn converts one error type into another, and `{name}` is generic: an `or raise` \
                 applies the conversion its two types name, {shape}"
            ),
        ));
    }
    if fd.params.len() != 1 {
        return Err(Diag::ty(
            fd.name.span,
            format!(
                "a `@raise` fn takes one parameter, the error it converts, and `{name}` takes {}: {shape}",
                fd.params.len()
            ),
        ));
    }
    if let Some(f) = &fd.fallible {
        return Err(Diag::ty(
            f.span(),
            format!(
                "a `@raise` fn cannot fail: it runs where an `or raise` hands an error on, and a second \
                 error there has nowhere to go. Drop `fallible(…)` from `{name}`"
            ),
        ));
    }
    if fd.ret.is_none() {
        return Err(Diag::ty(
            fd.name.span,
            format!("a `@raise` fn returns the error it converts into, and `{name}` returns nothing: {shape}"),
        ));
    }
    let Some(TopSymbol::Fn(sig)) = top.lookup(name) else { return Ok(None) };
    let (Some((_, from)), to) = (sig.params.first(), &sig.ret) else { return Ok(None) };
    if matches!(from, Ty::Unknown) || matches!(to, Ty::Unknown) {
        return Ok(None);
    }
    if from == to {
        return Err(Diag::ty(
            at,
            format!(
                "`{name}` converts `{}` into itself: an `or raise` of the enclosing fn's own error type hands it \
                 on as it is. Drop `@raise`",
                from.display()
            ),
        ));
    }
    // The orphan rule: a conversion is declared beside one of its types,
    // so the two seeds that declare them hold every conversion between
    // them, and no third seed can add one.
    let (own, p, e) = (seed_of_fn(name), seed_of_type(from, user_types), seed_of_type(to, user_types));
    if p != Some(own) && e != Some(own) {
        return Err(Diag::ty(
            at,
            format!(
                "`{name}` converts `{}` into `{}`, and a `@raise` fn is declared in the seed of one of its two \
                 types: `{}` {}, and `{}` {}. Move `{name}` to a seed that declares one of them, or translate \
                 where the error is raised, `or fail {} {{ … }}`",
                from.display(),
                to.display(),
                from.display(),
                seed_shown(p),
                to.display(),
                seed_shown(e),
                to.display(),
            ),
        ));
    }
    Ok(Some(RaiseFn { name: name.to_string(), from: from.clone(), to: to.clone(), span: fd.name.span }))
}

#[cfg(test)]
mod tests {
    use super::library_of;

    #[test]
    fn a_mangled_name_names_its_library() {
        assert_eq!(library_of("__lib_errs_errs_ApiError"), Some("errs"));
        assert_eq!(library_of("__lib_a_x2fb__lib__conv"), Some("a_x2fb"));
        assert_eq!(library_of("ApiError"), None);
    }
}
