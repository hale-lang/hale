//! GH #1530 (fourth pass, section 3): `@repo(T)`.
//!
//! A repo is a locus the compiler is told to trust as the place where
//! `T`'s important state lives. `@repo(T)` is a checker: the driver is
//! hand-written, and what the compiler holds it to is
//!
//! - `T` is a record with exactly one field tagged `` `db:"…,key"` ``;
//! - the locus carries the whole method set for `T` over that key `K`:
//!   `get(id: K) -> T`, `create(o: T) -> T`, `put(id: K, o: T) -> T`,
//!   `remove(id: K)`, each `fallible(E)` with one error type;
//! - every method carries exactly one state class from the family
//!   `reads(T) | writes(T) | creates(T) | deletes(T) | mutates(T)`
//!   (`@effects(is: { reads(T) })`), the four above carrying the class
//!   their name says, and an extra method (a `find_by_customer`) any one
//!   class of its own;
//! - every class but `reads` carries exactly one of `@idempotent` /
//!   `@not_idempotent`, and `reads` carries neither: there is no default,
//!   so the question is asked at every write;
//! - a repo declares no state class of another type.
//!
//! [`expand`] declares the family through the load's one class table
//! (`domain __RepoTypes = { T, … }; effect reads(__RepoTypes); …`), so
//! the classes propagate through calls as any declared class does and a
//! claim names them (`forbid reaches(Public, effects(writes(Order)))`).
//! [`laws`] reports what is wrong. Atomicity is per repo operation: a
//! `get` then a `put` is two operations.

use std::collections::BTreeSet;

use hale_syntax::ast::{
    flat_decls, EffectAssert, EffectClass, FnDecl, LocusDecl, LocusMember, Program, TopDecl,
    TypeDeclBody, TypeExpr,
};
use hale_syntax::{Diag, Span};

use crate::surfaces::type_text;

/// Where the generated family parses.
const REPO_BASE: u32 = hale_syntax::api_names::API_SYNTH_BASE + 0x0F00_0000;

/// The five state classes, as the effect family spells them.
pub const CLASSES: [&str; 5] = ["reads", "writes", "creates", "deletes", "mutates"];

/// The method set every repo carries: name, the class it must declare.
const REQUIRED: [(&str, &str); 4] = [
    ("get", "reads"),
    ("create", "creates"),
    ("put", "writes"),
    ("remove", "deletes"),
];

/// A record as an author wrote it, for messages: an imported record is
/// mangled (`__lib_store___store__Item`).
fn plain(t: &str) -> &str {
    if t.starts_with("__") {
        t.rsplit("__").next().filter(|s| !s.is_empty()).unwrap_or(t)
    } else {
        t
    }
}

fn repo_types(programs: &[&Program]) -> BTreeSet<String> {
    programs
        .iter()
        .flat_map(|p| flat_decls(&p.items))
        .filter_map(|d| match d {
            TopDecl::Locus(l) => l.repo.as_ref().map(|r| r.ty.name.clone()),
            _ => None,
        })
        .collect()
}

/// Declare the state-class family over the types some `@repo` holds.
pub fn expand(programs: &mut [&mut Program]) {
    let types = {
        let ro: Vec<&Program> = programs.iter().map(|p| &**p).collect();
        repo_types(&ro)
    };
    if types.is_empty() || programs.is_empty() {
        return;
    }
    let mut src = format!(
        "domain __RepoTypes = {{ {} }};\n",
        types.into_iter().collect::<Vec<_>>().join(", ")
    );
    for c in CLASSES {
        src.push_str(&format!("effect {c}(__RepoTypes);\n"));
    }
    // The declarations are registry-only (the parser reads on to the next
    // item), so the source ends in a placeholder fn, which is dropped.
    src.push_str("fn __repo_classes_end() { }\n");
    match crate::effect_classes::parse_generated(programs, &src, REPO_BASE) {
        Ok(generated) => programs[0].items.extend(
            generated
                .items
                .into_iter()
                .filter(|d| !matches!(d, TopDecl::Fn(f) if f.name.name == "__repo_classes_end")),
        ),
        Err(ds) => {
            eprintln!(
                "repo: the generated state classes did not parse: {}",
                ds.iter()
                    .map(|d| d.message.clone())
                    .collect::<Vec<_>>()
                    .join("; ")
            );
            return;
        }
    }
    // A state class names a record by the name its module wrote
    // (`reads(Item)`); the import mangles the record and not the class
    // text. Resolve each reference in its declaring module's scope, so the
    // class is keyed by the mangled record: two imported records of one
    // written name keep two classes, and a contract anywhere in a library
    // (`@effects(none: { writes(Item) })`) names the library's own.
    let names: Vec<String> = programs[0].effect_names.clone();
    let repos: BTreeSet<String> = {
        let ro: Vec<&Program> = programs.iter().map(|p| &**p).collect();
        repo_types(&ro)
    };
    let mut resolve = |classes: &mut Vec<EffectClass>, prefix: &str| {
        for c in classes.iter_mut() {
            let EffectClass::User(i) = c else { continue };
            let Some(name) = names.get(*i as usize) else {
                continue;
            };
            let Some((word, rest)) = name.split_once('(') else {
                continue;
            };
            let target = format!("{prefix}{}", rest.trim_end_matches(')'));
            if CLASSES.contains(&word) && repos.contains(&target) {
                if let Some(j) = names.iter().position(|n| *n == format!("{word}({target})")) {
                    *c = EffectClass::User(j as u16);
                }
            }
        }
    };
    let mut fix = |f: &mut FnDecl, prefix: &str| {
        for a in &mut f.effects {
            match a {
                EffectAssert::Forbid(cs)
                | EffectAssert::Carries(cs)
                | EffectAssert::Causes(cs)
                | EffectAssert::Only(cs) => resolve(cs, prefix),
                _ => {}
            }
        }
    };
    for p in programs.iter_mut() {
        hale_syntax::ast::for_each_decl_mut(&mut p.items, &mut |d| match d {
            TopDecl::Locus(l) => {
                let prefix = module_prefix(&l.name.name);
                for m in &mut l.members {
                    if let LocusMember::Fn(f) = m {
                        fix(f, &prefix);
                    }
                }
            }
            TopDecl::Fn(f) => {
                let prefix = module_prefix(&f.name.name);
                fix(f, &prefix);
            }
            _ => {}
        });
    }
}

/// The mangling prefix of an imported declaration's name
/// (`__lib_store___store__` of `__lib_store___store__Items`), empty for a
/// declaration of the root seed.
fn module_prefix(name: &str) -> String {
    match name.strip_prefix("__").and_then(|r| r.rfind("__")) {
        Some(i) => name[..i + 4].to_string(),
        None => String::new(),
    }
}

/// The key field of a record: the one tagged `db:"NAME,key"`.
fn key_fields(
    fields: &[hale_syntax::ast::StructField],
) -> Vec<(&hale_syntax::ast::StructField, bool)> {
    fields
        .iter()
        .filter_map(|f| {
            let tag = f.tag.as_deref()?;
            let at = tag.find("db:\"")? + 4;
            let body = &tag[at..];
            let body = &body[..body.find('"')?];
            Some((f, body.split(',').skip(1).any(|o| o.trim() == "key")))
        })
        .filter(|(_, key)| *key)
        .collect()
}

/// The state class a method declares, as `(class word, type)`, from the
/// names `@effects(is: { … })` interned.
fn declared_classes(f: &FnDecl, names: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for a in &f.effects {
        let EffectAssert::Carries(cs) = a else {
            continue;
        };
        for c in cs {
            let EffectClass::User(i) = c else { continue };
            let Some(name) = names.get(*i as usize) else {
                continue;
            };
            let Some((word, rest)) = name.split_once('(') else {
                continue;
            };
            if CLASSES.contains(&word) {
                out.push((word.to_string(), rest.trim_end_matches(')').to_string()));
            }
        }
    }
    out
}

fn mark(f: &FnDecl, name: &str) -> Option<Span> {
    f.decorators.iter().find(|d| d.name == name).map(|d| d.span)
}

fn ty_is(te: &TypeExpr, want: &str) -> bool {
    type_text(te) == want
}

fn check_repo(
    l: &LocusDecl,
    types: &[&hale_syntax::ast::TypeDecl],
    names: &[String],
    diags: &mut Vec<Diag>,
) {
    let Some(repo) = &l.repo else { return };
    let full = &repo.ty.name;
    let t = plain(full);
    let lname = &l.name.name;
    let Some(decl) = types.iter().find(|d| &d.name.name == full) else {
        diags.push(Diag::ty(
            repo.ty.span,
            format!("`@repo({t})` on `{lname}`: no record type `{t}` is declared"),
        ));
        return;
    };
    let TypeDeclBody::Struct(fields) = &decl.body else {
        diags.push(Diag::ty(repo.ty.span, format!("`@repo({t})` on `{lname}`: `{t}` is no record; a repo holds a record with a key field")));
        return;
    };
    let keys = key_fields(fields);
    let key_ty = match keys.as_slice() {
        [(f, _)] => type_text(&f.ty),
        [] => {
            diags.push(Diag::ty(
                repo.ty.span,
                format!("`@repo({t})` on `{lname}`: `{t}` has no key field; tag exactly one field `db:\"name,key\"`"),
            ));
            return;
        }
        [_, (second, _), ..] => {
            diags.push(Diag::ty(
                second.name.span,
                format!("`@repo({t})` on `{lname}`: `{t}` has more than one key field; tag exactly one `db:\"name,key\"`"),
            ));
            return;
        }
    };
    let fns: Vec<&FnDecl> = l
        .members
        .iter()
        .filter_map(|m| match m {
            LocusMember::Fn(f) => Some(f),
            _ => None,
        })
        .collect();
    // ---- the method set ----
    let mut err_ty: Option<(String, Span)> = None;
    for (name, class) in REQUIRED {
        let Some(f) = fns.iter().find(|f| f.name.name == name) else {
            diags.push(Diag::ty(
                l.name.span,
                format!("`{lname}` is a `@repo({t})` and has no `{name}`: a repo carries the whole method set (`get`, `create`, `put`, `remove`)"),
            ));
            continue;
        };
        let (params, ret): (Vec<String>, Option<&str>) = match name {
            "get" => (vec![key_ty.clone()], Some(full.as_str())),
            "create" => (vec![full.clone()], Some(full.as_str())),
            "put" => (vec![key_ty.clone(), full.clone()], Some(full.as_str())),
            _ => (vec![key_ty.clone()], None),
        };
        let shape = match name {
            "get" => format!("get(id: {key_ty}) -> {t}"),
            "create" => format!("create(o: {t}) -> {t}"),
            "put" => format!("put(id: {key_ty}, o: {t}) -> {t}"),
            _ => format!("remove(id: {key_ty})"),
        };
        let params_ok = f.params.len() == params.len()
            && f.params.iter().zip(&params).all(|(p, w)| ty_is(&p.ty, w));
        let ret_ok = match (&f.ret, ret) {
            (Some(r), Some(w)) => ty_is(r, w),
            (None, None) => true,
            (Some(r), None) => type_text(r) == "()",
            _ => false,
        };
        if !params_ok || !ret_ok {
            diags.push(Diag::ty(
                f.name.span,
                format!("`{lname}::{name}` of a `@repo({t})` must be `fn {shape} fallible(E)`"),
            ));
        }
        match &f.fallible {
            None => diags.push(Diag::ty(
                f.name.span,
                format!("`{lname}::{name}` of a `@repo({t})` must be `fallible(E)`: a transport maps the error's classes"),
            )),
            Some(e) => {
                let e = (type_text(e), f.name.span);
                match &err_ty {
                    None => err_ty = Some(e),
                    Some((prev, _)) if *prev != e.0 => diags.push(Diag::ty(
                        f.name.span,
                        format!("`{lname}::{name}` fails with `{}` where the repo's other methods fail with `{prev}`; a repo has one error type", e.0),
                    )),
                    _ => {}
                }
            }
        }
        let _ = class;
    }
    // ---- the classes and the markers, on every method ----
    for f in &fns {
        let fname = &f.name.name;
        let declared = declared_classes(f, names);
        for (word, ty) in &declared {
            if ty != full {
                diags.push(Diag::ty(
                    f.name.span,
                    format!("`{lname}::{fname}` declares `{word}({ty})`: a repo declares no state class of another type, it is a leaf"),
                ));
            }
        }
        let own: Vec<&String> = declared
            .iter()
            .filter(|(_, ty)| ty == full)
            .map(|(w, _)| w)
            .collect();
        let want = REQUIRED.iter().find(|(n, _)| n == fname).map(|(_, c)| *c);
        let class = match own.as_slice() {
            [c] => Some(c.as_str()),
            [] => {
                let hint = want.map(|c| format!(" (`{c}({t})`)")).unwrap_or_default();
                diags.push(Diag::ty(
                    f.name.span,
                    format!("`{lname}::{fname}` of a `@repo({t})` declares no state class: write `@effects(is: {{ <class>({t}) }})`{hint} with one of reads, writes, creates, deletes, mutates"),
                ));
                None
            }
            _ => {
                diags.push(Diag::ty(
                    f.name.span,
                    format!("`{lname}::{fname}` declares more than one state class; a method makes one kind of change"),
                ));
                None
            }
        };
        if let (Some(c), Some(w)) = (class, want) {
            if c != w {
                diags.push(Diag::ty(f.name.span, format!("`{lname}::{fname}` of a `@repo({t})` is `{c}({t})`; `{fname}` is `{w}({t})`")));
            }
        }
        let (idem, not) = (mark(f, "idempotent"), mark(f, "not_idempotent"));
        match (class, idem, not) {
            (Some("reads"), Some(s), _) | (Some("reads"), _, Some(s)) => diags.push(Diag::ty(
                s,
                format!("`{lname}::{fname}` is `reads({t})`, which is always safe to repeat; it carries neither `@idempotent` nor `@not_idempotent`"),
            )),
            (Some(c), None, None) if c != "reads" => diags.push(Diag::ty(
                f.name.span,
                format!("`{lname}::{fname}` is `{c}({t})` and says nothing of repeating it: write `@idempotent` or `@not_idempotent`; there is no default"),
            )),
            (_, Some(_), Some(s)) => diags.push(Diag::ty(s, format!("`{lname}::{fname}` is both `@idempotent` and `@not_idempotent`; it is one"))),
            _ => {}
        }
    }
}

/// What `@repo(T)` refuses, over a bundle's programs.
pub fn laws(programs: &[&Program]) -> Vec<Diag> {
    let mut diags = Vec::new();
    // the load's one class table: every program carries a prefix of it
    let empty = Vec::new();
    let names: &[String] = programs
        .iter()
        .map(|p| &p.effect_names)
        .max_by_key(|n| n.len())
        .unwrap_or(&empty);
    let types: Vec<&hale_syntax::ast::TypeDecl> = programs
        .iter()
        .flat_map(|p| flat_decls(&p.items))
        .filter_map(|d| match d {
            TopDecl::Type(t) => Some(t),
            _ => None,
        })
        .collect();
    for p in programs {
        for d in flat_decls(&p.items) {
            match d {
                TopDecl::Locus(l) if l.repo.is_some() => check_repo(l, &types, names, &mut diags),
                // the markers belong to a repo's methods
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        if let LocusMember::Fn(f) = m {
                            stray_markers(f, &mut diags);
                        }
                    }
                }
                TopDecl::Fn(f) => stray_markers(f, &mut diags),
                _ => {}
            }
        }
    }
    diags
}

fn stray_markers(f: &FnDecl, diags: &mut Vec<Diag>) {
    for d in &f.decorators {
        if d.name == "idempotent" || d.name == "not_idempotent" {
            diags.push(Diag::ty(
                d.span,
                format!(
                    "`@{}` marks a method of a `@repo(T)` locus; `{}` is not one",
                    d.name, f.name.name
                ),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two imported records of one written name (`Item`, mangled apart)
    /// keep two classes: each repo's `reads(Item)` is the class of its own
    /// record after the expansion.
    #[test]
    fn two_imported_records_of_one_written_name_keep_two_classes() {
        let src = r#"
type __a__Item { id: Int `db:"id,key"`; }
type __b__Item { id: Int `db:"id,key"`; }
@repo(__a__Item)
locus __a__Items {
    @effects(is: { reads(Item) })
    fn get(id: Int) -> Int { return id; }
}
@repo(__b__Item)
locus __b__Items {
    @effects(is: { reads(Item) })
    fn get(id: Int) -> Int { return id; }
}
fn main() { }
"#;
        let mut p = hale_syntax::parse_source(src).expect("parses");
        expand(&mut [&mut p]);
        let class_of = |locus: &str| -> String {
            for d in flat_decls(&p.items) {
                let TopDecl::Locus(l) = d else { continue };
                if l.name.name != locus {
                    continue;
                }
                for m in &l.members {
                    let LocusMember::Fn(f) = m else { continue };
                    for a in &f.effects {
                        if let EffectAssert::Carries(cs) = a {
                            if let EffectClass::User(i) = cs[0] {
                                return p.effect_names[i as usize].clone();
                            }
                        }
                    }
                }
            }
            panic!("no class on {locus}");
        };
        assert_eq!(class_of("__a__Items"), "reads(__a__Item)");
        assert_eq!(class_of("__b__Items"), "reads(__b__Item)");
    }
}
