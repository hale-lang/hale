//! GH #1835 (slice 1): the secrets a program reads, derived from its code.
//!
//! The requirements artifact (`hale check --dump-requirements`,
//! spec/verification.md § "The requirements artifact") states what a
//! program needs from whatever environment runs it. Slice 1 states its
//! **secrets**, from the reads the program already types:
//!
//! - a **declared secret** (`[dna.secrets.<name>]`, spec/dna.md §
//!   Declared secrets) reaches its readers as the variable
//!   `HALE_SECRET_<NAME>`, naming its vault entry (a family's as well
//!   `HALE_SECRET_<NAME>_FAMILY`, its members' prefix). A read of that
//!   variable (`std::env::var` / `std::env::var_exists` with the name
//!   written as a string literal or a const of one, or `env_var:` on a
//!   `std::secret` locus) is a read of the secret `<name>`;
//! - a `std::secret::Credential` or `std::secret::Signer` literal names
//!   its source: `vault:` an entry of the vault, `env_var:` a variable
//!   that holds the material, `key_file:` a file. A source written as a
//!   literal (or a const of one) is a secret by that name; a source that
//!   reads `HALE_SECRET_<NAME>` is the declared secret `<name>`, read as
//!   that locus; any other source is computed, and the read is listed as
//!   **unnamed** — a secret the program reads whose name the checker
//!   cannot see, never left out.
//!
//! The walk visits every expression the programs hold
//! ([`crate::listener_window::for_each_expression_root`]), so a read in a
//! param default or a const is found as one in a body is.
//!
//! What the code cannot say is the manifest's alone this slice: whether
//! the habitat draws the value or a person fills it (`kind = "drawn"` /
//! `"slot"`), and who may read it. Slice 2 makes a secret a parameter of
//! the main locus typed by that kind (`secret::Drawn`), and this walk's
//! declared reads become those parameters.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{flat_decls, Expr, Literal, LocusMember, Program, TopDecl, TypeDeclBody, TypeExpr};
use hale_syntax::Span;

use crate::listener_window::{for_each_expression_root, visit_block, visit_expr, Root, Visited};

/// The prefix of the variable a declared secret's reader is handed.
pub const DECLARED_PREFIX: &str = "HALE_SECRET_";
/// The suffix of a family's members' prefix variable.
pub const FAMILY_SUFFIX: &str = "_FAMILY";
/// The requirements artifact's schema version.
pub const REQUIREMENTS_VERSION: &str = "1.0";

/// Where a secret's value comes from, as the code names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// `[dna.secrets.<name>]`, through `HALE_SECRET_<NAME>`
    Declared,
    /// a vault entry by its literal name (`vault: "forge-token"`)
    Vault,
    /// a variable that holds the material (`env_var: "SIGNING_KEY"`)
    Env,
    /// a file that holds the material (`key_file: "/etc/key"`)
    File,
}

impl Source {
    pub fn word(self) -> &'static str {
        match self {
            Source::Declared => "declared",
            Source::Vault => "vault",
            Source::Env => "env",
            Source::File => "file",
        }
    }
}

/// How the program reads it: as the `std::secret` locus that presents it,
/// or only its entry's name (`entry`), carried on to code the walk does
/// not follow to a locus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Credential,
    Signer,
    Entry,
}

impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Kind::Credential => "credential",
            Kind::Signer => "signer",
            Kind::Entry => "entry",
        }
    }
}

/// One read of a named secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretRead {
    /// the declared secret's name, the vault entry, the variable or the file
    pub name: String,
    pub source: Source,
    pub kind: Kind,
    /// a declared family's members' prefix was read (`HALE_SECRET_<NAME>_FAMILY`)
    pub family: bool,
    pub span: Span,
}

/// A `std::secret` literal whose source is computed: a secret the program
/// reads by a name the checker cannot see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnnamedRead {
    pub kind: Kind,
    /// the source's field: `vault`, `env_var` or `key_file`
    pub field: String,
    pub span: Span,
}

/// Every secret read the programs make.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SecretReads {
    pub named: Vec<SecretRead>,
    pub unnamed: Vec<UnnamedRead>,
}

/// The declared secret a variable names: `HALE_SECRET_OPERATOR_TOKEN` is
/// `operator-token`, `HALE_SECRET_MODEL_TOKEN_FAMILY` the family
/// `model-token`'s members' prefix (`(name, true)`). `None` for any other
/// variable.
pub fn declared_name(var: &str) -> Option<(String, bool)> {
    let rest = var.strip_prefix(DECLARED_PREFIX)?;
    let (rest, family) = match rest.strip_suffix(FAMILY_SUFFIX) {
        Some(r) if !r.is_empty() => (r, true),
        _ => (rest, false),
    };
    if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
        return None;
    }
    Some((rest.to_ascii_lowercase().replace('_', "-"), family))
}

/// The variable a declared secret's readers are handed its entry in.
pub fn declared_var(name: &str) -> String {
    format!("{DECLARED_PREFIX}{}", name.to_ascii_uppercase().replace('-', "_"))
}

/// The secret reads of `programs`; `renames` are the bundle's import
/// renames (`["alias", "NAME"] -> mangled`), so an imported const names
/// its value.
pub fn secret_reads(programs: &[&Program], renames: &[(Vec<String>, String)]) -> SecretReads {
    let mut consts: BTreeMap<String, String> = BTreeMap::new();
    let mut aliases: BTreeMap<String, Kind> = BTreeMap::new();
    for p in programs {
        for d in flat_decls(&p.items) {
            match d {
                TopDecl::Const(c) => {
                    if let Expr::Literal(Literal::String(s), _) = &c.value {
                        consts.insert(c.name.name.clone(), s.clone());
                    }
                }
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        if let LocusMember::Const(c) = m {
                            if let Expr::Literal(Literal::String(s), _) = &c.value {
                                consts.entry(c.name.name.clone()).or_insert_with(|| s.clone());
                            }
                        }
                    }
                }
                TopDecl::Type(t) => {
                    // `type Cred = std::secret::Credential;`
                    if let TypeDeclBody::Alias(TypeExpr::Named { path, .. }) = &t.body {
                        let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
                        if let Some(k) = locus_kind(&segs) {
                            aliases.insert(t.name.name.clone(), k);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let w = Walk { consts: &consts, renames };
    let mut out = SecretReads::default();
    // the spans of the variable reads a `std::secret` literal's source
    // made, which that literal already counted as a typed read
    let mut typed: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut entries: Vec<SecretRead> = Vec::new();
    for p in programs {
        let mut on = |v: Visited<'_>| {
            let Visited::Expr(e) = v else { return };
            if let Some(r) = w.variable_read(e) {
                entries.push(r);
                return;
            }
            let Expr::Struct { path, inits, .. } = e else { return };
            let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
            let alias = || match segs.as_slice() {
                [one] => aliases.get(*one).copied(),
                _ => None,
            };
            let Some(kind) = locus_kind(&segs).or_else(alias) else { return };
            for init in inits {
                let source = match init.name.name.as_str() {
                    "vault" => Source::Vault,
                    "env_var" => Source::Env,
                    "key_file" if kind == Kind::Signer => Source::File,
                    _ => continue,
                };
                if let Some(s) = w.string(&init.value) {
                    if s.is_empty() {
                        continue;
                    }
                    // `env_var: "HALE_SECRET_X"` reads the entry's NAME as
                    // the material: a read of the declared secret all the
                    // same, by its variable
                    if let (Source::Env, Some((name, family))) = (source, declared_name(&s)) {
                        out.named.push(SecretRead { name, source: Source::Declared, kind: Kind::Entry, family, span: init.span });
                    } else {
                        out.named.push(SecretRead { name: s, source, kind, family: false, span: init.span });
                    }
                    continue;
                }
                // a source that reads `HALE_SECRET_<NAME>` anywhere in it
                // (`vault: std::env::var("HALE_SECRET_X")`, or behind an
                // `if std::env::var_exists(…)`) is that secret, read as
                // this locus
                let mut found: Vec<SecretRead> = Vec::new();
                visit_expr(&init.value, &mut |v| {
                    if let Visited::Expr(x) = v {
                        if let Some(r) = w.variable_read(x) {
                            found.push(r);
                        }
                    }
                });
                if found.is_empty() {
                    out.unnamed.push(UnnamedRead { kind, field: init.name.name.clone(), span: init.span });
                    continue;
                }
                for r in found {
                    typed.insert((r.span.start.as_usize(), r.span.end.as_usize()));
                    out.named.push(SecretRead { kind, ..r });
                }
            }
        };
        for_each_expression_root(&p.items, &mut |root| match root {
            Root::Block(b) => visit_block(b, &mut on),
            Root::Expr(e) => visit_expr(e, &mut on),
        });
    }
    out.named.extend(entries.into_iter().filter(|r| !typed.contains(&(r.span.start.as_usize(), r.span.end.as_usize()))));
    out.named.sort_by_key(|r| (r.source, r.name.clone(), r.span.start.as_usize()));
    out.unnamed.sort_by_key(|r| r.span.start.as_usize());
    out
}

/// The `std::secret` locus a type path names.
fn locus_kind(segs: &[&str]) -> Option<Kind> {
    match segs {
        ["std", "secret", "Credential"] | ["__StdSecretCredential"] => Some(Kind::Credential),
        ["std", "secret", "Signer"] | ["__StdSecretSigner"] => Some(Kind::Signer),
        _ => None,
    }
}

struct Walk<'a> {
    consts: &'a BTreeMap<String, String>,
    renames: &'a [(Vec<String>, String)],
}

impl Walk<'_> {
    /// A string the expression is: a literal, or a const of one.
    fn string(&self, e: &Expr) -> Option<String> {
        match e {
            Expr::Literal(Literal::String(s), _) => Some(s.clone()),
            Expr::Ident(id) => self.consts.get(&id.name).cloned(),
            Expr::Path(qn) => {
                let segs: Vec<String> = qn.segments.iter().map(|s| s.name.clone()).collect();
                let mangled = self.renames.iter().find(|(p, _)| *p == segs).map(|(_, m)| m.as_str());
                mangled.and_then(|m| self.consts.get(m)).cloned()
            }
            _ => None,
        }
    }

    /// `std::env::var("HALE_SECRET_<NAME>")` or `std::env::var_exists(…)`:
    /// a read of the declared secret's variable.
    fn variable_read(&self, e: &Expr) -> Option<SecretRead> {
        let Expr::Call { callee, args, span, .. } = e else { return None };
        let Expr::Path(qn) = callee.as_ref() else { return None };
        let segs: Vec<&str> = qn.segments.iter().map(|s| s.name.as_str()).collect();
        if !matches!(segs.as_slice(), ["std", "env", "var"] | ["std", "env", "var_exists"]) {
            return None;
        }
        let var = self.string(args.first()?)?;
        let (name, family) = declared_name(&var)?;
        Some(SecretRead { name, source: Source::Declared, kind: Kind::Entry, family, span: *span })
    }
}

/// Where a read is in the program: its file (relative to the checked
/// target), line and column.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Place {
    pub file: String,
    pub line: usize,
    pub col: usize,
}

/// One secret of the artifact: every read of it, grouped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Secret {
    pub name: String,
    pub source: Source,
    /// the strongest of its reads: a `credential`, else a `signer`, else
    /// its entry's name alone (`entry`)
    pub kind: Kind,
    pub family: bool,
    pub reads: Vec<(Place, Kind)>,
}

/// The secrets of `reads`, grouped by source and name, each read placed.
pub fn secrets(reads: &SecretReads, place: &dyn Fn(Span) -> Option<Place>) -> Vec<Secret> {
    let mut by: BTreeMap<(Source, String), Secret> = BTreeMap::new();
    for r in &reads.named {
        let s = by.entry((r.source, r.name.clone())).or_insert_with(|| Secret {
            name: r.name.clone(),
            source: r.source,
            kind: r.kind,
            family: false,
            reads: Vec::new(),
        });
        s.kind = s.kind.min(r.kind);
        s.family |= r.family;
        if let Some(p) = place(r.span) {
            s.reads.push((p, r.kind));
        }
    }
    let mut out: Vec<Secret> = by.into_values().collect();
    for s in &mut out {
        s.reads.sort();
        s.reads.dedup();
    }
    out
}

/// The requirements artifact (spec/verification.md § "The requirements
/// artifact"): `program` the checked program (its directory's name), each
/// secret with its readers (`readers`: the names the manifest's `readers`
/// give the checked program for it, its directory's or a host command's
/// `host.<command>`), and the unnamed reads.
pub fn render_json(
    program: &str,
    secrets: &[Secret],
    readers: &dyn Fn(&Secret) -> Vec<String>,
    unnamed: &[(Place, Kind, String)],
) -> String {
    use crate::topology::quote;
    let place = |p: &Place| format!("\"file\": {}, \"line\": {}, \"col\": {}", quote(&p.file), p.line, p.col);
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  \"schema\": \"hale.requirements\",\n");
    out.push_str(&format!("  \"version\": {},\n", quote(REQUIREMENTS_VERSION)));
    out.push_str(&format!("  \"program\": {},\n", quote(program)));
    out.push_str("  \"secrets\": [");
    for (i, s) in secrets.iter().enumerate() {
        out.push_str(if i == 0 { "\n" } else { ",\n" });
        out.push_str("    {\n");
        out.push_str(&format!("      \"name\": {},\n", quote(&s.name)));
        out.push_str(&format!("      \"source\": {},\n", quote(s.source.word())));
        if s.source == Source::Declared {
            out.push_str(&format!("      \"variable\": {},\n", quote(&declared_var(&s.name))));
        }
        out.push_str(&format!("      \"kind\": {},\n", quote(s.kind.word())));
        out.push_str(&format!("      \"family\": {},\n", s.family));
        let names: Vec<String> = readers(s).iter().map(|r| quote(r)).collect();
        out.push_str(&format!("      \"readers\": [{}],\n", names.join(", ")));
        out.push_str("      \"reads\": [");
        for (j, (p, k)) in s.reads.iter().enumerate() {
            out.push_str(if j == 0 { "\n" } else { ",\n" });
            out.push_str(&format!("        {{ {}, \"kind\": {} }}", place(p), quote(k.word())));
        }
        out.push_str(if s.reads.is_empty() { "]\n" } else { "\n      ]\n" });
        out.push_str("    }");
    }
    out.push_str(if secrets.is_empty() { "],\n" } else { "\n  ],\n" });
    out.push_str("  \"unnamed\": [");
    for (i, (p, k, field)) in unnamed.iter().enumerate() {
        out.push_str(if i == 0 { "\n" } else { ",\n" });
        out.push_str(&format!("    {{ {}, \"kind\": {}, \"field\": {} }}", place(p), quote(k.word()), quote(field)));
    }
    out.push_str(if unnamed.is_empty() { "]\n" } else { "\n  ]\n" });
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_variable_names_its_declared_secret() {
        // spelled through the prefix, so no variable is named literally in
        // shipped source (`env_knobs_are_documented`)
        let var = |rest: &str| format!("{DECLARED_PREFIX}{rest}");
        assert_eq!(declared_name(&var("OPERATOR_TOKEN")), Some(("operator-token".into(), false)));
        assert_eq!(declared_name(&var("MODEL_TOKEN_FAMILY")), Some(("model-token".into(), true)));
        assert_eq!(declared_name(&var("")), None);
        assert_eq!(declared_name(&var("lower")), None);
        assert_eq!(declared_name("PATH"), None);
        assert_eq!(declared_var("operator-token"), var("OPERATOR_TOKEN"));
    }
}
