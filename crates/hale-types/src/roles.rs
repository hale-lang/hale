//! The role rows (F.40 phase 4, A4; R4): every `role` declaration of the
//! bundle, over the programs after the desugar sequence. A role is
//! vocabulary a surface row's `requires` and a hub binding's `requires:`
//! name (`surfaces::surface_laws` law 4 reads the declarations from
//! here); the roles an environment maps are their projection
//! ([`RoleRows::declared_roles`]). Rows of the `role_rows` family: the
//! snapshot demands them once (`Snapshot::demand_role_rows`) and hands
//! them to the check (`CheckInputs::roles`).

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{Ident, TopDecl};
use hale_syntax::{Diag, Span};

use crate::law::{RuleId, Violation};
use crate::Bundle;

/// A role is declared once.
const DECLARED_ONCE: RuleId = RuleId::registered("verification/structural", "role-declared-once");
/// Every role an `includes` names is declared.
const DECLARED: RuleId = RuleId::registered("verification/structural", "role-declared");
/// `includes` is acyclic.
const ACYCLIC: RuleId = RuleId::registered("verification/structural", "role-includes-acyclic");

/// One `role` declaration, as written. Two declarations of one name are
/// two rows: the vocabulary's own rule reads them.
#[derive(Debug, Clone, PartialEq)]
pub struct RoleDeclRow {
    /// The role's name, with its span.
    pub name: Ident,
    /// The roles it `includes`, each with its span, in the order written.
    pub includes: Vec<Ident>,
    /// The declaration's span.
    pub span: Span,
    /// Its site: the bundle program that holds it, and its item index at
    /// each depth (a module's contents under the module's index).
    pub program: String,
    pub path: Vec<usize>,
}

/// The role rows of a bundle.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RoleRows {
    /// Every `role` declaration, in walk order.
    pub roles: Vec<RoleDeclRow>,
}

impl RoleRows {
    /// The roles an environment maps: the declared names, sorted and
    /// de-duplicated.
    pub fn declared_roles(&self) -> Vec<String> {
        self.vocabulary().into_iter().map(|(name, _)| name).collect()
    }

    /// Each declared role once, by name, with the `includes` of its first
    /// declaration.
    pub fn vocabulary(&self) -> Vec<(String, Vec<String>)> {
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for r in &self.roles {
            out.entry(r.name.name.clone())
                .or_insert_with(|| r.includes.iter().map(|i| i.name.clone()).collect());
        }
        out.into_iter().collect()
    }
}

/// The role rows' producer: one walk over the bundle's declarations, in
/// the bundle's program order.
pub fn role_rows(bundle: &Bundle<'_>) -> RoleRows {
    fn walk(items: &[TopDecl], program: &str, path: &mut Vec<usize>, rows: &mut RoleRows) {
        for (i, item) in items.iter().enumerate() {
            path.push(i);
            match item {
                TopDecl::Role(r) => rows.roles.push(RoleDeclRow {
                    name: r.name.clone(),
                    includes: r.includes.clone(),
                    span: r.span,
                    program: program.to_string(),
                    path: path.clone(),
                }),
                TopDecl::Module(m) => walk(&m.items, program, path, rows),
                _ => {}
            }
            path.pop();
        }
    }
    let mut rows = RoleRows::default();
    for (name, p) in &bundle.programs {
        walk(&p.items, name, &mut Vec::new(), &mut rows);
    }
    rows
}

// ---- the law ---------------------------------------------------------

/// GH #1109: the role rules, a law over the role rows.
///
/// Roles are declared vocabulary like `group` and `effect`: a role an
/// `includes` names that nothing declares is an error, bundle-wide, and
/// a role is declared once. `includes` is grant-only and union-only, so
/// a cycle says nothing and is refused. A `requires` naming an undeclared
/// role is the surface law's (`surfaces::surface_laws` law 4) and the hub
/// binding's.
///
/// The three rules are the three findings below, each a registered rule
/// whose finding is a [`Violation`] (F.40 phase 4, W5).
pub fn role_laws(rows: &RoleRows) -> Vec<Diag> {
    let mut found = Vec::new();
    role_walk(rows, &mut found);
    crate::law::diags(found)
}

/// The walk [`role_laws`] reports, in its order.
fn role_walk(rows: &RoleRows, found: &mut Vec<Violation>) {
    // The vocabulary: the first declaration of a name is the role.
    let mut decls: BTreeMap<&str, &RoleDeclRow> = BTreeMap::new();
    for r in &rows.roles {
        match decls.get(r.name.name.as_str()) {
            Some(first) => found.push(declared_twice(r, first)),
            None => {
                decls.insert(&r.name.name, r);
            }
        }
    }
    for (name, r) in &decls {
        for inc in &r.includes {
            if !decls.contains_key(inc.name.as_str()) {
                found.push(undeclared(inc, &format!("`role {} includes …`", name)));
            }
        }
        if includes_itself(name, &decls) {
            found.push(role_cycle(name, r.name.span));
        }
    }
}

/// `name` is reachable from its own `includes`.
fn includes_itself(name: &str, decls: &BTreeMap<&str, &RoleDeclRow>) -> bool {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut stack: Vec<&str> = decls[name].includes.iter().map(|i| i.name.as_str()).collect();
    while let Some(cur) = stack.pop() {
        if cur == name {
            return true;
        }
        if !seen.insert(cur) {
            continue;
        }
        if let Some(more) = decls.get(cur) {
            stack.extend(more.includes.iter().map(|i| i.name.as_str()));
        }
    }
    false
}

/// Rule: a role is declared once.
fn declared_twice(r: &RoleDeclRow, first: &RoleDeclRow) -> Violation {
    Violation::error(
        DECLARED_ONCE,
        r.name.span,
        format!(
            "role `{}` is declared twice; a role is one name the \
             deployment maps, so declare it once and `includes` it \
             where a wider role should hold it",
            r.name.name
        ),
    )
    .step(first.name.span, "the first declaration")
}

/// Rule: every role an `includes` names is declared. `at` says which.
fn undeclared(n: &Ident, at: &str) -> Violation {
    Violation::error(
        DECLARED,
        n.span,
        format!(
            "{} names role `{}`, which nothing declares — roles are declared \
             vocabulary: `role {};` at top level",
            at, n.name, n.name
        ),
    )
}

/// Rule: `includes` is acyclic.
fn role_cycle(name: &str, span: Span) -> Violation {
    Violation::error(
        ACYCLIC,
        span,
        format!(
            "role `{}` includes itself through its `includes` chain; \
             composition is grant-only and union-only, so a cycle says nothing",
            name
        ),
    )
}

